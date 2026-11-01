//! Ore veins and the stone-variant patches, ported from `BiomeDecorator` and
//! `WorldGenMinable`.
//!
//! Vanilla runs this once per chunk during `populate`, from a `Random` seeded
//! only by the chunk coordinates. Veins are placed at a random column in the
//! chunk and then, inside `WorldGenMinable`, centred **eight blocks further
//! east and south** — so roughly half of every vein lands in the neighbouring
//! chunk. That is why a chunk's ore comes from nine chunks' worth of passes,
//! and why this module replays the eight neighbours as well.

use crate::block::{self, BlockState};
use crate::chunk::ChunkColumn;
use crate::noise::JavaRandom;

/// One entry of `generateOres`.
struct OreSpec {
    block: u16,
    meta: u8,
    size: u32,
    count: i32,
    min_height: i32,
    max_height: i32,
    /// Lapis uses the second helper, which halves the height spread twice.
    centred: bool,
}

const fn ore(block: u16, meta: u8, size: u32, count: i32, min: i32, max: i32) -> OreSpec {
    OreSpec {
        block,
        meta,
        size,
        count,
        min_height: min,
        max_height: max,
        centred: false,
    }
}

/// `ChunkProviderSettings.Factory` defaults, in `generateOres` order.
const ORES: [OreSpec; 11] = [
    ore(block::ids::DIRT, 0, 33, 10, 0, 256),
    ore(block::ids::GRAVEL, 0, 33, 8, 0, 256),
    ore(block::ids::STONE, 3, 33, 10, 0, 80), // diorite
    ore(block::ids::STONE, 1, 33, 10, 0, 80), // granite
    ore(block::ids::STONE, 5, 33, 10, 0, 80), // andesite
    ore(block::ids::COAL_ORE, 0, 17, 20, 0, 128),
    ore(block::ids::IRON_ORE, 0, 9, 20, 0, 64),
    ore(block::ids::GOLD_ORE, 0, 9, 2, 0, 32),
    ore(block::ids::REDSTONE_ORE, 0, 8, 8, 0, 16),
    ore(block::ids::DIAMOND_ORE, 0, 8, 1, 0, 16),
    OreSpec {
        block: block::ids::LAPIS_ORE,
        meta: 0,
        size: 7,
        count: 1,
        min_height: 16,
        max_height: 16,
        centred: true,
    },
];

/// `ChunkProviderGenerate.populate`'s seed for a chunk's decoration.
pub(crate) fn decoration_seed(chunk_x: i32, chunk_z: i32) -> i64 {
    i64::from(chunk_x)
        .wrapping_mul(341_873_128_712)
        .wrapping_add(i64::from(chunk_z).wrapping_mul(132_897_987_541))
}

/// Place every ore vein that can reach this chunk.
///
/// Runs the pass for the chunk and its eight neighbours, keeping only blocks
/// that land inside `column`, so the result does not depend on generation
/// order.
pub fn decorate(column: &mut ChunkColumn) {
    let chunk_x = column.x;
    let chunk_z = column.z;

    for offset_x in -1..=1 {
        for offset_z in -1..=1 {
            let source_x = chunk_x + offset_x;
            let source_z = chunk_z + offset_z;
            let mut rand = JavaRandom::new(decoration_seed(source_x, source_z));

            for spec in ORES {
                if spec.centred {
                    // `genStandardOre2`: two draws around the centre height.
                    for _ in 0..spec.count {
                        let x = rand.next_int_bound(16);
                        let y = rand.next_int_bound(spec.max_height)
                            + rand.next_int_bound(spec.max_height)
                            + spec.min_height
                            - spec.max_height;
                        let z = rand.next_int_bound(16);
                        vein(
                            column,
                            source_x * 16 + x,
                            y,
                            source_z * 16 + z,
                            spec.block,
                            spec.meta,
                            spec.size,
                            &mut rand,
                        );
                    }
                } else {
                    // `genStandardOre1`.
                    let (low, high) = if spec.max_height < spec.min_height {
                        (spec.max_height, spec.min_height)
                    } else {
                        (spec.min_height, spec.max_height)
                    };
                    for _ in 0..spec.count {
                        let x = rand.next_int_bound(16);
                        let y = rand.next_int_bound(high - low) + low;
                        let z = rand.next_int_bound(16);
                        vein(
                            column,
                            source_x * 16 + x,
                            y,
                            source_z * 16 + z,
                            spec.block,
                            spec.meta,
                            spec.size,
                            &mut rand,
                        );
                    }
                }
            }
        }
    }
}

/// `WorldGenMinable.generate`: a vein stretched along a random axis, carved as
/// a chain of ellipsoids, replacing stone only.
#[allow(clippy::too_many_arguments)]
fn vein(
    column: &mut ChunkColumn,
    position_x: i32,
    position_y: i32,
    position_z: i32,
    block_id: u16,
    meta: u8,
    size: u32,
    rand: &mut JavaRandom,
) {
    let angle = rand.next_float() * std::f32::consts::PI;
    let spread = size as f32 / 8.0;

    let start_x = (position_x + 8) as f32 + angle.sin() * spread;
    let end_x = (position_x + 8) as f32 - angle.sin() * spread;
    let start_z = (position_z + 8) as f32 + angle.cos() * spread;
    let end_z = (position_z + 8) as f32 - angle.cos() * spread;
    let start_y = (position_y + rand.next_int_bound(3) - 2) as f64;
    let end_y = (position_y + rand.next_int_bound(3) - 2) as f64;

    for step in 0..size {
        let progress = step as f32 / size as f32;

        let centre_x = start_x + (end_x - start_x) * progress;
        let centre_y = start_y + (end_y - start_y) * progress as f64;
        let centre_z = start_z + (end_z - start_z) * progress;

        let wobble = rand.next_double() * f64::from(size) / 16.0;
        let radius_y = f64::from((std::f32::consts::PI * progress).sin() + 1.0) * wobble + 1.0;
        let radius_x = radius_y;
        let radius_z = radius_y;

        let low_x = (f64::from(centre_x) - radius_x / 2.0).floor() as i32;
        let low_y = (centre_y - radius_y / 2.0).floor() as i32;
        let low_z = (f64::from(centre_z) - radius_z / 2.0).floor() as i32;
        let high_x = (f64::from(centre_x) + radius_x / 2.0).floor() as i32;
        let high_y = (centre_y + radius_y / 2.0).floor() as i32;
        let high_z = (f64::from(centre_z) + radius_z / 2.0).floor() as i32;

        for x in low_x..=high_x {
            let dx = (f64::from(x) + 0.5 - f64::from(centre_x)) / (radius_x / 2.0);
            if dx * dx >= 1.0 {
                continue;
            }
            for y in low_y..=high_y {
                let dy = (f64::from(y) + 0.5 - centre_y) / (radius_y / 2.0);
                if dx * dx + dy * dy >= 1.0 {
                    continue;
                }
                for z in low_z..=high_z {
                    let dz = (f64::from(z) + 0.5 - f64::from(centre_z)) / (radius_z / 2.0);
                    if dx * dx + dy * dy + dz * dz >= 1.0 {
                        continue;
                    }
                    if !(0..=255).contains(&y) {
                        continue;
                    }
                    // Veins from any of the nine source chunks may reach here;
                    // keep only the blocks that belong to this column.
                    if x.div_euclid(16) != column.x || z.div_euclid(16) != column.z {
                        continue;
                    }
                    let local_x = x.rem_euclid(16) as usize;
                    let local_z = z.rem_euclid(16) as usize;
                    let y = y as usize;

                    // Only stone is replaced, exactly as the generator's
                    // predicate requires.
                    if column.get_block(local_x, y, local_z).id() == block::ids::STONE {
                        column.set_block(local_x, y, local_z, BlockState::new(block_id, meta));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_column(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        let mut column = ChunkColumn::empty(chunk_x, chunk_z);
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..256 {
                    column.set_block(x, y, z, BlockState::new(block::ids::STONE, 0));
                }
            }
        }
        column
    }

    #[test]
    fn ore_placement_is_deterministic() {
        let mut a = stone_column(0, 0);
        let mut b = stone_column(0, 0);
        decorate(&mut a);
        decorate(&mut b);
        for x in 0..16 {
            for y in 0..256 {
                for z in 0..16 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
    }

    #[test]
    fn every_ore_appears_in_a_chunk_or_its_neighbours() {
        let mut column = stone_column(0, 0);
        decorate(&mut column);

        let mut counts = std::collections::BTreeMap::new();
        for x in 0..16 {
            for y in 0..256 {
                for z in 0..16 {
                    let id = column.get_block(x, y, z).id();
                    if id != block::ids::STONE {
                        *counts.entry(id).or_insert(0usize) += 1;
                    }
                }
            }
        }

        assert!(
            counts.len() >= 6,
            "expected several ore types, got {counts:?}"
        );
        assert!(counts.contains_key(&block::ids::COAL_ORE), "{counts:?}");
        assert!(counts.contains_key(&block::ids::IRON_ORE), "{counts:?}");
        // Diamond is rare, so only require that a wider area has it.
        let mut total = 0usize;
        for cx in 0..4 {
            for cz in 0..4 {
                let mut column = stone_column(cx, cz);
                decorate(&mut column);
                for x in 0..16 {
                    for y in 0..256 {
                        for z in 0..16 {
                            if column.get_block(x, y, z).id() == block::ids::DIAMOND_ORE {
                                total += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(total > 0, "no diamond ore in a 4x4 chunk area");
    }

    #[test]
    fn only_stone_is_replaced() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..256 {
                    column.set_block(x, y, z, BlockState::new(block::ids::DIRT, 0));
                }
            }
        }
        decorate(&mut column);
        for x in 0..16 {
            for y in 0..256 {
                for z in 0..16 {
                    assert_eq!(column.get_block(x, y, z).id(), block::ids::DIRT);
                }
            }
        }
    }
}
