//! Block-level parity harness for the Nether, mirroring `block_parity.rs`.
//!
//! Reads `Nephele-private/parity/blocks-nether.bin.gz` — the block arrays a real
//! 1.8.9 server saved for `DIM-1` at seed 12345 (`generate-structures=false`, so
//! no nether fortresses). The file format is the same `NEPHBLK1` export that
//! `oracle/tools/Export.java` writes for the overworld:
//!
//! - `int32` chunk count and coordinates are **big** endian (`DataOutputStream`);
//! - the `uint16` block states are **little** endian (`(id << 4) | meta`),
//!   indexed `y * 256 + z * 16 + x`.
//!
//! What this can and cannot prove:
//!
//! - `ChunkProviderHell.provideChunk` is a pure function of the chunk
//!   coordinates, so terrain is comparable regardless of load order. That is
//!   the parity claim here.
//! - `ChunkProviderHell.populate` draws from the `hellRNG` left behind by the
//!   last `provideChunk`, so vanilla's own population depends on the order the
//!   server loaded chunks. We necessarily load them in our own order, so the
//!   *positions* of glowstone, quartz, fire, mushrooms and lava springs cannot
//!   match. They are reported as `population`, not counted as terrain failures.
//!
//! Run it with output:
//!
//! ```powershell
//! cargo test -p nephele-world --test nether_parity -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::PathBuf;

use flate2::read::GzDecoder;
use nephele_world::blocks;
use nephele_world::genworld::TestWorld;
use nephele_world::nether::NetherGenerator;

const SEED: i64 = 12345;
/// Inner window radius, in chunks. Two are lost to the fixture's own populated
/// ring; keep this small enough that every compared chunk has all four
/// neighbours.
const COMPARE_HALF: i32 = 6;

const MAGIC: &[u8; 8] = b"NEPHBLK1";
const BIOMES: usize = 4096;
const STATES: usize = 65536;

/// Which stage of `ChunkProviderHell` can put a reference block where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    /// `provideChunk` alone: netherrack, bedrock, the lava sea, soul sand and
    /// gravel from the surface pass, and cave air.
    Terrain,
    /// Both stages: lava (the sea is terrain; `WorldGenHellLava` also places it)
    /// and air (caves carve it, and fire/glowstone never *remove* terrain).
    Shared,
    /// Only `populate`: glowstone, quartz, fire and mushrooms.
    Population,
}

fn stage(raw: u16) -> Stage {
    match raw >> 4 {
        // `func_180515_a` / `func_180516_b` / `MapGenCavesHell`.
        0 | 7 | 13 | 87 | 88 => Stage::Terrain,
        // Lava sea vs `WorldGenHellLava`; flowing lava from its cascade.
        10 | 11 => Stage::Shared,
        // Glowstone, quartz ore, fire, brown mushroom, red mushroom.
        51 | 39 | 40 | 89 | 153 => Stage::Population,
        _ => Stage::Population,
    }
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Terrain => "terrain",
        Stage::Shared => "shared",
        Stage::Population => "population",
    }
}

/// Attribute a mismatch to the stage that could have caused **either** side to
/// differ. Our own population runs in a different load order than the server's,
/// so it lands blocks where the reference has terrain; looking only at the
/// reference state would blame those on the terrain pass.
fn classify(ours: u16, theirs: u16) -> Stage {
    let ours = stage(ours);
    let theirs = stage(theirs);
    if ours == Stage::Population || theirs == Stage::Population {
        Stage::Population
    } else if ours == Stage::Shared || theirs == Stage::Shared {
        Stage::Shared
    } else {
        Stage::Terrain
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Nephele-private/parity/blocks-nether.bin.gz")
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

#[derive(Debug, Default)]
struct IdDiff {
    count: u64,
    /// What we have there instead, most common first.
    instead: BTreeMap<u16, u64>,
}

#[derive(Debug, Default)]
struct Report {
    chunks: usize,
    blocks: u64,
    mismatches: u64,
    by_stage: BTreeMap<Stage, u64>,
    by_reference_state: BTreeMap<u16, IdDiff>,
    /// Terrain-only mismatches per world y, to separate the density field from
    /// the surface pass and the carvers.
    terrain_by_y: BTreeMap<usize, u64>,
    first: Option<(i32, i32, usize, usize, usize, u16, u16)>,
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

/// Diagnostic: print one FNV-1a hash per chunk so a maintainer can diff this
/// carver against the real `MapGenCavesHell`.
///
/// The Java side is `Nephele-private/oracle/hellprobe` (`HellCavesDump`), which
/// compiles the 1.8.9 class unchanged. It prints the same lines; the two lists
/// must be identical. Ignored because it needs that private tree.
#[ignore = "diagnostic: needs Nephele-private/oracle/hellprobe"]
#[test]
fn debug_carver_hashes() {
    let mut generator = NetherGenerator::new(SEED);
    for cz in -7..=5 {
        for cx in -7..=5 {
            let column = generator.provide_chunk(cx, cz);
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for index in 0..65536usize {
                let x = (index >> 12) & 15;
                let z = (index >> 8) & 15;
                let y = index & 255;
                hash ^= u64::from(column.get_block(x, y, z).id() & 0xFF);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            println!("{cx} {cz} {hash:x}");
        }
    }
}

#[test]
fn nether_terrain_matches_the_reference_except_where_vanilla_populates() {
    let path = fixture_path();
    if !path.is_file() {
        eprintln!(
            "skipped: no nether export at {} (see oracle/tools/Export.java)",
            path.display()
        );
        return;
    }

    let (min_x, max_x, min_z, max_z, fixture_chunks) = fixture_span(&path);
    let centre_x = (min_x + max_x) / 2;
    let centre_z = (min_z + max_z) / 2;
    let window_x = (centre_x - COMPARE_HALF)..=(centre_x + COMPARE_HALF);
    let window_z = (centre_z - COMPARE_HALF)..=(centre_z + COMPARE_HALF);

    // Generate the fixture's whole span, in raster order, so every compared
    // chunk has the 2x2 square its population pass needs.
    let mut generator = NetherGenerator::new(SEED);
    let mut world = TestWorld::new();
    for chunk_z in min_z..=max_z {
        for chunk_x in min_x..=max_x {
            world.insert(generator.provide_chunk(chunk_x, chunk_z));
            generator.populate(&mut world, chunk_x, chunk_z);
        }
    }

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

        if !window_x.contains(&chunk_x) || !window_z.contains(&chunk_z) {
            continue;
        }
        let column = world
            .chunk(chunk_x, chunk_z)
            .expect("we generate the whole fixture span");
        report.chunks += 1;

        // 1.8.9's nether height is 128; the export still pads to 256, and both
        // sides leave y >= 128 as air, so compare the real height only.
        for y in 0..128usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    let index = y * 256 + z * 16 + x;
                    let theirs = u16::from_le_bytes([states[index * 2], states[index * 2 + 1]]);
                    report.blocks += 1;
                    let ours = column.get_block(x, y, z).raw();
                    if ours == theirs {
                        continue;
                    }
                    report.mismatches += 1;
                    let kind = classify(ours, theirs);
                    *report.by_stage.entry(kind).or_default() += 1;
                    if kind == Stage::Terrain {
                        *report.terrain_by_y.entry(y).or_default() += 1;
                    }
                    let entry = report.by_reference_state.entry(theirs).or_default();
                    entry.count += 1;
                    *entry.instead.entry(ours).or_default() += 1;
                    if report.first.is_none() {
                        report.first = Some((chunk_x, chunk_z, x, y, z, ours, theirs));
                    }
                }
            }
        }
    }

    print_report(
        &report,
        centre_x,
        centre_z,
        &window_x,
        &window_z,
        fixture_chunks,
    );

    let expected_chunks = ((COMPARE_HALF * 2 + 1) * (COMPARE_HALF * 2 + 1)) as usize;
    assert_eq!(
        report.chunks, expected_chunks,
        "the fixture must cover the whole comparison window ({fixture_chunks} chunks)"
    );

    let population = report
        .by_stage
        .get(&Stage::Population)
        .copied()
        .unwrap_or(0);
    let shared = report.by_stage.get(&Stage::Shared).copied().unwrap_or(0);
    let terrain = report.by_stage.get(&Stage::Terrain).copied().unwrap_or(0);

    assert!(population > 0, "population places nothing at all?");
    // Terrain is the claim: the density field, surface pass and cave carver must
    // reproduce the server's terrain exactly, so this is a tight guard rather
    // than a percentage. At the time of writing it is 6 of 5,537,792 blocks;
    // anything over 0.05% means a terrain stage broke.
    assert!(
        terrain * 2000 < report.blocks,
        "terrain-only mismatches are {terrain} of {} blocks ({:.3}%), over the 0.05% guard",
        report.blocks,
        100.0 * terrain as f64 / report.blocks as f64
    );
    // The two together must still be the overwhelming majority of the diff.
    assert!(
        (population + shared) * 10 >= report.mismatches * 9,
        "population explains only {} of {} mismatches",
        population + shared,
        report.mismatches
    );
}

fn print_report(
    report: &Report,
    centre_x: i32,
    centre_z: i32,
    window_x: &std::ops::RangeInclusive<i32>,
    window_z: &std::ops::RangeInclusive<i32>,
    fixture_chunks: i32,
) {
    println!();
    println!("nether block parity against parity/blocks-nether.bin.gz");
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
        "  {} chunks, {} blocks compared ({fixture_chunks} chunks in the export)",
        report.chunks, report.blocks
    );
    println!(
        "  mismatched blocks: {} ({:.3}%)",
        report.mismatches,
        100.0 * report.mismatches as f64 / report.blocks.max(1) as f64
    );
    println!();
    println!(
        "  {:<12} {:>10} {:>8} {:>8}",
        "stage", "count", "% blocks", "% diffs"
    );
    for stage in [Stage::Population, Stage::Shared, Stage::Terrain] {
        let count = report.by_stage.get(&stage).copied().unwrap_or(0);
        println!(
            "  {:<12} {:>10} {:>7.3}% {:>7.1}%",
            stage_name(stage),
            count,
            100.0 * count as f64 / report.blocks.max(1) as f64,
            100.0 * count as f64 / report.mismatches.max(1) as f64
        );
    }

    println!();
    println!("terrain-only mismatches by y (only non-zero rows):");
    for (y, count) in &report.terrain_by_y {
        println!("  y={y:<3} {count}");
    }

    println!();
    println!("mismatch counts by reference block state:");
    println!(
        "  {:<30} {:>10}  {:<10} we have instead",
        "reference block", "count", "stage"
    );
    for (state, diff) in &report.by_reference_state {
        println!(
            "  {:<30} {:>10}  {:<10} {}",
            state_name(*state),
            diff.count,
            stage_name(stage(*state)),
            describe(diff)
        );
    }

    if let Some((chunk_x, chunk_z, x, y, z, ours, theirs)) = report.first {
        println!();
        println!(
            "first difference: chunk ({chunk_x}, {chunk_z}) block ({x}, {y}, {z}) world ({}, {y}, {})",
            chunk_x * 16 + x as i32,
            chunk_z * 16 + z as i32
        );
        println!("  ours:      {}", state_name(ours));
        println!("  reference: {}", state_name(theirs));
    }
}
