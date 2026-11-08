//! `nephele-client` â€” the headless side of Nephele.
//!
//! Owns the connection, the loaded world and the local player state, and
//! translates play-state packets into structured [`Event`]s. The renderer will
//! consume this later; for now the CLI drives it and prints statistics.

pub mod container;
pub mod entities;
pub mod mining;
pub mod physics;
pub mod skins;

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use nephele_protocol::packets::play;
use nephele_protocol::packets::play::ItemStack;
use nephele_protocol::{Connection, Error, LoginOutcome, NextState, Packet, Result, ServerAddress};
use nephele_world::{
    decode_chunk_bulk, decode_chunk_data, ChunkColumn, ChunkCoord, ChunkUpdate, World,
};

pub use container::LocalContainer;
pub use entities::{Entities, Entity, EntityKind};
pub use mining::{Aim, Miner, MiningEvent};
pub use skins::{SkinInfo, Skins};

/// A container window the server opened (e.g. a crafting table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindowState {
    pub id: u8,
    pub kind: String,
    /// Slot contents as sent by `S30`; empty until the first resync.
    pub slots: Vec<Option<ItemStack>>,
}

/// The public fields carried by protocol-47's player-list packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabListEntry {
    pub name: String,
    pub gamemode: i32,
    pub ping: i32,
    /// Raw chat-component JSON from the server, if present.
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn chunk(&self) -> ChunkCoord {
        ChunkCoord::from((self.x, self.z))
    }
}

impl fmt::Display for Vec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2} / {:.2} / {:.2}", self.x, self.y, self.z)
    }
}

#[derive(Debug, Clone)]
pub struct JoinInfo {
    pub username: String,
    pub uuid: String,
    pub entity_id: i32,
    pub gamemode: u8,
    pub dimension: i8,
    pub level_type: String,
    pub max_players: u8,
    pub compression: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// No packet arrived within the tick timeout.
    Idle,
    /// A packet arrived whole but this client could not parse it.
    ///
    /// Reported instead of ending the session: the stream is still in sync, so
    /// the worst case is one ignored message.
    Malformed {
        id: i32,
        message: String,
    },
    /// A chunk column was loaded.
    ChunkLoaded {
        x: i32,
        z: i32,
    },
    /// A chunk column was unloaded.
    ChunkUnloaded {
        x: i32,
        z: i32,
    },
    /// The server placed us; `spawned` flips true on the first one.
    Position {
        position: Vec3,
        yaw: f32,
        pitch: f32,
    },
    Chat(String),
    Health {
        health: f32,
        food: i32,
    },
    Respawn {
        dimension: i8,
    },
    /// One or more blocks changed in the world.
    BlocksChanged(usize),
    /// The held hotbar slot was changed by the server.
    HeldSlot(i16),
    /// The player inventory changed (contents, not a specific slot).
    Inventory,
    /// The server answered a container click (`S32`). `accepted = false` means
    /// the click was rejected and the slot will be resent; the UI should drop
    /// any prediction it made for it.
    ConfirmTransaction {
        action: i16,
        accepted: bool,
    },
    /// A container window opened (`S2D`); its slot contents follow in `S30`.
    WindowOpened {
        window_id: u8,
        kind: String,
    },
    /// A container window closed (`S2E`).
    WindowClosed,
    /// Another player is breaking a block (`S25`). `stage` 255 = stopped.
    BreakAnimation {
        breaker: i32,
        x: i32,
        y: i32,
        z: i32,
        stage: u8,
    },
    /// Our own entity appeared in the world (`S0C` for our id).
    Spawned,
    /// A remote player or mob appeared.
    EntitySpawned {
        entity_id: i32,
        name: String,
    },
    /// A tracked entity's position or angles changed.
    EntityMoved,
    /// Entities were removed; carries how many.
    EntitiesRemoved(usize),
    /// The tab list (and therefore skins) changed.
    PlayerList,
    /// A block entity update (`S35`) was parsed and retained.
    BlockEntityUpdated {
        x: i32,
        y: i32,
        z: i32,
        action: u8,
    },
    /// The server announced its brand on `MC|Brand`.
    ServerBrand(String),
    /// An unhandled server plugin channel was received.
    PluginMessage {
        channel: String,
    },
    /// Any other packet; carries its id so callers can log/trace.
    Packet(i32),
}

/// Default view distance reported to the server, in chunks.
pub const VIEW_DISTANCE: u8 = 8;

/// Minecraft yaw (degrees, 0 = +Z, increasing towards -X) to Nephele's camera
/// yaw (radians, 0 = -Z, increasing towards +X).
fn yaw_from_mc(yaw_degrees: f32) -> f32 {
    (yaw_degrees.to_radians() + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
}

/// Nephele camera yaw back to Minecraft's convention, in degrees.
fn yaw_to_mc(yaw_radians: f32) -> f32 {
    (yaw_radians - std::f32::consts::PI)
        .to_degrees()
        .rem_euclid(360.0)
}

/// Minecraft pitch is positive looking *down*; Nephele's is positive looking up.
fn pitch_from_mc(pitch_degrees: f32) -> f32 {
    -pitch_degrees.to_radians()
}

fn pitch_to_mc(pitch_radians: f32) -> f32 {
    -pitch_radians.to_degrees()
}

/// What a creative player spawns holding, so block placement works immediately.
pub const STARTER_HOTBAR: [nephele_world::BlockState; 9] = [
    nephele_world::BlockState::COBBLESTONE,
    nephele_world::BlockState::PLANKS,
    nephele_world::BlockState::STONE,
    nephele_world::BlockState::DIRT,
    nephele_world::BlockState::GRASS,
    nephele_world::BlockState::GLASS,
    nephele_world::BlockState::LOG,
    nephele_world::BlockState::SAND,
    nephele_world::BlockState::SNOW_BLOCK,
];

pub struct GameClient {
    connection: Connection,
    pub info: JoinInfo,
    pub world: World,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub spawned: bool,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    /// XP bar progress towards the next level, 0..1.
    pub xp_bar: f32,
    /// Experience level.
    pub xp_level: i32,
    /// Lifetime experience points.
    pub xp_total: i32,
    pub world_age: i64,
    pub time_of_day: i64,
    pub spawn_position: Option<(i32, i32, i32)>,
    /// Player container contents (`S30` / `S2F`), 45 slots; 36..=44 is the hotbar.
    pub inventory: Vec<Option<ItemStack>>,
    pub held_slot: i16,
    /// Next `C0E` action number per open window. Vanilla keeps one counter per
    /// container and the server echoes the value in `S32`.
    click_actions: HashMap<i8, i16>,
    /// The container the server has open, if any (window 0 is the player's own).
    pub open_window: Option<OpenWindowState>,
    pub chat: Vec<String>,
    /// Every entity the server has told us about except ourselves.
    pub entities: Entities,
    /// Skins advertised on the tab list, keyed by UUID.
    pub skins: Skins,
    /// Current protocol-47 tab-list rows, keyed by player UUID.
    pub player_list: HashMap<[u8; 16], TabListEntry>,
    /// Most recent update-tile-entity NBT, keyed by block position.
    pub block_entities: HashMap<(i32, i32, i32), play::BlockEntityUpdate>,
    /// The server's advertised `MC|Brand`, when it sends one.
    pub server_brand: Option<String>,
    view_distance: u8,
    pub packets_received: u64,
    pub chunks_loaded: u64,
    pub chunks_unloaded: u64,
    pub address: ServerAddress,
}

impl GameClient {
    /// Connect, authenticate offline, and enter the play state. The world is
    /// empty until [`GameClient::tick`] has run for a while.
    pub async fn join(address: &ServerAddress, username: &str) -> Result<Self> {
        let mut connection = Connection::connect(address).await?;
        connection.handshake(address, NextState::Login).await?;

        let (uuid, name) = match connection.login_start(username).await? {
            LoginOutcome::Success(success) => (success.uuid, success.username),
            LoginOutcome::Disconnect(reason) => return Err(Error::Disconnected(reason)),
            LoginOutcome::EncryptionRequest(_) => return Err(Error::EncryptionUnsupported),
        };

        Self::finish(connection, address, uuid, name).await
    }

    /// Connect to an online-mode server with a signed-in account.
    ///
    /// The server answers Login Start with an Encryption Request. We prove
    /// ownership to Mojang first, then hand back the RSA-encrypted shared
    /// secret and verify token, and the connection switches to AES for the
    /// rest of the session.
    pub async fn join_online(
        address: &ServerAddress,
        session: &nephele_auth::Session,
    ) -> Result<Self> {
        let mut connection = Connection::connect(address)
            .await
            .map_err(|err| Error::Protocol(format!("connecting to {address}: {err}")))?;
        connection
            .handshake(address, NextState::Login)
            .await
            .map_err(|err| Error::Protocol(format!("sending login handshake: {err}")))?;

        let login = connection
            .login_start(&session.profile.name)
            .await
            .map_err(|err| Error::Protocol(format!("reading Login Start response: {err}")))?;
        let (uuid, name) = match login {
            LoginOutcome::Success(success) => (success.uuid, success.username),
            LoginOutcome::Disconnect(reason) => return Err(Error::Disconnected(reason)),
            LoginOutcome::EncryptionRequest(request) => {
                let secret = nephele_protocol::crypto::new_shared_secret();
                let hash = nephele_protocol::crypto::server_id_hash(
                    &request.server_id,
                    &secret,
                    &request.public_key,
                );

                // The session server call is blocking HTTP; keep it off the
                // async worker.
                let token = session.access_token.clone();
                let profile = session.profile.id.clone();
                let join_hash = hash.clone();
                let joined = tokio::task::spawn_blocking(move || {
                    nephele_auth::join_server(&token, &profile, &join_hash)
                        .map_err(|err| err.to_string())
                })
                .await
                .map_err(|err| Error::Disconnected(format!("join task failed: {err}")))?;
                joined.map_err(Error::Disconnected)?;

                let encrypted_secret =
                    nephele_protocol::crypto::rsa_encrypt(&request.public_key, &secret)
                        .map_err(Error::Disconnected)?;
                let encrypted_verify_token = nephele_protocol::crypto::rsa_encrypt(
                    &request.public_key,
                    &request.verify_token,
                )
                .map_err(Error::Disconnected)?;
                connection
                    .send_encryption_response(&secret, &encrypted_secret, &encrypted_verify_token)
                    .await
                    .map_err(|err| {
                        Error::Protocol(format!("sending Encryption Response: {err}"))
                    })?;

                let outcome = connection.read_login_outcome().await.map_err(|err| {
                    Error::Protocol(format!("reading Login Success after encryption: {err}"))
                })?;
                match outcome {
                    LoginOutcome::Success(success) => (success.uuid, success.username),
                    LoginOutcome::Disconnect(reason) => return Err(Error::Disconnected(reason)),
                    LoginOutcome::EncryptionRequest(_) => return Err(Error::EncryptionUnsupported),
                }
            }
        };

        Self::finish(connection, address, uuid, name).await
    }
    /// Finish a login once the connection is authenticated, offline or online.
    async fn finish(
        mut connection: Connection,
        address: &ServerAddress,
        uuid: String,
        name: String,
    ) -> Result<Self> {
        let join = connection.wait_for_join_game().await?;
        let compression = connection.compression_threshold();

        connection
            .write_packet(&play::client_settings(
                "en_US",
                VIEW_DISTANCE,
                0,
                true,
                0x7F,
            ))
            .await?;
        connection
            .write_packet(&play::brand_plugin_message("Nephele"))
            .await?;
        // Vanilla's "perform respawn"; tells the server we are ready.
        connection.write_packet(&play::client_status(0)).await?;

        let world = World::new(join.dimension);
        Ok(Self {
            connection,
            world,
            info: JoinInfo {
                username: name,
                uuid,
                entity_id: join.entity_id,
                gamemode: join.gamemode,
                dimension: join.dimension,
                level_type: join.level_type,
                max_players: join.max_players,
                compression,
            },
            position: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            spawned: false,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            xp_bar: 0.0,
            xp_level: 0,
            xp_total: 0,
            world_age: 0,
            time_of_day: 0,
            spawn_position: None,
            inventory: vec![None; play::PLAYER_CONTAINER_SLOTS],
            held_slot: 0,
            click_actions: HashMap::new(),
            open_window: None,
            chat: Vec::new(),
            entities: Entities::new(),
            skins: Skins::new(),
            player_list: HashMap::new(),
            block_entities: HashMap::new(),
            server_brand: None,
            view_distance: VIEW_DISTANCE,
            packets_received: 0,
            chunks_loaded: 0,
            chunks_unloaded: 0,
            address: address.clone(),
        })
    }

    /// Read one packet (waiting at most `timeout`) and apply it.
    pub async fn tick(&mut self, timeout: Duration) -> Result<Event> {
        let packet = match tokio::time::timeout(timeout, self.connection.next_play_packet()).await {
            Err(_) => return Ok(Event::Idle),
            Ok(Ok(packet)) => packet,
            Ok(Err(err)) => return Err(err),
        };
        self.packets_received += 1;
        let id = packet.id;
        match self.handle(packet).await {
            Ok(event) => Ok(event),
            // The frame itself is intact - the framer already consumed exactly
            // its declared length - so a parse failure is one packet we don't
            // understand, not a broken stream. Dropping the connection here
            // used to freeze the whole client on servers that send anything
            // unexpected, which read as "the game stopped responding".
            Err(err) => Ok(Event::Malformed {
                id,
                message: err.to_string(),
            }),
        }
    }

    /// Run ticks until `duration` has elapsed, returning the number of events
    /// applied.
    pub async fn pump(&mut self, duration: Duration, tick_timeout: Duration) -> Result<u64> {
        let deadline = tokio::time::Instant::now() + duration;
        let mut events = 0u64;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait = remaining.min(tick_timeout);
            match self.tick(wait).await? {
                Event::Idle => {}
                Event::Chat(text) => {
                    tracing::info!(target: "nephele::chat", "{text}");
                    events += 1;
                }
                _ => events += 1,
            }
        }
        Ok(events)
    }

    async fn handle(&mut self, packet: Packet) -> Result<Event> {
        match packet.id {
            play::clientbound::CHUNK_DATA => {
                let header = play::parse_chunk_data_header(&packet)?;
                match decode_chunk_data(&packet, self.world.has_skylight()) {
                    Ok(ChunkUpdate::Load(column)) => {
                        let (x, z) = (column.x, column.z);
                        let coord = ChunkCoord::new(x, z);
                        if header.ground_up {
                            self.clear_block_entities_in_chunk(coord);
                            self.world.insert(column);
                        } else {
                            let merged = merge_partial_column(self.world.get(coord), column);
                            self.world.insert(merged);
                        }
                        self.chunks_loaded += 1;
                        Ok(Event::ChunkLoaded { x, z })
                    }
                    Ok(ChunkUpdate::Unload { x, z }) => {
                        let coord = ChunkCoord::new(x, z);
                        self.world.remove(coord);
                        self.clear_block_entities_in_chunk(coord);
                        self.chunks_unloaded += 1;
                        Ok(Event::ChunkUnloaded { x, z })
                    }
                    Err(err) => Err(err),
                }
            }
            play::clientbound::MAP_CHUNK_BULK => {
                let columns = decode_chunk_bulk(&packet)?;
                let mut last = (0, 0);
                for column in columns {
                    last = (column.x, column.z);
                    self.clear_block_entities_in_chunk(ChunkCoord::new(column.x, column.z));
                    self.world.insert(column);
                    self.chunks_loaded += 1;
                }
                Ok(Event::ChunkLoaded {
                    x: last.0,
                    z: last.1,
                })
            }
            play::clientbound::PLAYER_POSITION_AND_LOOK => {
                let pos = play::parse_player_position_and_look(&packet)?;
                self.position = Vec3::new(pos.x, pos.y, pos.z);
                self.yaw = yaw_from_mc(pos.yaw);
                self.pitch = pitch_from_mc(pos.pitch);
                self.on_ground = true;
                self.spawned = true;
                // Confirm the teleport, echoing the packet's own values.
                self.connection
                    .write_packet(&play::player_position_and_look(
                        pos.x, pos.y, pos.z, pos.yaw, pos.pitch, true,
                    ))
                    .await?;
                Ok(Event::Position {
                    position: self.position,
                    yaw: self.yaw,
                    pitch: self.pitch,
                })
            }
            play::clientbound::CHAT_MESSAGE => {
                let (text, _position) = play::parse_chat_message(&packet)?;
                self.chat.push(text.clone());
                if self.chat.len() > 512 {
                    self.chat.remove(0);
                }
                Ok(Event::Chat(text))
            }
            play::clientbound::UPDATE_HEALTH => {
                let health = play::parse_update_health(&packet)?;
                self.health = health.health;
                self.food = health.food;
                self.saturation = health.saturation;
                Ok(Event::Health {
                    health: health.health,
                    food: health.food,
                })
            }
            play::clientbound::EXPERIENCE => {
                let xp = play::parse_experience(&packet)?;
                self.xp_bar = xp.bar;
                self.xp_level = xp.level;
                self.xp_total = xp.total;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::TIME_UPDATE => {
                let time = play::parse_time_update(&packet)?;
                self.world_age = time.world_age;
                self.time_of_day = time.time_of_day;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::SPAWN_POSITION => {
                let pos = play::parse_spawn_position(&packet)?;
                self.spawn_position = Some(pos);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::PLAYER_ABILITIES => {
                let abilities = play::parse_player_abilities(&packet)?;
                tracing::debug!(
                    flags = abilities.flags,
                    fly = abilities.flying_speed,
                    walk = abilities.walk_speed,
                    "player abilities"
                );
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::RESPAWN => {
                let respawn = play::parse_respawn(&packet)?;
                // A death respawn sends S07 with the *same* dimension. Vanilla
                // keeps the loaded chunks in that case; only a real dimension
                // change invalidates them.
                if self.world.dimension() != respawn.dimension {
                    self.world.set_dimension(respawn.dimension);
                }
                self.entities.clear();
                self.block_entities.clear();
                self.info.dimension = respawn.dimension;
                self.info.gamemode = respawn.gamemode;
                self.info.level_type = respawn.level_type;
                self.spawned = false;
                self.position = Vec3::default();
                self.yaw = 0.0;
                self.pitch = 0.0;
                self.on_ground = true;
                self.spawn_position = None;
                self.world_age = 0;
                self.time_of_day = 0;
                self.open_window = None;
                self.connection
                    .write_packet(&play::client_settings(
                        "en_US",
                        self.view_distance,
                        0,
                        true,
                        0x7F,
                    ))
                    .await?;
                Ok(Event::Respawn {
                    dimension: respawn.dimension,
                })
            }
            play::clientbound::BLOCK_CHANGE => {
                let change = play::parse_block_change(&packet)?;
                self.world.set_block(
                    change.x,
                    change.y,
                    change.z,
                    nephele_world::BlockState::from_raw(change.state),
                );
                Ok(Event::BlocksChanged(1))
            }
            play::clientbound::MULTI_BLOCK_CHANGE => {
                let update = play::parse_multi_block_change(&packet)?;
                let mut applied = 0;
                for change in &update.changes {
                    if self.world.set_block(
                        change.x,
                        change.y,
                        change.z,
                        nephele_world::BlockState::from_raw(change.state),
                    ) {
                        applied += 1;
                    }
                }
                Ok(Event::BlocksChanged(applied))
            }
            play::clientbound::BLOCK_BREAK_ANIMATION => {
                let animation = play::parse_block_break_animation(&packet)?;
                Ok(Event::BreakAnimation {
                    breaker: animation.breaker_entity_id,
                    x: animation.x,
                    y: animation.y,
                    z: animation.z,
                    stage: animation.stage,
                })
            }
            play::clientbound::HELD_ITEM_CHANGE => {
                let slot = play::parse_held_item_change(&packet)?;
                self.held_slot = slot.clamp(0, (play::HOTBAR_SLOTS - 1) as i16);
                Ok(Event::HeldSlot(self.held_slot))
            }
            play::clientbound::OPEN_WINDOW => {
                let window = play::parse_open_window(&packet)?;
                self.open_window = Some(OpenWindowState {
                    id: window.window_id,
                    kind: window.kind.clone(),
                    slots: Vec::new(),
                });
                Ok(Event::WindowOpened {
                    window_id: window.window_id,
                    kind: window.kind,
                })
            }
            play::clientbound::CLOSE_WINDOW => {
                let id = play::parse_close_window(&packet)?;
                if self.open_window.as_ref().is_some_and(|open| open.id == id) {
                    self.open_window = None;
                }
                Ok(Event::WindowClosed)
            }
            play::clientbound::SET_SLOT => {
                let update = play::parse_set_slot(&packet)?;
                if update.window_id == 0 && update.slot >= 0 {
                    // A server may fill the hotbar with individual Set Slot
                    // packets before any Window Items has sized the vector, and
                    // a lobby does exactly that. Growing it here means the first
                    // item is not silently dropped because slot 36 did not exist
                    // yet.
                    if self.inventory.len() < play::PLAYER_CONTAINER_SLOTS {
                        self.inventory.resize(play::PLAYER_CONTAINER_SLOTS, None);
                    }
                    let slot = update.slot as usize;
                    if slot < self.inventory.len() {
                        self.inventory[slot] = update.item;
                    }
                } else if let Some(open) = self.open_window.as_mut() {
                    if update.window_id as u8 == open.id && update.slot >= 0 {
                        let slot = update.slot as usize;
                        if slot < open.slots.len() {
                            open.slots[slot] = update.item;
                        }
                    }
                }
                Ok(Event::Inventory)
            }
            play::clientbound::WINDOW_ITEMS => {
                let update = play::parse_window_items(&packet)?;
                if update.window_id == 0 {
                    self.inventory.resize(play::PLAYER_CONTAINER_SLOTS, None);
                    for (slot, item) in update.items.iter().enumerate() {
                        if slot < self.inventory.len() {
                            self.inventory[slot] = *item;
                        }
                    }
                    for slot in update.items.len()..self.inventory.len() {
                        self.inventory[slot] = None;
                    }
                } else if let Some(open) = self.open_window.as_mut() {
                    if update.window_id as u8 == open.id {
                        open.slots.resize(update.items.len(), None);
                        open.slots.clone_from(&update.items);
                    }
                }
                Ok(Event::Inventory)
            }
            play::clientbound::CONFIRM_TRANSACTION => {
                let confirm = play::parse_confirm_transaction(&packet)?;
                Ok(Event::ConfirmTransaction {
                    action: confirm.action,
                    accepted: confirm.accepted,
                })
            }
            play::clientbound::SPAWN_PLAYER => {
                let spawn = play::parse_spawn_player(&packet)?;
                if spawn.entity_id == self.info.entity_id {
                    return Ok(Event::Spawned);
                }
                self.entities.spawn_player(
                    spawn.entity_id,
                    spawn.uuid,
                    spawn.name.clone(),
                    [spawn.x, spawn.y, spawn.z],
                    spawn.yaw,
                    spawn.pitch,
                );
                Ok(Event::EntitySpawned {
                    entity_id: spawn.entity_id,
                    name: spawn.name,
                })
            }
            play::clientbound::SPAWN_MOB => {
                let spawn = play::parse_spawn_mob(&packet)?;
                self.entities.spawn_mob(
                    spawn.entity_id,
                    spawn.kind,
                    spawn.kind == play::MOB_TYPE_PLAYER,
                    [spawn.x, spawn.y, spawn.z],
                    spawn.yaw,
                    spawn.pitch,
                );
                Ok(Event::EntitySpawned {
                    entity_id: spawn.entity_id,
                    name: String::new(),
                })
            }
            play::clientbound::SPAWN_OBJECT => {
                let object = play::parse_spawn_object(&packet)?;
                if object.kind != play::OBJECT_TYPE_ITEM {
                    // Arrows, falling blocks, boats, ... are not drawn yet, but
                    // parsing them keeps the packet stream understood.
                    return Ok(Event::Packet(packet.id));
                }
                let (item, meta) = play::item_from_object_data(object.data);
                self.entities.spawn_item(
                    object.entity_id,
                    item,
                    meta,
                    [object.x, object.y, object.z],
                );
                Ok(Event::EntitySpawned {
                    entity_id: object.entity_id,
                    name: String::new(),
                })
            }
            play::clientbound::COLLECT_ITEM => {
                let collect = play::parse_collect_item(&packet)?;
                self.entities.remove(collect.collected);
                Ok(Event::EntitiesRemoved(1))
            }
            play::clientbound::ENTITY_RELATIVE_MOVE => {
                let update = play::parse_entity_relative_move(&packet)?;
                self.entities
                    .relative_move(update.entity_id, update.dx, update.dy, update.dz);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_LOOK => {
                let update = play::parse_entity_look(&packet)?;
                self.entities
                    .look(update.entity_id, update.yaw, update.pitch);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_LOOK_AND_RELATIVE_MOVE => {
                let update = play::parse_entity_look_and_relative_move(&packet)?;
                self.entities.relative_move_and_look(
                    update.entity_id,
                    update.dx,
                    update.dy,
                    update.dz,
                    update.yaw,
                    update.pitch,
                );
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_TELEPORT => {
                let teleport = play::parse_entity_teleport(&packet)?;
                self.entities.teleport(
                    teleport.entity_id,
                    [teleport.x, teleport.y, teleport.z],
                    teleport.yaw,
                    teleport.pitch,
                );
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_HEAD_LOOK => {
                let (entity_id, yaw) = play::parse_entity_head_look(&packet)?;
                self.entities.head_look(entity_id, yaw);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_VELOCITY => {
                let update = play::parse_entity_velocity(&packet)?;
                self.entities
                    .set_velocity(update.entity_id, update.vx, update.vy, update.vz);
                Ok(Event::EntityMoved)
            }
            play::clientbound::DESTROY_ENTITIES => {
                let ids = play::parse_destroy_entities(&packet)?;
                self.entities.despawn_all(&ids);
                Ok(Event::EntitiesRemoved(ids.len()))
            }
            play::clientbound::ENTITY => {
                // Keep-alive; the entity is still alive but nothing to update.
                let _ = play::parse_entity(&packet)?;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::ENTITY_METADATA => Ok(Event::Packet(packet.id)),
            play::clientbound::PLAYER_LIST_ITEM => {
                let update = play::parse_player_list_item(&packet)?;
                self.apply_player_list_update(update);
                Ok(Event::PlayerList)
            }
            play::clientbound::BLOCK_ENTITY_DATA => {
                let update = play::parse_block_entity_update(&packet)?;
                let position = (update.x, update.y, update.z);
                let event = Event::BlockEntityUpdated {
                    x: update.x,
                    y: update.y,
                    z: update.z,
                    action: update.action,
                };
                self.block_entities.insert(position, update);
                Ok(event)
            }
            play::clientbound::PLUGIN_MESSAGE => {
                let message = play::parse_plugin_message(&packet)?;
                if message.channel == "MC|Brand" {
                    let brand = play::parse_plugin_brand(&message.data)?;
                    self.server_brand = Some(brand.clone());
                    Ok(Event::ServerBrand(brand))
                } else {
                    Ok(Event::PluginMessage {
                        channel: message.channel,
                    })
                }
            }
            play::clientbound::DISCONNECT => {
                Err(Error::Disconnected(play::parse_disconnect(&packet)?))
            }
            id => Ok(Event::Packet(id)),
        }
    }

    fn apply_player_list_update(&mut self, update: play::PlayerListUpdate) {
        match update {
            play::PlayerListUpdate::Add(entries) => {
                for added in entries {
                    self.skins.record(&added.player);
                    self.player_list.insert(
                        added.player.uuid,
                        TabListEntry {
                            name: added.player.name,
                            gamemode: added.gamemode,
                            ping: added.ping,
                            display_name: added.display_name,
                        },
                    );
                }
            }
            play::PlayerListUpdate::UpdateGameMode(updates) => {
                for update in updates {
                    if let Some(entry) = self.player_list.get_mut(&update.uuid) {
                        entry.gamemode = update.gamemode;
                    }
                }
            }
            play::PlayerListUpdate::UpdateLatency(updates) => {
                for update in updates {
                    if let Some(entry) = self.player_list.get_mut(&update.uuid) {
                        entry.ping = update.ping;
                    }
                }
            }
            play::PlayerListUpdate::UpdateDisplayName(updates) => {
                for update in updates {
                    if let Some(entry) = self.player_list.get_mut(&update.uuid) {
                        entry.display_name = update.display_name;
                    }
                }
            }
            play::PlayerListUpdate::Remove(uuids) => {
                for uuid in uuids {
                    self.player_list.remove(&uuid);
                }
            }
            play::PlayerListUpdate::Other => {}
        }
    }

    fn clear_block_entities_in_chunk(&mut self, coord: ChunkCoord) {
        self.block_entities
            .retain(|&(x, _y, z), _| x.div_euclid(16) != coord.x || z.div_euclid(16) != coord.z);
    }

    /// Creative mode (`JoinGame.gamemode`'s low three bits).
    pub fn is_creative(&self) -> bool {
        self.info.gamemode & 0x07 == 1
    }

    /// The stack in the selected hotbar slot.
    pub fn held_item(&self) -> Option<ItemStack> {
        let slot = play::HOTBAR_FIRST_SLOT + self.held_slot.max(0) as usize;
        self.inventory.get(slot).copied().flatten()
    }

    /// Ask the server to select a hotbar slot. The server confirms with `S09`.
    pub async fn set_held_slot(&mut self, slot: i16) -> Result<()> {
        let slot = slot.clamp(0, (play::HOTBAR_SLOTS - 1) as i16);
        self.held_slot = slot;
        self.connection
            .write_packet(&play::held_item_change(slot))
            .await
    }

    /// Send a `C07 Player Digging` action for `aim`.
    pub async fn dig(&mut self, status: u8, aim: &Aim) -> Result<()> {
        let packet = play::player_digging(status, aim.x, aim.y, aim.z, aim.face);
        self.connection.write_packet(&packet).await
    }

    /// Swing the arm (`C0A`).
    pub async fn swing(&mut self) -> Result<()> {
        self.connection.write_packet(&play::swing_arm()).await
    }

    /// Right-click `aim`: place the held stack against the hit face.
    ///
    /// Returns the position the block would land on, or `None` when nothing
    /// placeable is held.
    pub async fn place(&mut self, aim: &Aim) -> Result<Option<(i32, i32, i32)>> {
        let Some(held) = self.held_item() else {
            return Ok(None);
        };
        if held.block_state().is_none() {
            return Ok(None);
        }
        let normal = nephele_world::Face::from_protocol_id(aim.face)
            .map(|face| face.normal())
            .unwrap_or([0, 1, 0]);
        let target = (aim.x + normal[0], aim.y + normal[1], aim.z + normal[2]);

        let packet = play::block_placement(aim.x, aim.y, aim.z, aim.face, aim.cursor, Some(&held));
        self.connection.write_packet(&packet).await?;
        self.connection.write_packet(&play::swing_arm()).await?;
        Ok(Some(target))
    }

    /// Take the next `C0E` action number and bump the counter.
    fn next_click_action(&mut self, window: i8) -> i16 {
        take_click_action(&mut self.click_actions, window)
    }

    /// Window id clicks should target: the open container, else the player's.
    fn active_window(&self) -> i8 {
        self.open_window
            .as_ref()
            .map(|open| open.id as i8)
            .unwrap_or(0)
    }

    /// Ask the server to close the open container (`C0D`).
    pub async fn close_container(&mut self) -> Result<()> {
        let Some(id) = self.open_window.as_ref().map(|open| open.id) else {
            return Ok(());
        };
        self.connection
            .write_packet(&play::close_window(id, true))
            .await?;
        self.open_window = None;
        Ok(())
    }

    /// Send a `C0E` click in the player container (window 0).
    ///
    /// Returns the action number, which comes back in `S32 Confirm Transaction`.
    /// The packet carries *our* copy of the clicked slot, not the stack under
    /// the cursor: 1.8's server compares it against its own and rejects the
    /// click when they differ (then resends the slot), which is the stale
    /// inventory guard.
    pub async fn click_slot(&mut self, slot: i16, button: i8, mode: i8) -> Result<i16> {
        let clicked = clicked_stack(&self.inventory, slot);
        self.click_slot_with(slot, button, mode, clicked).await
    }

    /// [`GameClient::click_slot`] with the caller's predicted copy of the
    /// clicked slot. The UI predicts clicks between server ticks, so it must
    /// pass its own copy - sending `self.inventory` would report a stack the
    /// server has already replaced and get the click rejected.
    pub async fn click_slot_with(
        &mut self,
        slot: i16,
        button: i8,
        mode: i8,
        clicked: Option<ItemStack>,
    ) -> Result<i16> {
        let window = self.active_window();
        let action = self.next_click_action(window);
        self.connection
            .write_packet(&play::click_window(
                window as u8,
                slot,
                button,
                action,
                mode,
                clicked.as_ref(),
            ))
            .await?;
        Ok(action)
    }

    /// Right-click without placing: `C08` with an empty held slot. Servers use
    /// this to open crafting tables, furnaces and chests.
    pub async fn use_block(&mut self, aim: &Aim) -> Result<()> {
        let packet = play::block_placement(aim.x, aim.y, aim.z, aim.face, aim.cursor, None);
        self.connection.write_packet(&packet).await?;
        self.connection.write_packet(&play::swing_arm()).await?;
        Ok(())
    }

    /// Put `item` into a player-container slot (creative only). `slot` uses
    /// container numbering, so the hotbar is 36..=44.
    pub async fn creative_set_slot(&mut self, slot: i16, item: Option<ItemStack>) -> Result<()> {
        self.connection
            .write_packet(&play::creative_set_slot(slot, item.as_ref()))
            .await?;
        if slot >= 0 && (slot as usize) < self.inventory.len() {
            self.inventory[slot as usize] = item;
        }
        Ok(())
    }

    /// Fill the hotbar with a starter palette (creative mode only). Vanilla's
    /// creative client does the same thing from the inventory screen.
    pub async fn give_starter_hotbar(&mut self) -> Result<()> {
        if !self.is_creative() {
            return Ok(());
        }
        for (index, state) in STARTER_HOTBAR.iter().enumerate() {
            let slot = (play::HOTBAR_FIRST_SLOT + index) as i16;
            let item = ItemStack::new(state.id() as i16, 64, state.meta() as i16);
            self.creative_set_slot(slot, Some(item)).await?;
        }
        Ok(())
    }

    /// Re-send client settings, e.g. after the player changes render distance.
    pub async fn send_client_settings(&mut self, view_distance: u8) -> Result<()> {
        self.connection
            .write_packet(&play::client_settings(
                "en_US",
                view_distance,
                0,
                true,
                0x7F,
            ))
            .await?;
        self.view_distance = view_distance;
        Ok(())
    }

    /// Ask the server to respawn us. Vanilla calls this "perform respawn";
    /// the death screen's button is the only caller.
    pub async fn respawn(&mut self) -> Result<()> {
        self.connection.write_packet(&play::client_status(0)).await
    }

    pub async fn send_chat(&mut self, message: &str) -> Result<()> {
        self.connection.send_chat(message).await
    }

    /// Send a position update (used once client-side movement exists).
    pub async fn send_position(&mut self, position: Vec3, on_ground: bool) -> Result<()> {
        self.position = position;
        self.on_ground = on_ground;
        self.connection
            .write_packet(&play::player_position(
                position.x, position.y, position.z, on_ground,
            ))
            .await
    }

    /// Send position *and* look, which is what a walking client does each tick.
    pub async fn send_move(
        &mut self,
        position: Vec3,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) -> Result<()> {
        self.position = position;
        self.yaw = yaw;
        self.pitch = pitch;
        self.on_ground = on_ground;
        self.connection
            .write_packet(&play::player_position_and_look(
                position.x,
                position.y,
                position.z,
                yaw_to_mc(yaw),
                pitch_to_mc(pitch),
                on_ground,
            ))
            .await
    }

    pub async fn disconnect(&mut self) {
        self.connection.shutdown().await;
    }
}

/// Apply one partial section-mask update without dropping sections delivered by
/// other packets for the same chunk. 1.8 servers may split a column across
/// several `S21` packets, and those packets need not arrive in section order.
fn merge_partial_column(existing: Option<&ChunkColumn>, incoming: ChunkColumn) -> ChunkColumn {
    let Some(existing) = existing else {
        return incoming;
    };

    let mut sections = existing.sections().to_vec();
    for (index, update) in incoming.sections().iter().enumerate() {
        if let Some(section) = update {
            sections[index] = Some(section.clone());
        }
    }

    let mut merged = ChunkColumn::from_sections(incoming.x, incoming.z, sections);
    if let Some(biomes) = incoming.biomes().or_else(|| existing.biomes()) {
        merged.set_biomes(biomes);
    }
    merged
}

impl fmt::Debug for GameClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameClient")
            .field("info", &self.info)
            .field("position", &self.position)
            .field("chunks", &self.world.loaded_chunks())
            .finish()
    }
}

/// Take the next action number for `window`, bumping that window's counter.
/// Counters start at zero and wrap rather than overflow.
fn take_click_action(actions: &mut HashMap<i8, i16>, window: i8) -> i16 {
    let next = actions.entry(window).or_insert(0);
    let action = *next;
    *next = action.wrapping_add(1);
    action
}

/// The stack a click reports for `slot`, taken from our own container copy.
/// Negative slots (outside the window) and out-of-range slots report air.
fn clicked_stack(inventory: &[Option<ItemStack>], slot: i16) -> Option<ItemStack> {
    if slot < 0 {
        return None;
    }
    inventory.get(slot as usize).copied().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    async fn test_game_client() -> (GameClient, Connection) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = listener.local_addr().unwrap();
        let (client_stream, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(socket), listener.accept());
        let client_stream = client_stream.unwrap();
        let (server_stream, _) = accepted.unwrap();
        let address = ServerAddress::parse("127.0.0.1").unwrap();

        let client = GameClient {
            connection: Connection::from_stream(client_stream, nephele_protocol::State::Play),
            info: JoinInfo {
                username: "Tester".into(),
                uuid: "00000000000000000000000000000001".into(),
                entity_id: 1,
                gamemode: 0,
                dimension: 0,
                level_type: "default".into(),
                max_players: 20,
                compression: None,
            },
            world: World::new(0),
            position: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            spawned: false,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            xp_bar: 0.0,
            xp_level: 0,
            xp_total: 0,
            world_age: 0,
            time_of_day: 0,
            spawn_position: None,
            inventory: vec![None; play::PLAYER_CONTAINER_SLOTS],
            held_slot: 0,
            click_actions: HashMap::new(),
            open_window: None,
            chat: Vec::new(),
            entities: Entities::new(),
            skins: Skins::new(),
            player_list: HashMap::new(),
            block_entities: HashMap::new(),
            server_brand: None,
            view_distance: VIEW_DISTANCE,
            packets_received: 0,
            chunks_loaded: 0,
            chunks_unloaded: 0,
            address,
        };
        let peer = Connection::from_stream(server_stream, nephele_protocol::State::Play);
        (client, peer)
    }

    #[test]
    fn minecraft_yaw_zero_faces_south() {
        let yaw = yaw_from_mc(0.0);
        // Nephele yaw of PI points towards +Z.
        assert!((yaw - std::f32::consts::PI).abs() < 1e-4);
        assert!(yaw.sin().abs() < 1e-4);
        assert!((-yaw.cos() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn minecraft_yaw_ninety_faces_west() {
        let yaw = yaw_from_mc(90.0);
        assert!((yaw.sin() + 1.0).abs() < 1e-4);
        assert!((-yaw.cos()).abs() < 1e-4);
    }

    #[test]
    fn yaw_round_trips() {
        for degrees in [0.0f32, 45.0, 90.0, 180.0, 270.0, 359.0] {
            let back = yaw_to_mc(yaw_from_mc(degrees));
            assert!((back - degrees).abs() < 1e-3, "{degrees} -> {back}");
        }
    }

    #[test]
    fn click_actions_count_per_window_and_wrap() {
        let mut actions = HashMap::new();
        assert_eq!(take_click_action(&mut actions, 0), 0);
        assert_eq!(take_click_action(&mut actions, 0), 1);
        assert_eq!(take_click_action(&mut actions, 1), 0, "separate window");
        assert_eq!(take_click_action(&mut actions, 0), 2);

        actions.insert(0, i16::MAX);
        assert_eq!(take_click_action(&mut actions, 0), i16::MAX);
        assert_eq!(take_click_action(&mut actions, 0), i16::MIN, "wraps");
    }

    #[test]
    fn clicked_stack_reads_our_own_slot_copy() {
        let mut inventory = vec![None; play::PLAYER_CONTAINER_SLOTS];
        inventory[36] = Some(ItemStack::new(4, 64, 0));
        assert_eq!(
            clicked_stack(&inventory, 36),
            Some(ItemStack::new(4, 64, 0))
        );
        assert_eq!(clicked_stack(&inventory, 37), None, "empty slot");
        assert_eq!(clicked_stack(&inventory, play::SLOT_OUTSIDE), None);
        assert_eq!(clicked_stack(&inventory, 999), None, "out of range");
    }

    #[test]
    fn pitch_sign_is_inverted() {
        assert!(pitch_from_mc(90.0) < 0.0);
        assert!((pitch_to_mc(-FRAC_PI_2) - 90.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn login_finish_advertises_the_nephele_brand() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = listener.local_addr().unwrap();
        let (client_stream, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(socket), listener.accept());
        let client_stream = client_stream.unwrap();
        let (server_stream, _) = accepted.unwrap();
        let mut server = Connection::from_stream(server_stream, nephele_protocol::State::Play);
        let server_task = tokio::spawn(async move {
            let mut join = nephele_protocol::PacketWriter::new();
            join.write_i32(1)
                .write_u8(0)
                .write_i8(0)
                .write_u8(1)
                .write_u8(20)
                .write_string("default")
                .write_bool(false);
            server
                .write_packet(&Packet::new(
                    play::clientbound::JOIN_GAME,
                    join.into_bytes(),
                ))
                .await
                .unwrap();

            let settings = server.read_packet().await.unwrap();
            assert_eq!(settings.id, play::serverbound::CLIENT_SETTINGS);
            let brand = server.read_packet().await.unwrap();
            let brand = play::parse_plugin_message(&brand).unwrap();
            assert_eq!(brand.channel, "MC|Brand");
            assert_eq!(play::parse_plugin_brand(&brand.data).unwrap(), "Nephele");
            let status = server.read_packet().await.unwrap();
            assert_eq!(status.id, play::serverbound::CLIENT_STATUS);
            assert_eq!(status.reader().read_varint().unwrap(), 0);
        });

        let address = ServerAddress::parse("127.0.0.1").unwrap();
        let client = GameClient::finish(
            Connection::from_stream(client_stream, nephele_protocol::State::Play),
            &address,
            "00000000000000000000000000000001".into(),
            "Tester".into(),
        )
        .await
        .unwrap();
        assert_eq!(client.info.username, "Tester");
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn server_brand_and_block_entity_packets_are_retained() {
        let (mut client, _peer) = test_game_client().await;

        let mut brand_data = nephele_protocol::PacketWriter::new();
        brand_data.write_string("Paper");
        let mut plugin = nephele_protocol::PacketWriter::new();
        plugin
            .write_string("MC|Brand")
            .write_bytes(brand_data.as_slice());
        let event = client
            .handle(Packet::new(
                play::clientbound::PLUGIN_MESSAGE,
                plugin.into_bytes(),
            ))
            .await
            .unwrap();
        assert_eq!(event, Event::ServerBrand("Paper".into()));
        assert_eq!(client.server_brand.as_deref(), Some("Paper"));

        let mut block_entity = nephele_protocol::PacketWriter::new();
        play::write_position(&mut block_entity, -1, 70, 16);
        block_entity
            .write_u8(6)
            .write_u8(10)
            .write_u16(0)
            .write_u8(0);
        let event = client
            .handle(Packet::new(
                play::clientbound::BLOCK_ENTITY_DATA,
                block_entity.into_bytes(),
            ))
            .await
            .unwrap();
        assert_eq!(
            event,
            Event::BlockEntityUpdated {
                x: -1,
                y: 70,
                z: 16,
                action: 6,
            }
        );
        assert_eq!(client.block_entities[&(-1, 70, 16)].action, 6);
    }

    #[tokio::test]
    async fn partial_chunk_sections_merge_when_they_arrive_out_of_order() {
        let (mut client, _peer) = test_game_client().await;
        let mut upper = ChunkColumn::empty(-2, 3);
        upper.set_block(1, 130, 2, nephele_world::BlockState::STONE);
        let mut lower = ChunkColumn::empty(-2, 3);
        lower.set_block(1, 20, 2, nephele_world::BlockState::DIRT);

        client
            .handle(nephele_world::encode_chunk_data(&upper, true, false))
            .await
            .unwrap();
        client
            .handle(nephele_world::encode_chunk_data(&lower, true, false))
            .await
            .unwrap();

        assert_eq!(client.world.loaded_chunks(), 1);
        assert_eq!(
            client.world.get_block(-31, 130, 50),
            nephele_world::BlockState::STONE
        );
        assert_eq!(
            client.world.get_block(-31, 20, 50),
            nephele_world::BlockState::DIRT
        );
    }

    #[tokio::test]
    async fn respawn_clears_dimension_state_and_resends_view_distance() {
        let (mut client, mut peer) = test_game_client().await;
        client.send_client_settings(5).await.unwrap();
        let first_settings = peer.read_packet().await.unwrap();
        assert_eq!(first_settings.id, play::serverbound::CLIENT_SETTINGS);

        client.world.insert(ChunkColumn::empty(0, 0));
        client
            .entities
            .spawn_mob(7, 1, false, [1.0, 64.0, 1.0], 0.0, 0.0);
        client.block_entities.insert(
            (1, 64, 1),
            play::BlockEntityUpdate {
                x: 1,
                y: 64,
                z: 1,
                action: 1,
                nbt: vec![0],
            },
        );
        client.spawned = true;
        client.position = Vec3::new(1.0, 64.0, 1.0);
        client.spawn_position = Some((1, 64, 1));
        client.world_age = 99;
        client.time_of_day = 1200;
        client.open_window = Some(OpenWindowState {
            id: 2,
            kind: "minecraft:chest".into(),
            slots: Vec::new(),
        });

        let mut w = nephele_protocol::PacketWriter::new();
        w.write_i8(-1).write_u8(3).write_u8(1).write_string("flat");
        let event = client
            .handle(Packet::new(play::clientbound::RESPAWN, w.into_bytes()))
            .await
            .unwrap();
        assert_eq!(event, Event::Respawn { dimension: -1 });

        let settings = peer.read_packet().await.unwrap();
        assert_eq!(settings.id, play::serverbound::CLIENT_SETTINGS);
        let mut settings_reader = settings.reader();
        assert_eq!(settings_reader.read_string().unwrap(), "en_US");
        assert_eq!(settings_reader.read_u8().unwrap(), 5);

        assert_eq!(client.info.dimension, -1);
        assert_eq!(client.info.gamemode, 1);
        assert_eq!(client.info.level_type, "flat");
        assert!(!client.world.has_skylight());
        assert_eq!(client.world.loaded_chunks(), 0);
        assert!(client.entities.is_empty());
        assert!(client.block_entities.is_empty());
        assert!(!client.spawned);
        assert_eq!(client.position, Vec3::default());
        assert_eq!(client.spawn_position, None);
        assert_eq!((client.world_age, client.time_of_day), (0, 0));
        assert!(client.open_window.is_none());
    }

    #[tokio::test]
    async fn a_respawn_in_the_same_dimension_keeps_the_loaded_chunks() {
        let (mut client, mut peer) = test_game_client().await;
        client.world.insert(ChunkColumn::empty(0, 0));

        let mut w = nephele_protocol::PacketWriter::new();
        w.write_i8(0)
            .write_u8(3)
            .write_u8(1)
            .write_string("default");
        let event = client
            .handle(Packet::new(play::clientbound::RESPAWN, w.into_bytes()))
            .await
            .unwrap();
        assert_eq!(event, Event::Respawn { dimension: 0 });
        // The client still sends the settings packet, as vanilla does.
        let settings = peer.read_packet().await.unwrap();
        assert_eq!(settings.id, play::serverbound::CLIENT_SETTINGS);

        assert_eq!(client.world.loaded_chunks(), 1);
        assert!(client.world.has_skylight());
        assert!(!client.spawned);
        assert_eq!(client.position, Vec3::default());
    }

    #[tokio::test]
    async fn tab_list_actions_keep_rows_current() {
        let (mut client, _peer) = test_game_client().await;
        let uuid = [9; 16];
        client.apply_player_list_update(play::PlayerListUpdate::Add(vec![
            play::PlayerListAddEntry {
                player: play::PlayerListEntry {
                    uuid,
                    name: "Alex".into(),
                    properties: Vec::new(),
                },
                gamemode: 0,
                ping: 35,
                display_name: None,
            },
        ]));
        assert_eq!(client.player_list[&uuid].name, "Alex");
        assert_eq!(client.skins.get(&uuid).unwrap().name, "Alex");

        client.apply_player_list_update(play::PlayerListUpdate::UpdateLatency(vec![
            play::PlayerListLatencyUpdate { uuid, ping: 82 },
        ]));
        client.apply_player_list_update(play::PlayerListUpdate::UpdateGameMode(vec![
            play::PlayerListGameModeUpdate { uuid, gamemode: 1 },
        ]));
        client.apply_player_list_update(play::PlayerListUpdate::UpdateDisplayName(vec![
            play::PlayerListDisplayNameUpdate {
                uuid,
                display_name: Some(r#"{"text":"A"}"#.into()),
            },
        ]));
        assert_eq!(
            client.player_list[&uuid],
            TabListEntry {
                name: "Alex".into(),
                gamemode: 1,
                ping: 82,
                display_name: Some(r#"{"text":"A"}"#.into()),
            }
        );

        client.apply_player_list_update(play::PlayerListUpdate::Remove(vec![uuid]));
        assert!(!client.player_list.contains_key(&uuid));
    }

    #[tokio::test]
    #[ignore = "manual online acceptance; needs a cached Microsoft session and public network"]
    async fn joins_a_public_protocol_47_server_with_the_cached_session() {
        let app_data = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        let session_path = std::path::PathBuf::from(app_data)
            .join("Nephele")
            .join("auth.json");
        let stored = nephele_auth::StoredSession::load(&session_path).expect("cached session");
        let address = ServerAddress::parse("mc.hypixel.net").unwrap();
        let mut client = GameClient::join_online(&address, &stored.session())
            .await
            .expect("online join");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut got_position = false;
        while tokio::time::Instant::now() < deadline {
            let packet = match tokio::time::timeout(
                Duration::from_secs(2),
                client.connection.next_play_packet(),
            )
            .await
            {
                Err(_) => continue,
                Ok(Err(err)) => panic!("reading play packet: {err}"),
                Ok(Ok(packet)) => packet,
            };
            client.packets_received += 1;
            let packet_id = packet.id;
            let event = client
                .handle(packet)
                .await
                .unwrap_or_else(|err| panic!("handling play packet 0x{packet_id:02X}: {err}"));
            if let Event::Position { .. } = event {
                got_position = true;
            }
            if got_position && client.world.loaded_chunks() > 0 {
                break;
            }
        }

        assert!(got_position, "server did not send a spawn position");
        assert!(client.world.loaded_chunks() > 0, "server sent no chunks");
        let walked = Vec3::new(
            client.position.x + 0.1,
            client.position.y,
            client.position.z,
        );
        client
            .send_move(walked, client.yaw, client.pitch, true)
            .await
            .expect("movement update");
        client.disconnect().await;
    }
}
