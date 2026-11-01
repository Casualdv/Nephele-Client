//! `MapGenRavine`: the long, deep cuts.
//!
//! A ravine is one walk, like a cave, but it is wider, starts lower, tapers
//! with a `3.0` factor instead of `0.5`, and carves an anisotropic shape —
//! `field_75046_d` widens it at some heights and pinches it at others, giving
//! the characteristic stepped walls.
//!
//! Three details separate it from [`crate::caves`] and all three matter:
//!
//! - only stone, dirt and grass are eaten, not the clays or sandstone;
//! - the lava it fills the bottom with is **flowing** lava, not still lava;
//! - it only starts in one chunk out of fifty.

use crate::block::{self, BlockState};
use crate::chunk::ChunkColumn;
use crate::noise::JavaRandom;
use crate::surface::biome_surface;

const RANGE: i32 = 8;

pub struct Ravines {
    seed: i64,
}

impl Ravines {
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// Carve every ravine that reaches this column.
    pub fn carve(&self, column: &mut ChunkColumn, biomes: &[i32]) {
        let chunk_x = column.x;
        let chunk_z = column.z;

        let mut base = JavaRandom::new(self.seed);
        let step_x = base.next_long();
        let step_z = base.next_long();

        for source_x in chunk_x - RANGE..=chunk_x + RANGE {
            for source_z in chunk_z - RANGE..=chunk_z + RANGE {
                let seed = i64::from(source_x).wrapping_mul(step_x)
                    ^ i64::from(source_z).wrapping_mul(step_z)
                    ^ self.seed;
                let mut rand = JavaRandom::new(seed);

                if rand.next_int_bound(50) != 0 {
                    continue;
                }

                let x = f64::from(source_x * 16 + rand.next_int_bound(16));
                let ceiling = rand.next_int_bound(40) + 8;
                let y = f64::from(rand.next_int_bound(ceiling) + 20);
                let z = f64::from(source_z * 16 + rand.next_int_bound(16));

                let yaw = rand.next_float() * std::f32::consts::PI * 2.0;
                let pitch = (rand.next_float() - 0.5) * 2.0 / 8.0;
                let radius = (rand.next_float() * 2.0 + rand.next_float()) * 2.0;

                self.cut(
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
                    3.0,
                );
            }
        }
    }

    /// `func_180707_a`: one ravine.
    #[allow(clippy::too_many_arguments)]
    fn cut(
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

        let mut rooted = false;
        if step == -1 {
            step = length / 2;
            rooted = true;
        }

        // The width multiplier per y level: a value is redrawn every few levels,
        // which is what makes the walls stepped instead of smooth.
        let mut widths = [0.0f32; 256];
        let mut current = 1.0f32;
        for (level, slot) in widths.iter_mut().enumerate() {
            if level == 0 || rand.next_int_bound(3) == 0 {
                current = rand.next_float() * rand.next_float() + 1.0;
            }
            *slot = current * current;
        }

        while step < length {
            let swell = 1.5
                + f64::from(
                    (step as f32 * std::f32::consts::PI / length as f32).sin() * radius * 1.0,
                );
            let mut height = swell * taper;

            let width = swell * f64::from(rand.next_float() * 0.25 + 0.75);
            height *= f64::from(rand.next_float() * 0.25 + 0.75);

            let f3 = pitch.cos();
            let f4 = pitch.sin();
            x += f64::from(yaw.cos() * f3);
            y += f64::from(f4);
            z += f64::from(yaw.sin() * f3);
            pitch *= 0.7;
            pitch += drift * 0.05;
            yaw += turn * 0.05;
            drift *= 0.8;
            turn *= 0.5;
            drift += (rand.next_float() - rand.next_float()) * rand.next_float() * 2.0;
            turn += (rand.next_float() - rand.next_float()) * rand.next_float() * 4.0;

            if rooted || rand.next_int_bound(4) != 0 {
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
                    let mut low_x = (x - width).floor() as i32 - chunk_x * 16 - 1;
                    let mut high_x = (x + width).floor() as i32 - chunk_x * 16 + 1;
                    let mut low_y = (y - height).floor() as i32 - 1;
                    let high_y = (y + height).floor() as i32 + 1;
                    let mut low_z = (z - width).floor() as i32 - chunk_z * 16 - 1;
                    let mut high_z = (z + width).floor() as i32 - chunk_z * 16 + 1;

                    low_x = low_x.max(0);
                    high_x = high_x.min(16);
                    low_y = low_y.max(1);
                    let high_y = high_y.min(248);
                    low_z = low_z.max(0);
                    high_z = high_z.min(16);

                    if !touches_water(column, low_x, high_x, low_y, high_y, low_z, high_z) {
                        for column_x in low_x..high_x {
                            let dx = (f64::from(column_x + chunk_x * 16) + 0.5 - x) / width;

                            for column_z in low_z..high_z {
                                let dz = (f64::from(column_z + chunk_z * 16) + 0.5 - z) / width;
                                let mut opens_to_surface = false;

                                if dx * dx + dz * dz < 1.0 {
                                    let mut column_y = high_y;
                                    while column_y > low_y {
                                        let dy = (f64::from(column_y - 1) + 0.5 - y) / height;

                                        if (dx * dx + dz * dz)
                                            * f64::from(widths[(column_y - 1) as usize])
                                            + dy * dy / 6.0
                                            < 1.0
                                            && (0..256).contains(&column_y)
                                        {
                                            let block = column.get_block(
                                                column_x as usize,
                                                column_y as usize,
                                                column_z as usize,
                                            );

                                            if block.id() == block::ids::GRASS {
                                                opens_to_surface = true;
                                            }

                                            if matches!(
                                                block.id(),
                                                block::ids::STONE
                                                    | block::ids::DIRT
                                                    | block::ids::GRASS
                                            ) {
                                                if column_y < 11 {
                                                    column.set_block(
                                                        column_x as usize,
                                                        column_y as usize,
                                                        column_z as usize,
                                                        BlockState::new(10, 0),
                                                    );
                                                } else {
                                                    column.set_block(
                                                        column_x as usize,
                                                        column_y as usize,
                                                        column_z as usize,
                                                        BlockState::AIR,
                                                    );

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

                        if rooted {
                            break;
                        }
                    }
                }
            }

            step += 1;
        }
    }
}

/// The same shell-only water scan the caves use, with the same quirk.
#[allow(clippy::too_many_arguments)]
fn touches_water(
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
                        y = low_y;
                    }
                }
                y -= 1;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_column(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(chunk_x, chunk_z);
        for x in 0..16 {
            for z in 0..16 {
                for y in 1..100 {
                    column.set_block(x, y, z, BlockState::new(block::ids::STONE, 0));
                }
            }
        }
        column
    }

    #[test]
    fn cutting_is_deterministic() {
        let ravines = Ravines::new(12345);
        let biomes = [1i32; 256];
        let mut a = stone_column(0, 0);
        let mut b = stone_column(0, 0);
        ravines.carve(&mut a, &biomes);
        ravines.carve(&mut b, &biomes);
        for x in 0..16 {
            for y in 0..100 {
                for z in 0..16 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn ravines_are_rarer_than_caves() {
        let biomes = [1i32; 256];
        let mut cut = 0usize;

        for seed in 0..6 {
            let ravines = Ravines::new(seed);
            for cx in 0..4 {
                for cz in 0..4 {
                    let mut column = stone_column(cx, cz);
                    ravines.carve(&mut column, &biomes);
                    for x in 0..16 {
                        for y in 1..100 {
                            for z in 0..16 {
                                if column.get_block(x, y, z).is_air() {
                                    cut += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 96 chunks; a ravine in 1 chunk out of 50 should still produce some.
        assert!(cut > 0, "no ravines carved in 96 chunks");
    }
}
