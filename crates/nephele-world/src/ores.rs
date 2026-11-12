//! `BiomeDecorator.generateOres` and `WorldGenMinable`, on the population pass.
//!
//! Reimplemented from the 1.8.9 classes. The pass runs once per owner chunk from
//! the population `Random`, after the lake and dungeon draws, and it writes into
//! the owner chunk **and its neighbours**: `WorldGenMinable` centres every vein
//! eight blocks east and south of its chosen column, so roughly half of a vein
//! crosses the chunk border. Callers therefore pass a [`GenWorld`], not a single
//! column.
//!
//! The ore table is `ChunkProviderSettings.Factory`'s defaults, in
//! `generateOres` order.

use crate::block::{self, BlockState};
use crate::genworld::GenWorld;
use crate::math;
use crate::noise::JavaRandom;

/// One entry of `generateOres`.
struct OreSpec {
    block: u16,
    meta: u8,
    size: u32,
    count: i32,
    min_height: i32,
    max_height: i32,
    /// Lapis uses `genStandardOre2`, which spreads around a centre height.
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

/// The old single-column feature pass's seed. Kept because `features.rs` still
/// has its own tests; new code must use the population stream instead.
pub(crate) fn decoration_seed(chunk_x: i32, chunk_z: i32) -> i64 {
    i64::from(chunk_x)
        .wrapping_mul(341_873_128_712)
        .wrapping_add(i64::from(chunk_z).wrapping_mul(132_897_987_541))
}

/// `ChunkProviderSettings.Factory` defaults in `generateOres` order: dirt,
/// gravel, diorite, granite, andesite, coal, iron, gold, redstone, diamond,
/// lapis.
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

/// `BiomeDecorator.generateOres`, writing through `world`.
///
/// `origin_x`/`origin_z` are the owner chunk's block origin; `rand` is the
/// population stream, already advanced past the lake and dungeon draws.
pub fn generate_ores<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    origin_x: i32,
    origin_z: i32,
) {
    for spec in ORES.iter() {
        if spec.centred {
            // `genStandardOre2`.
            for _ in 0..spec.count {
                let x = origin_x + rand.next_int_bound(16);
                let y = rand.next_int_bound(spec.max_height)
                    + rand.next_int_bound(spec.max_height)
                    + spec.min_height
                    - spec.max_height;
                let z = origin_z + rand.next_int_bound(16);
                minable(
                    world,
                    rand,
                    x,
                    y,
                    z,
                    spec.size,
                    BlockState::new(spec.block, spec.meta),
                    block::ids::STONE,
                );
            }
        } else {
            // `genStandardOre1` normalises the bounds first.
            let (mut min, mut max) = (spec.min_height, spec.max_height);
            if max < min {
                std::mem::swap(&mut min, &mut max);
            } else if max == min {
                if min < 255 {
                    max += 1;
                } else {
                    min -= 1;
                }
            }
            for _ in 0..spec.count {
                let x = origin_x + rand.next_int_bound(16);
                let y = rand.next_int_bound(max - min) + min;
                let z = origin_z + rand.next_int_bound(16);
                minable(
                    world,
                    rand,
                    x,
                    y,
                    z,
                    spec.size,
                    BlockState::new(spec.block, spec.meta),
                    block::ids::STONE,
                );
            }
        }
    }
}

/// `WorldGenMinable.generate`: a vein stretched along a random axis, carved as a
/// chain of ellipsoids, replacing only blocks whose id is `replace`.
#[allow(clippy::too_many_arguments)]
pub fn minable<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    x: i32,
    y: i32,
    z: i32,
    size: u32,
    ore: BlockState,
    replace: u16,
) {
    // `MathHelper.sin/cos`, not the libm ones: vanilla quantises angles through
    // the same table.
    //
    // The float grouping matters. Vanilla writes
    // `MathHelper.sin(f) * (float)n / 8.0F`, which multiplies first and divides
    // after; pre-dividing into a `spread` rounds differently and moves the
    // ellipse enough to change which blocks a `floor` picks up.
    let angle = rand.next_float() * std::f32::consts::PI;
    let size_f = size as f32;
    let start_x = (x + 8) as f32 + math::sin(angle) * size_f / 8.0;
    let end_x = (x + 8) as f32 - math::sin(angle) * size_f / 8.0;
    let start_z = (z + 8) as f32 + math::cos(angle) * size_f / 8.0;
    let end_z = (z + 8) as f32 - math::cos(angle) * size_f / 8.0;
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
                        && world.get_block(block_x, block_y, block_z).id() == replace
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

    fn stone_world(chunks: i32) -> TestWorld {
        let mut world = TestWorld::new();
        for chunk_x in 0..chunks {
            for chunk_z in 0..chunks {
                let mut column = crate::ChunkColumn::empty(chunk_x, chunk_z);
                for x in 0..16usize {
                    for z in 0..16usize {
                        for y in 0..128usize {
                            column.set_block(x, y, z, BlockState::STONE);
                        }
                    }
                }
                world.insert(column);
            }
        }
        world
    }

    #[test]
    fn ores_replace_stone_only() {
        let mut world = TestWorld::new();
        let mut column = crate::ChunkColumn::empty(0, 0);
        for x in 0..16usize {
            for z in 0..16usize {
                for y in 0..64usize {
                    column.set_block(x, y, z, BlockState::DIRT);
                }
            }
        }
        world.insert(column);
        let mut rand = JavaRandom::new(12345);
        generate_ores(&mut world, &mut rand, 0, 0);
        for y in 0..64usize {
            for x in 0..16usize {
                for z in 0..16usize {
                    assert_eq!(
                        world.get_block(x as i32, y as i32, z as i32).id(),
                        block::ids::DIRT,
                        "{x},{y},{z}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_ore_appears_over_a_small_area() {
        let mut world = stone_world(4);
        let mut rand = JavaRandom::new(12345);
        for chunk_x in 0..4 {
            for chunk_z in 0..4 {
                generate_ores(&mut world, &mut rand, chunk_x * 16, chunk_z * 16);
            }
        }

        let mut found = std::collections::BTreeSet::new();
        for (_, column) in world.iter() {
            for y in 0..64usize {
                for x in 0..16usize {
                    for z in 0..16usize {
                        found.insert(column.get_block(x, y, z).id());
                    }
                }
            }
        }
        for id in [
            block::ids::COAL_ORE,
            block::ids::IRON_ORE,
            block::ids::GOLD_ORE,
            block::ids::REDSTONE_ORE,
            block::ids::DIAMOND_ORE,
            block::ids::LAPIS_ORE,
        ] {
            assert!(found.contains(&id), "missing ore id {id}: {found:?}");
        }
    }

    #[test]
    fn the_pass_is_deterministic_for_a_seed() {
        let mut first = stone_world(2);
        let mut second = stone_world(2);
        let mut a = JavaRandom::new(99);
        let mut b = JavaRandom::new(99);
        generate_ores(&mut first, &mut a, 0, 0);
        generate_ores(&mut second, &mut b, 0, 0);
        for chunk_x in 0..2 {
            for chunk_z in 0..2 {
                let a = first.chunk(chunk_x, chunk_z).unwrap();
                let b = second.chunk(chunk_x, chunk_z).unwrap();
                for y in 0..64usize {
                    for x in 0..16usize {
                        for z in 0..16usize {
                            assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                        }
                    }
                }
            }
        }
    }
}
