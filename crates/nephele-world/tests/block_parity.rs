//! Block-level parity harness against the structures-on reference world.
//!
//! Reads `Nephele-private/parity/blocks-random.bin.gz` â€” a gzipped dump of every
//! chunk the real 1.8.9 server saved (seed 12345, DEFAULT, structures **on**,
//! 625 chunks / 25x25 around spawn), produced by
//! `Nephele-private/oracle/tools/Export.java`. The format is documented at the
//! top of that file:
//!
//! ```text
//! char[8]  "NEPHBLK1"
//! int32    chunk count
//! per chunk:
//!   int32  chunkX
//!   int32  chunkZ
//!   byte[4096]   biome ids, index = z * 16 + x
//!   uint16[65536] block states, index = y * 256 + z * 16 + x, value = (id << 4) | meta
//! ```
//!
//! Byte order is mixed, and the comment at the top of `Export.java` claiming "all
//! little endian" is wrong about half of it: the chunk count and coordinates go
//! through `DataOutputStream.writeInt`, so they are **big**-endian, while the
//! block states are packed by hand one byte at a time and are **little**-endian.
//! The biomes are raw bytes either way.
//!
//! Only the inner `+-8` chunks are compared, counting from the fixture's own
//! 25x25 grid, not from 0,0 â€” the world spawns at a negative chunk position, so
//! the grid is not centred on the origin. The outer ring is skipped because
//! population writes into the chunk east and south of its owner, and the fixture
//! has no neighbours beyond its own edge.
//!
//! Run it with output:
//!
//! ```powershell
//! cargo test -p nephele-world --test block_parity -- --nocapture
//! ```
//!
//! The report groups every mismatch into one of three buckets **by block id**,
//! not by "what can our terrain stage produce". The earlier reference-state rule
//! was unreadable once population became partial: a tree that we misplace where
//! vanilla has stone is a *population* error, but the reference block there is
//! stone, so it was filed as terrain and inflated the regression guard while the
//! population bucket looked healthy.
//!
//! - **population** â€” a block only the decorate pass places: leaves, logs, ores,
//!   flowers, tall grass, mushrooms, crops, structures (mossy cobble, spawner,
//!   chest, rails, fences), the granite/diorite/andesite veins, and water or lava
//!   **above sea level**, i.e. a pool rather than an ocean.
//! - **shared** â€” blocks both stages place: dirt and gravel (vein material *and*
//!   terrain filler), sand, snow layer, ice, and water/lava at or below sea level.
//! - **terrain** â€” everything else, which is the height-field and cave guard.
//!
//! A mismatch is population if **either** side is a population block, so a
//! missing tree and a tree in the wrong place both land there.
//!
//! Mismatches are also split by whether the chunk's biome map agrees with vanilla.
//! The biome layer is a known open bug (AGENTS.md Â§6), and a wrong biome means a
//! wrong surface and a wrong height field, so the two have to be reported apart.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::PathBuf;

use flate2::read::GzDecoder;
use nephele_world::worldgen::VanillaGenerator;
use nephele_world::{blocks, ChunkColumn, TestWorld};

const SEED: i64 = 12345;
/// Chunks per axis the inner window keeps, `+-COMPARE_HALF`, from the fixture's
/// own centre: 25 - 2 * 4 = 17.
const COMPARE_HALF: i32 = 8;

const MAGIC: &[u8; 8] = b"NEPHBLK1";
const BIOMES: usize = 4096;
const STATES: usize = 65536;
/// The 16x16 biome map 1.8.9 actually stores; the export pads it to 4096.
const BIOME_CELLS: usize = 256;

/// Every block state `provide_chunk` can write.
///
/// Derived from the code that writes blocks in the stage â€” `terrain::primer`
/// (stone/water), `surface::apply` (bedrock, the per-biome top and filler pair,
/// ice at the waterline, the swamp lily), `surface::generate_biome_terrain`
/// (sandstone under sand), `mesa::MesaTerrain` (coloured clay bands) and the
/// cave/ravine carvers (lava at the bottom, flowing lava in ravines).
///
/// [`terrain_states_are_complete`] checks that our own output stays inside the
/// list, so it cannot silently go stale.
const PROVIDED_STATES: &[u16] = &[
    0,            // air (the density field leaves gaps, caves carve holes)
    1 << 4,       // stone
    2 << 4,       // grass
    3 << 4,       // dirt
    (3 << 4) | 1, // coarse dirt, the mesa cap
    7 << 4,       // bedrock
    9 << 4,       // water
    10 << 4,      // flowing lava (ravines)
    11 << 4,      // lava (caves)
    12 << 4,      // sand / red sand
    13 << 4,      // gravel
    24 << 4,      // sandstone
    78 << 4,      // snow layer (ice plains surface)
    79 << 4,      // ice
    110 << 4,     // mycelium
    111 << 4,     // water lily
    159 << 4,     // stained hardened clay (metas vary; see `is_provided`)
    172 << 4,     // hardened clay
];

fn is_provided(raw: u16) -> bool {
    let id = raw >> 4;
    if id == 159 {
        // All sixteen dye colours appear in the mesa band table.
        return true;
    }
    PROVIDED_STATES.contains(&raw)
}

/// Which bucket a mismatch belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    /// Only `provide_chunk` places this: the height field and cave guard.
    Terrain,
    /// Both stages place it.
    Shared,
    /// Only `populate` places this.
    Population,
}

/// Sea level, from AGENTS.md Â§6. Pools above it are population's; water and lava
/// at or below it could be the ocean, so they are shared and cannot be used to
/// inflate the population bucket.
const SEA_LEVEL: u16 = 63;

/// Blocks only vanilla's decorate (population) pass places.
///
/// Read off `BiomeDecorator`, the per-biome `decorate` overrides, `WorldGenMinable`
/// (ores and the stone-variant patches), `WorldGenLakes`, `WorldGenDungeons`,
/// `WorldGenMineshaft` and the village pieces.
fn is_population_id(id: u16) -> bool {
    matches!(
        id,
        // Trees and their parts.
        17 | 18 | 161 | 162
        // Plants: saplings, tall grass, dead bush, flowers, mushrooms, cactus,
        // reeds, pumpkins and stems, vines, lilies, crops, cocoa, double plants.
        | 6 | 31 | 32 | 37..=40 | 59 | 81 | 83 | 86 | 103..=106 | 111 | 115 | 127 | 141
        | 142 | 175
        // Ores, and the granite/diorite/andesite veins that are population-only.
        | 14..=16 | 21 | 56 | 73 | 74 | 129 | 153
        // Structures: dungeon shell/spawner/chest, mineshaft rails and planks,
        // village fences, the extreme-hills monster egg, obsidian and cobweb.
        | 4 | 5 | 30 | 48 | 49 | 52 | 54 | 66 | 85 | 97
        // Clay patches under water.
        | 82
    )
}

/// Blocks both stages place, so a block-level diff cannot name the culprit.
fn is_shared_id(id: u16) -> bool {
    matches!(
        id,
        // Dirt and gravel veins run through terrain's own dirt and gravel.
        3 | 13
        // Sand, the snow layer and ice from the surface sweep and the snow pass.
        | 12 | 78 | 79
        // Water and lava: an ocean is terrain's, a lake is population's.
        | 8..=11
    )
}

/// Population blocks that share an id with a terrain block: stone's granite,
/// diorite and andesite variants are only ever placed by the ore pass.
fn is_population_state(raw: u16) -> bool {
    let id = raw >> 4;
    is_population_id(id) || (id == 1 && matches!(raw & 0x0F, 1 | 3 | 5))
}

/// Bucket a mismatch at `y` by the blocks on both sides.
fn bucket(ours: u16, theirs: u16, y: usize) -> Stage {
    let above_sea = y > SEA_LEVEL as usize;
    let is_pool = |raw: u16| matches!(raw >> 4, 8..=11);

    if is_population_state(ours)
        || is_population_state(theirs)
        || (above_sea && (is_pool(ours) || is_pool(theirs)))
    {
        return Stage::Population;
    }
    let ids = [ours >> 4, theirs >> 4];
    if ids.iter().any(|id| is_shared_id(*id)) {
        return Stage::Shared;
    }
    Stage::Terrain
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Terrain => "terrain",
        Stage::Shared => "shared",
        Stage::Population => "population",
    }
}

/// How a *reference* block reads on its own, for the per-state table. A full
/// bucket needs both sides (see [`bucket`]), so this only labels the row.
fn reference_class(raw: u16) -> &'static str {
    if is_population_state(raw) {
        "population"
    } else if is_shared_id(raw >> 4) || raw >> 4 == 78 {
        "shared"
    } else {
        "terrain"
    }
}

/// Whether this block can sit on *top* of the terrain without being part of it:
/// everything population places, plus the snow layer its ice/snow sweep drops.
fn dresses_the_top(raw: u16) -> bool {
    is_population_state(raw) || raw >> 4 == 78
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Nephele-private/parity/blocks-random.bin.gz")
}

/// Generate the reference world's own span through the real two-stage pipeline:
/// `provide_chunk` for every chunk, then `populate` for whichever owner each
/// insertion unblocks, in generation order.
fn generate(seed: i64, min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> TestWorld {
    let mut generator = VanillaGenerator::new(seed);
    let mut world = TestWorld::new();
    for chunk_z in min_z..=max_z {
        for chunk_x in min_x..=max_x {
            world.insert(generator.provide_chunk(chunk_x, chunk_z));
            generator.populate(&mut world, chunk_x, chunk_z);
        }
    }
    world
}

fn read_i32(reader: &mut impl Read) -> io::Result<i32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_be_bytes(bytes))
}

fn open(path: &PathBuf) -> GzDecoder<BufReader<File>> {
    let file = File::open(path).expect("the export opens");
    let mut reader = GzDecoder::new(BufReader::with_capacity(1 << 20, file));
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).expect("magic");
    assert_eq!(&magic, MAGIC, "not a Nephele block export");
    reader
}

/// The span the fixture covers. One pass over the coordinates and a skip over
/// each payload; the file is decompressed twice, which costs about a second.
fn fixture_span(path: &PathBuf) -> (i32, i32, i32, i32, i32) {
    let mut reader = open(path);
    let count = read_i32(&mut reader).expect("chunk count");
    let mut skip = vec![0u8; BIOMES + STATES * 2];
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for _ in 0..count {
        let chunk_x = read_i32(&mut reader).expect("chunk x");
        let chunk_z = read_i32(&mut reader).expect("chunk z");
        reader.read_exact(&mut skip).expect("chunk payload");
        min_x = min_x.min(chunk_x);
        max_x = max_x.max(chunk_x);
        min_z = min_z.min(chunk_z);
        max_z = max_z.max(chunk_z);
    }
    (min_x, max_x, min_z, max_z, count)
}

/// One block where we disagree with vanilla.
#[derive(Debug, Clone, Copy)]
struct Mismatch {
    chunk_x: i32,
    chunk_z: i32,
    x: usize,
    y: usize,
    z: usize,
    ours: u16,
    theirs: u16,
}

impl Mismatch {
    fn world_x(&self) -> i32 {
        self.chunk_x * 16 + self.x as i32
    }

    fn world_z(&self) -> i32 {
        self.chunk_z * 16 + self.z as i32
    }
}

#[derive(Debug, Default)]
struct IdDiff {
    count: u64,
    /// What we have there instead, most common first.
    instead: BTreeMap<u16, u64>,
}

#[derive(Debug, Default)]
struct Report {
    chunks: usize,
    chunks_with_matching_biomes: usize,
    blocks: u64,
    mismatches: u64,
    /// Mismatches inside chunks whose biome map agrees with vanilla.
    mismatches_in_matching_chunks: u64,
    by_stage: BTreeMap<Stage, u64>,
    by_stage_exact: BTreeMap<Stage, u64>,
    /// Terrain-stage mismatches at or within `SURFACE_DEPTH` blocks of the
    /// reference's top block, versus deeper down (caves).
    terrain_surface: u64,
    terrain_buried: u64,
    biome_mismatches: u64,
    /// Columns whose top block is at the same height as vanilla's, counting only
    /// columns nothing population-placed sits on top of.
    columns: u64,
    columns_with_matching_height: u64,
    columns_dressed_by_population: u64,
    by_reference_state: BTreeMap<u16, IdDiff>,
    /// Same, restricted to chunks whose biome map agrees.
    by_reference_state_exact: BTreeMap<u16, IdDiff>,
    first: Option<Mismatch>,
    first_in_matching_chunk: Option<Mismatch>,
}

/// How close to the top a terrain-stage mismatch has to be to read as a surface
/// error rather than a cave.
const SURFACE_DEPTH: u16 = 4;

impl Report {
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        column: &ChunkColumn,
        biomes_match: bool,
        x: usize,
        y: usize,
        z: usize,
        reference_top: u16,
        theirs: u16,
    ) {
        let ours = column.get_block(x, y, z).raw();
        if ours == theirs {
            return;
        }
        self.mismatches += 1;
        let kind = bucket(ours, theirs, y);
        *self.by_stage.entry(kind).or_default() += 1;
        if kind == Stage::Terrain {
            if (y as u16) + SURFACE_DEPTH >= reference_top {
                self.terrain_surface += 1;
            } else {
                self.terrain_buried += 1;
            }
        }

        let bump = |map: &mut BTreeMap<u16, IdDiff>| {
            let entry = map.entry(theirs).or_default();
            entry.count += 1;
            *entry.instead.entry(ours).or_default() += 1;
        };
        bump(&mut self.by_reference_state);
        if biomes_match {
            self.mismatches_in_matching_chunks += 1;
            *self.by_stage_exact.entry(kind).or_default() += 1;
            bump(&mut self.by_reference_state_exact);
            if self.first_in_matching_chunk.is_none() {
                self.first_in_matching_chunk = Some(Mismatch {
                    chunk_x,
                    chunk_z,
                    x,
                    y,
                    z,
                    ours,
                    theirs,
                });
            }
        }
        if self.first.is_none() {
            self.first = Some(Mismatch {
                chunk_x,
                chunk_z,
                x,
                y,
                z,
                ours,
                theirs,
            });
        }
    }

    fn count(&self, stage: Stage) -> u64 {
        self.by_stage.get(&stage).copied().unwrap_or(0)
    }

    fn count_exact(&self, stage: Stage) -> u64 {
        self.by_stage_exact.get(&stage).copied().unwrap_or(0)
    }
}

fn state_name(raw: u16) -> String {
    let id = raw >> 4;
    let meta = raw & 0x0F;
    let name = blocks::name(id);
    if meta == 0 {
        format!("{id} {name}")
    } else {
        format!("{id}:{meta} {name}")
    }
}

fn describe(entry: &IdDiff) -> String {
    entry
        .instead
        .iter()
        .map(|(state, count)| format!("{} ({count})", state_name(*state)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn mismatches_are_bucketed_by_block_id() {
    let stone = 1 << 4;
    let leaves = 18 << 4;
    let dirt = 3 << 4;
    let water = 9 << 4;

    // A missing tree and a misplaced tree are both population.
    assert_eq!(bucket(stone, leaves, 70), Stage::Population);
    assert_eq!(bucket(leaves, stone, 70), Stage::Population);
    // Ores are population wherever they are.
    assert_eq!(bucket(stone, 16 << 4, 12), Stage::Population);
    // Dirt/gravel veins are shared: both stages place them.
    assert_eq!(bucket(stone, dirt, 12), Stage::Shared);
    // A pool above sea level is population's; an ocean at sea level is terrain's.
    assert_eq!(bucket(stone, water, 70), Stage::Population);
    assert_eq!(bucket(stone, water, 40), Stage::Shared);
    // Air against stone is the height-field and cave guard.
    assert_eq!(bucket(0, stone, 40), Stage::Terrain);
    // A stone variant vein is population-only, plain stone is not.
    assert_eq!(bucket(stone, (1 << 4) | 3, 12), Stage::Population);
}

#[test]
fn terrain_states_are_complete() {
    // The capability list above must cover everything the generator writes, or
    // the parity report would label our own terrain bugs as population blocks.
    let mut generator = VanillaGenerator::new(SEED);
    let mut seen = BTreeSet::new();
    for chunk_z in -12..=12 {
        for chunk_x in -12..=12 {
            let column = generator.provide_chunk(chunk_x, chunk_z);
            for (_, section) in column.iter_sections() {
                for raw in section.blocks() {
                    seen.insert(*raw);
                }
            }
        }
    }
    let unexpected: Vec<String> = seen
        .iter()
        .filter(|raw| !is_provided(**raw))
        .map(|raw| state_name(*raw))
        .collect();
    assert!(
        unexpected.is_empty(),
        "provide_chunk writes states missing from PROVIDED_STATES: {unexpected:?}"
    );
}

#[test]
fn terrain_matches_the_reference_except_where_vanilla_populates() {
    let path = fixture_path();
    if !path.is_file() {
        eprintln!(
            "skipped: no block export at {} (see oracle/tools/Export.java)",
            path.display()
        );
        return;
    }

    let (min_x, max_x, min_z, max_z, fixture_chunks) = fixture_span(&path);
    let centre_x = (min_x + max_x) / 2;
    let centre_z = (min_z + max_z) / 2;
    let window_x = (centre_x - COMPARE_HALF)..=(centre_x + COMPARE_HALF);
    let window_z = (centre_z - COMPARE_HALF)..=(centre_z + COMPARE_HALF);

    // Generate exactly what the fixture covers, so every population pass that
    // could write into the window has its 2x2 square available.
    let world = generate(SEED, min_x, max_x, min_z, max_z);

    let mut report = Report::default();
    let mut biomes = vec![0u8; BIOMES];
    let mut states = vec![0u8; STATES * 2];
    let mut reader = open(&path);
    let count = read_i32(&mut reader).expect("chunk count");

    for _ in 0..count {
        let chunk_x = read_i32(&mut reader).expect("chunk x");
        let chunk_z = read_i32(&mut reader).expect("chunk z");
        reader.read_exact(&mut biomes).expect("biomes");
        reader.read_exact(&mut states).expect("block states");

        // The outer ring has no populated neighbours in the fixture.
        if !window_x.contains(&chunk_x) || !window_z.contains(&chunk_z) {
            continue;
        }
        let column = world
            .chunk(chunk_x, chunk_z)
            .expect("we generate the whole fixture span");
        report.chunks += 1;

        let stored = column.biomes().expect("provide_chunk stores biomes");
        let mut chunk_biome_mismatches = 0usize;
        for index in 0..BIOME_CELLS {
            if stored[index] != biomes[index] {
                chunk_biome_mismatches += 1;
            }
        }
        report.biome_mismatches += chunk_biome_mismatches as u64;
        let biomes_match = chunk_biome_mismatches == 0;
        if biomes_match {
            report.chunks_with_matching_biomes += 1;
        }

        // The height field on its own, separate from everything carved into it.
        // The reference's top block is the top of any tree, snow or flower that
        // population dropped there, so only columns whose top block is terrain's
        // are comparable.
        let mut reference_top = [0u16; 256];
        let mut top_state = [0u16; 256];
        for z in 0..16usize {
            for x in 0..16usize {
                let mut top = 0u16;
                let mut state = 0u16;
                for y in (0..256usize).rev() {
                    let index = y * 256 + z * 16 + x;
                    let raw = u16::from_le_bytes([states[index * 2], states[index * 2 + 1]]);
                    if raw != 0 {
                        top = (y + 1) as u16;
                        state = raw;
                        break;
                    }
                }
                reference_top[z * 16 + x] = top;
                top_state[z * 16 + x] = state;
            }
        }
        for z in 0..16usize {
            for x in 0..16usize {
                if dresses_the_top(top_state[z * 16 + x]) {
                    // A tree, a plant or a snow layer sits on top here, so the
                    // height map is not what the reference reports.
                    report.columns_dressed_by_population += 1;
                    continue;
                }
                report.columns += 1;
                if column.height_map_at(x, z) == i32::from(reference_top[z * 16 + x]) {
                    report.columns_with_matching_height += 1;
                }
            }
        }

        for y in 0..256usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    let index = y * 256 + z * 16 + x;
                    let theirs = u16::from_le_bytes([states[index * 2], states[index * 2 + 1]]);
                    report.blocks += 1;
                    report.observe(
                        chunk_x,
                        chunk_z,
                        column,
                        biomes_match,
                        x,
                        y,
                        z,
                        reference_top[z * 16 + x],
                        theirs,
                    );
                }
            }
        }
    }

    print_report(
        &report,
        centre_x,
        centre_z,
        window_x.clone(),
        window_z.clone(),
    );

    let expected_chunks = ((COMPARE_HALF * 2 + 1) * (COMPARE_HALF * 2 + 1)) as usize;
    assert_eq!(
        report.chunks, expected_chunks,
        "the fixture must cover the whole comparison window ({fixture_chunks} chunks)"
    );
    assert!(
        report.chunks_with_matching_biomes > 0,
        "no chunk has a matching biome map, so nothing can be concluded"
    );

    // The acceptance test. Two claims:
    //
    // 1. Vanilla's population pass accounts for the overwhelming majority of the
    //    difference. It cannot be *all* of it: population and terrain place the
    //    same blocks sometimes (dirt and gravel veins, snow, lake water), and the
    //    height field itself is not exact yet (AGENTS.md Â§6).
    let population = report.count(Stage::Population);
    let shared = report.count(Stage::Shared);
    assert!(
        (population + shared) * 10 >= report.mismatches * 9,
        "population explains only {} of {} mismatches",
        population + shared,
        report.mismatches
    );
    assert!(population > 0, "population places nothing at all?");

    // 2. The blocks only the terrain stage can place are a regression guard, not a
    //    parity claim: at the time of writing they are 0.23% of all compared
    //    blocks and every one of them is an air/stone/grass column height that
    //    disagrees with the reference, i.e. the known height-field gap. Anything
    //    above 0.5% means the pipeline broke, not that M1b moved.
    let terrain = report.count(Stage::Terrain);
    assert!(
        terrain * 200 < report.blocks,
        "terrain-only mismatches are {terrain} of {} blocks ({:.3}%), over the 0.5% guard",
        report.blocks,
        100.0 * terrain as f64 / report.blocks as f64
    );
}

fn print_report(
    report: &Report,
    centre_x: i32,
    centre_z: i32,
    window_x: std::ops::RangeInclusive<i32>,
    window_z: std::ops::RangeInclusive<i32>,
) {
    let percent = 100.0 * report.mismatches as f64 / report.blocks.max(1) as f64;
    println!();
    println!("block parity against parity/blocks-random.bin.gz");
    println!(
        "  window: chunks x {}..={} z {}..={} (inner +-{} of a grid centred on {},{})",
        window_x.start(),
        window_x.end(),
        window_z.start(),
        window_z.end(),
        COMPARE_HALF,
        centre_x,
        centre_z
    );
    println!(
        "  {} chunks, {} blocks compared",
        report.chunks, report.blocks
    );
    println!(
        "  biome map agrees in {} of {} chunks; {} of {} biome cells differ ({:.2}%)",
        report.chunks_with_matching_biomes,
        report.chunks,
        report.biome_mismatches,
        report.chunks as u64 * BIOME_CELLS as u64,
        100.0 * report.biome_mismatches as f64 / (report.chunks.max(1) as f64 * BIOME_CELLS as f64)
    );
    println!(
        "  mismatched blocks: {} ({:.3}%)",
        report.mismatches, percent
    );
    println!(
        "  terrain-stage mismatches: {} near the surface, {} buried (caves)",
        report.terrain_surface, report.terrain_buried
    );
    println!(
        "  column heights: {} of {} comparable columns agree ({:.2}%); {} columns skipped",
        report.columns_with_matching_height,
        report.columns,
        100.0 * report.columns_with_matching_height as f64 / report.columns.max(1) as f64,
        report.columns_dressed_by_population
    );
    println!();
    println!(
        "  {:<12} {:>10} {:>8} {:>8}  in biome-matching chunks",
        "stage", "count", "% blocks", "% diffs"
    );
    for stage in [Stage::Population, Stage::Shared, Stage::Terrain] {
        let count = report.count(stage);
        println!(
            "  {:<12} {:>10} {:>7.3}% {:>7.1}%  {}",
            stage_name(stage),
            count,
            100.0 * count as f64 / report.blocks.max(1) as f64,
            100.0 * count as f64 / report.mismatches.max(1) as f64,
            report.count_exact(stage)
        );
    }

    println!();
    println!("mismatch counts by reference block state (all chunks):");
    println!(
        "  {:<30} {:>10}  {:<10} we have instead",
        "reference block", "count", "stage"
    );
    for (state, diff) in &report.by_reference_state {
        println!(
            "  {:<30} {:>10}  {:<10} {}",
            state_name(*state),
            diff.count,
            reference_class(*state),
            describe(diff)
        );
    }

    if report.by_reference_state_exact.is_empty() {
        println!("\nno mismatches in biome-matching chunks");
    } else {
        println!();
        println!("mismatch counts in biome-matching chunks only:");
        for (state, diff) in &report.by_reference_state_exact {
            println!(
                "  {:<30} {:>10}  {:<10} {}",
                state_name(*state),
                diff.count,
                reference_class(*state),
                describe(diff)
            );
        }
    }

    if let Some(first) = report.first {
        println!();
        print_mismatch("first difference", &first);
    }
    if let Some(first) = report.first_in_matching_chunk {
        print_mismatch("first difference in a biome-matching chunk", &first);
    }
}

fn print_mismatch(label: &str, first: &Mismatch) {
    println!(
        "{label}: chunk ({}, {}) block ({}, {}, {}) world ({}, {}, {})",
        first.chunk_x,
        first.chunk_z,
        first.x,
        first.y,
        first.z,
        first.world_x(),
        first.y,
        first.world_z()
    );
    println!("  ours:      {}", state_name(first.ours));
    println!("  reference: {}", state_name(first.theirs));
}
