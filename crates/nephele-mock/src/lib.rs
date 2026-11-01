//! `nephele-mock` — a tiny fake Minecraft 1.8.9 server.
//!
//! It speaks just enough of protocol 47 for Nephele to join, receive a
//! deterministic world, chat and stay alive. It exists so the client can be
//! developed and tested without a real Java server running.
//!
//! ```no_run
//! # async fn demo() {
//! use nephele_mock::{MockConfig, MockServer};
//! let server = MockServer::start(MockConfig::default()).await.unwrap();
//! println!("mock server on {}", server.address());
//! # }
//! ```

pub mod terrain;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nephele_protocol::packets::{login, play, status};
use nephele_protocol::{Connection, Error, Packet, PacketWriter, Result, State};
use nephele_world::{encode_chunk_bulk, BlockState, Face, World};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// How long between server keep-alives.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct MockConfig {
    /// Chunk radius sent on join (2 → a 5×5 region).
    pub radius: i32,
    /// Compression threshold announced in login; `None` disables compression.
    pub compression_threshold: Option<u32>,
    /// Send a chat greeting after the world loads.
    pub greeting: bool,
    /// Spawn fake walking players around the spawn point, so entity tracking,
    /// interpolation and the humanoid renderer can be exercised offline.
    pub bots: bool,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            radius: 2,
            compression_threshold: Some(64),
            greeting: true,
            bots: false,
        }
    }
}

pub struct MockServer {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl MockServer {
    pub async fn start(config: MockConfig) -> std::io::Result<Self> {
        Self::bind("127.0.0.1:0", config).await
    }

    pub async fn start_on(port: u16, config: MockConfig) -> std::io::Result<Self> {
        Self::bind(&format!("127.0.0.1:{port}"), config).await
    }

    async fn bind(addr: &str, config: MockConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let config = Arc::new(config);

        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let config = Arc::clone(&config);
                        tokio::spawn(async move {
                            let mut connection = Connection::from_stream(stream, State::Handshake);
                            if let Err(err) = serve_connection(&mut connection, config).await {
                                tracing::debug!("mock connection ended: {err}");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::debug!("mock accept failed: {err}");
                        break;
                    }
                }
            }
        });

        Ok(Self { addr, task })
    }

    pub fn socket_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn address(&self) -> String {
        self.addr.to_string()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_connection(connection: &mut Connection, config: Arc<MockConfig>) -> Result<()> {
    let handshake = connection.read_packet().await?;
    handshake.check_id(0x00, "handshake")?;
    let mut reader = handshake.reader();
    let _protocol = reader.read_varint()?;
    let _host = reader.read_string()?;
    let _port = reader.read_u16()?;
    let next_state = reader.read_varint()?;

    match next_state {
        1 => {
            connection.set_state(State::Status);
            serve_status(connection).await
        }
        2 => {
            connection.set_state(State::Login);
            serve_login(connection, config).await
        }
        other => Err(Error::Protocol(format!(
            "handshake asked for unknown next state {other}"
        ))),
    }
}

async fn serve_status(connection: &mut Connection) -> Result<()> {
    let request = connection.read_packet().await?;
    request.check_id(status::SERVERBOUND_REQUEST, "status")?;

    let json = r#"{"version":{"name":"1.8.9","protocol":47},"players":{"max":20,"online":1},"description":{"text":"Nephele mock server","extra":[{"text":" (synthetic world)","color":"gray"}]}}"#;
    let mut w = PacketWriter::new();
    w.write_string(json);
    connection
        .write_packet(&Packet::new(status::CLIENTBOUND_RESPONSE, w.into_bytes()))
        .await?;

    let ping = connection.read_packet().await?;
    if ping.id == status::SERVERBOUND_PING {
        let nonce = ping.reader().read_i64()?;
        let mut w = PacketWriter::new();
        w.write_i64(nonce);
        connection
            .write_packet(&Packet::new(status::CLIENTBOUND_PONG, w.into_bytes()))
            .await?;
    }
    Ok(())
}

async fn serve_login(connection: &mut Connection, config: Arc<MockConfig>) -> Result<()> {
    let start = connection.read_packet().await?;
    start.check_id(login::SERVERBOUND_LOGIN_START, "login")?;
    let username = start.reader().read_string()?;

    if let Some(threshold) = config.compression_threshold {
        let mut w = PacketWriter::new();
        w.write_varint(threshold as i32);
        connection
            .write_packet(&Packet::new(
                login::CLIENTBOUND_SET_COMPRESSION,
                w.into_bytes(),
            ))
            .await?;
        connection.set_compression_threshold(Some(threshold));
    }

    let mut w = PacketWriter::new();
    w.write_string(&fake_uuid(&username))
        .write_string(&username);
    connection
        .write_packet(&Packet::new(
            login::CLIENTBOUND_LOGIN_SUCCESS,
            w.into_bytes(),
        ))
        .await?;

    connection.set_state(State::Play);

    let columns = send_join_sequence(connection, &config).await?;
    let mut world = World::new(0);
    for column in columns {
        world.insert(column);
    }
    play_loop(connection, &username, &mut world, config.bots).await
}

async fn send_join_sequence(
    connection: &mut Connection,
    config: &MockConfig,
) -> Result<Vec<nephele_world::ChunkColumn>> {
    // Join Game (creative, overworld, survival difficulty, 20 slots)
    let mut w = PacketWriter::new();
    w.write_i32(1)
        .write_u8(1)
        .write_i8(0)
        .write_u8(1)
        .write_u8(20)
        .write_string("default")
        .write_bool(false);
    connection
        .write_packet(&Packet::new(play::clientbound::JOIN_GAME, w.into_bytes()))
        .await?;

    // Spawn Position
    let spawn_y = terrain::height_at(8, 8) as i64 + 1;
    // Bots are shown in the air, clear of the tree at the spawn point, so the
    // humanoid renderer is visible without needing a specific camera angle.
    let player_y = spawn_y;
    let mut w = PacketWriter::new();
    w.write_i64((spawn_y & 0xFFF) << 26);
    connection
        .write_packet(&Packet::new(
            play::clientbound::SPAWN_POSITION,
            w.into_bytes(),
        ))
        .await?;

    // Player Abilities: invulnerable | flying | allow flying | creative
    let mut w = PacketWriter::new();
    w.write_u8(0x0F).write_f32(0.05).write_f32(0.1);
    connection
        .write_packet(&Packet::new(
            play::clientbound::PLAYER_ABILITIES,
            w.into_bytes(),
        ))
        .await?;

    // Player Position And Look. Tilted 25° down so the section outline and the
    // block you spawn on are in view immediately; with bots enabled it looks
    // level instead, so the fake players stand against the sky.
    let pitch = if config.bots { 0.0 } else { 55.0 };
    let mut w = PacketWriter::new();
    // There is a tree trunk one block south of the spawn, so with bots the
    // player faces north instead, where the view is open.
    let yaw = if config.bots { 180.0 } else { 0.0 };
    w.write_f64(8.5)
        .write_f64(player_y as f64)
        .write_f64(8.5)
        .write_f32(yaw)
        .write_f32(pitch)
        .write_u8(0);
    connection
        .write_packet(&Packet::new(
            play::clientbound::PLAYER_POSITION_AND_LOOK,
            w.into_bytes(),
        ))
        .await?;

    // The world, in one bulk packet.
    let columns = terrain::generate_region(config.radius);
    connection
        .write_packet(&encode_chunk_bulk(&columns, true))
        .await?;

    // Time Update
    let mut w = PacketWriter::new();
    w.write_i64(0).write_i64(6_000);
    connection
        .write_packet(&Packet::new(play::clientbound::TIME_UPDATE, w.into_bytes()))
        .await?;

    // Update Health
    let mut w = PacketWriter::new();
    w.write_f32(20.0).write_varint(20).write_f32(5.0);
    connection
        .write_packet(&Packet::new(
            play::clientbound::UPDATE_HEALTH,
            w.into_bytes(),
        ))
        .await?;

    if config.greeting {
        send_chat(connection, "Welcome to the Nephele mock server.").await?;
    }
    // Bots last, after the world, so the client has chunks to stand them on.
    if config.bots {
        let bots = make_bots(player_y as f64);
        send_bot_player_list(connection, &bots).await?;
        send_bot_spawns(connection, &bots).await?;
    }
    Ok(columns)
}

/// One unverified account's textures property, as a real tab-list entry carries.
/// The hash is intentionally not a real skin: the client should fall back to its
/// built-in skin when the download fails.
const BOT_TEXTURES: &str = "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHA6Ly90ZXh0dXJlcy5taW5lY3JhZnQubmV0L3RleHR1cmUvNmVwaGVsZSI6IiJ9fX0=";

/// Fake players the mock server spawns and walks around, so entity tracking,
/// interpolation and the humanoid renderer can be exercised without a server.
struct MockBot {
    entity_id: i32,
    uuid: [u8; 16],
    name: &'static str,
    /// Position of the circle's centre; the bot orbits it.
    centre: (f64, f64),
    radius: f64,
    /// Starting angle in radians, so the bots don't overlap.
    phase: f64,
    /// Height their feet rest at.
    feet_y: f64,
}

const BOT_NAMES: [&str; 8] = [
    "Botto",
    "Steve",
    "Alex",
    "Herobrine",
    "Notch",
    "Jeb",
    "Dinnerbone",
    "Grumm",
];

const BOT_COUNT: usize = 8;

/// Bots ringed around the spawn point, so they are in view whichever way the
/// player happens to be looking.
fn make_bots(feet_y: f64) -> Vec<MockBot> {
    (0..BOT_COUNT)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::TAU / BOT_COUNT as f64;
            MockBot {
                entity_id: 2 + index as i32,
                uuid: [0x10 + index as u8; 16],
                name: BOT_NAMES[index % BOT_NAMES.len()],
                centre: (8.5 + 3.0 * angle.cos(), 8.5 + 3.0 * angle.sin()),
                radius: 0.9,
                phase: angle,
                feet_y,
            }
        })
        .collect()
}

fn bot_position(bot: &MockBot, angle: f64) -> (f64, f64) {
    (
        bot.centre.0 + bot.radius * angle.cos(),
        bot.centre.1 + bot.radius * angle.sin(),
    )
}

/// `S38` add-player entries, one per bot.
async fn send_bot_player_list(connection: &mut Connection, bots: &[MockBot]) -> Result<()> {
    let mut w = PacketWriter::new();
    w.write_varint(0).write_varint(bots.len() as i32);
    for bot in bots {
        w.write_uuid(&bot.uuid)
            .write_string(bot.name)
            .write_varint(1)
            .write_string("textures")
            .write_string(BOT_TEXTURES)
            .write_bool(false)
            .write_varint(1)
            .write_varint(30)
            .write_bool(false);
    }
    connection
        .write_packet(&Packet::new(
            play::clientbound::PLAYER_LIST_ITEM,
            w.into_bytes(),
        ))
        .await
}

/// `S0C` spawn player.
#[allow(clippy::too_many_arguments)]
async fn send_spawn_bot(
    connection: &mut Connection,
    id: i32,
    uuid: &[u8; 16],
    name: &str,
    x: f64,
    y: f64,
    z: f64,
    yaw: f32,
) -> Result<()> {
    let mut w = PacketWriter::new();
    w.write_varint(id)
        .write_uuid(uuid)
        .write_string(name)
        .write_i32((x * 32.0) as i32)
        .write_i32((y * 32.0) as i32)
        .write_i32((z * 32.0) as i32)
        .write_u8(((yaw / 360.0) * 256.0) as u8)
        .write_u8(0)
        .write_i16(0)
        .write_u8(0);
    connection
        .write_packet(&Packet::new(
            play::clientbound::SPAWN_PLAYER,
            w.into_bytes(),
        ))
        .await
}

/// `S17` look + relative move, in 1/32-block steps.
async fn send_bot_move(
    connection: &mut Connection,
    id: i32,
    dx: f64,
    dy: f64,
    dz: f64,
    yaw: f32,
) -> Result<()> {
    let mut w = PacketWriter::new();
    w.write_varint(id)
        .write_i8((dx * 32.0).round() as i8)
        .write_i8((dy * 32.0).round() as i8)
        .write_i8((dz * 32.0).round() as i8)
        .write_u8(((yaw / 360.0) * 256.0) as u8)
        .write_u8(0);
    connection
        .write_packet(&Packet::new(
            play::clientbound::ENTITY_LOOK_AND_RELATIVE_MOVE,
            w.into_bytes(),
        ))
        .await
}

/// `S19` head yaw.
async fn send_bot_head(connection: &mut Connection, id: i32, yaw: f32) -> Result<()> {
    let mut w = PacketWriter::new();
    w.write_varint(id).write_u8(((yaw / 360.0) * 256.0) as u8);
    connection
        .write_packet(&Packet::new(
            play::clientbound::ENTITY_HEAD_LOOK,
            w.into_bytes(),
        ))
        .await
}

async fn send_bot_spawns(connection: &mut Connection, bots: &[MockBot]) -> Result<()> {
    for bot in bots {
        let (x, z) = bot_position(bot, bot.phase);
        let y = bot.feet_y;
        // Face the orbit's tangent direction.
        let yaw = ((bot.phase + std::f64::consts::FRAC_PI_2).to_degrees() as f32).rem_euclid(360.0);
        send_spawn_bot(connection, bot.entity_id, &bot.uuid, bot.name, x, y, z, yaw).await?;
    }
    Ok(())
}

/// Walk the bots around their circles and turn their heads to face the player.
async fn send_bot_tick(connection: &mut Connection, bots: &[MockBot], tick: u64) -> Result<()> {
    let step = 0.06;
    for bot in bots {
        let angle = bot.phase + step * tick as f64;
        let (x, z) = bot_position(bot, angle);
        let (px, pz) = bot_position(bot, angle - step);
        send_bot_move(
            connection,
            bot.entity_id,
            x - px,
            0.0,
            z - pz,
            (angle + std::f64::consts::FRAC_PI_2).to_degrees() as f32,
        )
        .await?;
        // Look back towards the player at (8.5, 8.5).
        let to_player = (8.5 - x, 8.5 - z);
        let facing = to_player.1.atan2(to_player.0).to_degrees() as f32 + 90.0;
        send_bot_head(connection, bot.entity_id, facing.rem_euclid(360.0)).await?;
    }
    Ok(())
}

/// Tell the client one block changed (`S23`).
async fn send_block_change(
    connection: &mut Connection,
    x: i32,
    y: i32,
    z: i32,
    state: u16,
) -> Result<()> {
    let mut w = PacketWriter::new();
    play::write_position(&mut w, x, y, z);
    w.write_varint(state as i32);
    connection
        .write_packet(&Packet::new(
            play::clientbound::BLOCK_CHANGE,
            w.into_bytes(),
        ))
        .await
}

async fn play_loop(
    connection: &mut Connection,
    username: &str,
    world: &mut World,
    bots_enabled: bool,
) -> Result<()> {
    let bots = make_bots(0.0);
    let mut interval = tokio::time::interval(KEEP_ALIVE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut keep_alive_id: i32 = 0;
    // Bots move once per server tick (20 Hz), like a real mob.
    let mut bot_ticker = tokio::time::interval(Duration::from_millis(50));
    bot_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut bot_tick: u64 = 0;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let mut w = PacketWriter::new();
                w.write_varint(keep_alive_id);
                keep_alive_id = keep_alive_id.wrapping_add(1);
                connection
                    .write_packet(&Packet::new(play::clientbound::KEEP_ALIVE, w.into_bytes()))
                    .await?;
            }
            _ = bot_ticker.tick(), if bots_enabled => {
                send_bot_tick(connection, &bots, bot_tick).await?;
                bot_tick = bot_tick.wrapping_add(1);
            }
            packet = connection.read_packet() => {
                match packet {
                    Ok(packet) => match packet.id {
                        play::serverbound::CHAT_MESSAGE => {
                            let text = packet.reader().read_string()?;
                            send_chat(connection, &format!("<{username}> {text}")).await?;
                        }
                        play::serverbound::PLAYER_DIGGING => {
                            let mut r = packet.reader();
                            let status = r.read_varint()? as u8;
                            let (x, y, z) = r.read_position()?;
                            let _face = r.read_u8()?;
                            // Creative clients destroy the block themselves and
                            // still send START, so apply on either action.
                            let breaks = status == play::digging::START_DESTROY_BLOCK
                                || status == play::digging::STOP_DESTROY_BLOCK;
                            if breaks && !world.get_block(x, y, z).is_air() {
                                world.set_block(x, y, z, BlockState::AIR);
                                send_block_change(connection, x, y, z, 0).await?;
                            }
                        }
                        play::serverbound::PLAYER_BLOCK_PLACEMENT => {
                            let mut r = packet.reader();
                            let (x, y, z) = r.read_position()?;
                            let face = r.read_u8()?;
                            let held = play::parse_slot(&mut r)?;
                            let _cursor = [r.read_u8()?, r.read_u8()?, r.read_u8()?];
                            if let Some(item) = held {
                                if let Some(state) = item.block_state() {
                                    let normal = Face::from_protocol_id(face)
                                        .map(|face| face.normal())
                                        .unwrap_or([0, 1, 0]);
                                    let (tx, ty, tz) =
                                        (x + normal[0], y + normal[1], z + normal[2]);
                                    // Only replace air (no block collision rules yet).
                                    if world.get_block(tx, ty, tz).is_air() {
                                        world.set_block(tx, ty, tz, BlockState::from_raw(state));
                                        send_block_change(connection, tx, ty, tz, state).await?;
                                    }
                                }
                            }
                        }
                        // Everything else (keep-alive echoes, settings,
                        // movement, creative inventory) is intentionally ignored.
                        _ => {}
                    },
                    Err(_) => return Ok(()),
                }
            }
        }
    }
}

async fn send_chat(connection: &mut Connection, text: &str) -> Result<()> {
    let json = serde_json_escape(text);
    let mut w = PacketWriter::new();
    w.write_string(&format!(r#"{{"text":"{json}"}}"#))
        .write_u8(0);
    connection
        .write_packet(&Packet::new(
            play::clientbound::CHAT_MESSAGE,
            w.into_bytes(),
        ))
        .await
}

/// Minimal JSON string escaping for the chat payloads we generate.
fn serde_json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Deterministic UUID so repeated joins are identical.
pub fn fake_uuid(username: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in username.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let other = hash.rotate_left(32) ^ 0x9e37_79b9_7f4a_7c15;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (hash >> 32) as u32,
        (hash >> 16) as u16,
        hash as u16,
        (other >> 48) as u16,
        other & 0xFFFF_FFFF_FFFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_is_stable_and_shaped() {
        let a = fake_uuid("Nephele");
        let b = fake_uuid("Nephele");
        assert_eq!(a, b);
        assert_ne!(a, fake_uuid("Other"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
    }

    #[test]
    fn chat_json_escapes_quotes() {
        assert_eq!(serde_json_escape(r#"say "hi""#), r#"say \"hi\""#);
    }
}
