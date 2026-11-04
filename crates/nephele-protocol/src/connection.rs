use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::crypto::Cfb8;
use crate::error::{Error, Result};
use crate::framing::{decode_body, encode_frame, MAX_PACKET_LEN};
use crate::packet::Packet;
use crate::packets::handshake::Handshake;
use crate::packets::login::{self, EncryptionRequest, LoginSuccess};
use crate::packets::play::{self, JoinGame};
use crate::packets::status::{self, StatusResponse};
use crate::state::{NextState, State};
use crate::{MINECRAFT_VERSION, PROTOCOL_VERSION};

pub const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(Error::BadAddress {
                input: input.to_owned(),
            });
        }

        if let Some(rest) = input.strip_prefix('[') {
            let (host, after) = rest.split_once(']').ok_or_else(|| Error::BadAddress {
                input: input.to_owned(),
            })?;
            let port = match after.strip_prefix(':') {
                Some(port) if !port.is_empty() => port.parse().map_err(|_| Error::BadAddress {
                    input: input.to_owned(),
                })?,
                _ => DEFAULT_PORT,
            };
            return Ok(Self {
                host: host.to_owned(),
                port,
            });
        }

        match input.rsplit_once(':') {
            Some((host, port))
                if !host.is_empty()
                    && !port.is_empty()
                    && port.chars().all(|c| c.is_ascii_digit()) =>
            {
                Ok(Self {
                    host: host.to_owned(),
                    port: port.parse().map_err(|_| Error::BadAddress {
                        input: input.to_owned(),
                    })?,
                })
            }
            _ => Ok(Self {
                host: input.to_owned(),
                port: DEFAULT_PORT,
            }),
        }
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone)]
pub enum LoginOutcome {
    Success(LoginSuccess),
    Disconnect(String),
    EncryptionRequest(EncryptionRequest),
}

/// A framed, optionally-encrypted connection to a Minecraft 1.8.9 server.
pub struct Connection {
    stream: TcpStream,
    state: State,
    threshold: Option<u32>,
    enc: Option<Cfb8>,
    dec: Option<Cfb8>,
    /// Raw bytes read from the socket that are not yet part of a complete
    /// frame.
    ///
    /// Reads are driven from a `select!` alongside the 20 Hz tick, so a packet
    /// read can be cancelled mid-frame. Buffering the whole frame before
    /// consuming anything keeps that safe: cancelling drops only an incomplete
    /// future, never bytes. The previous `read_exact`-per-field reader silently
    /// dropped whatever it had already consumed, which desynchronised the
    /// stream and showed up as a zlib error several packets later.
    buffer: Vec<u8>,
    /// Length of the decrypted prefix of `buffer`.
    ///
    /// Decryption is applied lazily, only to bytes a frame actually needs. It
    /// has to be lazy: encryption can be switched on mid-stream, and a read
    /// that ran ahead of that point is holding ciphertext, not plaintext.
    /// Decrypting on arrival would have turned the post-handshake packet into
    /// unparsed garbage.
    decrypted: usize,
    /// Opt-in (`NEPHELE_TRACE=1`) ring of the most recently parsed frames.
    /// Dumped when a frame length or body is rejected, so a desync names the
    /// packet it followed instead of only showing the garbage length.
    trace: Option<VecDeque<TraceFrame>>,
    /// `NEPHELE_TRACE_CAPTURE=<path>` appends every parsed packet as
    /// `id length hex`, for turning a real-server bug into a test fixture.
    capture: Option<std::fs::File>,
    /// `NEPHELE_TRACE_RAW=<path>` appends play-state frames after compression
    /// is decoded, each with a plain VarInt length prefix. The replay test
    /// feeds this file back through the framer without a server.
    raw_capture: Option<std::fs::File>,
    raw_bytes: usize,
}

/// One parsed frame, kept for `NEPHELE_TRACE`.
#[derive(Debug, Clone)]
struct TraceFrame {
    id: i32,
    /// Frame body length on the wire (everything after the length VarInt).
    length: usize,
    /// Payload bytes the parser actually read. `None` until the client has
    /// handled the packet.
    consumed: Option<usize>,
    payload_len: usize,
    skipped: bool,
}

const TRACE_FRAMES: usize = 16;
/// Default cap for `NEPHELE_TRACE_RAW`, in bytes (4 MiB is ~a minute of a busy
/// lobby, which is plenty for a replay fixture).
const RAW_CAPTURE_MAX: usize = 4 * 1024 * 1024;

impl Connection {
    pub async fn connect(address: &ServerAddress) -> Result<Self> {
        let stream = TcpStream::connect((address.host.as_str(), address.port)).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            state: State::Handshake,
            threshold: None,
            enc: None,
            dec: None,
            buffer: Vec::new(),
            decrypted: 0,
            trace: trace_enabled().then(|| VecDeque::with_capacity(TRACE_FRAMES)),
            capture: capture_file(),
            raw_capture: raw_capture_file(),
            raw_bytes: 0,
        })
    }

    /// Wrap an accepted socket (used by in-process test servers and, later, by
    /// `nephele-server`).
    pub fn from_stream(stream: TcpStream, state: State) -> Self {
        Self {
            stream,
            state,
            threshold: None,
            enc: None,
            dec: None,
            buffer: Vec::new(),
            decrypted: 0,
            trace: trace_enabled().then(|| VecDeque::with_capacity(TRACE_FRAMES)),
            capture: capture_file(),
            raw_capture: raw_capture_file(),
            raw_bytes: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn compression_threshold(&self) -> Option<u32> {
        self.threshold
    }

    pub fn set_compression_threshold(&mut self, threshold: Option<u32>) {
        self.threshold = threshold;
    }

    pub fn is_encrypted(&self) -> bool {
        self.enc.is_some()
    }

    pub fn peer_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.stream.peer_addr()
    }

    /// Turn on AES-128/CFB8 for both directions. The response that negotiated
    /// encryption must already have been written.
    pub fn enable_encryption(&mut self, key: &[u8; 16], iv: &[u8; 16]) {
        self.enc = Some(Cfb8::new(key, iv));
        self.dec = Some(Cfb8::new(key, iv));
    }

    pub async fn handshake(&mut self, address: &ServerAddress, next: NextState) -> Result<()> {
        if self.state != State::Handshake {
            return Err(Error::Protocol(format!(
                "handshake already sent (state is {})",
                self.state
            )));
        }
        let packet = Handshake::new(address.host.clone(), address.port, next).to_packet();
        self.write_packet(&packet).await?;
        self.state = next.to_state();
        Ok(())
    }

    /// Full server-list ping. Returns the parsed MOTD and round-trip latency.
    pub async fn status(&mut self) -> Result<StatusResponse> {
        if self.state != State::Status {
            return Err(Error::Protocol(format!(
                "status request requires the status state, currently {}",
                self.state
            )));
        }

        self.write_packet(&status::request_packet()).await?;
        let response = self.read_packet().await?;
        // Note: in the status state id 0x00 is the response, not a disconnect.
        response.check_id(status::CLIENTBOUND_RESPONSE, "status")?;
        let mut parsed = status::parse_response(&response)?;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as i64)
            .unwrap_or(0);
        let sent = Instant::now();
        self.write_packet(&status::ping_packet(nonce)).await?;
        let pong = self.read_packet().await?;
        pong.check_id(status::CLIENTBOUND_PONG, "status")?;
        let echoed = status::parse_pong(&pong)?;
        if echoed != nonce {
            return Err(Error::Protocol(format!(
                "ping nonce mismatch: sent {nonce}, server echoed {echoed}"
            )));
        }
        parsed.latency = Some(sent.elapsed());
        Ok(parsed)
    }

    /// Send Login Start and consume the login response, handling
    /// Set Compression transparently.
    pub async fn login_start(&mut self, username: &str) -> Result<LoginOutcome> {
        self.state = State::Login;
        self.write_packet(&login::login_start(username)).await?;
        self.read_login_outcome().await
    }

    /// Answer an Encryption Request and switch the connection to AES.
    ///
    /// The response goes out in the clear and encryption starts immediately
    /// afterwards, which is the order the server expects.
    /// The raw secret is used only for AES/CFB8; both secret and verify token
    /// ciphertexts are carried in the packet.
    pub async fn send_encryption_response(
        &mut self,
        shared_secret: &[u8; 16],
        encrypted_shared_secret: &[u8],
        encrypted_verify_token: &[u8],
    ) -> Result<()> {
        let packet = login::encryption_response(encrypted_shared_secret, encrypted_verify_token);
        self.write_packet(&packet).await?;
        self.enable_encryption(shared_secret, shared_secret);
        self.state = State::Login;
        Ok(())
    }

    /// Read the login response after the handshake continues — used after an
    /// Encryption Response to wait for Login Success.
    pub async fn read_login_outcome(&mut self) -> Result<LoginOutcome> {
        loop {
            let packet = self.read_packet().await?;
            match packet.id {
                login::CLIENTBOUND_SET_COMPRESSION => {
                    let threshold = login::parse_set_compression(&packet)?;
                    self.threshold = Some(threshold);
                    tracing::debug!(threshold, "compression enabled");
                }
                login::CLIENTBOUND_DISCONNECT => {
                    return Ok(LoginOutcome::Disconnect(login::parse_disconnect(&packet)?))
                }
                login::CLIENTBOUND_ENCRYPTION_REQUEST => {
                    return Ok(LoginOutcome::EncryptionRequest(
                        login::parse_encryption_request(&packet)?,
                    ))
                }
                login::CLIENTBOUND_LOGIN_SUCCESS => {
                    return Ok(LoginOutcome::Success(login::parse_login_success(&packet)?))
                }
                id => return Err(Error::UnexpectedPacket { id, state: "login" }),
            }
        }
    }

    /// Switch to the play state and read until Join Game arrives, answering
    /// keep-alives and honouring a play-state Set Compression in the meantime.
    pub async fn wait_for_join_game(&mut self) -> Result<JoinGame> {
        self.state = State::Play;
        loop {
            let packet = self.read_packet().await?;
            match packet.id {
                play::clientbound::JOIN_GAME => return play::parse_join_game(&packet),
                play::clientbound::DISCONNECT => {
                    return Err(Error::Disconnected(play::parse_disconnect(&packet)?))
                }
                play::clientbound::SET_COMPRESSION => {
                    self.threshold = Some(login::parse_set_compression(&packet)?);
                }
                play::clientbound::KEEP_ALIVE => self.answer_keep_alive(&packet).await?,
                _ => {}
            }
        }
    }

    /// Read one play packet, automatically answering keep-alives.
    pub async fn next_play_packet(&mut self) -> Result<Packet> {
        loop {
            let packet = self.read_packet().await?;
            match packet.id {
                play::clientbound::KEEP_ALIVE => {
                    self.answer_keep_alive(&packet).await?;
                    return Ok(packet);
                }
                play::clientbound::SET_COMPRESSION => {
                    self.threshold = Some(login::parse_set_compression(&packet)?);
                }
                play::clientbound::DISCONNECT => {
                    return Err(Error::Disconnected(play::parse_disconnect(&packet)?))
                }
                _ => return Ok(packet),
            }
        }
    }

    pub async fn answer_keep_alive(&mut self, packet: &Packet) -> Result<()> {
        let id = packet.reader().read_varint()?;
        self.write_packet(&play::keep_alive_response(id)).await
    }

    pub async fn send_chat(&mut self, message: &str) -> Result<()> {
        self.write_packet(&play::chat_message(message)).await
    }

    pub async fn read_packet(&mut self) -> Result<Packet> {
        // Decrypt just enough to read the length prefix, then wait for the
        // whole frame before draining anything.
        let (length, header) = loop {
            if let Some((length, used)) = decode_varint_prefix(&self.buffer[..self.decrypted])? {
                break (length, used);
            }
            if self.decrypted < self.buffer.len() {
                self.decrypt_upto(self.decrypted + 1);
            } else {
                self.fill().await?;
            }
        };

        if length <= 0 {
            self.dump_trace(&format!("invalid frame length {length}"));
            return Err(Error::Protocol(format!("invalid frame length {length}")));
        }
        let length = length as usize;
        if length > MAX_PACKET_LEN {
            self.dump_trace(&format!(
                "outer VarInt: wire_body_declared={length} limit={MAX_PACKET_LEN} wire_header_consumed={header}"
            ));
            return Err(Error::PacketTooLarge {
                len: length,
                max: MAX_PACKET_LEN,
            });
        }

        while self.buffer.len() < header + length {
            self.fill().await.inspect_err(|_| {
                self.dump_trace(&format!(
                    "socket read failed: buffered_wire_bytes={} frame_declared={length}",
                    self.buffer.len()
                ));
            })?;
        }
        self.decrypt_upto(header + length);

        let body: Vec<u8> = self.buffer[header..header + length].to_vec();
        self.buffer.drain(..header + length);
        self.decrypted -= header + length;

        let payload = match decode_body(&body, self.threshold) {
            Ok(payload) => payload,
            Err(err) => {
                self.dump_trace(&format!("decode failed: wire_body={}", body.len()));
                return Err(err);
            }
        };
        self.raw_capture_payload(&payload);

        let packet = Packet::from_payload(&payload).inspect_err(|_| {
            self.dump_trace(&format!(
                "invalid packet ID: wire_body={} decoded_with_id={}",
                body.len(),
                payload.len()
            ));
        })?;
        self.record_trace(&packet, body.len());
        Ok(packet)
    }

    /// Append this frame's *decoded* payload to `NEPHELE_TRACE_RAW`.
    ///
    /// Compression is per-frame, so the replay capture stores plain payloads
    /// with a normal length prefix: the framing bug being guarded against is the
    /// same either way, and a replay then needs no threshold.
    fn raw_capture_payload(&mut self, payload: &[u8]) {
        if self.state != State::Play {
            return;
        }
        let Some(file) = self.raw_capture.as_mut() else {
            return;
        };
        if self.raw_bytes + payload.len() > RAW_CAPTURE_MAX {
            return;
        }
        let Ok(frame) = encode_frame(payload, None) else {
            return;
        };
        use std::io::Write;
        if file.write_all(&frame).is_ok() {
            self.raw_bytes += frame.len();
        }
    }

    /// Remember the last few parsed frames so a desync can be attributed.
    fn record_trace(&mut self, packet: &Packet, frame_length: usize) {
        if let Some(capture) = self.capture.as_mut() {
            use std::io::Write;
            // Login and play packet ids overlap, so record the state too.
            let _ = writeln!(
                capture,
                "{} {:02X} {} {}",
                self.state.name(),
                packet.id,
                frame_length,
                hex(&packet.data)
            );
        }
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        if trace.len() == TRACE_FRAMES {
            trace.pop_front();
        }
        trace.push_back(TraceFrame {
            id: packet.id,
            length: frame_length,
            consumed: None,
            payload_len: packet.data.len(),
            skipped: false,
        });
    }

    /// Record how many payload bytes the client's parser read from the most
    /// recently returned packet. Kept separate from `record_trace` because the
    /// connection hands the packet out before anything parses it.
    pub fn note_consumed(&mut self, consumed: usize, skipped: bool) {
        if let Some(frame) = self.trace.as_mut().and_then(|trace| trace.back_mut()) {
            frame.consumed = Some(consumed);
            frame.skipped = skipped;
        }
    }

    /// Print the frame ring to stderr when `NEPHELE_TRACE=1` is set.
    pub fn dump_trace(&self, reason: &str) {
        let Some(trace) = self.trace.as_ref() else {
            return;
        };
        eprintln!("[nephele trace] {reason}");
        eprintln!("[nephele trace] last {} parsed frames:", trace.len());
        for frame in trace {
            let consumed = frame
                .consumed
                .map(|bar| bar.to_string())
                .unwrap_or_else(|| "-".into());
            eprintln!(
                "[nephele trace] id=0x{:02X} wire_body={} decoded_payload={} parser_consumed={} skipped={}",
                frame.id, frame.length, frame.payload_len, consumed, frame.skipped
            );
        }
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        let payload = packet.to_payload();
        let mut frame = encode_frame(&payload, self.threshold)?;
        if let Some(enc) = self.enc.as_mut() {
            enc.encrypt(&mut frame);
        }
        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Read one more chunk off the socket and append it, still encrypted if
    /// encryption is on, to `buffer`.
    ///
    /// Cancelling this future is safe: its only `await` is a single `read`, and
    /// the bytes it returns are moved into `buffer` before the next await. That
    /// is the whole reason packets are buffered instead of read field by field.
    async fn fill(&mut self) -> Result<()> {
        let mut chunk = [0u8; 8192];
        let read = self.stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(Error::UnexpectedEof { needed: 1 });
        }
        self.buffer.extend_from_slice(&chunk[..read]);
        Ok(())
    }

    /// Decrypt the buffered ciphertext up to `end` bytes.
    ///
    /// Bytes that were already decrypted are never touched again, so the CFB8
    /// feedback keeps advancing in stream order.
    fn decrypt_upto(&mut self, end: usize) {
        let end = end.min(self.buffer.len());
        if end <= self.decrypted {
            return;
        }
        if let Some(dec) = self.dec.as_mut() {
            dec.decrypt(&mut self.buffer[self.decrypted..end]);
        }
        self.decrypted = end;
    }

    pub async fn shutdown(&mut self) {
        let _ = self.stream.shutdown().await;
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("state", &self.state)
            .field("threshold", &self.threshold)
            .field("encrypted", &self.enc.is_some())
            .finish()
    }
}

/// `1.8.9 (protocol 47)` — handy for banners and logs.
pub fn version_string() -> String {
    format!("{MINECRAFT_VERSION} (protocol {PROTOCOL_VERSION})")
}

/// Convenience: connect and run a full status ping in one call.
pub async fn ping(address: &ServerAddress, timeout: Duration) -> Result<StatusResponse> {
    let mut connection = tokio::time::timeout(timeout, Connection::connect(address))
        .await
        .map_err(|_| Error::Protocol(format!("timed out connecting to {address}")))??;
    tokio::time::timeout(timeout, async {
        connection.handshake(address, NextState::Status).await?;
        connection.status().await
    })
    .await
    .map_err(|_| Error::Protocol(format!("timed out pinging {address}")))?
}

/// `NEPHELE_TRACE=1` turns on the frame ring dump.
fn trace_enabled() -> bool {
    std::env::var("NEPHELE_TRACE").is_ok_and(|value| value == "1")
}

/// `NEPHELE_TRACE_CAPTURE=<path>` opens an append log of every parsed packet.
fn capture_file() -> Option<std::fs::File> {
    let path = std::env::var_os("NEPHELE_TRACE_CAPTURE")?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// `NEPHELE_TRACE_RAW=<path>` opens an append log of exact wire frames.
fn raw_capture_file() -> Option<std::fs::File> {
    let path = std::env::var_os("NEPHELE_TRACE_RAW")?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// Lowercase hex, for the trace dump.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decode a VarInt at the start of `bytes` without consuming anything.
///
/// `None` means "not enough bytes yet, read more"; a fifth continuation byte is
/// an error rather than another request for data.
fn decode_varint_prefix(bytes: &[u8]) -> Result<Option<(i32, usize)>> {
    let mut value = 0u32;
    for index in 0..5 {
        let Some(&byte) = bytes.get(index) else {
            return Ok(None);
        };
        value |= ((byte & 0x7F) as u32) << (7 * index);
        if byte & 0x80 == 0 {
            return Ok(Some((value as i32, index + 1)));
        }
    }
    Err(Error::VarIntTooBig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_host() {
        let addr = ServerAddress::parse("mc.example.net").unwrap();
        assert_eq!(addr.host, "mc.example.net");
        assert_eq!(addr.port, DEFAULT_PORT);
    }

    #[test]
    fn parses_host_and_port() {
        let addr = ServerAddress::parse("mc.example.net:25566").unwrap();
        assert_eq!(addr.host, "mc.example.net");
        assert_eq!(addr.port, 25566);
    }

    #[test]
    fn parses_bracketed_ipv6() {
        let addr = ServerAddress::parse("[::1]:25567").unwrap();
        assert_eq!(addr.host, "::1");
        assert_eq!(addr.port, 25567);
    }

    #[test]
    fn ipv6_without_port_keeps_default() {
        let addr = ServerAddress::parse("[::1]").unwrap();
        assert_eq!(addr.host, "::1");
        assert_eq!(addr.port, DEFAULT_PORT);
    }

    #[test]
    fn rejects_empty() {
        assert!(ServerAddress::parse("   ").is_err());
    }

    #[test]
    fn displays_host_and_port() {
        assert_eq!(
            ServerAddress::parse("localhost").unwrap().to_string(),
            "localhost:25565"
        );
    }

    /// Cancelling a packet read mid-frame must not lose the rest of the frame.
    ///
    /// This is the bug that showed up on a real server as
    /// `compression failure: corrupt deflate stream`: the network loop reads
    /// packets inside a `select!` against the 20 Hz ticker, so a tick landing
    /// mid-frame used to cancel a `read_exact` that had already swallowed part
    /// of the body. The stream then started one frame late and every later
    /// length was garbage.
    #[tokio::test]
    async fn cancelling_a_mid_frame_read_keeps_the_stream_in_sync() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let first = encode_frame(&Packet::new(0x03, b"first".to_vec()).to_payload(), None).unwrap();
        let second =
            encode_frame(&Packet::new(0x04, b"second".to_vec()).to_payload(), None).unwrap();

        let server = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            // Split the first frame so the client is guaranteed to be parked
            // inside the body read when the timeout fires.
            peer.write_all(&first[..4]).await.unwrap();
            peer.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            peer.write_all(&first[4..]).await.unwrap();
            peer.write_all(&second).await.unwrap();
            peer.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        });

        let stream = TcpStream::connect(address).await.unwrap();
        let mut connection = Connection::from_stream(stream, State::Play);

        // Stand in for the ticker winning the race.
        let cancelled =
            tokio::time::timeout(Duration::from_millis(10), connection.read_packet()).await;
        assert!(cancelled.is_err(), "the read should have been cancelled");

        let decoded = connection.read_packet().await.expect("the cancelled frame");
        assert_eq!(
            decoded.to_payload(),
            Packet::new(0x03, b"first".to_vec()).to_payload()
        );
        let decoded = connection.read_packet().await.expect("the following frame");
        assert_eq!(
            decoded.to_payload(),
            Packet::new(0x04, b"second".to_vec()).to_payload()
        );

        server.await.unwrap();
    }
}
