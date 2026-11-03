use std::collections::HashMap;
use std::path::Path;

use image::{Rgba, RgbaImage};

use crate::block_textures::{all_texture_names, MISSING_TEXTURE};
use crate::item_textures::all_item_texture_layers;

/// Vanilla block textures are 16×16.
pub const TILE_SIZE: u32 = 16;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("asset directory `{0}` does not exist")]
    MissingDirectory(String),
    #[error("could not read `{path}`: {source}")]
    Image {
        path: String,
        #[source]
        source: image::ImageError,
    },
    #[error("no block textures could be loaded from `{0}`")]
    NoTextures(String),
    #[error("the block texture array needs at least one layer")]
    EmptyArray,
}

/// One GPU texture-array layer per referenced block or supported item texture.
///
/// Every block face samples its own layer, so unlike an atlas there is no
/// bleeding between neighbouring textures and mipmapping stays safe.
#[derive(Debug, Clone)]
pub struct TextureArray {
    tile_size: u32,
    layer_count: u32,
    pixels: Vec<u8>,
    index: HashMap<&'static str, u32>,
    missing: u32,
}

impl TextureArray {
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    pub fn layer_count(&self) -> u32 {
        self.layer_count
    }

    pub fn bytes_per_layer(&self) -> usize {
        (self.tile_size * self.tile_size * 4) as usize
    }

    /// RGBA8 data for every layer, laid out layer by layer.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn layer(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Layer for `name`, falling back to the generated "missing" texture.
    pub fn layer_or_missing(&self, name: &str) -> u32 {
        self.layer(name).unwrap_or(self.missing)
    }

    pub fn name_of(&self, layer: u32) -> Option<&'static str> {
        self.index
            .iter()
            .find(|(_, value)| **value == layer)
            .map(|(name, _)| *name)
    }

    /// Load referenced block textures and supported item textures from the
    /// vanilla `textures/blocks` and sibling `textures/items` directories.
    pub fn load_from_dir(blocks_dir: &Path) -> Result<Self, AssetError> {
        if !blocks_dir.is_dir() {
            return Err(AssetError::MissingDirectory(
                blocks_dir.display().to_string(),
            ));
        }

        let mut tiles: Vec<(&'static str, RgbaImage)> = Vec::new();
        for name in all_texture_names() {
            let path = blocks_dir.join(format!("{name}.png"));
            if !path.is_file() {
                continue;
            }
            let image = image::open(&path).map_err(|source| AssetError::Image {
                path: path.display().to_string(),
                source,
            })?;
            let mut tile = to_tile(&image.to_rgba8());

            // 1.8 stores grass and foliage as grayscale and tints them at
            // runtime with the biome colour. Bake the plains colour in.
            match name {
                "grass_top" => tint(&mut tile, GRASS_TINT),
                "leaves_oak" | "leaves_acacia" => tint(&mut tile, FOLIAGE_TINT),
                "grass_side" => {
                    if let Ok(overlay) = image::open(blocks_dir.join("grass_side_overlay.png")) {
                        let mut overlay = to_tile(&overlay.to_rgba8());
                        tint(&mut overlay, GRASS_TINT);
                        blend_over(&mut tile, &overlay);
                    }
                }
                _ => {}
            }

            tiles.push((name, tile));
        }

        if tiles.is_empty() {
            return Err(AssetError::NoTextures(blocks_dir.display().to_string()));
        }

        if let Some(items_dir) = blocks_dir
            .parent()
            .map(|textures_dir| textures_dir.join("items"))
            .filter(|items_dir| items_dir.is_dir())
        {
            for (layer, file) in all_item_texture_layers() {
                let path = items_dir.join(format!("{file}.png"));
                if !path.is_file() {
                    continue;
                }
                let image = image::open(&path).map_err(|source| AssetError::Image {
                    path: path.display().to_string(),
                    source,
                })?;
                tiles.push((layer, to_tile(&image.to_rgba8())));
            }
        }
        Ok(Self::from_tiles(tiles))
    }

    /// Build the array from already-decoded 16×16 RGBA tiles. A generated
    /// "missing" tile is appended if the caller didn't provide one.
    pub fn from_tiles(mut tiles: Vec<(&'static str, RgbaImage)>) -> Self {
        if !tiles.iter().any(|(name, _)| *name == MISSING_TEXTURE) {
            tiles.push((MISSING_TEXTURE, missing_texture()));
        }
        if tiles.is_empty() {
            tiles.push((MISSING_TEXTURE, missing_texture()));
        }

        let layer_count = tiles.len() as u32;
        let mut pixels =
            Vec::with_capacity(layer_count as usize * (TILE_SIZE * TILE_SIZE * 4) as usize);
        let mut index = HashMap::with_capacity(tiles.len());
        let mut missing = 0;

        for (layer, (name, tile)) in tiles.into_iter().enumerate() {
            let layer = layer as u32;
            if name == MISSING_TEXTURE {
                missing = layer;
            }
            index.insert(name, layer);

            for y in 0..TILE_SIZE {
                for x in 0..TILE_SIZE {
                    let pixel = *tile.get_pixel(x.min(tile.width() - 1), y.min(tile.height() - 1));
                    pixels.extend_from_slice(&pixel.0);
                }
            }
        }

        Self {
            tile_size: TILE_SIZE,
            layer_count,
            pixels,
            index,
            missing,
        }
    }
}

/// Plains-biome grass colour (vanilla `0x79C05A`).
const GRASS_TINT: [f32; 3] = [0.475, 0.753, 0.353];
/// Plains-biome foliage colour (vanilla `0x59AE30`).
const FOLIAGE_TINT: [f32; 3] = [0.349, 0.682, 0.188];

fn tint(image: &mut RgbaImage, color: [f32; 3]) {
    for pixel in image.pixels_mut() {
        let [r, g, b, _] = pixel.0;
        pixel.0[0] = (r as f32 * color[0]) as u8;
        pixel.0[1] = (g as f32 * color[1]) as u8;
        pixel.0[2] = (b as f32 * color[2]) as u8;
    }
}

fn blend_over(base: &mut RgbaImage, overlay: &RgbaImage) {
    for y in 0..base.height().min(overlay.height()) {
        for x in 0..base.width().min(overlay.width()) {
            let src = overlay.get_pixel(x, y).0;
            let alpha = src[3] as f32 / 255.0;
            if alpha <= 0.0 {
                continue;
            }
            let dst = base.get_pixel(x, y).0;
            let mix = |s: u8, d: u8| -> u8 { (s as f32 * alpha + d as f32 * (1.0 - alpha)) as u8 };
            base.put_pixel(
                x,
                y,
                Rgba([
                    mix(src[0], dst[0]),
                    mix(src[1], dst[1]),
                    mix(src[2], dst[2]),
                    255,
                ]),
            );
        }
    }
}

/// Crop the first animation frame and normalise to 16×16.
fn to_tile(image: &RgbaImage) -> RgbaImage {
    let size = image.width().min(image.height());
    let cropped = image::imageops::crop_imm(image, 0, 0, size, size).to_image();
    if size == TILE_SIZE && cropped.height() == TILE_SIZE {
        return cropped;
    }
    image::imageops::resize(
        &cropped,
        TILE_SIZE,
        TILE_SIZE,
        image::imageops::FilterType::Nearest,
    )
}

/// Magenta/black checkerboard used when a texture is absent.
fn missing_texture() -> RgbaImage {
    let mut image = RgbaImage::new(TILE_SIZE, TILE_SIZE);
    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            let color = if checker {
                Rgba([255, 0, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            };
            image.put_pixel(x, y, color);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(color: Rgba<u8>) -> RgbaImage {
        RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, color)
    }

    #[test]
    fn layers_are_sequential_and_unique() {
        let tiles = vec![
            ("a", solid(Rgba([255, 0, 0, 255]))),
            ("b", solid(Rgba([0, 255, 0, 255]))),
            ("c", solid(Rgba([0, 0, 255, 255]))),
        ];
        let array = TextureArray::from_tiles(tiles);
        assert_eq!(array.layer("a"), Some(0));
        assert_eq!(array.layer("b"), Some(1));
        assert_eq!(array.layer("c"), Some(2));
        // "missing" is appended last.
        assert_eq!(array.layer(MISSING_TEXTURE), Some(3));
        assert_eq!(array.layer_count(), 4);
    }

    #[test]
    fn pixels_are_layer_major() {
        let array = TextureArray::from_tiles(vec![
            ("a", solid(Rgba([10, 0, 0, 255]))),
            ("b", solid(Rgba([20, 0, 0, 255]))),
        ]);
        assert_eq!(array.pixels().len(), 3 * array.bytes_per_layer());
        let bytes = array.bytes_per_layer();
        assert_eq!(&array.pixels()[0..4], &[10, 0, 0, 255]);
        assert_eq!(&array.pixels()[bytes..bytes + 4], &[20, 0, 0, 255]);
        // every pixel of a layer is the same colour
        let layer_a = &array.pixels()[0..bytes];
        assert!(layer_a.chunks_exact(4).all(|p| p == [10, 0, 0, 255]));
    }

    #[test]
    fn unknown_names_fall_back_to_missing() {
        let array = TextureArray::from_tiles(vec![("x", solid(Rgba([9, 9, 9, 255])))]);
        assert!(array.layer("nope").is_none());
        assert_eq!(
            array.layer_or_missing("nope"),
            array.layer(MISSING_TEXTURE).unwrap()
        );
    }

    #[test]
    fn namespaced_item_layers_do_not_collide_with_block_layers() {
        let array = TextureArray::from_tiles(vec![
            ("brick", solid(Rgba([255, 0, 0, 255]))),
            ("item:brick", solid(Rgba([0, 255, 0, 255]))),
        ]);
        assert_ne!(array.layer("brick"), array.layer("item:brick"));
        assert_eq!(
            array.name_of(array.layer("item:brick").unwrap()),
            Some("item:brick")
        );
    }

    #[test]
    fn prepared_crafting_items_load_into_the_texture_array_when_available() {
        let Some(minecraft) = crate::test_support::minecraft() else {
            eprintln!("skipping item texture check: run `nephele-prepare` first");
            return;
        };
        let atlas = TextureArray::load_from_dir(&minecraft.join("textures/blocks"))
            .expect("prepared block textures load");
        let items_dir = minecraft.join("textures/items");
        for (layer, file) in all_item_texture_layers() {
            assert!(
                items_dir.join(format!("{file}.png")).is_file(),
                "missing item file {file}"
            );
            assert!(
                atlas.layer(layer).is_some(),
                "missing prepared layer {layer}"
            );
        }
    }

    #[test]
    fn tall_animated_textures_are_cropped_to_the_first_frame() {
        let mut tall = RgbaImage::from_pixel(16, 64, Rgba([0, 0, 255, 255]));
        tall.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let tile = to_tile(&tall);
        assert_eq!((tile.width(), tile.height()), (TILE_SIZE, TILE_SIZE));
        assert_eq!(*tile.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn tint_only_scales_rgb() {
        let mut image = RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, Rgba([100, 200, 50, 128]));
        tint(&mut image, [0.5, 0.5, 0.5]);
        let pixel = image.get_pixel(0, 0).0;
        assert_eq!(pixel, [50, 100, 25, 128]);
    }

    #[test]
    fn blend_keeps_base_where_overlay_is_transparent() {
        let mut base = RgbaImage::from_pixel(16, 16, Rgba([10, 20, 30, 255]));
        let mut overlay = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        overlay.put_pixel(0, 0, Rgba([200, 100, 0, 255]));
        blend_over(&mut base, &overlay);
        assert_eq!(*base.get_pixel(0, 0), Rgba([200, 100, 0, 255]));
        assert_eq!(*base.get_pixel(1, 0), Rgba([10, 20, 30, 255]));
    }
}
