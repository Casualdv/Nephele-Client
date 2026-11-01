//! `nephele-world` Ã¢â‚¬â€ block storage and 1.8.9 chunk (de)serialisation.
//!
//! A chunk column is 16 sections of 16Ãƒâ€”16Ãƒâ€”16 blocks. In 1.8.9 a block is a
//! single `u16` where the high 12 bits are the block id and the low 4 bits are
//! the metadata nibble.
//!
//! The wire layout (verified against vanilla `S21PacketChunkData`):
//! ```text
//! for each present section, ascending: 4096 Ãƒâ€” u16 (little-endian) block states
//! for each present section, ascending: 2048 bytes block light nibbles
//! if the dimension has skylight, for each section: 2048 bytes sky light
//! if ground-up: 256 biome bytes
//! ```
//! `Map Chunk Bulk` (0x26) reuses that payload per column, with the column
//! sizes derived from each column's bitmask.

pub mod biome;
pub mod block;
pub mod chunk;
pub mod decode;
pub mod encode;
pub mod generation;
pub mod genlayer;
pub mod noise;
pub mod raycast;
pub mod terrain;
pub mod world;

pub use block::BlockState;
pub use chunk::{section_index, ChunkColumn, Section, SECTION_COUNT, SECTION_DIM, SECTION_VOLUME};
pub use decode::{decode_chunk_bulk, decode_chunk_data, ChunkUpdate};
pub use encode::{encode_chunk_bulk, encode_chunk_data};
pub use generation::{TerrainGenerator, SEA_LEVEL, SNOW_LINE};
pub use noise::{JavaRandom, NoiseImproved, NoiseOctaves};
pub use raycast::{raycast, raycast_to, Face, RayHit};
pub use world::{ChunkCoord, World};
