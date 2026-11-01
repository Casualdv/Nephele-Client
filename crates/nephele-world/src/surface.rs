//! The surface pass: `ChunkProviderGenerate.replaceBlocksForBiome` and the
//! per-biome `genTerrainBlocks` that sits behind it.
//!
//! The density field leaves a chunk of bare stone, water and air. This layer
//! walks each column from the top down and turns the buried stone into a
//! biome's top block plus a few blocks of filler, adds gravel where the ground
//! sits far below sea level, turns sand into sandstone, and buries the bottom
//! of the world in bedrock.
//!
//! Everything here follows `BiomeGenBase.generateBiomeTerrain` line by line.
//! `Rand` is vanilla's shared `JavaRandom`, so the caller controls the stream;
//! the heights and block *types* it produces do not depend on it, only the
//! filler depth and the bedrock roughness do.

use crate::biome;
use crate::block::{self, BlockState};
use crate::chunk::ChunkColumn;
use crate::noise::{JavaRandom, NoisePerlin};

/// The world's sea level as `ChunkProviderSettings` defines it.
pub const SEA_LEVEL: i32 = 63;

/// Air, so `Option<u16>` can carry "no top block" the way vanilla's null does.
const AIR: u16 = block::ids::AIR;

/// The block pair a biome lays down: what goes on the surface, and what fills
/// the few blocks under it.
///
/// Defaults are grass over dirt; only biomes that differ are listed.
pub fn biome_surface(biome_id: i32) -> (u16, u16) {
    // A mutated biome copies its base biome's pair in `BiomeGenMutated`.
    let base = if biome_id >= 128 {
        biome_id - 128
    } else {
        biome_id
    };

    match base {
        // Beach and desert are sand all the way down; the generic pass turns
        // the lower sand into sandstone.
        2 | 16 => (block::ids::SAND, block::ids::SAND),
        // The End is dirt over dirt.
        9 => (block::ids::DIRT, block::ids::DIRT),
        // Ice plains and ice mountains top out with a snow layer.
        12 | 13 => (block::ids::SNOW_LAYER, block::ids::DIRT),
        14 => (block::ids::MYCELIUM, block::ids::DIRT),
        25 => (block::ids::STONE, block::ids::STONE),
        37..=39 => (block::ids::SAND, block::ids::STAINED_HARDENED_CLAY),
        _ => (block::ids::GRASS, block::ids::DIRT),
    }
}

/// The two extra noise fields the surface pass consults besides the stone
/// noise: one for temperature (ice versus water at the waterline) and one for
/// the swamp's patches.
pub struct SurfaceNoise {
    temperature: NoisePerlin,
    grass_color: NoisePerlin,
}

impl SurfaceNoise {
    pub fn new() -> Self {
        // Vanilla's own static seeds, unrelated to the world seed.
        let mut temperature = JavaRandom::new(1234);
        let mut grass = JavaRandom::new(2345);
        Self {
            temperature: NoisePerlin::new(&mut temperature, 1),
            grass_color: NoisePerlin::new(&mut grass, 1),
        }
    }

    /// `BiomeGenBase.getFloatTemperature`: above y = 64 the biome temperature
    /// is cooled by noise and by height.
    fn temperature(&self, biome_id: i32, x: i32, y: i32, z: i32) -> f32 {
        let base = biome::any(biome_id)
            .map(|biome| biome.base.temperature)
            .unwrap_or(0.5);
        if y > 64 {
            let wobble = self
                .temperature
                .sample(f64::from(x) / 8.0, f64::from(z) / 8.0)
                * 4.0;
            base - (wobble as f32 + (y - 64) as f32) * 0.05 / 30.0
        } else {
            base
        }
    }

    /// `GRASS_COLOR_NOISE`, reused by the swamp for its water patches.
    fn swamp_patch(&self, x: i32, z: i32) -> f64 {
        self.grass_color
            .sample(f64::from(x) * 0.25, f64::from(z) * 0.25)
    }
}

impl Default for SurfaceNoise {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs the surface pass over a chunk.
///
/// `biomes` is the 16x16 per-block biome map (`Level.Biomes`), and `stone_noise`
/// is the 16x16 field `replaceBlocksForBiome` computes beforehand.
pub fn apply(
    column: &mut ChunkColumn,
    biomes: &[i32],
    stone_noise: &[f64; 256],
    noise: &SurfaceNoise,
    rand: &mut JavaRandom,
) {
    assert_eq!(biomes.len(), 256);
    let chunk_x = column.x;
    let chunk_z = column.z;

    for z in 0..16usize {
        for x in 0..16usize {
            let biome_id = biomes[z * 16 + x];
            let (mut top, mut filler) = biome_surface(biome_id);
            let world_x = chunk_x * 16 + x as i32;
            let world_z = chunk_z * 16 + z as i32;

            // `BiomeGenHills.genTerrainBlocks`. The biome carries a flag that
            // is 0 for the plain hills, 1 for Extreme Hills+, and 2 for the
            // mutated ones; gravel only appears on the mutated ones, and stone
            // replaces grass above noise 1.0 everywhere except Extreme Hills+.
            if let Some(biome) = biome::any(biome_id) {
                if biome.class == biome::BiomeClass::Hills {
                    let value = stone_noise[z * 16 + x];
                    if biome.mutated {
                        if !(-1.0..=2.0).contains(&value) {
                            top = block::ids::GRAVEL;
                            filler = block::ids::GRAVEL;
                        } else if value > 1.0 {
                            top = block::ids::STONE;
                            filler = block::ids::STONE;
                        }
                    } else if biome.base.id != 34 && value > 1.0 {
                        top = block::ids::STONE;
                        filler = block::ids::STONE;
                    }
                }
            }

            if biome::is_swamp_class(biome_id) {
                swamp_water(column, x, z, world_x, world_z, noise);
            }

            let temperature = noise.temperature(biome_id, world_x, 62, world_z);
            generate_biome_terrain(
                column,
                x,
                z,
                top,
                filler,
                stone_noise[z * 16 + x],
                temperature,
                rand,
            );
        }
    }
}

/// `BiomeGenSwamp.genTerrainBlocks`' pre-pass: where the swamp noise is
/// positive, the block sitting exactly at sea level becomes water, sometimes
/// with a lily pad on top.
fn swamp_water(
    column: &mut ChunkColumn,
    x: usize,
    z: usize,
    world_x: i32,
    world_z: i32,
    noise: &SurfaceNoise,
) {
    let patch = noise.swamp_patch(world_x, world_z);
    if patch <= 0.0 {
        return;
    }

    for y in (0..=255usize).rev() {
        let state = column.get_block(x, y, z);
        if state.is_air() {
            continue;
        }
        if y == 62 && state.id() != block::ids::WATER {
            column.set_block(x, y, z, BlockState::new(block::ids::WATER, 0));
            if patch < 0.12 {
                column.set_block(x, y + 1, z, BlockState::new(block::ids::WATER_LILY, 0));
            }
        }
        break;
    }
}

/// `BiomeGenBase.generateBiomeTerrain`.
#[allow(clippy::too_many_arguments)]
fn generate_biome_terrain(
    column: &mut ChunkColumn,
    x: usize,
    z: usize,
    top: u16,
    filler: u16,
    noise_value: f64,
    temperature: f32,
    rand: &mut JavaRandom,
) {
    let mut top_block = Some(top);
    let mut filler_block = filler;
    let mut depth = -1i32;
    let thickness = (noise_value / 3.0 + 3.0 + rand.next_double() * 0.25) as i32;

    for y in (0..=255i32).rev() {
        if y <= rand.next_int_bound(5) {
            column.set_block(x, y as usize, z, BlockState::new(block::ids::BEDROCK, 0));
            continue;
        }

        let state = column.get_block(x, y as usize, z);
        if state.is_air() {
            depth = -1;
        } else if state.id() == block::ids::STONE {
            if depth == -1 {
                if thickness <= 0 {
                    top_block = None;
                    filler_block = block::ids::STONE;
                } else if (SEA_LEVEL - 4..=SEA_LEVEL + 1).contains(&y) {
                    top_block = Some(top);
                    filler_block = filler;
                }

                if y < SEA_LEVEL && matches!(top_block, None | Some(AIR)) {
                    top_block = Some(if temperature < 0.15 {
                        block::ids::ICE
                    } else {
                        block::ids::WATER
                    });
                }

                depth = thickness;

                if y >= SEA_LEVEL - 1 {
                    if let Some(block) = top_block {
                        column.set_block(x, y as usize, z, BlockState::new(block, 0));
                    }
                } else if y < SEA_LEVEL - 7 - thickness {
                    top_block = None;
                    filler_block = block::ids::STONE;
                    column.set_block(x, y as usize, z, BlockState::new(block::ids::GRAVEL, 0));
                } else {
                    column.set_block(x, y as usize, z, BlockState::new(filler_block, 0));
                }
            } else if depth > 0 {
                depth -= 1;
                column.set_block(x, y as usize, z, BlockState::new(filler_block, 0));

                if depth == 0 && filler_block == block::ids::SAND {
                    depth = rand.next_int_bound(4) + (y - 63).max(0);
                    filler_block = block::ids::SANDSTONE;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_pair_is_grass_over_dirt() {
        for id in [0, 1, 3, 4, 5, 6, 7, 10, 21, 27, 29, 34, 35] {
            assert_eq!(
                biome_surface(id),
                (block::ids::GRASS, block::ids::DIRT),
                "biome {id}"
            );
        }
    }

    #[test]
    fn sandy_and_snowy_biomes_use_their_own_pair() {
        assert_eq!(biome_surface(2), (block::ids::SAND, block::ids::SAND));
        assert_eq!(biome_surface(16), (block::ids::SAND, block::ids::SAND));
        assert_eq!(
            biome_surface(12),
            (block::ids::SNOW_LAYER, block::ids::DIRT)
        );
        assert_eq!(biome_surface(14), (block::ids::MYCELIUM, block::ids::DIRT));
        assert_eq!(biome_surface(25), (block::ids::STONE, block::ids::STONE));
    }

    #[test]
    fn mutated_biomes_inherit_their_base_pair() {
        for id in [2, 4, 12, 25, 35] {
            assert_eq!(biome_surface(id + 128), biome_surface(id), "biome {id}");
        }
    }

    #[test]
    fn a_grass_column_gets_a_grass_top_over_dirt() {
        let mut column = ChunkColumn::empty(0, 0);
        for y in 0..=64 {
            column.set_block(3, y, 5, BlockState::new(block::ids::STONE, 0));
        }
        let mut rand = JavaRandom::new(7);
        let biomes = [1i32; 256];
        let noise = [0.0f64; 256];
        apply(
            &mut column,
            &biomes,
            &noise,
            &SurfaceNoise::new(),
            &mut rand,
        );

        assert_eq!(
            column.get_block(3, 64, 5).id(),
            block::ids::GRASS,
            "the top solid block becomes grass"
        );
        assert_eq!(
            column.get_block(3, 63, 5).id(),
            block::ids::DIRT,
            "and the block under it becomes dirt"
        );
        assert_eq!(
            column.get_block(3, 0, 5).id(),
            block::ids::BEDROCK,
            "the bottom of the world is bedrock"
        );
    }

    /// Block types that only appear on top because something was placed there
    /// afterwards; a column topped by one of these says nothing about the
    /// surface pass.
    fn is_decoration(id: u16) -> bool {
        matches!(
            id,
            6 | 17 | 18 | 31 | 32 | 37 | 38 | 39 | 40 | 81 | 83 | 86 | 103 | 106 | 111
        )
    }

    #[test]
    fn surface_blocks_match_a_world_the_real_server_generated() {
        use crate::genlayer::BiomeLayers;
        use crate::terrain::TerrainNoise;
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        if !dir.is_dir() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let mut layers = BiomeLayers::new(12345);
        let mut terrain = TerrainNoise::new(12345);
        let noise = SurfaceNoise::new();

        let mut compared = 0usize;
        let mut mismatched = 0usize;
        let mut example = None;
        let mut per_biome: std::collections::BTreeMap<i32, (usize, usize)> =
            std::collections::BTreeMap::new();

        for name in ["r.-1.-1.mca", "r.-1.0.mca"] {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let region = RegionFile::open(&path).expect("region opens");
            for lz in 0..32 {
                for lx in 0..32 {
                    let Some(bytes) = region.chunk_nbt(lx, lz).ok().flatten() else {
                        continue;
                    };
                    let Ok(stored) = parse_chunk(&bytes) else {
                        continue;
                    };

                    // Only chunks whose per-block biome map we reproduce: the
                    // surface pass reads biomes, and a wrong biome here would
                    // blame it for the generation bug upstream.
                    let biomes = layers.chunk_biomes(stored.x, stored.z);
                    if biomes.len() != stored.biomes.len()
                        || biomes
                            .iter()
                            .zip(&stored.biomes)
                            .any(|(ours, real)| *ours != i32::from(*real))
                    {
                        continue;
                    }

                    let mut column = terrain.primer(&mut layers, stored.x, stored.z);
                    let heights = terrain.surface_heights();
                    let stone = terrain.stone_noise(stored.x, stored.z);
                    let mut rand = JavaRandom::new(12345);
                    apply(&mut column, &biomes, &stone, &noise, &mut rand);

                    for z in 0..16usize {
                        for x in 0..16usize {
                            let our_top = heights[z * 16 + x];
                            if our_top < 0 {
                                continue;
                            }
                            let our_top = our_top as usize;

                            let mut reference_top = None;
                            for y in (0..256).rev() {
                                if stored.state(x, y, z) >> 4 != 0 {
                                    reference_top = Some(y);
                                    break;
                                }
                            }
                            let Some(reference_top) = reference_top else {
                                continue;
                            };
                            // Only where both agree on the height, so this
                            // measures the surface pass and not the height
                            // field.
                            if reference_top != our_top {
                                continue;
                            }
                            let reference = stored.state(x, reference_top, z) >> 4;
                            if is_decoration(reference) {
                                continue;
                            }

                            compared += 1;
                            let entry = per_biome.entry(biomes[z * 16 + x]).or_insert((0, 0));
                            entry.0 += 1;
                            let ours = column.get_block(x, our_top, z).id();
                            if ours != reference {
                                if example.is_none() {
                                    example = Some((
                                        stored.x * 16 + x as i32,
                                        our_top,
                                        stored.z * 16 + z as i32,
                                        ours,
                                        reference,
                                    ));
                                }
                                mismatched += 1;
                                entry.1 += 1;
                            }
                        }
                    }
                }
            }
        }

        for (biome, (total, bad)) in &per_biome {
            if *bad > 50 {
                println!(
                    "biome {biome}: {bad} of {total} wrong ({:.0}%)",
                    100.0 * *bad as f64 / *total as f64
                );
            }
        }
        assert!(compared > 1000, "only {compared} columns compared");
        let percent = mismatched as f64 / compared as f64 * 100.0;
        assert!(
            percent < 3.0,
            "{mismatched} of {compared} surfaces differ ({percent:.1}%), \
             first (block x, y, block z, ours, reference) = {example:?}"
        );
    }

    #[test]
    fn deep_ground_below_sea_level_gets_gravel() {
        let mut column = ChunkColumn::empty(0, 0);
        // A column whose surface is 40 blocks under the sea.
        for y in 0..=20 {
            column.set_block(0, y, 0, BlockState::new(block::ids::STONE, 0));
        }
        for y in 21..63 {
            column.set_block(0, y, 0, BlockState::new(block::ids::WATER, 0));
        }
        let mut rand = JavaRandom::new(11);
        let biomes = [1i32; 256];
        let noise = [0.0f64; 256];
        apply(
            &mut column,
            &biomes,
            &noise,
            &SurfaceNoise::new(),
            &mut rand,
        );

        assert_eq!(
            column.get_block(0, 20, 0).id(),
            block::ids::GRAVEL,
            "far under the sea the surface layer is gravel"
        );
    }
}
