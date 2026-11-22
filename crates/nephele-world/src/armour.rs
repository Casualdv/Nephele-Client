//! Vanilla 1.8 armour points and damage absorption (`ItemArmor`,
//! `CombatRules.getDamageAfterAbsorb`).
//!
//! 1.8 has no armour toughness: every armour point absorbs 4% of incoming
//! damage, and the point total is capped at 20 (a full diamond set) for 80%.

/// Armour points one worn item contributes, or `None` if it is not armour.
pub fn points(id: i16) -> Option<i32> {
    Some(match id {
        // Leather: helmet, chestplate, leggings, boots.
        298 => 1,
        299 => 3,
        300 => 2,
        301 => 1,
        // Chainmail.
        302 => 2,
        303 => 5,
        304 => 4,
        305 => 1,
        // Iron.
        306 => 2,
        307 => 6,
        308 => 5,
        309 => 2,
        // Diamond.
        310 => 3,
        311 => 8,
        312 => 6,
        313 => 3,
        // Gold.
        314 => 2,
        315 => 5,
        316 => 3,
        317 => 1,
        _ => return None,
    })
}

/// Damage left after `armour_points` absorb 4% each, capped at 20 points.
pub fn reduce(armour_points: i32, damage: f32) -> f32 {
    if damage <= 0.0 {
        return 0.0;
    }
    let reduction = (armour_points.clamp(0, 20) as f32) * 0.04;
    (damage * (1.0 - reduction)).max(0.0)
}

/// Vanilla fall damage: nothing for the first three blocks, then one point per
/// extra block, then armour absorption.
pub fn fall_damage(fall_distance: f64, armour_points: i32) -> f32 {
    if fall_distance <= 3.0 {
        0.0
    } else {
        reduce(armour_points, (fall_distance - 3.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_iron_set_absorbs_sixty_percent() {
        // 2 + 6 + 5 + 2 = 15 points.
        let iron = points(306).unwrap()
            + points(307).unwrap()
            + points(308).unwrap()
            + points(309).unwrap();
        assert_eq!(iron, 15);
        assert!((reduce(iron, 10.0) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn a_full_diamond_set_caps_at_eighty_percent() {
        let diamond = points(310).unwrap()
            + points(311).unwrap()
            + points(312).unwrap()
            + points(313).unwrap();
        assert_eq!(diamond, 20);
        assert!((reduce(diamond, 10.0) - 2.0).abs() < 1e-6);
        // Beyond 20 points the reduction does not keep growing.
        assert!((reduce(40, 10.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn fall_damage_starts_after_three_blocks_and_respects_armour() {
        assert_eq!(fall_damage(3.0, 15), 0.0);
        assert!((fall_damage(5.0, 0) - 2.0).abs() < 1e-6);
        assert!((fall_damage(5.0, 15) - 0.8).abs() < 1e-6);
    }
}
