use nephele_protocol::{Error, Packet, Result};

use crate::chunk::{
    ChunkColumn, Section, SECTION_BLOCK_BYTES, SECTION_COUNT, SECTION_LIGHT_BYTES, SECTION_VOLUME,
};

pub mod ids {
    /// Clientbound `Chunk Data`.
    pub const CHUNK_DATA: i32 = 0x21;
    /// Clientbound `Map Chunk Bulk`.
    pub const CHUNK_BULK: i32 = 0x26;
}

/// Result of decoding a `Chunk Data` packet.
#[derive(Debug, Clone)]
pub enum ChunkUpdate {
    Load(ChunkColumn),
    /// Ground-up packet with an empty bitmask: drop this chunk.
    Unload {
        x: i32,
        z: i32,
    },
}

/// Decode a `Chunk Data` packet.
pub fn decode_chunk_data(packet: &Packet, has_skylight: bool) -> Result<ChunkUpdate> {
    if packet.id != ids::CHUNK_DATA {
        return Err(Error::UnexpectedPacket {
            id: packet.id,
            state: "chunk data",
        });
    }
    let mut reader = packet.reader();
    let x = reader.read_i32()?;
    let z = reader.read_i32()?;
    let ground_up = reader.read_bool()?;
    let mask = reader.read_u16()?;
    let data = reader.read_byte_array()?;

    if ground_up && mask == 0 {
        return Ok(ChunkUpdate::Unload { x, z });
    }
    decode_column(x, z, mask, &data, has_skylight, ground_up).map(ChunkUpdate::Load)
}

/// Decode a `Map Chunk Bulk` packet into its columns.
pub fn decode_chunk_bulk(packet: &Packet) -> Result<Vec<ChunkColumn>> {
    if packet.id != ids::CHUNK_BULK {
        return Err(Error::UnexpectedPacket {
            id: packet.id,
            state: "chunk bulk",
        });
    }
    let mut reader = packet.reader();
    let has_skylight = reader.read_bool()?;
    let count = reader.read_varint()?;
    if count < 0 {
        return Err(Error::Protocol(format!(
            "negative chunk bulk count {count}"
        )));
    }
    let count = count as usize;
    if count > 4096 {
        return Err(Error::Protocol(format!(
            "implausible chunk bulk count {count}"
        )));
    }

    let mut columns = Vec::with_capacity(count);
    for _ in 0..count {
        let x = reader.read_i32()?;
        let z = reader.read_i32()?;
        let mask = reader.read_u16()?;
        columns.push((x, z, mask));
    }

    // Vanilla derives each column's byte length from its bitmask; there is no
    // per-column length prefix in 1.8.9.
    let mut decoded = Vec::with_capacity(count);
    for (x, z, mask) in columns {
        let size = expected_len(mask, has_skylight, true);
        let remaining = reader.remaining();
        if remaining < size {
            return Err(Error::Protocol(format!(
                "chunk bulk column ({x}, {z}) needs {size} bytes, only {remaining} left"
            )));
        }
        let slice = reader.take(size)?;
        decoded.push(decode_column(x, z, mask, slice, has_skylight, true)?);
    }

    Ok(decoded)
}

fn section_count(mask: u16) -> usize {
    mask.count_ones() as usize
}

/// Exact payload size for a column, matching vanilla's `func_180737_a`.
pub fn expected_len(mask: u16, has_skylight: bool, ground_up: bool) -> usize {
    let sections = section_count(mask);
    sections * SECTION_BLOCK_BYTES
        + sections * SECTION_LIGHT_BYTES
        + if has_skylight {
            sections * SECTION_LIGHT_BYTES
        } else {
            0
        }
        + if ground_up { 256 } else { 0 }
}

fn decode_column(
    x: i32,
    z: i32,
    mask: u16,
    data: &[u8],
    has_skylight: bool,
    ground_up: bool,
) -> Result<ChunkColumn> {
    let expected = expected_len(mask, has_skylight, ground_up);
    if data.len() != expected {
        return Err(Error::Protocol(format!(
            "chunk ({x}, {z}) payload is {} bytes, expected {expected} \
             ({} sections, skylight={has_skylight}, ground_up={ground_up})",
            data.len(),
            section_count(mask),
        )));
    }

    let mut sections: Vec<Option<Section>> = vec![None; SECTION_COUNT];
    let mut cursor = 0usize;

    for (bit, slot) in sections.iter_mut().enumerate() {
        if mask & (1u16 << bit) == 0 {
            continue;
        }
        let slice = &data[cursor..cursor + SECTION_BLOCK_BYTES];
        cursor += SECTION_BLOCK_BYTES;

        let mut blocks = Vec::with_capacity(SECTION_VOLUME);
        let mut non_air = 0u32;
        for pair in slice.chunks_exact(2) {
            // 1.8.9 writes each block state little-endian.
            let raw = u16::from_le_bytes([pair[0], pair[1]]);
            if raw != 0 {
                non_air += 1;
            }
            blocks.push(raw);
        }
        *slot = Some(Section::from_blocks(blocks, non_air));
    }

    for (bit, slot) in sections.iter_mut().enumerate() {
        if mask & (1u16 << bit) == 0 {
            continue;
        }
        let slice = &data[cursor..cursor + SECTION_LIGHT_BYTES];
        cursor += SECTION_LIGHT_BYTES;
        if let Some(section) = slot.as_mut() {
            section.set_block_light(slice);
        }
    }

    if has_skylight {
        for (bit, slot) in sections.iter_mut().enumerate() {
            if mask & (1u16 << bit) == 0 {
                continue;
            }
            let slice = &data[cursor..cursor + SECTION_LIGHT_BYTES];
            cursor += SECTION_LIGHT_BYTES;
            if let Some(section) = slot.as_mut() {
                section.set_sky_light(slice);
            }
        }
    }

    let mut column = ChunkColumn::from_sections(x, z, sections);
    if ground_up {
        column.set_biomes(&data[cursor..cursor + 256]);
    }
    Ok(column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockState;
    use crate::encode::{encode_chunk_bulk, encode_chunk_data};
    use crate::world::World;

    fn sample_column(x: i32, z: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(x, z);
        for bx in 0..16 {
            for bz in 0..16 {
                for y in 0..=64 {
                    let block = match y {
                        0 => BlockState::BEDROCK,
                        64 => BlockState::GRASS,
                        _ => BlockState::STONE,
                    };
                    column.set_block(bx, y, bz, block);
                }
                column.set_block(bx, 70, bz, BlockState::LOG);
            }
        }
        if let Some(section) = column.section_mut(4) {
            section.set_sky_light(&vec![0xFF; SECTION_LIGHT_BYTES]);
        }
        column.set_biomes(&vec![1u8; 256]);
        column
    }

    #[test]
    fn expected_len_matches_vanilla_formula() {
        // 1 section, skylight, ground-up: 8192 + 2048 + 2048 + 256
        assert_eq!(expected_len(0b1, true, true), 12_544);
        // 2 sections, no skylight, not ground-up: 2*8192 + 2*2048
        assert_eq!(expected_len(0b11, false, false), 20_480);
        assert_eq!(expected_len(0, false, false), 0);
    }

    #[test]
    fn chunk_data_roundtrip() {
        let original = sample_column(3, -7);
        let packet = encode_chunk_data(&original, true, true);
        let ChunkUpdate::Load(decoded) = decode_chunk_data(&packet, true).unwrap() else {
            panic!("expected a loaded column");
        };

        assert_eq!(decoded.x, 3);
        assert_eq!(decoded.z, -7);
        assert_eq!(decoded.mask(), original.mask());
        assert_eq!(decoded.non_air_count(), original.non_air_count());
        assert_eq!(decoded.get_block(0, 64, 0), BlockState::GRASS);
        assert_eq!(decoded.get_block(0, 63, 0), BlockState::STONE);
        assert_eq!(decoded.get_block(0, 0, 0), BlockState::BEDROCK);
        assert_eq!(decoded.get_block(0, 70, 0), BlockState::LOG);
        assert_eq!(decoded.biomes().unwrap()[0], 1);
    }

    #[test]
    fn chunk_bulk_roundtrip() {
        let columns: Vec<ChunkColumn> = (0..4).map(|i| sample_column(i, i * 2)).collect();
        let packet = encode_chunk_bulk(&columns, true);
        let decoded = decode_chunk_bulk(&packet).unwrap();

        assert_eq!(decoded.len(), 4);
        for (index, column) in decoded.iter().enumerate() {
            assert_eq!(column.x, index as i32);
            assert_eq!(column.z, index as i32 * 2);
            assert_eq!(column.get_block(3, 64, 3), BlockState::GRASS);
        }
    }

    #[test]
    fn rejects_truncated_payload() {
        let column = sample_column(0, 0);
        let mut packet = encode_chunk_data(&column, true, true);
        // Shrink the declared byte array by rewriting the last byte.
        let last = packet.data.len() - 1;
        packet.data[last] ^= 0xFF;
        // Corrupting a byte keeps the length valid, so instead truncate the body.
        let payload_len_pos = 13; // 4+4+1+2 then varint length
        packet.data[payload_len_pos] -= 1;
        assert!(decode_chunk_data(&packet, true).is_err());
    }

    #[test]
    fn ground_up_with_empty_mask_unloads() {
        let packet = Packet::new(ids::CHUNK_DATA, {
            let mut w = nephele_protocol::PacketWriter::new();
            w.write_i32(1)
                .write_i32(2)
                .write_bool(true)
                .write_u16(0)
                .write_byte_array(&[]);
            w.into_bytes()
        });
        let ChunkUpdate::Unload { x, z } = decode_chunk_data(&packet, true).unwrap() else {
            panic!("expected an unload");
        };
        assert_eq!((x, z), (1, 2));
    }

    #[test]
    fn decoded_columns_land_in_a_world() {
        let mut world = World::new(0);
        let columns: Vec<ChunkColumn> = (0..2).map(|i| sample_column(i, 0)).collect();
        let packet = encode_chunk_bulk(&columns, true);
        for column in decode_chunk_bulk(&packet).unwrap() {
            world.insert(column);
        }
        assert_eq!(world.loaded_chunks(), 2);
        // world x=5, z=5 is chunk 0,0
        assert_eq!(world.get_block(5, 64, 5), BlockState::GRASS);
        // world x=20 is chunk 1
        assert_eq!(world.get_block(20, 64, 5), BlockState::GRASS);
    }
}
