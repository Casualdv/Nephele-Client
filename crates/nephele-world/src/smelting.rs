//! Furnace smelting, following the 1.8 recipe table.
//!
//! Only id/damage pairs matter: the client has no experience or tool rules to
//! apply, and vanilla's `FurnaceRecipes` is a flat `input -> output` map.

/// One smelting result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Smelt {
    pub id: u16,
    pub damage: u16,
    pub count: u8,
}

const fn out(id: u16, damage: u16, count: u8) -> Smelt {
    Smelt { id, damage, count }
}

/// Ticks a furnace needs per smelted item (10 s at 20 tps).
pub const COOK_TICKS: u32 = 200;

/// Fuel burn times, in ticks, for the fuels 1.8 accepts.
pub fn fuel_ticks(id: u16) -> Option<u32> {
    Some(match id {
        263 => 1600,  // coal
        173 => 16000, // coal block
        327 => 20000, // lava bucket
        // Planks, logs, wood stairs/slabs/fences, chests, crafting tables.
        5 | 17 | 25 | 47 | 54 | 58 | 85 | 126 | 134 | 135 | 136 | 163 | 164 => 300,
        280 => 100, // stick
        50 => 80,   // torch
        81 => 20,   // cactus
        _ => return None,
    })
}

/// The result of smelting `(id, damage)`, if it has one.
pub fn smelt(id: u16, damage: u16) -> Option<Smelt> {
    Some(match id {
        4 => out(1, 0, 1),                    // cobblestone -> stone
        12 | 24 => out(20, 0, 1),             // sand -> glass
        13 => out(318, 0, 1),                 // gravel -> flint
        14 => out(266, 0, 1),                 // gold ore -> gold ingot
        15 => out(265, 0, 1),                 // iron ore -> iron ingot
        16 => out(263, 0, 1),                 // coal ore -> coal
        17 | 162 => out(263, 1, 1),           // logs -> charcoal
        21 => out(351, 4, 1),                 // lapis ore -> blue dye
        56 => out(264, 0, 1),                 // diamond ore -> diamond
        73 => out(331, 0, 1),                 // redstone ore -> redstone
        82 | 337 => out(336, 0, 1),           // clay -> brick
        87 => out(405, 0, 1),                 // netherrack -> nether brick
        129 => out(388, 0, 1),                // emerald ore -> emerald
        153 => out(406, 0, 1),                // nether quartz ore -> quartz
        319 => out(320, 0, 1),                // raw porkchop -> cooked
        349 if damage == 1 => out(350, 1, 1), // raw salmon -> cooked salmon
        349 => out(350, 0, 1),                // raw fish -> cooked fish
        363 => out(364, 0, 1),                // raw beef -> steak
        365 => out(366, 0, 1),                // raw chicken -> cooked
        392 => out(393, 0, 1),                // potato -> baked potato
        411 => out(412, 0, 1),                // raw rabbit -> cooked
        423 => out(424, 0, 1),                // raw mutton -> cooked
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iron_ore_smelts_into_an_iron_ingot() {
        assert_eq!(smelt(15, 0), Some(out(265, 0, 1)));
    }

    #[test]
    fn logs_smelt_into_charcoal_whatever_the_variant() {
        assert_eq!(smelt(17, 0), Some(out(263, 1, 1)));
        assert_eq!(smelt(17, 3), Some(out(263, 1, 1)));
    }

    #[test]
    fn cobblestone_smelts_into_stone_but_stone_does_not_smelt() {
        assert_eq!(smelt(4, 0), Some(out(1, 0, 1)));
        assert_eq!(smelt(1, 0), None);
        assert_eq!(smelt(20, 0), None);
        assert_eq!(smelt(35, 0), None);
    }
}
