//! `MapGenCavesHell` — the nether's only carver.
//!
//! Reimplemented from the 1.8.9 class of the same name. It shares
//! [`MapGenBase`](crate::caves)'s 17×17 source-chunk sweep with the overworld
//! carver, but the tunnels are a different algorithm: a tunnel **aborts entirely
//! when its bounding box touches lava** instead of replacing blocks with it, and
//! rooms are spheres rather than collapsed caverns.
//!
//! Two details are load-bearing for parity: the sine/cosine *table* (see
//! [`crate::math`]) and the order of `Random` draws, including the lava scan's
//! short-circuit.

use crate::block::{ids, BlockState};
use crate::chunk::ChunkColumn;
use crate::math;
use crate::noise::JavaRandom;

/// `MapGenBase.range`: each chunk is carved from a 17×17 neighbourhood.
const RANGE: i32 = 8;

/// The nether carver. Stateless apart from the world seed; every chunk's
/// sequence is a pure function of the seed and the source-chunk coordinates.
#[derive(Debug, Clone)]
pub struct HellCaves {
    seed: i64,
}

impl HellCaves {
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// `MapGenBase.generate`: walk the 17×17 neighbourhood and let each source
    /// chunk carve into `column` (the target chunk).
    pub fn carve(&self, column: &mut ChunkColumn) {
        let target_x = column.x;
        let target_z = column.z;

        let mut base = JavaRandom::new(self.seed);
        let step_x = base.next_long();
        let step_z = base.next_long();

        for source_x in target_x - RANGE..=target_x + RANGE {
            for source_z in target_z - RANGE..=target_z + RANGE {
                let seed = i64::from(source_x).wrapping_mul(step_x)
                    ^ i64::from(source_z).wrapping_mul(step_z)
                    ^ self.seed;
                let mut rand = JavaRandom::new(seed);
                self.recursive(&mut rand, source_x, source_z, target_x, target_z, column);
            }
        }
    }

    /// `recursiveGenerate`: how many caverns this source chunk starts.
    fn recursive(
        &self,
        rand: &mut JavaRandom,
        source_x: i32,
        source_z: i32,
        target_x: i32,
        target_z: i32,
        column: &mut ChunkColumn,
    ) {
        // Java's nested call reads inside-out: nextInt(10), then nextInt(a+1),
        // then nextInt(b+1).
        let inner = rand.next_int_bound(10);
        let middle = rand.next_int_bound(inner + 1);
        let mut count = rand.next_int_bound(middle + 1);
        if rand.next_int_bound(5) != 0 {
            count = 0;
        }

        for _ in 0..count {
            let x = f64::from(source_x * 16 + rand.next_int_bound(16));
            let y = f64::from(rand.next_int_bound(128));
            let z = f64::from(source_z * 16 + rand.next_int_bound(16));

            let mut branches = 1i32;
            if rand.next_int_bound(4) == 0 {
                // A room: radius 1..7, no direction, step = -1 (sphere mode).
                let seed = rand.next_long();
                let radius = 1.0 + rand.next_float() * 6.0;
                self.tunnel(
                    seed, target_x, target_z, column, x, y, z, radius, 0.0, 0.0, -1, -1, 0.5,
                );
                branches += rand.next_int_bound(4);
            }

            for _ in 0..branches {
                let yaw = rand.next_float() * std::f32::consts::PI * 2.0;
                let pitch = (rand.next_float() - 0.5) * 2.0 / 8.0;
                let radius = rand.next_float() * 2.0 + rand.next_float();
                let seed = rand.next_long();
                // `func_180704_a(..., f2 * 2.0F, ...)`: the branch radius is
                // doubled at the call site. Passing `f2` here halves every
                // tunnel and, because the lava scan box is built from the
                // radius, makes tunnels abort less often than vanilla's.
                self.tunnel(
                    seed,
                    target_x,
                    target_z,
                    column,
                    x,
                    y,
                    z,
                    radius * 2.0,
                    yaw,
                    pitch,
                    0,
                    0,
                    0.5,
                );
            }
        }
    }

    /// `func_180704_a`: one cavern, carved as a chain of ellipsoids.
    #[allow(clippy::too_many_arguments)]
    fn tunnel(
        &self,
        seed: i64,
        target_x: i32,
        target_z: i32,
        column: &mut ChunkColumn,
        mut x: f64,
        mut y: f64,
        mut z: f64,
        radius: f32,
        mut yaw: f32,
        mut pitch: f32,
        mut step: i32,
        mut length: i32,
        taper: f64,
    ) {
        let centre_x = f64::from(target_x * 16 + 8);
        let centre_z = f64::from(target_z * 16 + 8);
        let mut drift_yaw = 0.0f32;
        let mut drift_pitch = 0.0f32;

        let mut rand = JavaRandom::new(seed);

        if length <= 0 {
            // `range * 16 - 16` = 112; the room/segment never spans the whole
            // neighbourhood.
            let span = RANGE * 16 - 16;
            length = span - rand.next_int_bound(span / 4);
        }

        let room = step == -1;
        if room {
            step = length / 2;
        }

        let fork_at = rand.next_int_bound(length / 2) + length / 4;
        let round = rand.next_int_bound(6) == 0;

        while step < length {
            let width = 1.5
                + f64::from(
                    math::sin(step as f32 * std::f32::consts::PI / length as f32) * radius * 1.0,
                );
            let height = width * taper;
            let planar = math::cos(pitch);
            let vertical = math::sin(pitch);
            x += f64::from(math::cos(yaw) * planar);
            y += f64::from(vertical);
            z += f64::from(math::sin(yaw) * planar);

            // `p_180704_14_` is the *vertical* angle: vanilla scales it by
            // 0.92/0.7 and feeds it the 0.9/×2 drift (`f1`). Yaw
            // (`p_180704_13_`) only takes the 0.75/×4 drift (`f`). Swapping the
            // pair bends every tunnel away from vanilla's path.
            pitch *= if round { 0.92 } else { 0.7 };
            pitch += drift_pitch * 0.1;
            yaw += drift_yaw * 0.1;
            drift_pitch *= 0.9;
            drift_yaw *= 0.75;
            drift_pitch += (rand.next_float() - rand.next_float()) * rand.next_float() * 2.0;
            drift_yaw += (rand.next_float() - rand.next_float()) * rand.next_float() * 4.0;

            if !room && step == fork_at && radius > 1.0 {
                // Split into two child tunnels and stop the parent here.
                let seed = rand.next_long();
                let child_radius = rand.next_float() * 0.5 + 0.5;
                self.tunnel(
                    seed,
                    target_x,
                    target_z,
                    column,
                    x,
                    y,
                    z,
                    child_radius,
                    yaw - (std::f32::consts::PI / 2.0),
                    pitch / 3.0,
                    step,
                    length,
                    1.0,
                );
                let seed = rand.next_long();
                let child_radius = rand.next_float() * 0.5 + 0.5;
                self.tunnel(
                    seed,
                    target_x,
                    target_z,
                    column,
                    x,
                    y,
                    z,
                    child_radius,
                    yaw + (std::f32::consts::PI / 2.0),
                    pitch / 3.0,
                    step,
                    length,
                    1.0,
                );
                return;
            }

            if room || rand.next_int_bound(4) != 0 {
                let dx = x - centre_x;
                let dz = z - centre_z;
                let remaining = f64::from(length - step);
                let reach = f64::from(radius + 2.0 + 16.0);
                if dx * dx + dz * dz - remaining * remaining > reach * reach {
                    return;
                }

                if x >= centre_x - 16.0 - width * 2.0
                    && z >= centre_z - 16.0 - width * 2.0
                    && x <= centre_x + 16.0 + width * 2.0
                    && z <= centre_z + 16.0 + width * 2.0
                {
                    let from_x = (math::floor(x - width) - target_x * 16 - 1).max(0);
                    let to_x = (math::floor(x + width) - target_x * 16 + 1).min(16);
                    let from_y = (math::floor(y - height) - 1).max(1);
                    let to_y = (math::floor(y + height) + 1).min(120);
                    let from_z = (math::floor(z - width) - target_z * 16 - 1).max(0);
                    let to_z = (math::floor(z + width) - target_z * 16 + 1).min(16);

                    // Pass A: abort if the box touches lava. Only the shell of
                    // the box is sampled — the interior columns snap the scan to
                    // the floor, which is the vanilla optimisation.
                    let mut lava = false;
                    let mut bx = from_x;
                    while !lava && bx < to_x {
                        let mut bz = from_z;
                        while !lava && bz < to_z {
                            let mut by = to_y + 1;
                            while !lava && by >= from_y - 1 {
                                if (0..128).contains(&by) {
                                    let state =
                                        column.get_block(bx as usize, by as usize, bz as usize);
                                    if state.id() == ids::FLOWING_LAVA || state.id() == ids::LAVA {
                                        lava = true;
                                    }
                                    if by != from_y - 1
                                        && bx != from_x
                                        && bx != to_x - 1
                                        && bz != from_z
                                        && bz != to_z - 1
                                    {
                                        by = from_y;
                                    }
                                }
                                by -= 1;
                            }
                            bz += 1;
                        }
                        bx += 1;
                    }
                    if !lava {
                        // Pass B: carve the ellipsoid out of netherrack.
                        for carve_x in from_x..to_x {
                            let nx = (f64::from(carve_x + target_x * 16) + 0.5 - x) / width;
                            for carve_z in from_z..to_z {
                                let nz = (f64::from(carve_z + target_z * 16) + 0.5 - z) / width;
                                let mut carve_y = to_y;
                                while carve_y > from_y {
                                    let ny = (f64::from(carve_y - 1) + 0.5 - y) / height;
                                    if ny > -0.7 && nx * nx + ny * ny + nz * nz < 1.0 {
                                        let state = column.get_block(
                                            carve_x as usize,
                                            carve_y as usize,
                                            carve_z as usize,
                                        );
                                        if matches!(
                                            state.id(),
                                            ids::NETHERRACK | ids::DIRT | ids::GRASS
                                        ) {
                                            column.set_block(
                                                carve_x as usize,
                                                carve_y as usize,
                                                carve_z as usize,
                                                BlockState::AIR,
                                            );
                                        }
                                    }
                                    carve_y -= 1;
                                }
                            }
                        }

                        if room {
                            break;
                        }
                    }
                }
            }

            step += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_netherrack(x: i32, z: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(x, z);
        for y in 0..128usize {
            for lz in 0..16usize {
                for lx in 0..16usize {
                    column.set_block(lx, y, lz, BlockState::NETHERRACK);
                }
            }
        }
        column
    }

    #[test]
    fn carving_is_deterministic() {
        let caves = HellCaves::new(12345);
        let mut first = solid_netherrack(0, 0);
        let mut second = solid_netherrack(0, 0);
        caves.carve(&mut first);
        caves.carve(&mut second);
        for y in 0..128 {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(
                        first.get_block(x, y, z),
                        second.get_block(x, y, z),
                        "{x},{y},{z}"
                    );
                }
            }
        }
    }

    #[test]
    fn hell_caves_open_up_the_netherrack() {
        let caves = HellCaves::new(12345);
        let mut carved = 0usize;
        for chunk in -4..4 {
            let mut column = solid_netherrack(chunk, 0);
            caves.carve(&mut column);
            for y in 1..128usize {
                for lz in 0..16usize {
                    for lx in 0..16usize {
                        if column.get_block(lx, y, lz).is_air() {
                            carved += 1;
                        }
                    }
                }
            }
        }
        // A few percent of the volume is cave, as in the overworld; a zero
        // would mean the carver never ran, a huge number that it ate the world.
        assert!(carved > 1000, "only {carved} blocks carved");
        assert!(carved < 8 * 16 * 16 * 127 / 2, "carved {carved} blocks");
    }

    #[test]
    fn hell_caves_never_break_the_floor_or_the_ceiling() {
        let caves = HellCaves::new(99);
        for chunk in -2..2 {
            let mut column = solid_netherrack(chunk, chunk + 1);
            caves.carve(&mut column);
            for lz in 0..16usize {
                for lx in 0..16usize {
                    assert!(!column.get_block(lx, 0, lz).is_air(), "y=0 opened");
                    assert!(!column.get_block(lx, 127, lz).is_air(), "y=127 opened");
                }
            }
        }
    }
}
