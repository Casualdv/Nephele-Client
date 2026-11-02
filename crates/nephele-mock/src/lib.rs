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
    /// Join as survival instead of creative, so mining timing and drops are
    /// exercised instead of the creative one-click path.
    pub survival: bool,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            radius: 2,
            compression_threshold: Some(64),
            greeting: true,
            bots: false,
            survival: false,
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
    play_loop(connection, &username, &mut world, &config).await
}

async fn send_join_sequence(
    connection: &mut Connection,
    config: &MockConfig,
) -> Result<Vec<nephele_world::ChunkColumn>> {
    // Join Game (creative or survival, overworld, survival difficulty, 20 slots)
    let mut w = PacketWriter::new();
    w.write_i32(1)
        .write_u8(if config.survival { 0 } else { 1 })
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

/// How many bots `MockConfig::bots` spawns. Reported by the CLI on startup.
pub const BOT_COUNT: usize = 8;

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

/// The mock always gives the player entity id 1 (see the Join Game packet).
const PLAYER_ENTITY_ID: i32 = 1;
/// Player container slots: crafting and armour, then storage, then the hotbar.
const PLAY_CONTAINER_SLOTS: usize = play::PLAYER_CONTAINER_SLOTS;
/// A dropped item is picked up this long after the block breaks.
const PICKUP_DELAY: Duration = Duration::from_millis(1000);
/// Stack size ceiling for merges, shared by pickups and container clicks.
const MAX_STACK: i8 = 64;

struct PendingPickup {
    due: tokio::time::Instant,
    entity_id: i32,
    item: play::ItemStack,
}

/// First empty hotbar slot, then storage, then the crafting/armour slots — so
/// a pickup shows up in the player's hand rather than somewhere invisible.
fn first_free_slot(inventory: &[Option<play::ItemStack>; PLAY_CONTAINER_SLOTS]) -> Option<usize> {
    (play::HOTBAR_FIRST_SLOT..PLAY_CONTAINER_SLOTS)
        .chain(9..PLAY_CONTAINER_SLOTS - play::HOTBAR_SLOTS)
        .chain(0..9)
        .find(|&slot| inventory[slot].is_none())
}

// ---- container clicks -------------------------------------------------------

/// Player-container slot numbers used by the click handler (`C0E`).
const CRAFT_RESULT_SLOT: usize = 0;
const CRAFT_GRID_FIRST: usize = 1;
const CRAFT_GRID_SLOTS: usize = 4;
const STORAGE_FIRST_SLOT: usize = 9;
const STORAGE_LAST_SLOT: usize = play::HOTBAR_FIRST_SLOT;

const LOG_ITEM: i16 = 17;
const PLANKS_ITEM: i16 = 5;
const STICK_ITEM: i16 = 280;
const CRAFTING_TABLE_ITEM: i16 = 58;

/// What one click did. `changed` lists the slots whose contents moved so the
/// caller can mirror them to the client with `S2F`.
struct ClickResult {
    accepted: bool,
    changed: Vec<usize>,
}

fn same_item(a: &play::ItemStack, b: &play::ItemStack) -> bool {
    a.id == b.id && a.damage == b.damage
}

fn mark(changed: &mut Vec<usize>, slot: usize) {
    if !changed.contains(&slot) {
        changed.push(slot);
    }
}

/// The 2x2 craft the player container offers. Vanilla keys recipes off item
/// ids and ignores stack sizes, so one cell holding three logs still yields
/// planks (consuming one log per craft).
fn craft_output(
    inventory: &[Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
) -> Option<play::ItemStack> {
    let cells: [Option<play::ItemStack>; CRAFT_GRID_SLOTS] =
        std::array::from_fn(|index| inventory[CRAFT_GRID_FIRST + index]);
    let items: Vec<play::ItemStack> = cells.iter().flatten().copied().collect();

    // One log anywhere -> four planks of the matching wood type.
    if items.len() == 1 && items[0].id == LOG_ITEM {
        return Some(play::ItemStack::new(
            PLANKS_ITEM,
            4,
            items[0].damage.clamp(0, 3),
        ));
    }

    if items.is_empty() || items.iter().any(|item| item.id != PLANKS_ITEM) {
        return None;
    }
    if items.iter().any(|item| item.damage != items[0].damage) {
        return None;
    }

    // Four planks -> a crafting table (the 3x3 screen is a later slice).
    if items.len() == 4 {
        return Some(play::ItemStack::new(CRAFTING_TABLE_ITEM, 1, 0));
    }

    // Two planks in one column -> sticks.
    let left_column = cells[0].is_some() && cells[2].is_some();
    let right_column = cells[1].is_some() && cells[3].is_some();
    if items.len() == 2 && (left_column || right_column) {
        return Some(play::ItemStack::new(STICK_ITEM, 4, 0));
    }

    None
}

/// Spend one item from every occupied crafting cell.
fn consume_craft_grid(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    changed: &mut Vec<usize>,
) {
    for (index, slot) in inventory
        .iter_mut()
        .enumerate()
        .skip(CRAFT_GRID_FIRST)
        .take(CRAFT_GRID_SLOTS)
    {
        if let Some(mut stack) = *slot {
            stack.count -= 1;
            *slot = if stack.count <= 0 { None } else { Some(stack) };
            mark(changed, index);
        }
    }
}

/// Keep slot 0 showing what the grid would produce.
fn refresh_craft_output(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    changed: &mut Vec<usize>,
) {
    let preview = craft_output(inventory);
    if inventory[CRAFT_RESULT_SLOT] != preview {
        inventory[CRAFT_RESULT_SLOT] = preview;
        mark(changed, CRAFT_RESULT_SLOT);
    }
}

/// Move a whole stack into the first slot in `targets` that has room, merging
/// into matching stacks before using empty slots.
fn quick_move(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    from: usize,
    targets: std::ops::Range<usize>,
    changed: &mut Vec<usize>,
) -> bool {
    let Some(mut stack) = inventory[from] else {
        return false;
    };
    let original = stack.count;
    for target in targets {
        match inventory[target] {
            None => {
                inventory[target] = Some(stack);
                inventory[from] = None;
                mark(changed, from);
                mark(changed, target);
                return true;
            }
            Some(mut existing) if same_item(&existing, &stack) && existing.count < MAX_STACK => {
                let take = stack.count.min(MAX_STACK - existing.count);
                existing.count += take;
                inventory[target] = Some(existing);
                stack.count -= take;
                mark(changed, target);
                if stack.count == 0 {
                    inventory[from] = None;
                    mark(changed, from);
                    return true;
                }
            }
            _ => {}
        }
    }
    // Partially merged into one or more stacks: keep the remainder where it was.
    if stack.count != original {
        inventory[from] = Some(stack);
        mark(changed, from);
        return true;
    }
    false
}

/// Put `stack` somewhere in the player's storage, hotbar first.
fn insert_into(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    stack: play::ItemStack,
    changed: &mut Vec<usize>,
) -> bool {
    let targets = (play::HOTBAR_FIRST_SLOT..PLAY_CONTAINER_SLOTS)
        .chain(STORAGE_FIRST_SLOT..STORAGE_LAST_SLOT);
    for target in targets {
        match inventory[target] {
            None => {
                inventory[target] = Some(stack);
                mark(changed, target);
                return true;
            }
            Some(mut existing)
                if same_item(&existing, &stack) && existing.count + stack.count <= MAX_STACK =>
            {
                existing.count += stack.count;
                inventory[target] = Some(existing);
                mark(changed, target);
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Resolve one `C0E` against the server's own slots and cursor.
///
/// Supports normal left/right clicks, shift-click quick moves and number-key
/// swaps; drag and double-click modes are refused, which the client reads as
/// "do not trust the prediction".
fn click_container(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    cursor: &mut Option<play::ItemStack>,
    slot: i16,
    button: i8,
    mode: i8,
) -> ClickResult {
    let mut changed = Vec::new();

    // Dropping the cursor outside the window: the mock voids the stack instead
    // of spawning an item entity.
    if slot == play::SLOT_OUTSIDE {
        *cursor = None;
        return ClickResult {
            accepted: true,
            changed,
        };
    }
    let Ok(index) = usize::try_from(slot) else {
        return ClickResult {
            accepted: false,
            changed,
        };
    };
    if index >= PLAY_CONTAINER_SLOTS {
        return ClickResult {
            accepted: false,
            changed,
        };
    }

    match mode {
        play::click_mode::CLICK => {
            if index == CRAFT_RESULT_SLOT {
                if let Some(result) = craft_output(inventory) {
                    match cursor.as_mut() {
                        None => {
                            *cursor = Some(result);
                            consume_craft_grid(inventory, &mut changed);
                        }
                        Some(held) if same_item(held, &result) => {
                            if held.count + result.count <= MAX_STACK {
                                held.count += result.count;
                                consume_craft_grid(inventory, &mut changed);
                            }
                        }
                        Some(_) => {}
                    }
                }
            } else if button == 0 {
                match (*cursor, inventory[index]) {
                    (None, Some(stack)) => {
                        *cursor = Some(stack);
                        inventory[index] = None;
                        mark(&mut changed, index);
                    }
                    (Some(held), None) => {
                        inventory[index] = Some(held);
                        *cursor = None;
                        mark(&mut changed, index);
                    }
                    (Some(held), Some(on_slot))
                        if same_item(&held, &on_slot) && on_slot.count < MAX_STACK =>
                    {
                        let take = held.count.min(MAX_STACK - on_slot.count);
                        let mut merged = on_slot;
                        merged.count += take;
                        inventory[index] = Some(merged);
                        let left = held.count - take;
                        *cursor = if left == 0 {
                            None
                        } else {
                            Some(play::ItemStack::new(held.id, left, held.damage))
                        };
                        mark(&mut changed, index);
                    }
                    (Some(held), Some(on_slot)) => {
                        inventory[index] = Some(held);
                        *cursor = Some(on_slot);
                        mark(&mut changed, index);
                    }
                    (None, None) => {}
                }
            } else {
                // Right click: take half, or place one.
                match (*cursor, inventory[index]) {
                    (None, Some(stack)) if stack.count > 0 => {
                        let take = (stack.count + 1) / 2;
                        let mut moved = stack;
                        moved.count = take;
                        inventory[index] = if stack.count == take {
                            None
                        } else {
                            Some(play::ItemStack::new(
                                stack.id,
                                stack.count - take,
                                stack.damage,
                            ))
                        };
                        *cursor = Some(moved);
                        mark(&mut changed, index);
                    }
                    (Some(held), None) => {
                        let mut placed = held;
                        placed.count = 1;
                        inventory[index] = Some(placed);
                        *cursor = if held.count <= 1 {
                            None
                        } else {
                            Some(play::ItemStack::new(held.id, held.count - 1, held.damage))
                        };
                        mark(&mut changed, index);
                    }
                    (Some(held), Some(on_slot))
                        if same_item(&held, &on_slot) && on_slot.count < MAX_STACK =>
                    {
                        let mut merged = on_slot;
                        merged.count += 1;
                        inventory[index] = Some(merged);
                        *cursor = if held.count <= 1 {
                            None
                        } else {
                            Some(play::ItemStack::new(held.id, held.count - 1, held.damage))
                        };
                        mark(&mut changed, index);
                    }
                    _ => {}
                }
            }
        }
        play::click_mode::SHIFT_CLICK => {
            if index == CRAFT_RESULT_SLOT {
                if let Some(result) = craft_output(inventory) {
                    if insert_into(inventory, result, &mut changed) {
                        consume_craft_grid(inventory, &mut changed);
                    }
                }
            } else if (play::HOTBAR_FIRST_SLOT..PLAY_CONTAINER_SLOTS).contains(&index) {
                quick_move(
                    inventory,
                    index,
                    STORAGE_FIRST_SLOT..STORAGE_LAST_SLOT,
                    &mut changed,
                );
            } else {
                quick_move(
                    inventory,
                    index,
                    play::HOTBAR_FIRST_SLOT..PLAY_CONTAINER_SLOTS,
                    &mut changed,
                );
            }
        }
        play::click_mode::NUMBER_KEY if button >= 0 && (button as usize) < play::HOTBAR_SLOTS => {
            let hotbar = play::HOTBAR_FIRST_SLOT + button as usize;
            if index != hotbar {
                match (inventory[index], inventory[hotbar]) {
                    (Some(a), Some(b)) if same_item(&a, &b) && a.count + b.count <= MAX_STACK => {
                        let mut merged = b;
                        merged.count += a.count;
                        inventory[hotbar] = Some(merged);
                        inventory[index] = None;
                        mark(&mut changed, index);
                        mark(&mut changed, hotbar);
                    }
                    (Some(a), other) => {
                        inventory[index] = other;
                        inventory[hotbar] = Some(a);
                        mark(&mut changed, index);
                        mark(&mut changed, hotbar);
                    }
                    (None, Some(b)) => {
                        inventory[index] = Some(b);
                        inventory[hotbar] = None;
                        mark(&mut changed, index);
                        mark(&mut changed, hotbar);
                    }
                    (None, None) => {}
                }
            }
        }
        _ => {
            return ClickResult {
                accepted: false,
                changed,
            };
        }
    }

    if (CRAFT_GRID_FIRST..CRAFT_GRID_FIRST + CRAFT_GRID_SLOTS).contains(&index)
        || index == CRAFT_RESULT_SLOT
    {
        refresh_craft_output(inventory, &mut changed);
    }

    ClickResult {
        accepted: true,
        changed,
    }
}

// ---- crafting table ---------------------------------------------------------

/// Slots in a crafting-table window: result, 3x3 grid, 27 storage, 9 hotbar.
const TABLE_SLOTS: usize = 46;

fn table_window(
    inventory: &[Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
) -> Vec<Option<play::ItemStack>> {
    let mut slots = vec![None; TABLE_SLOTS];
    slots[10..37].copy_from_slice(&inventory[9..36]);
    slots[37..46].copy_from_slice(&inventory[36..45]);
    slots
}

fn table_to_inventory(
    table: &[Option<play::ItemStack>],
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
) {
    inventory[9..36].copy_from_slice(&table[10..37]);
    inventory[36..45].copy_from_slice(&table[37..46]);
}

/// Put `stack` back somewhere in the player's storage, merging first.
fn add_to_inventory(
    inventory: &mut [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS],
    stack: play::ItemStack,
) {
    for slot in
        (play::HOTBAR_FIRST_SLOT..PLAY_CONTAINER_SLOTS).chain(STORAGE_FIRST_SLOT..STORAGE_LAST_SLOT)
    {
        match inventory[slot] {
            None => {
                inventory[slot] = Some(stack);
                return;
            }
            Some(mut existing)
                if same_item(&existing, &stack) && existing.count + stack.count <= MAX_STACK =>
            {
                existing.count += stack.count;
                inventory[slot] = Some(existing);
                return;
            }
            _ => {}
        }
    }
}

/// Recipes the mock knows how to make in a 3x3 grid. `0` is an empty cell.
/// Rows are trimmed of empty edges before comparing, so a shape can sit
/// anywhere in the grid like vanilla.
fn table_recipe(grid: &[Option<play::ItemStack>]) -> Option<play::ItemStack> {
    // The table itself lives in nephele-world so singleplayer and the mock
    // cannot drift apart; this only adapts the protocol's stack type to it.
    let mut cells: [Option<(u16, u16)>; 9] = [None; 9];
    for (slot, cell) in grid.iter().take(9).enumerate() {
        if let Some(item) = cell {
            cells[slot] = Some((item.id.max(0) as u16, item.damage.max(0) as u16));
        }
    }
    nephele_world::crafting::craft(&cells).map(|recipe| {
        play::ItemStack::new(recipe.id as i16, recipe.count as i8, recipe.damage as i16)
    })
}

/// Spend one item from every occupied grid cell (slots 1..=9 of a window).
fn spend_grid(table: &mut [Option<play::ItemStack>], changed: &mut Vec<usize>) {
    for (index, cell) in table.iter_mut().enumerate().skip(1).take(9) {
        if let Some(mut stack) = *cell {
            stack.count -= 1;
            *cell = (stack.count > 0).then_some(stack);
            mark(changed, index);
        }
    }
}

/// One click in the crafting-table window.
fn click_table(
    table: &mut [Option<play::ItemStack>],
    cursor: &mut Option<play::ItemStack>,
    slot: i16,
    button: i8,
    mode: i8,
) -> ClickResult {
    let mut changed = Vec::new();
    let Ok(index) = usize::try_from(slot) else {
        return ClickResult {
            accepted: false,
            changed,
        };
    };
    if index >= TABLE_SLOTS {
        return ClickResult {
            accepted: false,
            changed,
        };
    }
    let grid: Vec<Option<play::ItemStack>> = table[1..10].to_vec();

    if mode == play::click_mode::CLICK && index == 0 {
        if let Some(result) = table_recipe(&grid) {
            match cursor.as_mut() {
                None => {
                    *cursor = Some(result);
                    spend_grid(table, &mut changed);
                }
                Some(held) if same_item(held, &result) => {
                    if held.count + result.count <= MAX_STACK {
                        held.count += result.count;
                        spend_grid(table, &mut changed);
                    }
                }
                Some(_) => {}
            }
        }
    } else if mode == play::click_mode::SHIFT_CLICK {
        // Quick move: output crafts into the player half; storage <-> hotbar.
        if index == 0 {
            if let Some(result) = table_recipe(&grid) {
                if insert_into_window(table, result, &mut changed) {
                    spend_grid(table, &mut changed);
                }
            }
        } else if (10..37).contains(&index) {
            quick_move_table(table, index, 37..46, &mut changed);
        } else {
            quick_move_table(table, index, 10..37, &mut changed);
        }
    } else {
        let (new_cursor, new_slot) = click_pair(*cursor, table[index], button);
        *cursor = new_cursor;
        if new_slot != table[index] {
            table[index] = new_slot;
            mark(&mut changed, index);
        }
    }

    // Keep the result preview in step with the grid.
    let preview = table_recipe(&table[1..10]);
    if table[0] != preview {
        table[0] = preview;
        mark(&mut changed, 0);
    }
    ClickResult {
        accepted: true,
        changed,
    }
}

fn insert_into_window(
    table: &mut [Option<play::ItemStack>],
    stack: play::ItemStack,
    changed: &mut Vec<usize>,
) -> bool {
    for target in (37..46).chain(10..37) {
        match table[target] {
            None => {
                table[target] = Some(stack);
                mark(changed, target);
                return true;
            }
            Some(mut existing)
                if same_item(&existing, &stack) && existing.count + stack.count <= MAX_STACK =>
            {
                existing.count += stack.count;
                table[target] = Some(existing);
                mark(changed, target);
                return true;
            }
            _ => {}
        }
    }
    false
}

fn quick_move_table(
    table: &mut [Option<play::ItemStack>],
    from: usize,
    targets: std::ops::Range<usize>,
    changed: &mut Vec<usize>,
) {
    let Some(mut stack) = table[from] else {
        return;
    };
    let original = stack.count;
    for target in targets {
        match table[target] {
            None => {
                table[target] = Some(stack);
                table[from] = None;
                mark(changed, from);
                mark(changed, target);
                return;
            }
            Some(mut existing) if same_item(&existing, &stack) && existing.count < MAX_STACK => {
                let take = stack.count.min(MAX_STACK - existing.count);
                existing.count += take;
                table[target] = Some(existing);
                stack.count -= take;
                mark(changed, target);
                if stack.count == 0 {
                    table[from] = None;
                    mark(changed, from);
                    return;
                }
            }
            _ => {}
        }
    }
    if stack.count != original {
        table[from] = Some(stack);
        mark(changed, from);
    }
}

/// The cursor half of a normal click, shared by the player window and the table.
fn click_pair(
    cursor: Option<play::ItemStack>,
    slot: Option<play::ItemStack>,
    button: i8,
) -> (Option<play::ItemStack>, Option<play::ItemStack>) {
    match (cursor, slot, button) {
        (None, Some(on_slot), 0) => (Some(on_slot), None),
        (Some(held), None, 0) => (None, Some(held)),
        (Some(held), Some(on_slot), 0)
            if same_item(&held, &on_slot) && on_slot.count < MAX_STACK =>
        {
            let take = held.count.min(MAX_STACK - on_slot.count);
            let mut merged = on_slot;
            merged.count += take;
            let left = held.count - take;
            let cursor = (left > 0).then_some(play::ItemStack::new(held.id, left, held.damage));
            (cursor, Some(merged))
        }
        (Some(held), Some(on_slot), 0) => (Some(on_slot), Some(held)),
        // Right click: take half or place one.
        (None, Some(on_slot), _) if on_slot.count > 0 => {
            let take = (on_slot.count + 1) / 2;
            let left = on_slot.count - take;
            let moved = play::ItemStack::new(on_slot.id, take, on_slot.damage);
            let slot = (left > 0).then_some(play::ItemStack::new(on_slot.id, left, on_slot.damage));
            (Some(moved), slot)
        }
        (Some(held), _, _) if held.count > 0 => {
            let left = held.count - 1;
            let cursor = (left > 0).then_some(play::ItemStack::new(held.id, left, held.damage));
            let placed = play::ItemStack::new(held.id, 1, held.damage);
            let slot = match slot {
                Some(on_slot) if same_item(&held, &on_slot) && on_slot.count < MAX_STACK => {
                    let mut merged = on_slot;
                    merged.count += 1;
                    Some(merged)
                }
                None => Some(placed),
                other => other,
            };
            (cursor, slot)
        }
        (cursor, slot, _) => (cursor, slot),
    }
}

/// Drop `block` as an item entity where it was broken, and schedule its pickup.
///
/// Real servers pop the drop out with a little velocity and let gravity settle
/// it; the mock puts it straight on the ground, which is enough to exercise the
/// client's spawn, interpolation, render and collect paths.
async fn send_drop(
    connection: &mut Connection,
    entity_id: i32,
    block: BlockState,
    x: i32,
    y: i32,
    z: i32,
    pending: &mut std::collections::VecDeque<PendingPickup>,
) -> Result<()> {
    let item = play::ItemStack::new(block.id() as i16, 1, block.meta() as i16);
    connection
        .write_packet(&play::spawn_object(
            entity_id,
            play::OBJECT_TYPE_ITEM,
            [x as f64 + 0.5, y as f64, z as f64 + 0.5],
            0.0,
            0.0,
            play::item_object_data(item.id, item.damage),
            [0.0; 3],
        ))
        .await?;
    pending.push_back(PendingPickup {
        due: tokio::time::Instant::now() + PICKUP_DELAY,
        entity_id,
        item,
    });
    Ok(())
}

async fn play_loop(
    connection: &mut Connection,
    username: &str,
    world: &mut World,
    config: &MockConfig,
) -> Result<()> {
    let bots = make_bots(0.0);
    // Everything the player has collected, by container slot (36..45 is the
    // hotbar). Pickups land here and are mirrored to the client as SET_SLOT.
    let mut inventory: [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS] =
        [None; PLAY_CONTAINER_SLOTS];
    // The stack the server thinks is attached to the player's cursor. `C0E`
    // does not carry it; the client only reports the slot it clicked.
    let mut cursor: Option<play::ItemStack> = None;
    // The open crafting-table window (id 1), if any.
    let mut table: Option<Vec<Option<play::ItemStack>>> = None;
    let mut next_drop_id: i32 = 1_000;
    let mut picks_up_at: std::collections::VecDeque<PendingPickup> =
        std::collections::VecDeque::new();
    let mut interval = tokio::time::interval(KEEP_ALIVE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut keep_alive_id: i32 = 0;
    // Bots move once per server tick (20 Hz), like a real mob.
    let mut bot_ticker = tokio::time::interval(Duration::from_millis(50));
    bot_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut bot_tick: u64 = 0;
    // Dropped items are picked up a moment after they spawn; a hundred
    // milliseconds is plenty for a demo server.
    let mut pickup_ticker = tokio::time::interval(Duration::from_millis(100));
    pickup_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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
            _ = pickup_ticker.tick(), if !picks_up_at.is_empty() => {
                let now = tokio::time::Instant::now();
                while picks_up_at.front().is_some_and(|drop| drop.due <= now) {
                    let drop = picks_up_at.pop_front().expect("front checked above");
                    connection
                        .write_packet(&play::collect_item(drop.entity_id, PLAYER_ENTITY_ID))
                        .await?;
                    if let Some(slot) = first_free_slot(&inventory) {
                        inventory[slot] = Some(drop.item);
                        connection
                            .write_packet(&play::set_slot(0, slot as i16, Some(&drop.item)))
                            .await?;
                    }
                }
            }
            _ = bot_ticker.tick(), if config.bots => {
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
                                let block = world.get_block(x, y, z);
                                world.set_block(x, y, z, BlockState::AIR);
                                send_block_change(connection, x, y, z, 0).await?;
                                send_drop(
                                    connection,
                                    next_drop_id,
                                    block,
                                    x,
                                    y,
                                    z,
                                    &mut picks_up_at,
                                )
                                .await?;
                                next_drop_id += 1;
                            }
                        }
                        play::serverbound::PLAYER_BLOCK_PLACEMENT => {
                            let mut r = packet.reader();
                            let (x, y, z) = r.read_position()?;
                            let face = r.read_u8()?;
                            let held = play::parse_slot(&mut r)?;
                            let _cursor = [r.read_u8()?, r.read_u8()?, r.read_u8()?];
                            // Right-clicking a crafting table opens the 3x3 GUI.
                            if world.get_block(x, y, z).id() as i16 == CRAFTING_TABLE_ITEM
                                && held.and_then(|item| item.block_state()).is_none()
                            {
                                let slots = table_window(&inventory);
                                connection
                                    .write_packet(&play::open_window(
                                        1,
                                        "minecraft:crafting_table",
                                        "{\"text\":\"Crafting\"}",
                                        TABLE_SLOTS as u8,
                                    ))
                                    .await?;
                                connection
                                    .write_packet(&play::window_items(1, &slots))
                                    .await?;
                                table = Some(slots);
                                continue;
                            }
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
                        play::serverbound::CREATIVE_INVENTORY_ACTION => {
                            let mut r = packet.reader();
                            let slot = r.read_i16()?;
                            let item = play::parse_slot(&mut r)?;
                            if slot >= 0 && (slot as usize) < PLAY_CONTAINER_SLOTS {
                                let index = slot as usize;
                                inventory[index] = item;
                                connection
                                    .write_packet(&play::set_slot(0, slot, item.as_ref()))
                                    .await?;
                            }
                        }
                        play::serverbound::CLICK_WINDOW => {
                            let mut r = packet.reader();
                            let window_id = r.read_u8()?;
                            let slot = r.read_i16()?;
                            let button = r.read_i8()?;
                            let action = r.read_i16()?;
                            let mode = r.read_i8()?;
                            let clicked = play::parse_slot(&mut r)?;

                            if window_id == 1 {
                                let Some(slots) = table.as_mut() else {
                                    continue;
                                };
                                let stale = usize::try_from(slot)
                                    .ok()
                                    .filter(|&index| index < TABLE_SLOTS)
                                    .is_some_and(|index| slots[index] != clicked);
                                let mut accepted = !stale;
                                let mut changed = Vec::new();
                                if accepted {
                                    let outcome =
                                        click_table(slots, &mut cursor, slot, button, mode);
                                    accepted = outcome.accepted;
                                    changed = outcome.changed;
                                }
                                table_to_inventory(slots, &mut inventory);
                                for index in changed {
                                    let item = slots[index];
                                    connection
                                        .write_packet(&play::set_slot(1, index as i16, item.as_ref()))
                                        .await?;
                                }
                                if !accepted {
                                    if let Ok(index) = usize::try_from(slot) {
                                        if index < TABLE_SLOTS {
                                            let item = slots[index];
                                            connection
                                                .write_packet(&play::set_slot(
                                                    1,
                                                    slot,
                                                    item.as_ref(),
                                                ))
                                                .await?;
                                        }
                                    }
                                }
                                connection
                                    .write_packet(&play::confirm_transaction(1, action, accepted))
                                    .await?;
                                continue;
                            }

                            // The client reports its copy of the clicked slot.
                            // When ours differs the click is stale, so resend
                            // that slot and refuse to apply it.
                            let mut accepted = window_id == 0;
                            if accepted {
                                if let Ok(index) = usize::try_from(slot) {
                                    if index < PLAY_CONTAINER_SLOTS
                                        && inventory[index] != clicked
                                    {
                                        accepted = false;
                                    }
                                }
                            }

                            let mut changed = Vec::new();
                            if accepted {
                                let outcome =
                                    click_container(&mut inventory, &mut cursor, slot, button, mode);
                                accepted = outcome.accepted;
                                changed = outcome.changed;
                            }
                            for index in changed {
                                let item = inventory[index];
                                connection
                                    .write_packet(&play::set_slot(0, index as i16, item.as_ref()))
                                    .await?;
                            }
                            if !accepted {
                                if let Ok(index) = usize::try_from(slot) {
                                    if index < PLAY_CONTAINER_SLOTS {
                                        let item = inventory[index];
                                        connection
                                            .write_packet(&play::set_slot(
                                                0,
                                                slot,
                                                item.as_ref(),
                                            ))
                                            .await?;
                                    }
                                }
                            }
                            connection
                                .write_packet(&play::confirm_transaction(0, action, accepted))
                                .await?;
                        }
                        play::serverbound::CLOSE_WINDOW => {
                            let id = packet.reader().read_u8()?;
                            if id == 1 {
                                if let Some(mut slots) = table.take() {
                                    // Return the grid and the cursor to the player.
                                    for cell in slots.iter_mut().skip(1).take(9) {
                                        if let Some(item) = cell.take() {
                                            add_to_inventory(&mut inventory, item);
                                        }
                                    }
                                    if let Some(item) = cursor.take() {
                                        add_to_inventory(&mut inventory, item);
                                    }
                                    connection
                                        .write_packet(&play::window_items(0, &inventory))
                                        .await?;
                                    connection
                                        .write_packet(&play::close_window(1, false))
                                        .await?;
                                }
                            }
                        }
                        // Everything else (keep-alive echoes, settings,
                        // movement) is intentionally ignored.
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

    /// Pickups go to the hotbar first, so the player sees what they collected.
    #[test]
    fn a_pickup_lands_in_the_first_free_hotbar_slot() {
        let mut inventory: [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS] =
            [None; PLAY_CONTAINER_SLOTS];
        assert_eq!(first_free_slot(&inventory), Some(play::HOTBAR_FIRST_SLOT));

        for slot in inventory
            .iter_mut()
            .take(PLAY_CONTAINER_SLOTS)
            .skip(play::HOTBAR_FIRST_SLOT)
        {
            *slot = Some(play::ItemStack::new(1, 1, 0));
        }
        // Hotbar full: storage starts at 9, not at 0.
        assert_eq!(first_free_slot(&inventory), Some(9));

        for slot in inventory.iter_mut().take(36).skip(9) {
            *slot = Some(play::ItemStack::new(1, 1, 0));
        }
        assert_eq!(first_free_slot(&inventory), Some(0));

        for slot in inventory.iter_mut().take(9) {
            *slot = Some(play::ItemStack::new(1, 1, 0));
        }
        assert_eq!(first_free_slot(&inventory), None);
    }

    #[test]
    fn the_survival_chain_reaches_a_pickaxe() {
        // M2's exit criterion, pinned as data: one log becomes planks, planks
        // become sticks and a table, and planks plus sticks make a pickaxe.
        let cell = |id: i16| Some(play::ItemStack::new(id, 1, 0));

        let mut log = vec![None; 9];
        log[4] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));
        let planks = table_recipe(&log).expect("a log makes planks");
        assert_eq!((planks.id, planks.count), (PLANKS_ITEM, 4));

        let mut sticks = vec![None; 9];
        sticks[0] = cell(PLANKS_ITEM);
        sticks[3] = cell(PLANKS_ITEM);
        assert_eq!(
            table_recipe(&sticks).map(|s| (s.id, s.count)),
            Some((STICK_ITEM, 4))
        );

        let table: Vec<Option<play::ItemStack>> = (0..9)
            .map(|i| {
                if matches!(i, 0 | 1 | 3 | 4) {
                    cell(PLANKS_ITEM)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            table_recipe(&table).map(|s| s.id),
            Some(CRAFTING_TABLE_ITEM)
        );

        let mut pickaxe = vec![None; 9];
        for slot in pickaxe.iter_mut().take(3) {
            *slot = cell(PLANKS_ITEM);
        }
        pickaxe[4] = cell(STICK_ITEM);
        pickaxe[7] = cell(STICK_ITEM);
        assert_eq!(table_recipe(&pickaxe).map(|s| s.id), Some(270));
    }
    #[test]
    fn the_3x3_table_knows_the_survival_recipes() {
        let cell = |id: i16| Some(play::ItemStack::new(id, 1, 0));
        // Pickaxe: three planks over two sticks.
        let mut grid = vec![None; 9];
        grid[0] = cell(PLANKS_ITEM);
        grid[1] = cell(PLANKS_ITEM);
        grid[2] = cell(PLANKS_ITEM);
        grid[4] = cell(STICK_ITEM);
        grid[7] = cell(STICK_ITEM);
        assert_eq!(table_recipe(&grid), Some(play::ItemStack::new(270, 1, 0)));

        // Two planks in one column, shifted right, still make sticks.
        let mut shifted = vec![None; 9];
        shifted[1] = cell(PLANKS_ITEM);
        shifted[4] = cell(PLANKS_ITEM);
        assert_eq!(
            table_recipe(&shifted),
            Some(play::ItemStack::new(280, 4, 0))
        );

        // Torch, furnace and chest.
        let mut torch = vec![None; 9];
        torch[1] = cell(263);
        torch[4] = cell(STICK_ITEM);
        assert_eq!(table_recipe(&torch), Some(play::ItemStack::new(50, 4, 0)));

        let cobble: Vec<Option<play::ItemStack>> = (0..9)
            .map(|i| if i == 4 { None } else { cell(4) })
            .collect();
        assert_eq!(table_recipe(&cobble), Some(play::ItemStack::new(61, 1, 0)));

        // Malformed shapes do not craft.
        let mut wrong = vec![None; 9];
        wrong[0] = cell(STICK_ITEM);
        wrong[1] = cell(STICK_ITEM);
        assert_eq!(table_recipe(&wrong), None);
    }

    #[test]
    fn clicking_the_result_crafts_and_spends_the_grid() {
        let mut table = vec![None; TABLE_SLOTS];
        for cell in [1, 2, 3] {
            table[cell] = Some(play::ItemStack::new(PLANKS_ITEM, 2, 0));
        }
        table[5] = Some(play::ItemStack::new(STICK_ITEM, 2, 0));
        table[8] = Some(play::ItemStack::new(STICK_ITEM, 2, 0));
        let mut cursor = None;

        let result = click_table(&mut table, &mut cursor, 0, 0, play::click_mode::CLICK);
        assert!(result.accepted);
        assert_eq!(cursor, Some(play::ItemStack::new(270, 1, 0)));
        assert_eq!(table[1], Some(play::ItemStack::new(PLANKS_ITEM, 1, 0)));
        assert_eq!(
            table[5],
            Some(play::ItemStack::new(STICK_ITEM, 1, 0)),
            "one stick per craft is spent"
        );
        assert_eq!(
            table[0],
            Some(play::ItemStack::new(270, 1, 0)),
            "the preview is recomputed"
        );
    }

    #[test]
    fn chat_json_escapes_quotes() {
        assert_eq!(serde_json_escape(r#"say "hi""#), r#"say \"hi\""#);
    }

    fn empty_inventory() -> [Option<play::ItemStack>; PLAY_CONTAINER_SLOTS] {
        [None; PLAY_CONTAINER_SLOTS]
    }

    #[test]
    fn one_log_crafts_four_planks() {
        let mut inventory = empty_inventory();
        inventory[1] = Some(play::ItemStack::new(LOG_ITEM, 3, 2));
        assert_eq!(
            craft_output(&inventory),
            Some(play::ItemStack::new(PLANKS_ITEM, 4, 2)),
            "stack size does not change the recipe, and birch stays birch"
        );

        inventory[1] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));
        inventory[4] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));
        assert_eq!(craft_output(&inventory), None, "two logs is not a recipe");
    }

    #[test]
    fn two_planks_make_sticks_and_four_make_a_table() {
        let mut inventory = empty_inventory();
        inventory[1] = Some(play::ItemStack::new(PLANKS_ITEM, 1, 0));
        inventory[3] = Some(play::ItemStack::new(PLANKS_ITEM, 1, 0));
        assert_eq!(
            craft_output(&inventory),
            Some(play::ItemStack::new(STICK_ITEM, 4, 0))
        );

        // The same two planks played horizontally are not a recipe.
        inventory[3] = None;
        inventory[2] = Some(play::ItemStack::new(PLANKS_ITEM, 1, 0));
        assert_eq!(craft_output(&inventory), None);

        // Mixed wood types do not combine.
        inventory[1] = Some(play::ItemStack::new(PLANKS_ITEM, 1, 0));
        inventory[2] = Some(play::ItemStack::new(PLANKS_ITEM, 1, 1));
        assert_eq!(craft_output(&inventory), None);

        for slot in inventory
            .iter_mut()
            .take(CRAFT_GRID_FIRST + CRAFT_GRID_SLOTS)
            .skip(1)
        {
            *slot = Some(play::ItemStack::new(PLANKS_ITEM, 1, 1));
        }
        assert_eq!(
            craft_output(&inventory),
            Some(play::ItemStack::new(CRAFTING_TABLE_ITEM, 1, 0))
        );
    }

    #[test]
    fn clicking_moves_stacks_between_the_cursor_and_a_slot() {
        let mut inventory = empty_inventory();
        let mut cursor = None;
        inventory[36] = Some(play::ItemStack::new(LOG_ITEM, 5, 0));

        // Take the stack.
        let taken = click_container(&mut inventory, &mut cursor, 36, 0, play::click_mode::CLICK);
        assert!(taken.accepted);
        assert_eq!(cursor, Some(play::ItemStack::new(LOG_ITEM, 5, 0)));
        assert_eq!(inventory[36], None);

        // Put it in the crafting grid; the output slot updates.
        let placed = click_container(&mut inventory, &mut cursor, 1, 0, play::click_mode::CLICK);
        assert!(placed.accepted);
        assert!(placed.changed.contains(&1));
        assert!(placed.changed.contains(&CRAFT_RESULT_SLOT));
        assert_eq!(cursor, None);
        assert_eq!(inventory[1], Some(play::ItemStack::new(LOG_ITEM, 5, 0)));
        assert_eq!(
            inventory[CRAFT_RESULT_SLOT],
            Some(play::ItemStack::new(PLANKS_ITEM, 4, 0))
        );

        // Taking the result spends one log and leaves four planks on the cursor.
        click_container(
            &mut inventory,
            &mut cursor,
            CRAFT_RESULT_SLOT as i16,
            0,
            play::click_mode::CLICK,
        );
        assert_eq!(cursor, Some(play::ItemStack::new(PLANKS_ITEM, 4, 0)));
        assert_eq!(
            inventory[1],
            Some(play::ItemStack::new(LOG_ITEM, 4, 0)),
            "the grid loses one item per craft"
        );
        assert_eq!(
            inventory[CRAFT_RESULT_SLOT],
            Some(play::ItemStack::new(PLANKS_ITEM, 4, 0)),
            "the preview is recomputed for the next craft"
        );
    }

    #[test]
    fn right_click_takes_half_and_places_one() {
        let mut inventory = empty_inventory();
        let mut cursor = None;
        inventory[36] = Some(play::ItemStack::new(STICK_ITEM, 5, 0));

        click_container(&mut inventory, &mut cursor, 36, 1, play::click_mode::CLICK);
        assert_eq!(cursor, Some(play::ItemStack::new(STICK_ITEM, 3, 0)));
        assert_eq!(inventory[36], Some(play::ItemStack::new(STICK_ITEM, 2, 0)));

        click_container(&mut inventory, &mut cursor, 9, 1, play::click_mode::CLICK);
        assert_eq!(cursor, Some(play::ItemStack::new(STICK_ITEM, 2, 0)));
        assert_eq!(inventory[9], Some(play::ItemStack::new(STICK_ITEM, 1, 0)));
    }

    #[test]
    fn shift_click_moves_a_stack_to_the_other_region() {
        let mut inventory = empty_inventory();
        let mut cursor = None;
        inventory[36] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));

        let moved = click_container(
            &mut inventory,
            &mut cursor,
            36,
            0,
            play::click_mode::SHIFT_CLICK,
        );
        assert!(moved.accepted);
        assert_eq!(inventory[36], None);
        assert_eq!(inventory[9], Some(play::ItemStack::new(LOG_ITEM, 1, 0)));

        // And back again, into the hotbar.
        click_container(
            &mut inventory,
            &mut cursor,
            9,
            0,
            play::click_mode::SHIFT_CLICK,
        );
        assert_eq!(inventory[9], None);
        assert_eq!(
            inventory[play::HOTBAR_FIRST_SLOT],
            Some(play::ItemStack::new(LOG_ITEM, 1, 0))
        );
    }

    #[test]
    fn number_key_swaps_with_the_hotbar() {
        let mut inventory = empty_inventory();
        let mut cursor = None;
        inventory[9] = Some(play::ItemStack::new(STICK_ITEM, 2, 0));
        inventory[37] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));

        click_container(
            &mut inventory,
            &mut cursor,
            9,
            1,
            play::click_mode::NUMBER_KEY,
        );
        assert_eq!(inventory[9], Some(play::ItemStack::new(LOG_ITEM, 1, 0)));
        assert_eq!(inventory[37], Some(play::ItemStack::new(STICK_ITEM, 2, 0)));

        // Same item merges instead of swapping.
        inventory[9] = Some(play::ItemStack::new(STICK_ITEM, 1, 0));
        inventory[37] = Some(play::ItemStack::new(STICK_ITEM, 2, 0));
        click_container(
            &mut inventory,
            &mut cursor,
            9,
            1,
            play::click_mode::NUMBER_KEY,
        );
        assert_eq!(inventory[9], None);
        assert_eq!(inventory[37], Some(play::ItemStack::new(STICK_ITEM, 3, 0)));
    }

    #[test]
    fn unsupported_click_modes_are_refused() {
        let mut inventory = empty_inventory();
        let mut cursor = None;
        inventory[36] = Some(play::ItemStack::new(LOG_ITEM, 1, 0));
        let result = click_container(&mut inventory, &mut cursor, 36, 0, play::click_mode::DRAG);
        assert!(!result.accepted);
        assert!(result.changed.is_empty());
        assert_eq!(inventory[36], Some(play::ItemStack::new(LOG_ITEM, 1, 0)));
    }
}
