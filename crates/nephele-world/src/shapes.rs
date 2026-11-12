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

/// A fence post; the rails are added by `fence_shape` when it connects.
const FENCE: &[ShapeBox] = &[b([6.0, 0.0, 6.0], [10.0, 16.0, 10.0])];

/// An isolated iron bar: the vanilla two-pixel post.
const BARS_POST: ShapeBox = b([7.0, 0.0, 7.0], [9.0, 16.0, 9.0]);

/// Iron bars, as one two-pixel-thick half-slab per connected side.
///
/// Vanilla's `bars_*` models are zero-thickness planes plus separate two-pixel
/// rim boxes. A thin slab has the same silhouette from every angle and needs no
/// rim geometry, which matters here because the mesher has no per-face UV
/// sub-ranges to map those rims correctly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarsShape {
    boxes: [ShapeBox; 4],
    len: usize,
}

impl BarsShape {
    pub fn as_slice(&self) -> &[ShapeBox] {
        &self.boxes[..self.len]
    }
}

pub fn bars_shape(connections: PaneConnections) -> BarsShape {
    let mut boxes = [BARS_POST; 4];
    let mut len = 0;
    if connections.west {
        boxes[len] = b([0.0, 0.0, 7.0], [8.0, 16.0, 9.0]);
        len += 1;
    }
    if connections.east {
        boxes[len] = b([8.0, 0.0, 7.0], [16.0, 16.0, 9.0]);
        len += 1;
    }
    if connections.north {
        boxes[len] = b([7.0, 0.0, 0.0], [9.0, 16.0, 8.0]);
        len += 1;
    }
    if connections.south {
        boxes[len] = b([7.0, 0.0, 8.0], [9.0, 16.0, 16.0]);
        len += 1;
    }
    if len == 0 {
        boxes[0] = BARS_POST;
        len = 1;
    }
    BarsShape { boxes, len }
}

/// Fence post plus the two rails on each connected side. The rails are the
/// vanilla heights (6..9 and 12..15), so a fence line reads as rails rather
/// than a solid wall of planks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FenceShape {
    boxes: [ShapeBox; 9],
    len: usize,
}

impl FenceShape {
    pub fn as_slice(&self) -> &[ShapeBox] {
        &self.boxes[..self.len]
    }
}

pub fn fence_shape(connections: PaneConnections) -> FenceShape {
    let mut boxes = [b([6.0, 0.0, 6.0], [10.0, 16.0, 10.0]); 9];
    let mut len = 1;
    let mut rail = |min: [f32; 3], max: [f32; 3]| {
        boxes[len] = b(min, max);
        len += 1;
    };
    if connections.west {
        rail([0.0, 6.0, 6.0], [6.0, 9.0, 10.0]);
        rail([0.0, 12.0, 6.0], [6.0, 15.0, 10.0]);
    }
    if connections.east {
        rail([10.0, 6.0, 6.0], [16.0, 9.0, 10.0]);
        rail([10.0, 12.0, 6.0], [16.0, 15.0, 10.0]);
    }
    if connections.north {
        rail([6.0, 6.0, 0.0], [10.0, 9.0, 6.0]);
        rail([6.0, 12.0, 0.0], [10.0, 15.0, 6.0]);
    }
    if connections.south {
        rail([6.0, 6.0, 10.0], [10.0, 9.0, 16.0]);
        rail([6.0, 12.0, 10.0], [10.0, 15.0, 16.0]);
    }
    FenceShape { boxes, len }
}

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

/// A pressure plate's selection bounds: inset 1/16 on each side and 1/16 tall.
const PLATE: &[ShapeBox] = &[b([1.0, 0.0, 1.0], [15.0, 1.0, 15.0])];
/// The same plate while pressed, at half its thickness.
const PLATE_PRESSED: &[ShapeBox] = &[b([1.0, 0.0, 1.0], [15.0, 0.5, 15.0])];

/// Cross-quad plants. They have no collision and must not be treated as a
/// full cube by neighbour culling or ambient occlusion: a flower that claims
/// to be solid culls the grass top underneath it and leaves a hole.
const CROSS_PLANTS: [u16; 14] = [6, 31, 32, 37, 38, 39, 40, 59, 83, 104, 105, 115, 141, 142];

/// Flat ground cover: lily pads and carpet occupy one pixel.
const FLAT_LAYER: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 1.0, 16.0])];

/// Snow layers: two pixels per level, one to eight levels.
const SNOW_LAYERS: [&[ShapeBox]; 8] = [
    &[b([0.0, 0.0, 0.0], [16.0, 2.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 4.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 6.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 8.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 10.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 12.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 14.0, 16.0])],
    &[b([0.0, 0.0, 0.0], [16.0, 16.0, 16.0])],
];

/// Rails: vanilla's flat plane, given a pixel of thickness so it survives
/// grazing angles. Slopes (`meta & 7` 2..5) are drawn flat as well; the mesher
/// has no per-vertex ramp support and a flat rail reads better than a cube.
const RAIL: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 1.0, 16.0])];

/// Redstone wire and tripwire: the same flat plane, below the feet.
const WIRE: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 1.0, 16.0])];

/// A tripwire hook's post; the arm that points at the string is cosmetic here.
const TRIPWIRE_HOOK: &[ShapeBox] = &[b([6.0, 0.0, 6.0], [10.0, 10.0, 10.0])];

/// Ladders: a one-pixel plate against the wall the metadata names.
const LADDER_NORTH: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 16.0, 1.0])];
const LADDER_SOUTH: &[ShapeBox] = &[b([0.0, 0.0, 15.0], [16.0, 16.0, 16.0])];
const LADDER_WEST: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [1.0, 16.0, 16.0])];
const LADDER_EAST: &[ShapeBox] = &[b([15.0, 0.0, 0.0], [16.0, 16.0, 16.0])];

/// A bed is 9/16 tall in vanilla; the model's corner posts are cosmetic.
const BED: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 9.0, 16.0])];

/// Button: 6x6 pixels on a wall, 2 pixels proud of it. Meta 0 is the floor,
/// 5 the ceiling, 1..4 the four walls (`BlockButton.getFacing`).
const BUTTON_DOWN: &[ShapeBox] = &[b([5.0, 0.0, 5.0], [11.0, 2.0, 11.0])];
const BUTTON_UP: &[ShapeBox] = &[b([5.0, 14.0, 5.0], [11.0, 16.0, 11.0])];
const BUTTON_EAST: &[ShapeBox] = &[b([14.0, 5.0, 5.0], [16.0, 11.0, 11.0])];
const BUTTON_WEST: &[ShapeBox] = &[b([0.0, 5.0, 5.0], [2.0, 11.0, 11.0])];
const BUTTON_SOUTH: &[ShapeBox] = &[b([5.0, 5.0, 14.0], [11.0, 11.0, 16.0])];
const BUTTON_NORTH: &[ShapeBox] = &[b([5.0, 5.0, 0.0], [11.0, 11.0, 2.0])];

/// Lever: the cobblestone base plus a centred handle. Vanilla tilts the handle
/// and flips it for powered levers; the box keeps the silhouette.
const LEVER: &[ShapeBox] = &[
    b([5.0, 0.0, 4.0], [11.0, 3.0, 12.0]),
    b([7.0, 3.0, 7.0], [9.0, 10.0, 9.0]),
];

/// Anvil: a 12-pixel base with a narrower top. `meta & 1` swaps the long axis.
const ANVIL_X: &[ShapeBox] = &[
    b([2.0, 0.0, 2.0], [14.0, 4.0, 14.0]),
    b([3.0, 4.0, 5.0], [13.0, 10.0, 11.0]),
];
const ANVIL_Z: &[ShapeBox] = &[
    b([2.0, 0.0, 2.0], [14.0, 4.0, 14.0]),
    b([5.0, 4.0, 3.0], [11.0, 10.0, 13.0]),
];

/// Enchanting table and daylight detector: full footprint, shorter body.
const ENCHANTING_TABLE: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 12.0, 16.0])];
const DAYLIGHT_DETECTOR: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 6.0, 16.0])];

/// Cake: 14x8x14 on the plate, bites not modelled.
const CAKE: &[ShapeBox] = &[b([1.0, 0.0, 1.0], [15.0, 8.0, 15.0])];

/// Cauldron: four 4-pixel walls around a 2-pixel floor.
const CAULDRON: &[ShapeBox] = &[
    b([0.0, 0.0, 0.0], [16.0, 4.0, 2.0]),
    b([0.0, 0.0, 14.0], [16.0, 4.0, 16.0]),
    b([0.0, 0.0, 2.0], [2.0, 4.0, 14.0]),
    b([14.0, 0.0, 2.0], [16.0, 4.0, 14.0]),
    b([2.0, 0.0, 2.0], [14.0, 2.0, 14.0]),
];

/// A flower pot's bowl.
const FLOWER_POT: &[ShapeBox] = &[b([5.0, 0.0, 5.0], [11.0, 6.0, 11.0])];

/// A skull sitting on the block.
const SKULL: &[ShapeBox] = &[b([4.0, 0.0, 4.0], [12.0, 8.0, 12.0])];

/// Piston head: a 4-pixel cap on the side the metadata faces.
const PISTON_HEAD_DOWN: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 4.0, 16.0])];
const PISTON_HEAD_UP: &[ShapeBox] = &[b([0.0, 12.0, 0.0], [16.0, 16.0, 16.0])];
const PISTON_HEAD_NORTH: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 16.0, 4.0])];
const PISTON_HEAD_SOUTH: &[ShapeBox] = &[b([0.0, 0.0, 12.0], [16.0, 16.0, 16.0])];
const PISTON_HEAD_WEST: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [4.0, 16.0, 16.0])];
const PISTON_HEAD_EAST: &[ShapeBox] = &[b([12.0, 0.0, 0.0], [16.0, 16.0, 16.0])];

/// A closed trapdoor, bottom or top half; an open one is a 3-pixel plate on
/// the facing side.
const TRAPDOOR_BOTTOM: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 3.0, 16.0])];
const TRAPDOOR_TOP: &[ShapeBox] = &[b([0.0, 13.0, 0.0], [16.0, 16.0, 16.0])];
const TRAPDOOR_NORTH: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [16.0, 16.0, 3.0])];
const TRAPDOOR_SOUTH: &[ShapeBox] = &[b([0.0, 0.0, 13.0], [16.0, 16.0, 16.0])];
const TRAPDOOR_WEST: &[ShapeBox] = &[b([0.0, 0.0, 0.0], [3.0, 16.0, 16.0])];
const TRAPDOOR_EAST: &[ShapeBox] = &[b([13.0, 0.0, 0.0], [16.0, 16.0, 16.0])];

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

/// Wooden and nether-brick fences share the post-and-rails geometry.
pub const fn is_fence(id: u16) -> bool {
    matches!(id, 85 | 113 | 139)
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

    world.set_block(x, lower_y, z, BlockState::new(lower.id(), lower.meta() ^ 4))
}

/// The boxes making up `id`/`meta`. Unknown blocks are a single full cube.
pub fn block_shape(id: u16, meta: u8) -> &'static [ShapeBox] {
    match id {
        44 | 126 | 182 | 205 => {
            if meta & 8 != 0 {
                SLAB_TOP
            } else {
                SLAB_BOTTOM
            }
        }
        // Stairs: wood, cobble, brick, stone brick, nether brick, quartz,
        // sandstone, red sandstone, purpur.
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180 | 203 => {
            &STAIRS[(meta & 3) as usize]
        }
        85 | 113 | 139 => FENCE,
        101 | 102 | 160 => PANE,
        id if is_door(id) => {
            if meta & 8 != 0 {
                door_shape(0, meta)
            } else {
                door_shape(meta, 0)
            }
        }
        50 | 75 | 76 => TORCH,
        // Stone, wooden, weighted iron and weighted gold plates. Metadata is
        // the pressed flag for the first two and the redstone strength for the
        // weighted ones; either way a non-zero value means pressed.
        70 | 72 | 147 | 148 => {
            if meta != 0 {
                PLATE_PRESSED
            } else {
                PLATE
            }
        }
        // Rails: powered, detector and activator variants share the shape.
        27 | 28 | 66 | 157 => RAIL,
        55 | 132 => WIRE,
        131 => TRIPWIRE_HOOK,
        65 => match meta {
            2 => LADDER_NORTH,
            3 => LADDER_SOUTH,
            4 => LADDER_WEST,
            _ => LADDER_EAST,
        },
        26 => BED,
        143 | 77 => match meta & 7 {
            0 => BUTTON_DOWN,
            1 => BUTTON_EAST,
            2 => BUTTON_WEST,
            3 => BUTTON_SOUTH,
            4 => BUTTON_NORTH,
            _ => BUTTON_UP,
        },
        69 => LEVER,
        145 => {
            if meta & 1 == 0 {
                ANVIL_X
            } else {
                ANVIL_Z
            }
        }
        116 => ENCHANTING_TABLE,
        151 | 178 => DAYLIGHT_DETECTOR,
        92 => CAKE,
        118 => CAULDRON,
        140 => FLOWER_POT,
        144 => SKULL,
        34 | 36 => match meta & 7 {
            0 => PISTON_HEAD_DOWN,
            1 => PISTON_HEAD_UP,
            2 => PISTON_HEAD_NORTH,
            3 => PISTON_HEAD_SOUTH,
            4 => PISTON_HEAD_WEST,
            _ => PISTON_HEAD_EAST,
        },
        96 | 167 => {
            if meta & 4 != 0 {
                // Open: the panel swings to the side the metadata faces.
                match meta & 3 {
                    0 => TRAPDOOR_NORTH,
                    1 => TRAPDOOR_EAST,
                    2 => TRAPDOOR_SOUTH,
                    _ => TRAPDOOR_WEST,
                }
            } else if meta & 8 != 0 {
                TRAPDOOR_TOP
            } else {
                TRAPDOOR_BOTTOM
            }
        }
        // Cross-quad plants and crops occupy no solid volume. The mesher draws
        // their geometry; this entry is what keeps culling and AO honest.
        id if CROSS_PLANTS.contains(&id) => EMPTY,
        175 => EMPTY,
        78 => SNOW_LAYERS[(meta & 7) as usize],
        111 | 171 => FLAT_LAYER,
        // Fire is drawn as crossed, animated quads by the mesher; it must not
        // masquerade as a full cube to its neighbours.
        51 => EMPTY,
        // Air and blocks with no model yet.
        0 => EMPTY,
        _ => FULL,
    }
}

/// Whether entities collide with `state`.
///
/// Vanilla's `BlockBasePressurePlate.getCollisionBoundingBox` returns null for
/// every plate, so a player walks through it and stands on the block below.
/// The 1/16 box is the selection/render shape, not a collision volume.
pub fn has_collision(state: BlockState) -> bool {
    !matches!(state.id(), 70 | 72 | 147 | 148) && blocks_movement(state)
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
    fn plants_cover_and_fire_are_never_full_cubes() {
        for id in [
            6u16, 31, 32, 37, 38, 39, 40, 51, 59, 78, 83, 104, 105, 111, 115, 141, 142, 171, 175,
        ] {
            assert!(
                !is_full_cube(block_shape(id, 0)),
                "{id} must not report a full cube"
            );
        }
        assert_eq!(block_shape(37, 0), EMPTY);
        assert_eq!(block_shape(78, 7)[0].max[1], 16.0);
        assert_eq!(block_shape(111, 0)[0].max[1], 1.0);
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
    fn fences_get_rails_only_where_they_connect() {
        let isolated = fence_shape(PaneConnections::default());
        assert_eq!(
            isolated.as_slice(),
            &[b([6.0, 0.0, 6.0], [10.0, 16.0, 10.0])]
        );

        let east = fence_shape(PaneConnections {
            east: true,
            ..PaneConnections::default()
        });
        assert_eq!(east.as_slice().len(), 3);
        assert!(east
            .as_slice()
            .iter()
            .any(|part| part.min == [10.0, 6.0, 6.0] && part.max == [16.0, 9.0, 10.0]));
        assert!(east
            .as_slice()
            .iter()
            .any(|part| part.min == [10.0, 12.0, 6.0] && part.max == [16.0, 15.0, 10.0]));

        let all = fence_shape(PaneConnections {
            west: true,
            east: true,
            north: true,
            south: true,
        });
        assert_eq!(all.as_slice().len(), 9);
    }

    #[test]
    fn bars_are_two_pixel_slabs_not_paper_thin_cards() {
        // An isolated bar is the vanilla two-pixel post.
        let isolated = bars_shape(PaneConnections::default());
        assert_eq!(isolated.as_slice(), &[BARS_POST]);

        // A north-south line is one slab per half block, two pixels thick in x
        // and continuous along z: a plane would vanish edge-on, which is the
        // "cross-plant" look this shape exists to avoid.
        let line = bars_shape(PaneConnections {
            north: true,
            south: true,
            ..PaneConnections::default()
        });
        assert_eq!(line.as_slice().len(), 2);
        let north = line.as_slice()[0];
        assert_eq!(north.min, [7.0, 0.0, 0.0]);
        assert_eq!(north.max, [9.0, 16.0, 8.0]);
        let south = line.as_slice()[1];
        assert_eq!(south.min, [7.0, 0.0, 8.0]);
        assert_eq!(south.max, [9.0, 16.0, 16.0]);
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
        assert!(all
            .as_slice()
            .iter()
            .all(|shape| !is_full_cube(std::slice::from_ref(shape))));
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
    fn pressure_plates_are_a_sixteenth_high_and_walk_through() {
        let plate = block_shape(72, 0);
        assert_eq!(plate.len(), 1);
        assert_eq!(
            plate[0].max[1], 1.0,
            "one pixel is 1/16 of a block, not the old full cube"
        );
        assert_eq!(plate[0].min[0], 1.0);
        assert_eq!(plate[0].max[0], 15.0);
        assert_eq!(
            block_shape(72, 1)[0].max[1],
            0.5,
            "pressed plates are lower"
        );

        for id in [70u16, 72, 147, 148] {
            assert!(!has_collision(BlockState::new(id, 0)), "id {id}");
            assert!(!is_full_cube(block_shape(id, 0)), "id {id}");
        }
        assert!(has_collision(BlockState::new(44, 0)), "slabs still collide");
        assert!(has_collision(BlockState::STONE));
    }

    #[test]
    fn unknown_blocks_stay_cubes_and_air_is_empty() {
        assert_eq!(block_shape(1, 0), FULL);
        assert!(is_full_cube(block_shape(1, 0)));
        assert_eq!(block_shape(0, 0), EMPTY);
    }

    /// The second pass over the shape audit: flat plates and simple boxes that
    /// used to fall through to the cube path.
    #[test]
    fn the_remaining_audit_boxes_are_not_cubes() {
        for (id, meta) in [
            (26u16, 0u8), // bed
            (27, 0),      // powered rail
            (28, 0),      // detector rail
            (34, 2),      // piston head
            (55, 0),      // redstone wire
            (65, 2),      // ladder
            (66, 0),      // rail
            (69, 0),      // lever
            (77, 0),      // stone button
            (92, 0),      // cake
            (96, 0),      // trapdoor
            (116, 0),     // enchanting table
            (118, 0),     // cauldron
            (131, 0),     // tripwire hook
            (132, 0),     // tripwire
            (140, 0),     // flower pot
            (143, 0),     // button
            (144, 0),     // skull
            (145, 0),     // anvil
            (151, 0),     // daylight detector
            (157, 0),     // activator rail
            (167, 0),     // iron trapdoor
            (178, 0),     // inverted daylight detector
        ] {
            let shape = block_shape(id, meta);
            assert!(!shape.is_empty(), "block {id}:{meta} has no shape");
            assert!(!is_full_cube(shape), "block {id}:{meta} is still a cube");
        }
    }

    #[test]
    fn ladders_hug_the_wall_their_metadata_names() {
        assert_eq!(block_shape(65, 2), LADDER_NORTH);
        assert_eq!(block_shape(65, 3), LADDER_SOUTH);
        assert_eq!(block_shape(65, 4), LADDER_WEST);
        assert_eq!(block_shape(65, 5), LADDER_EAST);
    }

    #[test]
    fn trapdoors_flip_between_halves_and_walls() {
        assert_eq!(block_shape(96, 0), TRAPDOOR_BOTTOM);
        assert_eq!(block_shape(96, 8), TRAPDOOR_TOP);
        assert_eq!(block_shape(96, 4), TRAPDOOR_NORTH);
        assert_eq!(block_shape(96, 5), TRAPDOOR_EAST);
        assert_eq!(block_shape(96, 12), TRAPDOOR_NORTH);
    }

    #[test]
    fn anvils_turn_with_their_metadata() {
        assert_eq!(block_shape(145, 0), ANVIL_X);
        assert_eq!(block_shape(145, 1), ANVIL_Z);
        // The top sits on the base either way.
        assert_eq!(block_shape(145, 0)[1].min[1], 4.0);
    }
}
