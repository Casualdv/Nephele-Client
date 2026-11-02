//! Non-full-cube block shapes, in 0..16 block pixels.
//!
//! The chunk mesher consumes this table for non-full-cube geometry. Facing/meta
//! variants are resolved here so the mesher never has to know about stair
//! orientation.
//!
//! Boxes are axis-aligned and measured from the block's minimum corner. Door
//! and pane orientation come straight from their metadata nibbles.

/// An axis-aligned box inside one block, in 0..16 pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

const fn b(min: [f32; 3], max: [f32; 3]) -> ShapeBox {
    ShapeBox { min, max }
}

const FULL: &[ShapeBox] = &[b([0.0; 3], [16.0; 3])];
const EMPTY: &[ShapeBox] = &[];

/// Slab at the bottom, or at the top for `meta & 8`.
const SLAB_BOTTOM: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0])];
const SLAB_TOP: &[ShapeBox] = &[b([0.0, 8.0, 0.0], [16.0, 16.0, 16.0])];

/// Bottom slab plus the step. The step side follows `meta & 3`.
const STAIRS: [[ShapeBox; 2]; 4] = [
    [
        b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0]),
        b([8.0, 8.0, 0.0], [16.0, 16.0, 16.0]),
    ],
    [
        b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0]),
        b([0.0, 8.0, 0.0], [8.0, 16.0, 16.0]),
    ],
    [
        b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0]),
        b([0.0, 8.0, 8.0], [16.0, 16.0, 16.0]),
    ],
    [
        b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0]),
        b([0.0, 8.0, 0.0], [16.0, 16.0, 8.0]),
    ],
];

/// A fence post; the rails get added once connected-texture state exists.
const FENCE: &[ShapeBox] = &[b([6.0, 0.0, 6.0], [10.0, 16.0, 10.0])];

/// A pane along Z (the thin axis), as placed by default.
const PANE: &[ShapeBox] = &[b([7.0, 0.0, 0.0], [9.0, 16.0, 16.0])];

/// A door slab: thin along X. `meta & 1` shifts it to the other face.
const DOOR: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [3.0, 16.0, 16.0])];
const DOOR_REVERSED: &[ShapeBox] = &[b([13.0, 0.0, 0.0], [16.0, 16.0, 16.0])];

/// A standing torch: 2x10x2, centred, resting on the block floor.
const TORCH: &[ShapeBox] = &[b([7.0, 0.0, 7.0], [9.0, 10.0, 9.0])];

/// The boxes making up `id`/`meta`. Unknown blocks are a single full cube.
pub fn block_shape(id: u16, meta: u8) -> &'static [ShapeBox] {
    match id {
        44 | 126 | 182 => {
            if meta & 8 != 0 {
                SLAB_TOP
            } else {
                SLAB_BOTTOM
            }
        }
        // Stairs: wood, cobble, brick, stone brick, nether brick, quartz,
        // sandstone, red sandstone, purpur.
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180 => {
            &STAIRS[(meta & 3) as usize]
        }
        85 | 113 => FENCE,
        101 | 102 | 160 => PANE,
        64 | 71 | 193 | 194 | 195 | 196 | 197 => {
            if meta & 8 != 0 {
                DOOR
            } else {
                DOOR_REVERSED
            }
        }
        50 => TORCH,
        // Air and blocks with no model yet.
        0 => EMPTY,
        _ => FULL,
    }
}

/// Whether every box fills the block (so the mesher can use the cube path).
pub fn is_full_cube(shape: &[ShapeBox]) -> bool {
    shape.len() == 1 && shape[0].min == [0.0; 3] && shape[0].max == [16.0; 3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slabs_are_half_blocks_that_flip_with_meta() {
        assert_eq!(block_shape(44, 0), SLAB_BOTTOM);
        assert_eq!(block_shape(44, 8), SLAB_TOP);
        assert!(!is_full_cube(block_shape(44, 0)));
    }

    #[test]
    fn stairs_are_a_slab_plus_a_step_in_four_directions() {
        for facing in 0..4 {
            let shape = block_shape(53, facing);
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0].max[1], 8.0, "half slab at the bottom");
            assert_eq!(shape[1].min[1], 8.0, "step on top");
            assert!(
                (shape[1].max[0] - shape[1].min[0]) * (shape[1].max[2] - shape[1].min[2]) <= 128.0
            );
        }
        assert_eq!(block_shape(53, 0)[1].min[0], 8.0);
        assert_eq!(block_shape(53, 1)[1].max[0], 8.0);
        assert_eq!(block_shape(53, 2)[1].min[2], 8.0);
        assert_eq!(block_shape(53, 3)[1].max[2], 8.0);
    }

    #[test]
    fn props_and_doors_are_not_full_cubes() {
        assert_eq!(block_shape(85, 0), FENCE);
        assert_eq!(block_shape(101, 0), PANE);
        assert_eq!(block_shape(64, 0), DOOR_REVERSED);
        assert_eq!(block_shape(64, 8), DOOR);
        assert_eq!(block_shape(50, 0), TORCH);
        for id in [85u16, 101, 64, 50] {
            let shape = block_shape(id, 0);
            assert!(!is_full_cube(shape), "{id} should not be a cube");
            assert!(shape
                .iter()
                .all(|b| b.min.iter().zip(b.max.iter()).all(|(a, b)| a <= b)));
        }
    }

    #[test]
    fn unknown_blocks_stay_cubes_and_air_is_empty() {
        assert_eq!(block_shape(1, 0), FULL);
        assert!(is_full_cube(block_shape(1, 0)));
        assert_eq!(block_shape(0, 0), EMPTY);
    }
}
