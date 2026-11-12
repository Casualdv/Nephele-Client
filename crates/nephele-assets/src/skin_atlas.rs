//! Skin textures for remote players.
//!
//! A Minecraft skin is a 64×64 RGBA image unwrapped onto a humanoid model. The
//! renderer needs them in a single bindable texture, so they are packed into one
//! 2048×2048 atlas of 8×8 slots, each slot holding a skin upscaled 4× (256×256)
//! so nearest-neighbour sampling stays pixel-crisp at close range. Maps reuse a
//! slot at 2× (a 128×128 map fills 256×256), so the entity pipeline keeps one
//! bind group for skins, mob sheets, block entities and framed maps.

use image::RgbaImage;

/// Vanilla skin dimensions.
pub const SKIN_SIZE: u32 = 64;
/// Pixels per skin in the atlas after the 4× upscale.
pub const SLOT_SIZE: u32 = 256;
/// Slots per atlas row and column.
///
/// 8×8 rather than 4×4: a busy lobby can hold a dozen player skins, several
/// mob textures, the block-entity sheets and a few maps at once.
pub const SLOTS_PER_ROW: u32 = 8;
/// Total slots in one atlas.
pub const SLOT_COUNT: u32 = SLOTS_PER_ROW * SLOTS_PER_ROW;
/// Full atlas edge in pixels.
pub const ATLAS_SIZE: u32 = SLOT_SIZE * SLOTS_PER_ROW;
/// Slot 0 is the built-in stand-in skin, slot 1 is a solid white texel used by
/// tinted armour boxes. Dynamic skins/maps start at slot 2.
pub const SOLID_SLOT: u32 = 1;

/// A packed collection of player skins.
#[derive(Debug, Clone)]
pub struct SkinAtlas {
    pixels: Vec<u8>,
    /// Texture hash occupying each slot; `None` means free.
    slots: [Option<String>; SLOT_COUNT as usize],
    /// Slots changed since the last GPU upload. The renderer drains this with a
    /// per-frame budget, so one busy map cannot stall the frame.
    dirty: [bool; SLOT_COUNT as usize],
    /// Round-robin cursor for evicting slots when the atlas is full.
    next_slot: u32,
    version: u64,
}

impl Default for SkinAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinAtlas {
    /// An atlas whose slot 0 holds a generated stand-in skin, so an entity with
    /// no (or not yet fetched) texture still renders as a recognisable humanoid.
    pub fn new() -> Self {
        let mut atlas = Self {
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            slots: [const { None }; SLOT_COUNT as usize],
            dirty: [true; SLOT_COUNT as usize],
            next_slot: 2,
            version: 1,
        };
        atlas.set_skin_pixels(0, &default_skin());
        atlas.slots[0] = Some(DEFAULT_SKIN_KEY.to_string());
        let solid = RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        atlas.set_skin_pixels(SOLID_SLOT, &solid);
        atlas.slots[SOLID_SLOT as usize] = Some(SOLID_KEY.to_string());
        atlas
    }

    /// Slot index for the built-in stand-in skin.
    pub const DEFAULT_SLOT: u32 = 0;

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn slot_of(&self, hash: &str) -> Option<u32> {
        self.slots
            .iter()
            .position(|slot| slot.as_deref() == Some(hash))
            .map(|slot| slot as u32)
    }

    /// Reserve a slot for `hash`, evicting the oldest assigned one when full.
    pub fn assign(&mut self, hash: &str) -> u32 {
        if let Some(slot) = self.slot_of(hash) {
            return slot;
        }
        // Prefer a free slot, keeping slots 0/1 (stand-in + solid) reserved.
        if let Some(free) = self
            .slots
            .iter()
            .enumerate()
            .position(|(index, slot)| index >= 2 && slot.is_none())
        {
            self.slots[free] = Some(hash.to_string());
            return free as u32;
        }
        let mut slot = self.next_slot.max(2) % SLOT_COUNT;
        if slot < 2 {
            slot = 2;
        }
        self.next_slot = slot + 1;
        self.slots[slot as usize] = Some(hash.to_string());
        slot
    }

    /// Copy a decoded 64×64 skin into `slot`, upscaling 4×.
    pub fn set_skin(&mut self, slot: u32, image: &RgbaImage) {
        self.set_skin_pixels(slot, image);
    }

    fn set_skin_pixels(&mut self, slot: u32, image: &RgbaImage) {
        let (origin_x, origin_y) = slot_origin(slot);
        for sy in 0..SKIN_SIZE {
            for sx in 0..SKIN_SIZE {
                let pixel = *image.get_pixel(
                    sx.min(image.width().saturating_sub(1)),
                    sy.min(image.height().saturating_sub(1)),
                );
                for dy in 0..4 {
                    for dx in 0..4 {
                        let x = origin_x + sx * 4 + dx;
                        let y = origin_y + sy * 4 + dy;
                        let index = ((y * ATLAS_SIZE + x) * 4) as usize;
                        self.pixels[index..index + 4].copy_from_slice(&pixel.0);
                    }
                }
            }
        }
        self.dirty[slot as usize % SLOT_COUNT as usize] = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Copy a map image (normally 128×128) into `slot`, upscaled to fill the
    /// 256×256 slot. Maps and skins share the atlas so framed maps and the map
    /// item can be drawn by the entity pipeline without a second bind group.
    pub fn set_map(&mut self, slot: u32, image: &RgbaImage) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        let (origin_x, origin_y) = slot_origin(slot);
        for sy in 0..SLOT_SIZE {
            let src_y = (sy * image.height() / SLOT_SIZE).min(image.height() - 1);
            for sx in 0..SLOT_SIZE {
                let src_x = (sx * image.width() / SLOT_SIZE).min(image.width() - 1);
                let pixel = *image.get_pixel(src_x, src_y);
                let index = (((origin_y + sy) * ATLAS_SIZE + origin_x + sx) * 4) as usize;
                self.pixels[index..index + 4].copy_from_slice(&pixel.0);
            }
        }
        self.dirty[slot as usize % SLOT_COUNT as usize] = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Contiguous pixels for one slot, for a sub-rectangle texture upload.
    /// The atlas rows are wider than a slot, so this copies the slot's 256
    /// rows into a packed buffer; `bytes_per_row` is then `4 * SLOT_SIZE`.
    pub fn slot_pixels(&self, slot: u32) -> Vec<u8> {
        let (origin_x, origin_y) = slot_origin(slot);
        let row_bytes = (SLOT_SIZE * 4) as usize;
        let mut out = Vec::with_capacity(row_bytes * SLOT_SIZE as usize);
        for y in 0..SLOT_SIZE {
            let start = (((origin_y + y) * ATLAS_SIZE + origin_x) * 4) as usize;
            out.extend_from_slice(&self.pixels[start..start + row_bytes]);
        }
        out
    }

    /// Take up to `limit` slots that changed since the last upload.
    pub fn take_dirty(&mut self, limit: usize) -> Vec<u32> {
        let mut slots = Vec::with_capacity(limit.min(SLOT_COUNT as usize));
        for (index, dirty) in self.dirty.iter_mut().enumerate() {
            if *dirty {
                *dirty = false;
                slots.push(index as u32);
                if slots.len() == limit {
                    break;
                }
            }
        }
        slots
    }

    /// Slot holding a solid white texel, for tinted armour and placeholder
    /// framed items.
    pub const fn solid_slot() -> u32 {
        SOLID_SLOT
    }

    /// UV of the middle of the solid slot.
    ///
    /// Sample the *centre*, not `uv(SOLID_SLOT, 0.0, 0.0)`: the slot's top-left
    /// corner sits exactly on a texel boundary, so linear filtering picks the
    /// neighbouring slot per fragment and a tinted box dithers against whatever
    /// is behind it.
    pub fn solid_uv() -> [f32; 2] {
        Self::uv(SOLID_SLOT, SKIN_SIZE as f32 * 0.5, SKIN_SIZE as f32 * 0.5)
    }

    /// Atlas pixel rectangle `(x, y, size)` for a slot.
    pub fn slot_rect(slot: u32) -> (u32, u32, u32) {
        let (x, y) = slot_origin(slot);
        (x, y, SLOT_SIZE)
    }

    /// Normalised UV for a skin pixel inside `slot`.
    ///
    /// `u`/`v` are in skin pixels (0..64) and are treated as texture edges, so a
    /// rect spanning pixels `x..x2` maps exactly onto those texels.
    pub fn uv(slot: u32, u: f32, v: f32) -> [f32; 2] {
        let (origin_x, origin_y) = slot_origin(slot);
        [
            (origin_x as f32 + u * 4.0) / ATLAS_SIZE as f32,
            (origin_y as f32 + v * 4.0) / ATLAS_SIZE as f32,
        ]
    }
}

fn slot_origin(slot: u32) -> (u32, u32) {
    let slot = slot % SLOT_COUNT;
    let column = slot % SLOTS_PER_ROW;
    let row = slot / SLOTS_PER_ROW;
    (column * SLOT_SIZE, row * SLOT_SIZE)
}

/// Key used for the stand-in skin in the slot table.
pub const DEFAULT_SKIN_KEY: &str = "<default>";
/// Key used for the solid white slot.
pub const SOLID_KEY: &str = "<solid>";

/// A flat, Steve-shaped stand-in skin so unknown players don't render as a
/// featureless white blob.
pub fn default_skin() -> RgbaImage {
    // Start fully painted: a face whose UV happens to touch a region boundary
    // must never sample an alpha-0 texel, or the model silently loses a face.
    let mut image = RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([120, 120, 120, 255]));

    const SKIN: [u8; 4] = [198, 145, 106, 255];
    const HAIR: [u8; 4] = [58, 40, 24, 255];
    const SHIRT: [u8; 4] = [0, 174, 174, 255];
    const PANTS: [u8; 4] = [62, 62, 152, 255];
    const SHOE: [u8; 4] = [60, 60, 60, 255];

    let fill = |image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, colour: [u8; 4]| {
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, image::Rgba(colour));
            }
        }
    };

    // The hat/jacket/sleeve/pants unwraps are a second layer. A skin that does
    // not paint them must leave them transparent, or every unknown player is
    // drawn inside an opaque grey shell. They do not overlap the base unwrap.
    for (x0, y0, x1, y1) in [
        (32, 0, 64, 16),
        (16, 32, 40, 48),
        (40, 32, 56, 48),
        (48, 48, 64, 64),
        (0, 32, 16, 48),
        (0, 48, 16, 64),
    ] {
        fill(&mut image, x0, y0, x1, y1, [0, 0, 0, 0]);
    }

    // Head unwrap: (0,0)-(32,16) for the base layer.
    fill(&mut image, 0, 0, 32, 8, HAIR);
    fill(&mut image, 0, 8, 32, 16, SKIN);
    // Face: keep the front face skin-toned with dark eyes.
    fill(&mut image, 8, 8, 16, 16, SKIN);
    fill(&mut image, 9, 11, 11, 13, HAIR);
    fill(&mut image, 13, 11, 16, 13, HAIR);

    // Body: (16,16)-(40,32).
    fill(&mut image, 16, 16, 40, 32, SHIRT);
    // Right arm (40,16) and right leg (0,16).
    fill(&mut image, 40, 16, 56, 24, SHIRT);
    fill(&mut image, 40, 24, 56, 32, SKIN);
    fill(&mut image, 0, 16, 16, 24, PANTS);
    fill(&mut image, 0, 24, 16, 32, SHOE);

    // Left arm (32,48) and left leg (16,48): 64×64 skins only.
    fill(&mut image, 32, 48, 48, 56, SHIRT);
    fill(&mut image, 32, 56, 48, 64, SKIN);
    fill(&mut image, 16, 48, 32, 56, PANTS);
    fill(&mut image, 16, 56, 32, 64, SHOE);

    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_is_the_expected_size() {
        let atlas = SkinAtlas::new();
        assert_eq!(atlas.pixels().len() as u32, ATLAS_SIZE * ATLAS_SIZE * 4);
    }

    #[test]
    fn slot_zero_holds_the_stand_in_skin() {
        let atlas = SkinAtlas::new();
        assert_eq!(atlas.slot_of(DEFAULT_SKIN_KEY), Some(0));
        // The default skin is not transparent everywhere.
        assert!(atlas.pixels().chunks_exact(4).any(|p| p[3] == 255));
    }

    #[test]
    fn assigning_is_stable_and_unique() {
        let mut atlas = SkinAtlas::new();
        let a = atlas.assign("aaa");
        let b = atlas.assign("bbb");
        assert_ne!(a, b);
        assert_eq!(atlas.assign("aaa"), a);
        // Slots 0 and 1 are reserved for the stand-in and solid white.
        assert!(a >= 2);
        assert!(b >= 2);
    }

    #[test]
    fn dirty_slots_are_drained_with_a_budget() {
        let mut atlas = SkinAtlas::new();
        // Construction marks everything dirty; drain it first.
        let _ = atlas.take_dirty(usize::MAX);
        let a = atlas.assign("aaa");
        let b = atlas.assign("bbb");
        atlas.set_skin(
            a,
            &RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([1, 2, 3, 255])),
        );
        atlas.set_skin(
            b,
            &RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([4, 5, 6, 255])),
        );
        let first = atlas.take_dirty(1);
        assert_eq!(first.len(), 1);
        let second = atlas.take_dirty(4);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0], second[0]);
        assert!(atlas.take_dirty(4).is_empty());
    }

    #[test]
    fn a_map_fills_its_slot_and_is_uploaded() {
        let mut atlas = SkinAtlas::new();
        let _ = atlas.take_dirty(usize::MAX);
        let slot = atlas.assign("map:7");
        let map = RgbaImage::from_pixel(128, 128, image::Rgba([10, 20, 30, 255]));
        atlas.set_map(slot, &map);
        let (x, y, _) = SkinAtlas::slot_rect(slot);
        let index = ((y * ATLAS_SIZE + x) * 4) as usize;
        assert_eq!(&atlas.pixels()[index..index + 4], &[10, 20, 30, 255]);
        assert_eq!(atlas.take_dirty(4), vec![slot]);
    }

    #[test]
    fn writes_land_in_the_right_slot() {
        let mut atlas = SkinAtlas::new();
        let slot = atlas.assign("ccc");
        let mut skin = RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([1, 2, 3, 255]));
        skin.put_pixel(0, 0, image::Rgba([9, 8, 7, 255]));
        atlas.set_skin(slot, &skin);

        let (x, y, _) = SkinAtlas::slot_rect(slot);
        let index = ((y * ATLAS_SIZE + x) * 4) as usize;
        // Pixel (0,0) of the skin fills a 4×4 block in the atlas.
        assert_eq!(&atlas.pixels()[index..index + 4], &[9, 8, 7, 255]);
        // Somewhere in the middle it is the fill colour.
        let mid = (((y + 100) * ATLAS_SIZE + x + 100) * 4) as usize;
        assert_eq!(&atlas.pixels()[mid..mid + 4], &[1, 2, 3, 255]);
    }

    #[test]
    fn uv_covers_one_slot() {
        let [u0, v0] = SkinAtlas::uv(1, 0.0, 0.0);
        let [u1, v1] = SkinAtlas::uv(1, SKIN_SIZE as f32, SKIN_SIZE as f32);
        assert!((u1 - u0 - SLOT_SIZE as f32 / ATLAS_SIZE as f32).abs() < 1e-6);
        assert!((v1 - v0 - SLOT_SIZE as f32 / ATLAS_SIZE as f32).abs() < 1e-6);
        // Slot 1 is the second column of the first row.
        assert!((u0 - 256.0 / ATLAS_SIZE as f32).abs() < 1e-6);
        assert!(v0.abs() < 1e-6);
    }

    #[test]
    fn version_changes_when_pixels_do() {
        let mut atlas = SkinAtlas::new();
        let before = atlas.version();
        let slot = atlas.assign("ddd");
        atlas.set_skin(
            slot,
            &RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([5, 5, 5, 255])),
        );
        assert!(atlas.version() > before);
    }

    #[test]
    fn default_skin_has_distinct_regions() {
        let skin = default_skin();
        assert_eq!(skin.dimensions(), (SKIN_SIZE, SKIN_SIZE));
        // Face centre is skin-toned, hair above it differs, shirt below differs.
        let face = skin.get_pixel(10, 14).0;
        let hair = skin.get_pixel(10, 4).0;
        let shirt = skin.get_pixel(20, 20).0;
        assert_ne!(face, hair);
        assert_ne!(face, shirt);
        assert_eq!(hair[3], 255);
    }
}
