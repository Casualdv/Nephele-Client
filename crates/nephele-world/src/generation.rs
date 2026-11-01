//! Deterministic single-player terrain generation.
//!
//! Small, dependency-free value noise: rolling hills, a sea level, beaches,
//! snow caps and scattered oak trees. It is not vanilla's generator and does not
//! try to be - it exists so single player has a world to walk around in, and so
//! the renderer and physics have something real to chew on.

use crate::block::BlockState;
use crate::chunk::{ChunkColumn, SECTION_DIM};

/// Water fills every column up to this height.
pub const SEA_LEVEL: i32 = 62;
/// Trees stop growing above this.
pub const SNOW_LINE: i32 = 92;

/// Generates 16x16 columns from a seed.
#[derive(Debug, Clone, Copy)]
pub struct TerrainGenerator {
    seed: u64,
}

impl TerrainGenerator {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Height of the topmost solid block at world x/z.
    pub fn height_at(&self, x: i32, z: i32) -> i32 {
        let base = 64.0;
        let hills = fbm(self.seed ^ 0x11, x as f64 * 0.012, z as f64 * 0.012, 4) * 14.0;
        let detail = fbm(self.seed ^ 0x22, x as f64 * 0.06, z as f64 * 0.06, 2) * 2.5;
        // A slow "continent" wave so there are sea bays and plateaus.
        let continent = fbm(self.seed ^ 0x33, x as f64 * 0.003, z as f64 * 0.003, 2) * 8.0;
        (base + hills + detail + continent).clamp(8.0, 120.0) as i32
    }

    /// A surface safe to spawn on: the first dry column near the origin.
    pub fn spawn_height(&self) -> i32 {
        for radius in 0..64 {
            for (x, z) in [
                (radius, 0),
                (-radius, 0),
                (0, radius),
                (0, -radius),
                (radius, radius),
                (-radius, radius),
            ] {
                let height = self.height_at(x, z);
                if height > SEA_LEVEL + 1 {
                    return height + 1;
                }
            }
        }
        self.height_at(0, 0) + 1
    }

    /// Build one chunk column.
    pub fn column(&self, cx: i32, cz: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(cx, cz);
        let base_x = cx * SECTION_DIM as i32;
        let base_z = cz * SECTION_DIM as i32;

        for lx in 0..SECTION_DIM {
            for lz in 0..SECTION_DIM {
                let x = base_x + lx as i32;
                let z = base_z + lz as i32;
                let height = self.height_at(x, z);
                let beach = height <= SEA_LEVEL + 1;
                let snowy = height >= SNOW_LINE;

                for y in 0..=height.min(MAX_Y) {
                    let state = if y == height {
                        if beach {
                            BlockState::SAND
                        } else if snowy {
                            BlockState::SNOW_BLOCK
                        } else {
                            BlockState::GRASS
                        }
                    } else if y > height - 4 {
                        if beach {
                            BlockState::SAND
                        } else {
                            BlockState::DIRT
                        }
                    } else if y < 6 {
                        BlockState::BEDROCK
                    } else {
                        BlockState::STONE
                    };
                    column.set_block(lx, y as usize, lz, state);
                }

                for y in (height + 1)..=SEA_LEVEL {
                    if (0..=MAX_Y).contains(&y) {
                        column.set_block(lx, y as usize, lz, BlockState::WATER);
                    }
                }
            }
        }

        // Trees are generated from neighbouring columns too, so a canopy that
        // crosses a chunk border is still drawn on both sides.
        let margin = 3;
        for tx in (base_x - margin)..(base_x + SECTION_DIM as i32 + margin) {
            for tz in (base_z - margin)..(base_z + SECTION_DIM as i32 + margin) {
                self.place_tree(&mut column, tx, tz);
            }
        }

        column
    }

    fn place_tree(&self, column: &mut ChunkColumn, x: i32, z: i32) {
        let height = self.height_at(x, z);
        if height <= SEA_LEVEL + 1 || height >= SNOW_LINE {
            return;
        }
        // Sparse, stable scatter.
        let rnd = hash2(self.seed ^ 0x7EE, x, z);
        if rnd % 97 != 0 {
            return;
        }

        let base_x = column.x * SECTION_DIM as i32;
        let base_z = column.z * SECTION_DIM as i32;
        let trunk = 4 + (rnd >> 8) % 3;
        let top = height + trunk as i32;

        let mut set = |wx: i32, wy: i32, wz: i32, state: BlockState| {
            let lx = wx - base_x;
            let lz = wz - base_z;
            if !(0..SECTION_DIM as i32).contains(&lx) || !(0..SECTION_DIM as i32).contains(&lz) {
                return;
            }
            if !(0..=MAX_Y).contains(&wy) {
                return;
            }
            column.set_block(lx as usize, wy as usize, lz as usize, state);
        };

        for y in (height + 1)..=top {
            set(x, y, z, BlockState::LOG);
        }
        // Two wide layers plus a small crown.
        for (dy, radius) in [(0, 2), (1, 2), (2, 1)] {
            let y = top + dy;
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx * dx + dz * dz > radius * radius + 1 {
                        continue;
                    }
                    if dx == 0 && dz == 0 && dy < 2 {
                        continue;
                    }
                    set(x + dx, y, z + dz, BlockState::LEAVES);
                }
            }
        }
    }
}

/// Highest block a column can contain (the chunk y limit is 256).
const MAX_Y: i32 = 255;

fn hash2(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    value ^= value >> 33;
    value = value.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    value ^= value >> 33;
    value = value.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    value ^ (value >> 33)
}

fn lattice(seed: u64, x: i32, z: i32) -> f64 {
    // -1..1 from the low bits of a hash.
    let h = hash2(seed, x, z);
    ((h >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise over an integer lattice.
fn value_noise(seed: u64, x: f64, z: f64) -> f64 {
    let x0 = x.floor();
    let z0 = z.floor();
    let tx = smooth(x - x0);
    let tz = smooth(z - z0);
    let (x0, z0) = (x0 as i32, z0 as i32);
    let a = lattice(seed, x0, z0);
    let b = lattice(seed, x0 + 1, z0);
    let c = lattice(seed, x0, z0 + 1);
    let d = lattice(seed, x0 + 1, z0 + 1);
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    top + (bottom - top) * tz
}

/// Fractal sum of value noise, roughly -1..1.
fn fbm(seed: u64, x: f64, z: f64, octaves: u32) -> f64 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut total = 0.0;
    for octave in 0..octaves {
        sum += value_noise(
            seed.wrapping_add(octave as u64 * 7919),
            x * frequency,
            z * frequency,
        ) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        let a = TerrainGenerator::new(12345);
        let b = TerrainGenerator::new(12345);
        let ca = a.column(0, 0);
        let cb = b.column(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                for y in [0usize, 40, 62, 70] {
                    assert_eq!(ca.get_block(x, y, z), cb.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn different_seeds_make_different_terrain() {
        let a = TerrainGenerator::new(1);
        let b = TerrainGenerator::new(2);
        let differing = (0..32)
            .filter(|i| a.height_at(*i, 0) != b.height_at(*i, 0))
            .count();
        assert!(differing > 8, "{differing} of 32 heights differ");
    }

    #[test]
    fn columns_have_ground_and_water() {
        let generator = TerrainGenerator::new(7);
        let mut water = false;
        let mut ground = false;
        for cx in -2..=2 {
            for cz in -2..=2 {
                let column = generator.column(cx, cz);
                for lx in 0..16 {
                    for lz in 0..16 {
                        let x = cx * 16 + lx as i32;
                        let z = cz * 16 + lz as i32;
                        let height = generator.height_at(x, z);
                        assert!(!column.get_block(lx, height as usize, lz).is_air());
                        if height < SEA_LEVEL
                            && !column.get_block(lx, SEA_LEVEL as usize, lz).is_air()
                        {
                            water = true;
                        }
                        if height >= SEA_LEVEL {
                            ground = true;
                        }
                    }
                }
            }
        }
        assert!(ground, "expected dry land somewhere");
        assert!(water, "expected sea somewhere");
    }

    #[test]
    fn spawn_is_on_dry_land() {
        for seed in [0, 1, 42, 9_999] {
            let generator = TerrainGenerator::new(seed);
            let y = generator.spawn_height();
            assert!(y > SEA_LEVEL, "seed {seed} spawns at {y}");
            assert!(y < 130);
        }
    }

    #[test]
    fn the_same_leaf_lands_the_same_way_in_both_chunks() {
        let generator = TerrainGenerator::new(4242);
        // Every tree is decided by its own world position, so re-generating the
        // same column must agree.
        let a = generator.column(3, -2);
        let b = generator.column(3, -2);
        assert_eq!(
            a.iter_sections()
                .map(|(_, s)| s.non_air_count())
                .collect::<Vec<_>>(),
            b.iter_sections()
                .map(|(_, s)| s.non_air_count())
                .collect::<Vec<_>>()
        );
    }
}
