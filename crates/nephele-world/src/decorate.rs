//! The overworld population pass: `ChunkProviderGenerate.populate`, its lakes
//! and dungeons, and the biome pre-pass that runs before `BiomeDecorator`.
//!
//! Reimplemented from the 1.8.9 classes named in each function's comment. The
//! caller supplies the population `Random` (seeded by
//! [`crate::worldgen::VanillaGenerator::population_seed`]); this module preserves
//! vanilla's draw order. That is why lakes and dungeons are here even though
//! their block placement is only part of the feature set: everything after them
//! in the decorator draws from the same stream.
//!
//! Deliberate approximations, all documented where they occur:
//! - dungeon chest *contents* are not generated (chests are placed, the loot
//!   draws are consumed, but no inventory exists);
//! - the lake rim's dirt→grass conversion and the freeze pass are skipped (both
//!   are light/temperature queries with no RNG draws);
//! - animal spawning is skipped: it consumes RNG but places no blocks, and the
//!   only later consumer (the snow pass) draws no random numbers.

use crate::block::{self, BlockState};
use crate::genworld::GenWorld;
use crate::noise::JavaRandom;

/// `ChunkProviderSettings.Factory` defaults that `populate` reads.
const WATER_LAKE_CHANCE: i32 = 4;
const LAVA_LAKE_CHANCE: i32 = 80;
const DUNGEON_CHANCE: i32 = 8;

/// `ChunkProviderGenerate.populate`: one owner chunk's population pass.
pub fn decorate<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    biome_id: i32,
    chunk_x: i32,
    chunk_z: i32,
) {
    let origin_x = chunk_x * 16;
    let origin_z = chunk_z * 16;

    // `MapGenStructure.generateStructure` calls are not ported. In vanilla they
    // consume `rand` and write pieces for chunks a village or mineshaft
    // intersects, so such a chunk diverges from here on.

    let is_desert = biome_id == 2 || biome_id == 17;
    if !is_desert && rand.next_int_bound(WATER_LAKE_CHANCE) == 0 {
        let x = origin_x + rand.next_int_bound(16) + 8;
        let y = rand.next_int_bound(256);
        let z = origin_z + rand.next_int_bound(16) + 8;
        lake(world, rand, x, y, z, BlockState::WATER);
    }

    if rand.next_int_bound(LAVA_LAKE_CHANCE / 10) == 0 {
        let x = origin_x + rand.next_int_bound(16) + 8;
        let inner = rand.next_int_bound(248);
        let y = rand.next_int_bound(inner + 8);
        let z = origin_z + rand.next_int_bound(16) + 8;
        if y < world.sea_level() || rand.next_int_bound(LAVA_LAKE_CHANCE / 8) == 0 {
            lake(world, rand, x, y, z, BlockState::LAVA);
        }
    }

    for _ in 0..DUNGEON_CHANCE {
        let x = origin_x + rand.next_int_bound(16) + 8;
        let y = rand.next_int_bound(256);
        let z = origin_z + rand.next_int_bound(16) + 8;
        dungeon(world, rand, x, y, z);
    }

    biome_pre_decorate(world, rand, biome_id, origin_x, origin_z);
    biome_decorator(world, rand, origin_x, origin_z);
}

/// `BiomeDecorator.decorate` → `genDecorations`. This is the ore half; the
/// surface features follow in later commits, in vanilla's order.
fn biome_decorator<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    origin_x: i32,
    origin_z: i32,
) {
    crate::ores::generate_ores(world, rand, origin_x, origin_z);
}

/// The per-biome `decorate` overrides that run **before** `super.decorate`.
///
/// Several biomes generate features before handing off to `BiomeDecorator`, and
/// those features draw from the same population `Random`, so they shift every
/// ore that follows. This pass reproduces the draws (and the blocks) for the
/// biomes ported so far: the taiga family and the savanna double-plant sweep.
/// Other overrides (plains, roofed forest, mega taiga blobs, snow spikes) are
/// still missing and keep their chunks' ore stream shifted; see AGENTS.md.
fn biome_pre_decorate<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    biome_id: i32,
    origin_x: i32,
    origin_z: i32,
) {
    // `BiomeGenTaiga.decorate` / `BiomeGenSavanna.decorate`: seven double-plant
    // attempts, each with three position draws and a generator call. Mutated
    // biomes go through `BiomeGenMutated.decorate`, which calls the decorator
    // directly and does **not** run this pre-pass.
    let variant: u8 = match biome_id {
        // FERN, for the whole taiga family.
        5 | 19 | 30 | 31 => 3,
        // GRASS.
        35 | 36 => 2,
        _ => return,
    };

    for _ in 0..7 {
        let x = origin_x + rand.next_int_bound(16) + 8;
        let z = origin_z + rand.next_int_bound(16) + 8;
        let height = world.height(x, z) + 32;
        if height <= 0 {
            continue;
        }
        let y = rand.next_int_bound(height);
        double_plant(world, rand, x, y, z, variant);
    }
}

/// `WorldGenDoublePlant.generate`: sixty-four attempts at a two-block plant.
///
/// The draw count (six per attempt) is what matters for later generation; the
/// placement follows vanilla's checks so the blocks appear too. The light test
/// in `BlockBush.canBlockStay` is not modelled (generation has no light volume).
fn double_plant<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    x: i32,
    y: i32,
    z: i32,
    variant: u8,
) {
    for _ in 0..64 {
        let dx = rand.next_int_bound(8) - rand.next_int_bound(8);
        let dy = rand.next_int_bound(4) - rand.next_int_bound(4);
        let dz = rand.next_int_bound(8) - rand.next_int_bound(8);
        let (px, py, pz) = (x + dx, y + dy, z + dz);
        if py >= 254 || !world.is_air(px, py, pz) || !world.is_air(px, py + 1, pz) {
            continue;
        }
        let ground = world.get_block(px, py - 1, pz).id();
        if !matches!(
            ground,
            block::ids::GRASS | block::ids::DIRT | block::ids::FARMLAND
        ) {
            continue;
        }
        world.set_block(
            px,
            py,
            pz,
            BlockState::new(block::ids::DOUBLE_PLANT, variant),
        );
        world.set_block(
            px,
            py + 1,
            pz,
            BlockState::new(block::ids::DOUBLE_PLANT, variant | 8),
        );
    }
}

/// `WorldGenLakes`: an ellipsoidal water or lava pocket, five blocks tall.
fn lake<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    mut x: i32,
    mut y: i32,
    mut z: i32,
    liquid: BlockState,
) -> bool {
    // `position = position.add(-8, 0, -8)`, then fall while the cell is air.
    x -= 8;
    z -= 8;
    while y > 5 && world.is_air(x, y, z) {
        y -= 1;
    }
    if y <= 4 {
        return false;
    }
    y -= 4;

    let mut cells = [false; 2048];
    let blobs = rand.next_int_bound(4) + 4;
    for _ in 0..blobs {
        let rx = rand.next_double() * 6.0 + 3.0;
        let ry = rand.next_double() * 4.0 + 2.0;
        let rz = rand.next_double() * 6.0 + 3.0;
        let cx = rand.next_double() * (16.0 - rx - 2.0) + 1.0 + rx / 2.0;
        let cy = rand.next_double() * (8.0 - ry - 4.0) + 2.0 + ry / 2.0;
        let cz = rand.next_double() * (16.0 - rz - 2.0) + 1.0 + rz / 2.0;

        for bx in 1..15usize {
            for bz in 1..15usize {
                for by in 1..7usize {
                    let dx = (bx as f64 - cx) / (rx / 2.0);
                    let dy = (by as f64 - cy) / (ry / 2.0);
                    let dz = (bz as f64 - cz) / (rz / 2.0);
                    if dx * dx + dy * dy + dz * dz < 1.0 {
                        cells[(bx * 16 + bz) * 8 + by] = true;
                    }
                }
            }
        }
    }

    // Reject the lake if its rim cuts into a liquid or into anything that is not
    // solid (and not the same liquid already there).
    for bx in 0..16usize {
        for bz in 0..16usize {
            for by in 0..8usize {
                let edge = !cells[(bx * 16 + bz) * 8 + by]
                    && (bx < 15 && cells[((bx + 1) * 16 + bz) * 8 + by]
                        || bx > 0 && cells[((bx - 1) * 16 + bz) * 8 + by]
                        || bz < 15 && cells[(bx * 16 + bz + 1) * 8 + by]
                        || bz > 0 && cells[(bx * 16 + (bz - 1)) * 8 + by]
                        || by < 7 && cells[(bx * 16 + bz) * 8 + by + 1]
                        || by > 0 && cells[(bx * 16 + bz) * 8 + (by - 1)]);
                if !edge {
                    continue;
                }
                let state = world.get_block(x + bx as i32, y + by as i32, z + bz as i32);
                if by >= 4 && block::is_liquid(state) {
                    return false;
                }
                if by < 4 && !block::is_solid(state) && state != liquid {
                    return false;
                }
            }
        }
    }

    for bx in 0..16usize {
        for bz in 0..16usize {
            for by in 0..8usize {
                if cells[(bx * 16 + bz) * 8 + by] {
                    let state = if by >= 4 { BlockState::AIR } else { liquid };
                    world.set_block(x + bx as i32, y + by as i32, z + bz as i32, state);
                }
            }
        }
    }

    // The lava variant lines its shell with stone, with a 50% roll per block.
    if liquid.id() == block::ids::LAVA {
        for bx in 0..16usize {
            for bz in 0..16usize {
                for by in 0..8usize {
                    let edge = !cells[(bx * 16 + bz) * 8 + by]
                        && (bx < 15 && cells[((bx + 1) * 16 + bz) * 8 + by]
                            || bx > 0 && cells[((bx - 1) * 16 + bz) * 8 + by]
                            || bz < 15 && cells[(bx * 16 + bz + 1) * 8 + by]
                            || bz > 0 && cells[(bx * 16 + (bz - 1)) * 8 + by]
                            || by < 7 && cells[(bx * 16 + bz) * 8 + by + 1]
                            || by > 0 && cells[(bx * 16 + bz) * 8 + (by - 1)]);
                    if edge
                        && (by < 4 || rand.next_int_bound(2) != 0)
                        && block::is_solid(world.get_block(
                            x + bx as i32,
                            y + by as i32,
                            z + bz as i32,
                        ))
                    {
                        world.set_block(
                            x + bx as i32,
                            y + by as i32,
                            z + bz as i32,
                            BlockState::STONE,
                        );
                    }
                }
            }
        }
    }

    true
}

/// `WorldGenDungeons`: a cobblestone room with a spawner and up to two chests.
fn dungeon<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) -> bool {
    let half_x = rand.next_int_bound(2) + 2;
    let half_z = rand.next_int_bound(2) + 2;
    let from_x = -half_x - 1;
    let to_x = half_x + 1;
    let from_z = -half_z - 1;
    let to_z = half_z + 1;

    let mut openings = 0;
    for dx in from_x..=to_x {
        for dy in -1..=4 {
            for dz in from_z..=to_z {
                let solid = world.is_solid(x + dx, y + dy, z + dz);
                if dy == -1 && !solid {
                    return false;
                }
                if dy == 4 && !solid {
                    return false;
                }
                let on_wall = (dx == from_x || dx == to_x || dz == from_z || dz == to_z) && dy == 0;
                if on_wall
                    && world.is_air(x + dx, y + dy, z + dz)
                    && world.is_air(x + dx, y + dy + 1, z + dz)
                {
                    openings += 1;
                }
            }
        }
    }

    if !(1..=5).contains(&openings) {
        return false;
    }

    for dx in from_x..=to_x {
        // Vanilla descends 3..=-1; a descending inclusive range is empty in
        // Rust, so walk it ascending and reverse instead.
        for dy in (-1..=3).rev() {
            for dz in from_z..=to_z {
                let interior =
                    dx != from_x && dy != -1 && dz != from_z && dx != to_x && dy != 4 && dz != to_z;
                if interior {
                    if world.get_block(x + dx, y + dy, z + dz).id() != block::ids::CHEST {
                        world.set_block(x + dx, y + dy, z + dz, BlockState::AIR);
                    }
                } else if y + dy >= 0 && !world.is_solid(x + dx, y + dy - 1, z + dz) {
                    world.set_block(x + dx, y + dy, z + dz, BlockState::AIR);
                } else if world.is_solid(x + dx, y + dy, z + dz)
                    && world.get_block(x + dx, y + dy, z + dz).id() != block::ids::CHEST
                {
                    if dy == -1 && rand.next_int_bound(4) != 0 {
                        world.set_block(
                            x + dx,
                            y + dy,
                            z + dz,
                            BlockState::new(block::ids::MOSSY_COBBLESTONE, 0),
                        );
                    } else {
                        world.set_block(x + dx, y + dy, z + dz, BlockState::COBBLESTONE);
                    }
                }
            }
        }
    }

    // Up to two chests against a wall.
    for _ in 0..2 {
        for _ in 0..3 {
            let cx = x + rand.next_int_bound(half_x * 2 + 1) - half_x;
            let cz = z + rand.next_int_bound(half_z * 2 + 1) - half_z;
            if world.is_air(cx, y, cz) {
                let mut walls = 0;
                for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                    if world.is_solid(nx, y, nz) {
                        walls += 1;
                    }
                }
                if walls == 1 {
                    world.set_block(cx, y, cz, BlockState::new(block::ids::CHEST, 0));
                    chest_loot_draws(rand);
                    break;
                }
            }
        }
    }

    world.set_block(x, y, z, BlockState::new(block::ids::MOB_SPAWNER, 0));
    // `pickMobSpawner`: SPAWNERTYPES has four entries.
    let _ = rand.next_int_bound(4);
    true
}

/// The RNG consumption of `WeightedRandomChestContent.generateChestContents`
/// with `WorldGenDungeons.CHESTCONTENT`: eight items, each one weight roll, one
/// stack-size roll and one slot roll.
///
/// `Items.enchanted_book.getRandom(rand)` is called once before the loot loop,
/// but its `addRandomEnchantment` returns immediately because a book's
/// enchantability is 0 — it draws nothing. No table entry can roll a stack
/// larger than its max stack size, so the split-into-singles branch is never
/// taken. The chest's contents are not tracked; only the draws affect later
/// generation.
fn chest_loot_draws(rand: &mut JavaRandom) {
    /// `(weight, min stack, max stack)`, in the table's order.
    const LOOT: [(i32, i32, i32); 16] = [
        (10, 1, 1), // saddle
        (10, 1, 4), // iron ingot
        (10, 1, 1), // bread
        (10, 1, 4), // wheat
        (10, 1, 4), // gunpowder
        (10, 1, 4), // string
        (10, 1, 1), // bucket
        (1, 1, 1),  // golden apple
        (10, 1, 4), // redstone
        (4, 1, 1),  // record 13
        (4, 1, 1),  // record cat
        (10, 1, 1), // name tag
        (2, 1, 1),  // golden horse armour
        (5, 1, 1),  // iron horse armour
        (1, 1, 1),  // diamond horse armour
        (1, 1, 1),  // enchanted book
    ];

    for _ in 0..8 {
        let roll = rand.next_int_bound(108);
        let mut remaining = roll;
        let mut chosen = LOOT[LOOT.len() - 1];
        for entry in LOOT {
            remaining -= entry.0;
            if remaining < 0 {
                chosen = entry;
                break;
            }
        }
        let (_, min, max) = chosen;
        let _ = rand.next_int_bound(max - min + 1);
        let _ = rand.next_int_bound(27);
    }
}
