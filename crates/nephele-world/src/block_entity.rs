//! Block entities whose block *model* is not a cube.
//!
//! Vanilla draws these through `TileEntityRenderer` classes (`ModelChest`,
//! `ModelSign`, `ModelBanner`, `ModelBed`) rather than the block model JSONs,
//! which is why they used to fall through to the mesher's cube path.
//!
//! This module is pure data: which kind a block state is, and which of the
//! mesher's paths should draw it. Geometry lives in `nephele-render`.

use crate::shapes::{block_shape, is_door, is_fence, is_full_cube, is_pane};

/// How a chest is rendered. Resolved from the neighbours by the mesher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChestSize {
    Single,
    Double,
}

/// A sign's mounting. Text is not rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignKind {
    /// `standing_sign` (63): a post with a board on top.
    Post,
    /// `wall_sign` (68): a board fixed to a wall, no post.
    Wall,
}

/// The block entities this client draws. `EnchantingTable` and `Beacon` are
/// block entities in vanilla, but their *block* models are a 12/16 box and a
/// full cube, so they keep the shape/cube path and only appear here so callers
/// can recognise them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockEntityKind {
    Chest {
        size: ChestSize,
    },
    Sign {
        kind: SignKind,
    },
    /// A bed half; `head` picks the head texture and end face.
    Bed {
        head: bool,
    },
    Banner {
        wall: bool,
    },
    EnchantingTable,
    Beacon,
}

impl BlockEntityKind {
    /// The kind for a block id, before neighbour refinement. Chests start as
    /// single; the mesher turns a pair into [`ChestSize::Double`].
    pub fn of(id: u16, meta: u8) -> Option<Self> {
        Some(match id {
            54 | 130 | 146 => Self::Chest {
                size: ChestSize::Single,
            },
            63 => Self::Sign {
                kind: SignKind::Post,
            },
            68 => Self::Sign {
                kind: SignKind::Wall,
            },
            26 => Self::Bed {
                head: meta & 8 != 0,
            },
            176 => Self::Banner { wall: false },
            177 => Self::Banner { wall: true },
            116 => Self::EnchantingTable,
            138 => Self::Beacon,
            _ => return None,
        })
    }

    /// Whether the mesher has a bespoke model for this kind.
    pub fn has_model(&self) -> bool {
        !matches!(self, Self::EnchantingTable | Self::Beacon)
    }
}

/// Which mesher path draws a block state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind {
    /// A full cube: the direct unit-cube path.
    Cube,
    /// `block_shape` boxes (slabs, stairs, panes, doors, torches, ...).
    Shape,
    /// Two crossed, alpha-tested quads (grass, flowers, crops, ...).
    CrossPlant,
    /// Iron bars: two-pixel slabs per connected side.
    Bars,
    /// Animated, blended crossed layers.
    Fire,
    /// A block-entity model.
    BlockEntity(BlockEntityKind),
}

/// Cross-quad plants. They have no solid volume and must not be treated as a
/// full cube by neighbour culling or ambient occlusion.
pub fn is_cross_plant(id: u16) -> bool {
    matches!(
        id,
        6 | 31 | 32 | 37 | 38 | 39 | 40 | 59 | 83 | 104 | 105 | 115 | 141 | 142 | 175
    )
}

/// The render kind table: one place that decides how a state is drawn.
pub fn render_kind(id: u16, meta: u8) -> RenderKind {
    if let Some(kind) = BlockEntityKind::of(id, meta) {
        if kind.has_model() {
            return RenderKind::BlockEntity(kind);
        }
    }
    match id {
        101 => return RenderKind::Bars,
        51 => return RenderKind::Fire,
        _ if is_cross_plant(id) => return RenderKind::CrossPlant,
        _ => {}
    }
    if is_pane(id) || is_fence(id) || is_door(id) || !is_full_cube(block_shape(id, meta)) {
        RenderKind::Shape
    } else {
        RenderKind::Cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_requested_block_entity_is_in_the_table() {
        for (id, meta) in [
            (54u16, 3u8),
            (146, 3),
            (130, 3),
            (63, 0),
            (68, 2),
            (26, 0),
            (26, 8),
            (176, 0),
            (177, 2),
            (116, 0),
            (138, 0),
        ] {
            assert!(
                BlockEntityKind::of(id, meta).is_some(),
                "id {id}:{meta} is not a block entity"
            );
        }
    }

    #[test]
    fn modelled_block_entities_leave_the_cube_path() {
        for (id, meta) in [
            (54u16, 2u8),
            (146, 5),
            (63, 0),
            (68, 3),
            (26, 1),
            (176, 0),
            (177, 4),
        ] {
            assert!(
                matches!(render_kind(id, meta), RenderKind::BlockEntity(_)),
                "id {id}:{meta}"
            );
        }
    }

    #[test]
    fn table_and_beacon_keep_their_box_and_cube() {
        // The enchanting table's vanilla block model is a 12/16 box, already in
        // `block_shape`; the beacon is a full cube.
        assert_eq!(render_kind(116, 0), RenderKind::Shape);
        assert_eq!(render_kind(138, 0), RenderKind::Cube);
    }

    #[test]
    fn bed_metadata_picks_the_half() {
        assert_eq!(
            BlockEntityKind::of(26, 0),
            Some(BlockEntityKind::Bed { head: false })
        );
        assert_eq!(
            BlockEntityKind::of(26, 8),
            Some(BlockEntityKind::Bed { head: true })
        );
    }

    #[test]
    fn the_table_keeps_the_existing_paths() {
        assert_eq!(render_kind(101, 0), RenderKind::Bars);
        assert_eq!(render_kind(51, 0), RenderKind::Fire);
        assert_eq!(render_kind(31, 1), RenderKind::CrossPlant);
        assert_eq!(render_kind(1, 0), RenderKind::Cube);
        assert_eq!(render_kind(44, 0), RenderKind::Shape);
        assert_eq!(render_kind(53, 0), RenderKind::Shape);
        assert_eq!(render_kind(85, 0), RenderKind::Shape);
        assert_eq!(render_kind(102, 0), RenderKind::Shape);
        assert_eq!(render_kind(64, 0), RenderKind::Shape);
    }
}
