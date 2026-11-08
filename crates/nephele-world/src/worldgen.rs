//! The complete vanilla chunk pipeline: biome layers, then the density
//! height field, then the per-biome surface pass.
//!
//! Generation here is per chunk and independent of request order. That is
//! correct for `provideChunk` only: vanilla reseeds before surface generation,
//! so the surface pass genuinely does not depend on earlier chunks. It is not
//! correct for decoration — ores, lakes, dungeons, flora and structures live in
//! a separate population stage that shares blocks with neighbouring chunks and
//! cannot be produced one chunk at a time. See AGENTS.md section 6 before adding
//! any feature generator here.

use crate::chunk::ChunkColumn;
use crate::genlayer::BiomeLayers;
use crate::noise::JavaRandom;
use crate::surface::{self, SurfaceNoise};
use crate::terrain::TerrainNoise;

/// Generates vanilla 1.8.9 terrain for a world seed.
pub struct VanillaGenerator {
    seed: i64,
    layers: BiomeLayers,
    terrain: TerrainNoise,
    surface: SurfaceNoise,
}

impl VanillaGenerator {
    pub fn new(seed: i64) -> Self {
        Self {
            seed,
            layers: BiomeLayers::new(seed),
            terrain: TerrainNoise::new(seed),
            surface: SurfaceNoise::with_seed(seed),
        }
    }

    /// A whole chunk, ready to be inserted into a [`crate::World`].
    pub fn column(&mut self, chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        let mut column = self.terrain.primer(&mut self.layers, chunk_x, chunk_z);
        let biomes = self.layers.chunk_biomes(chunk_x, chunk_z);
        // `ChunkProviderGenerate` stores the 1:1 map as `Level.Biomes`; without
        // this the column carries no biome at all, every consumer falls back to
        // plains, and the whole GenLayer stack never reaches the screen.
        let biome_bytes: Vec<u8> = biomes.iter().map(|biome| *biome as u8).collect();
        column.set_biomes(&biome_bytes);
        let stone_noise = self.terrain.stone_noise(chunk_x, chunk_z);
        let mut rand = JavaRandom::new(chunk_seed(self.seed, chunk_x, chunk_z));
        surface::apply(&mut column, &biomes, &stone_noise, &self.surface, &mut rand);
        // Vanilla order: caves and ravines carve the terrain, then veins are
        // placed into what is left.
        crate::caves::Caves::new(self.seed).carve(&mut column, &biomes);
        crate::ravines::Ravines::new(self.seed).carve(&mut column, &biomes);
        crate::ores::decorate(&mut column);
        crate::features::decorate(&mut column, &biomes);
        column
    }

    /// A dry spawn point near the origin for a brand-new world.
    ///
    /// Vanilla's `WorldProvider.getRandomizedSpawnPoint` searches a small area
    /// for a column above sea level; this keeps that intent without the world
    /// provider. Chunks are generated once each, in ring order, so the result
    /// stays as close to `(0, 0)` as the terrain allows.
    pub fn spawn_position(&mut self) -> [i32; 3] {
        const SEARCH: i32 = 32;
        let mut current: Option<((i32, i32), ChunkColumn)> = None;
        for radius in 0..=SEARCH {
            for x in -radius..=radius {
                for z in -radius..=radius {
                    if radius > 0 && x.abs() != radius && z.abs() != radius {
                        continue;
                    }
                    let coord = (x.div_euclid(16), z.div_euclid(16));
                    if current.as_ref().map(|(c, _)| *c) != Some(coord) {
                        current = Some((coord, self.column(coord.0, coord.1)));
                    }
                    let Some((_, column)) = current.as_ref() else {
                        continue;
                    };
                    let local_x = x.rem_euclid(16) as usize;
                    let local_z = z.rem_euclid(16) as usize;
                    if let Some(top) = highest_solid(column, local_x, local_z) {
                        if top + 1 > crate::terrain::SEA_LEVEL {
                            return [x, top + 1, z];
                        }
                    }
                }
            }
        }
        [0, crate::terrain::SEA_LEVEL + 1, 0]
    }

    /// The y a player's feet rest at on this column.
    pub fn height_at(&mut self, world_x: i32, world_z: i32) -> i32 {
        let chunk_x = world_x.div_euclid(16);
        let chunk_z = world_z.div_euclid(16);
        let column = self.column(chunk_x, chunk_z);
        let x = world_x.rem_euclid(16) as usize;
        let z = world_z.rem_euclid(16) as usize;
        for y in (0..256).rev() {
            if !column.get_block(x, y, z).is_air() {
                return y as i32 + 1;
            }
        }
        0
    }

    /// The biome map for one chunk, as `Level.Biomes` stores it.
    pub fn biomes(&mut self, chunk_x: i32, chunk_z: i32) -> Vec<i32> {
        self.layers.chunk_biomes(chunk_x, chunk_z)
    }
}

/// A stable per-chunk seed, so a chunk always looks the same whatever order it
/// is generated in.
///
/// Plain XOR of two coordinates collides (1,3 against 3,1, for instance), so
/// the pair is hashed with a 64-bit finaliser instead.
fn chunk_seed(seed: i64, chunk_x: i32, chunk_z: i32) -> i64 {
    let mut value = (seed as u64) ^ ((u64::from(chunk_x as u32)) << 32) ^ u64::from(chunk_z as u32);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (value ^ (value >> 31)) as i64
}

/// The highest non-air block of a column, or `None` for an empty one.
pub fn highest_solid(column: &ChunkColumn, x: usize, z: usize) -> Option<i32> {
    for y in (0..256).rev() {
        if !column.get_block(x, y, z).is_air() {
            return Some(y as i32);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::ids;

    #[test]
    fn cave_volume_is_in_the_same_range_as_the_reference_world() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        if !dir.is_dir() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let mut theirs = 0usize;
        let mut ours = 0usize;
        let mut chunks = 0usize;

        let mut generator = VanillaGenerator::new(12345);
        let path = dir.join("r.-1.0.mca");
        let region = RegionFile::open(&path).expect("region opens");
        for lz in 0..32 {
            for lx in 0..32 {
                let Some(bytes) = region.chunk_nbt(lx, lz).ok().flatten() else {
                    continue;
                };
                let Ok(stored) = parse_chunk(&bytes) else {
                    continue;
                };
                chunks += 1;
                let column = generator.column(stored.x, stored.z);

                // Air well below the surface is what caves and ravines leave.
                for x in 0..16usize {
                    for z in 0..16usize {
                        for y in 1..56usize {
                            if column.get_block(x, y, z).is_air() {
                                ours += 1;
                            }
                            if stored.state(x, y, z) >> 4 == 0 {
                                theirs += 1;
                            }
                        }
                    }
                }
            }
        }

        let mine = ours as f64 / chunks as f64;
        let real = theirs as f64 / chunks as f64;
        println!("cave air below y=56: ours {mine:.0}/chunk, reference {real:.0}/chunk");
        assert!(chunks > 100);
        assert!(
            (mine - real).abs() < real * 0.15,
            "cave volume off: {mine:.0} vs {real:.0}"
        );
    }
    #[test]
    fn ore_density_matches_the_reference_world() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        if !dir.is_dir() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let tracked = [
            ids::COAL_ORE,
            ids::IRON_ORE,
            ids::GOLD_ORE,
            ids::REDSTONE_ORE,
            ids::DIAMOND_ORE,
            ids::LAPIS_ORE,
        ];
        let mut theirs = [0usize; 6];
        let mut ours = [0usize; 6];
        let mut chunks = 0usize;

        let mut generator = VanillaGenerator::new(12345);
        let path = dir.join("r.-1.0.mca");
        let region = RegionFile::open(&path).expect("region opens");
        for lz in 0..32 {
            for lx in 0..32 {
                let Some(bytes) = region.chunk_nbt(lx, lz).ok().flatten() else {
                    continue;
                };
                let Ok(stored) = parse_chunk(&bytes) else {
                    continue;
                };
                chunks += 1;

                let column = generator.column(stored.x, stored.z);
                for (index, block) in tracked.iter().enumerate() {
                    for x in 0..16usize {
                        for y in 0..256usize {
                            for z in 0..16usize {
                                if column.get_block(x, y, z).id() == *block {
                                    ours[index] += 1;
                                }
                                if stored.state(x, y, z) >> 4 == *block {
                                    theirs[index] += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        let names = ["coal", "iron", "gold", "redstone", "diamond", "lapis"];
        let mut failures = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let mine = ours[index] as f64 / chunks as f64;
            let real = theirs[index] as f64 / chunks as f64;
            println!("{name}: ours {mine:.1}/chunk, reference {real:.1}/chunk");

            // Ours still runs slightly high: vanilla's decoration pass places
            // a few veins the terrain pass never sees.
            if (mine - real).abs() > real * 0.2 {
                failures.push(format!("{name}: {mine:.1} vs {real:.1}"));
            }
        }
        assert!(chunks > 100, "only {chunks} chunks compared");
        assert!(failures.is_empty(), "ore density off: {failures:?}");
    }
    /// The parity suite proves the GenLayer map; this proves it reaches the
    /// chunk. `Level.Biomes` is what the renderer, the save writer and
    /// `biome_at` read, and generated columns used to leave it unset.
    #[test]
    fn generated_chunks_carry_the_biome_map() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.column(-28, 4);
        let expected = generator.biomes(-28, 4);
        let carried: Vec<i32> = column
            .biomes()
            .expect("generated chunks store Level.Biomes")
            .iter()
            .map(|biome| *biome as i32)
            .collect();
        assert_eq!(carried, expected);
        assert!(
            carried.iter().any(|biome| *biome != 1),
            "a whole chunk of plains means the map never landed"
        );
    }

    /// The same 160,000-cell window as the GenLayer test, but read back out of
    /// `ChunkColumn::biomes()` after a full `column()` pass — the exact bytes a
    /// renderer or a save would see.
    #[ignore = "diagnostic: generates all 625 reference chunks"]
    #[test]
    fn generated_chunk_biomes_match_the_reference_world() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        if !dir.is_dir() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let mut generator = VanillaGenerator::new(12345);
        let mut cells = 0usize;
        let mut bad = 0usize;
        let mut chunks = 0usize;
        let mut named: std::collections::BTreeMap<i32, (i32, i32)> =
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
                    if stored.biomes.len() != 256 {
                        continue;
                    }
                    chunks += 1;
                    let column = generator.column(stored.x, stored.z);
                    let carried = column.biomes().expect("generated biomes");
                    for (index, expected) in stored.biomes.iter().enumerate() {
                        cells += 1;
                        if carried[index] != *expected {
                            bad += 1;
                        }
                    }
                    named
                        .entry(stored.biomes[0] as i32)
                        .or_insert((stored.x * 16, stored.z * 16));
                }
            }
        }
        println!("{chunks} chunks, {cells} cells, {bad} mismatched");
        for (biome, (x, z)) in &named {
            println!("  block ({x},{z}) is biome {biome}");
        }
        assert!(chunks > 100, "only {chunks} chunks compared");
        assert_eq!(bad, 0, "{bad} of {cells} generated biome cells differ");
    }

    /// `local_loop` spawns a brand-new world at whatever this returns, so it has
    /// to be dry land in the column it names.
    #[test]
    fn spawn_position_is_on_dry_land() {
        for seed in [12345i64, 7] {
            let mut generator = VanillaGenerator::new(seed);
            let [x, y, z] = generator.spawn_position();
            assert!(
                y > crate::terrain::SEA_LEVEL,
                "seed {seed} spawns at y={y}, at or below sea level"
            );
            let column = generator.column(x.div_euclid(16), z.div_euclid(16));
            let top = highest_solid(
                &column,
                x.rem_euclid(16) as usize,
                z.rem_euclid(16) as usize,
            )
            .expect("a solid block under spawn");
            assert_eq!(top + 1, y, "spawn sits on the surface of its own column");
        }
    }

    #[test]
    fn the_same_chunk_is_generated_twice_identically() {
        let mut generator = VanillaGenerator::new(12345);
        let first = generator.column(-3, 7);
        let second = generator.column(-3, 7);
        for y in 0..256 {
            for x in 0..16 {
                for z in 0..16 {
                    assert_eq!(
                        first.get_block(x, y, z),
                        second.get_block(x, y, z),
                        "block {x},{y},{z}"
                    );
                }
            }
        }
    }

    fn same_blocks(a: &ChunkColumn, b: &ChunkColumn) -> bool {
        (0..16).all(|x| {
            (0..256).all(|y| {
                (0..16).all(|z| {
                    a.get_block(x as usize, y, z as usize) == b.get_block(x as usize, y, z as usize)
                })
            })
        })
    }

    #[test]
    fn chunks_do_not_depend_on_generation_order() {
        let mut forward = VanillaGenerator::new(99);
        let a = forward.column(0, 0);
        let mut backward = VanillaGenerator::new(99);
        backward.column(5, 5);
        backward.column(-2, 1);
        let b = backward.column(0, 0);
        assert!(same_blocks(&a, &b));
    }

    #[test]
    fn the_world_has_a_bedrock_floor_a_water_layer_and_a_surface() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.column(-28, 4);

        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(
                    column.get_block(x, 0, z).id(),
                    ids::BEDROCK,
                    "the bottom block is bedrock"
                );
                let top = highest_solid(&column, x, z).expect("a column with blocks");
                assert!(top > 1, "column {x},{z} is empty");
            }
        }

        // Somewhere in the chunk there is water, since this area has ocean.
        let mut water = 0;
        for y in 0..64 {
            for x in 0..16 {
                for z in 0..16 {
                    if column.get_block(x, y, z).id() == ids::WATER {
                        water += 1;
                    }
                }
            }
        }
        assert!(water > 0, "expected water below sea level");
    }

    #[test]
    fn a_mesa_chunk_is_made_of_coloured_clay() {
        // Seed 22 has mesa at this chunk; mesa is rare enough that picking a
        // seed by hand is cheaper than scanning inside the test.
        let mut generator = VanillaGenerator::new(22);
        let biomes = generator.biomes(-19, -30);
        let column = generator.column(-19, -30);

        let mut mesa_columns = 0;
        let mut banded = 0;
        let mut grass = 0;

        for (index, biome) in biomes.iter().enumerate() {
            if !matches!(biome, 37..=39) {
                continue;
            }
            mesa_columns += 1;
            let x = index % 16;
            let z = index / 16;

            let mut clay = 0;
            for y in (0..256usize).rev() {
                match column.get_block(x, y, z).id() {
                    ids::STAINED_HARDENED_CLAY | ids::HARDENED_CLAY => clay += 1,
                    ids::GRASS => grass += 1,
                    _ => {}
                }
                if clay > 0 {
                    break;
                }
            }
            if clay > 0 {
                banded += 1;
            }
        }

        assert!(mesa_columns > 0, "the chunk has no mesa columns");
        // Caves can break through the surface, so allow a few columns to be
        // punched open.
        assert!(
            banded * 10 >= mesa_columns * 8,
            "{banded} of {mesa_columns} mesa columns expose coloured clay"
        );
        assert_eq!(grass, 0, "mesa must not fall back to the grass surface");
    }
    #[test]
    fn height_at_agrees_with_the_generated_column() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.column(2, 2);
        for (x, z) in [(0, 0), (7, 9), (15, 15)] {
            let expected = highest_solid(&column, x, z).unwrap() + 1;
            let actual = generator.height_at(2 * 16 + x as i32, 2 * 16 + z as i32);
            assert_eq!(actual, expected, "column {x},{z}");
        }
    }

    #[test]
    fn the_surface_of_land_is_a_surface_block() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.column(0, 0);
        let mut seen_grass = false;
        for x in 0..16 {
            for z in 0..16 {
                let Some(top) = highest_solid(&column, x, z) else {
                    continue;
                };
                let id = column.get_block(x, top as usize, z).id();
                if id == ids::GRASS {
                    seen_grass = true;
                }
                assert!(
                    matches!(
                        id,
                        ids::LOG
                            | ids::LEAVES
                            | 31
                            | 37
                            | 38
                            | 81
                            | ids::GRASS
                            | ids::DIRT
                            | ids::SAND
                            | ids::GRAVEL
                            | ids::STONE
                            | ids::WATER
                            | ids::SNOW_LAYER
                            | ids::MYCELIUM
                    ),
                    "unexpected surface block {id} at {x},{top},{z}"
                );
            }
        }
        assert!(seen_grass, "expected some grass at the surface");
    }

    #[test]
    fn different_seeds_produce_different_terrain() {
        let mut a = VanillaGenerator::new(1);
        let mut b = VanillaGenerator::new(2);
        assert!(!same_blocks(&a.column(0, 0), &b.column(0, 0)));
    }

    #[test]
    fn chunk_seeds_spread_out() {
        let mut seen = std::collections::HashSet::new();
        for cx in -4..4 {
            for cz in -4..4 {
                assert!(seen.insert(chunk_seed(7, cx, cz)), "collision at {cx},{cz}");
            }
        }
    }

    #[test]
    fn an_empty_column_has_no_height() {
        let column = ChunkColumn::empty(0, 0);
        assert_eq!(highest_solid(&column, 3, 3), None);
    }
}
