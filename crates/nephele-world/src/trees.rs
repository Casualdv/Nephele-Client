//! The tree pass of `BiomeDecorator.genDecorations`.
//!
//! Reimplemented from `WorldGenTaiga1`, `WorldGenTaiga2` and `WorldGenTrees`, in
//! the order `genDecorations` calls them: ores, the three underwater patch
//! passes, then trees.
//!
//! Counting draws is the hard part, and the three generators differ:
//!
//! - `WorldGenTaiga1` draws three values on entry and never again.
//! - `WorldGenTaiga2` draws three, then two *more* only if the ground test
//!   passes, so a rejected spruce consumes three.
//! - `WorldGenTrees` draws one on entry and one per canopy **corner cell**, so
//!   its draw count depends on the canopy shape, which depends on the height it
//!   drew first.
//!
//! `genBigTreeChance` itself draws too, and the count differs by biome, so the
//! species choice is part of the stream and cannot be made from the height or
//! anything else.

use crate::block::{self, BlockState};
use crate::genworld::GenWorld;
use crate::noise::JavaRandom;

/// `BlockOldLog` / `BlockOldLeaf` variants.
const SPRUCE: u8 = 1;
const OAK: u8 = 0;

/// The `check_decay` flag every tree generator sets.
///
/// `WorldGenTaiga1`/`Taiga2`/`Trees` all place leaves with
/// `BlockLeaves.CHECK_DECAY = false`, i.e. variant `| 8`, and that is what we
/// write. Reading the fixture back gives a *mix*: roughly half its leaves carry
/// the bit and half do not, in the same proportion for oak and spruce. The bit is
/// cleared by `BlockLeaves.updateTick` the first time a leaf is randomly ticked
/// in a loaded chunk, so the split records how long each chunk sat in memory, not
/// what generation produced.
///
/// Generation parity therefore has to ignore it, and `block_parity.rs` masks the
/// bit for both sides. Do not "fix" this by writing the cleared state: that
/// matches the half that ticked and loses the half that did not.
const NO_CHECK_DECAY: u8 = 8;

fn log(variant: u8) -> BlockState {
    BlockState::new(block::ids::LOG, variant)
}

fn leaves(variant: u8) -> BlockState {
    BlockState::new(block::ids::LEAVES, variant | NO_CHECK_DECAY)
}

/// Which generator `BiomeGenBase.genBigTreeChance` picked for one tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeKind {
    Spruce1,
    Spruce2,
    Oak,
}

/// `BiomeGenBase.genBigTreeChance`, which draws from the shared stream.
///
/// `BiomeGenTaiga` rolls `nextInt(3)` and picks `WorldGenTaiga1` on zero.
/// `BiomeGenHills` rolls `nextInt(3)` first and only falls through to the base
/// implementation — a second `nextInt(10)` — on zero. `BiomeGenMutated`
/// delegates to its base biome, so a mutated taiga still rolls here.
///
/// `WorldGenBigTree` is not ported: the 1-in-10 hills fallback uses the oak
/// generator instead. That costs those few chunks their later trees (the draws
/// no longer line up) but leaves every other biome exact.
pub fn gen_big_tree_chance(rand: &mut JavaRandom, biome_id: i32) -> TreeKind {
    // `BiomeGenMutated.genBigTreeChance` delegates to its base biome, so a
    // mutated id has to be folded back before the species roll.
    let biome_id = if biome_id >= 128 {
        biome_id - 128
    } else {
        biome_id
    };
    match biome_id {
        // Taiga, taiga hills, cold taiga, cold taiga hills, mega taiga,
        // mega taiga hills — `field_150644_aH` is 0, 0, 2, 2, 1, 1, and none of
        // those values takes the boulder branch before the plants (that is a
        // `decorate` pre-pass, see `crate::decorate`).
        5 | 19 | 30 | 31 | 32 | 33 => {
            if rand.next_int_bound(3) == 0 {
                TreeKind::Spruce1
            } else {
                TreeKind::Spruce2
            }
        }
        // Extreme hills and extreme hills+ — `BiomeGenHills`.
        3 | 34 => {
            if rand.next_int_bound(3) > 0 {
                TreeKind::Spruce2
            } else {
                // `super.genBigTreeChance`: 1 in 10 asks for a big oak.
                let _big = rand.next_int_bound(10) == 0;
                TreeKind::Oak
            }
        }
        // `BiomeGenBase.genBigTreeChance`.
        _ => {
            let _big = rand.next_int_bound(10) == 0;
            TreeKind::Oak
        }
    }
}

/// `BiomeDecorator.treesPerChunk` for a biome.
///
/// The base decorator starts at zero and only a few biomes raise it. Values moved
/// into a subclass constructor are resolved here; `BiomeGenMutated` delegates to
/// its base biome, so `biome - 128` is the lookup key.
pub fn trees_per_chunk(biome_id: i32) -> i32 {
    let base = if biome_id >= 128 {
        biome_id - 128
    } else {
        biome_id
    };
    match base {
        // Forest and birch forest (birch extends forest), roofed forest.
        4 | 18 | 27 | 28 => 10,
        29 => 6,
        // Jungle and jungle hills; jungle edge is nearly bare.
        21 | 22 => 50,
        23 => 2,
        // Taiga family.
        5 | 19 | 30 | 31 | 32 | 33 => 10,
        // Extreme hills and extreme hills+.
        3 | 34 => 3,
        35 => 1,
        36 => 2,
        _ => 0,
    }
}

/// `func_150523_a`: what a tree may grow through.
///
/// Air, leaves, grass, dirt, both log blocks, saplings and vines. Note this is
/// *not* the same test `WorldGenTaiga2` uses, which admits only air and leaves.
fn can_grow_through(state: BlockState) -> bool {
    state.is_air()
        || state.id() == block::ids::LEAVES
        || state.id() == block::ids::GRASS
        || state.id() == block::ids::DIRT
        || state.id() == block::ids::LOG
        || state.id() == 162 // log2
        || state.id() == 6   // sapling
        || state.id() == 106 // vine
}

/// `Block.isFullBlock()`: a leaf may not overwrite a full cube.
fn is_full_block(state: BlockState) -> bool {
    block::is_solid(state) && !block::is_leaves(state)
}

/// `func_175921_a`: the block under a trunk becomes dirt.
fn set_dirt_under<W: GenWorld>(world: &mut W, x: i32, y: i32, z: i32) {
    if world.get_block(x, y, z).id() != block::ids::DIRT {
        world.set_block(x, y, z, BlockState::DIRT);
    }
}

/// Run one tree generator at `(x, y, z)`, where `y` is the block above the ground.
pub fn generate<W: GenWorld>(
    world: &mut W,
    rand: &mut JavaRandom,
    x: i32,
    y: i32,
    z: i32,
    kind: TreeKind,
) -> bool {
    match kind {
        TreeKind::Spruce1 => spruce1(world, rand, x, y, z),
        TreeKind::Spruce2 => spruce2(world, rand, x, y, z),
        TreeKind::Oak => oak(world, rand, x, y, z),
    }
}

/// `WorldGenTaiga1.generate`: the narrow spruce with a ragged trim.
fn spruce1<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) -> bool {
    let height = rand.next_int_bound(5) + 7;
    let trunk_stop = height - rand.next_int_bound(2) - 3;
    let _gap = height - trunk_stop;
    let wide = 1 + rand.next_int_bound(_gap + 1);

    if !(y > 0 && y + height < 256) {
        return false;
    }

    // Volume test: every cell the canopy could touch must be growable.
    for step in y..=(y + 1 + height) {
        let reach = if step - y < trunk_stop { 0 } else { wide };
        for px in (x - reach)..=(x + reach) {
            for pz in (z - reach)..=(z + reach) {
                if !(0..256).contains(&step) || !can_grow_through(world.get_block(px, step, pz)) {
                    return false;
                }
            }
        }
    }

    let ground = world.get_block(x, y - 1, z);
    if !matches!(ground.id(), block::ids::GRASS | block::ids::DIRT) || y >= 256 - height - 1 {
        return false;
    }

    set_dirt_under(world, x, y - 1, z);

    let mut radius = 0;
    let mut step = y + height;
    while step >= y + trunk_stop {
        for px in (x - radius)..=(x + radius) {
            let dx = px - x;
            for pz in (z - radius)..=(z + radius) {
                let dz = pz - z;
                if (dx.abs() != radius || dz.abs() != radius || radius <= 0)
                    && !is_full_block(world.get_block(px, step, pz))
                {
                    world.set_block(px, step, pz, leaves(SPRUCE));
                }
            }
        }
        if radius >= 1 && step == y + trunk_stop + 1 {
            radius -= 1;
        } else if radius < wide {
            radius += 1;
        }
        step -= 1;
    }

    for offset in 0..(height - 1) {
        let state = world.get_block(x, y + offset, z);
        if state.is_air() || state.id() == block::ids::LEAVES {
            world.set_block(x, y + offset, z, log(SPRUCE));
        }
    }
    true
}

/// `WorldGenTaiga2.generate`: the fuller spruce with a two-wide base.
fn spruce2<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) -> bool {
    let height = rand.next_int_bound(4) + 6;
    let trunk_stop = 1 + rand.next_int_bound(2);
    let gap = height - trunk_stop;
    let wide = 2 + rand.next_int_bound(2);

    if !(y > 0 && y + height < 256) {
        return false;
    }

    for step in y..=(y + 1 + height) {
        let reach = if step - y < trunk_stop { 0 } else { wide };
        for px in (x - reach)..=(x + reach) {
            for pz in (z - reach)..=(z + reach) {
                if !(0..256).contains(&step) {
                    return false;
                }
                let state = world.get_block(px, step, pz);
                // Only air and leaves — stricter than `can_grow_through`.
                if !state.is_air() && state.id() != block::ids::LEAVES {
                    return false;
                }
            }
        }
    }

    let ground = world.get_block(x, y - 1, z);
    if !matches!(
        ground.id(),
        block::ids::GRASS | block::ids::DIRT | block::ids::FARMLAND
    ) || y >= 256 - height - 1
    {
        return false;
    }

    set_dirt_under(world, x, y - 1, z);

    let mut radius = rand.next_int_bound(2);
    let mut cap = 1;
    let mut floor = 0;
    for level in 0..=gap {
        let step = y + height - level;
        for px in (x - radius)..=(x + radius) {
            let dx = px - x;
            for pz in (z - radius)..=(z + radius) {
                let dz = pz - z;
                if (dx.abs() != radius || dz.abs() != radius || radius <= 0)
                    && !is_full_block(world.get_block(px, step, pz))
                {
                    world.set_block(px, step, pz, leaves(SPRUCE));
                }
            }
        }
        if radius >= cap {
            radius = floor;
            floor = 1;
            cap += 1;
            if cap > wide {
                cap = wide;
            }
        } else {
            radius += 1;
        }
    }

    let bare = rand.next_int_bound(3);
    for offset in 0..(height - bare) {
        let state = world.get_block(x, y + offset, z);
        if state.is_air() || state.id() == block::ids::LEAVES {
            world.set_block(x, y + offset, z, log(SPRUCE));
        }
    }
    true
}

/// `WorldGenTrees.generate` with `WorldGenTrees(false)`: oak, no vines, no cocoa.
fn oak<W: GenWorld>(world: &mut W, rand: &mut JavaRandom, x: i32, y: i32, z: i32) -> bool {
    const MIN_HEIGHT: i32 = 4;
    let height = rand.next_int_bound(3) + MIN_HEIGHT;

    if !(y > 0 && y + height < 256) {
        return false;
    }

    for step in y..=(y + 1 + height) {
        let reach = if step == y {
            0
        } else if step >= y + 1 + height - 2 {
            2
        } else {
            1
        };
        for px in (x - reach)..=(x + reach) {
            for pz in (z - reach)..=(z + reach) {
                if !(0..256).contains(&step) || !can_grow_through(world.get_block(px, step, pz)) {
                    return false;
                }
            }
        }
    }

    let ground = world.get_block(x, y - 1, z);
    if !matches!(
        ground.id(),
        block::ids::GRASS | block::ids::DIRT | block::ids::FARMLAND
    ) || y >= 256 - height - 1
    {
        return false;
    }

    set_dirt_under(world, x, y - 1, z);

    // Canopy: from the top of the trunk down three levels.
    let top = y + height;
    for level in (y - 3 + height)..=top {
        let from_top = level - top; // 0 at the crown, -3 at the skirt
        let reach = 1 - from_top / 2; // 1 at the crown, 2 lower down
        for px in (x - reach)..=(x + reach) {
            let dx = px - x;
            for pz in (z - reach)..=(z + reach) {
                let dz = pz - z;
                // The `rand.nextInt(2)` is short-circuited away except on the
                // corner cells, and it is drawn before the `from_top != 0` test.
                let corner = dx.abs() == reach && dz.abs() == reach;
                let place = if corner {
                    !(rand.next_int_bound(2) != 0 && from_top != 0)
                } else {
                    true
                };
                if !place {
                    continue;
                }
                let state = world.get_block(px, level, pz);
                if state.is_air() || state.id() == block::ids::LEAVES || state.id() == 106
                // vine
                {
                    world.set_block(px, level, pz, leaves(OAK));
                }
            }
        }
    }

    for offset in 0..height {
        let state = world.get_block(x, y + offset, z);
        if state.is_air() || state.id() == block::ids::LEAVES || state.id() == 106 {
            world.set_block(x, y + offset, z, log(OAK));
        }
    }
    true
}
