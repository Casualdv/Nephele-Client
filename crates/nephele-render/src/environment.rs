//! Per-frame sky, fog and time-of-day inputs.

/// Fog and sky state for one frame.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// Minecraft ticks (0..24000), where 6000 is noon.
    pub time_of_day: f32,
    pub fog_color: [f32; 3],
    pub fog_start: f32,
    pub fog_end: f32,
    /// 0 in deep water, 1 on land.
    pub fog_scale: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            time_of_day: 6000.0,
            fog_color: [0.68, 0.78, 0.92],
            fog_start: 72.0,
            fog_end: 112.0,
            fog_scale: 1.0,
        }
    }
}

/// Vanilla-ish biome tint used as the horizon/fog colour.
///
/// 1.8's `WorldProvider` supplies one fog colour for the whole overworld, but
/// tinting it by the biome the player is standing in is what makes a swamp read
/// as damp and a mesa read as dusty at a glance.
pub fn biome_fog_color(biome: i32) -> [f32; 3] {
    match biome {
        2 | 17 => [0.86, 0.76, 0.55],                 // desert
        3 | 20 | 34 => [0.70, 0.76, 0.80],            // extreme hills
        6 => [0.48, 0.58, 0.40],                      // swampland
        10..=13 | 26 | 30 | 31 => [0.74, 0.82, 0.91], // frozen
        21..=23 => [0.55, 0.74, 0.48],                // jungle
        24 => [0.42, 0.58, 0.74],                     // deep ocean
        37..=39 => [0.82, 0.67, 0.48],                // mesa
        35 | 36 => [0.84, 0.79, 0.58],                // savanna
        0 | 7 => [0.58, 0.72, 0.86],                  // ocean / river
        _ => [0.68, 0.78, 0.92],                      // plains / forest / default
    }
}
