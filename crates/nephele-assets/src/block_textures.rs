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

/// Blocks that should not occlude the face behind them.
pub fn is_transparent(id: u16) -> bool {
    matches!(
        id,
        ids::AIR
            | ids::WATER
            | ids::LEAVES
            | ids::GLASS
            | 31   // tall grass
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

/// Every texture name the table can produce. Derived from `face_textures`
/// itself, so the list can never drift from the mapping.
pub fn all_texture_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for id in 0..=255u16 {
        for meta in 0..16u8 {
            if let Some(textures) = face_textures(BlockState::new(id, meta)) {
                for name in [textures.top, textures.bottom, textures.side] {
                    if !names.contains(&name) {
                        names.push(name);
                    }
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
        let names = all_texture_names();
        for id in 0..=200u16 {
            for meta in 0..16u8 {
                if let Some(textures) = face_textures(BlockState::new(id, meta)) {
                    for name in [textures.top, textures.bottom, textures.side] {
                        assert!(
                            names.contains(&name),
                            "id {id} meta {meta} references {name}, missing from the list"
                        );
                    }
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
    fn transparency_covers_cutout_blocks() {
        assert!(is_transparent(ids::AIR));
        assert!(is_transparent(ids::LEAVES));
        assert!(is_transparent(ids::GLASS));
        assert!(is_transparent(95));
        assert!(!is_transparent(ids::STONE));
    }
}
