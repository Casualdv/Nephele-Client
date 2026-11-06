//! The vanilla chunk pipeline, split the way vanilla splits it.
//!
//! 1.8.9 has two stages, and the split is load-bearing:
//!
//! - [`VanillaGenerator::provide_chunk`] — everything that is a pure function of
//!   the chunk coordinates: base terrain, biome surface, caves, ravines,
//!   structure-start discovery, skylight. Structure *starts* are cached here and
//!   placed later; `MapGenStructure.recursiveGenerate` never writes a block.
//! - [`VanillaGenerator::populate`] — the decoration pass, which reads and writes
//!   neighbouring chunks (ores, lakes, dungeons, flora, structure pieces,
//!   ice/snow) and is gated by a 2×2 ownership rule.
//!
//! Everything that used to be squashed into one per-chunk pass — ore veins and
//! surface features — lives in the second stage, which is why
//! [`crate::ores`] and [`crate::features`] are no longer called from here: their
//! seeds and their `+8` offsets are population-shaped and cannot be reproduced
//! per chunk. See AGENTS.md §6.

use std::collections::HashMap;

use crate::chunk::ChunkColumn;
use crate::genlayer::BiomeLayers;
use crate::genworld::GenWorld;
use crate::noise::JavaRandom;
use crate::surface::{self, SurfaceNoise};
use crate::terrain::TerrainNoise;
use crate::world::ChunkCoord;

/// The four owners a newly available chunk can unblock, in the order vanilla
/// checks them (`Chunk.populateChunk`).
const QUADRANTS: [(i32, i32); 4] = [(0, 0), (-1, 0), (0, -1), (-1, -1)];

/// Generates vanilla 1.8.9 terrain for a world seed.
pub struct VanillaGenerator {
    seed: i64,
    layers: BiomeLayers,
    terrain: TerrainNoise,
    surface: SurfaceNoise,
    caves: crate::caves::Caves,
    ravines: crate::ravines::Ravines,
    structures: StructureStarts,
}

impl VanillaGenerator {
    pub fn new(seed: i64) -> Self {
        Self {
            seed,
            layers: BiomeLayers::new(seed),
            terrain: TerrainNoise::new(seed),
            surface: SurfaceNoise::with_seed(seed),
            caves: crate::caves::Caves::new(seed),
            ravines: crate::ravines::Ravines::new(seed),
            structures: StructureStarts::default(),
        }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    /// `ChunkProviderGenerate.provideChunk`: one chunk, no neighbours.
    ///
    /// The order is vanilla's:
    ///
    /// 1. terrain (`setBlocksInChunk`) — stone above the density surface, water
    ///    below sea level.
    /// 2. the surface pass (`replaceBlocksForBiome`), which re-seeds `rand` from
    ///    the chunk coordinates: that stream is what every biome's
    ///    `genTerrainBlocks` draws from for bedrock depth, sandstone and the mesa
    ///    bands.
    /// 3. caves.
    /// 4. ravines.
    /// 5. structure-start discovery — cached, never placed.
    /// 6. skylight (`generateSkylightMap`).
    ///
    /// Nothing here touches another chunk, so the result does not depend on the
    /// order chunks are requested in. Ore veins, lakes, dungeons, flora and
    /// structure *pieces* are not here — they belong to [`Self::populate`].
    pub fn provide_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        // 1. Base terrain.
        let mut column = self.terrain.primer(&mut self.layers, chunk_x, chunk_z);

        // Vanilla reseeds its shared `Random` from the chunk coordinates before
        // the surface pass. Note the world seed does *not* enter this formula.
        let mut rand = JavaRandom::new(chunk_rand_seed(chunk_x, chunk_z));

        // 2. Biome surface.
        let biomes = self.layers.chunk_biomes(chunk_x, chunk_z);
        let stone_noise = self.terrain.stone_noise(chunk_x, chunk_z);
        surface::apply(&mut column, &biomes, &stone_noise, &self.surface, &mut rand);

        // Vanilla stores the per-block biome map on the chunk here; the cast to
        // `byte` is vanilla's too, so mutated biomes (>= 128) wrap.
        let stored: Vec<u8> = biomes.iter().map(|id| *id as u8).collect();
        column.set_biomes(&stored);

        // 3.-4. Carving after the surface, so a tunnel that breaks through the
        // grass exposes the dirt under it instead of relaying the surface.
        self.caves.carve(&mut column, &biomes);
        self.ravines.carve(&mut column, &biomes);

        // 5. Structure starts: cache only, no block writes.
        self.discover_structures(chunk_x, chunk_z);

        // 6. Skylight. `generateSkylightMap` also fills the sky-light nibbles; see
        // the height-map note on `ChunkColumn::generate_height_map` for why that
        // half waits for a light-opacity table. The height map is what population
        // reads back through `GenWorld::height`.
        column.generate_height_map();

        column
    }

    /// `Chunk.populateChunk`: run the population passes this chunk unblocks.
    ///
    /// Vanilla checks, in this order, the owners `(x, z)`, `(x-1, z)`,
    /// `(x, z-1)` and `(x-1, z-1)`, and runs an owner's pass only when the three
    /// chunks completing its 2×2 square — `(x+1, z)`, `(x, z+1)`, `(x+1, z+1)` —
    /// all exist (and the owner itself has not been populated yet).
    ///
    /// Those passes write across chunk borders (a feature placed at the owner's
    /// `+8` offset can land in the diagonal neighbour), so a shared border's
    /// final state depends on the order chunks are loaded in. Call this
    /// immediately after inserting a chunk, in generation order.
    ///
    /// Returns the owners whose pass ran.
    pub fn populate<W: GenWorld>(
        &mut self,
        world: &mut W,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Vec<ChunkCoord> {
        populate_owners(world, chunk_x, chunk_z, |world, owner_x, owner_z| {
            self.populate_owner(world, owner_x, owner_z);
        })
    }

    /// `ChunkProviderGenerate.populate`: one owner chunk's decoration pass.
    ///
    /// Cell order, and therefore how the shared `Random` is consumed, is fixed:
    /// structure pieces (mineshaft, village, stronghold, scattered feature,
    /// monument), water lake, lava lake, dungeons, `BiomeDecorator` (ores first),
    /// animal spawns, then the ice/snow sweep.
    pub fn populate_owner<W: GenWorld>(&mut self, world: &mut W, chunk_x: i32, chunk_z: i32) {
        // Vanilla reseeds one shared `Random` here, so a fresh instance seeded
        // the vanilla way is the same stream.
        let mut rand = JavaRandom::new(self.population_seed(chunk_x, chunk_z));

        // `world.getBiomeGenForCoords(blockpos.add(16, 0, 16))` — sampled at the
        // chunk's south-east corner, not its origin and not its centre.
        let biome = self.decorating_biome(chunk_x, chunk_z);

        self.decorate(world, &mut rand, biome, chunk_x, chunk_z);
    }

    /// The population seed for one owner chunk.
    ///
    /// Two `nextLong()` draws from the world seed, each pulled onto an odd number
    /// (`n / 2 * 2 + 1`, Java's truncating division), then
    /// `(x * first + z * second) ^ seed`. This is not the same seed as
    /// `provide_chunk`'s, and it is deliberately independent of load order.
    pub fn population_seed(&self, chunk_x: i32, chunk_z: i32) -> i64 {
        let mut rand = JavaRandom::new(self.seed);
        let first = rand
            .next_long()
            .wrapping_div(2)
            .wrapping_mul(2)
            .wrapping_add(1);
        let second = rand
            .next_long()
            .wrapping_div(2)
            .wrapping_mul(2)
            .wrapping_add(1);
        i64::from(chunk_x)
            .wrapping_mul(first)
            .wrapping_add(i64::from(chunk_z).wrapping_mul(second))
            ^ self.seed
    }

    /// The biome that decorates a chunk: `World.getBiomeGenForCoords` at
    /// `(chunkX * 16 + 16, chunkZ * 16 + 16)` — the first block of the chunk to
    /// the south-east, which is not the owner's own biome.
    pub fn decorating_biome(&mut self, chunk_x: i32, chunk_z: i32) -> i32 {
        self.layers
            .biomes(chunk_x * 16 + 16, chunk_z * 16 + 16, 1, 1)[0]
    }

    /// The decoration pass itself.
    ///
    /// None of it is ported yet: every stage below writes blocks, so an empty
    /// body is the only honest state. They are listed in vanilla's order because
    /// the order is what a port must preserve — the lakes and dungeons draw from
    /// the same `Random` as the biome decorator, so adding them later shifts every
    /// subsequent draw unless they are inserted exactly here.
    ///
    /// ```text
    /// for kind in STRUCTURE_KINDS      MapGenStructure.generateStructure
    /// water lake                       WorldGenLakes(water), skipped in deserts
    /// lava lake                        WorldGenLakes(lava)
    /// dungeons                         8 x WorldGenDungeons
    /// biome.decorate                   BiomeDecorator; ores first (see crate::ores)
    /// animal spawns                    SpawnerAnimals.performWorldGenSpawning
    /// ice / snow                       per-column freeze and snowfall
    /// ```
    fn decorate<W: GenWorld>(
        &mut self,
        _world: &mut W,
        _rand: &mut JavaRandom,
        _biome: i32,
        _chunk_x: i32,
        _chunk_z: i32,
    ) {
    }

    /// `provideChunk`'s five `MapGenStructure.generate` calls, in order.
    fn discover_structures(&mut self, chunk_x: i32, chunk_z: i32) {
        for kind in StructureKind::ORDER {
            self.structures.discover(kind, chunk_x, chunk_z);
        }
    }

    /// The starts discovered so far. Empty until structure generators are ported.
    pub fn structures(&self) -> &StructureStarts {
        &self.structures
    }

    /// The y a player's feet rest at on this column.
    pub fn height_at(&mut self, world_x: i32, world_z: i32) -> i32 {
        let chunk_x = world_x.div_euclid(16);
        let chunk_z = world_z.div_euclid(16);
        let column = self.provide_chunk(chunk_x, chunk_z);
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

/// `Chunk.populateChunk`'s eligibility test: the owner plus the three chunks that
/// complete its 2×2 square.
pub fn can_populate<W: GenWorld>(world: &W, chunk_x: i32, chunk_z: i32) -> bool {
    world.has_chunk(chunk_x, chunk_z)
        && world.has_chunk(chunk_x + 1, chunk_z)
        && world.has_chunk(chunk_x, chunk_z + 1)
        && world.has_chunk(chunk_x + 1, chunk_z + 1)
}

/// `Chunk.populateChunk`'s driver, shared by all three dimensions.
///
/// Owners are visited `(x,z)`, `(x-1,z)`, `(x,z-1)`, `(x-1,z-1)`; each runs once,
/// guarded by the 2×2 rule. `run` is the dimension's own decoration pass. The
/// caller is responsible for the provider-specific `Random` state: the nether
/// draws from whatever its last `provide_chunk` left behind, so this must be
/// called immediately after that chunk was generated.
pub fn populate_owners<W, F>(
    world: &mut W,
    chunk_x: i32,
    chunk_z: i32,
    mut run: F,
) -> Vec<ChunkCoord>
where
    W: GenWorld,
    F: FnMut(&mut W, i32, i32),
{
    let mut ran = Vec::with_capacity(1);
    for (offset_x, offset_z) in QUADRANTS {
        let (owner_x, owner_z) = (chunk_x + offset_x, chunk_z + offset_z);
        if world.is_terrain_populated(owner_x, owner_z) || !can_populate(world, owner_x, owner_z) {
            continue;
        }
        // `ChunkProviderServer.populate` marks the chunk before decorating it.
        world.set_terrain_populated(owner_x, owner_z, true);
        run(world, owner_x, owner_z);
        ran.push(ChunkCoord::new(owner_x, owner_z));
    }
    ran
}

/// `provideChunk`'s reseed: `(long)x * 341873128712L + (long)z * 132897987541L`.
///
/// No world seed: two different worlds have the same surface-random stream for a
/// given chunk. It is still a pure function of the coordinates, so a chunk looks
/// the same whatever order it is generated in.
fn chunk_rand_seed(chunk_x: i32, chunk_z: i32) -> i64 {
    i64::from(chunk_x)
        .wrapping_mul(341_873_128_712)
        .wrapping_add(i64::from(chunk_z).wrapping_mul(132_897_987_541))
}

/// The `MapGenStructure` subclasses, in `provideChunk`'s call order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    Mineshaft,
    Village,
    Stronghold,
    ScatteredFeature,
    OceanMonument,
}

impl StructureKind {
    /// The order vanilla calls them in, which matters because each draws from its
    /// own generator's `Random` in that sequence.
    pub const ORDER: [StructureKind; 5] = [
        StructureKind::Mineshaft,
        StructureKind::Village,
        StructureKind::Stronghold,
        StructureKind::ScatteredFeature,
        StructureKind::OceanMonument,
    ];
}

/// One cached `StructureStart`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureStart {
    pub kind: StructureKind,
    pub chunk_x: i32,
    pub chunk_z: i32,
    /// Number of components, `StructureStart.components.size()`.
    pub size: u32,
}

/// The `structureMap` a `MapGenStructure` keeps: starts discovered during
/// `provide_chunk`, placed during `populate`.
///
/// This is the seam the five structure generators plug into. `discover` is where
/// `canSpawnStructureAtCoords` and `getStructureStart` go; the cache itself is
/// already the shape `populate`'s `generateStructure` will read.
#[derive(Debug, Default)]
pub struct StructureStarts {
    starts: HashMap<ChunkCoord, Vec<StructureStart>>,
}

impl StructureStarts {
    /// `MapGenStructure.recursiveGenerate` for one generator.
    ///
    /// Vanilla: `rand.nextInt()` (discarded), then, if
    /// `canSpawnStructureAtCoords(chunkX, chunkZ)`, build the start and cache it.
    /// No generator is ported, so nothing can satisfy that test yet.
    pub fn discover(&mut self, _kind: StructureKind, _chunk_x: i32, _chunk_z: i32) {}

    /// Cache a start, as `getStructureStart` would.
    pub fn record(&mut self, start: StructureStart) {
        self.starts
            .entry(ChunkCoord::new(start.chunk_x, start.chunk_z))
            .or_default()
            .push(start);
    }

    pub fn at(&self, chunk_x: i32, chunk_z: i32) -> &[StructureStart] {
        self.starts
            .get(&ChunkCoord::new(chunk_x, chunk_z))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.starts.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }
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
    use crate::genworld::TestWorld;

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
                let column = generator.provide_chunk(stored.x, stored.z);

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
    fn the_same_chunk_is_generated_twice_identically() {
        let mut generator = VanillaGenerator::new(12345);
        let first = generator.provide_chunk(-3, 7);
        let second = generator.provide_chunk(-3, 7);
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
        let a = forward.provide_chunk(0, 0);
        let mut backward = VanillaGenerator::new(99);
        backward.provide_chunk(5, 5);
        backward.provide_chunk(-2, 1);
        let b = backward.provide_chunk(0, 0);
        assert!(same_blocks(&a, &b));
    }

    #[test]
    fn the_world_has_a_bedrock_floor_a_water_layer_and_a_surface() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.provide_chunk(-28, 4);

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
        let column = generator.provide_chunk(-19, -30);

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
        let column = generator.provide_chunk(2, 2);
        for (x, z) in [(0, 0), (7, 9), (15, 15)] {
            let expected = highest_solid(&column, x, z).unwrap() + 1;
            let actual = generator.height_at(2 * 16 + x as i32, 2 * 16 + z as i32);
            assert_eq!(actual, expected, "column {x},{z}");
        }
    }

    #[test]
    fn the_surface_of_land_is_a_surface_block() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.provide_chunk(0, 0);
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
                // A surface block, and nothing a feature placed: no logs, leaves,
                // flowers or ores can come out of `provide_chunk`.
                assert!(
                    matches!(
                        id,
                        ids::GRASS
                            | ids::DIRT
                            | ids::SAND
                            | ids::SANDSTONE
                            | ids::GRAVEL
                            | ids::STONE
                            | ids::WATER
                            | ids::WATER_LILY
                            | ids::ICE
                            | ids::CLAY
                            | ids::MYCELIUM
                            | ids::STAINED_HARDENED_CLAY
                            | ids::HARDENED_CLAY
                            | ids::RED_SANDSTONE
                    ),
                    "unexpected surface block {id} at {x},{top},{z}"
                );
            }
        }
        assert!(seen_grass, "expected some grass at the surface");
    }

    #[test]
    fn provide_chunk_places_nothing_from_the_population_pass() {
        let mut generator = VanillaGenerator::new(12345);
        // A chunk with land, a cave and an ocean column, so every stage runs.
        for (chunk_x, chunk_z) in [(0, 0), (-28, 4), (7, -3)] {
            let column = generator.provide_chunk(chunk_x, chunk_z);
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..256 {
                        let id = column.get_block(x, y, z).id();
                        // Ores, trees, flowers, grass and dead bushes are all
                        // population blocks. (The swamp's water lily is not: vanilla
                        // places it in `BiomeGenSwamp.genTerrainBlocks`.)
                        assert!(
                            !matches!(
                                id,
                                ids::COAL_ORE
                                    | ids::IRON_ORE
                                    | ids::GOLD_ORE
                                    | ids::DIAMOND_ORE
                                    | ids::REDSTONE_ORE
                                    | ids::LAPIS_ORE
                                    | ids::LOG
                                    | ids::LEAVES
                                    | 37
                                    | 38
                                    | 31
                                    | 32
                            ),
                            "population block {id} at {x},{y},{z} of chunk {chunk_x},{chunk_z}"
                        );
                    }
                }
            }
            assert!(
                generator.structures().is_empty(),
                "starts are cached, not placed"
            );
        }
    }

    #[test]
    fn the_height_map_is_one_above_the_top_block() {
        let mut generator = VanillaGenerator::new(12345);
        let column = generator.provide_chunk(0, 0);
        for x in 0..16usize {
            for z in 0..16usize {
                let top = column.height_at(x, z).map(|y| y + 1).unwrap_or(0) as i32;
                assert_eq!(column.height_map_at(x, z), top, "column {x},{z}");
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_terrain() {
        let mut a = VanillaGenerator::new(1);
        let mut b = VanillaGenerator::new(2);
        assert!(!same_blocks(&a.provide_chunk(0, 0), &b.provide_chunk(0, 0)));
    }

    #[test]
    fn chunk_rand_seeds_spread_out() {
        let mut seen = std::collections::HashSet::new();
        for cx in -4..4 {
            for cz in -4..4 {
                assert!(
                    seen.insert(chunk_rand_seed(cx, cz)),
                    "collision at {cx},{cz}"
                );
            }
        }
    }

    #[test]
    fn an_empty_column_has_no_height() {
        let column = ChunkColumn::empty(0, 0);
        assert_eq!(highest_solid(&column, 3, 3), None);
    }

    #[test]
    fn the_population_seed_is_the_vanilla_formula() {
        // Pinned values: a refactor that alters the two odd `nextLong()` draws, the
        // multiply-add or the XOR has to come here and say so.
        let generator = VanillaGenerator::new(12345);
        assert_eq!(generator.population_seed(0, 0), 12345);
        assert_eq!(generator.population_seed(-3, 7), 8_218_855_382_820_530_821);

        // Independent statement of the same formula, from a fresh stream.
        let mut rand = JavaRandom::new(12345);
        let first = rand.next_long() / 2 * 2 + 1;
        let second = rand.next_long() / 2 * 2 + 1;
        for (cx, cz) in [(0, 0), (-3, 7), (12, -12)] {
            let expected = (i64::from(cx)
                .wrapping_mul(first)
                .wrapping_add(i64::from(cz).wrapping_mul(second)))
                ^ 12345;
            assert_eq!(generator.population_seed(cx, cz), expected);
        }
    }

    #[test]
    fn the_odd_multiplier_is_always_odd() {
        for seed in [0i64, 1, -1, 12345, i64::MAX, i64::MIN + 1] {
            let mut rand = JavaRandom::new(seed);
            for _ in 0..8 {
                let drawn = rand.next_long() / 2 * 2 + 1;
                assert_eq!(drawn & 1, 1, "seed {seed}");
            }
        }
    }

    #[test]
    fn the_decorating_biome_is_the_south_east_chunk_corner() {
        let mut generator = VanillaGenerator::new(12345);
        // The sampled block belongs to the chunk to the south-east, so the answer
        // is that chunk's first column — not the owner's.
        for (cx, cz) in [(0, 0), (4, -7), (-12, 12)] {
            let expected = generator
                .biomes(cx + 1, cz + 1)
                .first()
                .copied()
                .expect("a biome map");
            assert_eq!(generator.decorating_biome(cx, cz), expected);
        }
        // And it is demonstrably not just the owner's own biome.
        let differing = (-6..6).any(|cx| {
            let owner = generator.biomes(cx, 3)[0];
            generator.decorating_biome(cx, 3) != owner
        });
        assert!(differing, "the sample is not the owner's biome");
    }

    /// Insert chunks one at a time, the way `ChunkProviderServer.loadChunk` does,
    /// and record which owners each insertion unblocked.
    fn load(
        world: &mut TestWorld,
        generator: &mut VanillaGenerator,
        x: i32,
        z: i32,
    ) -> Vec<ChunkCoord> {
        world.insert(generator.provide_chunk(x, z));
        generator.populate(world, x, z)
    }

    #[test]
    fn population_waits_for_the_whole_2x2_square() {
        let mut generator = VanillaGenerator::new(12345);
        let mut world = TestWorld::new();

        // A lone chunk cannot populate itself: it has no eastern or southern
        // neighbour, so its square is incomplete.
        assert!(load(&mut world, &mut generator, 0, 0).is_empty());
        assert!(!world.is_terrain_populated(0, 0));

        // Nor can the eastern or the southern neighbour: (0,0) is waiting on
        // (0,1) and (1,1).
        assert!(load(&mut world, &mut generator, 1, 0).is_empty());
        assert!(load(&mut world, &mut generator, 0, 1).is_empty());

        // (1,1) closes (0,0)'s square, and only (0,0)'s: every other candidate is
        // still one chunk short.
        assert_eq!(
            load(&mut world, &mut generator, 1, 1),
            vec![ChunkCoord::new(0, 0)]
        );
        assert!(world.is_terrain_populated(0, 0));
        assert!(!world.is_terrain_populated(1, 1));
        assert!(!world.is_terrain_populated(1, 0));
        assert!(!world.is_terrain_populated(0, 1));
    }

    #[test]
    fn one_insertion_can_unblock_all_four_owners_in_vanilla_order() {
        let mut generator = VanillaGenerator::new(12345);
        let mut world = TestWorld::new();

        // Pre-load everything each of the four owners needs except (0,0) itself:
        // (0,0)'s square needs (1,0), (0,1) and (1,1); (-1,0) also needs (-1,1);
        // (0,-1) also needs (1,-1); (-1,-1) is itself one of them. None of these
        // can populate anything on their own — they are each a chunk short.
        for (cx, cz) in [
            (1, 0),
            (0, 1),
            (1, 1),
            (-1, 1),
            (1, -1),
            (0, -1),
            (-1, 0),
            (-1, -1),
        ] {
            assert!(
                load(&mut world, &mut generator, cx, cz).is_empty(),
                "{cx},{cz}"
            );
        }

        assert_eq!(
            load(&mut world, &mut generator, 0, 0),
            vec![
                ChunkCoord::new(0, 0),
                ChunkCoord::new(-1, 0),
                ChunkCoord::new(0, -1),
                ChunkCoord::new(-1, -1)
            ]
        );
    }

    #[test]
    fn population_runs_once_per_owner() {
        let mut generator = VanillaGenerator::new(12345);
        let mut world = TestWorld::new();
        for cz in 0..3 {
            for cx in 0..3 {
                load(&mut world, &mut generator, cx, cz);
            }
        }

        // Re-announcing a chunk runs nothing: every owner in its 2x2 is done.
        assert!(load(&mut world, &mut generator, 2, 2).is_empty());

        // ... and the pass only ever ran once per chunk in the first place.
        let populated: Vec<_> = (0..3)
            .flat_map(|cz| (0..3).map(move |cx| (cx, cz)))
            .filter(|(cx, cz)| world.is_terrain_populated(*cx, *cz))
            .collect();
        assert_eq!(
            populated,
            vec![(0, 0), (1, 0), (0, 1), (1, 1)],
            "only owners with a complete square run"
        );
    }
}
