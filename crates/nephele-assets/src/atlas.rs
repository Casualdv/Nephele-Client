use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};

use crate::block_textures::{referenced_texture_names, MISSING_TEXTURE};
use crate::item_textures::all_item_texture_layers;

/// Vanilla block textures are 16×16.
pub const TILE_SIZE: u32 = 16;
/// Tiles packed into one texture-array layer, in a 4×4 grid.
pub const TILES_PER_LAYER: u32 = 16;
/// Tiles per row inside one layer.
pub const TILES_PER_ROW: u32 = 4;
/// Width and height of one packed texture-array layer.
pub const LAYER_SIZE: u32 = TILE_SIZE * TILES_PER_ROW;

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

/// Packed block and item textures.
///
/// Sixteen 16×16 tiles share one 64×64 texture-array layer, so the whole
/// vanilla set fits in a handful of layers instead of one layer per texture.
/// [`TextureArray::slot`] returns a *tile slot*: the GPU layer is
/// `slot / TILES_PER_LAYER` and the 4×4 position inside it is
/// `slot % TILES_PER_LAYER`. The shaders perform that unpack.
#[derive(Debug, Clone)]
pub struct TextureArray {
    tile_size: u32,
    layer_size: u32,
    layer_count: u32,
    tile_count: u32,
    pixels: Vec<u8>,
    index: HashMap<String, u32>,
    missing: u32,
    unresolved: Vec<&'static str>,
}

impl TextureArray {
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// Width and height of one packed layer (64 for 16×16 tiles).
    pub fn layer_size(&self) -> u32 {
        self.layer_size
    }

    /// Number of texture-array layers uploaded to the GPU.
    pub fn layer_count(&self) -> u32 {
        self.layer_count
    }

    /// Number of individual 16×16 tiles across all layers.
    pub fn tile_count(&self) -> u32 {
        self.tile_count
    }

    pub fn bytes_per_layer(&self) -> usize {
        (self.layer_size * self.layer_size * 4) as usize
    }

    /// RGBA8 data for every layer, laid out layer by layer.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Packed tile slot for `name`.
    pub fn slot(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Tile slot for `name`, falling back to the generated "missing" tile.
    pub fn slot_or_missing(&self, name: &str) -> u32 {
        self.slot(name).unwrap_or(self.missing)
    }

    pub fn name_of(&self, slot: u32) -> Option<&str> {
        self.index
            .iter()
            .find(|(_, value)| **value == slot)
            .map(|(name, _)| name.as_str())
    }

    /// Texture names referenced by the block table that were not present in
    /// `textures/blocks`, plus `missing` if a 1.8 variant still resolves to it.
    pub fn unresolved_names(&self) -> &[&'static str] {
        &self.unresolved
    }

    /// Load every `textures/blocks/*.png` plus the supported item textures.
    ///
    /// Loading the directory instead of a curated name list means a texture
    /// that exists in the jar can never be dropped because the hand-written
    /// table forgot it. `unresolved_names` still reports names the block table
    /// asks for but the directory does not contain.
    pub fn load_from_dir(blocks_dir: &Path) -> Result<Self, AssetError> {
        if !blocks_dir.is_dir() {
            return Err(AssetError::MissingDirectory(
                blocks_dir.display().to_string(),
            ));
        }

        let mut paths: Vec<PathBuf> = std::fs::read_dir(blocks_dir)
            .map_err(|_| AssetError::MissingDirectory(blocks_dir.display().to_string()))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))
            .collect();
        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        let mut tiles: Vec<(String, RgbaImage)> = Vec::with_capacity(paths.len());
        for path in paths {
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
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

            tiles.push((name.to_string(), tile));
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
                tiles.push((layer.to_string(), to_tile(&image.to_rgba8())));
            }
        }

        let mut array = Self::from_tiles(tiles);
        let mut unresolved: Vec<&'static str> = Vec::new();
        for name in referenced_texture_names() {
            if (name == MISSING_TEXTURE || array.slot(name).is_none())
                && !unresolved.contains(&name)
            {
                unresolved.push(name);
            }
        }
        array.unresolved = unresolved;
        Ok(array)
    }

    /// Build the packed array from already-decoded 16×16 RGBA tiles. A
    /// generated "missing" tile is appended if the caller didn't provide one.
    pub fn from_tiles(mut tiles: Vec<(String, RgbaImage)>) -> Self {
        if !tiles.iter().any(|(name, _)| name == MISSING_TEXTURE) {
            tiles.push((MISSING_TEXTURE.to_string(), missing_texture()));
        }

        let tile_count = tiles.len() as u32;
        let layer_count = tile_count.div_ceil(TILES_PER_LAYER);
        let mut pixels = vec![0u8; layer_count as usize * (LAYER_SIZE * LAYER_SIZE * 4) as usize];
        let mut index = HashMap::with_capacity(tiles.len());
        let mut missing = 0;

        for (slot, (name, tile)) in tiles.into_iter().enumerate() {
            let slot = slot as u32;
            let layer = slot / TILES_PER_LAYER;
            let local = slot % TILES_PER_LAYER;
            let tile_x = local % TILES_PER_ROW;
            let tile_y = local / TILES_PER_ROW;
            if name == MISSING_TEXTURE {
                missing = slot;
            }

            for y in 0..TILE_SIZE {
                for x in 0..TILE_SIZE {
                    let pixel = *tile.get_pixel(x.min(tile.width() - 1), y.min(tile.height() - 1));
                    let px = tile_x * TILE_SIZE + x;
                    let py = tile_y * TILE_SIZE + y;
                    let offset = ((layer * LAYER_SIZE + py) * LAYER_SIZE + px) as usize * 4;
                    pixels[offset..offset + 4].copy_from_slice(&pixel.0);
                }
            }

            index.insert(name, slot);
        }

        Self {
            tile_size: TILE_SIZE,
            layer_size: LAYER_SIZE,
            layer_count,
            tile_count,
            pixels,
            index,
            missing,
            unresolved: Vec::new(),
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

    fn named(name: &str, color: Rgba<u8>) -> (String, RgbaImage) {
        (name.to_string(), solid(color))
    }

    #[test]
    fn layers_are_sequential_and_unique() {
        let tiles = vec![
            named("a", Rgba([255, 0, 0, 255])),
            named("b", Rgba([0, 255, 0, 255])),
            named("c", Rgba([0, 0, 255, 255])),
        ];
        let array = TextureArray::from_tiles(tiles);
        assert_eq!(array.slot("a"), Some(0));
        assert_eq!(array.slot("b"), Some(1));
        assert_eq!(array.slot("c"), Some(2));
        // "missing" is appended last.
        assert_eq!(array.slot(MISSING_TEXTURE), Some(3));
        assert_eq!(array.tile_count(), 4);
        assert_eq!(array.layer_count(), 1);
        assert_eq!(array.layer_size(), LAYER_SIZE);
    }

    #[test]
    fn sixteen_tiles_share_one_layer() {
        let tiles = (0..17)
            .map(|index| named(&format!("t{index}"), Rgba([index as u8, 0, 0, 255])))
            .collect();
        let array = TextureArray::from_tiles(tiles);
        assert_eq!(array.layer_count(), 2);
        assert_eq!(array.slot("t0"), Some(0));
        assert_eq!(array.slot("t15"), Some(15));
        assert_eq!(array.slot("t16"), Some(16));
        assert_eq!(array.name_of(16), Some("t16"));
    }

    #[test]
    fn pixels_are_layer_major_and_tile_packed() {
        let array = TextureArray::from_tiles(vec![
            named("a", Rgba([10, 0, 0, 255])),
            named("b", Rgba([20, 0, 0, 255])),
        ]);
        assert_eq!(array.pixels().len(), array.bytes_per_layer());
        let bytes = array.bytes_per_layer();
        // Tile 0 sits at (0, 0), tile 1 at (16, 0) inside the same layer.
        assert_eq!(&array.pixels()[0..4], &[10, 0, 0, 255]);
        assert_eq!(&array.pixels()[16 * 4..16 * 4 + 4], &[20, 0, 0, 255]);
        assert!(array.pixels()[..16 * 4]
            .chunks_exact(4)
            .all(|p| p == [10, 0, 0, 255]));
        assert!(array.pixels()[bytes..].is_empty());
    }

    #[test]
    fn unknown_names_fall_back_to_missing() {
        let array = TextureArray::from_tiles(vec![named("x", Rgba([9, 9, 9, 255]))]);
        assert!(array.slot("nope").is_none());
        assert_eq!(
            array.slot_or_missing("nope"),
            array.slot(MISSING_TEXTURE).unwrap()
        );
    }

    #[test]
    fn namespaced_item_layers_do_not_collide_with_block_layers() {
        let array = TextureArray::from_tiles(vec![
            named("brick", Rgba([255, 0, 0, 255])),
            named("item:brick", Rgba([0, 255, 0, 255])),
        ]);
        assert_ne!(array.slot("brick"), array.slot("item:brick"));
        assert_eq!(
            array.name_of(array.slot("item:brick").unwrap()),
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
                atlas.slot(layer).is_some(),
                "missing prepared layer {layer}"
            );
        }
    }

    #[test]
    fn prepared_block_textures_report_no_missing_names() {
        let Some(minecraft) = crate::test_support::minecraft() else {
            eprintln!("skipping missing-name check: run `nephele-prepare` first");
            return;
        };
        let atlas = TextureArray::load_from_dir(&minecraft.join("textures/blocks"))
            .expect("prepared block textures load");
        assert!(
            atlas.unresolved_names().is_empty(),
            "unresolved block textures: {:?}",
            atlas.unresolved_names()
        );
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
