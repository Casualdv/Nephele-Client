use nephele_world::{block::ids, BlockState};

/// Generated fallback tile, always present in the texture array.
pub const MISSING_TEXTURE: &str = "missing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceTextures {
    pub top: &'static str,
    pub bottom: &'static str,
    pub side: &'static str,
}

const fn same(name: &'static str) -> FaceTextures {
    FaceTextures {
        top: name,
        bottom: name,
        side: name,
    }
}

/// One texture per face, with distinct top/bottom/side.
const fn split(top: &'static str, bottom: &'static str, side: &'static str) -> FaceTextures {
    FaceTextures { top, bottom, side }
}

/// 1.8 dye order; `silver` is the light-grey texture name.
const DYES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "silver",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// Textures for a block state, or `None` for blocks Nephele does not render
/// (air, and non-cube shapes).
pub fn face_textures(state: BlockState) -> Option<FaceTextures> {
    let id = state.id();
    let meta = state.meta();
    let textures = match id {
        ids::AIR => return None,

        ids::STONE => match meta {
            1 => same("stone_granite"),
            2 => same("stone_granite_smooth"),
            3 => same("stone_diorite"),
            4 => same("stone_diorite_smooth"),
            5 => same("stone_andesite"),
            6 => same("stone_andesite_smooth"),
            _ => same("stone"),
        },
        ids::GRASS => split("grass_top", "dirt", "grass_side"),
        ids::DIRT => same("dirt"),
        ids::COBBLESTONE => same("cobblestone"),
        ids::PLANKS => same(planks(meta)),
        ids::BEDROCK => same("bedrock"),
        ids::WATER => same("water_still"),
        10 => same("lava_flow"),
        11 => same("lava_still"),
        ids::SAND => same("sand"),
        ids::GRAVEL => same("gravel"),
        14 => same("gold_ore"),
        15 => same("iron_ore"),
        16 => same("coal_ore"),
        ids::LOG => split(log_top(meta), log_top(meta), log_side(meta)),
        ids::LEAVES => same(leaves(meta)),
        ids::GLASS => same("glass"),
        ids::SANDSTONE => match meta {
            1 => split("sandstone_top", "sandstone_bottom", "sandstone_carved"),
            2 => split("sandstone_top", "sandstone_bottom", "sandstone_smooth"),
            _ => split("sandstone_top", "sandstone_bottom", "sandstone_normal"),
        },
        35 => same(wool(meta)),
        41 => same("gold_block"),
        42 => same("iron_block"),
        45 => same("brick"),
        47 => split("planks_oak", "planks_oak", "bookshelf"),
        48 => same("cobblestone_mossy"),
        49 => same("obsidian"),
        51 => same("fire_layer_0"),
        56 => same("diamond_ore"),
        57 => same("diamond_block"),
        58 => split("crafting_table_top", "planks_oak", "crafting_table_side"),
        61 => split("furnace_top", "furnace_top", "furnace_side"),
        73 => same("redstone_ore"),
        79 => same("ice"),
        80 => same("snow"),
        81 => split("cactus_top", "cactus_bottom", "cactus_side"),
        82 => same("clay"),
        84 => split("jukebox_top", "jukebox_side", "jukebox_side"),
        87 => same("netherrack"),
        88 => same("soul_sand"),
        89 => same("glowstone"),
        90 => same("portal"),
        91 => split("pumpkin_top", "pumpkin_top", "pumpkin_side"),
        95 => same(STAINED_GLASS[meta.min(15) as usize]),
        98 => match meta {
            1 => same("stonebrick_mossy"),
            2 => same("stonebrick_cracked"),
            3 => same("stonebrick_carved"),
            _ => same("stonebrick"),
        },
        103 => split("melon_top", "melon_top", "melon_side"),
        110 => split("mycelium_top", "dirt", "mycelium_side"),
        112 => same("nether_brick"),
        121 => same("end_stone"),
        123 => same("redstone_lamp_off"),
        129 => same("emerald_ore"),
        133 => same("emerald_block"),
        152 => same("redstone_block"),
        153 => same("quartz_ore"),
        155 => split(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        159 => same(stained_clay(meta)),
        162 => split(log2_top(meta), log2_top(meta), log2_side(meta)),
        168 => same("prismarine_rough"),
        170 => split("hay_block_top", "hay_block_top", "hay_block_side"),
        172 => same("hardened_clay"),
        173 => same("coal_block"),
        174 => same("ice_packed"),
        179 => match meta {
            1 => split(
                "red_sandstone_top",
                "red_sandstone_bottom",
                "red_sandstone_carved",
            ),
            2 => split(
                "red_sandstone_top",
                "red_sandstone_bottom",
                "red_sandstone_smooth",
            ),
            _ => split(
                "red_sandstone_top",
                "red_sandstone_bottom",
                "red_sandstone_normal",
            ),
        },

        8 => same("water_flow"),
        6 => same(sapling(meta)),
        19 => same(if meta == 1 { "sponge_wet" } else { "sponge" }),
        21 => same("lapis_ore"),
        22 => same("lapis_block"),
        23 => split("furnace_top", "furnace_top", "dispenser_front_horizontal"),
        25 => same("noteblock"),
        26 => bed(meta),
        27 => same("rail_golden_powered"),
        28 => same("rail_detector"),
        29 => split("piston_top_sticky", "piston_bottom", "piston_side"),
        30 => same("web"),
        31 => same(if meta == 2 { "fern" } else { "tallgrass" }),
        32 => same("deadbush"),
        33 | 34 | 36 => split("piston_top_normal", "piston_bottom", "piston_side"),
        37 => same("flower_dandelion"),
        38 => same(flower(meta)),
        39 => same("mushroom_brown"),
        40 => same("mushroom_red"),
        43 | 44 => slab(meta),
        46 => split("tnt_top", "tnt_bottom", "tnt_side"),
        50 => same("torch_on"),
        52 => same("mob_spawner"),
        53 => same("planks_oak"),
        54 => same("planks_oak"),
        55 => same("redstone_dust_cross"),
        59 => same("wheat_stage_7"),
        60 => {
            let farmland = if meta == 0 {
                "farmland_dry"
            } else {
                "farmland_wet"
            };
            split(farmland, "dirt", farmland)
        }
        62 => split("furnace_top", "furnace_top", "furnace_front_on"),
        63 => same("planks_oak"),
        64 => door("door_wood_upper", "door_wood_lower"),
        65 => same("ladder"),
        66 => same("rail_normal"),
        67 => same("cobblestone"),
        68 => same("planks_oak"),
        69 => same("lever"),
        70 => same("stone"),
        71 => door("door_iron_upper", "door_iron_lower"),
        72 => same("planks_oak"),
        74 => same("redstone_ore"),
        75 => same("redstone_torch_on"),
        76 => same("redstone_torch_off"),
        77 => same("stone"),
        78 => same("snow"),
        83 => same("reeds"),
        85 => same("planks_oak"),
        86 => split("pumpkin_top", "pumpkin_top", "pumpkin_face_off"),
        92 => split("cake_top", "cake_bottom", "cake_side"),
        93 => same("repeater_off"),
        94 => same("repeater_on"),
        96 => same("trapdoor"),
        97 => same(infested(meta)),
        99 => same("mushroom_block_skin_brown"),
        100 => same("mushroom_block_skin_red"),
        101 => same("iron_bars"),
        102 => split("glass_pane_top", "glass_pane_top", "glass"),
        104 => same("melon_stem_disconnected"),
        105 => same("pumpkin_stem_disconnected"),
        106 => same("vine"),
        107 => same("planks_oak"),
        108 => same("brick"),
        109 => same("stonebrick"),
        111 => same("waterlily"),
        113 | 114 => same("nether_brick"),
        115 => same("nether_wart_stage_2"),
        116 => split(
            "enchanting_table_top",
            "enchanting_table_bottom",
            "enchanting_table_side",
        ),
        117 => split("brewing_stand", "brewing_stand_base", "brewing_stand_base"),
        118 => split("cauldron_top", "cauldron_bottom", "cauldron_side"),
        119 => same("portal"),
        120 => split("obsidian", "obsidian", "obsidian"),
        122 => same("dragon_egg"),
        124 => same("redstone_lamp_on"),
        125 | 126 => same(planks(meta & 7)),
        127 => same("cocoa_stage_2"),
        128 => split("sandstone_top", "sandstone_bottom", "sandstone_normal"),
        130 => same("obsidian"),
        131 => same("trip_wire_source"),
        132 => same("trip_wire"),
        134 => same("planks_spruce"),
        135 => same("planks_birch"),
        136 => same("planks_jungle"),
        137 => same("command_block"),
        138 => same("beacon"),
        139 => same(if meta == 1 {
            "cobblestone_mossy"
        } else {
            "cobblestone"
        }),
        140 => same("flower_pot"),
        141 => same("carrots_stage_3"),
        142 => same("potatoes_stage_3"),
        143 => same("planks_oak"),
        144 => same("stone"),
        145 => split("anvil_top_damaged_0", "anvil_base", "anvil_base"),
        146 => same("planks_oak"),
        147 => same("gold_block"),
        148 => same("iron_block"),
        149 => same("comparator_off"),
        150 => same("comparator_on"),
        151 => split(
            "daylight_detector_top",
            "daylight_detector_side",
            "daylight_detector_side",
        ),
        154 => split("hopper_top", "hopper_outside", "hopper_outside"),
        156 => split(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        157 => same("rail_activator"),
        158 => split("furnace_top", "furnace_top", "dropper_front_horizontal"),
        160 => pane(meta),
        161 => same("leaves_acacia"),
        163 => same("planks_acacia"),
        164 => same("planks_big_oak"),
        165 => same("slime"),
        166 => same("glass"),
        167 => same("iron_trapdoor"),
        169 => same("sea_lantern"),
        171 => same(wool(meta)),
        175 => double_plant(meta),
        176 | 177 => same(wool(meta)),
        178 => same("daylight_detector_inverted_top"),
        180..=182 => split(
            "red_sandstone_top",
            "red_sandstone_bottom",
            "red_sandstone_normal",
        ),
        183 => same("planks_spruce"),
        184 => same("planks_birch"),
        185 => same("planks_jungle"),
        186 => same("planks_big_oak"),
        187 => same("planks_acacia"),
        188 => same("planks_spruce"),
        189 => same("planks_birch"),
        190 => same("planks_jungle"),
        191 => same("planks_big_oak"),
        192 => same("planks_acacia"),
        193 => door("door_spruce_upper", "door_spruce_lower"),
        194 => door("door_birch_upper", "door_birch_lower"),
        195 => door("door_jungle_upper", "door_jungle_lower"),
        196 => door("door_acacia_upper", "door_acacia_lower"),
        197 => door("door_dark_oak_upper", "door_dark_oak_lower"),

        _ => return Some(same(MISSING_TEXTURE)),
    };
    Some(textures)
}

/// Stained glass reuses the plain glass texture tinted? 1.8 has dedicated
/// `glass_<colour>` textures, but only for the pane-less block.
const STAINED_GLASS: [&str; 16] = [
    "glass_white",
    "glass_orange",
    "glass_magenta",
    "glass_light_blue",
    "glass_yellow",
    "glass_lime",
    "glass_pink",
    "glass_gray",
    "glass_silver",
    "glass_cyan",
    "glass_purple",
    "glass_blue",
    "glass_brown",
    "glass_green",
    "glass_red",
    "glass_black",
];

fn planks(meta: u8) -> &'static str {
    match meta {
        0 => "planks_oak",
        1 => "planks_spruce",
        2 => "planks_birch",
        3 => "planks_jungle",
        4 => "planks_acacia",
        _ => "planks_big_oak",
    }
}

fn log_side(meta: u8) -> &'static str {
    match meta {
        1 => "log_spruce",
        2 => "log_birch",
        3 => "log_jungle",
        _ => "log_oak",
    }
}

fn log_top(meta: u8) -> &'static str {
    match meta {
        1 => "log_spruce_top",
        2 => "log_birch_top",
        3 => "log_jungle_top",
        _ => "log_oak_top",
    }
}

fn leaves(meta: u8) -> &'static str {
    match meta {
        1 => "leaves_spruce",
        2 => "leaves_birch",
        3 => "leaves_jungle",
        _ => "leaves_oak",
    }
}

fn log2_side(meta: u8) -> &'static str {
    match meta {
        0 => "log_acacia",
        _ => "log_big_oak",
    }
}

fn log2_top(meta: u8) -> &'static str {
    match meta {
        0 => "log_acacia_top",
        _ => "log_big_oak_top",
    }
}

fn wool(meta: u8) -> &'static str {
    match meta {
        0 => "wool_colored_white",
        1 => "wool_colored_orange",
        2 => "wool_colored_magenta",
        3 => "wool_colored_light_blue",
        4 => "wool_colored_yellow",
        5 => "wool_colored_lime",
        6 => "wool_colored_pink",
        7 => "wool_colored_gray",
        8 => "wool_colored_silver",
        9 => "wool_colored_cyan",
        10 => "wool_colored_purple",
        11 => "wool_colored_blue",
        12 => "wool_colored_brown",
        13 => "wool_colored_green",
        14 => "wool_colored_red",
        _ => "wool_colored_black",
    }
}

fn stained_clay(meta: u8) -> &'static str {
    match meta {
        0 => "hardened_clay_stained_white",
        1 => "hardened_clay_stained_orange",
        2 => "hardened_clay_stained_magenta",
        3 => "hardened_clay_stained_light_blue",
        4 => "hardened_clay_stained_yellow",
        5 => "hardened_clay_stained_lime",
        6 => "hardened_clay_stained_pink",
        7 => "hardened_clay_stained_gray",
        8 => "hardened_clay_stained_silver",
        9 => "hardened_clay_stained_cyan",
        10 => "hardened_clay_stained_purple",
        11 => "hardened_clay_stained_blue",
        12 => "hardened_clay_stained_brown",
        13 => "hardened_clay_stained_green",
        14 => "hardened_clay_stained_red",
        _ => "hardened_clay_stained_black",
    }
}

const fn door(upper: &'static str, lower: &'static str) -> FaceTextures {
    split(upper, lower, lower)
}

fn sapling(meta: u8) -> &'static str {
    match meta & 7 {
        1 => "sapling_spruce",
        2 => "sapling_birch",
        3 => "sapling_jungle",
        4 => "sapling_acacia",
        5 => "sapling_roofed_oak",
        _ => "sapling_oak",
    }
}

fn bed(meta: u8) -> FaceTextures {
    if meta & 8 != 0 {
        split("bed_head_top", "planks_oak", "bed_head_side")
    } else {
        split("bed_feet_top", "planks_oak", "bed_feet_side")
    }
}

fn flower(meta: u8) -> &'static str {
    match meta {
        1 => "flower_blue_orchid",
        2 => "flower_allium",
        3 => "flower_houstonia",
        4 => "flower_tulip_red",
        5 => "flower_tulip_orange",
        6 => "flower_tulip_white",
        7 => "flower_tulip_pink",
        8 => "flower_oxeye_daisy",
        _ => "flower_rose",
    }
}

fn slab(meta: u8) -> FaceTextures {
    match meta & 7 {
        1 => split("sandstone_top", "sandstone_bottom", "sandstone_normal"),
        2 => same("planks_oak"),
        3 => same("cobblestone"),
        4 => same("brick"),
        5 => same("stonebrick"),
        6 => same("nether_brick"),
        7 => split(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        _ => split("stone_slab_top", "stone_slab_top", "stone_slab_side"),
    }
}

fn infested(meta: u8) -> &'static str {
    match meta {
        1 => "cobblestone",
        2 => "stonebrick",
        3 => "stonebrick_mossy",
        4 => "stonebrick_cracked",
        5 => "stonebrick_carved",
        _ => "stone",
    }
}

const STAINED_GLASS_PANES: [FaceTextures; 16] = [
    split(
        "glass_pane_top_white",
        "glass_pane_top_white",
        "glass_white",
    ),
    split(
        "glass_pane_top_orange",
        "glass_pane_top_orange",
        "glass_orange",
    ),
    split(
        "glass_pane_top_magenta",
        "glass_pane_top_magenta",
        "glass_magenta",
    ),
    split(
        "glass_pane_top_light_blue",
        "glass_pane_top_light_blue",
        "glass_light_blue",
    ),
    split(
        "glass_pane_top_yellow",
        "glass_pane_top_yellow",
        "glass_yellow",
    ),
    split("glass_pane_top_lime", "glass_pane_top_lime", "glass_lime"),
    split("glass_pane_top_pink", "glass_pane_top_pink", "glass_pink"),
    split("glass_pane_top_gray", "glass_pane_top_gray", "glass_gray"),
    split(
        "glass_pane_top_silver",
        "glass_pane_top_silver",
        "glass_silver",
    ),
    split("glass_pane_top_cyan", "glass_pane_top_cyan", "glass_cyan"),
    split(
        "glass_pane_top_purple",
        "glass_pane_top_purple",
        "glass_purple",
    ),
    split("glass_pane_top_blue", "glass_pane_top_blue", "glass_blue"),
    split(
        "glass_pane_top_brown",
        "glass_pane_top_brown",
        "glass_brown",
    ),
    split(
        "glass_pane_top_green",
        "glass_pane_top_green",
        "glass_green",
    ),
    split("glass_pane_top_red", "glass_pane_top_red", "glass_red"),
    split(
        "glass_pane_top_black",
        "glass_pane_top_black",
        "glass_black",
    ),
];

fn pane(meta: u8) -> FaceTextures {
    STAINED_GLASS_PANES[meta.min(15) as usize]
}

fn double_plant(meta: u8) -> FaceTextures {
    match meta {
        1 => split(
            "double_plant_syringa_top",
            "double_plant_syringa_bottom",
            "double_plant_syringa_top",
        ),
        2 => split(
            "double_plant_grass_top",
            "double_plant_grass_bottom",
            "double_plant_grass_top",
        ),
        3 => split(
            "double_plant_fern_top",
            "double_plant_fern_bottom",
            "double_plant_fern_top",
        ),
        4 => split(
            "double_plant_rose_top",
            "double_plant_rose_bottom",
            "double_plant_rose_top",
        ),
        5 => split(
            "double_plant_paeonia_top",
            "double_plant_paeonia_bottom",
            "double_plant_paeonia_top",
        ),
        _ => split(
            "double_plant_sunflower_top",
            "double_plant_sunflower_bottom",
            "double_plant_sunflower_top",
        ),
    }
}

/// Blocks that should not occlude the face behind them.
pub fn is_transparent(id: u16) -> bool {
    matches!(
        id,
        ids::AIR
            | ids::WATER
            | ids::LEAVES
            | ids::GLASS
            | 6    // sapling
            | 31   // tall grass
            | 32   // dead bush
            | 37 | 38 | 39 | 40 // flowers and mushrooms
            | 50 | 75 | 76 // torches
            | 51   // fire (crossed, cutout/blended quads)
            | 59   // wheat
            | 83   // sugar cane
            | 104 | 105 // stems
            | 111  // lily pad
            | 115  // nether wart
            | 141 | 142 // carrots / potatoes
            | 171  // carpet
            | 175  // double plant
            | 78   // snow layer
            | 95   // stained glass
            | 101  // iron bars
            | 102  // glass pane
            | 160 // stained glass pane
    )
}

/// Crack overlay textures, in break-progress order.
pub const DESTROY_STAGES: [&str; 10] = [
    "destroy_stage_0",
    "destroy_stage_1",
    "destroy_stage_2",
    "destroy_stage_3",
    "destroy_stage_4",
    "destroy_stage_5",
    "destroy_stage_6",
    "destroy_stage_7",
    "destroy_stage_8",
    "destroy_stage_9",
];

/// Every texture name the 1.8 block table's variants ask for, derived from
/// `face_textures` itself so the list can never drift from the mapping.
///
/// Post-1.8 ids in [`nephele_world::blocks::VARIANTS`] are skipped: the
/// prepared 1.8.9 jar cannot contain their textures.
pub fn referenced_texture_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for &(id, meta, _) in nephele_world::blocks::VARIANTS.iter() {
        if id > 197 {
            continue;
        }
        if let Some(textures) = face_textures(BlockState::new(id, meta)) {
            for name in [textures.top, textures.bottom, textures.side] {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    for name in DESTROY_STAGES {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[allow(dead_code)]
const _: [&str; 16] = DYES;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_has_no_texture() {
        assert!(face_textures(BlockState::AIR).is_none());
    }

    #[test]
    fn grass_uses_three_textures() {
        let grass = face_textures(BlockState::GRASS).unwrap();
        assert_eq!(grass.top, "grass_top");
        assert_eq!(grass.bottom, "dirt");
        assert_eq!(grass.side, "grass_side");
    }

    #[test]
    fn metadata_picks_variants() {
        assert_eq!(
            face_textures(BlockState::new(5, 0)).unwrap().side,
            "planks_oak"
        );
        assert_eq!(
            face_textures(BlockState::new(5, 1)).unwrap().side,
            "planks_spruce"
        );
        assert_eq!(
            face_textures(BlockState::new(5, 5)).unwrap().side,
            "planks_big_oak"
        );
        assert_eq!(
            face_textures(BlockState::new(35, 14)).unwrap().side,
            "wool_colored_red"
        );
        assert_eq!(
            face_textures(BlockState::new(1, 3)).unwrap().side,
            "stone_diorite"
        );
        assert_eq!(
            face_textures(BlockState::new(17, 2)).unwrap().side,
            "log_birch"
        );
        assert_eq!(
            face_textures(BlockState::new(17, 2)).unwrap().top,
            "log_birch_top"
        );
    }

    #[test]
    fn unknown_blocks_fall_back_to_missing() {
        assert_eq!(
            face_textures(BlockState::new(999, 0)).unwrap().top,
            MISSING_TEXTURE
        );
    }

    #[test]
    fn name_list_covers_every_mapping() {
        let names = referenced_texture_names();
        for &(id, meta, _) in nephele_world::blocks::VARIANTS.iter() {
            if id > 197 {
                continue;
            }
            if let Some(textures) = face_textures(BlockState::new(id, meta)) {
                for name in [textures.top, textures.bottom, textures.side] {
                    assert!(
                        names.contains(&name),
                        "id {id} meta {meta} references {name}, missing from the list"
                    );
                }
            }
        }
        assert!(
            names.len() > 100,
            "expected broad coverage, got {}",
            names.len()
        );
    }

    #[test]
    fn every_1_8_variant_resolves_to_a_real_texture() {
        for &(id, meta, name) in nephele_world::blocks::VARIANTS.iter() {
            if id > 197 || id == 0 {
                continue;
            }
            let textures = face_textures(BlockState::new(id, meta))
                .unwrap_or_else(|| panic!("{id}:{meta} {name} has no texture mapping"));
            for texture in [textures.top, textures.bottom, textures.side] {
                assert_ne!(
                    texture, MISSING_TEXTURE,
                    "{id}:{meta} {name} still falls back to the missing texture"
                );
            }
        }
    }

    #[test]
    fn transparency_covers_cutout_blocks() {
        assert!(is_transparent(ids::AIR));
        assert!(is_transparent(ids::LEAVES));
        assert!(is_transparent(ids::GLASS));
        assert!(is_transparent(95));
        assert!(!is_transparent(ids::STONE));
    }
}
