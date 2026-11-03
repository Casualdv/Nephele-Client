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

/// One animation strip packed into the separate 2D animation atlas.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextureAnimation {
    /// Top-left corner of the strip in the animation atlas, in UV space.
    pub origin: [f32; 2],
    /// Size of one frame in UV space.
    pub frame_size: [f32; 2],
    /// Horizontal distance between two frames in UV space.
    pub step: f32,
    pub frames: u32,
    pub frametime: f32,
}

/// One GPU texture-array layer per referenced block or supported item texture.
///
/// Every block face samples its own layer, so unlike an atlas there is no
/// bleeding between neighbouring textures and mipmapping stays safe. Animated
/// textures keep their first frame in the array (for UI/fallback use) and are
/// sampled from [`TextureArray::animation_atlas`] at runtime.
#[derive(Debug, Clone)]
pub struct TextureArray {
    tile_size: u32,
    layer_count: u32,
    pixels: Vec<u8>,
    index: HashMap<&'static str, u32>,
    missing: u32,
    mip_levels: u32,
    mips: Vec<Vec<u8>>,
    animations: Vec<TextureAnimation>,
    animation_index: HashMap<&'static str, u32>,
    animation_width: u32,
    animation_height: u32,
    animation_pixels: Vec<u8>,
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

    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    pub fn mip_size(&self, level: u32) -> u32 {
        (self.tile_size >> level.min(self.mip_levels.saturating_sub(1))).max(1)
    }

    /// RGBA8 data for one mip level, laid out layer by layer.
    pub fn mip_pixels(&self, level: u32) -> &[u8] {
        if level == 0 {
            &self.pixels
        } else {
            &self.mips[(level - 1) as usize]
        }
    }

    /// RGBA8 data for level 0, laid out layer by layer.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn layer(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    pub fn animation(&self, name: &str) -> Option<u32> {
        self.animation_index.get(name).copied()
    }

    pub fn animations(&self) -> &[TextureAnimation] {
        &self.animations
    }

    /// Packed 2D animation strip atlas and its dimensions.
    pub fn animation_atlas(&self) -> (&[u8], u32, u32) {
        (
            &self.animation_pixels,
            self.animation_width,
            self.animation_height,
        )
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
        let mut animation_strips: Vec<(&'static str, Vec<RgbaImage>, u32)> = Vec::new();
        for name in all_texture_names() {
            let path = blocks_dir.join(format!("{name}.png"));
            if !path.is_file() {
                continue;
            }
            let image = image::open(&path).map_err(|source| AssetError::Image {
                path: path.display().to_string(),
                source,
            })?;
            let rgba = image.to_rgba8();
            let mut tile = to_tile(&rgba);
            if let Some((indices, frametime)) = read_mcmeta(&path) {
                let frames = animation_frames(&rgba, &indices);
                if frames.len() > 1 {
                    animation_strips.push((name, frames, frametime));
                }
            }

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
        Ok(Self::from_tiles_and_animations(tiles, animation_strips))
    }

    /// Build the array from already-decoded 16×16 RGBA tiles. A generated
    /// "missing" tile is appended if the caller didn't provide one.
    pub fn from_tiles(tiles: Vec<(&'static str, RgbaImage)>) -> Self {
        Self::from_tiles_and_animations(tiles, Vec::new())
    }

    fn from_tiles_and_animations(
        mut tiles: Vec<(&'static str, RgbaImage)>,
        animation_strips: Vec<(&'static str, Vec<RgbaImage>, u32)>,
    ) -> Self {
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

        let mips = generate_mips(&pixels, TILE_SIZE, layer_count);
        let animation_width = animation_strips
            .iter()
            .map(|(_, frames, _)| frames.len() as u32 * TILE_SIZE)
            .max()
            .unwrap_or(TILE_SIZE)
            .max(TILE_SIZE);
        let animation_height = (animation_strips.len() as u32 * TILE_SIZE).max(TILE_SIZE);
        let mut animation_pixels = vec![0u8; (animation_width * animation_height * 4) as usize];
        let mut animation_index = HashMap::with_capacity(animation_strips.len());
        let mut animations = Vec::with_capacity(animation_strips.len());

        for (row, (name, frames, frametime)) in animation_strips.into_iter().enumerate() {
            let animation = animations.len() as u32;
            animation_index.insert(name, animation);
            for (frame_index, frame) in frames.iter().enumerate() {
                let x0 = frame_index as u32 * TILE_SIZE;
                let y0 = row as u32 * TILE_SIZE;
                for y in 0..TILE_SIZE {
                    for x in 0..TILE_SIZE {
                        let pixel =
                            *frame.get_pixel(x.min(frame.width() - 1), y.min(frame.height() - 1));
                        let offset = (((y0 + y) * animation_width + x0 + x) * 4) as usize;
                        animation_pixels[offset..offset + 4].copy_from_slice(&pixel.0);
                    }
                }
            }
            animations.push(TextureAnimation {
                origin: [0.0, row as f32 * TILE_SIZE as f32 / animation_height as f32],
                frame_size: [
                    TILE_SIZE as f32 / animation_width as f32,
                    TILE_SIZE as f32 / animation_height as f32,
                ],
                step: TILE_SIZE as f32 / animation_width as f32,
                frames: frames.len() as u32,
                frametime: frametime.max(1) as f32,
            });
        }

        Self {
            tile_size: TILE_SIZE,
            layer_count,
            pixels,
            index,
            missing,
            mip_levels: TILE_SIZE.trailing_zeros() + 1,
            mips,
            animations,
            animation_index,
            animation_width,
            animation_height,
            animation_pixels,
        }
    }
}

/// Read `.png.mcmeta` animation metadata. Returns frame indices (empty means
/// "all frames in order") and the default frame time in game ticks.
fn read_mcmeta(png_path: &Path) -> Option<(Vec<u32>, u32)> {
    let text = std::fs::read_to_string(png_path.with_extension("png.mcmeta")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let animation = json.get("animation")?;
    let frametime = animation
        .get("frametime")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as u32;
    let indices = animation
        .get("frames")
        .and_then(serde_json::Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .filter_map(|frame| {
                    frame
                        .as_u64()
                        .or_else(|| frame.get("index").and_then(serde_json::Value::as_u64))
                })
                .map(|index| index as u32)
                .collect()
        })
        .unwrap_or_default();
    Some((indices, frametime))
}

fn animation_frames(image: &RgbaImage, indices: &[u32]) -> Vec<RgbaImage> {
    let size = image.width().min(image.height());
    if size == 0 {
        return Vec::new();
    }
    let available = image.height() / size;
    let mut indices = if indices.is_empty() {
        (0..available).collect::<Vec<_>>()
    } else {
        indices.to_vec()
    };
    indices.retain(|index| *index < available);
    if indices.is_empty() {
        indices.push(0);
    }
    indices
        .into_iter()
        .map(|index| {
            let cropped = image::imageops::crop_imm(image, 0, index * size, size, size).to_image();
            if size == TILE_SIZE {
                cropped
            } else {
                image::imageops::resize(
                    &cropped,
                    TILE_SIZE,
                    TILE_SIZE,
                    image::imageops::FilterType::Nearest,
                )
            }
        })
        .collect()
}

fn generate_mips(pixels: &[u8], tile: u32, layers: u32) -> Vec<Vec<u8>> {
    let levels = tile.trailing_zeros();
    let mut mips = Vec::with_capacity(levels as usize);
    let mut previous = pixels.to_vec();
    let mut previous_size = tile;
    for _ in 0..levels {
        let next_size = (previous_size / 2).max(1);
        let mut next = vec![0u8; (layers * next_size * next_size * 4) as usize];
        for layer in 0..layers {
            for y in 0..next_size {
                for x in 0..next_size {
                    let mut rgb = [0.0f32; 3];
                    let mut alpha = 0.0;
                    let mut count = 0.0;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (x * 2 + dx).min(previous_size - 1);
                            let sy = (y * 2 + dy).min(previous_size - 1);
                            let offset =
                                ((layer * previous_size * previous_size + sy * previous_size + sx)
                                    * 4) as usize;
                            let a = previous[offset + 3] as f32 / 255.0;
                            rgb[0] += previous[offset] as f32 * a;
                            rgb[1] += previous[offset + 1] as f32 * a;
                            rgb[2] += previous[offset + 2] as f32 * a;
                            alpha += a;
                            count += 1.0;
                        }
                    }
                    let divisor = if alpha > 0.0 { alpha } else { count };
                    let offset = ((layer * next_size * next_size + y * next_size + x) * 4) as usize;
                    next[offset] = (rgb[0] / divisor).round().clamp(0.0, 255.0) as u8;
                    next[offset + 1] = (rgb[1] / divisor).round().clamp(0.0, 255.0) as u8;
                    next[offset + 2] = (rgb[2] / divisor).round().clamp(0.0, 255.0) as u8;
                    next[offset + 3] = (alpha / count * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        mips.push(next.clone());
        previous = next;
        previous_size = next_size;
    }
    mips
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
    fn prepared_animated_textures_expose_all_mcmeta_frames() {
        let Some(minecraft) = crate::test_support::minecraft() else {
            eprintln!("skipping animation check: run `nephele-prepare` first");
            return;
        };
        let atlas = TextureArray::load_from_dir(&minecraft.join("textures/blocks"))
            .expect("prepared block textures load");
        let water = atlas.animation("water_still").expect("water animation");
        assert_eq!(atlas.animations()[water as usize].frames, 32);
        assert_eq!(atlas.animations()[water as usize].frametime, 2.0);
        let lava = atlas.animation("lava_still").expect("lava animation");
        assert_eq!(atlas.animations()[lava as usize].frames, 38);
        let (pixels, width, height) = atlas.animation_atlas();
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(width >= 608);
    }

    #[test]
    fn mip_chain_halves_every_layer() {
        let array = TextureArray::from_tiles(vec![(
            "a",
            RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, Rgba([64, 128, 192, 255])),
        )]);
        assert_eq!(array.mip_levels(), 5);
        assert_eq!(array.mip_size(0), 16);
        assert_eq!(array.mip_size(1), 8);
        assert_eq!(array.mip_size(4), 1);
        for level in 1..array.mip_levels() {
            let size = array.mip_size(level);
            assert_eq!(
                array.mip_pixels(level).len(),
                (size * size * 4 * array.layer_count()) as usize
            );
        }
        assert_eq!(&array.mip_pixels(1)[..4], &[64, 128, 192, 255]);
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
