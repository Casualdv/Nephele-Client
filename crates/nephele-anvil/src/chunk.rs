//! Turning a stored chunk's NBT into blocks and biomes.
//!
//! In 1.8.9 a chunk column is stored as `Level.Sections[]`, each with a 4096-byte
//! `Blocks` array (YZX order, one signed byte per block id), an optional `Add`
//! nibble array for ids above 255, and a `Data` nibble array of metadata.
//! `Level.Biomes` holds one biome id per column, 16x16, X fastest.

use crate::nbt::{self, NbtError, Tag};
use indexmap::IndexMap;

pub const SECTION_VOLUME: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredSection {
    pub y: i8,
    pub blocks: [u8; SECTION_VOLUME],
    pub add: Option<[u8; SECTION_VOLUME / 2]>,
    pub data: Option<[u8; SECTION_VOLUME / 2]>,
    pub block_light: Option<[u8; SECTION_VOLUME / 2]>,
    pub sky_light: Option<[u8; SECTION_VOLUME / 2]>,
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
                let block_light = read_nibbles(entry.get("BlockLight"));
                let sky_light = read_nibbles(entry.get("SkyLight"));

                sections.push(StoredSection {
                    y: y as i8,
                    blocks: packed,
                    add,
                    data,
                    block_light,
                    sky_light,
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

/// Encodes one stored chunk as the named NBT root used by a 1.8 Anvil region.
pub fn encode_chunk_nbt(chunk: &StoredChunk) -> Result<Vec<u8>, ChunkError> {
    if chunk.biomes.len() != 256 {
        return Err(ChunkError::BadBiomeLength {
            length: chunk.biomes.len(),
        });
    }

    let mut sections = chunk.sections.clone();
    sections.sort_by_key(|section| section.y);
    for (index, section) in sections.iter().enumerate() {
        if !(0..16).contains(&section.y) {
            return Err(ChunkError::BadSectionY(section.y));
        }
        if index > 0 && sections[index - 1].y == section.y {
            return Err(ChunkError::DuplicateSectionY(section.y));
        }
    }

    let section_tags = sections
        .iter()
        .map(|section| {
            let mut fields = IndexMap::new();
            fields.insert("Y".to_string(), Tag::Byte(section.y));
            fields.insert(
                "Blocks".to_string(),
                Tag::ByteArray(section.blocks.to_vec()),
            );
            if let Some(add) = section.add {
                fields.insert("Add".to_string(), Tag::ByteArray(add.to_vec()));
            }
            fields.insert(
                "Data".to_string(),
                Tag::ByteArray(section.data.unwrap_or([0; SECTION_VOLUME / 2]).to_vec()),
            );
            fields.insert(
                "BlockLight".to_string(),
                Tag::ByteArray(
                    section
                        .block_light
                        .unwrap_or([0; SECTION_VOLUME / 2])
                        .to_vec(),
                ),
            );
            fields.insert(
                "SkyLight".to_string(),
                Tag::ByteArray(
                    section
                        .sky_light
                        .unwrap_or([0; SECTION_VOLUME / 2])
                        .to_vec(),
                ),
            );
            Tag::Compound(fields)
        })
        .collect();

    let height_map = (0..16)
        .flat_map(|z| {
            (0..16).map(move |x| {
                chunk
                    .height(x, z)
                    .map(|height| height as i32 + 1)
                    .unwrap_or(0)
            })
        })
        .collect();

    let mut level = IndexMap::new();
    level.insert("xPos".to_string(), Tag::Int(chunk.x));
    level.insert("zPos".to_string(), Tag::Int(chunk.z));
    level.insert("LastUpdate".to_string(), Tag::Long(0));
    level.insert("InhabitedTime".to_string(), Tag::Long(0));
    level.insert("TerrainPopulated".to_string(), Tag::Byte(1));
    level.insert("LightPopulated".to_string(), Tag::Byte(1));
    level.insert("Sections".to_string(), Tag::List(section_tags));
    level.insert("Biomes".to_string(), Tag::ByteArray(chunk.biomes.clone()));
    level.insert("HeightMap".to_string(), Tag::IntArray(height_map));
    level.insert("Entities".to_string(), Tag::List(Vec::new()));
    level.insert("TileEntities".to_string(), Tag::List(Vec::new()));
    level.insert("TileTicks".to_string(), Tag::List(Vec::new()));

    let mut root = IndexMap::new();
    root.insert("Level".to_string(), Tag::Compound(level));
    Ok(nbt::encode("", &Tag::Compound(root))?)
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
    #[error("chunk has {length} biome bytes, expected 256")]
    BadBiomeLength { length: usize },
    #[error("cannot encode section with Y={0}; expected 0..16")]
    BadSectionY(i8),
    #[error("chunk contains duplicate section Y={0}")]
    DuplicateSectionY(i8),
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
            block_light: None,
            sky_light: None,
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
            block_light: None,
            sky_light: None,
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

    #[test]
    fn encoded_chunk_nbt_roundtrips_states_and_biomes() {
        let mut section = StoredSection {
            y: 4,
            blocks: [0; SECTION_VOLUME],
            add: Some([0; SECTION_VOLUME / 2]),
            data: Some([0; SECTION_VOLUME / 2]),
            block_light: Some([0xAB; SECTION_VOLUME / 2]),
            sky_light: Some([0xFF; SECTION_VOLUME / 2]),
        };
        let index = (2 * 16 + 3) * 16 + 1;
        section.blocks[index] = 3;
        section.add.as_mut().unwrap()[index / 2] = 0x10;
        section.data.as_mut().unwrap()[index / 2] = 0x50;
        let original = StoredChunk {
            x: -7,
            z: 12,
            sections: vec![section],
            biomes: vec![6; 256],
        };

        let bytes = encode_chunk_nbt(&original).unwrap();
        let decoded = parse_chunk(&bytes).unwrap();

        assert_eq!((decoded.x, decoded.z), (-7, 12));
        assert_eq!(decoded.state(1, 4 * 16 + 2, 3), (0x103 << 4) | 5);
        assert_eq!(decoded.biome(15, 15), Some(6));
        assert_eq!(
            decoded.sections[0].block_light,
            original.sections[0].block_light
        );
        assert_eq!(
            decoded.sections[0].sky_light,
            original.sections[0].sky_light
        );
    }
}
