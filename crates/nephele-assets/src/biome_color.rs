//! Vanilla biome colourizer tables for grass, foliage and water.
//!
//! 1.8.9 stores grass and foliage colours in two 256x256 PNGs and looks them up
//! by `(temperature, rainfall)`. Those files are extracted from the player's
//! own client jar by `nephele-prepare`; the fallback below keeps a fresh tree
//! usable before resources are prepared.

use std::path::Path;

use image::RgbaImage;
use nephele_world::biome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintKind {
    Grass,
    Foliage,
    Water,
}

#[derive(Debug, Clone)]
pub struct BiomeColorizer {
    grass: Option<ColorMap>,
    foliage: Option<ColorMap>,
}

impl BiomeColorizer {
    pub fn load(textures_dir: &Path) -> Self {
        let colormap = textures_dir.join("colormap");
        Self {
            grass: ColorMap::load(&colormap.join("grass.png")),
            foliage: ColorMap::load(&colormap.join("foliage.png")),
        }
    }

    pub fn fallback() -> Self {
        Self {
            grass: None,
            foliage: None,
        }
    }

    /// Vanilla `BiomeColorHelper` colour for one biome.
    ///
    /// The three resolvers are exactly vanilla's: grass, foliage and the water
    /// multiplier. Block-specific constants (spruce/birch leaves, lily pad) are
    /// not resolvers and live in the mesher.
    pub fn tint(&self, kind: TintKind, biome_id: i32) -> [f32; 3] {
        let biome =
            biome::any(biome_id).unwrap_or_else(|| biome::any(1).expect("plains is registered"));
        let temperature = biome.base.temperature.clamp(0.0, 1.0) as f64;
        let rainfall = biome.base.rainfall.clamp(0.0, 1.0) as f64;
        // `BiomeGenBase` subclasses override the colormap lookup outright.
        match kind {
            TintKind::Grass => {
                // `BiomeGenSwamp.getGrassColorAtPos` picks between two fixed
                // values from a noise field; the blotchy half is not modelled.
                if biome::is_swamp_class(biome_id) {
                    return rgb(0x6A7039);
                }
                // `BiomeGenForest` roofed variant: `(c & 0xFEFEFE) + 0x28340A >> 1`.
                if matches!(biome_id, 29 | 157) {
                    let base = self.grass_color(temperature, rainfall);
                    let channel = |value: f32, add: u32| {
                        ((((value * 255.0) as u32 & 0xFE) + add) >> 1) as f32 / 255.0
                    };
                    return [
                        channel(base[0], 0x28),
                        channel(base[1], 0x34),
                        channel(base[2], 0x0A),
                    ];
                }
                // `BiomeGenMesa.getGrassColorAtPos`.
                if biome::is_mesa(biome_id) {
                    return rgb(0x90814D);
                }
                self.grass_color(temperature, rainfall)
            }
            TintKind::Foliage => {
                // `BiomeGenSwamp.getFoliageColorAtPos`.
                if biome::is_swamp_class(biome_id) {
                    return rgb(0x6A7039);
                }
                // `BiomeGenMesa.getFoliageColorAtPos`.
                if biome::is_mesa(biome_id) {
                    return rgb(0x9E814D);
                }
                self.foliage_color(temperature, rainfall)
            }
            TintKind::Water => {
                // `BiomeGenBase.waterColorMultiplier` is white except swamp.
                if biome::is_swamp_class(biome_id) {
                    rgb(0xE0FFAE)
                } else {
                    [1.0, 1.0, 1.0]
                }
            }
        }
    }

    /// One colormap lookup, or the approximation before resources exist.
    fn grass_color(&self, temperature: f64, rainfall: f64) -> [f32; 3] {
        self.grass
            .as_ref()
            .map(|map| map.get(temperature, rainfall))
            .unwrap_or_else(|| approximate(temperature, rainfall, true))
    }

    fn foliage_color(&self, temperature: f64, rainfall: f64) -> [f32; 3] {
        self.foliage
            .as_ref()
            .map(|map| map.get(temperature, rainfall))
            .unwrap_or_else(|| approximate(temperature, rainfall, false))
    }
}

fn rgb(packed: u32) -> [f32; 3] {
    [
        ((packed >> 16) & 0xFF) as f32 / 255.0,
        ((packed >> 8) & 0xFF) as f32 / 255.0,
        (packed & 0xFF) as f32 / 255.0,
    ]
}

#[derive(Debug, Clone)]
struct ColorMap {
    pixels: RgbaImage,
}

impl ColorMap {
    fn load(path: &Path) -> Option<Self> {
        let image = image::open(path).ok()?.to_rgba8();
        if image.width() != 256 || image.height() != 256 {
            return None;
        }
        Some(Self { pixels: image })
    }

    fn get(&self, temperature: f64, rainfall: f64) -> [f32; 3] {
        // ColorizerGrass/ColorizerFoliage: `i` from temperature, `j` from
        // temperature * rainfall, index `j << 8 | i`.
        let i = ((1.0 - temperature) * 255.0) as u32;
        let j = ((1.0 - rainfall * temperature) * 255.0) as u32;
        let pixel = self.pixels.get_pixel(i.min(255), j.min(255)).0;
        [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ]
    }
}

fn approximate(temperature: f64, rainfall: f64, grass: bool) -> [f32; 3] {
    let humidity = (temperature * rainfall).clamp(0.0, 1.0);
    let hue = if grass {
        0.255 + (1.0 - temperature) * 0.045 - (1.0 - humidity) * 0.030
    } else {
        0.275 + (1.0 - temperature) * 0.040 - (1.0 - humidity) * 0.025
    };
    let saturation = 0.36 + humidity * 0.34;
    let value = if grass { 0.58 } else { 0.54 } + (1.0 - temperature * 0.55) * 0.28;
    hsv_to_rgb(hue, saturation, value)
}

fn hsv_to_rgb(mut hue: f64, saturation: f64, value: f64) -> [f32; 3] {
    hue = hue.rem_euclid(1.0);
    let h = hue * 6.0;
    let sector = h.floor() as i32;
    let f = h - sector as f64;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * f);
    let t = value * (1.0 - saturation * (1.0 - f));
    let (r, g, b) = match sector.rem_euclid(6) {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    [r as f32, g as f32, b as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swamp_water_is_green_and_others_are_white() {
        let colors = BiomeColorizer::fallback();
        assert_eq!(
            colors.tint(TintKind::Water, 6),
            [224.0 / 255.0, 1.0, 174.0 / 255.0]
        );
        assert_eq!(colors.tint(TintKind::Water, 1), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn biome_subclasses_override_the_colormap() {
        let colors = BiomeColorizer::fallback();
        // BiomeGenSwamp: both resolvers are constants; water is 0xE0FFAE.
        assert_eq!(
            colors.tint(TintKind::Foliage, 6),
            [
                0x6A as f32 / 255.0,
                0x70 as f32 / 255.0,
                0x39 as f32 / 255.0
            ]
        );
        // BiomeGenMesa: grass 0x90814D, foliage 0x9E814D.
        assert_eq!(
            colors.tint(TintKind::Grass, 37),
            [
                0x90 as f32 / 255.0,
                0x81 as f32 / 255.0,
                0x4D as f32 / 255.0
            ]
        );
        assert_eq!(
            colors.tint(TintKind::Foliage, 37),
            [
                0x9E as f32 / 255.0,
                0x81 as f32 / 255.0,
                0x4D as f32 / 255.0
            ]
        );
        // Roofed forest halves the colormap toward 0x28340A; it must differ
        // from the plain forest it shares a class with.
        assert_ne!(
            colors.tint(TintKind::Grass, 29),
            colors.tint(TintKind::Grass, 4)
        );
    }

    #[test]
    fn fallback_grass_varies_with_temperature_and_rainfall() {
        let colors = BiomeColorizer::fallback();
        let desert = colors.tint(TintKind::Grass, 2);
        let plains = colors.tint(TintKind::Grass, 1);
        let jungle = colors.tint(TintKind::Grass, 21);
        assert_ne!(desert, plains);
        assert_ne!(plains, jungle);
    }

    #[test]
    fn loaded_colormaps_stay_inside_the_unit_range() {
        let Some(minecraft) = crate::test_support::minecraft() else {
            eprintln!("skipping colourmap check: run `nephele-prepare` first");
            return;
        };
        let colors = BiomeColorizer::load(&minecraft.join("textures"));
        for biome in 0..40 {
            for kind in [TintKind::Grass, TintKind::Foliage, TintKind::Water] {
                let color = colors.tint(kind, biome);
                assert!(color.iter().all(|channel| (0.0..=1.0).contains(channel)));
            }
        }
    }
}
