//! 1.8.9 item ids to prepared `textures/items` images.
//!
//! Block items continue to use their block faces. These keys cover the
//! non-block items currently shown by survival crafting, armour, food and
//! dropped-item rendering.

macro_rules! item {
    ($file:literal) => {
        concat!("item:", $file)
    };
}

/// Namespaced texture-array key for a non-block item, if Nephele maps one.
pub fn item_texture_name(id: i16, meta: i16) -> Option<&'static str> {
    Some(match id {
        256 => item!("iron_shovel"),
        257 => item!("iron_pickaxe"),
        258 => item!("iron_axe"),
        260 => item!("apple"),
        263 if meta == 1 => item!("charcoal"),
        263 => item!("coal"),
        264 => item!("diamond"),
        265 => item!("iron_ingot"),
        266 => item!("gold_ingot"),
        267 => item!("iron_sword"),
        268 => item!("wood_sword"),
        269 => item!("wood_shovel"),
        270 => item!("wood_pickaxe"),
        271 => item!("wood_axe"),
        272 => item!("stone_sword"),
        273 => item!("stone_shovel"),
        274 => item!("stone_pickaxe"),
        275 => item!("stone_axe"),
        276 => item!("diamond_sword"),
        277 => item!("diamond_shovel"),
        278 => item!("diamond_pickaxe"),
        279 => item!("diamond_axe"),
        280 => item!("stick"),
        281 => item!("bowl"),
        282 => item!("mushroom_stew"),
        283 => item!("gold_sword"),
        284 => item!("gold_shovel"),
        285 => item!("gold_pickaxe"),
        286 => item!("gold_axe"),
        287 => item!("string"),
        288 => item!("feather"),
        289 => item!("gunpowder"),
        290 => item!("wood_hoe"),
        291 => item!("stone_hoe"),
        292 => item!("iron_hoe"),
        293 => item!("diamond_hoe"),
        294 => item!("gold_hoe"),
        295 => item!("seeds_wheat"),
        296 => item!("wheat"),
        297 => item!("bread"),
        298 => item!("leather_helmet"),
        299 => item!("leather_chestplate"),
        300 => item!("leather_leggings"),
        301 => item!("leather_boots"),
        302 => item!("chainmail_helmet"),
        303 => item!("chainmail_chestplate"),
        304 => item!("chainmail_leggings"),
        305 => item!("chainmail_boots"),
        306 => item!("iron_helmet"),
        307 => item!("iron_chestplate"),
        308 => item!("iron_leggings"),
        309 => item!("iron_boots"),
        310 => item!("diamond_helmet"),
        311 => item!("diamond_chestplate"),
        312 => item!("diamond_leggings"),
        313 => item!("diamond_boots"),
        314 => item!("gold_helmet"),
        315 => item!("gold_chestplate"),
        316 => item!("gold_leggings"),
        317 => item!("gold_boots"),
        318 => item!("flint"),
        319 => item!("porkchop_raw"),
        320 => item!("porkchop_cooked"),
        322 => item!("apple_golden"),
        349 => match meta {
            0 => item!("fish_cod_raw"),
            1 => item!("fish_salmon_raw"),
            2 => item!("fish_clownfish_raw"),
            3 => item!("fish_pufferfish_raw"),
            _ => return None,
        },
        350 => match meta {
            0 => item!("fish_cod_cooked"),
            1 => item!("fish_salmon_cooked"),
            _ => return None,
        },
        351 => match meta {
            0 => item!("dye_powder_black"),
            1 => item!("dye_powder_red"),
            2 => item!("dye_powder_green"),
            3 => item!("dye_powder_brown"),
            4 => item!("dye_powder_blue"),
            5 => item!("dye_powder_purple"),
            6 => item!("dye_powder_cyan"),
            7 => item!("dye_powder_silver"),
            8 => item!("dye_powder_gray"),
            9 => item!("dye_powder_pink"),
            10 => item!("dye_powder_lime"),
            11 => item!("dye_powder_yellow"),
            12 => item!("dye_powder_light_blue"),
            13 => item!("dye_powder_magenta"),
            14 => item!("dye_powder_orange"),
            15 => item!("dye_powder_white"),
            _ => return None,
        },
        352 => item!("bone"),
        353 => item!("sugar"),
        357 => item!("cookie"),
        358 => item!("map_filled"),
        363 => item!("beef_raw"),
        364 => item!("beef_cooked"),
        365 => item!("chicken_raw"),
        366 => item!("chicken_cooked"),
        367 => item!("rotten_flesh"),
        373 => item!("potion_bottle_drinkable"),
        374 => item!("potion_bottle_empty"),
        375 => item!("spider_eye"),
        376 => item!("spider_eye_fermented"),
        377 => item!("blaze_powder"),
        378 => item!("magma_cream"),
        391 => item!("carrot"),
        392 => item!("potato"),
        393 => item!("potato_baked"),
        394 => item!("potato_poisonous"),
        396 => item!("carrot_golden"),
        400 => item!("pumpkin_pie"),
        411 => item!("rabbit_raw"),
        412 => item!("rabbit_cooked"),
        413 => item!("rabbit_stew"),
        414 => item!("rabbit_foot"),
        415 => item!("rabbit_hide"),
        423 => item!("mutton_raw"),
        424 => item!("mutton_cooked"),
        _ => return None,
    })
}

/// Unique namespaced keys and their file stems for the item textures this
/// module maps. Missing files are simply left on the generated fallback layer.
pub fn all_item_texture_layers() -> Vec<(&'static str, &'static str)> {
    let mut layers = Vec::new();
    for id in 256i16..=424 {
        for meta in 0i16..=15 {
            let Some(key) = item_texture_name(id, meta) else {
                continue;
            };
            if layers.iter().any(|(known, _)| *known == key) {
                continue;
            }
            let file = key
                .strip_prefix("item:")
                .expect("item texture keys are namespaced");
            layers.push((key, file));
        }
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn maps_crafted_tools_armour_and_food_to_item_layers() {
        assert_eq!(item_texture_name(270, 0), Some("item:wood_pickaxe"));
        assert_eq!(item_texture_name(257, 12), Some("item:iron_pickaxe"));
        assert_eq!(item_texture_name(307, 0), Some("item:iron_chestplate"));
        assert_eq!(item_texture_name(297, 0), Some("item:bread"));
        assert_eq!(item_texture_name(349, 3), Some("item:fish_pufferfish_raw"));
        assert_eq!(item_texture_name(999, 0), None);
    }

    #[test]
    fn item_layer_keys_are_unique_and_namespaced() {
        let layers = all_item_texture_layers();
        let keys: HashSet<_> = layers.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys.len(), layers.len());
        assert!(layers
            .iter()
            .all(|(key, file)| key.starts_with("item:") && !file.is_empty()));
    }
}
