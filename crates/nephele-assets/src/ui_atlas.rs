//! One RGBA8 atlas holding the 2D GUI art: the vanilla bitmap font, the HUD
//! icons sheet, the widget sheet and the title logo.
//!
//! Everything the HUD, the menus and the title screen draw comes out of this
//! single texture, so the whole UI is a couple of draw calls.

use std::path::Path;

use image::{Rgba, RgbaImage};

use crate::atlas::AssetError;

/// The atlas is a fixed 1024x1024 so UV maths stays a simple divide.
pub const ATLAS_SIZE: u32 = 1024;

/// A pixel rectangle inside [`UiAtlas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl SpriteRect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// A rectangle relative to this one.
    pub const fn sub(&self, x: u32, y: u32, w: u32, h: u32) -> SpriteRect {
        SpriteRect {
            x: self.x + x,
            y: self.y + y,
            w,
            h,
        }
    }

    pub const fn uv(self) -> [f32; 4] {
        let size = ATLAS_SIZE as f32;
        [
            self.x as f32 / size,
            self.y as f32 / size,
            (self.x + self.w) as f32 / size,
            (self.y + self.h) as f32 / size,
        ]
    }
}

/// Where each source sheet lives in the atlas.
pub mod sheets {
    use super::SpriteRect;

    /// `textures/font/ascii.png` - a 16x16 grid of 8x8 glyphs.
    pub const FONT: SpriteRect = SpriteRect::new(0, 0, 128, 128);
    /// `textures/gui/icons.png` - hearts, food, the crosshair, the XP bar.
    pub const ICONS: SpriteRect = SpriteRect::new(128, 0, 256, 256);
    /// `textures/gui/widgets.png` - hotbar, selection frame and buttons.
    pub const WIDGETS: SpriteRect = SpriteRect::new(128, 256, 256, 256);
    /// `textures/gui/title/minecraft.png` - the title wordmark.
    pub const LOGO: SpriteRect = SpriteRect::new(512, 0, 256, 256);
    /// `textures/gui/options_background.png` - the tiled dirt backdrop.
    pub const OPTIONS_BACKGROUND: SpriteRect = SpriteRect::new(512, 256, 16, 16);
    /// `textures/gui/container/inventory.png` - the player inventory panel.
    pub const INVENTORY: SpriteRect = SpriteRect::new(0, 512, 256, 256);
    /// `textures/gui/container/crafting_table.png` - the 3x3 table panel.
    pub const CRAFTING_TABLE: SpriteRect = SpriteRect::new(256, 512, 256, 256);
    /// `textures/gui/container/furnace.png` - the furnace panel plus its
    /// flame and progress-arrow sprites.
    pub const FURNACE: SpriteRect = SpriteRect::new(512, 512, 256, 256);
    /// `textures/gui/container/generic_54.png` - the chest panel.
    pub const CHEST: SpriteRect = SpriteRect::new(768, 512, 256, 256);
}

/// Individual sprites, in the coordinates vanilla's GUI classes use.
pub mod sprites {
    use super::{sheets, SpriteRect};

    pub const CROSSHAIR: SpriteRect = sheets::ICONS.sub(0, 0, 16, 16);
    pub const HEART_BACKGROUND: SpriteRect = sheets::ICONS.sub(16, 0, 9, 9);
    pub const HEART_FULL: SpriteRect = sheets::ICONS.sub(54, 0, 9, 9);
    pub const HEART_HALF: SpriteRect = sheets::ICONS.sub(63, 0, 9, 9);
    pub const FOOD_BACKGROUND: SpriteRect = sheets::ICONS.sub(16, 27, 9, 9);
    pub const FOOD_FULL: SpriteRect = sheets::ICONS.sub(52, 27, 9, 9);
    pub const FOOD_HALF: SpriteRect = sheets::ICONS.sub(61, 27, 9, 9);
    pub const XP_BAR_BACKGROUND: SpriteRect = sheets::ICONS.sub(0, 64, 182, 5);
    pub const XP_BAR_FILL: SpriteRect = sheets::ICONS.sub(0, 69, 182, 5);

    pub const HOTBAR: SpriteRect = sheets::WIDGETS.sub(0, 0, 182, 22);
    pub const HOTBAR_SELECTION: SpriteRect = sheets::WIDGETS.sub(0, 22, 24, 22);

    /// 200x20 button face; vanilla draws three vertical copies of one strip.
    pub const BUTTON: SpriteRect = sheets::WIDGETS.sub(0, 46, 200, 20);
    pub const BUTTON_HOVER: SpriteRect = sheets::WIDGETS.sub(0, 66, 200, 20);
    pub const BUTTON_DISABLED: SpriteRect = sheets::WIDGETS.sub(0, 26, 200, 20);

    /// Only the inked part of the 256x256 logo image (the rest is transparent).
    pub const LOGO_INK: SpriteRect = sheets::LOGO.sub(0, 0, 199, 89);

    /// The drawn part of each container panel (the rest of the sheet is unused).
    pub const INVENTORY_PANEL: SpriteRect = sheets::INVENTORY.sub(0, 0, 176, 166);
    pub const CRAFTING_TABLE_PANEL: SpriteRect = sheets::CRAFTING_TABLE.sub(0, 0, 176, 166);
    pub const FURNACE_PANEL: SpriteRect = sheets::FURNACE.sub(0, 0, 176, 166);
    /// A 27-slot chest: the three chest rows, then the player section, both cut
    /// from `generic_54.png` so the panel is 176x168 like vanilla's.
    pub const CHEST_TOP: SpriteRect = sheets::CHEST.sub(0, 0, 176, 72);
    pub const CHEST_BOTTOM: SpriteRect = sheets::CHEST.sub(0, 126, 176, 96);
    /// Furnace progress art, taken from the right-hand column of its sheet.
    pub const FURNACE_FLAME: SpriteRect = sheets::FURNACE.sub(176, 0, 14, 14);
    pub const FURNACE_ARROW: SpriteRect = sheets::FURNACE.sub(176, 14, 24, 16);
    /// Empty/full versions of the arrow and flame, so progress can be clipped.
    pub const FURNACE_ARROW_BACKGROUND: SpriteRect = sheets::FURNACE.sub(176, 30, 24, 16);
    pub const FURNACE_FLAME_BACKGROUND: SpriteRect = sheets::FURNACE.sub(176, 46, 14, 14);
}

/// Bitmap font metrics for `ascii.png`.
pub mod font {
    use super::{sheets, SpriteRect};
    use image::RgbaImage;

    /// Glyph cell size in the sheet.
    pub const CELL: u32 = 8;
    /// Columns in the 16x16 grid.
    pub const COLUMNS: u32 = 16;
    /// Fallback advance when the sheet hasn't been measured.
    pub const ADVANCE: f32 = 6.0;
    /// Glyph height, in GUI pixels.
    pub const HEIGHT: f32 = 8.0;

    static WIDTHS: std::sync::OnceLock<[u8; 256]> = std::sync::OnceLock::new();

    /// Advance for `ch`, in GUI pixels.
    ///
    /// Vanilla measures this from the glyph bitmap (`FontRenderer
    /// .readFontTexture`): the rightmost inked column plus a one-pixel gap.
    pub fn advance(ch: char) -> f32 {
        let code = ch as usize;
        match WIDTHS.get() {
            Some(table) if code < table.len() => table[code] as f32,
            _ => ADVANCE,
        }
    }

    /// Measure every glyph in an 8x8-cell font sheet.
    pub fn measure(sheet: &RgbaImage) -> [u8; 256] {
        let mut widths = [ADVANCE as u8; 256];
        for (code, width) in widths.iter_mut().enumerate() {
            let col = (code % COLUMNS as usize) as u32;
            let row = (code / COLUMNS as usize) as u32;
            let mut rightmost: Option<u32> = None;
            for y in 0..CELL {
                for x in 0..CELL {
                    let px = col * CELL + x;
                    let py = row * CELL + y;
                    if px >= sheet.width() || py >= sheet.height() {
                        continue;
                    }
                    if sheet.get_pixel(px, py)[3] > 0 {
                        rightmost = Some(rightmost.map_or(x, |current| current.max(x)));
                    }
                }
            }
            *width = match (code, rightmost) {
                (0x20, _) => 4,
                (_, None) => 1,
                (_, Some(last)) => (last + 2).min(CELL) as u8,
            };
        }
        widths
    }

    pub(crate) fn set_widths(table: [u8; 256]) {
        let _ = WIDTHS.set(table);
    }

    /// Atlas rectangle for `ch`, or `None` if the sheet has no glyph for it.
    ///
    /// `ascii.png` is indexed by the **raw character code** (vanilla's
    /// `FontRenderer.renderDefaultChar` uses `ch % 16 * 8`, `ch / 16 * 8`), so
    /// `'A'` (65) sits at cell (1, 4), not at the start of the letters.
    pub fn glyph(ch: char) -> Option<SpriteRect> {
        let code = ch as u32;
        if code > 0xFF {
            return None;
        }
        let col = code % COLUMNS;
        let row = code / COLUMNS;
        Some(sheets::FONT.sub(col * CELL, row * CELL, CELL, CELL))
    }
}

#[derive(Debug, Clone)]
pub struct UiAtlas {
    pixels: Vec<u8>,
}

impl UiAtlas {
    pub const SIZE: u32 = ATLAS_SIZE;

    /// Compose the atlas from the vanilla `textures` directory.
    pub fn load(textures_dir: &Path) -> Result<Self, AssetError> {
        let mut image = RgbaImage::from_pixel(ATLAS_SIZE, ATLAS_SIZE, Rgba([0, 0, 0, 0]));
        let mut loaded = 0;

        for (sheet, path) in [
            (sheets::FONT, "font/ascii.png"),
            (sheets::ICONS, "gui/icons.png"),
            (sheets::WIDGETS, "gui/widgets.png"),
            (sheets::LOGO, "gui/title/minecraft.png"),
            (sheets::INVENTORY, "gui/container/inventory.png"),
            (sheets::CRAFTING_TABLE, "gui/container/crafting_table.png"),
            (sheets::FURNACE, "gui/container/furnace.png"),
            (sheets::CHEST, "gui/container/generic_54.png"),
        ] {
            let full = textures_dir.join(path);
            if !full.is_file() {
                continue;
            }
            let decoded = image::open(&full).map_err(|source| AssetError::Image {
                path: full.display().to_string(),
                source,
            })?;
            let decoded = decoded.to_rgba8();
            if sheet == sheets::FONT {
                font::set_widths(font::measure(&decoded));
            }
            blit(&mut image, &decoded, sheet.x, sheet.y);
            loaded += 1;
        }

        if loaded == 0 {
            return Err(AssetError::NoTextures(textures_dir.display().to_string()));
        }

        // A single opaque white texel, used for solid-colour quads.
        image.put_pixel(WHITE.x, WHITE.y, Rgba([255, 255, 255, 255]));

        Ok(Self {
            pixels: image.into_raw(),
        })
    }

    /// A blank atlas, for modes that never draw UI.
    pub fn blank() -> Self {
        Self {
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
        }
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// UV rectangle for a solid white fill.
    pub fn white_uv() -> [f32; 4] {
        let size = ATLAS_SIZE as f32;
        let inset = 0.5;
        [
            (WHITE.x as f32 + inset) / size,
            (WHITE.y as f32 + inset) / size,
            (WHITE.x as f32 + inset) / size,
            (WHITE.y as f32 + inset) / size,
        ]
    }
}

/// The 1x1 white texel, tucked into unused atlas space.
const WHITE: SpriteRect = SpriteRect::new(1020, 1020, 1, 1);

fn blit(dest: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32) {
    for sy in 0..src.height() {
        for sx in 0..src.width() {
            let px = x + sx;
            let py = y + sy;
            if px >= dest.width() || py >= dest.height() {
                continue;
            }
            dest.put_pixel(px, py, *src.get_pixel(sx, sy));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_tile_into_the_font_sheet() {
        // Space is code 32 -> row 2, column 0.
        assert_eq!(font::glyph(' ').unwrap(), SpriteRect::new(0, 16, 8, 8));
        assert_eq!(font::glyph('!').unwrap(), SpriteRect::new(8, 16, 8, 8));
        // 'A' is 65 -> row 4, column 1.
        assert_eq!(font::glyph('A').unwrap(), SpriteRect::new(8, 32, 8, 8));
        assert_eq!(font::glyph('0').unwrap(), SpriteRect::new(0, 24, 8, 8));
        assert!(font::glyph('\u{4e00}').is_none());
    }

    #[test]
    fn glyph_widths_come_from_the_bitmap() {
        let mut sheet = RgbaImage::from_pixel(128, 128, Rgba([0, 0, 0, 0]));
        for y in 0..8 {
            for x in 0..5 {
                sheet.put_pixel(8 + x, 32 + y, Rgba([255, 255, 255, 255]));
            }
        }
        let widths = font::measure(&sheet);
        assert_eq!(widths['A' as usize], 6, "five ink columns plus a gap");
        assert_eq!(widths[' ' as usize], 4);
        assert_eq!(widths['B' as usize], 1, "an empty cell is one pixel");
    }

    #[test]
    fn uv_is_normalised() {
        let rect = SpriteRect::new(0, 0, 1024, 1024);
        assert_eq!(rect.uv(), [0.0, 0.0, 1.0, 1.0]);
        let quarter = SpriteRect::new(0, 0, 512, 256);
        assert_eq!(quarter.uv(), [0.0, 0.0, 0.5, 0.25]);
    }

    #[test]
    fn sub_rects_are_relative() {
        let base = SpriteRect::new(128, 0, 256, 256);
        assert_eq!(base.sub(54, 0, 9, 9), SpriteRect::new(182, 0, 9, 9));
    }
}
