//! `BiomeGenMesa`: layered hardened clay bands instead of ordinary soil.
//!
//! Mesa does not use the generic surface routine at all. It builds a 64-entry
//! table of coloured clay, one entry per y level, and fills each column from
//! that table, with red sand capping the top and orange clay just under it.
//!
//! Two details are vanilla quirks rather than design, and both are reproduced:
//! the band offset noise is sampled with the **x** coordinate on both axes, and
//! the table is regenerated from the world seed rather than from the chunk.

use crate::block::BlockState;
use crate::chunk::ChunkColumn;
use crate::noise::{JavaRandom, NoisePerlin};
use crate::surface::SEA_LEVEL;

const HARDENED_CLAY: u16 = 172;
const STAINED_CLAY: u16 = 159;
const RED_SAND: u16 = 12;
const STONE: u16 = 1;
const BEDROCK: u16 = 7;
const WATER: u16 = 9;
const GRASS: u16 = 2;
const DIRT: u16 = 3;

/// `EnumDyeColor` metadata values.
const WHITE: u8 = 0;
const ORANGE: u8 = 1;
const YELLOW: u8 = 4;
const SILVER: u8 = 8;
const BROWN: u8 = 12;
const RED: u8 = 14;

fn stained(colour: u8) -> BlockState {
    BlockState::new(STAINED_CLAY, colour)
}

fn orange_clay() -> BlockState {
    stained(ORANGE)
}

/// The coloured clay table plus the three noise fields mesa consults.
pub struct MesaTerrain {
    bands: [BlockState; 64],
    /// `field_150625_aG`, which shifts the bands up and down.
    band_noise: NoisePerlin,
    /// `field_150623_aE`, four octaves, for the Bryce pillars.
    hills_noise: NoisePerlin,
    /// `field_150624_aF`, one octave, for the Bryce pillar heights.
    stripes_noise: NoisePerlin,
}

impl MesaTerrain {
    pub fn new(seed: i64) -> Self {
        // `func_150619_a`: one Random builds the band noise first, then the
        // table itself, so the two are not independent.
        let mut random = JavaRandom::new(seed);
        let band_noise = NoisePerlin::new(&mut random, 1);

        let mut bands = [BlockState::new(HARDENED_CLAY, 0); 64];

        let mut level = 0i32;
        while level < 64 {
            level += random.next_int_bound(5) + 1;
            if level < 64 {
                bands[level as usize] = orange_clay();
            }
        }

        let orange_runs = random.next_int_bound(4) + 2;
        for _ in 0..orange_runs {
            let length = random.next_int_bound(3) + 1;
            let start = random.next_int_bound(64);
            for offset in 0..length {
                if start + offset < 64 {
                    bands[(start + offset) as usize] = stained(YELLOW);
                }
            }
        }

        let brown_runs = random.next_int_bound(4) + 2;
        for _ in 0..brown_runs {
            let length = random.next_int_bound(3) + 2;
            let start = random.next_int_bound(64);
            for offset in 0..length {
                if start + offset < 64 {
                    bands[(start + offset) as usize] = stained(BROWN);
                }
            }
        }

        let red_runs = random.next_int_bound(4) + 2;
        for _ in 0..red_runs {
            let length = random.next_int_bound(3) + 1;
            let start = random.next_int_bound(64);
            for offset in 0..length {
                if start + offset < 64 {
                    bands[(start + offset) as usize] = stained(RED);
                }
            }
        }

        // Thin white/silver seams, each with a chance of a bleached neighbour.
        let seams = random.next_int_bound(3) + 3;
        let mut cursor = 0i32;
        for _ in 0..seams {
            cursor += random.next_int_bound(16) + 4;
            if cursor < 64 {
                bands[cursor as usize] = stained(WHITE);
                if cursor > 1 && random.next_boolean() {
                    bands[(cursor - 1) as usize] = stained(SILVER);
                }
                if cursor < 63 && random.next_boolean() {
                    bands[(cursor + 1) as usize] = stained(SILVER);
                }
            }
        }

        // The two Bryce fields come from their own Random, as in vanilla.
        let mut shape = JavaRandom::new(seed);
        let hills_noise = NoisePerlin::new(&mut shape, 4);
        let stripes_noise = NoisePerlin::new(&mut shape, 1);

        Self {
            bands,
            band_noise,
            hills_noise,
            stripes_noise,
        }
    }

    /// `func_180629_a`: the band for this height, shifted by the noise.
    ///
    /// Vanilla samples the noise with `x` on both axes; that is a bug in 1.8.9
    /// and is kept so the bands line up with the real world.
    fn band(&self, x: i32, y: i32) -> BlockState {
        let scale = f64::from(x) / 512.0;
        let offset = (self.band_noise.sample(scale, scale) * 2.0).round() as i32;
        self.bands[((y + offset + 64) % 64) as usize]
    }

    /// `BiomeGenMesa.genTerrainBlocks`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate(
        &self,
        column: &mut ChunkColumn,
        x: usize,
        z: usize,
        world_x: i32,
        world_z: i32,
        noise_value: f64,
        bryce: bool,
        plateau: bool,
        rand: &mut JavaRandom,
    ) {
        // Bryce (the mutated mesa) raises pillars out of the noise field.
        let mut pillar_top = 0.0f64;
        if bryce {
            let i = (world_x & -16) + (world_z & 15);
            let j = (world_z & -16) + (world_x & 15);
            let strength = noise_value.abs().min(
                self.hills_noise
                    .sample(f64::from(i) * 0.25, f64::from(j) * 0.25),
            );
            if strength > 0.0 {
                let fine = (self
                    .stripes_noise
                    .sample(f64::from(i) * 0.001_953_125, f64::from(j) * 0.001_953_125))
                .abs();
                let mut height = strength * strength * 2.5;
                let limit = (fine * 50.0).ceil() + 14.0;
                if height > limit {
                    height = limit;
                }
                pillar_top = height + 64.0;
            }
        }

        // The biome's own pair: red sand over stained clay.
        let mut top_block: Option<BlockState> = Some(BlockState::new(RED_SAND, 1));
        let mut filler_block = stained(WHITE);
        let thickness = (noise_value / 3.0 + 3.0 + rand.next_double() * 0.25) as i32;
        let banded = f64::cos(noise_value / 3.0 * std::f64::consts::PI) > 0.0;
        let mut depth = -1i32;
        let mut painted_top = false;

        for y in (0..=255i32).rev() {
            let state = column.get_block(x, y as usize, z);

            if state.is_air() && f64::from(y) < pillar_top {
                column.set_block(x, y as usize, z, BlockState::new(STONE, 0));
            }

            if y <= rand.next_int_bound(5) {
                column.set_block(x, y as usize, z, BlockState::new(BEDROCK, 0));
                continue;
            }

            let state = column.get_block(x, y as usize, z);
            if state.is_air() {
                depth = -1;
            } else if state.id() == STONE {
                if depth == -1 {
                    painted_top = false;

                    if thickness <= 0 {
                        top_block = None;
                        filler_block = BlockState::new(STONE, 0);
                    } else if (SEA_LEVEL - 4..=SEA_LEVEL + 1).contains(&y) {
                        top_block = Some(stained(WHITE));
                        filler_block = stained(WHITE);
                    }

                    if y < SEA_LEVEL && top_block.is_none_or(|block| block.is_air()) {
                        top_block = Some(BlockState::new(WATER, 0));
                    }

                    depth = thickness + (y - SEA_LEVEL).max(0);

                    if y < SEA_LEVEL - 1 {
                        column.set_block(x, y as usize, z, filler_block);
                        if filler_block.id() == STAINED_CLAY {
                            column.set_block(x, y as usize, z, orange_clay());
                        }
                    } else if plateau && y > 86 + thickness * 2 {
                        let surface = if banded {
                            BlockState::new(DIRT, 1)
                        } else {
                            BlockState::new(GRASS, 0)
                        };
                        column.set_block(x, y as usize, z, surface);
                    } else if y <= SEA_LEVEL + 3 + thickness {
                        if let Some(block) = top_block {
                            column.set_block(x, y as usize, z, block);
                        }
                        painted_top = true;
                    } else {
                        let block = if (64..=127).contains(&y) {
                            if banded {
                                BlockState::new(HARDENED_CLAY, 0)
                            } else {
                                self.band(world_x, y)
                            }
                        } else {
                            orange_clay()
                        };
                        column.set_block(x, y as usize, z, block);
                    }
                } else if depth > 0 {
                    depth -= 1;
                    let block = if painted_top {
                        orange_clay()
                    } else {
                        self.band(world_x, y)
                    };
                    column.set_block(x, y as usize, z, block);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_table_is_deterministic_for_a_seed() {
        let a = MesaTerrain::new(12345);
        let b = MesaTerrain::new(12345);
        for level in 0..64 {
            assert_eq!(a.bands[level], b.bands[level], "level {level}");
        }
    }

    #[test]
    fn the_band_table_contains_coloured_clay() {
        let mesa = MesaTerrain::new(12345);
        let coloured = mesa
            .bands
            .iter()
            .filter(|band| band.id() == STAINED_CLAY)
            .count();
        assert!(coloured > 8, "only {coloured} coloured bands");
    }

    #[test]
    fn different_seeds_make_different_bands() {
        let a = MesaTerrain::new(1);
        let b = MesaTerrain::new(2);
        assert!(a.bands.iter().zip(&b.bands).any(|(x, y)| x != y));
    }

    #[test]
    fn a_mesa_column_is_red_sand_over_coloured_clay() {
        let mesa = MesaTerrain::new(12345);
        let mut column = ChunkColumn::empty(0, 0);
        for y in 0..=80 {
            column.set_block(4, y, 4, BlockState::new(STONE, 0));
        }
        let mut rand = JavaRandom::new(99);
        mesa.generate(&mut column, 4, 4, 4, 4, 0.0, false, false, &mut rand);

        let mut saw_band = false;
        for y in 0..=80usize {
            let state = column.get_block(4, y, 4);
            if state.id() == STAINED_CLAY || state.id() == HARDENED_CLAY {
                saw_band = true;
            }
        }
        assert!(saw_band, "expected hardened clay somewhere in the column");
        assert_eq!(column.get_block(4, 0, 4).id(), BEDROCK);
    }
}
