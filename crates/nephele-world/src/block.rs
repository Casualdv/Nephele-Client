use std::fmt;

/// A 1.8.9 block state: `id << 4 | meta`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockState(pub u16);

impl BlockState {
    pub const AIR: BlockState = BlockState(0);

    pub const fn new(id: u16, meta: u8) -> Self {
        BlockState((id << 4) | (meta as u16 & 0x0F))
    }

    pub const fn from_raw(raw: u16) -> Self {
        BlockState(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    pub const fn id(self) -> u16 {
        self.0 >> 4
    }

    pub const fn meta(self) -> u8 {
        (self.0 & 0x0F) as u8
    }

    pub const fn is_air(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.id(), self.meta())
    }
}

/// The handful of 1.8.9 block ids Nephele needs by name today.
pub mod ids {
    pub const AIR: u16 = 0;
    pub const STONE: u16 = 1;
    pub const GRASS: u16 = 2;
    pub const DIRT: u16 = 3;
    pub const COBBLESTONE: u16 = 4;
    pub const PLANKS: u16 = 5;
    pub const BEDROCK: u16 = 7;
    pub const WATER: u16 = 9;
    pub const FLOWING_LAVA: u16 = 10;
    pub const LAVA: u16 = 11;
    pub const SAND: u16 = 12;
    pub const GRAVEL: u16 = 13;
    pub const LOG: u16 = 17;
    pub const LEAVES: u16 = 18;
    pub const GLASS: u16 = 20;
    pub const SANDSTONE: u16 = 24;
    pub const SNOW_LAYER: u16 = 78;
    pub const ICE: u16 = 79;
    pub const PACKED_ICE: u16 = 174;
    pub const SLIME: u16 = 165;
    pub const FARMLAND: u16 = 60;
    pub const CLAY: u16 = 82;
    pub const MYCELIUM: u16 = 110;
    pub const WATER_LILY: u16 = 111;
    pub const STAINED_HARDENED_CLAY: u16 = 159;
    pub const HARDENED_CLAY: u16 = 172;
    pub const RED_SANDSTONE: u16 = 179;
    pub const GOLD_ORE: u16 = 14;
    pub const IRON_ORE: u16 = 15;
    pub const COAL_ORE: u16 = 16;
    pub const LAPIS_ORE: u16 = 21;
    pub const DIAMOND_ORE: u16 = 56;
    pub const REDSTONE_ORE: u16 = 73;
    // The Nether and the End.
    pub const BROWN_MUSHROOM: u16 = 39;
    pub const RED_MUSHROOM: u16 = 40;
    pub const OBSIDIAN: u16 = 49;
    pub const FIRE: u16 = 51;
    pub const NETHERRACK: u16 = 87;
    pub const SOUL_SAND: u16 = 88;
    pub const GLOWSTONE: u16 = 89;
    pub const END_STONE: u16 = 121;
    pub const QUARTZ_ORE: u16 = 153;
    // Decoration and population-pass blocks.
    pub const MOSSY_COBBLESTONE: u16 = 48;
    pub const MOB_SPAWNER: u16 = 52;
    pub const CHEST: u16 = 54;
    pub const DOUBLE_PLANT: u16 = 175;
}

impl BlockState {
    pub const STONE: BlockState = BlockState::new(ids::STONE, 0);
    pub const GRASS: BlockState = BlockState::new(ids::GRASS, 0);
    pub const DIRT: BlockState = BlockState::new(ids::DIRT, 0);
    pub const BEDROCK: BlockState = BlockState::new(ids::BEDROCK, 0);
    pub const LOG: BlockState = BlockState::new(ids::LOG, 0);
    pub const LEAVES: BlockState = BlockState::new(ids::LEAVES, 0);
    pub const SAND: BlockState = BlockState::new(ids::SAND, 0);
    pub const WATER: BlockState = BlockState::new(ids::WATER, 0);
    pub const SNOW: BlockState = BlockState::new(78, 0);
    pub const SNOW_BLOCK: BlockState = BlockState::new(80, 0);
    pub const GLASS: BlockState = BlockState::new(ids::GLASS, 0);
    pub const PLANKS: BlockState = BlockState::new(ids::PLANKS, 0);
    pub const COBBLESTONE: BlockState = BlockState::new(ids::COBBLESTONE, 0);
    pub const NETHERRACK: BlockState = BlockState::new(ids::NETHERRACK, 0);
    pub const SOUL_SAND: BlockState = BlockState::new(ids::SOUL_SAND, 0);
    pub const GLOWSTONE: BlockState = BlockState::new(ids::GLOWSTONE, 0);
    pub const LAVA: BlockState = BlockState::new(ids::LAVA, 0);
    pub const FLOWING_LAVA: BlockState = BlockState::new(ids::FLOWING_LAVA, 0);
    pub const GRAVEL: BlockState = BlockState::new(ids::GRAVEL, 0);
    pub const OBSIDIAN: BlockState = BlockState::new(ids::OBSIDIAN, 0);
    pub const END_STONE: BlockState = BlockState::new(ids::END_STONE, 0);
    pub const QUARTZ_ORE: BlockState = BlockState::new(ids::QUARTZ_ORE, 0);
    pub const FIRE: BlockState = BlockState::new(ids::FIRE, 0);
    pub const BROWN_MUSHROOM: BlockState = BlockState::new(ids::BROWN_MUSHROOM, 0);
    pub const RED_MUSHROOM: BlockState = BlockState::new(ids::RED_MUSHROOM, 0);
}

/// `Block.getMaterial().isLiquid()` — `MaterialLiquid`, i.e. water and lava in
/// both their flowing and stationary forms (ids 8..=11).
pub fn is_liquid(state: BlockState) -> bool {
    matches!(state.id(), 8..=11)
}

/// `Material.water`: still and flowing water.
pub fn is_water(state: BlockState) -> bool {
    matches!(state.id(), 8 | 9)
}

/// `Material.isSolid()`, which in 1.8.9 is the same predicate as
/// `Material.blocksMovement()`.
///
/// Vanilla answers per *material*, not per block: `MaterialLiquid` (water,
/// lava), `MaterialLogic` (plants, crops, snow cover, vines) and
/// `MaterialTransparent` (air, fire) all return `false`, and every other
/// material returns `true`. This crate has no material table, so the `false`
/// set is spelled out — only the ids generation can actually produce. Anything
/// not listed is solid, which is `Material`'s own default, so the predicate errs
/// the way vanilla does.
///
/// Two vanilla quirks this preserves: leaves are **solid** (the
/// `getTopSolidOrLiquidBlock` caller filters them out separately), and so are
/// torches and rails (`Material.circuits`).
///
/// Not to be confused with [`blocks_movement`] below, which answers the
/// renderer's ray-trace question and includes torches, rails and panes.
pub fn is_solid(state: BlockState) -> bool {
    !matches!(
        state.id(),
        0   // air                        MaterialTransparent
        | 6   // sapling                    Material.plants
        | 8 | 9 | 10 | 11 // water, lava  MaterialLiquid
        | 31  // dead bush                  Material.plants
        | 32  // tall grass                 Material.plants
        | 37 | 38 // dandelion, poppy       Material.plants
        | 39 | 40 // mushrooms              Material.plants
        | 51  // fire                       MaterialTransparent
        | 59  // wheat                      Material.plants
        | 78  // snow cover                 Material.snow (MaterialLogic)
        | 83  // sugar cane                 Material.plants
        | 106 // vine                       MaterialLogic
        | 111 // water lily                 Material.plants
        | 115 // nether wart                Material.plants
        | 141 | 142 // carrots, potatoes  Material.plants
        | 175 // double plant               Material.plants
    )
}

/// `Material.leaves`, which `getTopSolidOrLiquidBlock` skips over.
pub fn is_leaves(state: BlockState) -> bool {
    state.id() == ids::LEAVES
}

/// `Block.isReplaceable(world, pos)`.
///
/// 1.8.9 only overrides the default `false` in six places, and worldgen asks
/// this constantly (can a tree trunk go here, may a lake fill this). The
/// overrides are air, dead bush, tall grass, double plant, vine, and a
/// single-layer snow cover.
pub fn is_replaceable(state: BlockState) -> bool {
    match state.id() {
        0 | 31 | 32 | 106 | 175 => true,
        // `BlockSnow.isReplaceable`: only the one-layer state.
        78 => state.meta() == 0,
        _ => false,
    }
}

/// Hardness of `id`, copied from 1.8.9's block registrations (`Block.setHardness`).
///
/// `UNBREAKABLE` means "cannot be broken at all"; `0.0` breaks instantly.
pub const UNBREAKABLE: f32 = -1.0;

pub fn hardness(id: u16) -> f32 {
    match id {
        0 => 0.0,                 // air
        1 => 1.5,                 // stone
        2 => 0.6,                 // grass
        3 => 0.5,                 // dirt
        4 => 2.0,                 // cobblestone
        5 => 2.0,                 // planks
        6 => 0.0,                 // sapling
        7 => UNBREAKABLE,         // bedrock
        8 | 9 => 100.0,           // water
        10 | 11 => 100.0,         // lava
        12 => 0.5,                // sand
        13 => 0.6,                // gravel
        14..=16 => 3.0,           // ores
        17 => 2.0,                // log
        18 => 0.2,                // leaves
        19 => 0.6,                // sponge
        20 => 0.3,                // glass
        21 => 3.0,                // lapis ore
        22 => 3.0,                // lapis block
        23 => 3.5,                // dispenser
        24 => 0.8,                // sandstone
        25 => 0.8,                // note block
        26 => 0.2,                // bed
        27 | 28 => 0.7,           // rails
        29 => 0.5,                // sticky piston
        30 => 4.0,                // cobweb
        31 => 0.0,                // tall grass
        32 => 0.0,                // dead bush
        33 => 0.5,                // piston
        34 => 1.5,                // piston head
        35 => 0.8,                // wool
        37 | 38 => 0.0,           // flowers
        39 | 40 => 0.0,           // mushrooms
        41 => 3.0,                // gold block
        42 => 5.0,                // iron block
        43 | 44 => 2.0,           // stone slabs
        45 => 2.0,                // bricks
        46 => 0.0,                // tnt
        47 => 1.5,                // bookshelf
        48 => 2.0,                // mossy cobblestone
        49 => 50.0,               // obsidian
        50 => 0.0,                // torch
        51 => 0.0,                // fire
        52 => 5.0,                // mob spawner
        53 => 2.0,                // oak stairs (planks)
        54 => 2.5,                // chest
        55 => 0.0,                // redstone wire
        56 => 3.0,                // diamond ore
        57 => 5.0,                // diamond block
        58 => 2.5,                // crafting table
        59 => 0.0,                // wheat
        60 => 0.6,                // farmland
        61 | 62 => 3.5,           // furnace
        63 => 1.0,                // sign
        64 => 3.0,                // wooden door
        65 => 0.4,                // ladder
        66 => 0.7,                // rail
        67 => 1.5,                // stone stairs (stone)
        68 => 1.0,                // wall sign
        69 => 0.5,                // lever
        70 => 0.5,                // stone pressure plate
        71 => 5.0,                // iron door
        72 => 0.5,                // wooden pressure plate
        73 | 74 => 3.0,           // redstone ore
        75 | 76 => 0.0,           // redstone torch
        77 => 0.5,                // stone button
        78 => 0.1,                // snow layer
        79 => 0.5,                // ice
        80 => 0.2,                // snow block
        81 => 0.4,                // cactus
        82 => 0.6,                // clay
        83 => 0.0,                // reeds
        84 => 2.0,                // jukebox
        85 => 2.0,                // fence
        86 => 1.0,                // pumpkin
        87 => 0.4,                // netherrack
        88 => 0.5,                // soul sand
        89 => 0.3,                // glowstone
        90 => UNBREAKABLE,        // portal
        91 => 1.0,                // jack o'lantern
        92 => 0.5,                // cake
        93 | 94 => 0.0,           // repeaters
        95 => 0.3,                // stained glass
        96 => 3.0,                // trapdoor
        97 => 0.75,               // silverfish egg
        98 => 1.5,                // stone bricks
        99 | 100 => 0.2,          // huge mushrooms
        101 => 5.0,               // iron bars
        102 => 0.3,               // glass pane
        103 => 1.0,               // melon
        104 | 105 => 0.0,         // stems
        106 => 0.2,               // vine
        107 => 2.0,               // fence gate
        108 => 2.0,               // brick stairs
        109 => 1.5,               // stone brick stairs
        110 => 0.6,               // mycelium
        111 => 0.0,               // lily pad
        112 => 2.0,               // nether bricks
        113 => 2.0,               // nether brick fence
        114 => 2.0,               // nether brick stairs
        115 => 0.0,               // nether wart
        116 => 5.0,               // enchanting table
        117 => 0.5,               // brewing stand
        118 => 2.0,               // cauldron
        119 | 120 => UNBREAKABLE, // end portal + frame
        121 => 3.0,               // end stone
        122 => 3.0,               // dragon egg
        123 | 124 => 0.3,         // redstone lamp
        125 | 126 => 2.0,         // wooden slabs
        127 => 0.2,               // cocoa
        128 => 0.8,               // sandstone stairs
        129 => 3.0,               // emerald ore
        130 => 22.5,              // ender chest
        131 | 132 => 0.0,         // tripwire
        133 => 5.0,               // emerald block
        134..=136 => 2.0,         // wood stairs
        137 => UNBREAKABLE,       // command block
        138 => 3.0,               // beacon
        139 => 1.5,               // cobblestone wall (stone bricks)
        140 => 0.0,               // flower pot
        141 | 142 => 0.0,         // crops
        143 => 0.5,               // wooden button
        144 => 1.0,               // skull
        145 => 5.0,               // anvil
        146 => 2.5,               // trapped chest
        147 | 148 => 0.5,         // weighted pressure plates
        149 | 150 => 0.0,         // comparators
        151 => 0.2,               // daylight detector
        152 => 5.0,               // redstone block
        153 => 3.0,               // quartz ore
        154 => 3.0,               // hopper
        155 => 0.8,               // quartz block
        156 => 0.8,               // quartz stairs
        157 => 0.7,               // activator rail
        158 => 3.5,               // dropper
        159 => 1.25,              // stained hardened clay
        160 => 0.3,               // stained glass pane
        161 => 0.2,               // leaves 2
        162 => 2.0,               // log 2
        163 | 164 => 2.0,         // acacia / dark oak stairs
        165 => 0.0,               // slime block
        166 => UNBREAKABLE,       // barrier
        167 => 5.0,               // iron trapdoor
        168 => 1.5,               // prismarine
        169 => 0.3,               // sea lantern
        170 => 0.5,               // hay block
        171 => 0.1,               // carpet
        172 => 1.25,              // hardened clay
        173 => 5.0,               // coal block
        174 => 0.5,               // packed ice
        175 => 0.0,               // double plant
        176 | 177 => 1.0,         // banners
        178 => 0.2,               // inverted daylight detector
        179 => 0.8,               // red sandstone
        180 => 0.8,               // red sandstone stairs
        181 | 182 => 2.0,         // red sandstone slabs
        183..=187 => 2.0,         // fence gates
        188..=192 => 2.0,         // fences
        193..=197 => 3.0,         // doors
        _ => 1.0,
    }
}

/// Whether harvesting this block needs a pickaxe (`setHarvestLevel("pickaxe")`).
///
/// Only affects digging speed for now: without a tool, un-harvestable blocks
/// take the slower `hardness * 100` rate instead of `hardness * 30`.
pub fn requires_tool(id: u16) -> bool {
    matches!(
        id,
        1 | 4
            | 14
            | 15
            | 16
            | 21
            | 22
            | 23
            | 41
            | 42
            | 43
            | 44
            | 45
            | 49
            | 52
            | 56
            | 57
            | 61
            | 62
            | 67
            | 73
            | 74
            | 98
            | 101
            | 109
            | 112
            | 114
            | 116
            | 121
            | 128
            | 129
            | 133
            | 137
            | 139
            | 145
            | 152
            | 153
            | 155
            | 156
            | 159
            | 168
            | 173
            | 174
            | 179
            | 180
    )
}

/// Blocks that stop a ray trace or a player — vanilla's
/// `blockMaterial.blocksMovement()`.
///
/// Air, liquids, plants, torches, rails, pressure plates, ladders and panes
/// are passable; everything else is solid.
pub fn blocks_movement(state: BlockState) -> bool {
    !matches!(
        state.id(),
        0 | 6
            | 8
            | 9
            | 10
            | 11
            | 27
            | 28
            | 30
            | 31
            | 32
            | 37
            | 38
            | 39
            | 40
            | 50
            | 51
            | 55
            | 59
            | 63
            | 65
            | 66
            | 68
            | 69
            | 75
            | 76
            | 77
            | 78
            | 83
            | 90
            | 93
            | 94
            | 96
            | 101
            | 102
            | 106
            | 111
            | 115
            | 119
            | 131
            | 132
            | 140
            | 141
            | 142
            | 143
            | 149
            | 150
            | 157
            | 160
            | 171
            | 175
            | 176
            | 177
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_id_and_meta() {
        let state = BlockState::new(35, 14);
        assert_eq!(state.id(), 35);
        assert_eq!(state.meta(), 14);
        assert_eq!(state.raw(), (35 << 4) | 14);
    }

    #[test]
    fn masks_meta_to_a_nibble() {
        assert_eq!(BlockState::new(1, 0xFF).meta(), 0x0F);
    }

    #[test]
    fn air_is_zero() {
        assert!(BlockState::AIR.is_air());
        assert!(!BlockState::STONE.is_air());
        assert_eq!(BlockState::from_raw(0x0010), BlockState::from_raw(0x0010));
    }

    #[test]
    fn hardness_matches_vanilla_spot_checks() {
        assert_eq!(hardness(ids::AIR), 0.0);
        assert_eq!(hardness(ids::STONE), 1.5);
        assert_eq!(hardness(ids::DIRT), 0.5);
        assert_eq!(hardness(ids::GRASS), 0.6);
        assert_eq!(hardness(ids::COBBLESTONE), 2.0);
        assert_eq!(hardness(ids::BEDROCK), UNBREAKABLE);
        assert_eq!(hardness(49), 50.0); // obsidian
        assert_eq!(hardness(30), 4.0); // cobweb
        assert_eq!(hardness(64), 3.0); // wooden door
    }

    #[test]
    fn stones_need_a_tool_but_soil_does_not() {
        assert!(requires_tool(ids::STONE));
        assert!(requires_tool(ids::COBBLESTONE));
        assert!(!requires_tool(ids::DIRT));
        assert!(!requires_tool(ids::GRASS));
        assert!(!requires_tool(ids::LOG));
    }

    #[test]
    fn passable_blocks_are_not_solid() {
        for id in [0u16, 8, 9, 10, 11, 31, 50, 65, 78, 106] {
            assert!(!blocks_movement(BlockState::new(id, 0)), "id {id}");
        }
        for id in [1u16, 2, 3, 4, 5, 12, 20, 35, 43, 44, 53, 95] {
            assert!(blocks_movement(BlockState::new(id, 0)), "id {id}");
        }
    }
}
