//! 1.8.9 item ids to prepared `textures/items` images.
//!
//! Block items keep their block faces; this table covers every non-block item
//! id the 1.8 item table defines. Ids that have no sprite in the jar (mob
//! skulls, for example) return `None` and are reported by
//! [`unmapped_item_ids`] so the boot log can name them instead of leaving a
//! silent hole in the inventory.

macro_rules! item {
    ($file:literal) => {
        concat!("item:", $file)
    };
}

/// First item id in the 1.8 table.
pub const FIRST_ITEM_ID: i16 = 256;
/// Last ordinary item id in the 1.8 table (the banner added in 1.8).
pub const LAST_ITEM_ID: i16 = 425;
/// First music-disc id (`record_13`).
pub const FIRST_RECORD_ID: i16 = 2256;
/// Last music-disc id (`record_wait`).
pub const LAST_RECORD_ID: i16 = 2267;

/// Namespaced texture-array key for a non-block item, if the jar has a sprite.
pub fn item_texture_name(id: i16, meta: i16) -> Option<&'static str> {
    Some(match id {
        256 => item!("iron_shovel"),
        257 => item!("iron_pickaxe"),
        258 => item!("iron_axe"),
        259 => item!("flint_and_steel"),
        260 => item!("apple"),
        261 => item!("bow_standby"),
        262 => item!("arrow"),
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
        321 => item!("painting"),
        322 => item!("apple_golden"),
        323 => item!("sign"),
        324 => item!("door_wood"),
        325 => item!("bucket_empty"),
        326 => item!("bucket_water"),
        327 => item!("bucket_lava"),
        328 => item!("minecart_normal"),
        329 => item!("saddle"),
        330 => item!("door_iron"),
        331 => item!("redstone_dust"),
        332 => item!("snowball"),
        333 => item!("boat"),
        334 => item!("leather"),
        335 => item!("bucket_milk"),
        336 => item!("brick"),
        337 => item!("clay_ball"),
        338 => item!("reeds"),
        339 => item!("paper"),
        340 => item!("book_normal"),
        341 => item!("slimeball"),
        342 => item!("minecart_chest"),
        343 => item!("minecart_furnace"),
        344 => item!("egg"),
        345 => item!("compass"),
        346 => item!("fishing_rod_uncast"),
        347 => item!("clock"),
        348 => item!("glowstone_dust"),
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
        354 => item!("cake"),
        355 => item!("bed"),
        356 => item!("repeater"),
        357 => item!("cookie"),
        358 => item!("map_filled"),
        359 => item!("shears"),
        360 => item!("melon"),
        361 => item!("seeds_pumpkin"),
        362 => item!("seeds_melon"),
        363 => item!("beef_raw"),
        364 => item!("beef_cooked"),
        365 => item!("chicken_raw"),
        366 => item!("chicken_cooked"),
        367 => item!("rotten_flesh"),
        368 => item!("ender_pearl"),
        369 => item!("blaze_rod"),
        370 => item!("ghast_tear"),
        371 => item!("gold_nugget"),
        372 => item!("nether_wart"),
        373 => item!("potion_bottle_drinkable"),
        374 => item!("potion_bottle_empty"),
        375 => item!("spider_eye"),
        376 => item!("spider_eye_fermented"),
        377 => item!("blaze_powder"),
        378 => item!("magma_cream"),
        379 => item!("brewing_stand"),
        380 => item!("cauldron"),
        381 => item!("ender_eye"),
        382 => item!("melon_speckled"),
        383 => item!("spawn_egg"),
        384 => item!("experience_bottle"),
        385 => item!("fireball"),
        386 => item!("book_writable"),
        387 => item!("book_written"),
        388 => item!("emerald"),
        389 => item!("item_frame"),
        390 => item!("flower_pot"),
        391 => item!("carrot"),
        392 => item!("potato"),
        393 => item!("potato_baked"),
        394 => item!("potato_poisonous"),
        395 => item!("map_empty"),
        396 => item!("carrot_golden"),
        // 397 is the mob skull, which has no `textures/items` sprite in 1.8.
        398 => item!("fireworks"),
        399 => item!("fireworks_charge"),
        400 => item!("book_enchanted"),
        401 => item!("comparator"),
        402 => item!("netherbrick"),
        403 => item!("quartz"),
        404 => item!("minecart_tnt"),
        405 => item!("minecart_hopper"),
        406 => item!("wooden_armorstand"),
        407 => item!("iron_horse_armor"),
        408 => item!("gold_horse_armor"),
        409 => item!("diamond_horse_armor"),
        410 => item!("minecart_command_block"),
        411 => item!("rabbit_raw"),
        412 => item!("rabbit_cooked"),
        413 => item!("rabbit_stew"),
        414 => item!("rabbit_foot"),
        415 => item!("rabbit_hide"),
        416 => item!("wooden_armorstand"),
        417 => item!("iron_horse_armor"),
        418 => item!("gold_horse_armor"),
        419 => item!("diamond_horse_armor"),
        420 => item!("lead"),
        421 => item!("name_tag"),
        422 => item!("minecart_command_block"),
        423 => item!("mutton_raw"),
        424 => item!("mutton_cooked"),
        425 => item!("banner_base"),
        2256 => item!("record_13"),
        2257 => item!("record_cat"),
        2258 => item!("record_blocks"),
        2259 => item!("record_chirp"),
        2260 => item!("record_far"),
        2261 => item!("record_mall"),
        2262 => item!("record_mellohi"),
        2263 => item!("record_stal"),
        2264 => item!("record_strad"),
        2265 => item!("record_ward"),
        2266 => item!("record_11"),
        2267 => item!("record_wait"),
        _ => return None,
    })
}

/// Every item id the 1.8 table defines that has no sprite in `textures/items`.
///
/// The boot log prints these so a missing icon is a named line rather than a
/// silent fallback in the inventory.
pub fn unmapped_item_ids() -> Vec<i16> {
    let mut ids = Vec::new();
    for id in FIRST_ITEM_ID..=LAST_ITEM_ID {
        if item_texture_name(id, 0).is_none() {
            ids.push(id);
        }
    }
    for id in FIRST_RECORD_ID..=LAST_RECORD_ID {
        if item_texture_name(id, 0).is_none() {
            ids.push(id);
        }
    }
    ids
}

/// Unique namespaced keys and their file stems for the item textures this
/// module maps. Missing files are simply left on the generated fallback layer.
pub fn all_item_texture_layers() -> Vec<(&'static str, &'static str)> {
    let mut layers = Vec::new();
    let mut push = |key: &'static str| {
        if layers.iter().any(|(known, _)| *known == key) {
            return;
        }
        let file = key
            .strip_prefix("item:")
            .expect("item texture keys are namespaced");
        layers.push((key, file));
    };
    for id in FIRST_ITEM_ID..=LAST_ITEM_ID {
        for meta in 0i16..=15 {
            if let Some(key) = item_texture_name(id, meta) {
                push(key);
            }
        }
    }
    for id in FIRST_RECORD_ID..=LAST_RECORD_ID {
        if let Some(key) = item_texture_name(id, 0) {
            push(key);
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

    #[test]
    fn only_the_sprite_less_ids_are_unmapped() {
        // 397 is the mob skull; it is the one ordinary 1.8 item with no
        // `textures/items` sprite.
        assert_eq!(unmapped_item_ids(), vec![397]);
    }

    #[test]
    fn every_crafting_recipe_output_has_a_sprite() {
        use crate::block_textures::face_textures;
        use nephele_world::BlockState;

        for (id, damage) in nephele_world::crafting::recipe_outputs() {
            let item = item_texture_name(id as i16, damage as i16).is_some();
            let block = face_textures(BlockState::new(id, damage as u8)).is_some();
            assert!(item || block, "recipe output {id} has no icon");
        }
    }
}
