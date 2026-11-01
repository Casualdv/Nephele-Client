//! Turning a stored chunk's NBT into blocks and biomes.
//!
//! In 1.8.9 a chunk column is stored as `Level.Sections[]`, each with a 4096-byte
//! `Blocks` array (YZX order, one signed byte per block id), an optional `Add`
//! nibble array for ids above 255, and a `Data` nibble array of metadata.
//! `Level.Biomes` holds one biome id per column, 16x16, X fastest.

use crate::nbt::{self, NbtError, Tag};

pub const SECTION_VOLUME: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredSection {
    pub y: i8,
    pub blocks: [u8; SECTION_VOLUME],
    pub add: Option<[u8; SECTION_VOLUME / 2]>,
    pub data: Option<[u8; SECTION_VOLUME / 2]>,
}

impl StoredSection {
    pub fn block(&self, x: usize, y: usize, z: usize) -> u16 {
        let index = (y * 16 + z) * 16 + x;
        let mut id = self.blocks[index] as u16;
        if let Some(add) = &self.add {
            let nibble = nibble_at(add, index) as u16;
            id |= nibble << 8;
        }
        id
    }

    pub fn metadata(&self, x: usize, y: usize, z: usize) -> u8 {
        match &self.data {
            Some(data) => nibble_at(data, (y * 16 + z) * 16 + x),
            None => 0,
        }
    }

    /// The `(id << 4) | meta` packed state Nephele uses internally.
    pub fn state(&self, x: usize, y: usize, z: usize) -> u16 {
        (self.block(x, y, z) << 4) | self.metadata(x, y, z) as u16
    }
}

fn nibble_at(packed: &[u8; SECTION_VOLUME / 2], index: usize) -> u8 {
    let byte = packed[index / 2];
    if index % 2 == 0 {
        byte & 0x0F
    } else {
        byte >> 4
    }
}

#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<StoredSection>,
    pub biomes: Vec<u8>,
}

impl StoredChunk {
    /// Parses the `Level` payload of a saved chunk.
    pub fn parse(root: &Tag) -> Result<StoredChunk, ChunkError> {
        let level = root.get("Level").ok_or(ChunkError::MissingLevel)?;
        let x = level
            .get("xPos")
            .and_then(Tag::as_int)
            .ok_or(ChunkError::MissingPosition)?;
        let z = level
            .get("zPos")
            .and_then(Tag::as_int)
            .ok_or(ChunkError::MissingPosition)?;

        let mut sections = Vec::new();
        if let Some(entries) = level.get("Sections").and_then(Tag::as_list) {
            for entry in entries {
                // 1.8 writes section Y as a byte; later formats use an int.
                let y = match entry.get("Y") {
                    Some(Tag::Byte(value)) => *value as i32,
                    Some(Tag::Int(value)) => *value,
                    _ => continue,
                };
                let Some(blocks) = entry.get("Blocks").and_then(Tag::as_byte_array) else {
                    continue;
                };
                if blocks.len() != SECTION_VOLUME {
                    return Err(ChunkError::BadSectionLength {
                        y: y as i8,
                        length: blocks.len(),
                    });
                }
                let mut packed = [0u8; SECTION_VOLUME];
                packed.copy_from_slice(blocks);

                let add = read_nibbles(entry.get("Add"));
                let data = read_nibbles(entry.get("Data"));

                sections.push(StoredSection {
                    y: y as i8,
                    blocks: packed,
                    add,
                    data,
                });
            }
        }
        sections.sort_by_key(|section| section.y);

        let biomes = level
            .get("Biomes")
            .and_then(Tag::as_byte_array)
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();

        Ok(StoredChunk {
            x,
            z,
            sections,
            biomes,
        })
    }

    pub fn section(&self, y: i8) -> Option<&StoredSection> {
        self.sections.iter().find(|section| section.y == y)
    }

    /// Packed `(id << 4) | meta` at chunk-local coordinates.
    pub fn state(&self, x: usize, y: usize, z: usize) -> u16 {
        let section_y = (y / 16) as i8;
        match self.section(section_y) {
            Some(section) => section.state(x, y % 16, z),
            None => 0,
        }
    }

    /// Biome id at chunk-local coordinates.
    pub fn biome(&self, x: usize, z: usize) -> Option<u8> {
        self.biomes.get(z * 16 + x).copied()
    }

    /// Highest non-air block in a column, `None` when the column is empty.
    pub fn height(&self, x: usize, z: usize) -> Option<usize> {
        for section in self.sections.iter().rev() {
            for y in (0..16).rev() {
                if section.block(x, y, z) != 0 {
                    return Some(section.y as usize * 16 + y);
                }
            }
        }
        None
    }
}

fn read_nibbles(tag: Option<&Tag>) -> Option<[u8; SECTION_VOLUME / 2]> {
    let bytes = tag.and_then(Tag::as_byte_array)?;
    if bytes.len() != SECTION_VOLUME / 2 {
        return None;
    }
    let mut packed = [0u8; SECTION_VOLUME / 2];
    packed.copy_from_slice(bytes);
    Some(packed)
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("chunk has no Level compound")]
    MissingLevel,
    #[error("chunk has no xPos/zPos")]
    MissingPosition,
    #[error("section Y={y} has {length} blocks, expected {SECTION_VOLUME}")]
    BadSectionLength { y: i8, length: usize },
    #[error(transparent)]
    Nbt(#[from] NbtError),
}

/// Convenience: decompressed chunk NBT straight to a [`StoredChunk`].
pub fn parse_chunk(nbt_bytes: &[u8]) -> Result<StoredChunk, ChunkError> {
    let (_, tag) = nbt::parse(nbt_bytes)?;
    StoredChunk::parse(&tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::RegionFile;
    use std::path::PathBuf;

    fn reference_region() -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region/r.-1.0.mca");
        path.is_file().then_some(path)
    }

    #[test]
    fn parses_a_real_vanilla_chunk() {
        let Some(path) = reference_region() else {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        };
        let region = RegionFile::open(&path).expect("region opens");
        let present = region.present();
        assert!(!present.is_empty());

        let mut parsed = 0;
        for (x, z) in present.iter().take(8) {
            let Some(bytes) = region.chunk_nbt(*x, *z).expect("chunk reads") else {
                continue;
            };
            let chunk = parse_chunk(&bytes).expect("chunk parses");
            assert_eq!(
                chunk.biomes.len(),
                256,
                "1.8 chunks store one biome per column"
            );
            assert!(
                !chunk.sections.is_empty(),
                "chunk {},{} has no sections",
                chunk.x,
                chunk.z
            );
            for section in &chunk.sections {
                assert!(section.y >= 0 && section.y < 16);
            }
            assert!(
                chunk.height(0, 0).is_some(),
                "chunk {},{} column 0,0 is empty",
                chunk.x,
                chunk.z
            );
            parsed += 1;
        }
        assert!(parsed > 0, "expected at least one chunk to parse");
    }

    #[test]
    fn section_indexing_is_yzx() {
        let mut section = StoredSection {
            y: 0,
            blocks: [0; SECTION_VOLUME],
            add: None,
            data: None,
        };
        // Index = (y*16 + z)*16 + x
        section.blocks[(3 * 16 + 5) * 16 + 7] = 42;
        assert_eq!(section.block(7, 3, 5), 42);
        assert_eq!(section.block(5, 3, 7), 0);
    }

    #[test]
    fn add_nibbles_extend_the_block_id() {
        let mut section = StoredSection {
            y: 0,
            blocks: [0; SECTION_VOLUME],
            add: Some([0; SECTION_VOLUME / 2]),
            data: Some([0; SECTION_VOLUME / 2]),
        };
        // (x, y, z) = (1, 0, 0) is index ((y * 16) + z) * 16 + x = 1.
        let index = 1;
        section.blocks[index] = 3;
        // Odd index -> high nibble.
        section.add.as_mut().unwrap()[index / 2] = 0x10;
        section.data.as_mut().unwrap()[index / 2] = 0x50;
        assert_eq!(section.block(1, 0, 0), 0x103);
        assert_eq!(section.metadata(1, 0, 0), 5);
        assert_eq!(section.state(1, 0, 0), (0x103 << 4) | 5);
    }
}
