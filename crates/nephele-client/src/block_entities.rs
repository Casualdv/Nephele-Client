//! Block entities decoded from `S35 Update Tile Entity`.
//!
//! The wire keeps the signed NBT root; this module turns the supported 1.8
//! tile entities into the small amount of data the renderer needs.

use nephele_protocol::nbt::{self, Tag};
use nephele_protocol::packets::play::BlockEntityUpdate;
use nephele_world::BlockState;

/// One banner layer from a banner's `Patterns` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerPattern {
    pub pattern: String,
    pub color: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockEntityKind {
    Sign {
        lines: [String; 4],
        color: String,
    },
    Chest,
    Banner {
        base: u8,
        patterns: Vec<BannerPattern>,
    },
    Beacon {
        levels: i32,
        primary: i32,
        secondary: i32,
    },
    EnchantingTable,
    Unknown {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntity {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub kind: BlockEntityKind,
}

impl BlockEntity {
    /// Decode a tile entity, falling back to an opaque `Unknown` record when
    /// the NBT is malformed or the id is not one this client draws.
    pub fn from_update(update: &BlockEntityUpdate) -> Self {
        let root = match nbt::parse_named_tag(&update.nbt) {
            Ok((_, tag)) => tag,
            Err(_) => Tag::End,
        };
        let raw_id = root
            .get("id")
            .and_then(Tag::as_str)
            .unwrap_or_default()
            .to_string();
        let id = raw_id.rsplit(':').next().unwrap_or(&raw_id).to_string();
        let kind = match id.as_str() {
            "Sign" => BlockEntityKind::Sign {
                lines: [
                    text(&root, "Text1"),
                    text(&root, "Text2"),
                    text(&root, "Text3"),
                    text(&root, "Text4"),
                ],
                color: text(&root, "Color"),
            },
            "Chest" => BlockEntityKind::Chest,
            "Banner" => BlockEntityKind::Banner {
                base: root.get("Base").and_then(Tag::as_int).unwrap_or(0) as u8,
                patterns: root
                    .get("Patterns")
                    .and_then(Tag::as_list)
                    .map(|patterns| {
                        patterns
                            .iter()
                            .filter_map(|pattern| {
                                Some(BannerPattern {
                                    pattern: pattern
                                        .get("Pattern")
                                        .and_then(Tag::as_str)?
                                        .to_string(),
                                    color: pattern.get("Color").and_then(Tag::as_int).unwrap_or(0)
                                        as u8,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "Beacon" => BlockEntityKind::Beacon {
                levels: root.get("Levels").and_then(Tag::as_int).unwrap_or(0),
                primary: root.get("Primary").and_then(Tag::as_int).unwrap_or(0),
                secondary: root.get("Secondary").and_then(Tag::as_int).unwrap_or(0),
            },
            "EnchantingTable" => BlockEntityKind::EnchantingTable,
            other => BlockEntityKind::Unknown {
                id: other.to_string(),
            },
        };
        Self {
            x: update.x,
            y: update.y,
            z: update.z,
            kind,
        }
    }

    /// Infer a block entity from its block state. This is what makes locally
    /// placed signs, chests and banners render without a server NBT packet.
    pub fn from_block(x: i32, y: i32, z: i32, state: BlockState) -> Option<Self> {
        let kind = match state.id() {
            63 | 68 => BlockEntityKind::Sign {
                lines: Default::default(),
                color: String::new(),
            },
            54 | 146 => BlockEntityKind::Chest,
            176 | 177 => BlockEntityKind::Banner {
                base: 0,
                patterns: Vec::new(),
            },
            138 => BlockEntityKind::Beacon {
                // A locally placed beacon has no server NBT; show its beam.
                levels: 1,
                primary: 0,
                secondary: 0,
            },
            116 => BlockEntityKind::EnchantingTable,
            _ => return None,
        };
        Some(Self { x, y, z, kind })
    }
}

fn text(root: &Tag, key: &str) -> String {
    root.get(key)
        .and_then(Tag::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_protocol::PacketWriter;

    fn sign_nbt() -> Vec<u8> {
        let mut w = PacketWriter::new();
        // Root compound named "sign".
        w.write_u8(10).write_u16(4).write_bytes(b"sign");
        // id: TAG_String "Sign"
        w.write_u8(8).write_u16(2).write_bytes(b"id");
        w.write_u16(4).write_bytes(b"Sign");
        // Text1: TAG_String "hello"
        w.write_u8(8).write_u16(5).write_bytes(b"Text1");
        w.write_u16(5).write_bytes(b"hello");
        w.write_u8(0);
        w.into_bytes()
    }

    #[test]
    fn signs_decode_their_lines() {
        let update = BlockEntityUpdate {
            x: 1,
            y: 64,
            z: 2,
            action: 9,
            nbt: sign_nbt(),
        };
        let entity = BlockEntity::from_update(&update);
        assert_eq!(entity.x, 1);
        assert_eq!(
            entity.kind,
            BlockEntityKind::Sign {
                lines: ["hello".into(), String::new(), String::new(), String::new()],
                color: String::new(),
            }
        );
    }

    #[test]
    fn block_ids_infer_local_block_entities() {
        assert!(matches!(
            BlockEntity::from_block(0, 0, 0, BlockState::new(54, 0))
                .unwrap()
                .kind,
            BlockEntityKind::Chest
        ));
        assert!(BlockEntity::from_block(0, 0, 0, BlockState::STONE).is_none());
    }
}
