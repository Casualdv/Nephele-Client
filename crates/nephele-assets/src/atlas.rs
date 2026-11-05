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
/// Width and height of one packed texture-array layer at mip level 0.
pub const LAYER_SIZE: u32 = TILE_SIZE * TILES_PER_ROW;

/// Highest mip level the packed block sampler may use.
///
/// Sixteen tiles share a 64×64 layer, so a tile is 4×4 texels at level 2 and
/// its own texels still cover the whole face. From level 3 the tile is 2×2 and
/// at level 4 it is a single texel, where a bilinear fetch at the face edge is
/// half the neighbouring tile. The chain is still generated and uploaded in
/// full; only sampling is capped.
pub const BLOCK_LOD_MAX: f32 = 2.0;

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

/// Packed block and item textures.
///
/// Sixteen 16×16 tiles share one 64×64 texture-array layer, so the whole
/// vanilla set fits in a handful of layers instead of one layer per texture.
/// [`TextureArray::slot`] returns a *tile slot*: the GPU layer is
/// `slot / TILES_PER_LAYER` and the 4×4 position inside it is
/// `slot % TILES_PER_LAYER`. The shaders perform that unpack.
///
/// Animated textures keep their first frame in the array for UI/fallback use
/// and are sampled from [`TextureArray::animation_atlas`] at runtime. Mips are
/// generated per tile, then packed into each layer's mip level, so the
/// 16→1 chain is preserved without a mip level averaging two different tiles
/// together.
#[derive(Debug, Clone)]
pub struct TextureArray {
    tile_size: u32,
    layer_size: u32,
    layer_count: u32,
    tile_count: u32,
    pixels: Vec<u8>,
    index: HashMap<String, u32>,
    missing: u32,
    mip_levels: u32,
    mips: Vec<Vec<u8>>,
    animations: Vec<TextureAnimation>,
    animation_index: HashMap<String, u32>,
    animation_width: u32,
    animation_height: u32,
    animation_pixels: Vec<u8>,
    unresolved: Vec<&'static str>,
}

impl TextureArray {
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// Width and height of one packed layer at mip level 0 (64 for 16×16 tiles).
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

    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    /// Width and height of one packed layer at `level` (64, 32, 16, 8, 4).
    pub fn mip_size(&self, level: u32) -> u32 {
        (self.layer_size >> level.min(self.mip_levels.saturating_sub(1))).max(1)
    }

    /// Width and height of one tile at `level` (16, 8, 4, 2, 1).
    pub fn tile_mip_size(&self, level: u32) -> u32 {
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

    /// Packed tile slot for `name`.
    pub fn slot(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Tile slot for `name`, falling back to the generated "missing" tile.
    pub fn slot_or_missing(&self, name: &str) -> u32 {
        self.slot(name).unwrap_or(self.missing)
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
        let mut animation_strips: Vec<(String, Vec<RgbaImage>, u32)> = Vec::new();
        for path in paths {
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let image = image::open(&path).map_err(|source| AssetError::Image {
                path: path.display().to_string(),
                source,
            })?;
            let rgba = image.to_rgba8();
            let mut tile = to_tile(&rgba);
            if let Some((indices, frametime)) = read_mcmeta(&path) {
                let frames = animation_frames(&rgba, &indices);
                if frames.len() > 1 {
                    animation_strips.push((name.to_string(), frames, frametime));
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

        let mut array = Self::from_tiles_and_animations(tiles, animation_strips);
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

    /// Build the array from already-decoded 16×16 RGBA tiles. A generated
    /// "missing" tile is appended if the caller didn't provide one.
    pub fn from_tiles(tiles: Vec<(String, RgbaImage)>) -> Self {
        Self::from_tiles_and_animations(tiles, Vec::new())
    }

    fn from_tiles_and_animations(
        mut tiles: Vec<(String, RgbaImage)>,
        animation_strips: Vec<(String, Vec<RgbaImage>, u32)>,
    ) -> Self {
        if !tiles.iter().any(|(name, _)| name == MISSING_TEXTURE) {
            tiles.push((MISSING_TEXTURE.to_string(), missing_texture()));
        }

        let tile_count = tiles.len() as u32;
        let layer_count = tile_count.div_ceil(TILES_PER_LAYER);
        let mut index = HashMap::with_capacity(tiles.len());
        let mut missing = 0;

        // One 16→1 mip chain per tile, then pack each level into its layer.
        let mut chains: Vec<Vec<RgbaImage>> = Vec::with_capacity(tiles.len());
        for (slot, (name, tile)) in tiles.into_iter().enumerate() {
            if name == MISSING_TEXTURE {
                missing = slot as u32;
            }
            index.insert(name, slot as u32);
            chains.push(tile_mip_chain(&tile));
        }

        let mip_levels = TILE_SIZE.trailing_zeros() + 1;
        let mut pixels = Vec::new();
        let mut mips = Vec::new();
        for level in 0..mip_levels {
            let level_pixels = pack_level(level, &chains, layer_count);
            if level == 0 {
                pixels = level_pixels;
            } else {
                mips.push(level_pixels);
            }
        }

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
            layer_size: LAYER_SIZE,
            layer_count,
            tile_count,
            pixels,
            index,
            missing,
            mip_levels,
            mips,
            animations,
            animation_index,
            animation_width,
            animation_height,
            animation_pixels,
            unresolved: Vec::new(),
        }
    }
}

/// Build one tile's 16→1 chain with the alpha-weighted box filter M1 uses.
fn tile_mip_chain(tile: &RgbaImage) -> Vec<RgbaImage> {
    let mut chain = Vec::with_capacity(TILE_SIZE.trailing_zeros() as usize + 1);
    chain.push(tile.clone());
    let mut size = TILE_SIZE;
    while size > 1 {
        let next_size = size / 2;
        let previous = chain.last().expect("chain has level 0");
        let mut next = RgbaImage::new(next_size, next_size);
        for y in 0..next_size {
            for x in 0..next_size {
                let mut rgb = [0.0f32; 3];
                let mut alpha = 0.0;
                let mut count = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (x * 2 + dx).min(size - 1);
                        let sy = (y * 2 + dy).min(size - 1);
                        let pixel = previous.get_pixel(sx, sy).0;
                        let a = pixel[3] as f32 / 255.0;
                        rgb[0] += pixel[0] as f32 * a;
                        rgb[1] += pixel[1] as f32 * a;
                        rgb[2] += pixel[2] as f32 * a;
                        alpha += a;
                        count += 1.0;
                    }
                }
                let divisor = if alpha > 0.0 { alpha } else { count };
                next.put_pixel(
                    x,
                    y,
                    Rgba([
                        (rgb[0] / divisor).round().clamp(0.0, 255.0) as u8,
                        (rgb[1] / divisor).round().clamp(0.0, 255.0) as u8,
                        (rgb[2] / divisor).round().clamp(0.0, 255.0) as u8,
                        (alpha / count * 255.0).round().clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }
        chain.push(next);
        size = next_size;
    }
    chain
}

/// Pack every tile's `level` mip into the same 4×4 cell of a packed layer.
fn pack_level(level: u32, chains: &[Vec<RgbaImage>], layer_count: u32) -> Vec<u8> {
    let tile_size = (TILE_SIZE >> level).max(1);
    let layer_size = (LAYER_SIZE >> level).max(1);
    let mut pixels = vec![0u8; layer_count as usize * (layer_size * layer_size * 4) as usize];

    for (slot, chain) in chains.iter().enumerate() {
        let slot = slot as u32;
        let layer = slot / TILES_PER_LAYER;
        let local = slot % TILES_PER_LAYER;
        let tile_x = local % TILES_PER_ROW;
        let tile_y = local / TILES_PER_ROW;
        let tile = &chain[level as usize];
        for y in 0..tile_size {
            for x in 0..tile_size {
                let pixel = *tile.get_pixel(x.min(tile.width() - 1), y.min(tile.height() - 1));
                let px = tile_x * tile_size + x;
                let py = tile_y * tile_size + y;
                let offset = ((layer * layer_size + py) * layer_size + px) as usize * 4;
                pixels[offset..offset + 4].copy_from_slice(&pixel.0);
            }
        }
    }
    pixels
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

    #[test]
    fn mip_levels_above_the_clamp_would_blend_neighbouring_tiles() {
        // A bilinear fetch inside a packed tile reaches into the next tile when
        // the sample lands in the outer half-texel of the face. The affected
        // band is therefore 0.5 / texels_per_tile of the face on each side.
        let band = |level: u32| 0.5 / ((TILE_SIZE >> level).max(1) as f32);

        // At the clamp (level 2) a tile is 4x4 texels: only the outer 12.5% of
        // the face can pick up a neighbour.
        assert_eq!(BLOCK_LOD_MAX as u32, 2);
        assert!((band(2) - 0.125).abs() < 1e-6);

        // One level further the band doubles, and at level 4 a tile is a single
        // texel, so half of every face is the neighbouring tile.
        assert!((band(3) - 0.25).abs() < 1e-6);
        assert!((band(4) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_clamp_still_leaves_four_texels_per_tile() {
        assert_eq!(TILE_SIZE >> BLOCK_LOD_MAX as u32, 4);
    }

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
        // Tile 0 sits at (0, 0), tile 1 at (16, 0) inside the same layer.
        assert_eq!(&array.pixels()[0..4], &[10, 0, 0, 255]);
        assert_eq!(&array.pixels()[16 * 4..16 * 4 + 4], &[20, 0, 0, 255]);
        assert!(array.pixels()[..16 * 4]
            .chunks_exact(4)
            .all(|p| p == [10, 0, 0, 255]));
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
    fn mip_chain_halves_every_layer_and_tile() {
        let array = TextureArray::from_tiles(vec![(
            "a".to_string(),
            RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, Rgba([64, 128, 192, 255])),
        )]);
        assert_eq!(array.mip_levels(), 5);
        assert_eq!(array.mip_size(0), 64);
        assert_eq!(array.mip_size(1), 32);
        assert_eq!(array.mip_size(4), 4);
        assert_eq!(array.tile_mip_size(0), 16);
        assert_eq!(array.tile_mip_size(4), 1);
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
    fn mip_levels_do_not_mix_neighbouring_tiles() {
        let array = TextureArray::from_tiles(vec![
            named("red", Rgba([255, 0, 0, 255])),
            named("blue", Rgba([0, 0, 255, 255])),
        ]);
        // Level 4 holds one texel per tile: red at slot 0, blue at slot 1.
        let level = array.mip_pixels(4);
        assert_eq!(&level[0..4], &[255, 0, 0, 255]);
        assert_eq!(&level[4..8], &[0, 0, 255, 255]);
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
