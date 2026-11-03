//! Non-full-cube block shapes, in 0..16 block pixels.
//!
//! The chunk mesher consumes this table for non-full-cube geometry. Facing/meta
//! variants are resolved here so the mesher never has to know about stair
//! orientation.
//!
//! Boxes are axis-aligned and measured from the block's minimum corner. Door
//! and pane orientation come straight from their metadata nibbles.

use crate::block::blocks_movement;
use crate::{BlockState, World};

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

/// The centre post of an unconnected pane.
const PANE_POST: ShapeBox = b([7.0, 0.0, 7.0], [9.0, 16.0, 9.0]);
const PANE: &[ShapeBox] = &[PANE_POST];

/// A closed door panel against each horizontal block edge.
const DOOR_NORTH: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 16.0, 3.0])];
const DOOR_EAST: &[ShapeBox] = &[b([13.0, 0.0, 0.0], [16.0, 16.0, 16.0])];
const DOOR_SOUTH: &[ShapeBox] = &[b([0.0, 0.0, 13.0], [16.0, 16.0, 16.0])];
const DOOR_WEST: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [3.0, 16.0, 16.0])];

/// A standing torch: 2x10x2, centred, resting on the block floor.
const TORCH: &[ShapeBox] = &[b([7.0, 0.0, 7.0], [9.0, 10.0, 9.0])];

/// Horizontal pane connections around one block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneConnections {
    pub west: bool,
    pub east: bool,
    pub north: bool,
    pub south: bool,
}

/// The pane's centre post plus one narrow arm for each connected side.
/// Keeping the arms separate avoids turning a four-way pane into a glass cube.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneShape {
    boxes: [ShapeBox; 5],
    len: usize,
}

impl PaneShape {
    pub fn as_slice(&self) -> &[ShapeBox] {
        &self.boxes[..self.len]
    }
}

/// Build pane arms towards adjacent panes or full solid blocks.
pub fn pane_shape(connections: PaneConnections) -> PaneShape {
    let mut boxes = [PANE_POST; 5];
    let mut len = 1;

    if connections.west {
        boxes[len] = b([0.0, 0.0, 7.0], [7.0, 16.0, 9.0]);
        len += 1;
    }
    if connections.east {
        boxes[len] = b([9.0, 0.0, 7.0], [16.0, 16.0, 9.0]);
        len += 1;
    }
    if connections.north {
        boxes[len] = b([7.0, 0.0, 0.0], [9.0, 16.0, 7.0]);
        len += 1;
    }
    if connections.south {
        boxes[len] = b([7.0, 0.0, 9.0], [9.0, 16.0, 16.0]);
        len += 1;
    }

    PaneShape { boxes, len }
}

pub const fn is_pane(id: u16) -> bool {
    matches!(id, 101 | 102 | 160)
}

pub const fn is_door(id: u16) -> bool {
    matches!(id, 64 | 71 | 193 | 194 | 195 | 196 | 197)
}

/// Resolve the legacy door metadata into a thin panel shape.
///
/// The lower half owns facing (`lower_meta & 3`) and open (`lower_meta & 4`);
/// the upper half owns hinge (`upper_meta & 1`).
pub fn door_shape(lower_meta: u8, upper_meta: u8) -> &'static [ShapeBox] {
    let facing = lower_meta & 3;
    let open = lower_meta & 4 != 0;
    let hinge = upper_meta & 1 != 0;
    let panel_facing = if open {
        (facing + if hinge { 1 } else { 3 }) & 3
    } else {
        facing
    };

    match panel_facing {
        0 => DOOR_NORTH,
        1 => DOOR_EAST,
        2 => DOOR_SOUTH,
        _ => DOOR_WEST,
    }
}

/// Quantize player yaw to the nearest horizontal door facing. The result uses
/// north/east/south/west as 0/1/2/3, matching the lower-half facing bits.
pub fn door_facing_from_yaw(yaw: f32) -> u8 {
    if !yaw.is_finite() {
        return 0;
    }
    ((yaw / std::f32::consts::FRAC_PI_2).round() as i32).rem_euclid(4) as u8
}

/// Return the lower and upper states for a newly placed door.
pub fn door_states(id: u16, yaw: f32) -> Option<[BlockState; 2]> {
    if !is_door(id) {
        return None;
    }
    Some([
        BlockState::new(id, door_facing_from_yaw(yaw)),
        BlockState::new(id, 8),
    ])
}

/// Place both halves of a door over a solid support block.
pub fn place_door(world: &mut World, x: i32, y: i32, z: i32, id: u16, yaw: f32) -> bool {
    if y <= 0 || y >= 255 || !is_door(id) {
        return false;
    }
    if !blocks_movement(world.get_block(x, y - 1, z))
        || !world.get_block(x, y, z).is_air()
        || !world.get_block(x, y + 1, z).is_air()
    {
        return false;
    }

    let [lower, upper] = door_states(id, yaw).expect("door id checked above");
    if !world.set_block(x, y, z, lower) {
        return false;
    }
    if !world.set_block(x, y + 1, z, upper) {
        world.set_block(x, y, z, BlockState::AIR);
        return false;
    }
    true
}

/// Toggle a door from either half, changing only the lower half's open bit.
pub fn toggle_door_open(world: &mut World, x: i32, y: i32, z: i32) -> bool {
    let clicked = world.get_block(x, y, z);
    if !is_door(clicked.id()) {
        return false;
    }

    let lower_y = if clicked.meta() & 8 != 0 { y - 1 } else { y };
    if lower_y < 0 || lower_y + 1 >= 256 {
        return false;
    }
    let lower = world.get_block(x, lower_y, z);
    let upper = world.get_block(x, lower_y + 1, z);
    if lower.id() != clicked.id()
        || lower.meta() & 8 != 0
        || upper.id() != clicked.id()
        || upper.meta() & 8 == 0
    {
        return false;
    }

    world.set_block(
        x,
        lower_y,
        z,
        BlockState::new(lower.id(), lower.meta() ^ 4),
    )
}

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
        id if is_door(id) => {
            if meta & 8 != 0 {
                door_shape(0, meta)
            } else {
                door_shape(meta, 0)
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
        assert_eq!(block_shape(64, 0), DOOR_NORTH);
        assert_eq!(block_shape(64, 8), DOOR_NORTH);
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
    fn pane_connections_add_only_the_connected_arms() {
        let isolated = pane_shape(PaneConnections::default());
        assert_eq!(isolated.as_slice(), &[PANE_POST]);

        let north_east = pane_shape(PaneConnections {
            east: true,
            north: true,
            ..PaneConnections::default()
        });
        assert_eq!(north_east.as_slice().len(), 3);
        assert!(north_east
            .as_slice()
            .iter()
            .any(|shape| shape.min == [9.0, 0.0, 7.0] && shape.max == [16.0, 16.0, 9.0]));
        assert!(north_east
            .as_slice()
            .iter()
            .any(|shape| shape.min == [7.0, 0.0, 0.0] && shape.max == [9.0, 16.0, 7.0]));

        let all = pane_shape(PaneConnections {
            west: true,
            east: true,
            north: true,
            south: true,
        });
        assert_eq!(all.as_slice().len(), 5);
        assert!(all.as_slice().iter().all(|shape| !is_full_cube(std::slice::from_ref(shape))));
    }

    #[test]
    fn door_facing_open_and_hinge_bits_select_panel_edges() {
        assert_eq!(door_shape(0, 0), DOOR_NORTH);
        assert_eq!(door_shape(1, 0), DOOR_EAST);
        assert_eq!(door_shape(2, 0), DOOR_SOUTH);
        assert_eq!(door_shape(3, 0), DOOR_WEST);
        assert_eq!(door_shape(4, 0), DOOR_WEST);
        assert_eq!(door_shape(4, 1), DOOR_EAST);
        assert_eq!(door_shape(0, 1), DOOR_NORTH);
    }

    #[test]
    fn door_yaw_quantizes_to_four_horizontal_facings() {
        let quarter_turn = std::f32::consts::FRAC_PI_2;
        assert_eq!(door_facing_from_yaw(0.0), 0);
        assert_eq!(door_facing_from_yaw(quarter_turn), 1);
        assert_eq!(door_facing_from_yaw(2.0 * quarter_turn), 2);
        assert_eq!(door_facing_from_yaw(3.0 * quarter_turn), 3);
        assert_eq!(door_facing_from_yaw(-quarter_turn), 3);
        assert_eq!(door_facing_from_yaw(f32::NAN), 0);
    }

    #[test]
    fn placing_door_writes_two_halves_and_use_toggles_from_either_half() {
        let mut column = crate::ChunkColumn::empty(0, 0);
        column.set_block(4, 63, 4, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        assert!(place_door(
            &mut world,
            4,
            64,
            4,
            64,
            std::f32::consts::FRAC_PI_2
        ));
        assert_eq!(world.get_block(4, 64, 4), BlockState::new(64, 1));
        assert_eq!(world.get_block(4, 65, 4), BlockState::new(64, 8));

        assert!(toggle_door_open(&mut world, 4, 65, 4));
        assert_eq!(world.get_block(4, 64, 4), BlockState::new(64, 5));
        assert_eq!(world.get_block(4, 65, 4), BlockState::new(64, 8));
        assert!(toggle_door_open(&mut world, 4, 64, 4));
        assert_eq!(world.get_block(4, 64, 4), BlockState::new(64, 1));
    }

    #[test]
    fn door_placement_rejects_missing_support_and_obstructed_upper_half() {
        let mut column = crate::ChunkColumn::empty(0, 0);
        column.set_block(4, 65, 4, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        assert!(!place_door(&mut world, 4, 64, 4, 64, 0.0));
        world.set_block(4, 63, 4, BlockState::STONE);
        assert!(!place_door(&mut world, 4, 64, 4, 64, 0.0));
        assert!(world.get_block(4, 64, 4).is_air());
    }

    #[test]
    fn unknown_blocks_stay_cubes_and_air_is_empty() {
        assert_eq!(block_shape(1, 0), FULL);
        assert!(is_full_cube(block_shape(1, 0)));
        assert_eq!(block_shape(0, 0), EMPTY);
    }
}
