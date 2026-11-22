//! Entity textures used by block-entity models.
//!
//! 1.8.9 keeps chest, sign and banner textures under `textures/entity/`, not in
//! the block atlas: a chest texture is 64Ã—64 (128Ã—64 for the double chest), so
//! it cannot live in the block renderer's 16Ã—16 tile grid. This module packs
//! them into one small 2D atlas and exposes fixed UV rects, so the mesher can
//! emit model UVs without owning a GPU resource.
//!
//! The layout is a constant. A texture that is missing from the prepared tree
//! leaves its slot magenta instead of shifting every other sprite.

use std::path::Path;

use image::RgbaImage;

pub const ATLAS_WIDTH: u32 = 256;
pub const ATLAS_HEIGHT: u32 = 256;

/// One packed entity texture. `w`/`h` are the pixels used in the atlas,
/// `source_w`/`source_h` the size vanilla's model UVs are authored against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntitySprite {
    pub key: &'static str,
    /// Path under `textures/entity/`.
    pub file: &'static str,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub source_w: u32,
    pub source_h: u32,
}

impl EntitySprite {
    /// Map a model-space UV in source pixels (0..`source_w`) to atlas UVs.
    pub fn uv(&self, u: f32, v: f32) -> [f32; 2] {
        [
            (self.x as f32 + u / self.source_w as f32 * self.w as f32) / ATLAS_WIDTH as f32,
            (self.y as f32 + v / self.source_h as f32 * self.h as f32) / ATLAS_HEIGHT as f32,
        ]
    }
}

/// Shorthand for a sprite whose atlas size matches its source size.
const fn whole(
    key: &'static str,
    file: &'static str,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> EntitySprite {
    EntitySprite {
        key,
        file,
        x,
        y,
        w,
        h,
        source_w: w,
        source_h: h,
    }
}

/// The atlas layout. Doubles are 128 wide, so they get their own column.
pub const SPRITES: &[EntitySprite] = &[
    whole(
        "chest_normal_double",
        "chest/normal_double.png",
        0,
        0,
        128,
        64,
    ),
    whole(
        "chest_trapped_double",
        "chest/trapped_double.png",
        0,
        64,
        128,
        64,
    ),
    whole(
        "chest_christmas_double",
        "chest/christmas_double.png",
        0,
        192,
        128,
        64,
    ),
    whole("chest_normal", "chest/normal.png", 128, 0, 64, 64),
    whole("chest_trapped", "chest/trapped.png", 128, 64, 64, 64),
    whole("chest_ender", "chest/ender.png", 128, 128, 64, 64),
    whole("chest_christmas", "chest/christmas.png", 128, 192, 64, 64),
    whole("sign", "sign.png", 192, 0, 64, 32),
    whole("banner_base", "banner/base.png", 192, 64, 64, 64),
];

pub fn sprite(key: &str) -> Option<&'static EntitySprite> {
    SPRITES.iter().find(|sprite| sprite.key == key)
}

/// The packed RGBA atlas.
#[derive(Debug, Clone)]
pub struct EntityAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    missing: Vec<&'static str>,
}

impl EntityAtlas {
    /// A 1Ã—1 white atlas: enough for a renderer that never draws a block
    /// entity (the launcher preview, tests).
    pub fn empty() -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: vec![255; 4],
            missing: Vec::new(),
        }
    }

    pub fn load(textures_dir: &Path) -> Self {
        let mut image =
            RgbaImage::from_pixel(ATLAS_WIDTH, ATLAS_HEIGHT, image::Rgba([255, 0, 255, 255]));
        let mut missing = Vec::new();
        for sprite in SPRITES {
            let path = textures_dir.join("entity").join(sprite.file);
            let Ok(decoded) = image::open(&path) else {
                missing.push(sprite.key);
                continue;
            };
            let decoded = decoded.to_rgba8();
            if decoded.width() != sprite.source_w || decoded.height() != sprite.source_h {
                // Wrong size means the wrong file; keep the slot magenta so it
                // is obvious instead of silently stretching.
                missing.push(sprite.key);
                continue;
            }
            image::imageops::replace(&mut image, &decoded, sprite.x as i64, sprite.y as i64);
        }
        if !missing.is_empty() || std::env::var_os("NEPHELE_ENTITY_ATLAS_DEBUG").is_some() {
            println!(
                "Entity atlas: {} sprites, missing {:?}",
                SPRITES.len(),
                missing
            );
        }
        if let Ok(path) = std::env::var("NEPHELE_ENTITY_ATLAS_DUMP") {
            let _ = image.save(path);
        }
        Self {
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
            pixels: image.into_raw(),
            missing,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Sprite keys that did not load; empty on a complete prepared tree.
    pub fn missing(&self) -> &[&'static str] {
        &self.missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sprite_fits_inside_the_atlas_without_overlapping() {
        for sprite in SPRITES {
            assert!(sprite.x + sprite.w <= ATLAS_WIDTH, "{}", sprite.key);
            assert!(sprite.y + sprite.h <= ATLAS_HEIGHT, "{}", sprite.key);
        }
        for (index, a) in SPRITES.iter().enumerate() {
            for b in &SPRITES[index + 1..] {
                let overlaps =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlaps, "{} overlaps {}", a.key, b.key);
            }
        }
    }

    #[test]
    fn uv_maps_the_source_origin_to_the_slot_corner() {
        let chest = sprite("chest_normal").expect("chest sprite");
        assert_eq!(chest.uv(0.0, 0.0), [128.0 / 256.0, 0.0]);
        assert_eq!(chest.uv(64.0, 64.0), [192.0 / 256.0, 64.0 / 256.0]);

        let double = sprite("chest_normal_double").expect("double sprite");
        assert_eq!(double.uv(128.0, 0.0), [128.0 / 256.0, 0.0]);
    }

    #[test]
    fn the_empty_atlas_is_a_single_white_pixel() {
        let atlas = EntityAtlas::empty();
        assert_eq!((atlas.width(), atlas.height()), (1, 1));
        assert_eq!(atlas.pixels(), &[255, 255, 255, 255]);
    }
}
