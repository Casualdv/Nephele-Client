//! `nephele-client` Ã¢â‚¬â€ the headless side of Nephele.
//!
//! Owns the connection, the loaded world and the local player state, and
//! translates play-state packets into structured [`Event`]s. The renderer will
//! consume this later; for now the CLI drives it and prints statistics.

pub mod container;
pub mod entities;
pub mod maps;
pub mod mining;
pub mod physics;
pub mod scene;
pub mod scoreboard;
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
pub use maps::{MapState, Maps};
pub use mining::{Aim, Miner, MiningEvent};
pub use scene::{EntityRender, EntityRenderKind, Nametag, SceneView};
pub use scoreboard::{Objective, Scoreboard, SidebarRow, Team};
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
        relative_flags: u8,
    },
    /// The server set our own motion (launch pads, knockback, falls).
    /// In blocks per second, matching `PlayerState`.
    Velocity {
        vx: f64,
        vy: f64,
        vz: f64,
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
    /// A block did something visible (`S24`). Note blocks use this: `data` is
    /// the note (0..=24) and `block_id` says whether it was a note block.
    BlockAction {
        x: i32,
        y: i32,
        z: i32,
        event: u8,
        data: u8,
        block_id: i32,
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
    /// A scoreboard objective, score, display slot or team changed. The new
    /// state is readable from `GameClient::scoreboard`.
    Scoreboard,
    /// A block entity update (`S35`) was parsed and retained.
    BlockEntityUpdated {
        x: i32,
        y: i32,
        z: i32,
        action: u8,
    },
    /// The server announced its brand on `MC|Brand`.
    ServerBrand(String),
    /// The server offered a resource pack. The UI decides whether to accept.
    ResourcePack {
        url: String,
        hash: String,
    },
    /// An unhandled server plugin channel was received.
    PluginMessage {
        channel: String,
    },
    /// A map's picture or markers were updated (`S34`). A framed map is looked
    /// up by the item's damage value; `map_id` is the id in the packet.
    MapUpdated {
        map_id: i32,
    },
    /// Any other packet; carries its id so callers can log/trace.
    Packet(i32),
    Skipped(i32),
}

/// Default view distance reported to the server, in chunks.
pub const VIEW_DISTANCE: u8 = 8;

/// Server ticks per second. Velocity packets carry blocks per tick; the
/// physics state stores blocks per second.
pub const TICKS_PER_SECOND: f64 = 20.0;

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

/// Last fields actually transmitted by the 20 Hz movement loop.
#[derive(Default)]
struct MovementReport {
    position: Vec3,
    look: (f32, f32),
    ticks_since_position: u8,
}

impl MovementReport {
    fn packet(&mut self, pos: Vec3, yaw: f32, pitch: f32, on_ground: bool) -> Packet {
        let dx = pos.x - self.position.x;
        let dy = pos.y - self.position.y;
        let dz = pos.z - self.position.z;
        let moved = dx * dx + dy * dy + dz * dz > 0.0009 || self.ticks_since_position >= 20;
        let turned = self.look != (yaw, pitch);
        if moved {
            self.position = pos;
            self.ticks_since_position = 0;
        } else {
            self.ticks_since_position += 1;
        }
        self.look = (yaw, pitch);
        match (moved, turned) {
            (true, true) => {
                play::player_position_and_look(pos.x, pos.y, pos.z, yaw, pitch, on_ground)
            }
            (true, false) => play::player_position(pos.x, pos.y, pos.z, on_ground),
            (false, true) => play::player_look(yaw, pitch, on_ground),
            (false, false) => play::player(on_ground),
        }
    }
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
    /// Objectives, scores, display slots and teams (`S3B`-`S3E`).
    pub scoreboard: Scoreboard,
    /// Most recent update-tile-entity NBT, keyed by block position.
    pub block_entities: HashMap<(i32, i32, i32), play::BlockEntityUpdate>,
    /// Every map the server has sent picture data for, keyed by map id.
    pub maps: Maps,
    /// Last metadata seen per entity, so a renderer can read block states, held
    /// items and rotations without re-parsing packets.
    pub entity_metadata: HashMap<i32, play::EntityMetadata>,
    /// The server's advertised `MC|Brand`, when it sends one.
    pub server_brand: Option<String>,
    view_distance: u8,
    movement: MovementReport,
    pub packets_received: u64,
    /// Payload length of the last frame handed to `handle`.
    last_frame_len: usize,
    /// Payload bytes the last handler actually read.
    last_consumed: usize,
    /// Parses that did not consume their whole declared payload. Non-zero here
    /// is the desync class; the connection also prints each occurrence.
    pub payload_mismatches: u64,
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
            scoreboard: Scoreboard::new(),
            block_entities: HashMap::new(),
            maps: Maps::new(),
            entity_metadata: HashMap::new(),
            server_brand: None,
            view_distance: VIEW_DISTANCE,
            packets_received: 0,
            movement: MovementReport::default(),
            last_frame_len: 0,
            last_consumed: 0,
            payload_mismatches: 0,
            chunks_loaded: 0,
            chunks_unloaded: 0,
            address: address.clone(),
        })
    }

    /// Read one packet (waiting at most `timeout`) and apply it.
    ///
    /// The timeout covers *waiting for data*, never a half-read frame: a
    /// cancelled `read_exact` would drop bytes and desynchronise the stream,
    /// which on a real server shows up as a zlib error on the next packet.
    pub async fn tick(&mut self, timeout: Duration) -> Result<Event> {
        let packet = match tokio::time::timeout(timeout, self.connection.next_play_packet()).await {
            Err(_) => return Ok(Event::Idle),
            Ok(Ok(packet)) => packet,
            Ok(Err(err)) => return Err(err),
        };
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

    /// Only this half may be cancelled by the simulation ticker.
    pub async fn receive_frame(&mut self) -> Result<Packet> {
        self.connection.read_packet().await
    }

    /// Loud check for the desync class: a handler that leaves payload bytes
    /// unread. The frame boundary is still intact in this implementation, but
    /// the packet is almost always being misparsed, so name it immediately.
    fn check_payload_consumption(&mut self, packet_id: i32) {
        if self.last_consumed == self.last_frame_len {
            return;
        }
        self.payload_mismatches += 1;
        tracing::warn!(
            target: "nephele::packet",
            packet = format!("0x{packet_id:02X}"),
            consumed = self.last_consumed,
            declared = self.last_frame_len,
            "parser did not consume its whole payload"
        );
        eprintln!(
            "[nephele] payload mismatch: packet 0x{packet_id:02X} consumed {} of {} bytes",
            self.last_consumed, self.last_frame_len
        );
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

    /// Finish dispatch and any server replies before the next simulation tick.
    pub async fn handle(&mut self, packet: Packet) -> Result<Event> {
        self.packets_received += 1;
        self.last_frame_len = packet.data.len();
        let result = self.dispatch(&packet).await;
        self.last_consumed = packet.consumed();
        let skipped = matches!(result, Ok(Event::Skipped(_)));
        self.connection.note_consumed(self.last_consumed, skipped);
        if result.is_err() {
            self.connection
                .dump_trace("play packet handler returned an error");
        } else if !skipped {
            self.check_payload_consumption(packet.id);
        }
        result
    }

    async fn dispatch(&mut self, packet: &Packet) -> Result<Event> {
        match packet.id {
            play::clientbound::KEEP_ALIVE => {
                // `Connection::next_play_packet` has already answered it: the
                // auto-answer lives in the connection so every caller (this
                // client, the replay tooling) stays responsive. Writing a
                // second response here would send the frame twice.
                //
                // Read the id anyway so the payload-consumption check sees the
                // whole one-byte body consumed.
                let _ = play::parse_keep_alive(packet)?;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::RESOURCE_PACK_SEND => {
                let (url, hash) = play::parse_resource_pack_send(packet)?;
                Ok(Event::ResourcePack { url, hash })
            }
            play::clientbound::CHUNK_DATA => {
                let header = play::parse_chunk_data_header(packet)?;
                match decode_chunk_data(packet, self.world.has_skylight()) {
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
                let columns = decode_chunk_bulk(packet)?;
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
                let pos = play::parse_player_position_and_look(packet)?;
                let relative = |flag| pos.flags & flag != 0u8;
                self.position = Vec3::new(
                    pos.x + if relative(1) { self.position.x } else { 0.0 },
                    pos.y + if relative(2) { self.position.y } else { 0.0 },
                    pos.z + if relative(4) { self.position.z } else { 0.0 },
                );
                let yaw = pos.yaw
                    + if relative(8) {
                        yaw_to_mc(self.yaw)
                    } else {
                        0.0
                    };
                let pitch = pos.pitch
                    + if relative(16) {
                        pitch_to_mc(self.pitch)
                    } else {
                        0.0
                    };
                self.yaw = yaw_from_mc(yaw);
                self.pitch = pitch_from_mc(pitch);
                self.spawned = true;
                self.connection
                    .write_packet(&play::player_position_and_look(
                        self.position.x,
                        self.position.y,
                        self.position.z,
                        yaw,
                        pitch,
                        false,
                    ))
                    .await?;
                Ok(Event::Position {
                    position: self.position,
                    yaw: self.yaw,
                    pitch: self.pitch,
                    relative_flags: pos.flags,
                })
            }
            play::clientbound::CHAT_MESSAGE => {
                let (text, _position) = play::parse_chat_message(packet)?;
                self.chat.push(text.clone());
                if self.chat.len() > 512 {
                    self.chat.remove(0);
                }
                Ok(Event::Chat(text))
            }
            play::clientbound::UPDATE_HEALTH => {
                let health = play::parse_update_health(packet)?;
                self.health = health.health;
                self.food = health.food;
                self.saturation = health.saturation;
                Ok(Event::Health {
                    health: health.health,
                    food: health.food,
                })
            }
            play::clientbound::EXPERIENCE => {
                let xp = play::parse_experience(packet)?;
                self.xp_bar = xp.bar;
                self.xp_level = xp.level;
                self.xp_total = xp.total;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::TIME_UPDATE => {
                let time = play::parse_time_update(packet)?;
                self.world_age = time.world_age;
                self.time_of_day = time.time_of_day;
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::SPAWN_POSITION => {
                let pos = play::parse_spawn_position(packet)?;
                self.spawn_position = Some(pos);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::PLAYER_ABILITIES => {
                let abilities = play::parse_player_abilities(packet)?;
                tracing::debug!(
                    flags = abilities.flags,
                    fly = abilities.flying_speed,
                    walk = abilities.walk_speed,
                    "player abilities"
                );
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::RESPAWN => {
                let respawn = play::parse_respawn(packet)?;
                // `World` keys dimensions by the byte-sized ids vanilla uses
                // (-1, 0, 1); the wire carries an int, so narrow it here.
                let dimension = respawn.dimension as i8;
                self.world.set_dimension(dimension);
                self.entities.clear();
                self.block_entities.clear();
                // Map ids and frame entity ids are per-world, like chunk
                // coordinates: a Nether map must not reuse an overworld one.
                self.maps.clear();
                self.entity_metadata.clear();
                // Vanilla builds a fresh `Scoreboard` for the new world; the
                // server re-sends what it wants shown.
                self.scoreboard.clear();
                self.info.dimension = dimension;
                self.info.gamemode = respawn.gamemode;
                self.info.level_type = respawn.level_type;
                self.spawned = false;
                self.movement = MovementReport::default();
                self.position = Vec3::default();
                self.yaw = 0.0;
                self.pitch = 0.0;
                self.on_ground = true;
                self.spawn_position = None;
                self.world_age = 0;
                self.time_of_day = 0;
                self.open_window = None;
                self.click_actions.clear();
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
                    dimension: respawn.dimension as i8,
                })
            }
            play::clientbound::BLOCK_CHANGE => {
                let change = play::parse_block_change(packet)?;
                self.world.set_block(
                    change.x,
                    change.y,
                    change.z,
                    nephele_world::BlockState::from_raw(change.state),
                );
                Ok(Event::BlocksChanged(1))
            }
            play::clientbound::MULTI_BLOCK_CHANGE => {
                let update = play::parse_multi_block_change(packet)?;
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
                let animation = play::parse_block_break_animation(packet)?;
                Ok(Event::BreakAnimation {
                    breaker: animation.breaker_entity_id,
                    x: animation.x,
                    y: animation.y,
                    z: animation.z,
                    stage: animation.stage,
                })
            }
            play::clientbound::BLOCK_ACTION => {
                let action = play::parse_block_action(packet)?;
                Ok(Event::BlockAction {
                    x: action.x,
                    y: action.y,
                    z: action.z,
                    event: action.event,
                    data: action.data,
                    block_id: action.block_id,
                })
            }
            play::clientbound::HELD_ITEM_CHANGE => {
                let slot = play::parse_held_item_change(packet)?;
                self.held_slot = slot.clamp(0, (play::HOTBAR_SLOTS - 1) as i16);
                Ok(Event::HeldSlot(self.held_slot))
            }
            play::clientbound::OPEN_WINDOW => {
                let window = play::parse_open_window(packet)?;
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
                let id = play::parse_close_window(packet)?;
                if self.open_window.as_ref().is_some_and(|open| open.id == id) {
                    self.open_window = None;
                }
                Ok(Event::WindowClosed)
            }
            play::clientbound::SET_SLOT => {
                let update = play::parse_set_slot(packet)?;
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
                let update = play::parse_window_items(packet)?;
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
                let confirm = play::parse_confirm_transaction(packet)?;
                Ok(Event::ConfirmTransaction {
                    action: confirm.action,
                    accepted: confirm.accepted,
                })
            }
            play::clientbound::SPAWN_PLAYER => {
                let spawn = play::parse_spawn_player(packet)?;
                if spawn.entity_id == self.info.entity_id {
                    return Ok(Event::Spawned);
                }
                // Protocol 47's Spawn Player carries no name; it comes from
                // the tab list entry with the same UUID.
                let name = self
                    .skins
                    .get(&spawn.uuid)
                    .map(|info| info.name.clone())
                    .or_else(|| {
                        self.player_list
                            .get(&spawn.uuid)
                            .map(|entry| entry.name.clone())
                    })
                    .unwrap_or_default();
                self.entities.spawn_player(
                    spawn.entity_id,
                    spawn.uuid,
                    name.clone(),
                    [spawn.x, spawn.y, spawn.z],
                    spawn.yaw,
                    spawn.pitch,
                );
                Ok(Event::EntitySpawned {
                    entity_id: spawn.entity_id,
                    name,
                })
            }
            play::clientbound::SPAWN_MOB => {
                let spawn = play::parse_spawn_mob(packet)?;
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
                let object = play::parse_spawn_object(packet)?;
                if object.kind == crate::entities::OBJECT_TYPE_ARMOR_STAND {
                    self.entities
                        .spawn_armor_stand(object.entity_id, [object.x, object.y, object.z]);
                    return Ok(Event::EntitySpawned {
                        entity_id: object.entity_id,
                        name: String::new(),
                    });
                }
                if object.kind == play::OBJECT_TYPE_ITEM_FRAME {
                    // The frame's `data` field is its facing; the framed stack
                    // and rotation arrive later on `S1C`.
                    self.entities.spawn_item_frame(
                        object.entity_id,
                        object.data as u8,
                        [object.x, object.y, object.z],
                    );
                    return Ok(Event::EntitySpawned {
                        entity_id: object.entity_id,
                        name: String::new(),
                    });
                }
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
                let collect = play::parse_collect_item(packet)?;
                self.entities.remove(collect.collected);
                Ok(Event::EntitiesRemoved(1))
            }
            play::clientbound::ENTITY_RELATIVE_MOVE => {
                let update = play::parse_entity_relative_move(packet)?;
                self.entities
                    .relative_move(update.entity_id, update.dx, update.dy, update.dz);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_LOOK => {
                let update = play::parse_entity_look(packet)?;
                self.entities
                    .look(update.entity_id, update.yaw, update.pitch);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_LOOK_AND_RELATIVE_MOVE => {
                let update = play::parse_entity_look_and_relative_move(packet)?;
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
                let teleport = play::parse_entity_teleport(packet)?;
                self.entities.teleport(
                    teleport.entity_id,
                    [teleport.x, teleport.y, teleport.z],
                    teleport.yaw,
                    teleport.pitch,
                );
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_HEAD_LOOK => {
                let (entity_id, yaw) = play::parse_entity_head_look(packet)?;
                self.entities.head_look(entity_id, yaw);
                Ok(Event::EntityMoved)
            }
            play::clientbound::ENTITY_VELOCITY => {
                let update = play::parse_entity_velocity(packet)?;
                if update.entity_id == self.info.entity_id {
                    // Launch pads, knockback and server-driven falls arrive as
                    // raw motion on our own entity. The entity table only
                    // tracks other players, so hand this to the physics body.
                    self.on_ground = false;
                    return Ok(Event::Velocity {
                        vx: update.vx * TICKS_PER_SECOND,
                        vy: update.vy * TICKS_PER_SECOND,
                        vz: update.vz * TICKS_PER_SECOND,
                    });
                }
                self.entities
                    .set_velocity(update.entity_id, update.vx, update.vy, update.vz);
                Ok(Event::EntityMoved)
            }
            play::clientbound::DESTROY_ENTITIES => {
                let ids = play::parse_destroy_entities(packet)?;
                self.entities.despawn_all(&ids);
                for id in &ids {
                    self.entity_metadata.remove(id);
                }
                Ok(Event::EntitiesRemoved(ids.len()))
            }
            play::clientbound::ENTITY => {
                // Keep-alive; the entity is still alive, and its metadata
                // trailer is sometimes the only place a change shows up.
                let (entity_id, metadata) = play::parse_entity(packet)?;
                self.apply_entity_metadata(entity_id, metadata);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::ENTITY_EQUIPMENT => {
                let update = play::parse_entity_equipment(packet)?;
                self.entities
                    .apply_equipment(update.entity_id, update.slot, update.item);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::ANIMATION => {
                let animation = play::parse_entity_animation(packet)?;
                self.entities
                    .play_animation(animation.entity_id, animation.animation);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::ENTITY_METADATA => {
                let (entity_id, metadata) = play::parse_entity_metadata(packet)?;
                self.apply_entity_metadata(entity_id, metadata);
                Ok(Event::Packet(packet.id))
            }
            play::clientbound::MAP => {
                let update = play::parse_map_update(packet)?;
                self.maps.apply(&update);
                Ok(Event::MapUpdated {
                    map_id: update.map_id,
                })
            }
            play::clientbound::PLAYER_LIST_ITEM => {
                let update = play::parse_player_list_item(packet)?;
                self.apply_player_list_update(update);
                Ok(Event::PlayerList)
            }
            play::clientbound::SCOREBOARD_OBJECTIVE => {
                self.scoreboard
                    .apply_objective(play::parse_scoreboard_objective(packet)?);
                Ok(Event::Scoreboard)
            }
            play::clientbound::UPDATE_SCORE => {
                self.scoreboard
                    .apply_score(play::parse_update_score(packet)?);
                Ok(Event::Scoreboard)
            }
            play::clientbound::DISPLAY_SCOREBOARD => {
                self.scoreboard
                    .apply_display(play::parse_display_scoreboard(packet)?);
                Ok(Event::Scoreboard)
            }
            play::clientbound::TEAMS => {
                self.scoreboard.apply_team(play::parse_teams(packet)?);
                Ok(Event::Scoreboard)
            }
            play::clientbound::BLOCK_ENTITY_DATA => {
                let update = play::parse_block_entity_update(packet)?;
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
                let message = play::parse_plugin_message(packet)?;
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
                Err(Error::Disconnected(play::parse_disconnect(packet)?))
            }
            play::clientbound::SET_COMPRESSION => {
                let threshold = nephele_protocol::packets::login::parse_set_compression(packet)?;
                self.connection.set_compression_threshold(Some(threshold));
                Ok(Event::Packet(packet.id))
            }
            id => Ok(Event::Skipped(id)),
        }
    }

    pub async fn answer_resource_pack(&mut self, hash: &str, accept: bool) -> Result<()> {
        use play::resource_pack_status::{ACCEPTED, DECLINED, FAILED};
        let status = if accept { ACCEPTED } else { DECLINED };
        self.connection
            .write_packet(&play::resource_pack_status(hash, status))
            .await?;
        if accept {
            // The renderer cannot mount server packs yet.
            self.connection
                .write_packet(&play::resource_pack_status(hash, FAILED))
                .await?;
        }
        Ok(())
    }

    /// Read-only view of everything a renderer draws this frame.
    ///
    /// Borrowed, so calling it per frame costs nothing and the renderer never
    /// takes ownership of game state.
    pub fn scene(&self) -> SceneView<'_> {
        SceneView::new(&self.entities, &self.maps, &self.entity_metadata)
    }

    /// The plate to draw above `entity`, with the team prefix/suffix and the
    /// tab-list ping that only the client-level state knows about.
    ///
    /// Vanilla rules: an invisible player or mob hides its plate, an invisible
    /// armour stand keeps it (the hologram *is* the name), a custom name is
    /// gated by metadata index 3, and a team's `name_tag_visibility` is resolved
    /// relative to the local player's team.
    pub fn nametag(&self, entity: &EntityRender<'_>) -> Option<Nametag> {
        if entity.invisible() && !entity.is_armor_stand() {
            return None;
        }
        let name = if entity.is_player() {
            entity
                .custom_name()
                .or_else(|| entity.name())
                .unwrap_or_default()
        } else {
            if !entity.custom_name_visible() {
                return None;
            }
            entity.custom_name()?
        };
        if name.is_empty() {
            return None;
        }
        let mut tag = Nametag {
            name: name.to_string(),
            ..Nametag::default()
        };
        if let EntityRenderKind::Player {
            name: player_name,
            uuid,
            ..
        } = entity.kind
        {
            if let Some(team) = self.scoreboard.team_for(player_name) {
                if !self.can_see_name_tag(team) {
                    return None;
                }
                tag.prefix.clone_from(&team.prefix);
                tag.suffix.clone_from(&team.suffix);
            }
            if let Some(entry) = self.player_list.get(&uuid) {
                // 0 means "no measurement yet" on most servers.
                if entry.ping > 0 {
                    tag.ping = Some(entry.ping.min(u16::MAX as i32) as u16);
                }
            }
        }
        Some(tag)
    }

    fn can_see_name_tag(&self, team: &Team) -> bool {
        let viewer = self.scoreboard.team_name_for(&self.info.username);
        let same_team = viewer.is_some_and(|viewer| viewer == team.name);
        match team.name_tag_visibility.as_str() {
            "hideForOtherTeams" => !same_team,
            "hideForOwnTeam" => same_team,
            _ => true,
        }
    }

    /// Keep the last metadata seen for an entity, and fold the two entries an
    /// item frame uses into its entity record.
    ///
    /// Index 8 is the framed stack and index 9 is its rotation in 45-degree
    /// steps; both are item-frame-specific (`EntityItemFrame.entityInit`), but
    /// storing the whole map means a renderer can read block states, held items
    /// and rotations for anything else too.
    fn apply_entity_metadata(&mut self, entity_id: i32, metadata: play::EntityMetadata) {
        let item = match metadata.get(&play::metadata_index::ITEM) {
            Some(play::MetadataValue::Slot(stack)) => Some(*stack),
            _ => None,
        };
        let rotation = match metadata.get(&play::metadata_index::ITEM_ROTATION) {
            Some(play::MetadataValue::Byte(value)) => Some(*value as u8),
            _ => None,
        };
        if item.is_some() || rotation.is_some() {
            self.entities.set_item_frame(entity_id, item, rotation);
        }
        if let Some(status) = metadata.byte(play::metadata_index::ARMOR_STAND_STATUS) {
            self.entities
                .set_armor_stand_status(entity_id, status as u8);
        }
        if let Some(health) = metadata.float(play::metadata_index::HEALTH) {
            self.entities.set_health(entity_id, health);
        }
        // This server uses index 6 for head yaw in byte (1/256 turn) or
        // `Rotation` form; index 11 is the vanilla armour-stand head pose.
        // A `Float` at index 6 is health, handled above.
        let head_yaw = match metadata.get(&play::metadata_index::HEALTH) {
            Some(play::MetadataValue::Byte(turn)) => Some(*turn as f32 * (360.0 / 256.0)),
            Some(play::MetadataValue::Rotation([_, yaw, _])) => Some(*yaw),
            _ => None,
        }
        .or_else(|| {
            metadata
                .rotation(play::metadata_index::HEAD_ROTATION)
                .map(|rotation| rotation[1])
        });
        if let Some(yaw) = head_yaw {
            self.entities.head_look(entity_id, yaw);
        }
        self.entity_metadata.insert(entity_id, metadata);
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
    /// Right-click `aim`: use the block (or empty hand) rather than place.
    pub async fn use_block(&mut self, aim: &Aim) -> Result<()> {
        let packet = play::block_placement(aim.x, aim.y, aim.z, aim.face, aim.cursor, None);
        self.connection.write_packet(&packet).await?;
        self.connection.write_packet(&play::swing_arm()).await?;
        Ok(())
    }

    /// Send `C02 Use Entity`.
    ///
    /// `action` is one of `play::use_entity::{INTERACT, ATTACK, INTERACT_AT}`;
    /// `target` is required only for `INTERACT_AT`, where it is the click
    /// position relative to the entity's own origin, exactly as vanilla sends
    /// a right click on an entity.
    pub async fn use_entity(
        &mut self,
        entity_id: i32,
        action: i32,
        target: Option<[f32; 3]>,
    ) -> Result<()> {
        let packet = play::use_entity(entity_id, action, target);
        self.connection.write_packet(&packet).await
    }

    /// Left click an entity (`C02` action `ATTACK`), then swing.
    pub async fn attack_entity(&mut self, entity_id: i32) -> Result<()> {
        self.use_entity(entity_id, play::use_entity::ATTACK, None)
            .await?;
        self.connection.write_packet(&play::swing_arm()).await
    }

    /// Right click an entity (`C02` action `INTERACT_AT`).
    pub async fn interact_entity_at(&mut self, entity_id: i32, target: [f32; 3]) -> Result<()> {
        self.use_entity(entity_id, play::use_entity::INTERACT_AT, Some(target))
            .await?;
        self.connection.write_packet(&play::swing_arm()).await
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

    /// Report one simulation tick using the smallest protocol-47 movement variant.
    pub async fn send_movement_tick(
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
        let packet = self
            .movement
            .packet(position, yaw_to_mc(yaw), pitch_to_mc(pitch), on_ground);
        self.connection.write_packet(&packet).await
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
        let (client, stream) = test_game_client_with_raw_peer().await;
        (
            client,
            Connection::from_stream(stream, nephele_protocol::State::Play),
        )
    }

    /// The peer end is a bare socket so a test can split a frame across writes.
    async fn test_game_client_with_raw_peer() -> (GameClient, tokio::net::TcpStream) {
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
            scoreboard: Scoreboard::new(),
            block_entities: HashMap::new(),
            maps: Maps::new(),
            entity_metadata: HashMap::new(),
            server_brand: None,
            view_distance: VIEW_DISTANCE,
            packets_received: 0,
            movement: MovementReport::default(),
            last_frame_len: 0,
            last_consumed: 0,
            payload_mismatches: 0,
            chunks_loaded: 0,
            chunks_unloaded: 0,
            address,
        };
        (client, server_stream)
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
    async fn scoreboard_packets_update_client_state() {
        let (mut client, _peer) = test_game_client().await;

        let mut objective = nephele_protocol::PacketWriter::new();
        objective
            .write_string("board")
            .write_u8(0)
            .write_string("§e§lTEST")
            .write_string("integer");
        assert_eq!(
            client
                .handle(Packet::new(
                    play::clientbound::SCOREBOARD_OBJECTIVE,
                    objective.into_bytes(),
                ))
                .await
                .unwrap(),
            Event::Scoreboard
        );

        let mut display = nephele_protocol::PacketWriter::new();
        display
            .write_u8(play::scoreboard_slot::SIDEBAR)
            .write_string("board");
        client
            .handle(Packet::new(
                play::clientbound::DISPLAY_SCOREBOARD,
                display.into_bytes(),
            ))
            .await
            .unwrap();

        let mut team = nephele_protocol::PacketWriter::new();
        team.write_string("red")
            .write_u8(0)
            .write_string("Red")
            .write_string("§c[RED] ")
            .write_string("")
            .write_u8(3)
            .write_string("always")
            .write_i8(12)
            .write_varint(1)
            .write_string("Steve");
        client
            .handle(Packet::new(play::clientbound::TEAMS, team.into_bytes()))
            .await
            .unwrap();

        let mut score = nephele_protocol::PacketWriter::new();
        score
            .write_string("Steve")
            .write_varint(0)
            .write_string("board")
            .write_varint(5);
        client
            .handle(Packet::new(
                play::clientbound::UPDATE_SCORE,
                score.into_bytes(),
            ))
            .await
            .unwrap();

        let rows = client.scoreboard.sidebar_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text(), "§c[RED] Steve");
        assert_eq!(rows[0].color, Some(12));
        assert_eq!(client.scoreboard.prefix_for("Steve"), Some("§c[RED] "));
    }

    #[tokio::test]
    async fn block_action_packets_become_events() {
        let (mut client, _peer) = test_game_client().await;
        let mut action = nephele_protocol::PacketWriter::new();
        play::write_position(&mut action, 4, 70, -9);
        action.write_u8(0).write_u8(12).write_varint(25);
        let event = client
            .handle(Packet::new(
                play::clientbound::BLOCK_ACTION,
                action.into_bytes(),
            ))
            .await
            .unwrap();
        assert_eq!(
            event,
            Event::BlockAction {
                x: 4,
                y: 70,
                z: -9,
                event: 0,
                data: 12,
                block_id: 25,
            }
        );
    }

    #[tokio::test]
    async fn right_clicking_an_entity_sends_a_well_formed_use_entity() {
        let (mut client, stream) = test_game_client_with_raw_peer().await;
        let mut peer = Connection::from_stream(stream, nephele_protocol::State::Play);

        client
            .interact_entity_at(42, [0.25, 1.5, -0.75])
            .await
            .unwrap();

        let packet = peer.read_packet().await.unwrap();
        assert_eq!(packet.id, play::serverbound::USE_ENTITY);
        let (id, action, target) = play::parse_use_entity(&packet).unwrap();
        assert_eq!((id, action), (42, play::use_entity::INTERACT_AT));
        assert_eq!(target, Some([0.25, 1.5, -0.75]));
        assert_eq!(packet.consumed(), packet.data.len());

        // The swing follows, as it does in vanilla's right-click handler.
        assert_eq!(
            peer.read_packet().await.unwrap().id,
            play::serverbound::ANIMATION
        );
    }

    #[tokio::test]
    async fn answers_keep_alive_with_the_same_id() {
        let (mut client, mut peer) = test_game_client().await;
        peer.write_packet(&Packet::new(play::clientbound::KEEP_ALIVE, vec![0x2a]))
            .await
            .unwrap();
        let event = client
            .tick(Duration::from_secs(1))
            .await
            .expect("keep alive");
        assert_eq!(event, Event::Packet(play::clientbound::KEEP_ALIVE));

        let response = tokio::time::timeout(Duration::from_secs(1), peer.read_packet())
            .await
            .expect("a keep-alive response")
            .expect("response frame");
        assert_eq!(response.id, play::serverbound::KEEP_ALIVE);
        assert_eq!(play::parse_keep_alive(&response).unwrap(), 42);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), peer.read_packet())
                .await
                .is_err()
        );
    }

    /// Replay a real Hypixel play-state capture (join + lobby traffic) through
    /// the full client stack in awkward chunks. This is the regression test for
    /// the "next frame length read out of a payload" class: every frame must
    /// parse, every packet must be handled, and no parser may leave payload
    /// bytes behind.
    #[tokio::test]
    #[ignore = "requires NEPHELE_REPLAY pointing to a private decoded play capture"]
    async fn replays_a_captured_hypixel_session_frame_by_frame() {
        let frames = std::fs::read(std::env::var_os("NEPHELE_REPLAY").expect("NEPHELE_REPLAY"))
            .expect("read private capture");
        let (mut client, peer) = test_game_client_with_raw_peer().await;
        let expected = capture_frame_count(&frames);
        let handled = replay_capture(&mut client, peer, frames).await;

        assert_eq!(handled, expected, "frame count");
        assert_eq!(
            client.payload_mismatches, 0,
            "a parser under-read its payload in the capture"
        );

        // `NEPHELE_SCAN=1` turns the replay into a map dump around the player:
        // finding a launch pad in a captured lobby without walking every lane by
        // hand. Diagnostic only, so it stays off by default.
        if std::env::var_os("NEPHELE_SCAN").is_some() {
            scan_for_launch_pads(&client);
        }
    }

    /// A framed map, rebuilt from a captured Hypixel session.
    ///
    /// The main lobby never spawns item frames (four directional sweeps, ~1700
    /// packets, every `S0E` was an armour stand), so this takes the real
    /// captured spawn-object packet's id, position and angles and replays it as
    /// the item-frame object type, then sends the frame's metadata the way
    /// `EntityTrackerEntry` does. The map id is the filled map's damage value.
    #[tokio::test]
    #[ignore = "requires NEPHELE_REPLAY pointing to a private decoded play capture"]
    async fn a_framed_map_replica_from_captured_bytes_arrives_intact() {
        let frames = std::fs::read(std::env::var_os("NEPHELE_REPLAY").expect("NEPHELE_REPLAY"))
            .expect("read private capture");
        let (mut client, peer) = test_game_client_with_raw_peer().await;
        let captured_spawn = first_capture_payload(&frames, play::clientbound::SPAWN_OBJECT)
            .expect("the capture has a spawn-object packet");
        let captured_metadata = first_capture_payload(&frames, play::clientbound::ENTITY_METADATA)
            .expect("the capture has a metadata packet");
        replay_capture(&mut client, peer, frames).await;

        // Replaying the real metadata proves the S1C parser understands
        // Hypixel's bytes before anything is rebuilt.
        assert!(
            !client.entity_metadata.is_empty(),
            "no entity metadata was retained from the capture"
        );

        let source = play::parse_spawn_object(&Packet::new(
            play::clientbound::SPAWN_OBJECT,
            captured_spawn,
        ))
        .expect("captured spawn object");

        // Same packet, item-frame object type, captured position and angles.
        let frame_id = source.entity_id;
        client
            .handle(play::spawn_object(
                frame_id,
                play::OBJECT_TYPE_ITEM_FRAME,
                [source.x, source.y, source.z],
                source.pitch,
                source.yaw,
                3, // facing east
                [0.0; 3],
            ))
            .await
            .expect("framed-map replica spawn");

        // The framed stack and its rotation, in the captured packet's entity
        // metadata shape (index 8 slot, index 9 byte).
        let map_id: i16 = 91;
        let mut meta = nephele_protocol::PacketWriter::new();
        meta.write_varint(frame_id)
            .write_u8(5 << 5 | 8)
            .write_i16(play::ITEM_FILLED_MAP)
            .write_i8(1)
            .write_i16(map_id)
            .write_u8(0)
            .write_u8(9)
            .write_i8(3)
            .write_u8(0x7F);
        client
            .handle(Packet::new(
                play::clientbound::ENTITY_METADATA,
                meta.into_bytes(),
            ))
            .await
            .expect("framed-map metadata");

        // The captured metadata packet's entity id is reused to prove the
        // metadata path keys on the id the server uses.
        let (captured_entity, _) = play::parse_entity_metadata(&Packet::new(
            play::clientbound::ENTITY_METADATA,
            captured_metadata,
        ))
        .expect("captured metadata");
        assert!(
            client.entity_metadata.contains_key(&captured_entity)
                || client.entity_metadata.contains_key(&frame_id),
            "metadata keyed by the wrong entity"
        );

        let EntityKind::ItemFrame {
            direction,
            item,
            rotation,
        } = &client.entities.get(frame_id).expect("frame").kind
        else {
            panic!("the replica did not spawn an item frame");
        };
        assert_eq!(*direction, 3);
        assert_eq!(
            *item,
            Some(ItemStack::new(play::ITEM_FILLED_MAP, 1, map_id))
        );
        assert_eq!(*rotation, 3);
        eprintln!(
            "framed map replica: entity {frame_id} at ({:.1}, {:.1}, {:.1}), map id {map_id}, rotation {rotation}",
            source.x, source.y, source.z
        );
    }

    /// Drive `/hub` and prove the session is rebuilt, not carried over.
    ///
    /// Hypixel answers a lobby transfer either with `S07 Respawn` (a dimension
    /// change inside one server) or by dropping the connection so the client
    /// reconnects. Both have to end with no chunk, entity, map or metadata state
    /// from the previous lobby, which is what this watches.
    #[tokio::test]
    #[ignore = "requires a cached Microsoft session and a live Hypixel lobby"]
    async fn hub_transfer_rebuilds_the_session() {
        let app_data = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        let session_path = std::path::PathBuf::from(app_data)
            .join("Nephele")
            .join("auth.json");
        let stored = nephele_auth::StoredSession::load(&session_path).expect("cached session");
        let address = ServerAddress::parse("mc.hypixel.net").unwrap();
        let mut client = GameClient::join_online(&address, &stored.session())
            .await
            .expect("online join");

        let settle = tokio::time::Instant::now() + Duration::from_secs(40);
        while tokio::time::Instant::now() < settle
            && (!client.spawned || client.world.loaded_chunks() == 0)
        {
            let _ = client.tick(Duration::from_millis(200)).await.expect("join");
        }
        let chunks_before = client.world.loaded_chunks();
        assert!(chunks_before > 0, "no terrain before the transfer");

        // Plant a sentinel map so a leak is visible, then ask for a transfer.
        client
            .maps
            .apply(&nephele_protocol::packets::play::MapUpdate {
                map_id: 4242,
                scale: 0,
                icons: Vec::new(),
                columns: 1,
                rows: 1,
                start_x: 0,
                start_y: 0,
                colors: vec![9],
            });
        client.send_chat("/hub").await.expect("send /hub");

        let mut respawned = false;
        let mut disconnected = None;
        let mut chunks_after_reset = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        while tokio::time::Instant::now() < deadline {
            match client.tick(Duration::from_millis(500)).await {
                Ok(Event::Respawn { dimension }) => {
                    respawned = true;
                    eprintln!("hub: respawn into dimension {dimension}");
                    assert!(
                        client.maps.is_empty(),
                        "a dimension change kept the previous lobby's maps"
                    );
                    assert!(client.world.loaded_chunks() == 0, "chunks survived respawn");
                    chunks_after_reset = Some(0);
                }
                Ok(Event::Chat(line)) => eprintln!("hub: {line}"),
                Ok(_) => {}
                Err(Error::Disconnected(reason)) => {
                    eprintln!("hub: disconnected: {reason}");
                    disconnected = Some(reason);
                    break;
                }
                Err(err) => panic!("/hub drive failed: {err}"),
            }
            if respawned {
                break;
            }
        }

        // Either path leaves nothing behind: the client is reset, and a fresh
        // connection starts from an empty world.
        assert!(
            respawned || disconnected.is_some(),
            "neither a respawn nor a disconnect arrived; /hub was ignored"
        );
        if respawned {
            assert_eq!(chunks_after_reset, Some(0));
        } else {
            // A dropped connection is exactly the reconnect path the UI takes.
            assert_eq!(client.payload_mismatches, 0);
        }

        let mut fresh = {
            // The first session must be closed before a second join: Hypixel
            // kicks the newer connection when the old one is still online.
            client.disconnect().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            GameClient::join_online(&address, &stored.session())
                .await
                .expect("reconnect after /hub")
        };
        assert_eq!(fresh.world.loaded_chunks(), 0, "reconnect leaked chunks");
        assert!(fresh.maps.is_empty(), "reconnect leaked maps");
        assert!(fresh.entities.is_empty(), "reconnect leaked entities");
        fresh.disconnect().await;
        eprintln!(
            "hub: before {chunks_before} chunks, respawned={respawned}, disconnected={}",
            disconnected.is_some()
        );
    }

    /// First packet with `id` in a capture, as a payload.
    fn first_capture_payload(frames: &[u8], wanted: i32) -> Option<Vec<u8>> {
        let mut cursor = 0usize;
        while cursor < frames.len() {
            let (length, prefix) = capture_frame_length(&frames[cursor..]);
            let start = cursor + prefix;
            let end = start + length;
            if end > frames.len() {
                break;
            }
            if let Ok(packet) = Packet::from_payload(&frames[start..end]) {
                if packet.id == wanted {
                    return Some(packet.data.clone());
                }
                cursor = end;
            } else {
                break;
            }
        }
        None
    }

    /// Bounce off the real lobby launch pad, using the terrain in a capture.
    ///
    /// The Hypixel main lobby's pad is a 3x3 slime block platform, so 1.8's
    /// `BlockSlime.onLanded` is what launches you — not a server velocity
    /// packet. Replaying the capture gives this test the pad's actual position
    /// and the blocks around it instead of a hand-built mock.
    #[tokio::test]
    #[ignore = "requires NEPHELE_REPLAY pointing to a private decoded play capture"]
    async fn a_captured_lobby_slime_pad_bounces_the_player() {
        use crate::physics::PlayerState;

        let frames = std::fs::read(std::env::var_os("NEPHELE_REPLAY").expect("NEPHELE_REPLAY"))
            .expect("read private capture");
        let (mut client, peer) = test_game_client_with_raw_peer().await;
        replay_capture(&mut client, peer, frames).await;

        let mut pads = Vec::new();
        for (coord, _) in client.world.iter() {
            for x in coord.x * 16..coord.x * 16 + 16 {
                for z in coord.z * 16..coord.z * 16 + 16 {
                    for y in 0..255 {
                        if client.world.get_block(x, y, z).id() == 165 {
                            pads.push((x, y, z));
                        }
                    }
                }
            }
        }
        assert!(!pads.is_empty(), "the capture has no slime blocks");

        let count = pads.len() as f64;
        let cx = pads.iter().map(|p| p.0 as f64 + 0.5).sum::<f64>() / count;
        let cz = pads.iter().map(|p| p.2 as f64 + 0.5).sum::<f64>() / count;
        let pad_top = pads
            .iter()
            .map(|p| p.1 as f64 + 1.0)
            .fold(f64::MIN, f64::max);
        eprintln!(
            "pad: {} slime blocks around ({cx:.1}, {pad_top:.0}, {cz:.1})",
            pads.len()
        );

        let mut player = PlayerState::at(cx, pad_top + 4.0, cz);
        player.on_ground = false;

        let mut bounced = false;
        let mut apex: f64 = 0.0;
        for _ in 0..80 {
            player.step(0.05, &crate::physics::Input::default(), 0.0, &client.world);
            if player.vy > 1.0 {
                bounced = true;
            }
            if bounced {
                apex = apex.max(player.y);
            }
        }

        assert!(bounced, "the captured pad did not reflect the fall");
        assert!(
            apex > pad_top + 1.0,
            "bounce reached {apex:.2}, pad top is {pad_top:.2}"
        );
    }

    /// Number of frames in a `NEPHELE_TRACE_RAW` capture.
    fn capture_frame_count(frames: &[u8]) -> usize {
        let mut cursor = 0usize;
        let mut count = 0usize;
        while cursor < frames.len() {
            let (length, prefix) = capture_frame_length(&frames[cursor..]);
            cursor += prefix + length;
            assert!(cursor <= frames.len(), "capture ends inside a frame");
            count += 1;
        }
        count
    }

    /// Feed a captured play stream through the client in awkward chunks.
    ///
    /// Returns the number of frames handled. Every parse has to succeed, so a
    /// desync shows up here as a panic naming the packet it broke on.
    async fn replay_capture(
        client: &mut GameClient,
        peer: tokio::net::TcpStream,
        frames: Vec<u8>,
    ) -> usize {
        use tokio::io::AsyncWriteExt;

        let expected = capture_frame_count(&frames);
        let writer = tokio::spawn(async move {
            let mut peer = peer;
            for (index, chunk) in frames.chunks(53).enumerate() {
                // Pause occasionally so the reader sees partial frames.
                if index % 64 == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                peer.write_all(chunk).await.expect("fixture write");
            }
            peer
        });

        let mut handled = 0usize;
        while handled < expected {
            match client.tick(Duration::from_millis(50)).await {
                Ok(Event::Idle) => {}
                Ok(_) => handled += 1,
                Err(err) => panic!("replay failed after {handled} of {expected} packets: {err}"),
            }
        }

        // Keep the peer open until the loop is done: closing it would make the
        // client's keep-alive replies fail.
        let mut peer = writer.await.expect("writer task");
        peer.shutdown().await.ok();
        handled
    }

    /// Print every plate/slime/piston/redstone block near the captured player.
    fn scan_for_launch_pads(client: &GameClient) {
        const INTERESTING: [u16; 13] = [
            29, 33, 34, 36, // pistons
            41, 152, // gold, redstone
            70, 72, 147, 148, // pressure plates
            131, 132, // tripwire
            165, // slime
        ];
        let px = client.position.x.floor() as i32;
        let py = client.position.y.floor() as i32;
        let pz = client.position.z.floor() as i32;
        const RADIUS: i32 = 48;

        eprintln!(
            "scan: {} chunks loaded, player at ({px}, {py}, {pz})",
            client.world.loaded_chunks()
        );
        let mut found: Vec<(u16, u8, i32, i32, i32)> = Vec::new();
        for x in px - RADIUS..=px + RADIUS {
            for z in pz - RADIUS..=pz + RADIUS {
                for y in (py - 8).max(0)..=(py + 32).min(255) {
                    let state = client.world.get_block(x, y, z);
                    if INTERESTING.contains(&state.id()) {
                        found.push((state.id(), state.meta(), x, y, z));
                    }
                }
            }
        }
        found.sort_by_key(|&(id, _, x, y, z)| (id, y, x, z));
        eprintln!("scan: {} interactive candidates", found.len());
        for (id, meta, x, y, z) in &found {
            // Height above the block below, so a plate sitting on a block is
            // obvious rather than "id 147 in mid-air".
            let below = client.world.get_block(*x, *y - 1, *z);
            eprintln!(
                "scan: id={id} meta={meta} at ({x}, {y}, {z}) on id={} meta={}",
                below.id(),
                below.meta()
            );
        }
    }

    /// Length prefix of one frame in a `NEPHELE_TRACE_RAW` capture.
    fn capture_frame_length(bytes: &[u8]) -> (usize, usize) {
        let mut value = 0usize;
        let mut shift = 0u32;
        for (index, byte) in bytes.iter().copied().enumerate() {
            value |= ((byte & 0x7F) as usize) << shift;
            if byte & 0x80 == 0 {
                return (value, index + 1);
            }
            shift += 7;
            assert!(index < 4, "capture length VarInt is too long");
        }
        panic!("capture ends inside a length prefix");
    }

    #[tokio::test]
    async fn resource_pack_choices_echo_hash_and_report_load_failure() {
        let (mut client, mut peer) = test_game_client().await;
        let mut writer = nephele_protocol::PacketWriter::new();
        writer
            .write_string("http://example.invalid/pack.zip")
            .write_string("0123456789abcdef");

        let event = client
            .handle(Packet::new(
                play::clientbound::RESOURCE_PACK_SEND,
                writer.into_bytes(),
            ))
            .await
            .expect("resource pack");
        let Event::ResourcePack { hash, .. } = event else {
            panic!("missing offer")
        };
        client.answer_resource_pack(&hash, false).await.unwrap();
        client.answer_resource_pack(&hash, true).await.unwrap();
        for expected in [
            play::resource_pack_status::DECLINED,
            play::resource_pack_status::ACCEPTED,
            play::resource_pack_status::FAILED,
        ] {
            let response = peer.read_packet().await.unwrap();
            assert_eq!(response.id, play::serverbound::RESOURCE_PACK_STATUS);
            let mut reader = response.reader();
            assert_eq!(reader.read_string().unwrap(), hash);
            assert_eq!(reader.read_varint().unwrap(), expected);
            assert_eq!(reader.remaining(), 0);
        }
    }

    /// An item frame exists only as `S0E`(71) + two metadata entries; the map
    /// id is the filled map's damage value in 1.8, which is what ties the frame
    /// to the `S34` picture with the same id.
    #[tokio::test]
    async fn an_item_frame_keeps_its_item_rotation_and_map_id() {
        let (mut client, _peer) = test_game_client().await;

        let mut spawn = nephele_protocol::PacketWriter::new();
        spawn
            .write_varint(55)
            .write_u8(play::OBJECT_TYPE_ITEM_FRAME)
            .write_i32(32 * 10)
            .write_i32(32 * 64)
            .write_i32(32 * 3)
            .write_u8(0)
            .write_u8(0)
            .write_i32(1) // facing west; frames always carry a zero velocity
            .write_i16(0)
            .write_i16(0)
            .write_i16(0);
        client
            .handle(Packet::new(
                play::clientbound::SPAWN_OBJECT,
                spawn.into_bytes(),
            ))
            .await
            .expect("spawn frame");

        assert!(matches!(
            client.entities.get(55).expect("frame").kind,
            EntityKind::ItemFrame {
                direction: 1,
                item: None,
                rotation: 0
            }
        ));

        let mut meta = nephele_protocol::PacketWriter::new();
        meta.write_varint(55)
            .write_u8(5 << 5 | 8) // slot: filled map, damage is the map id
            .write_i16(play::ITEM_FILLED_MAP)
            .write_i8(1)
            .write_i16(91)
            .write_u8(0)
            .write_u8(9) // rotation
            .write_i8(2)
            .write_u8(0x7F);
        client
            .handle(Packet::new(
                play::clientbound::ENTITY_METADATA,
                meta.into_bytes(),
            ))
            .await
            .expect("frame metadata");

        match &client.entities.get(55).unwrap().kind {
            EntityKind::ItemFrame { item, rotation, .. } => {
                assert_eq!(*item, Some(ItemStack::new(play::ITEM_FILLED_MAP, 1, 91)));
                assert_eq!(*rotation, 2);
            }
            other => panic!("not a frame: {other:?}"),
        }
        // The whole metadata map is retained for the renderer, not just the
        // frame fields.
        assert!(client.entity_metadata[&55].contains_key(&8));

        // Emptying the frame is a real update, not a no-op.
        let mut cleared = nephele_protocol::PacketWriter::new();
        cleared
            .write_varint(55)
            .write_u8(5 << 5 | 8)
            .write_i16(ItemStack::EMPTY_ID)
            .write_u8(0x7F);
        client
            .handle(Packet::new(
                play::clientbound::ENTITY_METADATA,
                cleared.into_bytes(),
            ))
            .await
            .expect("clear frame");
        match &client.entities.get(55).unwrap().kind {
            EntityKind::ItemFrame { item, .. } => assert_eq!(*item, None),
            other => panic!("not a frame: {other:?}"),
        }
    }

    #[tokio::test]
    async fn map_patches_reach_the_store_through_the_packet_handler() {
        let (mut client, _peer) = test_game_client().await;

        let mut packet = nephele_protocol::PacketWriter::new();
        packet
            .write_varint(44)
            .write_u8(1)
            .write_varint(0)
            .write_u8(1) // one column
            .write_u8(1) // one row
            .write_u8(2) // start x
            .write_u8(3) // start y
            .write_varint(1)
            .write_u8(7);
        let event = client
            .handle(Packet::new(play::clientbound::MAP, packet.into_bytes()))
            .await
            .expect("map");
        assert_eq!(event, Event::MapUpdated { map_id: 44 });

        let map = client.maps.get(44).expect("map state");
        assert_eq!(map.color_at(2, 3), Some(7));
        assert_eq!(map.patches, 1);
    }

    #[tokio::test]
    async fn a_short_parse_is_counted_as_a_payload_mismatch() {
        let (mut client, _peer) = test_game_client().await;
        client.last_frame_len = 10;
        client.last_consumed = 4;
        client.check_payload_consumption(0x02);
        assert_eq!(client.payload_mismatches, 1);

        // Unknown packets are explicitly skipped. Known packets with zero consumption still count.
        client.last_consumed = 10;
        client.check_payload_consumption(0x02);
        client.handle(Packet::new(0x7f, vec![1])).await.unwrap();
        assert_eq!(client.payload_mismatches, 1);
        client.check_payload_consumption(0x02);
        assert_eq!(client.payload_mismatches, 2);
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
        w.write_i32(-1).write_u8(3).write_u8(1).write_string("flat");
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
    async fn a_tick_that_times_out_mid_frame_keeps_its_bytes() {
        use tokio::io::AsyncWriteExt;

        let (mut client, mut peer) = test_game_client_with_raw_peer().await;

        let mut w = nephele_protocol::PacketWriter::new();
        w.write_string(r#"{"text":"split"}"#).write_u8(0);
        let packet = Packet::new(play::clientbound::CHAT_MESSAGE, w.into_bytes());
        let frame = nephele_protocol::framing::encode_frame(&packet.to_payload(), None).unwrap();
        let (head, tail) = frame.split_at(frame.len() / 2);
        let head = head.to_vec();
        let tail = tail.to_vec();

        peer.write_all(&head).await.unwrap();
        peer.flush().await.unwrap();

        // The tick's own timeout fires with half a frame buffered, so it
        // returns `Idle`. Cancelling must not consume `head`: the connection
        // buffers raw bytes and only drains a frame once all of it is present,
        // so the next tick finishes the packet instead of desynchronising.
        let (event, ()) = tokio::join!(client.tick(Duration::from_millis(20)), async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            peer.write_all(&tail).await.unwrap();
            peer.flush().await.unwrap();
        });
        assert_eq!(event.expect("tick"), Event::Idle);

        assert_eq!(
            client.tick(Duration::from_secs(2)).await.expect("tick"),
            Event::Chat("split".into())
        );

        // The stream is still framed: another whole packet must decode.
        let mut w = nephele_protocol::PacketWriter::new();
        w.write_string(r#"{"text":"after"}"#).write_u8(0);
        let packet = Packet::new(play::clientbound::CHAT_MESSAGE, w.into_bytes());
        let frame = nephele_protocol::framing::encode_frame(&packet.to_payload(), None).unwrap();
        peer.write_all(&frame).await.unwrap();
        peer.flush().await.unwrap();
        assert_eq!(
            client.tick(Duration::from_secs(2)).await.expect("tick"),
            Event::Chat("after".into())
        );
    }

    #[tokio::test]
    async fn walking_updates_match_the_wire_cadence() {
        use play::serverbound::*;
        let (mut client, mut peer) = test_game_client().await;
        let mut pos = Vec3::new(0.0, 65.0, 0.0);
        let yaw = yaw_from_mc(0.0);
        for tick in 0..=26 {
            let expected = match tick {
                0 | 21 => PLAYER_POSITION,
                22 => PLAYER_LOOK,
                23 => {
                    pos.z += 0.1;
                    PLAYER_POSITION
                }
                24 => {
                    pos.y += 0.42;
                    PLAYER_POSITION
                }
                25 => {
                    pos.x += 0.02;
                    PLAYER
                }
                26 => {
                    pos.x += 0.02;
                    PLAYER_POSITION_AND_LOOK
                }
                _ => PLAYER,
            };
            let look = if tick == 26 {
                yaw + 0.2
            } else if tick >= 22 {
                yaw + 0.1
            } else {
                yaw
            };
            let grounded = tick < 24;
            client
                .send_movement_tick(pos, look, 0.0, grounded)
                .await
                .unwrap();
            let packet = peer.read_packet().await.unwrap();
            assert_eq!(packet.id, expected, "tick {tick}");
            assert_eq!(packet.data.last(), Some(&u8::from(grounded)));
        }
    }

    #[tokio::test]
    async fn relative_correction_preserves_launch_motion_on_relative_axes() {
        let (mut client, mut peer) = test_game_client().await;
        client.position = Vec3::new(8.0, 65.0, 8.0);
        client.yaw = yaw_from_mc(90.0);
        client.pitch = pitch_from_mc(15.0);
        let mut w = nephele_protocol::PacketWriter::new();
        w.write_f64(1.0)
            .write_f64(2.0)
            .write_f64(4.0)
            .write_f32(10.0)
            .write_f32(20.0)
            .write_u8(0x1b);
        peer.write_packet(&Packet::new(
            play::clientbound::PLAYER_POSITION_AND_LOOK,
            w.into_bytes(),
        ))
        .await
        .unwrap();
        let Event::Position {
            position,
            relative_flags,
            ..
        } = client.tick(Duration::from_secs(1)).await.unwrap()
        else {
            panic!("expected correction");
        };
        let mut player = physics::PlayerState::at(8.0, 65.0, 8.0);
        player.set_velocity(1.0, 20.0, 3.0);
        player.correct_position(position, relative_flags);
        assert_eq!(position, Vec3::new(9.0, 67.0, 4.0));
        assert_eq!((player.vx, player.vy, player.vz), (1.0, 20.0, 0.0));
        let ack = peer.read_packet().await.unwrap();
        assert_eq!(ack.id, play::serverbound::PLAYER_POSITION_AND_LOOK);
        let mut r = ack.reader();
        assert_eq!(
            [
                r.read_f64().unwrap(),
                r.read_f64().unwrap(),
                r.read_f64().unwrap()
            ],
            [9.0, 67.0, 4.0]
        );
        assert!((r.read_f32().unwrap() - 100.0).abs() < 1e-4);
        assert!((r.read_f32().unwrap() - 35.0).abs() < 1e-4);
        assert!(!r.read_bool().unwrap());
    }

    #[tokio::test]
    async fn server_velocity_launches_us_off_a_pressure_plate() {
        let (mut client, mut peer) = test_game_client().await;
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(8, 64, 8, nephele_world::BlockState::STONE);
        column.set_block(8, 65, 8, nephele_world::BlockState::new(70, 1));
        client.world.insert(column);
        let mut player = physics::PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let mut w = nephele_protocol::PacketWriter::new();
        w.write_varint(client.info.entity_id)
            .write_i16(4000)
            .write_i16(8000)
            .write_i16(0);
        peer.write_packet(&Packet::new(
            play::clientbound::ENTITY_VELOCITY,
            w.into_bytes(),
        ))
        .await
        .unwrap();
        let Event::Velocity { vx, vy, vz } = client.tick(Duration::from_secs(1)).await.unwrap()
        else {
            panic!("expected a local velocity update");
        };
        player.set_velocity(vx, vy, vz);
        player.step(0.05, &physics::Input::default(), 0.0, &client.world);
        assert_eq!((player.x, player.y), (9.0, 66.0));
        assert!(!player.on_ground);
        player.step(0.05, &physics::Input::default(), 0.0, &client.world);
        assert!((player.y - 66.9016).abs() < 1e-6);
    }

    #[tokio::test]
    async fn another_entitys_velocity_stays_in_the_entity_table() {
        let (mut client, _peer) = test_game_client().await;
        let mut payload = Vec::new();
        payload.push(7);
        payload.extend_from_slice(&8000i16.to_be_bytes());
        payload.extend_from_slice(&0i16.to_be_bytes());
        payload.extend_from_slice(&0i16.to_be_bytes());

        let event = client
            .handle(Packet::new(play::clientbound::ENTITY_VELOCITY, payload))
            .await
            .expect("velocity");
        assert_eq!(event, Event::EntityMoved);
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

        // `NEPHELE_TEST_SECONDS` keeps the session open for a longer soak.
        let stay_seconds = std::env::var("NEPHELE_TEST_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        let seconds = stay_seconds.unwrap_or(30);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        let mut got_position = false;
        while tokio::time::Instant::now() < deadline {
            let event = match client.tick(Duration::from_secs(2)).await {
                Ok(event) => event,
                Err(err) => panic!("reading play packet: {err}"),
            };
            if let Event::Position { .. } = event {
                got_position = true;
            }
            if stay_seconds.is_none() && got_position && client.world.loaded_chunks() > 0 {
                break;
            }
        }

        assert!(got_position, "server did not send a spawn position");
        assert!(client.world.loaded_chunks() > 0, "server sent no chunks");
        // The framing invariant, end to end against a real server: every
        // packet handled during the soak consumed its whole declared payload.
        assert_eq!(
            client.payload_mismatches, 0,
            "a parser under-read its payload during the soak"
        );
        eprintln!(
            "soak finished: {} packets, {} chunks loaded, {} mismatches",
            client.packets_received,
            client.world.loaded_chunks(),
            client.payload_mismatches
        );
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

    /// Drive a real walking, jumping player against a live server.
    ///
    /// This is the acceptance run for movement: twenty seconds of forward
    /// motion with periodic jumps, sending one movement packet per tick, while
    /// watching for `S08` corrections. A server only sends one when it disagrees
    /// with where the client says it is, so a large one is exactly the
    /// rubber-band a player would feel.
    #[tokio::test]
    #[ignore = "requires a cached Microsoft session and a live Hypixel lobby"]
    async fn walks_and_jumps_on_a_live_server_without_rubber_banding() {
        use crate::physics::{Input, PlayerState};

        let app_data = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        let session_path = std::path::PathBuf::from(app_data)
            .join("Nephele")
            .join("auth.json");
        let stored = nephele_auth::StoredSession::load(&session_path).expect("cached session");
        let address = ServerAddress::parse("mc.hypixel.net").unwrap();
        let mut client = GameClient::join_online(&address, &stored.session())
            .await
            .expect("online join");

        // Wait for a spawn and the terrain under it.
        let settle = tokio::time::Instant::now() + Duration::from_secs(40);
        while tokio::time::Instant::now() < settle
            && (!client.spawned || client.world.loaded_chunks() == 0)
        {
            let _ = client.tick(Duration::from_millis(200)).await.expect("join");
        }
        assert!(client.spawned, "never spawned");
        assert!(client.world.loaded_chunks() > 0, "no terrain");

        // Face +X by default (`forward` is `(sin yaw, -cos yaw)`); the
        // direction is overridable so a capture run can explore the lobby.
        let yaw = std::env::var("NEPHELE_WALK_YAW_DEGREES")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(90.0)
            .to_radians();
        let walk_seconds = std::env::var("NEPHELE_WALK_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(20);
        let mut player = PlayerState::at(client.position.x, client.position.y, client.position.z);
        player.on_ground = true;

        let mut corrections = 0usize;
        let mut worst_correction = 0.0f64;
        let mut jumps = 0usize;
        let mut ran = 0u32;
        let start = (player.x, player.z);
        // A rubber-band pins you to the same spot: the server repeats the same
        // correction position instead of letting you travel. Counting those is
        // meaningful even though a single correction's distance also includes
        // the round-trip latency (the client keeps walking while it arrives).
        let mut last_correction: Option<(f64, f64, u32)> = None;
        let mut pinned = 0usize;

        let end = tokio::time::Instant::now() + Duration::from_secs(walk_seconds);
        let mut next_tick = tokio::time::Instant::now();
        while tokio::time::Instant::now() < end {
            next_tick += Duration::from_millis(50);
            let jump = player.on_ground && ran % 20 == 0;
            let input = Input {
                forward: 1.0,
                jump,
                ..Input::default()
            };
            player.step(0.05, &input, yaw, &client.world);
            if jump {
                jumps += 1;
            }
            ran += 1;

            client
                .send_movement_tick(
                    Vec3::new(player.x, player.y, player.z),
                    yaw,
                    0.0,
                    player.on_ground,
                )
                .await
                .expect("movement packet");

            // Drain whatever arrived since the last tick, without blocking.
            loop {
                match client.tick(Duration::from_millis(1)).await {
                    Ok(Event::Idle) => break,
                    Ok(Event::Position { position, .. }) => {
                        let dx = position.x - player.x;
                        let dy = position.y - player.y;
                        let dz = position.z - player.z;
                        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
                        if distance > 2.0 {
                            corrections += 1;
                            worst_correction = worst_correction.max(distance);
                            eprintln!(
                                "correction {corrections} at tick {ran}: {distance:.1} blocks, client ({:.1}, {:.1}, {:.1}) -> server ({:.1}, {:.1}, {:.1})",
                                player.x, player.y, player.z, position.x, position.y, position.z
                            );
                            if let Some((lx, lz, ltick)) = last_correction {
                                let pinned_here = (position.x - lx).hypot(position.z - lz) < 0.25;
                                if pinned_here && ran - ltick <= 10 {
                                    pinned += 1;
                                }
                            }
                            last_correction = Some((position.x, position.z, ran));
                        }
                        // The live client teleports its body to the server's
                        // position on the spot; a test that ignored the
                        // correction would drift away and then count its own
                        // drift as the server's fault.
                        player.x = position.x;
                        player.y = position.y;
                        player.z = position.z;
                        player.vx = 0.0;
                        player.vy = 0.0;
                        player.vz = 0.0;
                    }
                    Ok(Event::Velocity { vx, vy, vz }) => {
                        // A launch pad or knockback: adopt it immediately.
                        player.set_velocity(vx, vy, vz);
                    }
                    Ok(_) => {}
                    Err(err) => panic!("disconnected while walking: {err}"),
                }
            }

            if let Some(delay) = next_tick.checked_duration_since(tokio::time::Instant::now()) {
                tokio::time::sleep(delay).await;
            }
        }

        eprintln!(
            "walk finished: {} ticks, {} jumps, {} corrections (worst {:.1} blocks), {} pinned, ended at ({:.1}, {:.1}, {:.1})",
            ran, jumps, corrections, worst_correction, pinned, player.x, player.y, player.z
        );
        let advanced = (player.x - start.0).hypot(player.z - start.1);
        // Being rubber-banded means being held in place: you cannot walk 40
        // blocks while the server keeps pulling you back to spawn.
        let expected = walk_seconds as f64 * 0.5;
        assert!(
            advanced > expected,
            "only advanced {advanced:.1} blocks in {walk_seconds} s; the server is holding us back"
        );
        assert_eq!(pinned, 0, "the server kept correcting us to the same spot");
        assert_eq!(client.payload_mismatches, 0);
        client.disconnect().await;
    }
}
