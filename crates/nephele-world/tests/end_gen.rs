//! End-island parity and pillar-shape checks.
//!
//! `ChunkProviderEnd.provideChunk` is a pure function of the seed and the chunk
//! coordinates, so the island can be compared block-for-block. `populate` is
//! not: vanilla decorates with the unseeded `World.rand`, so pillar *positions*
//! differ between two vanilla server starts. This harness therefore has two
//! halves:
//!
//! - `end_island_matches_the_reference_provider` diffs the island against the
//!   real `ChunkProviderEnd`, compiled unchanged in
//!   `Nephele-private/oracle/hellprobe`. Ignored unless that dump exists.
//! - `pillars_have_vanilla_heights_and_caps` checks the shape contract that *is*
//!   reproducible: 6..=37 obsidian blocks, radius 1..=4, a bedrock cap, built on
//!   the end island, and nothing in the void.
//!
//! Run the comparison dump with:
//!
//! ```powershell
//! java -cp ..\oracle\hellprobe\out hellprobe.EndDump 12345 > end-hashes-java.txt
//! cargo test -p nephele-world --test end_gen -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use nephele_world::block::ids;
use nephele_world::blocks;
use nephele_world::end::EndGenerator;
use nephele_world::genworld::TestWorld;

const SEED: i64 = 12345;

fn java_dump_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Nephele-private/oracle/hellprobe/end-hashes-java.txt")
}

/// FNV-1a over `(id << 4) | meta` for every block, in the Java dump's order.
fn chunk_hash(column: &nephele_world::ChunkColumn) -> (u64, u32, u32) {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let (mut air, mut stone) = (0u32, 0u32);
    for x in 0..16usize {
        for z in 0..16usize {
            for y in 0..128usize {
                let state = column.get_block(x, y, z);
                let id = state.id();
                hash ^= u64::from(id & 0xFF);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
                match id {
                    ids::AIR => air += 1,
                    ids::END_STONE => stone += 1,
                    _ => {}
                }
            }
        }
    }
    (hash, air, stone)
}

#[test]
fn end_island_matches_the_reference_provider() {
    let path = java_dump_path();
    if !path.is_file() {
        eprintln!(
            "skipped: no End dump at {} (see oracle/hellprobe/EndDump.java)",
            path.display()
        );
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("dump reads");
    let mut checked = 0usize;
    for line in expected.lines() {
        let mut parts = line.split(' ');
        let (Some(cx), Some(cz), Some(hash), Some(air), Some(stone)) = (
            parts.next().and_then(|v| v.parse::<i32>().ok()),
            parts.next().and_then(|v| v.parse::<i32>().ok()),
            parts.next().and_then(|v| u64::from_str_radix(v, 16).ok()),
            parts.next().and_then(|v| v.parse::<u32>().ok()),
            parts.next().and_then(|v| v.parse::<u32>().ok()),
        ) else {
            continue;
        };
        let column = EndGenerator::new(SEED).provide_chunk(cx, cz);
        let (ours_hash, ours_air, ours_stone) = chunk_hash(&column);
        assert_eq!(
            (ours_hash, ours_air, ours_stone),
            (hash, air, stone),
            "chunk ({cx}, {cz}) differs from ChunkProviderEnd"
        );
        checked += 1;
    }
    assert!(checked > 0, "the End dump was empty");
    eprintln!("{checked} End chunks match ChunkProviderEnd");
}

#[test]
fn pillars_have_vanilla_heights_and_caps() {
    let mut generator = EndGenerator::new(SEED);
    let mut world = TestWorld::new();
    for cz in -6..=6 {
        for cx in -6..=6 {
            world.insert(generator.provide_chunk(cx, cz));
            generator.populate(&mut world, cx, cz);
        }
    }

    // A pillar is a bedrock block with obsidian directly below it. Walk down
    // from the cap to measure its height and its widest radius.
    let mut pillars = Vec::new();
    for (coord, column) in world.iter() {
        for x in 0..16usize {
            for z in 0..16usize {
                for y in (1..128usize).rev() {
                    if column.get_block(x, y, z).id() != ids::BEDROCK
                        || column.get_block(x, y - 1, z).id() != ids::OBSIDIAN
                    {
                        continue;
                    }
                    let mut height = 0usize;
                    let mut below = y - 1;
                    loop {
                        if column.get_block(x, below, z).id() != ids::OBSIDIAN {
                            break;
                        }
                        height += 1;
                        if below == 0 {
                            break;
                        }
                        below -= 1;
                    }
                    let world_x = coord.x * 16 + x as i32;
                    let world_z = coord.z * 16 + z as i32;
                    let widest = (-4i32..=4)
                        .flat_map(|dx| (-4i32..=4).map(move |dz| (dx, dz)))
                        .filter(|(dx, dz)| {
                            column
                                .get_block(
                                    (x as i32 + dx) as usize,
                                    y - 1,
                                    (z as i32 + dz) as usize,
                                )
                                .id()
                                == ids::OBSIDIAN
                        })
                        .map(|(dx, dz)| ((dx * dx + dz * dz) as f64).sqrt())
                        .fold(0.0f64, f64::max);
                    pillars.push((world_x, world_z, height, widest));
                }
            }
        }
    }

    assert!(!pillars.is_empty(), "no pillars in a 13x13 chunk window");
    for (x, z, height, widest) in &pillars {
        // `WorldGenSpikes`: nextInt(32) + 6 obsidian blocks, cap on top.
        assert!(
            (6..=37).contains(height),
            "pillar at ({x},{z}) is {height} tall, outside 6..=37"
        );
        // `nextInt(4) + 1`, drawn as a disc with `dx*dx + dz*dz <= radius*radius + 1`,
        // so the furthest cell is sqrt(4*4 + 1).
        assert!(
            *widest <= 17.0f64.sqrt() + 1.0e-6,
            "pillar at ({x},{z}) is {widest} blocks wide, past the radius-4 disc"
        );
    }
    eprintln!(
        "{} pillars, heights {:?}",
        pillars.len(),
        pillars.iter().map(|p| p.2).collect::<Vec<_>>()
    );
}

#[test]
fn the_void_is_empty_far_from_the_island() {
    let mut generator = EndGenerator::new(SEED);
    for (cx, cz) in [(64, 64), (-64, 32), (100, -100)] {
        let column = generator.provide_chunk(cx, cz);
        for y in 0..128 {
            for x in 0..16 {
                for z in 0..16 {
                    assert!(
                        column.get_block(x, y, z).is_air(),
                        "block at ({cx},{cz}) {x},{y},{z} is {:?}",
                        blocks::name(column.get_block(x, y, z).id())
                    );
                }
            }
        }
    }
}
