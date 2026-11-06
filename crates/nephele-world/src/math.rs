//! The two pieces of `MathHelper` that the carvers need, reproduced exactly.
//!
//! Vanilla's `MapGenCaves` / `MapGenCavesHell` do **not** call `Math.sin`; they
//! index a 65,536-entry table:
//!
//! ```text
//! sin(x) = SIN_TABLE[(int)(x * 10430.378F) & 65535]
//! cos(x) = SIN_TABLE[(int)(x * 10430.378F + 16384.0F) & 65535]
//! ```
//!
//! The truncating cast and the 16-bit wrap matter: replacing this with a direct
//! `sinf` shifts every tunnel by a fraction of a block, which is enough to move
//! a cave mouth. The table contents are `Math.sin(i * 2π / 65536)` in `f64`,
//! stored as `f32`, exactly as `MathHelper` builds it.

use std::sync::LazyLock;

const TABLE_LEN: usize = 65536;
/// `MathHelper.SIN_TABLE`'s scale: `x * 10430.378F`.
const SIN_SCALE: f32 = 10430.378;

static SIN_TABLE: LazyLock<[f32; TABLE_LEN]> = LazyLock::new(|| {
    let mut table = [0.0f32; TABLE_LEN];
    for (index, slot) in table.iter_mut().enumerate() {
        *slot = (index as f64 * std::f64::consts::PI * 2.0 / TABLE_LEN as f64).sin() as f32;
    }
    table
});

/// `MathHelper.sin(float)`.
#[inline]
pub fn sin(value: f32) -> f32 {
    SIN_TABLE[((value * SIN_SCALE) as i32 & (TABLE_LEN as i32 - 1)) as usize]
}

/// `MathHelper.cos(float)`.
#[inline]
pub fn cos(value: f32) -> f32 {
    SIN_TABLE[((value * SIN_SCALE + 16384.0) as i32 & (TABLE_LEN as i32 - 1)) as usize]
}

/// `MathHelper.floor_double`, including its handling of negative values.
#[inline]
pub fn floor(value: f64) -> i32 {
    let truncated = value as i32;
    if value < f64::from(truncated) {
        truncated - 1
    } else {
        truncated
    }
}

/// `MathHelper.clamp_double`.
#[inline]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sine_table_matches_javas_quantisation() {
        // The table is periodic and the wrap is a 16-bit mask, so sin(0) is 0,
        // sin(pi/2) is 1 and a negative angle still lands inside the table.
        assert!(sin(0.0).abs() < 1.0e-6);
        assert!((sin(std::f32::consts::PI / 2.0) - 1.0).abs() < 1.0e-6);
        assert!((cos(0.0) - 1.0).abs() < 1.0e-6);
        assert!(sin(-0.25).is_finite());
        assert!(cos(-0.25).is_finite());
    }

    #[test]
    fn floor_rounds_toward_negative_infinity() {
        assert_eq!(floor(0.0), 0);
        assert_eq!(floor(1.9), 1);
        assert_eq!(floor(-0.1), -1);
        assert_eq!(floor(-1.0), -1);
    }
}
