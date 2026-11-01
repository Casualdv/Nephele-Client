//! Voxel ray casting — the "what am I looking at" query behind block
//! targeting, block breaking and block placing.
//!
//! Mirrors the parameters vanilla's `World.rayTraceBlocks` is called with for
//! the crosshair: solid blocks only (`stopOnLiquid = false`), at most 200
//! block steps, and the entry face reported in `EnumFacing` terms.

use crate::block::blocks_movement;
use crate::world::World;

/// One of the six block faces, in vanilla `EnumFacing` order.
///
/// The discriminants are exactly the ids protocol 47 puts in
/// `C07 / C08`'s face byte, so `Face as u8` is wire-ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::Down,
        Face::Up,
        Face::North,
        Face::South,
        Face::West,
        Face::East,
    ];

    pub const fn normal(self) -> [i32; 3] {
        match self {
            Face::Down => [0, -1, 0],
            Face::Up => [0, 1, 0],
            Face::North => [0, 0, -1],
            Face::South => [0, 0, 1],
            Face::West => [-1, 0, 0],
            Face::East => [1, 0, 0],
        }
    }

    pub const fn opposite(self) -> Face {
        match self {
            Face::Down => Face::Up,
            Face::Up => Face::Down,
            Face::North => Face::South,
            Face::South => Face::North,
            Face::West => Face::East,
            Face::East => Face::West,
        }
    }

    pub const fn protocol_id(self) -> u8 {
        self as u8
    }

    pub const fn from_protocol_id(id: u8) -> Option<Face> {
        match id {
            0 => Some(Face::Down),
            1 => Some(Face::Up),
            2 => Some(Face::North),
            3 => Some(Face::South),
            4 => Some(Face::West),
            5 => Some(Face::East),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Face::Down => "down",
            Face::Up => "up",
            Face::North => "north",
            Face::South => "south",
            Face::West => "west",
            Face::East => "east",
        }
    }
}

/// Where a ray entered a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// The block that was hit.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// The face the ray came in through (i.e. the face you can see).
    pub face: Face,
    /// Distance from the ray origin, in blocks.
    pub distance: f64,
    /// Exact world-space point of the intersection.
    pub point: [f64; 3],
}

/// Vanilla's step cap for a block ray trace.
pub const MAX_STEPS: i32 = 200;

/// Trace from `origin` towards `origin + direction * reach`.
pub fn raycast(world: &World, origin: [f64; 3], direction: [f64; 3], reach: f64) -> Option<RayHit> {
    let end = [
        origin[0] + direction[0] * reach,
        origin[1] + direction[1] * reach,
        origin[2] + direction[2] * reach,
    ];
    raycast_to(world, origin, end)
}

/// Trace between two points. Returns the first block that `blocks_movement`.
pub fn raycast_to(world: &World, origin: [f64; 3], end: [f64; 3]) -> Option<RayHit> {
    if origin
        .iter()
        .chain(end.iter())
        .any(|value| !value.is_finite())
    {
        return None;
    }

    let delta = [end[0] - origin[0], end[1] - origin[1], end[2] - origin[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if length < 1e-9 {
        return None;
    }

    let start = [
        origin[0].floor() as i32,
        origin[1].floor() as i32,
        origin[2].floor() as i32,
    ];
    let stop = [
        end[0].floor() as i32,
        end[1].floor() as i32,
        end[2].floor() as i32,
    ];

    let hit = |x: i32, y: i32, z: i32, face: Face, t: f64| RayHit {
        x,
        y,
        z,
        face,
        distance: t * length,
        point: [
            origin[0] + delta[0] * t,
            origin[1] + delta[1] * t,
            origin[2] + delta[2] * t,
        ],
    };

    // A ray that starts inside a block reports that block immediately, facing
    // back along the dominant axis of travel.
    if blocks_movement(world.get_block(start[0], start[1], start[2])) {
        let axis = dominant_axis(delta);
        let face = match axis {
            0 if delta[0] > 0.0 => Face::West,
            0 => Face::East,
            1 if delta[1] > 0.0 => Face::Down,
            1 => Face::Up,
            _ if delta[2] > 0.0 => Face::North,
            _ => Face::South,
        };
        return Some(hit(start[0], start[1], start[2], face, 0.0));
    }

    // Distance (in normalised ray units) to the first grid plane on each axis.
    let mut t_max = [f64::INFINITY; 3];
    let mut t_step = [f64::INFINITY; 3];
    let mut step = [0i32; 3];
    for axis in 0..3 {
        if delta[axis] == 0.0 {
            continue;
        }
        step[axis] = if delta[axis] > 0.0 { 1 } else { -1 };
        let boundary = if delta[axis] > 0.0 {
            (start[axis] + 1) as f64
        } else {
            start[axis] as f64
        };
        t_max[axis] = (boundary - origin[axis]) / delta[axis];
        t_step[axis] = (1.0 / delta[axis]).abs();
    }

    // Both ends inside one cell: nothing to step through.
    if start == stop {
        return None;
    }

    let mut cell = start;
    for _ in 0..MAX_STEPS {
        // Advance across whichever plane comes first.
        let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };

        // A ray that ends exactly on a block face still counts as a hit.
        let t = t_max[axis];
        if t > 1.0 {
            return None;
        }

        let face = match axis {
            0 if step[0] > 0 => Face::West,
            0 => Face::East,
            1 if step[1] > 0 => Face::Down,
            1 => Face::Up,
            _ if step[2] > 0 => Face::North,
            _ => Face::South,
        };

        cell[axis] += step[axis];
        t_max[axis] += t_step[axis];

        if blocks_movement(world.get_block(cell[0], cell[1], cell[2])) {
            return Some(hit(cell[0], cell[1], cell[2], face, t));
        }
        if cell == stop {
            return None;
        }
    }
    None
}

fn dominant_axis(delta: [f64; 3]) -> usize {
    let mut axis = 0;
    for candidate in 1..3 {
        if delta[candidate].abs() > delta[axis].abs() {
            axis = candidate;
        }
    }
    axis
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkColumn;
    use crate::world::World;
    use crate::BlockState;

    fn floor_world() -> World {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
        }
        let mut world = World::new(0);
        world.insert(column);
        world
    }

    #[test]
    fn straight_down_hits_the_floor_top_face() {
        let world = floor_world();
        // Reach past the floor's top face at y = 65 so the ray actually enters it.
        let hit = raycast(&world, [8.5, 70.0, 8.5], [0.0, -1.0, 0.0], 5.5).unwrap();
        assert_eq!((hit.x, hit.y, hit.z), (8, 64, 8));
        assert_eq!(hit.face, Face::Up);
        assert!((hit.distance - 5.0).abs() < 1e-6, "{}", hit.distance);
        assert!((hit.point[1] - 65.0).abs() < 1e-6);
    }

    #[test]
    fn empty_world_has_no_hit() {
        let world = World::new(0);
        assert!(raycast(&world, [0.5, 70.0, 0.5], [0.0, -1.0, 0.0], 8.0).is_none());
    }

    #[test]
    fn reach_limit_is_respected() {
        let world = floor_world();
        // Floor top is 5 blocks away; a 4-block reach must not find it.
        assert!(raycast(&world, [8.5, 70.0, 8.5], [0.0, -1.0, 0.0], 4.0).is_none());
    }

    #[test]
    fn picks_the_nearest_block_along_the_ray() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(10, 40, 0, BlockState::STONE);
        column.set_block(12, 40, 0, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        let hit = raycast(&world, [8.5, 40.5, 0.5], [1.0, 0.0, 0.0], 8.0).unwrap();
        assert_eq!(hit.x, 10);
        assert_eq!(hit.face, Face::West);
        assert!((hit.distance - 1.5).abs() < 1e-6, "{}", hit.distance);
    }

    #[test]
    fn detects_the_entry_face_on_every_axis() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(8, 40, 8, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        let cases = [
            ([8.5, 40.5, 20.0], [0.0, 0.0, -1.0], Face::South),
            ([8.5, 40.5, -10.0], [0.0, 0.0, 1.0], Face::North),
            ([20.0, 40.5, 8.5], [-1.0, 0.0, 0.0], Face::East),
            ([-10.0, 40.5, 8.5], [1.0, 0.0, 0.0], Face::West),
            ([8.5, 60.0, 8.5], [0.0, -1.0, 0.0], Face::Up),
            ([8.5, 20.0, 8.5], [0.0, 1.0, 0.0], Face::Down),
        ];
        for (origin, direction, expected) in cases {
            let hit = raycast(&world, origin, direction, 60.0)
                .unwrap_or_else(|| panic!("no hit for {direction:?}"));
            assert_eq!((hit.x, hit.y, hit.z), (8, 40, 8));
            assert_eq!(hit.face, expected, "direction {direction:?}");
        }
    }

    #[test]
    fn water_does_not_stop_the_ray() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(8, 50, 8, BlockState::WATER);
        column.set_block(8, 40, 8, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        let hit = raycast(&world, [8.5, 60.0, 8.5], [0.0, -1.0, 0.0], 30.0).unwrap();
        assert_eq!(hit.y, 40);
        assert_eq!(hit.face, Face::Up);
    }

    #[test]
    fn starting_inside_a_block_reports_it() {
        let world = floor_world();
        let hit = raycast(&world, [8.5, 64.5, 8.5], [0.0, -1.0, 0.0], 5.0).unwrap();
        assert_eq!((hit.x, hit.y, hit.z), (8, 64, 8));
        assert_eq!(hit.distance, 0.0);
    }
}
