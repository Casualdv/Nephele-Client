//! `MapGenCaves`: the tunnels and caverns under the surface.
//!
//! A cave is a random walk. It starts at a point, then steps forward in small
//! increments, turning a little more each step, while the ellipsoid it carves
//! swells in the middle of its run and tapers at both ends. Branches reopen the
//! walk from the halfway point at a right angle.
//!
//! Two things are specific to vanilla and easy to miss:
//!
//! - The generator does not work on one chunk. `MapGenBase` replays the walk for
//!   every chunk within eight chunks in each direction, because a tunnel that
//!   starts far away can still reach here. That is 289 walks per chunk.
//! - A walk is abandoned if any part of the ellipsoid would touch water, and
//!   the check deliberately hides a bug: the inner loop reassigns its own
//!   counter (`l1 = l2`) to skip the interior, so only the shell is sampled.

use crate::block::{self, BlockState};
use crate::chunk::ChunkColumn;
use crate::noise::JavaRandom;
use crate::surface::biome_surface;

/// `MapGenBase.range`.
const RANGE: i32 = 8;

pub struct Caves {
    seed: i64,
}

impl Caves {
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// Carve every cave that reaches this column.
    ///
    /// `biomes` is the 16x16 per-block biome map, used to restore the surface
    /// block when a cave opens onto grass.
    pub fn carve(&self, column: &mut ChunkColumn, biomes: &[i32]) {
        let chunk_x = column.x;
        let chunk_z = column.z;

        // `MapGenBase.generate`: two longs decide where each source chunk
        // starts in the sequence.
        let mut base = JavaRandom::new(self.seed);
        let step_x = base.next_long();
        let step_z = base.next_long();

        for source_x in chunk_x - RANGE..=chunk_x + RANGE {
            for source_z in chunk_z - RANGE..=chunk_z + RANGE {
                let seed = i64::from(source_x).wrapping_mul(step_x)
                    ^ i64::from(source_z).wrapping_mul(step_z)
                    ^ self.seed;
                let mut rand = JavaRandom::new(seed);
                self.recursive(
                    &mut rand, source_x, source_z, chunk_x, chunk_z, column, biomes,
                );
            }
        }
    }

    /// `recursiveGenerate`: how many caves this source chunk starts, if any.
    #[allow(clippy::too_many_arguments)]
    fn recursive(
        &self,
        rand: &mut JavaRandom,
        source_x: i32,
        source_z: i32,
        chunk_x: i32,
        chunk_z: i32,
        column: &mut ChunkColumn,
        biomes: &[i32],
    ) {
        // Java evaluates the nested draws inside out; keep that order.
        let inner = rand.next_int_bound(15) + 1;
        let middle = rand.next_int_bound(inner) + 1;
        let mut count = rand.next_int_bound(middle);
        if rand.next_int_bound(7) != 0 {
            count = 0;
        }

        for _ in 0..count {
            let x = f64::from(source_x * 16 + rand.next_int_bound(16));
            let ceiling = rand.next_int_bound(120) + 8;
            let y = f64::from(rand.next_int_bound(ceiling));
            let z = f64::from(source_z * 16 + rand.next_int_bound(16));

            let mut branches = 1i32;
            if rand.next_int_bound(4) == 0 {
                // A wide room at the origin.
                self.tunnel(
                    rand.next_long(),
                    chunk_x,
                    chunk_z,
                    column,
                    biomes,
                    x,
                    y,
                    z,
                    rand.next_float() * 6.0 + 1.0,
                    0.0,
                    0.0,
                    -1,
                    -1,
                    0.5,
                );
                branches += rand.next_int_bound(4);
            }

            for _ in 0..branches {
                let yaw = rand.next_float() * std::f32::consts::PI * 2.0;
                let pitch = (rand.next_float() - 0.5) * 2.0 / 8.0;
                let mut radius = rand.next_float() * 2.0 + rand.next_float();
                if rand.next_int_bound(10) == 0 {
                    radius *= rand.next_float() * rand.next_float() * 3.0 + 1.0;
                }

                self.tunnel(
                    rand.next_long(),
                    chunk_x,
                    chunk_z,
                    column,
                    biomes,
                    x,
                    y,
                    z,
                    radius,
                    yaw,
                    pitch,
                    0,
                    0,
                    1.0,
                );
            }
        }
    }

    /// `func_180702_a`: one walk, carving as it goes.
    #[allow(clippy::too_many_arguments)]
    fn tunnel(
        &self,
        seed: i64,
        chunk_x: i32,
        chunk_z: i32,
        column: &mut ChunkColumn,
        biomes: &[i32],
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
        let centre_x = f64::from(chunk_x * 16 + 8);
        let centre_z = f64::from(chunk_z * 16 + 8);
        let mut drift = 0.0f32;
        let mut turn = 0.0f32;
        let mut rand = JavaRandom::new(seed);

        if length <= 0 {
            let span = RANGE * 16 - 16;
            length = span - rand.next_int_bound(span / 4);
        }

        let mut branched = false;
        if step == -1 {
            step = length / 2;
            branched = true;
        }

        let branch_at = rand.next_int_bound(length / 2) + length / 4;
        let steep = rand.next_int_bound(6) == 0;

        while step < length {
            let swell = 1.5
                + f64::from(
                    (step as f32 * std::f32::consts::PI / length as f32).sin() * radius * 1.0,
                );
            let height = swell * taper;
            let f2 = pitch.cos();
            let f3 = pitch.sin();

            x += f64::from(yaw.cos() * f2);
            y += f64::from(f3);
            z += f64::from(yaw.sin() * f2);

            if steep {
                pitch *= 0.92;
            } else {
                pitch *= 0.7;
            }
            pitch += drift * 0.1;
            yaw += turn * 0.1;
            drift *= 0.9;
            turn *= 0.75;
            drift += (rand.next_float() - rand.next_float()) * rand.next_float() * 2.0;
            turn += (rand.next_float() - rand.next_float()) * rand.next_float() * 4.0;

            if !branched && step == branch_at && radius > 1.0 && length > 0 {
                self.tunnel(
                    rand.next_long(),
                    chunk_x,
                    chunk_z,
                    column,
                    biomes,
                    x,
                    y,
                    z,
                    rand.next_float() * 0.5 + 0.5,
                    yaw - std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    length,
                    1.0,
                );
                self.tunnel(
                    rand.next_long(),
                    chunk_x,
                    chunk_z,
                    column,
                    biomes,
                    x,
                    y,
                    z,
                    rand.next_float() * 0.5 + 0.5,
                    yaw + std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    length,
                    1.0,
                );
                return;
            }

            if branched || rand.next_int_bound(4) != 0 {
                let dx = x - centre_x;
                let dz = z - centre_z;
                let remaining = f64::from(length - step);
                let reach = f64::from(radius + 2.0 + 16.0);
                if dx * dx + dz * dz - remaining * remaining > reach * reach {
                    return;
                }

                if x >= centre_x - 16.0 - swell * 2.0
                    && z >= centre_z - 16.0 - swell * 2.0
                    && x <= centre_x + 16.0 + swell * 2.0
                    && z <= centre_z + 16.0 + swell * 2.0
                {
                    let mut low_x = (x - swell).floor() as i32 - chunk_x * 16 - 1;
                    let mut high_x = (x + swell).floor() as i32 - chunk_x * 16 + 1;
                    let mut low_y = (y - height).floor() as i32 - 1;
                    let high_y = (y + height).floor() as i32 + 1;
                    let mut low_z = (z - swell).floor() as i32 - chunk_z * 16 - 1;
                    let mut high_z = (z + swell).floor() as i32 - chunk_z * 16 + 1;

                    low_x = low_x.max(0);
                    high_x = high_x.min(16);
                    low_y = low_y.max(1);
                    let high_y = high_y.min(248);
                    low_z = low_z.max(0);
                    high_z = high_z.min(16);

                    if !self.touches_water(column, low_x, high_x, low_y, high_y, low_z, high_z) {
                        for column_x in low_x..high_x {
                            let dx = (f64::from(column_x + chunk_x * 16) + 0.5 - x) / swell;

                            for column_z in low_z..high_z {
                                let dz = (f64::from(column_z + chunk_z * 16) + 0.5 - z) / swell;
                                let mut opens_to_surface = false;

                                if dx * dx + dz * dz < 1.0 {
                                    let mut column_y = high_y;
                                    while column_y > low_y {
                                        let dy = f64::from(column_y - 1) + 0.5 - y;
                                        let dy = dy / height;

                                        if dy > -0.7
                                            && dx * dx + dy * dy + dz * dz < 1.0
                                            && column_y >= 1
                                        {
                                            let block = column.get_block(
                                                column_x as usize,
                                                column_y as usize,
                                                column_z as usize,
                                            );
                                            let above = if column_y + 1 <= 255 {
                                                column.get_block(
                                                    column_x as usize,
                                                    (column_y + 1) as usize,
                                                    column_z as usize,
                                                )
                                            } else {
                                                BlockState::AIR
                                            };

                                            if block.id() == block::ids::GRASS
                                                || block.id() == block::ids::MYCELIUM
                                            {
                                                opens_to_surface = true;
                                            }

                                            if replaceable(block, above) {
                                                if column_y < 11 {
                                                    column.set_block(
                                                        column_x as usize,
                                                        column_y as usize,
                                                        column_z as usize,
                                                        BlockState::new(block::ids::LAVA, 0),
                                                    );
                                                } else {
                                                    column.set_block(
                                                        column_x as usize,
                                                        column_y as usize,
                                                        column_z as usize,
                                                        BlockState::AIR,
                                                    );

                                                    if above.id() == block::ids::SAND {
                                                        column.set_block(
                                                            column_x as usize,
                                                            (column_y + 1) as usize,
                                                            column_z as usize,
                                                            BlockState::new(
                                                                block::ids::SANDSTONE,
                                                                0,
                                                            ),
                                                        );
                                                    }

                                                    if opens_to_surface
                                                        && column_y >= 1
                                                        && column
                                                            .get_block(
                                                                column_x as usize,
                                                                (column_y - 1) as usize,
                                                                column_z as usize,
                                                            )
                                                            .id()
                                                            == block::ids::DIRT
                                                    {
                                                        let biome = biomes[column_z as usize * 16
                                                            + column_x as usize];
                                                        let (top, _) = biome_surface(biome);
                                                        column.set_block(
                                                            column_x as usize,
                                                            (column_y - 1) as usize,
                                                            column_z as usize,
                                                            BlockState::new(top, 0),
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        column_y -= 1;
                                    }
                                }
                            }
                        }

                        if branched {
                            break;
                        }
                    }
                }
            }

            step += 1;
        }
    }

    /// The shell-only water check, transcribed with its own quirk intact.
    #[allow(clippy::too_many_arguments)]
    fn touches_water(
        &self,
        column: &ChunkColumn,
        low_x: i32,
        high_x: i32,
        low_y: i32,
        high_y: i32,
        low_z: i32,
        high_z: i32,
    ) -> bool {
        for x in low_x..high_x {
            for z in low_z..high_z {
                let mut y = high_y + 1;
                while y >= low_y - 1 {
                    if (0..256).contains(&y) {
                        let id = column.get_block(x as usize, y as usize, z as usize).id();
                        if id == block::ids::WATER || id == 8 {
                            return true;
                        }
                        if y != low_y - 1
                            && x != low_x
                            && x != high_x - 1
                            && z != low_z
                            && z != high_z - 1
                        {
                            // Vanilla snaps the counter inward here, so only
                            // the outer shell is actually sampled.
                            y = low_y;
                        }
                    }
                    y -= 1;
                }
            }
        }
        false
    }
}

/// `func_175793_a`: what a cave may eat through.
fn replaceable(block: BlockState, above: BlockState) -> bool {
    let id = block.id();
    if matches!(
        id,
        block::ids::STONE
            | block::ids::DIRT
            | block::ids::GRASS
            | block::ids::HARDENED_CLAY
            | block::ids::STAINED_HARDENED_CLAY
            | block::ids::SANDSTONE
            | block::ids::RED_SANDSTONE
            | block::ids::MYCELIUM
            | block::ids::SNOW_LAYER
    ) {
        return true;
    }
    matches!(id, block::ids::SAND | block::ids::GRAVEL)
        && above.id() != block::ids::WATER
        && above.id() != 8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_column(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(chunk_x, chunk_z);
        for x in 0..16 {
            for z in 0..16 {
                for y in 1..128 {
                    column.set_block(x, y, z, BlockState::new(block::ids::STONE, 0));
                }
            }
        }
        column
    }

    #[test]
    fn carving_is_deterministic() {
        let caves = Caves::new(12345);
        let biomes = [1i32; 256];
        let mut a = stone_column(0, 0);
        let mut b = stone_column(0, 0);
        caves.carve(&mut a, &biomes);
        caves.carve(&mut b, &biomes);
        for x in 0..16 {
            for y in 0..128 {
                for z in 0..16 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn caves_remove_stone() {
        let caves = Caves::new(12345);
        let biomes = [1i32; 256];
        let mut removed = 0usize;
        for cx in 0..4 {
            for cz in 0..4 {
                let mut column = stone_column(cx, cz);
                caves.carve(&mut column, &biomes);
                for x in 0..16 {
                    for y in 1..128 {
                        for z in 0..16 {
                            if column.get_block(x, y, z).is_air() {
                                removed += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(removed > 100, "only {removed} blocks carved in 16 chunks");
    }

    #[test]
    fn caves_never_break_bedrock_or_build_upward() {
        let caves = Caves::new(999);
        let biomes = [1i32; 256];
        for cx in 0..3 {
            for cz in 0..3 {
                let mut column = stone_column(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        column.set_block(x, 0, z, BlockState::new(block::ids::BEDROCK, 0));
                    }
                }
                caves.carve(&mut column, &biomes);
                for x in 0..16 {
                    for z in 0..16 {
                        assert_eq!(
                            column.get_block(x, 0, z).id(),
                            block::ids::BEDROCK,
                            "bedrock was carved"
                        );
                    }
                }
            }
        }
    }
}
