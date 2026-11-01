use nephele_protocol::{Packet, PacketWriter};

use crate::chunk::{ChunkColumn, SECTION_BLOCK_BYTES, SECTION_COUNT, SECTION_LIGHT_BYTES};
use crate::decode::ids;

/// Build the raw `Chunk Data` payload for a column (no packet framing).
pub fn encode_column_payload(
    column: &ChunkColumn,
    has_skylight: bool,
    ground_up: bool,
) -> (u16, Vec<u8>) {
    let mask = column.mask();
    let sections = mask.count_ones() as usize;
    let mut out =
        Vec::with_capacity(sections * (SECTION_BLOCK_BYTES + SECTION_LIGHT_BYTES * 2) + 256);

    for (_, section) in column.iter_sections() {
        for raw in section.blocks() {
            out.extend_from_slice(&raw.to_le_bytes());
        }
    }
    for (_, section) in column.iter_sections() {
        out.extend_from_slice(section.block_light());
    }
    if has_skylight {
        for (_, section) in column.iter_sections() {
            match section.sky_light() {
                Some(light) => out.extend_from_slice(light),
                None => {
                    let start = out.len();
                    out.resize(start + SECTION_LIGHT_BYTES, 0);
                }
            }
        }
    }
    if ground_up {
        match column.biomes() {
            Some(biomes) => out.extend_from_slice(biomes),
            None => {
                let start = out.len();
                out.resize(start + 256, 0);
            }
        }
    }

    (mask, out)
}

/// Encode one column as a `Chunk Data` (0x21) packet.
pub fn encode_chunk_data(column: &ChunkColumn, has_skylight: bool, ground_up: bool) -> Packet {
    let (mask, payload) = encode_column_payload(column, has_skylight, ground_up);
    let mut w = PacketWriter::with_capacity(payload.len() + 16);
    w.write_i32(column.x)
        .write_i32(column.z)
        .write_bool(ground_up)
        .write_u16(mask)
        .write_byte_array(&payload);
    Packet::new(ids::CHUNK_DATA, w.into_bytes())
}

/// Encode several columns as a `Map Chunk Bulk` (0x26) packet.
///
/// Bulk columns are always sent ground-up (biomes included) and the skylight
/// flag is shared by the whole packet, matching vanilla.
pub fn encode_chunk_bulk(columns: &[ChunkColumn], has_skylight: bool) -> Packet {
    let mut w = PacketWriter::with_capacity(64 + columns.len() * 12_000);
    w.write_bool(has_skylight)
        .write_varint(columns.len() as i32);
    for column in columns {
        w.write_i32(column.x)
            .write_i32(column.z)
            .write_u16(column.mask());
    }
    for column in columns {
        let (_, payload) = encode_column_payload(column, has_skylight, true);
        w.write_bytes(&payload);
    }
    Packet::new(ids::CHUNK_BULK, w.into_bytes())
}

/// Encode a "ground-up, empty mask" packet, which tells the client to unload
/// the chunk.
pub fn encode_chunk_unload(x: i32, z: i32) -> Packet {
    let mut w = PacketWriter::new();
    w.write_i32(x)
        .write_i32(z)
        .write_bool(true)
        .write_u16(0)
        .write_byte_array(&[]);
    Packet::new(ids::CHUNK_DATA, w.into_bytes())
}

const _: () = assert!(SECTION_COUNT == 16);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockState;

    #[test]
    fn payload_size_matches_the_formula() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 10, 0, BlockState::STONE);
        column.set_block(0, 200, 0, BlockState::STONE);
        let (mask, payload) = encode_column_payload(&column, true, true);
        assert_eq!(mask, (1 << 0) | (1 << 12));
        // 2 sections * (8192 + 2048 + 2048) + 256 biomes
        assert_eq!(payload.len(), 2 * 12_288 + 256);
    }

    #[test]
    fn missing_skylight_is_zero_filled() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 10, 0, BlockState::STONE);
        let (_, payload) = encode_column_payload(&column, true, false);
        // 8192 blocks + 2048 block light + 2048 zeroed sky light
        assert_eq!(payload.len(), 8192 + 2048 + 2048);
        assert!(payload[8192 + 2048..].iter().all(|b| *b == 0));
    }
}
