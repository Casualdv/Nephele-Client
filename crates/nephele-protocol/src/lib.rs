//! `nephele-protocol` — the wire protocol for Minecraft Java Edition 1.8.9
//! (protocol 47), reimplemented from scratch.
//!
//! Layers, bottom to top:
//! * [`read`] / [`write`] — Minecraft primitives: VarInt, VarLong, String, ...
//! * [`framing`] — length prefix plus optional zlib compression
//! * [`crypto`] — AES-128/CFB8 stream cipher and the server-id hash
//! * [`packet`] — `id + payload` representation
//! * [`packets`] — handshake, status, login and play packet definitions
//! * [`connection`] — async TCP connection and its state machine
//!
//! ```no_run
//! # async fn demo() -> nephele_protocol::Result<()> {
//! use nephele_protocol::{ping, ServerAddress};
//! use std::time::Duration;
//!
//! let address = ServerAddress::parse("mc.hypixel.net")?;
//! let status = ping(&address, Duration::from_secs(5)).await?;
//! println!("{} players online", status.online_players.unwrap_or(0));
//! # Ok(())
//! # }
//! ```

pub mod connection;
pub mod crypto;
pub mod error;
pub mod framing;
pub mod nbt;
pub mod packet;
pub mod packets;
pub mod read;
pub mod state;
pub mod write;

pub use connection::{ping, version_string, Connection, LoginOutcome, ServerAddress, DEFAULT_PORT};
pub use error::{Error, Result};
pub use packet::Packet;
pub use packets::login::{EncryptionRequest, LoginSuccess};
pub use packets::play::JoinGame;
pub use packets::status::StatusResponse;
pub use read::{encode_varint, varint_len, PacketReader};
pub use state::{NextState, State};
pub use write::PacketWriter;

/// Protocol number used by Minecraft 1.8.9.
pub const PROTOCOL_VERSION: i32 = 47;
/// Human-readable version this crate targets.
pub const MINECRAFT_VERSION: &str = "1.8.9";
