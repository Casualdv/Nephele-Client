use crate::error::Result;
use crate::packet::Packet;
use crate::packets::status::flatten_chat;
use crate::write::PacketWriter;

/// Clientbound packet ids for protocol 47 (Minecraft 1.8.9).
pub mod clientbound {
    pub const KEEP_ALIVE: i32 = 0x00;
    pub const JOIN_GAME: i32 = 0x01;
    pub const CHAT_MESSAGE: i32 = 0x02;
    pub const TIME_UPDATE: i32 = 0x03;
    pub const SPAWN_POSITION: i32 = 0x05;
    pub const UPDATE_HEALTH: i32 = 0x06;
    pub const RESPAWN: i32 = 0x07;
    pub const PLAYER_POSITION_AND_LOOK: i32 = 0x08;
    pub const HELD_ITEM_CHANGE: i32 = 0x09;
    pub const SPAWN_PLAYER: i32 = 0x0C;
    /// An entity was picked up or absorbed by another one.
    pub const COLLECT_ITEM: i32 = 0x0D;
    /// A non-living object: dropped items, arrows, falling blocks, ...
    pub const SPAWN_OBJECT: i32 = 0x0E;
    pub const SPAWN_MOB: i32 = 0x0F;
    pub const ENTITY_VELOCITY: i32 = 0x12;
    pub const DESTROY_ENTITIES: i32 = 0x13;
    pub const ENTITY: i32 = 0x14;
    pub const ENTITY_RELATIVE_MOVE: i32 = 0x15;
    pub const ENTITY_LOOK: i32 = 0x16;
    pub const ENTITY_LOOK_AND_RELATIVE_MOVE: i32 = 0x17;
    pub const ENTITY_TELEPORT: i32 = 0x18;
    pub const ENTITY_HEAD_LOOK: i32 = 0x19;
    pub const ENTITY_METADATA: i32 = 0x1C;
    pub const EXPERIENCE: i32 = 0x1F;
    pub const CHUNK_DATA: i32 = 0x21;
    pub const MULTI_BLOCK_CHANGE: i32 = 0x22;
    pub const BLOCK_CHANGE: i32 = 0x23;
    pub const BLOCK_BREAK_ANIMATION: i32 = 0x25;
    pub const MAP_CHUNK_BULK: i32 = 0x26;
    pub const SET_SLOT: i32 = 0x2F;
    pub const WINDOW_ITEMS: i32 = 0x30;
    pub const CONFIRM_TRANSACTION: i32 = 0x32;
    pub const PLAYER_LIST_ITEM: i32 = 0x38;
    pub const PLAYER_ABILITIES: i32 = 0x39;
    pub const DISCONNECT: i32 = 0x40;
    pub const SERVER_DIFFICULTY: i32 = 0x41;
    pub const SET_COMPRESSION: i32 = 0x46;
}

/// Serverbound packet ids for protocol 47 (Minecraft 1.8.9).
pub mod serverbound {
    pub const KEEP_ALIVE: i32 = 0x00;
    pub const CHAT_MESSAGE: i32 = 0x01;
    pub const PLAYER: i32 = 0x03;
    pub const PLAYER_POSITION: i32 = 0x04;
    pub const PLAYER_LOOK: i32 = 0x05;
    pub const PLAYER_POSITION_AND_LOOK: i32 = 0x06;
    pub const PLAYER_DIGGING: i32 = 0x07;
    pub const PLAYER_BLOCK_PLACEMENT: i32 = 0x08;
    pub const HELD_ITEM_CHANGE: i32 = 0x09;
    pub const ANIMATION: i32 = 0x0A;
    pub const CLICK_WINDOW: i32 = 0x0E;
    pub const CREATIVE_INVENTORY_ACTION: i32 = 0x10;
    pub const CLIENT_SETTINGS: i32 = 0x15;
    pub const CLIENT_STATUS: i32 = 0x16;
}

/// `C07 Packet Player Digging` status values (`C07PacketPlayerDigging.Action`
/// ordinals, sent as a VarInt).
pub mod digging {
    pub const START_DESTROY_BLOCK: u8 = 0;
    pub const ABORT_DESTROY_BLOCK: u8 = 1;
    pub const STOP_DESTROY_BLOCK: u8 = 2;
    pub const DROP_ALL_ITEMS: u8 = 3;
    pub const DROP_ITEM: u8 = 4;
    pub const RELEASE_USE_ITEM: u8 = 5;
}

/// Player container slot numbers, as used by `S30 Window Items`,
/// `S2F Set Slot` and `C10 Creative Inventory Action`.
///
/// The player container has 45 slots: 0 = crafting result, 1..=4 = crafting
/// grid, 5..=8 = armour, 9..=35 = main inventory, 36..=44 = the hotbar.
pub const PLAYER_CONTAINER_SLOTS: usize = 45;
pub const HOTBAR_SLOTS: usize = 9;
pub const HOTBAR_FIRST_SLOT: usize = 36;

/// Click modes for `C0E Packet Click Window` (`mode` on the wire).
pub mod click_mode {
    pub const CLICK: i8 = 0;
    pub const SHIFT_CLICK: i8 = 1;
    pub const NUMBER_KEY: i8 = 2;
    pub const MIDDLE_CLICK: i8 = 3;
    pub const DROP_KEY: i8 = 4;
    pub const DRAG: i8 = 5;
    pub const DOUBLE_CLICK: i8 = 6;
}

/// The slot number vanilla sends when the cursor leaves the window.
pub const SLOT_OUTSIDE: i16 = -999;

/// An item stack, as it appears in a slot on the wire.
///
/// Item NBT is skipped, not stored: Nephele only needs identity, count and
/// damage to render a hotbar and to re-send the held stack when placing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemStack {
    pub id: i16,
    pub count: i8,
    pub damage: i16,
}

impl ItemStack {
    /// Air (the `id` vanilla uses for "empty").
    pub const EMPTY_ID: i16 = -1;

    pub const fn new(id: i16, count: i8, damage: i16) -> Self {
        Self { id, count, damage }
    }

    /// Block items share their id with the block, so `1` is stone, `4` is
    /// cobblestone, and so on. Item ids at 256 and above are not blocks.
    pub const fn block_state(&self) -> Option<u16> {
        if self.id > 0 && self.id < 256 {
            Some(((self.id as u16) << 4) | (self.damage as u16 & 0x0F))
        } else {
            None
        }
    }
}

/// Packed block position, the 64-bit form used by most position fields.
pub fn write_position(writer: &mut PacketWriter, x: i32, y: i32, z: i32) -> &mut PacketWriter {
    let packed = (((x as i64) & 0x3FF_FFFF) << 38)
        | (((y as i64) & 0xFFF) << 26)
        | ((z as i64) & 0x3FF_FFFF);
    writer.write_i64(packed)
}

/// Write a slot. `None` encodes air.
pub fn write_slot<'a>(
    writer: &'a mut PacketWriter,
    item: Option<&ItemStack>,
) -> &'a mut PacketWriter {
    match item {
        None => writer.write_i16(ItemStack::EMPTY_ID),
        Some(item) => {
            writer
                .write_i16(item.id)
                .write_i8(item.count)
                .write_i16(item.damage)
                // No NBT alongside the stack.
                .write_u8(0)
        }
    }
}

/// Read a slot, skipping any NBT.
pub fn parse_slot(reader: &mut crate::read::PacketReader<'_>) -> Result<Option<ItemStack>> {
    let id = reader.read_i16()?;
    if id == ItemStack::EMPTY_ID {
        return Ok(None);
    }
    let count = reader.read_i8()?;
    let damage = reader.read_i16()?;
    crate::nbt::skip_named_tag(reader)?;
    Ok(Some(ItemStack { id, count, damage }))
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinGame {
    pub entity_id: i32,
    pub gamemode: u8,
    pub dimension: i8,
    pub difficulty: u8,
    pub max_players: u8,
    pub level_type: String,
    pub reduced_debug_info: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPositionAndLook {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u8,
}

pub fn parse_join_game(packet: &Packet) -> Result<JoinGame> {
    let mut r = packet.reader();
    Ok(JoinGame {
        entity_id: r.read_i32()?,
        gamemode: r.read_u8()?,
        dimension: r.read_i8()?,
        difficulty: r.read_u8()?,
        max_players: r.read_u8()?,
        level_type: r.read_string()?,
        reduced_debug_info: r.read_bool()?,
    })
}

pub fn parse_player_position_and_look(packet: &Packet) -> Result<PlayerPositionAndLook> {
    let mut r = packet.reader();
    Ok(PlayerPositionAndLook {
        x: r.read_f64()?,
        y: r.read_f64()?,
        z: r.read_f64()?,
        yaw: r.read_f32()?,
        pitch: r.read_f32()?,
        flags: r.read_u8()?,
    })
}

/// Parses a clientbound chat packet (JSON component + position byte).
pub fn parse_chat_message(packet: &Packet) -> Result<(String, u8)> {
    let mut r = packet.reader();
    let json = r.read_string()?;
    let position = r.read_u8()?;
    let text = match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => flatten_chat(&value),
        Err(_) => json,
    };
    Ok((text, position))
}

pub fn parse_disconnect(packet: &Packet) -> Result<String> {
    let json = packet.reader().read_string()?;
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => Ok(flatten_chat(&value)),
        Err(_) => Ok(json),
    }
}

pub fn keep_alive_response(id: i32) -> Packet {
    let mut w = PacketWriter::new();
    w.write_varint(id);
    Packet::new(serverbound::KEEP_ALIVE, w.into_bytes())
}

pub fn chat_message(message: &str) -> Packet {
    let mut w = PacketWriter::new();
    w.write_string(message);
    Packet::new(serverbound::CHAT_MESSAGE, w.into_bytes())
}

pub fn player(on_ground: bool) -> Packet {
    let mut w = PacketWriter::new();
    w.write_bool(on_ground);
    Packet::new(serverbound::PLAYER, w.into_bytes())
}

pub fn player_position(x: f64, y: f64, z: f64, on_ground: bool) -> Packet {
    let mut w = PacketWriter::new();
    w.write_f64(x)
        .write_f64(y)
        .write_f64(z)
        .write_bool(on_ground);
    Packet::new(serverbound::PLAYER_POSITION, w.into_bytes())
}

pub fn player_look(yaw: f32, pitch: f32, on_ground: bool) -> Packet {
    let mut w = PacketWriter::new();
    w.write_f32(yaw).write_f32(pitch).write_bool(on_ground);
    Packet::new(serverbound::PLAYER_LOOK, w.into_bytes())
}

pub fn player_position_and_look(
    x: f64,
    y: f64,
    z: f64,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
) -> Packet {
    let mut w = PacketWriter::new();
    w.write_f64(x)
        .write_f64(y)
        .write_f64(z)
        .write_f32(yaw)
        .write_f32(pitch)
        .write_bool(on_ground);
    Packet::new(serverbound::PLAYER_POSITION_AND_LOOK, w.into_bytes())
}

pub fn client_settings(
    locale: &str,
    view_distance: u8,
    chat_mode: u8,
    chat_colors: bool,
    skin_parts: u8,
) -> Packet {
    let mut w = PacketWriter::new();
    w.write_string(locale)
        .write_u8(view_distance)
        .write_u8(chat_mode)
        .write_bool(chat_colors)
        .write_u8(skin_parts);
    Packet::new(serverbound::CLIENT_SETTINGS, w.into_bytes())
}

/// Action 0 = perform respawn, which is what a vanilla client sends after
/// `Join Game`.
pub fn client_status(action: i32) -> Packet {
    let mut w = PacketWriter::new();
    w.write_varint(action);
    Packet::new(serverbound::CLIENT_STATUS, w.into_bytes())
}

/// `C07 Player Digging` — start / abort / stop breaking a block.
pub fn player_digging(status: u8, x: i32, y: i32, z: i32, face: u8) -> Packet {
    let mut w = PacketWriter::new();
    w.write_varint(status as i32);
    write_position(&mut w, x, y, z);
    w.write_u8(face);
    Packet::new(serverbound::PLAYER_DIGGING, w.into_bytes())
}

/// `C08 Player Block Placement`.
///
/// `cursor` is the hit point within the block, 0..1 on each axis; `held` is the
/// stack the server should validate against (our cached copy of the hotbar).
pub fn block_placement(
    x: i32,
    y: i32,
    z: i32,
    face: u8,
    cursor: [f32; 3],
    held: Option<&ItemStack>,
) -> Packet {
    let mut w = PacketWriter::new();
    write_position(&mut w, x, y, z);
    w.write_u8(face);
    write_slot(&mut w, held);
    for value in cursor {
        w.write_u8((value.clamp(0.0, 1.0) * 16.0) as u8);
    }
    Packet::new(serverbound::PLAYER_BLOCK_PLACEMENT, w.into_bytes())
}

/// `C09 Held Item Change` — select a hotbar slot (0..8).
pub fn held_item_change(slot: i16) -> Packet {
    let mut w = PacketWriter::new();
    w.write_i16(slot);
    Packet::new(serverbound::HELD_ITEM_CHANGE, w.into_bytes())
}

/// `C0A Animation` — swing the arm. The 1.8 packet has no body.
pub fn swing_arm() -> Packet {
    Packet::new(serverbound::ANIMATION, Vec::new())
}

/// `C10 Creative Inventory Action` — put a stack into (or clear) a container slot.
pub fn creative_set_slot(slot: i16, item: Option<&ItemStack>) -> Packet {
    let mut w = PacketWriter::new();
    w.write_i16(slot);
    write_slot(&mut w, item);
    Packet::new(serverbound::CREATIVE_INVENTORY_ACTION, w.into_bytes())
}

/// `C0E Click Window` — a single click in an open container.
///
/// `action` is the client's running counter for this window; the server echoes
/// it back in `S32 Confirm Transaction` so the client can tell which prediction
/// it is confirming. `clicked` is the stack *under the cursor* before the click,
/// which the server re-validates against its own copy.
pub fn click_window(
    window_id: u8,
    slot: i16,
    button: i8,
    action: i16,
    mode: i8,
    clicked: Option<&ItemStack>,
) -> Packet {
    let mut w = PacketWriter::new();
    w.write_u8(window_id);
    w.write_i16(slot);
    w.write_i8(button);
    w.write_i16(action);
    w.write_i8(mode);
    write_slot(&mut w, clicked);
    Packet::new(serverbound::CLICK_WINDOW, w.into_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockChange {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Raw block state (`id << 4 | meta`).
    pub state: u16,
}

pub fn parse_block_change(packet: &Packet) -> Result<BlockChange> {
    let mut r = packet.reader();
    let (x, y, z) = r.read_position()?;
    let state = r.read_varint()? as u16;
    Ok(BlockChange { x, y, z, state })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiBlockChange {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub changes: Vec<BlockChange>,
}

pub fn parse_multi_block_change(packet: &Packet) -> Result<MultiBlockChange> {
    let mut r = packet.reader();
    let chunk_x = r.read_i32()?;
    let chunk_z = r.read_i32()?;
    let count = r.read_varint()?.max(0);
    let mut changes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // A compact 16-bit offset: 4 bits x, 4 bits z, 8 bits y.
        let packed = r.read_i16()? as u16;
        let x = chunk_x * 16 + ((packed >> 12) & 0xF) as i32;
        let z = chunk_z * 16 + ((packed >> 8) & 0xF) as i32;
        let y = (packed & 0xFF) as i32;
        let state = r.read_varint()? as u16;
        changes.push(BlockChange { x, y, z, state });
    }
    Ok(MultiBlockChange {
        chunk_x,
        chunk_z,
        changes,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockBreakAnimation {
    pub breaker_entity_id: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 0..=9. Vanilla sends 255 to mean "stopped".
    pub stage: u8,
}

pub fn parse_block_break_animation(packet: &Packet) -> Result<BlockBreakAnimation> {
    let mut r = packet.reader();
    let breaker_entity_id = r.read_varint()?;
    let (x, y, z) = r.read_position()?;
    let stage = r.read_u8()?;
    Ok(BlockBreakAnimation {
        breaker_entity_id,
        x,
        y,
        z,
        stage,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowItems {
    pub window_id: u8,
    pub items: Vec<Option<ItemStack>>,
}

pub fn parse_window_items(packet: &Packet) -> Result<WindowItems> {
    let mut r = packet.reader();
    let window_id = r.read_u8()?;
    let count = r.read_i16()?.max(0) as usize;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(parse_slot(&mut r)?);
    }
    Ok(WindowItems { window_id, items })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSlot {
    pub window_id: i8,
    pub slot: i16,
    pub item: Option<ItemStack>,
}

pub fn parse_set_slot(packet: &Packet) -> Result<SetSlot> {
    let mut r = packet.reader();
    let window_id = r.read_i8()?;
    let slot = r.read_i16()?;
    let item = parse_slot(&mut r)?;
    Ok(SetSlot {
        window_id,
        slot,
        item,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmTransaction {
    pub window_id: i8,
    /// The `action` number of the `C0E` this answers.
    pub action: i16,
    /// `false` means the server rejected the click and the client should
    /// resync the window (`S30 Window Items`) rather than trust its prediction.
    pub accepted: bool,
}

pub fn parse_confirm_transaction(packet: &Packet) -> Result<ConfirmTransaction> {
    let mut r = packet.reader();
    Ok(ConfirmTransaction {
        window_id: r.read_i8()?,
        action: r.read_i16()?,
        accepted: r.read_bool()?,
    })
}

/// `S2F` written from the server side — the mock server uses it when a pickup
/// lands in a slot.
pub fn set_slot(window_id: i8, slot: i16, item: Option<&ItemStack>) -> Packet {
    let mut w = PacketWriter::new();
    w.write_i8(window_id).write_i16(slot);
    write_slot(&mut w, item);
    Packet::new(clientbound::SET_SLOT, w.into_bytes())
}

/// `S09 Held Item Change` — the server confirming which slot we're holding.
pub fn parse_held_item_change(packet: &Packet) -> Result<i16> {
    packet.reader().read_i16()
}

/// `S1F Experience` — the XP bar, level and lifetime total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Experience {
    /// Progress towards the next level, 0..1.
    pub bar: f32,
    pub level: i32,
    pub total: i32,
}

pub fn parse_experience(packet: &Packet) -> Result<Experience> {
    let mut r = packet.reader();
    Ok(Experience {
        bar: r.read_f32()?,
        level: r.read_varint()?,
        total: r.read_varint()?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeUpdate {
    pub world_age: i64,
    pub time_of_day: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateHealth {
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Respawn {
    pub dimension: i8,
    pub difficulty: u8,
    pub gamemode: u8,
    pub level_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAbilities {
    pub flags: u8,
    pub flying_speed: f32,
    pub walk_speed: f32,
}

pub fn parse_time_update(packet: &Packet) -> Result<TimeUpdate> {
    let mut r = packet.reader();
    Ok(TimeUpdate {
        world_age: r.read_i64()?,
        time_of_day: r.read_i64()?,
    })
}

pub fn parse_spawn_position(packet: &Packet) -> Result<(i32, i32, i32)> {
    packet.reader().read_position()
}

pub fn parse_update_health(packet: &Packet) -> Result<UpdateHealth> {
    let mut r = packet.reader();
    Ok(UpdateHealth {
        health: r.read_f32()?,
        food: r.read_varint()?,
        saturation: r.read_f32()?,
    })
}

pub fn parse_respawn(packet: &Packet) -> Result<Respawn> {
    let mut r = packet.reader();
    Ok(Respawn {
        dimension: r.read_i8()?,
        difficulty: r.read_u8()?,
        gamemode: r.read_u8()?,
        level_type: r.read_string()?,
    })
}

pub fn parse_player_abilities(packet: &Packet) -> Result<PlayerAbilities> {
    let mut r = packet.reader();
    Ok(PlayerAbilities {
        flags: r.read_u8()?,
        flying_speed: r.read_f32()?,
        walk_speed: r.read_f32()?,
    })
}

/// Entity coordinates are 1/32-block fixed point.
pub const FIXED_POINT: f64 = 1.0 / 32.0;
/// Velocity is in 1/8000 of a block per tick.
pub const VELOCITY_SCALE: f64 = 1.0 / 8000.0;

/// Entity angles are a full turn over 256 unsigned bytes.
fn angle_from_byte(value: u8) -> f32 {
    value as f32 * (360.0 / 256.0)
}

/// The inverse of [`angle_from_byte`], truncating like vanilla's cast.
fn angle_to_byte(degrees: f32) -> u8 {
    (degrees.rem_euclid(360.0) * (256.0 / 360.0)) as u8
}

/// Shortest-path wrap used for head yaw and deltas.
pub fn wrap_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped >= 180.0 {
        wrapped -= 360.0;
    }
    if wrapped < -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

fn read_fixed_point(r: &mut crate::read::PacketReader<'_>) -> Result<f64> {
    Ok(r.read_i32()? as f64 * FIXED_POINT)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPlayer {
    pub entity_id: i32,
    pub uuid: [u8; 16],
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

pub fn parse_spawn_player(packet: &Packet) -> Result<SpawnPlayer> {
    let mut r = packet.reader();
    let entity_id = r.read_varint()?;
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(r.take(16)?);
    let name = r.read_string()?;
    let x = read_fixed_point(&mut r)?;
    let y = read_fixed_point(&mut r)?;
    let z = read_fixed_point(&mut r)?;
    let yaw = angle_from_byte(r.read_u8()?);
    let pitch = angle_from_byte(r.read_u8()?);
    Ok(SpawnPlayer {
        entity_id,
        uuid,
        name,
        x,
        y,
        z,
        yaw,
        pitch,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnMob {
    pub entity_id: i32,
    /// Numeric mob id; 30 = player-shaped (see [`MOB_TYPE_PLAYER`]).
    pub kind: u8,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub head_pitch: f32,
}

/// The numeric type sent for armour stands and NPCs, which share the player
/// model. Vanilla reuses 30 for any player-shaped mob.
pub const MOB_TYPE_PLAYER: u8 = 30;

pub fn parse_spawn_mob(packet: &Packet) -> Result<SpawnMob> {
    let mut r = packet.reader();
    let entity_id = r.read_varint()?;
    let kind = r.read_u8()?;
    let x = read_fixed_point(&mut r)?;
    let y = read_fixed_point(&mut r)?;
    let z = read_fixed_point(&mut r)?;
    let yaw = angle_from_byte(r.read_u8()?);
    let pitch = angle_from_byte(r.read_u8()?);
    let head_pitch = angle_from_byte(r.read_u8()?);
    Ok(SpawnMob {
        entity_id,
        kind,
        x,
        y,
        z,
        yaw,
        pitch,
        head_pitch,
    })
}

/// `S13`: relative move in 1/32 block steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityMove {
    pub entity_id: i32,
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
}

fn read_delta(r: &mut crate::read::PacketReader<'_>) -> Result<f64> {
    Ok(r.read_i8()? as f64 * FIXED_POINT)
}

pub fn parse_entity_relative_move(packet: &Packet) -> Result<EntityMove> {
    let mut r = packet.reader();
    Ok(EntityMove {
        entity_id: r.read_varint()?,
        dx: read_delta(&mut r)?,
        dy: read_delta(&mut r)?,
        dz: read_delta(&mut r)?,
    })
}

/// `S16`: absolute angles only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityLookAngles {
    pub entity_id: i32,
    pub yaw: f32,
    pub pitch: f32,
}

pub fn parse_entity_look(packet: &Packet) -> Result<EntityLookAngles> {
    let mut r = packet.reader();
    Ok(EntityLookAngles {
        entity_id: r.read_varint()?,
        yaw: angle_from_byte(r.read_u8()?),
        pitch: angle_from_byte(r.read_u8()?),
    })
}

/// `S17`: relative move plus angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityLookAndMove {
    pub entity_id: i32,
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
    pub yaw: f32,
    pub pitch: f32,
}

pub fn parse_entity_look_and_relative_move(packet: &Packet) -> Result<EntityLookAndMove> {
    let mut r = packet.reader();
    Ok(EntityLookAndMove {
        entity_id: r.read_varint()?,
        dx: read_delta(&mut r)?,
        dy: read_delta(&mut r)?,
        dz: read_delta(&mut r)?,
        yaw: angle_from_byte(r.read_u8()?),
        pitch: angle_from_byte(r.read_u8()?),
    })
}

/// `S18`: absolute position, a hard correction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityTeleport {
    pub entity_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

pub fn parse_entity_teleport(packet: &Packet) -> Result<EntityTeleport> {
    let mut r = packet.reader();
    Ok(EntityTeleport {
        entity_id: r.read_varint()?,
        x: read_fixed_point(&mut r)?,
        y: read_fixed_point(&mut r)?,
        z: read_fixed_point(&mut r)?,
        yaw: angle_from_byte(r.read_u8()?),
        pitch: angle_from_byte(r.read_u8()?),
    })
}

/// `S19`: the head turns independently of the body.
pub fn parse_entity_head_look(packet: &Packet) -> Result<(i32, f32)> {
    let mut r = packet.reader();
    let entity_id = r.read_varint()?;
    Ok((entity_id, angle_from_byte(r.read_u8()?)))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityVelocityUpdate {
    pub entity_id: i32,
    /// Blocks per tick.
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
}

pub fn parse_entity_velocity(packet: &Packet) -> Result<EntityVelocityUpdate> {
    let mut r = packet.reader();
    Ok(EntityVelocityUpdate {
        entity_id: r.read_varint()?,
        vx: r.read_i16()? as f64 * VELOCITY_SCALE,
        vy: r.read_i16()? as f64 * VELOCITY_SCALE,
        vz: r.read_i16()? as f64 * VELOCITY_SCALE,
    })
}

/// `S13`: every entity the server dropped at once.
pub fn parse_destroy_entities(packet: &Packet) -> Result<Vec<i32>> {
    let mut r = packet.reader();
    let count = r.read_varint()?.max(0);
    let mut ids = Vec::with_capacity(count.min(256) as usize);
    for _ in 0..count {
        ids.push(r.read_varint()?);
    }
    Ok(ids)
}

/// `S14`: entity is still alive, only its id is sent (keep-alive for the
/// client's own entity cache).
pub fn parse_entity(packet: &Packet) -> Result<i32> {
    packet.reader().read_varint()
}

/// The object type used for a dropped item stack.
pub const OBJECT_TYPE_ITEM: u8 = 2;

/// `S0E`: a non-living object. Dropped items are the only kind the client
/// tracks; the rest are parsed so the wire format stays honest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnObject {
    pub entity_id: i32,
    /// Type of object; [`OBJECT_TYPE_ITEM`] is a dropped stack.
    pub kind: u8,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub pitch: f32,
    pub yaw: f32,
    /// Type-specific payload. For an item this is [`item_object_data`].
    pub data: i32,
    /// Blocks per tick, in vanilla's 1/8000 units.
    pub velocity: [f64; 3],
}

pub fn parse_spawn_object(packet: &Packet) -> Result<SpawnObject> {
    let mut r = packet.reader();
    let entity_id = r.read_varint()?;
    let kind = r.read_u8()?;
    let x = read_fixed_point(&mut r)?;
    let y = read_fixed_point(&mut r)?;
    let z = read_fixed_point(&mut r)?;
    let pitch = angle_from_byte(r.read_u8()?);
    let yaw = angle_from_byte(r.read_u8()?);
    let data = r.read_i32()?;
    let vx = r.read_i16()? as f64 * VELOCITY_SCALE;
    let vy = r.read_i16()? as f64 * VELOCITY_SCALE;
    let vz = r.read_i16()? as f64 * VELOCITY_SCALE;
    Ok(SpawnObject {
        entity_id,
        kind,
        x,
        y,
        z,
        pitch,
        yaw,
        data,
        velocity: [vx, vy, vz],
    })
}

/// Unpack an item object's `data`: the item id in the low half, metadata in
/// the high half, both signed.
pub fn item_from_object_data(data: i32) -> (i16, i16) {
    (
        (data & 0xFFFF) as u16 as i16,
        ((data >> 16) & 0xFFFF) as u16 as i16,
    )
}

/// The inverse of [`item_from_object_data`].
pub fn item_object_data(item: i16, meta: i16) -> i32 {
    i32::from(item as u16) | (i32::from(meta as u16) << 16)
}

/// `S0E` written from the server side — used by the mock server, and by the
/// round-trip test that keeps the writer and the parser in step.
#[allow(clippy::too_many_arguments)]
pub fn spawn_object(
    entity_id: i32,
    kind: u8,
    position: [f64; 3],
    pitch: f32,
    yaw: f32,
    data: i32,
    velocity: [f64; 3],
) -> Packet {
    let mut w = PacketWriter::new();
    w.write_varint(entity_id).write_u8(kind);
    for value in position {
        w.write_i32((value * 32.0) as i32);
    }
    w.write_u8(angle_to_byte(pitch))
        .write_u8(angle_to_byte(yaw));
    w.write_i32(data);
    for value in velocity {
        w.write_i16((value / VELOCITY_SCALE) as i16);
    }
    Packet::new(clientbound::SPAWN_OBJECT, w.into_bytes())
}

/// `S0D`: `collected` was picked up by `collector`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectItem {
    pub collected: i32,
    pub collector: i32,
}

pub fn parse_collect_item(packet: &Packet) -> Result<CollectItem> {
    let mut r = packet.reader();
    Ok(CollectItem {
        collected: r.read_varint()?,
        collector: r.read_varint()?,
    })
}

/// `S0D` from the server side.
pub fn collect_item(collected: i32, collector: i32) -> Packet {
    let mut w = PacketWriter::new();
    w.write_varint(collected).write_varint(collector);
    Packet::new(clientbound::COLLECT_ITEM, w.into_bytes())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerListEntry {
    pub uuid: [u8; 16],
    pub name: String,
    pub properties: Vec<PlayerProperty>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerListUpdate {
    /// The whole tab list entry: name, skin properties, gamemode, ping.
    Add(Vec<PlayerListEntry>),
    Remove(Vec<[u8; 16]>),
    /// Gamemode/latency/display-name-only updates, which carry no skin data.
    Other,
}

/// `S38`: the only packet that carries skin texture properties.
pub fn parse_player_list_item(packet: &Packet) -> Result<PlayerListUpdate> {
    let mut r = packet.reader();
    let action = r.read_varint()?;
    let count = r.read_varint()?.max(0);
    match action {
        0 => {
            let mut entries = Vec::with_capacity(count.min(64) as usize);
            for _ in 0..count {
                let mut uuid = [0u8; 16];
                uuid.copy_from_slice(r.take(16)?);
                let name = r.read_string()?;
                let property_count = r.read_varint()?.max(0);
                let mut properties = Vec::with_capacity(property_count.min(16) as usize);
                for _ in 0..property_count {
                    let name = r.read_string()?;
                    let value = r.read_string()?;
                    let signature = if r.read_bool()? {
                        Some(r.read_string()?)
                    } else {
                        None
                    };
                    properties.push(PlayerProperty {
                        name,
                        value,
                        signature,
                    });
                }
                // Gamemode, ping and the optional display name follow; we don't
                // render any of them yet but must consume them to stay framed.
                let _gamemode = r.read_varint()?;
                let _ping = r.read_varint()?;
                if r.read_bool()? {
                    let _display_name = r.read_string()?;
                }
                entries.push(PlayerListEntry {
                    uuid,
                    name,
                    properties,
                });
            }
            Ok(PlayerListUpdate::Add(entries))
        }
        4 => {
            let mut uuids = Vec::with_capacity(count.min(64) as usize);
            for _ in 0..count {
                let mut uuid = [0u8; 16];
                uuid.copy_from_slice(r.take(16)?);
                uuids.push(uuid);
            }
            Ok(PlayerListUpdate::Remove(uuids))
        }
        _ => Ok(PlayerListUpdate::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_join_game() {
        let mut w = PacketWriter::new();
        w.write_i32(42)
            .write_u8(0)
            .write_i8(0)
            .write_u8(1)
            .write_u8(20)
            .write_string("default")
            .write_bool(false);
        let join = parse_join_game(&Packet::new(clientbound::JOIN_GAME, w.into_bytes())).unwrap();
        assert_eq!(
            join,
            JoinGame {
                entity_id: 42,
                gamemode: 0,
                dimension: 0,
                difficulty: 1,
                max_players: 20,
                level_type: "default".into(),
                reduced_debug_info: false,
            }
        );
    }

    #[test]
    fn parses_position_and_look() {
        let mut w = PacketWriter::new();
        w.write_f64(0.5)
            .write_f64(64.0)
            .write_f64(-8.25)
            .write_f32(90.0)
            .write_f32(-10.0)
            .write_u8(0);
        let pos = parse_player_position_and_look(&Packet::new(
            clientbound::PLAYER_POSITION_AND_LOOK,
            w.into_bytes(),
        ))
        .unwrap();
        assert_eq!(pos.x, 0.5);
        assert_eq!(pos.y, 64.0);
        assert_eq!(pos.z, -8.25);
        assert_eq!(pos.yaw, 90.0);
        assert_eq!(pos.flags, 0);
    }

    #[test]
    fn keep_alive_echoes_id() {
        let packet = keep_alive_response(7_654_321);
        assert_eq!(packet.id, serverbound::KEEP_ALIVE);
        assert_eq!(packet.reader().read_varint().unwrap(), 7_654_321);
    }

    #[test]
    fn chat_parses_component() {
        let mut w = PacketWriter::new();
        w.write_string(r#"{"extra":[{"text":"<Nephele> "},{"text":"hi"}],"text":""}"#)
            .write_u8(0);
        let (text, position) =
            parse_chat_message(&Packet::new(clientbound::CHAT_MESSAGE, w.into_bytes())).unwrap();
        assert_eq!(text, "<Nephele> hi");
        assert_eq!(position, 0);
    }

    #[test]
    fn packed_position_matches_vanilla_layout() {
        let mut w = PacketWriter::new();
        write_position(&mut w, 1, 2, 3);
        let bytes = w.into_bytes();
        assert_eq!(bytes.len(), 8);
        let mut r = crate::read::PacketReader::new(&bytes);
        assert_eq!(r.read_position().unwrap(), (1, 2, 3));

        // Negative coordinates must survive the round trip.
        let mut w = PacketWriter::new();
        write_position(&mut w, -1000, 255, -30000000);
        let bytes = w.into_bytes();
        let mut r = crate::read::PacketReader::new(&bytes);
        assert_eq!(r.read_position().unwrap(), (-1000, 255, -30000000));
    }

    #[test]
    fn digging_packet_has_status_position_and_face() {
        let packet = player_digging(digging::START_DESTROY_BLOCK, 10, 64, -5, 1);
        assert_eq!(packet.id, serverbound::PLAYER_DIGGING);
        let mut r = packet.reader();
        assert_eq!(r.read_varint().unwrap(), 0);
        assert_eq!(r.read_position().unwrap(), (10, 64, -5));
        assert_eq!(r.read_u8().unwrap(), 1);
    }

    #[test]
    fn placement_packet_carries_cursor_and_held_stack() {
        let held = ItemStack::new(4, 64, 0);
        let packet = block_placement(8, 70, 8, 2, [0.25, 0.5, 0.75], Some(&held));
        assert_eq!(packet.id, serverbound::PLAYER_BLOCK_PLACEMENT);
        let mut r = packet.reader();
        assert_eq!(r.read_position().unwrap(), (8, 70, 8));
        assert_eq!(r.read_u8().unwrap(), 2);
        assert_eq!(parse_slot(&mut r).unwrap(), Some(held));
        assert_eq!(r.read_u8().unwrap(), 4);
        assert_eq!(r.read_u8().unwrap(), 8);
        assert_eq!(r.read_u8().unwrap(), 12);
    }

    #[test]
    fn click_window_carries_the_cursor_stack() {
        let held = ItemStack::new(3, 12, 0);
        let packet = click_window(0, 36, 0, 7, click_mode::CLICK, Some(&held));
        assert_eq!(packet.id, serverbound::CLICK_WINDOW);
        let mut r = packet.reader();
        assert_eq!(r.read_u8().unwrap(), 0, "window id");
        assert_eq!(r.read_i16().unwrap(), 36, "slot");
        assert_eq!(r.read_i8().unwrap(), 0, "button");
        assert_eq!(r.read_i16().unwrap(), 7, "action");
        assert_eq!(r.read_i8().unwrap(), click_mode::CLICK, "mode");
        assert_eq!(parse_slot(&mut r).unwrap(), Some(held));
        assert_eq!(r.read_u8().ok(), None, "no trailing bytes");
    }

    #[test]
    fn click_window_encodes_an_empty_cursor_as_air() {
        let packet = click_window(0, SLOT_OUTSIDE, 1, -3, click_mode::CLICK, None);
        let mut r = packet.reader();
        r.read_u8().unwrap();
        assert_eq!(r.read_i16().unwrap(), SLOT_OUTSIDE);
        assert_eq!(r.read_i8().unwrap(), 1, "right button");
        assert_eq!(r.read_i16().unwrap(), -3);
        assert_eq!(r.read_i8().unwrap(), click_mode::CLICK);
        assert_eq!(parse_slot(&mut r).unwrap(), None);
    }

    #[test]
    fn parses_confirm_transaction() {
        let mut w = PacketWriter::new();
        w.write_i8(0).write_i16(7).write_bool(false);
        let confirm = parse_confirm_transaction(&Packet::new(
            clientbound::CONFIRM_TRANSACTION,
            w.into_bytes(),
        ))
        .expect("confirm");
        assert_eq!(
            confirm,
            ConfirmTransaction {
                window_id: 0,
                action: 7,
                accepted: false,
            }
        );
    }

    #[test]
    fn empty_slots_encode_as_minus_one() {
        let mut w = PacketWriter::new();
        write_slot(&mut w, None);
        let bytes = w.into_bytes();
        assert_eq!(bytes, vec![0xFF, 0xFF]);
        let mut r = crate::read::PacketReader::new(&bytes);
        assert_eq!(parse_slot(&mut r).unwrap(), None);
    }

    #[test]
    fn slots_skip_trailing_nbt() {
        let mut w = PacketWriter::new();
        w.write_i16(276).write_i8(1).write_i16(0);
        // A named compound with one int inside.
        w.write_u8(10).write_string("");
        w.write_u8(3).write_string("Damage").write_i32(5);
        w.write_u8(0);
        w.write_i32(999);
        let bytes = w.into_bytes();
        let mut r = crate::read::PacketReader::new(&bytes);
        assert_eq!(parse_slot(&mut r).unwrap(), Some(ItemStack::new(276, 1, 0)));
        assert_eq!(r.read_i32().unwrap(), 999);
    }

    #[test]
    fn block_change_reads_position_and_state() {
        let mut w = PacketWriter::new();
        write_position(&mut w, 3, 70, -9);
        w.write_varint((4 << 4) | 2);
        let change =
            parse_block_change(&Packet::new(clientbound::BLOCK_CHANGE, w.into_bytes())).unwrap();
        assert_eq!(
            change,
            BlockChange {
                x: 3,
                y: 70,
                z: -9,
                state: 0x42
            }
        );
    }

    #[test]
    fn multi_block_change_unpacks_relative_offsets() {
        let mut w = PacketWriter::new();
        w.write_i32(2).write_i32(-1).write_varint(2);
        // x offset 5, z offset 7, y 64
        let packed = (5u16 << 12) | (7u16 << 8) | 64u16;
        w.write_i16(packed as i16).write_varint(1 << 4);
        // x offset 15, z offset 0, y 0
        let packed = 15u16 << 12;
        w.write_i16(packed as i16).write_varint(0);
        let change = parse_multi_block_change(&Packet::new(
            clientbound::MULTI_BLOCK_CHANGE,
            w.into_bytes(),
        ))
        .unwrap();
        assert_eq!(change.chunk_x, 2);
        assert_eq!(change.chunk_z, -1);
        assert_eq!(change.changes.len(), 2);
        assert_eq!(
            change.changes[0],
            BlockChange {
                x: 2 * 16 + 5,
                y: 64,
                z: -16 + 7,
                state: 0x10
            }
        );
        assert_eq!((change.changes[1].x, change.changes[1].z), (47, -16));
    }

    #[test]
    fn block_state_id_is_recovered_for_block_items() {
        assert_eq!(ItemStack::new(4, 1, 0).block_state(), Some(4 << 4));
        assert_eq!(ItemStack::new(5, 1, 3).block_state(), Some((5 << 4) | 3));
        assert_eq!(ItemStack::new(276, 1, 0).block_state(), None);
    }
}

#[cfg(test)]
mod entity_tests {
    use super::*;

    /// Angles are a byte over 256 steps; negatives wrap, as Minecraft does.
    fn deg(value: f32) -> u8 {
        (value / (360.0 / 256.0)).round().rem_euclid(256.0) as u8
    }

    #[test]
    fn parses_spawn_player() {
        let uuid = [7u8; 16];
        let mut w = PacketWriter::new();
        w.write_varint(99)
            .write_uuid(&uuid)
            .write_string("Notch")
            .write_i32(32 * 100)
            .write_i32(32 * 64)
            .write_i32(32 * -100)
            .write_u8(deg(90.0))
            .write_u8(deg(45.0))
            .write_i16(0)
            .write_bytes(&[0u8; 5]);
        let spawn = parse_spawn_player(&Packet::new(clientbound::SPAWN_PLAYER, w.into_bytes()))
            .expect("spawn player");
        assert_eq!(spawn.entity_id, 99);
        assert_eq!(spawn.uuid, uuid);
        assert_eq!(spawn.name, "Notch");
        assert_eq!((spawn.x, spawn.y, spawn.z), (100.0, 64.0, -100.0));
        assert!((spawn.yaw - 90.0).abs() < 2.0);
        assert!((spawn.pitch - 45.0).abs() < 2.0);
    }

    #[test]
    fn parses_relative_move_and_look() {
        let mut w = PacketWriter::new();
        // +1.0, -0.5, +0.25 blocks.
        w.write_varint(5)
            .write_i8(32)
            .write_i8(-16)
            .write_i8(8)
            .write_u8(deg(180.0))
            .write_u8(deg(-90.0));
        let packet = Packet::new(clientbound::ENTITY_LOOK_AND_RELATIVE_MOVE, w.into_bytes());
        let update = parse_entity_look_and_relative_move(&packet).expect("look and move");
        assert_eq!(update.entity_id, 5);
        assert_eq!((update.dx, update.dy, update.dz), (1.0, -0.5, 0.25));
        assert!((update.yaw - 180.0).abs() < 2.0);
        // Pitch is unsigned on the wire: -90° arrives as 270°.
        assert!((update.pitch - 270.0).abs() < 2.0);
    }

    #[test]
    fn parses_teleport_absolutely() {
        let mut w = PacketWriter::new();
        w.write_varint(12)
            .write_i32(32 * -64)
            .write_i32(32 * 70)
            .write_i32(32 * 64)
            .write_u8(0)
            .write_u8(0);
        let teleport =
            parse_entity_teleport(&Packet::new(clientbound::ENTITY_TELEPORT, w.into_bytes()))
                .expect("teleport");
        assert_eq!(teleport.entity_id, 12);
        assert_eq!((teleport.x, teleport.y, teleport.z), (-64.0, 70.0, 64.0));
    }

    #[test]
    fn parses_velocity_in_blocks_per_tick() {
        let mut w = PacketWriter::new();
        // 8000 = 1.0 block/tick.
        w.write_varint(3)
            .write_i16(8000)
            .write_i16(-4000)
            .write_i16(0);
        let velocity =
            parse_entity_velocity(&Packet::new(clientbound::ENTITY_VELOCITY, w.into_bytes()))
                .expect("velocity");
        assert_eq!(velocity.entity_id, 3);
        assert!((velocity.vx - 1.0).abs() < 1e-9);
        assert!((velocity.vy + 0.5).abs() < 1e-9);
    }

    #[test]
    fn parses_destroy_entities() {
        let mut w = PacketWriter::new();
        w.write_varint(3)
            .write_varint(10)
            .write_varint(11)
            .write_varint(300);
        let ids =
            parse_destroy_entities(&Packet::new(clientbound::DESTROY_ENTITIES, w.into_bytes()))
                .expect("destroy");
        assert_eq!(ids, vec![10, 11, 300]);
    }

    #[test]
    fn parses_player_list_add_with_skin_property() {
        let mut w = PacketWriter::new();
        w.write_varint(0)
            .write_varint(1)
            .write_uuid(&[1u8; 16])
            .write_string("Steve")
            .write_varint(1)
            .write_string("textures")
            .write_string("eyJ0ZXh0dXJlcyI6e319")
            .write_bool(false)
            .write_varint(0)
            .write_varint(42)
            .write_bool(false);
        let update =
            parse_player_list_item(&Packet::new(clientbound::PLAYER_LIST_ITEM, w.into_bytes()))
                .expect("player list");
        let PlayerListUpdate::Add(entries) = update else {
            panic!("expected an add entry");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Steve");
        assert_eq!(entries[0].properties.len(), 1);
        assert_eq!(entries[0].properties[0].name, "textures");
        assert_eq!(entries[0].properties[0].value, "eyJ0ZXh0dXJlcyI6e319");
        assert_eq!(entries[0].properties[0].signature, None);
    }

    #[test]
    fn parses_player_list_remove() {
        let mut w = PacketWriter::new();
        w.write_varint(4).write_varint(2);
        w.write_uuid(&[2u8; 16]).write_uuid(&[3u8; 16]);
        let update =
            parse_player_list_item(&Packet::new(clientbound::PLAYER_LIST_ITEM, w.into_bytes()))
                .expect("player list");
        assert_eq!(update, PlayerListUpdate::Remove(vec![[2u8; 16], [3u8; 16]]));
    }

    #[test]
    fn angles_wrap_to_the_short_way_round() {
        assert_eq!(wrap_degrees(350.0), -10.0);
        assert_eq!(wrap_degrees(-350.0), 10.0);
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(10.0), 10.0);
    }

    /// The writer is the mock server's side of the same format, so a
    /// round-trip is the only test that keeps the two in step.
    #[test]
    fn a_dropped_item_survives_a_round_trip() {
        let packet = spawn_object(
            7,
            OBJECT_TYPE_ITEM,
            [10.5, 64.0, -3.25],
            0.0,
            90.0,
            item_object_data(17, 2),
            [0.0, 0.1, 0.0],
        );
        let object = parse_spawn_object(&packet).expect("spawn object");
        assert_eq!(object.entity_id, 7);
        assert_eq!(object.kind, OBJECT_TYPE_ITEM);
        assert_eq!(object.x, 10.5);
        assert_eq!(object.y, 64.0);
        assert_eq!(object.z, -3.25);
        assert!((object.yaw - 90.0).abs() < 1.5);
        assert_eq!(item_from_object_data(object.data), (17, 2));
        assert!((object.velocity[1] - 0.1).abs() < 1e-3);
    }

    #[test]
    fn item_object_data_packs_signed_halves() {
        for (id, meta) in [(1i16, 0i16), (17, 3), (i16::MAX, i16::MIN)] {
            assert_eq!(
                item_from_object_data(item_object_data(id, meta)),
                (id, meta)
            );
        }
        // Item ids are positive in practice, but the low half is signed on the
        // wire, so an empty-ish id must not bleed into the metadata half.
        assert_eq!(item_object_data(0, 5), 0x0005_0000);
    }

    #[test]
    fn a_collect_item_round_trips() {
        let packet = collect_item(314, 1);
        assert_eq!(
            parse_collect_item(&packet).expect("collect"),
            CollectItem {
                collected: 314,
                collector: 1
            }
        );
    }
}
