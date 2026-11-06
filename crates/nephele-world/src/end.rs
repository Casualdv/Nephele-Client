//! `ChunkProviderEnd` — the end island and its obsidian pillars.
//!
//! Reimplemented from the 1.8.9 class of the same name.
//!
//! - [`EndGenerator::provide_chunk`] is pure: a 3×3×33 density field (the end's
//!   "island" falloff is `100 - 8 * distance_from_origin`, clamped
//!   `-100..80`), converted to `end_stone` where it is positive. A second
//!   surface pass exists in vanilla (`func_180519_a`) but only rewrites
//!   `Blocks.stone`, while this chunk is built from `end_stone` — it is dead
//!   code and is reproduced as such.
//! - [`EndGenerator::populate`] places the obsidian pillars through
//!   `BiomeEndDecorator` → `WorldGenSpikes`: one attempt per chunk when
//!   `nextInt(5) == 0`, at the top of a column in the chunk.
//!
//! **Determinism deviation, deliberate.** Vanilla's `ChunkProviderEnd.populate`
//! decorates with `world.rand`, a `new Random()` that is never seeded, so
//! pillar positions are *not* reproducible in vanilla either (they differ every
//! server start). This port seeds one `Random` per owner chunk from the chunk
//! coordinates, which keeps `chunk generation is a pure function of (seed, x, z)`
//! true for the whole world. Pillar shape, height distribution and the 1-in-5
//! rate are vanilla; exact positions necessarily are not, and no fixture can
//! compare them.

use crate::block::{ids, BlockState};
use crate::chunk::ChunkColumn;
use crate::genworld::GenWorld;
use crate::math;
use crate::noise::{JavaRandom, NoiseOctaves};
use crate::world::ChunkCoord;

/// `BiomeGenBase.sky.biomeID`, the only biome the end has.
pub const END_BIOME_ID: i32 = 9;

/// The end's terrain and pillar decoration.
pub struct EndGenerator {
    seed: i64,
    rand: JavaRandom,
    noise_1: NoiseOctaves,
    noise_2: NoiseOctaves,
    noise_3: NoiseOctaves,
    noise_4: NoiseOctaves,
    noise_5: NoiseOctaves,
}

impl EndGenerator {
    pub fn new(seed: i64) -> Self {
        let mut rand = JavaRandom::new(seed);
        let noise_1 = NoiseOctaves::new(&mut rand, 16);
        let noise_2 = NoiseOctaves::new(&mut rand, 16);
        let noise_3 = NoiseOctaves::new(&mut rand, 8);
        let noise_4 = NoiseOctaves::new(&mut rand, 10);
        let noise_5 = NoiseOctaves::new(&mut rand, 16);
        Self {
            seed,
            rand,
            noise_1,
            noise_2,
            noise_3,
            noise_4,
            noise_5,
        }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    /// `ChunkProviderEnd.provideChunk`.
    pub fn provide_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        self.rand.set_seed(chunk_rand_seed(chunk_x, chunk_z));

        let mut column = ChunkColumn::empty(chunk_x, chunk_z);
        self.island(chunk_x, chunk_z, &mut column);
        // `func_180519_a` (the surface pass) only matches `Blocks.stone`, and
        // this primer is `end_stone`/air, so vanilla's pass is a no-op.
        column.set_biomes(&[END_BIOME_ID as u8; 256]);
        // `chunk.generateSkylightMap()`: the height map always, the sky-light
        // nibbles never — the end has no sky.
        column.generate_height_map();
        column
    }

    /// `func_180520_a`: the island density field, `2×2` columns of `8×8×4`
    /// blocks per cell.
    fn island(&mut self, chunk_x: i32, chunk_z: i32, column: &mut ChunkColumn) {
        let cells = 2usize;
        let x_size = cells + 1; // 3
        let y_size = 33usize;
        let z_size = cells + 1; // 3

        let densities = self.initialize_noise_field(
            chunk_x * cells as i32,
            0,
            chunk_z * cells as i32,
            x_size,
            y_size,
            z_size,
        );
        let at = |x: usize, z: usize, y: usize| densities[(x * z_size + z) * y_size + y];

        for cell_x in 0..cells {
            for cell_z in 0..cells {
                for cell_y in 0..32usize {
                    let d1 = at(cell_x, cell_z, cell_y);
                    let d2 = at(cell_x, cell_z + 1, cell_y);
                    let d3 = at(cell_x + 1, cell_z, cell_y);
                    let d4 = at(cell_x + 1, cell_z + 1, cell_y);
                    let d5 = (at(cell_x, cell_z, cell_y + 1) - d1) * 0.25;
                    let d6 = (at(cell_x, cell_z + 1, cell_y + 1) - d2) * 0.25;
                    let d7 = (at(cell_x + 1, cell_z, cell_y + 1) - d3) * 0.25;
                    let d8 = (at(cell_x + 1, cell_z + 1, cell_y + 1) - d4) * 0.25;

                    let mut d1 = d1;
                    let mut d2 = d2;
                    let mut d3 = d3;
                    let mut d4 = d4;

                    for slice in 0..4usize {
                        let mut d10 = d1;
                        let mut d11 = d2;
                        let d12 = (d3 - d1) * 0.125;
                        let d13 = (d4 - d2) * 0.125;

                        for row in 0..8usize {
                            let mut d15 = d10;
                            let d16 = (d11 - d10) * 0.125;

                            for column_index in 0..8usize {
                                if d15 > 0.0 {
                                    column.set_block(
                                        row + cell_x * 8,
                                        slice + cell_y * 4,
                                        column_index + cell_z * 8,
                                        BlockState::END_STONE,
                                    );
                                }
                                d15 += d16;
                            }

                            d10 += d12;
                            d11 += d13;
                        }

                        d1 += d5;
                        d2 += d6;
                        d3 += d7;
                        d4 += d8;
                    }
                }
            }
        }
    }

    /// `initializeNoiseField`, including the radial island falloff.
    fn initialize_noise_field(
        &self,
        x: i32,
        y: i32,
        z: i32,
        x_size: usize,
        y_size: usize,
        z_size: usize,
    ) -> Vec<f64> {
        let mut data_4 = vec![0.0; x_size * z_size];
        let mut data_5 = vec![0.0; x_size * z_size];
        let mut data_1 = vec![0.0; x_size * y_size * z_size];
        let mut data_2 = vec![0.0; x_size * y_size * z_size];
        let mut data_3 = vec![0.0; x_size * y_size * z_size];

        // The 2D overloads: `(x, z, xSize, zSize, scale, scale, unused)`.
        self.noise_4
            .generate_2d(&mut data_4, x, z, x_size, z_size, 1.121, 1.121);
        self.noise_5
            .generate_2d(&mut data_5, x, z, x_size, z_size, 200.0, 200.0);

        let scale = 684.412 * 2.0;
        let vertical = 684.412;
        self.noise_3.generate_octaves(
            &mut data_1,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            scale / 80.0,
            vertical / 160.0,
            scale / 80.0,
        );
        self.noise_1.generate_octaves(
            &mut data_2,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            scale,
            vertical,
            scale,
        );
        self.noise_2.generate_octaves(
            &mut data_3,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            scale,
            vertical,
            scale,
        );

        let mut out = vec![0.0f64; x_size * y_size * z_size];
        let mut index = 0usize;
        for column_x in 0..x_size {
            for column_z in 0..z_size {
                let fx = (column_x as i32 + x) as f32;
                let fz = (column_z as i32 + z) as f32;
                let falloff = (100.0 - (fx * fx + fz * fz).sqrt() * 8.0).clamp(-100.0, 80.0);

                for height in 0..y_size {
                    let mut value;
                    let high = data_2[index] / 512.0;
                    let low = data_3[index] / 512.0;
                    let blend = (data_1[index] / 10.0 + 1.0) / 2.0;
                    if blend < 0.0 {
                        value = high;
                    } else if blend > 1.0 {
                        value = low;
                    } else {
                        value = high + (low - high) * blend;
                    }

                    value -= 8.0;
                    value += f64::from(falloff);

                    if height > y_size / 2 - 2 {
                        let scale = (height - (y_size / 2 - 2)) as f32 / 64.0;
                        let scale = math::clamp(f64::from(scale), 0.0, 1.0);
                        value = value * (1.0 - scale) + -3000.0 * scale;
                    }

                    if height < 8 {
                        let scale = (8 - height) as f32 / 7.0;
                        let scale = f64::from(scale);
                        value = value * (1.0 - scale) + -30.0 * scale;
                    }

                    out[index] = value;
                    index += 1;
                }
            }
        }
        out
    }

    /// `Chunk.populateChunk`'s driver.
    pub fn populate<W: GenWorld>(
        &mut self,
        world: &mut W,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Vec<ChunkCoord> {
        crate::worldgen::populate_owners(world, chunk_x, chunk_z, |world, owner_x, owner_z| {
            self.populate_owner(world, owner_x, owner_z);
        })
    }

    /// `ChunkProviderEnd.populate`.
    pub fn populate_owner<W: GenWorld>(&mut self, world: &mut W, chunk_x: i32, chunk_z: i32) {
        // Deliberate deviation: vanilla uses the unseeded `World.rand` here.
        let mut rand = JavaRandom::new(chunk_rand_seed(chunk_x, chunk_z));
        let base_x = chunk_x * 16;
        let base_z = chunk_z * 16;

        // `generateOres()` runs first in vanilla and consumes ~2,800 draws, but
        // every `WorldGenMinable` predicate is `Blocks.stone` and the end is
        // `end_stone`, so it never places a block. It is omitted rather than
        // reproduced because the RNG is per-chunk here anyway.
        if rand.next_int_bound(5) == 0 {
            let dx = rand.next_int_bound(16) + 8;
            let dz = rand.next_int_bound(16) + 8;
            let base_y = world.height(base_x + dx, base_z + dz);
            spike(world, &mut rand, base_x + dx, base_y, base_z + dz);
        }

        // The dragon spawn at chunk (0,0) is not modelled; the draw is kept so
        // the stream continues exactly where `BiomeEndDecorator` leaves it.
        if chunk_x == 0 && chunk_z == 0 {
            let _ = rand.next_float();
        }
    }
}

fn chunk_rand_seed(chunk_x: i32, chunk_z: i32) -> i64 {
    i64::from(chunk_x)
        .wrapping_mul(341_873_128_712)
        .wrapping_add(i64::from(chunk_z).wrapping_mul(132_897_987_541))
}

/// `WorldGenSpikes`: an obsidian pillar topped with a bedrock cap (the crystal
/// entity is not part of worldgen).
fn spike<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) {
    if !world.is_air(x, y, z) || world.get_block(x, y - 1, z).id() != ids::END_STONE {
        return;
    }

    let height = rand.next_int_bound(32) + 6;
    let radius = rand.next_int_bound(4) + 1;

    // The whole disc under the pillar must be end stone.
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz <= radius * radius + 1
                && world.get_block(x + dx, y - 1, z + dz).id() != ids::END_STONE
            {
                return;
            }
        }
    }

    for level in y..(y + height).min(256) {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx * dx + dz * dz <= radius * radius + 1 {
                    world.set_block(x + dx, level, z + dz, BlockState::OBSIDIAN);
                }
            }
        }
    }

    // `EntityEnderCrystal`'s yaw; the entity itself is out of scope.
    let _ = rand.next_float();
    world.set_block(x, y + height, z, BlockState::BEDROCK);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genworld::TestWorld;

    fn generated(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        EndGenerator::new(12345).provide_chunk(chunk_x, chunk_z)
    }

    #[test]
    fn the_island_exists_at_the_origin() {
        let column = generated(0, 0);
        let mut end_stone = 0;
        for y in 0..128usize {
            for x in 0..16usize {
                for z in 0..16usize {
                    if column.get_block(x, y, z).id() == ids::END_STONE {
                        end_stone += 1;
                    }
                }
            }
        }
        assert!(end_stone > 0, "the central island is empty");
    }

    #[test]
    fn far_from_the_origin_is_empty_void() {
        let column = generated(64, 64);
        for y in 0..128usize {
            for x in 0..16usize {
                for z in 0..16usize {
                    assert!(column.get_block(x, y, z).is_air(), "{x},{y},{z}");
                }
            }
        }
    }

    #[test]
    fn provide_chunk_is_order_independent() {
        let mut forward = EndGenerator::new(7);
        let a = forward.provide_chunk(1, 2);
        let mut backward = EndGenerator::new(7);
        backward.provide_chunk(-4, 9);
        let b = backward.provide_chunk(1, 2);
        for y in 0..128 {
            for x in 0..16 {
                for z in 0..16 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn pillars_are_obsidian_with_a_bedrock_cap() {
        let mut generator = EndGenerator::new(12345);
        let mut world = TestWorld::new();
        for z in -6..=6 {
            for x in -6..=6 {
                world.insert(generator.provide_chunk(x, z));
                generator.populate(&mut world, x, z);
            }
        }

        let mut pillars = 0usize;
        for (coord, column) in world.iter() {
            for z in 0..16usize {
                for x in 0..16usize {
                    // Look for the cap: bedrock over a column of obsidian.
                    for y in (1..127usize).rev() {
                        if column.get_block(x, y, z).id() == ids::BEDROCK
                            && column.get_block(x, y - 1, z).id() == ids::OBSIDIAN
                        {
                            pillars += 1;
                        }
                    }
                }
            }
            let _ = coord;
        }
        assert!(pillars > 0, "no obsidian pillar in a 13x13 chunk window");
    }
}
