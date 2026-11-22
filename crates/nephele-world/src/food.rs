//! Vanilla 1.8 food values (`ItemFood` / `FoodStats.addStats`).
//!
//! `hunger` is the half-shank count (two points per shank), and `saturation` is
//! the final saturation the item grants — already multiplied by the item's
//! `saturationModifier * 2` the way `FoodStats.addStats` does it, so callers do
//! not need the modifier table as well.

/// Vanilla's hunger cap.
pub const MAX_HUNGER: i32 = 20;

/// Ticks of use before a food item is consumed (`ItemFood.getMaxItemUseDuration`).
pub const EAT_TICKS: u32 = 32;

/// One food item's hunger and saturation contribution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Food {
    pub hunger: i32,
    pub saturation: f32,
}

const fn entry(hunger: i32, saturation: f32) -> Food {
    Food { hunger, saturation }
}

/// The food value of an item, or `None` when it is not edible.
pub fn food(id: i16, damage: i16) -> Option<Food> {
    Some(match id {
        322 if damage == 1 => entry(4, 9.6), // enchanted golden apple
        322 => entry(4, 9.6),                // golden apple
        260 => entry(4, 2.4),                // apple
        282 => entry(6, 7.2),                // mushroom stew
        297 => entry(5, 6.0),                // bread
        319 => entry(3, 1.8),                // raw porkchop
        320 => entry(8, 12.8),               // cooked porkchop
        349 => entry(2, 0.4),                // raw fish
        350 => entry(5, 6.0),                // cooked fish
        357 => entry(2, 0.4),                // cookie
        360 => entry(2, 1.2),                // melon slice
        363 => entry(3, 1.8),                // raw beef
        364 => entry(8, 12.8),               // steak
        365 => entry(2, 1.2),                // raw chicken
        366 => entry(6, 7.2),                // cooked chicken
        375 => entry(2, 3.2),                // spider eye
        391 => entry(3, 3.6),                // carrot
        392 => entry(1, 0.6),                // potato
        393 => entry(5, 6.0),                // baked potato
        394 => entry(2, 1.2),                // poisonous potato
        400 => entry(8, 4.8),                // pumpkin pie
        411 => entry(3, 1.8),                // raw rabbit
        412 => entry(5, 6.0),                // cooked rabbit
        413 => entry(10, 12.0),              // rabbit stew
        423 => entry(2, 1.2),                // raw mutton
        424 => entry(6, 9.6),                // cooked mutton
        _ => return None,
    })
}

/// `FoodStats.addStats`: hunger caps at 20, saturation at the resulting hunger.
pub fn apply(hunger: i32, saturation: f32, food: Food) -> (i32, f32) {
    let hunger = (hunger + food.hunger).min(MAX_HUNGER);
    let saturation = (saturation + food.saturation).min(hunger as f32);
    (hunger, saturation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steak_is_eight_hunger_and_twelve_point_eight_saturation() {
        let steak = food(364, 0).expect("steak is food");
        assert_eq!(steak, entry(8, 12.8));
        assert_eq!(apply(10, 0.0, steak), (18, 12.8));
    }

    #[test]
    fn eating_caps_hunger_and_saturation() {
        let steak = food(364, 0).unwrap();
        // At full hunger the caller refuses to start a bite; `apply` itself
        // only caps saturation at the resulting hunger.
        assert_eq!(apply(20, 0.0, steak), (20, 12.8));
        assert_eq!(apply(18, 1.0, steak), (20, 13.8));
    }

    #[test]
    fn non_food_items_have_no_value() {
        assert_eq!(food(1, 0), None); // stone
        assert_eq!(food(276, 0), None); // diamond sword
        assert_eq!(food(322, 0), Some(entry(4, 9.6)));
        assert_eq!(food(322, 1), Some(entry(4, 9.6)));
    }
}
