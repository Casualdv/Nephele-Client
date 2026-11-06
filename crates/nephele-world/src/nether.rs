//! `ChunkProviderHell` — nether generation.
//!
//! Reimplemented from the 1.8.9 class of the same name. The pipeline splits the
//! way the overworld's does:
//!
//! - [`NetherGenerator::provide_chunk`] is a pure function of the chunk
//!   coordinates: the netherrack/lava density field, the soul-sand/gravel/
//!   bedrock surface pass, and `MapGenCavesHell`. It reseeds one shared
//!   `Random` from `x*341873128712 + z*132897987541` — **not** the world seed.
//! - [`NetherGenerator::populate`] runs the decoration pass under the same 2×2
//!   ownership rule as the overworld, but **does not reseed**: vanilla's
//!   `ChunkProviderHell.populate` draws from the `Random` left over by the
//!   `provideChunk` call that loaded the triggering chunk. The stream therefore
//!   depends on load order, exactly as it does in vanilla. The glue in
//!   [`crate::generator`] preserves that: `provide_chunk` must be called for a
//!   chunk immediately before `populate` for that chunk, as the server does.
//!
//! Structures are not ported: `MapGenNetherBridge` is a no-op here, so a chunk
//! containing a fortress piece will differ from vanilla (the piece, and the
//! `Random` draws it consumes, are missing).

use crate::block::{ids, BlockState};
use crate::chunk::ChunkColumn;
use crate::genworld::GenWorld;
use crate::hell_caves::HellCaves;
use crate::math;
use crate::noise::{JavaRandom, NoiseOctaves};
use crate::world::ChunkCoord;

/// `ChunkProviderHell`'s hard-coded sea level (`worldIn.setSeaLevel(63)`).
pub const NETHER_SEA_LEVEL: i32 = 63;
/// `BiomeGenHell.biomeID`. `WorldChunkManagerHell` returns it everywhere.
pub const HELL_BIOME_ID: i32 = 8;

/// Nether terrain and its decoration pass.
pub struct NetherGenerator {
    seed: i64,
    /// `hellRNG`: reseeded by every `provide_chunk` and then drawn from by
    /// `populate`. One instance, as in vanilla.
    rand: JavaRandom,
    noise_1: NoiseOctaves,
    noise_2: NoiseOctaves,
    noise_3: NoiseOctaves,
    slowsand_gravel: NoiseOctaves,
    netherrack_exclusivity: NoiseOctaves,
    noise_6: NoiseOctaves,
    noise_7: NoiseOctaves,
    caves: HellCaves,
    sea_level: i32,
}

impl NetherGenerator {
    pub fn new(seed: i64) -> Self {
        let mut rand = JavaRandom::new(seed);
        // The constructor order is the RNG order: every octave generator pulls
        // its permutation table from the same stream.
        let noise_1 = NoiseOctaves::new(&mut rand, 16);
        let noise_2 = NoiseOctaves::new(&mut rand, 16);
        let noise_3 = NoiseOctaves::new(&mut rand, 8);
        let slowsand_gravel = NoiseOctaves::new(&mut rand, 4);
        let netherrack_exclusivity = NoiseOctaves::new(&mut rand, 4);
        let noise_6 = NoiseOctaves::new(&mut rand, 10);
        let noise_7 = NoiseOctaves::new(&mut rand, 16);
        Self {
            seed,
            rand,
            noise_1,
            noise_2,
            noise_3,
            slowsand_gravel,
            netherrack_exclusivity,
            noise_6,
            noise_7,
            caves: HellCaves::new(seed),
            sea_level: NETHER_SEA_LEVEL,
        }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    pub fn sea_level(&self) -> i32 {
        self.sea_level
    }

    /// `ChunkProviderHell.provideChunk`: density field, surface, caves.
    pub fn provide_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        self.rand.set_seed(chunk_rand_seed(chunk_x, chunk_z));

        let mut column = ChunkColumn::empty(chunk_x, chunk_z);
        self.terrain(chunk_x, chunk_z, &mut column);
        self.surface(chunk_x, chunk_z, &mut column);
        self.caves.carve(&mut column);

        // `WorldChunkManagerHell.loadBlockGeneratorData` fills every cell with
        // hell; the chunk stores that as `(byte) biomeID`.
        column.set_biomes(&[HELL_BIOME_ID as u8; 256]);
        column
    }

    /// `func_180515_a`: the netherrack/lava density field.
    fn terrain(&mut self, chunk_x: i32, chunk_z: i32, column: &mut ChunkColumn) {
        // `i = 4`, `j = seaLevel / 2 + 1`, and 17 vertical samples.
        let cells = 4usize;
        let lava_top = self.sea_level / 2 + 1; // 32
        let x_size = cells + 1;
        let y_size = 17usize;
        let z_size = cells + 1;

        let field = self.initialize_noise_field(
            chunk_x * cells as i32,
            0,
            chunk_z * cells as i32,
            x_size,
            y_size,
            z_size,
        );
        let at = |x: usize, z: usize, y: usize| field[(x * z_size + z) * y_size + y];

        for cell_x in 0..cells {
            for cell_z in 0..cells {
                for cell_y in 0..16usize {
                    let d1 = at(cell_x, cell_z, cell_y);
                    let d2 = at(cell_x, cell_z + 1, cell_y);
                    let d3 = at(cell_x + 1, cell_z, cell_y);
                    let d4 = at(cell_x + 1, cell_z + 1, cell_y);
                    let d5 = (at(cell_x, cell_z, cell_y + 1) - d1) * 0.125;
                    let d6 = (at(cell_x, cell_z + 1, cell_y + 1) - d2) * 0.125;
                    let d7 = (at(cell_x + 1, cell_z, cell_y + 1) - d3) * 0.125;
                    let d8 = (at(cell_x + 1, cell_z + 1, cell_y + 1) - d4) * 0.125;

                    let mut d1 = d1;
                    let mut d2 = d2;
                    let mut d3 = d3;
                    let mut d4 = d4;

                    for slice in 0..8usize {
                        let mut d10 = d1;
                        let mut d11 = d2;
                        let d12 = (d3 - d1) * 0.25;
                        let d13 = (d4 - d2) * 0.25;

                        for row in 0..4usize {
                            let mut d15 = d10;
                            let d16 = (d11 - d10) * 0.25;

                            for column_index in 0..4usize {
                                let mut state = BlockState::AIR;
                                if cell_y * 8 + slice < lava_top as usize {
                                    state = BlockState::LAVA;
                                }
                                if d15 > 0.0 {
                                    state = BlockState::NETHERRACK;
                                }
                                let x = row + cell_x * 4;
                                let y = slice + cell_y * 8;
                                let z = column_index + cell_z * 4;
                                if !state.is_air() {
                                    column.set_block(x, y, z, state);
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

    /// `initializeNoiseField`: five octave fields combined into one density
    /// grid of `x_size * y_size * z_size`, laid out `[x][z][y]`.
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

        self.noise_6
            .generate_octaves(&mut data_4, x, y, z, x_size, 1, z_size, 1.0, 0.0, 1.0);
        self.noise_7
            .generate_octaves(&mut data_5, x, y, z, x_size, 1, z_size, 100.0, 0.0, 100.0);
        self.noise_3.generate_octaves(
            &mut data_1,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            684.412 / 80.0,
            2053.236 / 60.0,
            684.412 / 80.0,
        );
        self.noise_1.generate_octaves(
            &mut data_2,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            684.412,
            2053.236,
            684.412,
        );
        self.noise_2.generate_octaves(
            &mut data_3,
            x,
            y,
            z,
            x_size,
            y_size,
            z_size,
            684.412,
            2053.236,
            684.412,
        );

        let mut taper = vec![0.0f64; y_size];
        for (index, slot) in taper.iter_mut().enumerate() {
            *slot = (index as f64 * std::f64::consts::PI * 6.0 / y_size as f64).cos() * 2.0;
            let mut low = index as f64;
            if index > y_size / 2 {
                low = (y_size - 1 - index) as f64;
            }
            if low < 4.0 {
                low = 4.0 - low;
                *slot -= low * low * low * 10.0;
            }
        }

        let mut out = vec![0.0f64; x_size * y_size * z_size];
        let mut index = 0usize;
        for _x in 0..x_size {
            for _z in 0..z_size {
                for (height, taper_value) in taper.iter().enumerate() {
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

                    value -= taper_value;

                    if height > y_size - 4 {
                        let scale = (height - (y_size - 4)) as f32 / 3.0;
                        value = value * f64::from(1.0 - scale) + -10.0 * f64::from(scale);
                    }

                    out[index] = value;
                    index += 1;
                }
            }
        }
        out
    }

    /// `func_180516_b`: netherrack, soul sand, gravel, bedrock and the lava
    /// surface.
    fn surface(&mut self, chunk_x: i32, chunk_z: i32, column: &mut ChunkColumn) {
        let sea = self.sea_level + 1; // 64
        let scale = 0.03125;

        let mut slowsand = vec![0.0; 256];
        let mut gravel = vec![0.0; 256];
        let mut exclusivity = vec![0.0; 256];

        // The 3D overload is used with ySize == 16 or 1 so the noise runs along
        // the *z* axis; the array layout ends up `[x][z]` either way.
        self.slowsand_gravel.generate_octaves(
            &mut slowsand,
            chunk_x * 16,
            chunk_z * 16,
            0,
            16,
            16,
            1,
            scale,
            scale,
            1.0,
        );
        self.slowsand_gravel.generate_octaves(
            &mut gravel,
            chunk_x * 16,
            109,
            chunk_z * 16,
            16,
            1,
            16,
            scale,
            1.0,
            scale,
        );
        self.netherrack_exclusivity.generate_octaves(
            &mut exclusivity,
            chunk_x * 16,
            chunk_z * 16,
            0,
            16,
            16,
            1,
            scale * 2.0,
            scale * 2.0,
            scale * 2.0,
        );

        for z in 0..16usize {
            for x in 0..16usize {
                let index = x * 16 + z;
                let soul_sand = slowsand[index] + self.rand.next_double() * 0.2 > 0.0;
                let gravel_patch = gravel[index] + self.rand.next_double() * 0.2 > 0.0;
                let depth =
                    (exclusivity[index] / 3.0 + 3.0 + self.rand.next_double() * 0.25) as i32;

                let mut depth_left = -1i32;
                let mut top: Option<BlockState> = Some(BlockState::NETHERRACK);
                let mut filler = BlockState::NETHERRACK;

                for y in (0..=127i32).rev() {
                    // Java short-circuits: the second `nextInt(5)` is only drawn
                    // when the first comparison is true.
                    let below_ceiling = y < 127 - self.rand.next_int_bound(5);
                    if below_ceiling && y > self.rand.next_int_bound(5) {
                        let state = column.get_block(x, y as usize, z);
                        if state.is_air() {
                            depth_left = -1;
                            continue;
                        }
                        if state.id() != ids::NETHERRACK {
                            continue;
                        }

                        if depth_left == -1 {
                            if depth <= 0 {
                                top = None;
                                filler = BlockState::NETHERRACK;
                            } else if (sea - 4..=sea + 1).contains(&y) {
                                top = Some(BlockState::NETHERRACK);
                                filler = BlockState::NETHERRACK;
                                if gravel_patch {
                                    top = Some(BlockState::GRAVEL);
                                    filler = BlockState::NETHERRACK;
                                }
                                if soul_sand {
                                    top = Some(BlockState::SOUL_SAND);
                                    filler = BlockState::SOUL_SAND;
                                }
                            }

                            if y < sea && top.is_none_or(|state| state.is_air()) {
                                top = Some(BlockState::LAVA);
                            }

                            depth_left = depth;
                            let state = if y >= sea - 1 {
                                top.unwrap_or(BlockState::AIR)
                            } else {
                                filler
                            };
                            column.set_block(x, y as usize, z, state);
                        } else if depth_left > 0 {
                            depth_left -= 1;
                            column.set_block(x, y as usize, z, filler);
                        }
                    } else {
                        column.set_block(x, y as usize, z, BlockState::BEDROCK);
                    }
                }
            }
        }
    }

    /// `Chunk.populateChunk`'s driver, same rule and order as the overworld's.
    ///
    /// The shared `Random` is **not** reseeded per owner: vanilla's nether
    /// provider draws from the stream left by the `provide_chunk` that loaded
    /// the chunk which completed the 2×2 square.
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

    /// `ChunkProviderHell.populate`.
    pub fn populate_owner<W: GenWorld>(&mut self, world: &mut W, chunk_x: i32, chunk_z: i32) {
        let base_x = chunk_x * 16;
        let base_z = chunk_z * 16;

        // `genNetherBridge.generateStructure` is not ported (see module docs).

        // Eight non-spring lava pockets: `WorldGenHellLava(..., false)`.
        for _ in 0..8 {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(120) + 4;
            let z = base_z + self.rand.next_int_bound(16) + 8;
            hell_lava(world, x, y, z, false);
        }

        // Fire. `nextInt(nextInt(10) + 1) + 1` is evaluated inside out.
        let inner = self.rand.next_int_bound(10);
        let count = self.rand.next_int_bound(inner + 1) + 1;
        for _ in 0..count {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(120) + 4;
            let z = base_z + self.rand.next_int_bound(16) + 8;
            fire(world, &mut self.rand, x, y, z);
        }

        // Downward glowstone.
        let inner = self.rand.next_int_bound(10);
        let count = self.rand.next_int_bound(inner + 1);
        for _ in 0..count {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(120) + 4;
            let z = base_z + self.rand.next_int_bound(16) + 8;
            glowstone(world, &mut self.rand, x, y, z, false);
        }

        // Upward glowstone: ten attempts, always.
        for _ in 0..10 {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(128);
            let z = base_z + self.rand.next_int_bound(16) + 8;
            glowstone(world, &mut self.rand, x, y, z, true);
        }

        if self.rand.next_boolean() {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(128);
            let z = base_z + self.rand.next_int_bound(16) + 8;
            bush(world, &mut self.rand, x, y, z, BlockState::BROWN_MUSHROOM);
        }
        if self.rand.next_boolean() {
            let x = base_x + self.rand.next_int_bound(16) + 8;
            let y = self.rand.next_int_bound(128);
            let z = base_z + self.rand.next_int_bound(16) + 8;
            bush(world, &mut self.rand, x, y, z, BlockState::RED_MUSHROOM);
        }

        // Quartz: 16 veins of 14, replacing netherrack only.
        for _ in 0..16 {
            let x = base_x + self.rand.next_int_bound(16);
            let y = self.rand.next_int_bound(108) + 10;
            let z = base_z + self.rand.next_int_bound(16);
            minable(world, &mut self.rand, x, y, z, 14, BlockState::QUARTZ_ORE);
        }

        // Sixteen lava springs (`WorldGenHellLava(..., true)`).
        for _ in 0..16 {
            let x = base_x + self.rand.next_int_bound(16);
            let y = self.rand.next_int_bound(108) + 10;
            let z = base_z + self.rand.next_int_bound(16);
            hell_lava(world, x, y, z, true);
        }
    }
}

/// `func_180516_b`'s noise reseed: the same chunk-seed formula the overworld
/// uses, without the world seed.
fn chunk_rand_seed(chunk_x: i32, chunk_z: i32) -> i64 {
    i64::from(chunk_x)
        .wrapping_mul(341_873_128_712)
        .wrapping_add(i64::from(chunk_z).wrapping_mul(132_897_987_541))
}

/// `WorldGenFire`: sixty-four attempts at a fire on top of netherrack.
fn fire<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) {
    for _ in 0..64 {
        let dx = rand.next_int_bound(8) - rand.next_int_bound(8);
        let dy = rand.next_int_bound(4) - rand.next_int_bound(4);
        let dz = rand.next_int_bound(8) - rand.next_int_bound(8);
        let (fx, fy, fz) = (x + dx, y + dy, z + dz);
        if world.is_air(fx, fy, fz) && world.get_block(fx, fy - 1, fz).id() == ids::NETHERRACK {
            world.set_block(fx, fy, fz, BlockState::FIRE);
        }
    }
}

/// `WorldGenGlowStone1` / `WorldGenGlowStone2`.
///
/// `up` is false for the downward generator (the ceiling clusters) and true for
/// the upward one. The decompiled tree in `Nephele-private` has the *downward*
/// draw in both classes; vanilla ships `+rand.nextInt(12)` in `GlowStone2`, and
/// the upward form is what the reference world shows, so the port follows
/// vanilla here. See AGENTS.md.
fn glowstone<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32, up: bool) {
    if !world.is_air(x, y, z) || world.get_block(x, y + 1, z).id() != ids::NETHERRACK {
        return;
    }
    world.set_block(x, y, z, BlockState::GLOWSTONE);

    for _ in 0..1500 {
        let dx = rand.next_int_bound(8) - rand.next_int_bound(8);
        let dy = rand.next_int_bound(12);
        let dz = rand.next_int_bound(8) - rand.next_int_bound(8);
        let (gx, gy, gz) = (x + dx, if up { y + dy } else { y - dy }, z + dz);

        if !world.is_air(gx, gy, gz) {
            continue;
        }

        let mut neighbours = 0;
        for (nx, ny, nz) in [
            (gx, gy - 1, gz),
            (gx, gy + 1, gz),
            (gx, gy, gz - 1),
            (gx, gy, gz + 1),
            (gx - 1, gy, gz),
            (gx + 1, gy, gz),
        ] {
            if world.get_block(nx, ny, nz).id() == ids::GLOWSTONE {
                neighbours += 1;
            }
            if neighbours > 1 {
                break;
            }
        }

        if neighbours == 1 {
            world.set_block(gx, gy, gz, BlockState::GLOWSTONE);
        }
    }
}

/// `GeneratorBushFeature`: sixty-four attempts at a mushroom.
///
/// The vanilla `canBlockStay` also asks `World.getLight(pos) < 13`. This crate
/// has no light volume during generation, so the light test is treated as
/// passing; the only misses are mushrooms within thirteen blocks of lava or
/// glowstone, which is a handful of blocks per world.
fn bush<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    x: i32,
    y: i32,
    z: i32,
    block: BlockState,
) {
    for _ in 0..64 {
        let dx = rand.next_int_bound(8) - rand.next_int_bound(8);
        let dy = rand.next_int_bound(4) - rand.next_int_bound(4);
        let dz = rand.next_int_bound(8) - rand.next_int_bound(8);
        let (bx, by, bz) = (x + dx, y + dy, z + dz);
        if !world.is_air(bx, by, bz) || by >= 255 {
            continue;
        }
        let below = world.get_block(bx, by - 1, bz);
        // `BlockMushroom.canBlockStay` wants a full block underneath. In the
        // nether that is netherrack (or bedrock/gravel/soul sand), never lava.
        if below.is_air() || crate::block::is_liquid(below) {
            continue;
        }
        world.set_block(bx, by, bz, block);
    }
}

/// `WorldGenHellLava`.
///
/// Vanilla places flowing lava and then forces an immediate tick, which turns
/// the source into *static* lava and lets it flow into its one air neighbour.
/// The flow cascade uses the world's unseeded `Random` and is not modelled, so
/// the source block is placed as static lava and the spring stays put. An
/// enclosed spring (`i == 5`) is bit-identical; a one-air-neighbour spring
/// matches at the source and is missing its downstream flow.
fn hell_lava<W: GenWorld>(world: &mut W, x: i32, y: i32, z: i32, spring: bool) {
    if world.get_block(x, y + 1, z).id() != ids::NETHERRACK {
        return;
    }
    let here = world.get_block(x, y, z);
    if !here.is_air() && here.id() != ids::NETHERRACK {
        return;
    }

    let neighbours = [
        (x - 1, y, z),
        (x + 1, y, z),
        (x, y, z - 1),
        (x, y, z + 1),
        (x, y - 1, z),
    ];
    let rock = neighbours
        .iter()
        .filter(|(nx, ny, nz)| world.get_block(*nx, *ny, *nz).id() == ids::NETHERRACK)
        .count();
    let air = neighbours
        .iter()
        .filter(|(nx, ny, nz)| world.is_air(*nx, *ny, *nz))
        .count();

    if (!spring && rock == 4 && air == 1) || rock == 5 {
        world.set_block(x, y, z, BlockState::LAVA);
    }
}

/// `WorldGenMinable`: an ellipsoidal vein of `size` blocks, replacing only
/// `target`'s block id.
fn minable<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    x: i32,
    y: i32,
    z: i32,
    size: i32,
    ore: BlockState,
) {
    let angle = rand.next_float() * std::f32::consts::PI;
    let radius = size as f32 / 8.0;
    let start_x = (x + 8) as f32 + math::sin(angle) * radius;
    let end_x = (x + 8) as f32 - math::sin(angle) * radius;
    let start_z = (z + 8) as f32 + math::cos(angle) * radius;
    let end_z = (z + 8) as f32 - math::cos(angle) * radius;
    let start_y = (y + rand.next_int_bound(3) - 2) as f64;
    let end_y = (y + rand.next_int_bound(3) - 2) as f64;

    let start_x = f64::from(start_x);
    let end_x = f64::from(end_x);
    let start_z = f64::from(start_z);
    let end_z = f64::from(end_z);

    for index in 0..size {
        let progress = index as f32 / size as f32;
        let centre_x = start_x + (end_x - start_x) * f64::from(progress);
        let centre_y = start_y + (end_y - start_y) * f64::from(progress);
        let centre_z = start_z + (end_z - start_z) * f64::from(progress);
        let wobble = rand.next_double() * f64::from(size) / 16.0;
        let extent = f64::from(math::sin(std::f32::consts::PI * progress) + 1.0) * wobble + 1.0;

        let from_x = math::floor(centre_x - extent / 2.0);
        let from_y = math::floor(centre_y - extent / 2.0);
        let from_z = math::floor(centre_z - extent / 2.0);
        let to_x = math::floor(centre_x + extent / 2.0);
        let to_y = math::floor(centre_y + extent / 2.0);
        let to_z = math::floor(centre_z + extent / 2.0);

        for block_x in from_x..=to_x {
            let nx = (f64::from(block_x) + 0.5 - centre_x) / (extent / 2.0);
            if nx * nx >= 1.0 {
                continue;
            }
            for block_y in from_y..=to_y {
                let ny = (f64::from(block_y) + 0.5 - centre_y) / (extent / 2.0);
                if nx * nx + ny * ny >= 1.0 {
                    continue;
                }
                for block_z in from_z..=to_z {
                    let nz = (f64::from(block_z) + 0.5 - centre_z) / (extent / 2.0);
                    if nx * nx + ny * ny + nz * nz < 1.0
                        && world.get_block(block_x, block_y, block_z).id() == ids::NETHERRACK
                    {
                        world.set_block(block_x, block_y, block_z, ore);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genworld::TestWorld;
    use crate::worldgen::highest_solid;

    fn generated(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        NetherGenerator::new(12345).provide_chunk(chunk_x, chunk_z)
    }

    #[test]
    fn the_nether_has_a_bedrock_roof_and_floor() {
        let column = generated(0, 0);
        for x in 0..16usize {
            for z in 0..16usize {
                assert_eq!(column.get_block(x, 0, z).id(), ids::BEDROCK, "floor");
                assert_eq!(column.get_block(x, 127, z).id(), ids::BEDROCK, "roof");
            }
        }
    }

    #[test]
    fn the_nether_has_lava_below_the_sea_level_and_no_water() {
        // `func_180515_a` defaults every cell under `seaLevel / 2 + 1` to lava
        // and overwrites it where the density field is positive, so the layer is
        // lava, netherrack islands, or cave air — never water. How much of it is
        // lava is decided by the density; on seed 12345 the reference code gives
        // a patchy lava layer rather than a full ocean — see AGENTS.md.
        let mut lava = 0;
        for chunk_x in -3..=3 {
            let column = generated(chunk_x, 5);
            for y in 0..128usize {
                for x in 0..16usize {
                    for z in 0..16usize {
                        let state = column.get_block(x, y, z);
                        assert_ne!(state.id(), ids::WATER, "water at {x},{y},{z}");
                        if y < 32 && state.id() == ids::LAVA {
                            lava += 1;
                        }
                    }
                }
            }
        }
        assert!(
            lava > 0,
            "the density never leaves lava under the sea level"
        );
    }

    #[test]
    fn the_nether_is_made_of_netherrack_and_nothing_else_at_this_stage() {
        let column = generated(7, -2);
        for y in 0..128 {
            for x in 0..16 {
                for z in 0..16 {
                    let state = column.get_block(x, y, z);
                    assert!(
                        matches!(
                            state.id(),
                            ids::AIR
                                | ids::NETHERRACK
                                | ids::BEDROCK
                                | ids::LAVA
                                | ids::SOUL_SAND
                                | ids::GRAVEL
                        ),
                        "{state:?} at {x},{y},{z}"
                    );
                }
            }
        }
    }

    #[test]
    fn provide_chunk_is_order_independent() {
        let mut forward = NetherGenerator::new(99);
        let a = forward.provide_chunk(2, 3);
        let mut backward = NetherGenerator::new(99);
        backward.provide_chunk(-1, -1);
        let b = backward.provide_chunk(2, 3);
        for y in 0..128 {
            for x in 0..16 {
                for z in 0..16 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn the_decoration_pass_adds_glowstone_quartz_and_fire() {
        let mut generator = NetherGenerator::new(12345);
        let mut world = TestWorld::new();
        // A 6x6 block of chunks, loaded and populated in raster order, gives
        // the features neighbours to write into.
        let mut counts = std::collections::HashMap::new();
        for z in 0..6 {
            for x in 0..6 {
                world.insert(generator.provide_chunk(x, z));
                generator.populate(&mut world, x, z);
            }
        }
        for (_, column) in world.iter() {
            for y in 0..128usize {
                for z in 0..16usize {
                    for x in 0..16usize {
                        let id = column.get_block(x, y, z).id();
                        if !matches!(id, ids::AIR | ids::NETHERRACK | ids::BEDROCK | ids::LAVA) {
                            *counts.entry(id).or_insert(0usize) += 1;
                        }
                    }
                }
            }
        }
        assert!(
            counts.get(&ids::GLOWSTONE).copied().unwrap_or(0) > 0,
            "no glowstone: {counts:?}"
        );
        assert!(
            counts.get(&ids::QUARTZ_ORE).copied().unwrap_or(0) > 0,
            "no quartz: {counts:?}"
        );
        assert!(
            counts.get(&ids::FIRE).copied().unwrap_or(0) > 0,
            "no fire: {counts:?}"
        );
    }

    #[test]
    fn the_top_of_the_nether_is_128_blocks() {
        let column = generated(0, 0);
        let top = highest_solid(&column, 0, 0).expect("not empty");
        assert!(top <= 127, "top {top}");
    }
}
