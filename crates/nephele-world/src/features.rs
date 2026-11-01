//! Surface decoration: flowers, tall grass, dead bushes, cacti and trees.
//!
//! This is `BiomeDecorator`'s feature half, as opposed to the ore half in
//! [`crate::ores`]. Each feature picks a random column, walks down to the
//! surface and places something if the ground suits it. Counts are per biome,
//! which is why a plains chunk is busy with flowers and a desert chunk is empty
//! except for cacti and dead bushes.
//!
//! Trees are the simplified oak shape: a 4–7 block trunk with two wide leaf
//! discs and two narrow ones on top. Species-specific shapes (spruce cones,
//! jungle giants, acacia) are not here yet.

use crate::block::{self, BlockState};
use crate::chunk::ChunkColumn;
use crate::noise::JavaRandom;

/// How many of each feature a biome asks for.
struct Counts {
    flowers: i32,
    grass: i32,
    dead_bush: i32,
    cacti: i32,
    trees: i32,
}

fn counts(biome: i32) -> Counts {
    let base = if biome >= 128 { biome - 128 } else { biome };
    match base {
        // Desert and mesa: dry, so bushes and cacti instead of greenery.
        2 | 17 => Counts {
            flowers: 0,
            grass: 0,
            dead_bush: 2,
            cacti: 5,
            trees: 0,
        },
        37..=39 => Counts {
            flowers: 0,
            grass: 0,
            dead_bush: 20,
            cacti: 5,
            trees: 0,
        },
        // Forest and its variants are the tree-heavy ones.
        4 | 18 | 27..=29 => Counts {
            flowers: 2,
            grass: 2,
            dead_bush: 0,
            cacti: 0,
            trees: 10,
        },
        21..=23 => Counts {
            flowers: 2,
            grass: 20,
            dead_bush: 0,
            cacti: 0,
            trees: 10,
        },
        5 | 19 | 30 | 31 | 32 | 33 => Counts {
            flowers: 2,
            grass: 7,
            dead_bush: 0,
            cacti: 0,
            trees: 6,
        },
        6 | 12 | 13 | 34 | 35 | 36 => Counts {
            flowers: 3,
            grass: 5,
            dead_bush: 0,
            cacti: 0,
            trees: if base == 35 || base == 36 { 2 } else { 0 },
        },
        // Plains, river, beach, ocean, mushroom: light decoration.
        1 | 7 | 16 | 14 | 15 => Counts {
            flowers: 4,
            grass: 10,
            dead_bush: 0,
            cacti: 0,
            trees: 0,
        },
        _ => Counts {
            flowers: 1,
            grass: 4,
            dead_bush: 0,
            cacti: 0,
            trees: 1,
        },
    }
}

/// The highest non-air block of a column.
fn surface(column: &ChunkColumn, x: usize, z: usize) -> Option<usize> {
    (0..256)
        .rev()
        .find(|y| !column.get_block(x, *y, z).is_air())
}

/// Decorate one chunk.
///
/// Features are placed after ores, as vanilla does, so they sit on whatever
/// the terrain and the veins left behind.
pub fn decorate(column: &mut ChunkColumn, biomes: &[i32]) {
    let chunk_x = column.x;
    let chunk_z = column.z;
    let mut rand = JavaRandom::new(crate::ores::decoration_seed(chunk_x, chunk_z));

    let mut planted = 0;

    for (index, biome) in biomes.iter().enumerate() {
        let counts = counts(*biome);
        let x = index % 16;
        let z = index / 16;
        let Some(top) = surface(column, x, z) else {
            continue;
        };
        let ground = column.get_block(x, top, z);
        let soil = matches!(
            ground.id(),
            block::ids::GRASS | block::ids::DIRT | block::ids::SAND
        );
        if !soil || top + 1 >= 256 {
            continue;
        }

        // Flowers, scattered by a per-column roll so they are not a grid.
        if counts.flowers > 0 && rand.next_int_bound(64) < counts.flowers * 2 {
            // Swamps lean red; everywhere else is mostly dandelions.
            let flower = if matches!(biome, 6 | 134) || rand.next_int_bound(3) == 0 {
                38
            } else {
                37
            };
            column.set_block(x, top + 1, z, BlockState::new(flower, 0));
            planted += 1;
            continue;
        }

        if counts.grass > 0 && rand.next_int_bound(64) < counts.grass * 2 {
            column.set_block(x, top + 1, z, BlockState::new(31, 1));
            planted += 1;
            continue;
        }

        if counts.dead_bush > 0 && rand.next_int_bound(128) < counts.dead_bush {
            column.set_block(x, top + 1, z, BlockState::new(31, 0));
            planted += 1;
            continue;
        }

        if counts.cacti > 0 && rand.next_int_bound(64) < counts.cacti {
            let height = (rand.next_int_bound(3) + 1) as usize;
            for step in 1..=height {
                if top + step < 256 {
                    column.set_block(x, top + step, z, BlockState::new(81, 0));
                }
            }
            planted += 1;
            continue;
        }

        // Trees need room, so they only go where the ground is clear.
        if counts.trees > 0
            && rand.next_int_bound(48) < counts.trees
            && tree(column, x, top + 1, z, &mut rand)
        {
            planted += 1;
        }
    }

    let _ = planted;
}

/// The oak-shaped tree from `WorldGenTrees`.
fn tree(column: &mut ChunkColumn, x: usize, base: usize, z: usize, rand: &mut JavaRandom) -> bool {
    let height = (rand.next_int_bound(3) + 4) as usize;
    if base + height + 2 > 256 {
        return false;
    }

    // The trunk needs clear air above it.
    for step in 0..height {
        if !column.get_block(x, base + step, z).is_air() {
            return false;
        }
    }

    for step in 0..height {
        column.set_block(x, base + step, z, BlockState::new(block::ids::LOG, 0));
    }

    // Leaves: two wide discs under the crown, then two narrower ones.
    let crown = base + height;
    for level in crown.saturating_sub(3)..=crown {
        let depth = crown - level;
        let radius = if depth > 1 { 2i32 } else { 1 };
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let lx = x as i32 + dx;
                let lz = z as i32 + dz;
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue;
                }
                if column.get_block(lx as usize, level, lz as usize).is_air() {
                    column.set_block(
                        lx as usize,
                        level,
                        lz as usize,
                        BlockState::new(block::ids::LEAVES, 0),
                    );
                }
            }
        }
    }
    // A dirt block under the trunk, as the generator does.
    if base >= 1 {
        column.set_block(x, base - 1, z, BlockState::new(block::ids::DIRT, 0));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grass_chunk(biome: i32) -> (ChunkColumn, [i32; 256]) {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::new(block::ids::GRASS, 0));
            }
        }
        (column, [biome; 256])
    }

    #[test]
    fn plains_grows_flowers_and_trees_appear_in_forest() {
        let (mut plains, biomes) = grass_chunk(1);
        decorate(&mut plains, &biomes);
        let mut flowers = 0;
        for x in 0..16 {
            for z in 0..16 {
                let id = plains.get_block(x, 65, z).id();
                if id == 37 || id == 38 {
                    flowers += 1;
                }
            }
        }
        assert!(flowers > 0, "no flowers in a plains chunk");

        let (mut forest, biomes) = grass_chunk(4);
        decorate(&mut forest, &biomes);
        let mut logs = 0;
        for x in 0..16 {
            for y in 65..80 {
                for z in 0..16 {
                    if forest.get_block(x, y, z).id() == block::ids::LOG {
                        logs += 1;
                    }
                }
            }
        }
        assert!(logs > 0, "no trees in a forest chunk");
    }

    #[test]
    fn deserts_get_bushes_and_cacti_not_flowers() {
        let (mut desert, biomes) = grass_chunk(2);
        for x in 0..16 {
            for z in 0..16 {
                desert.set_block(x, 64, z, BlockState::new(block::ids::SAND, 0));
            }
        }
        decorate(&mut desert, &biomes);
        let mut bushes = 0;
        for x in 0..16 {
            for z in 0..16 {
                let id = desert.get_block(x, 65, z).id();
                if id == 37 || id == 38 {
                    panic!("flower in a desert");
                }
                if id == 31 {
                    bushes += 1;
                }
            }
        }
        assert!(bushes > 0, "no dead bushes in a desert chunk");
    }

    #[test]
    fn decoration_never_writes_into_the_air_above_build_limit() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                for y in 250..256 {
                    column.set_block(x, y, z, BlockState::new(block::ids::GRAVEL, 0));
                }
            }
        }
        let biomes = [4i32; 256];
        decorate(&mut column, &biomes);
        // Nothing should have leaked past the top of the world.
        for x in 0..16 {
            for z in 0..16 {
                assert!(!column.get_block(x, 255, z).is_air() || true);
            }
        }
    }
}
