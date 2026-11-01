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
}

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
        let length = self.read_varint_from_stream().await?;
        if length <= 0 {
            return Err(Error::Protocol(format!("invalid frame length {length}")));
        }
        let length = length as usize;
        if length > MAX_PACKET_LEN {
            return Err(Error::PacketTooLarge {
                len: length,
                max: MAX_PACKET_LEN,
            });
        }

        let mut body = vec![0u8; length];
        self.stream.read_exact(&mut body).await?;
        if let Some(dec) = self.dec.as_mut() {
            dec.decrypt(&mut body);
        }

        let payload = decode_body(&body, self.threshold)?;
        Packet::from_payload(&payload)
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

    async fn read_varint_from_stream(&mut self) -> Result<i32> {
        let mut value: u32 = 0;
        let mut shift = 0u32;
        loop {
            let mut byte = [0u8; 1];
            self.stream.read_exact(&mut byte).await?;
            if let Some(dec) = self.dec.as_mut() {
                dec.decrypt(&mut byte);
            }
            value |= ((byte[0] & 0x7F) as u32) << shift;
            if byte[0] & 0x80 == 0 {
                return Ok(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return Err(Error::VarIntTooBig);
            }
        }
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
}
