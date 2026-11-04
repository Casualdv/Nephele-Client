//! One RGBA8 atlas holding the 2D GUI art: the vanilla bitmap font, the HUD
//! icons sheet, the widget sheet and the title logo.
//!
//! Everything the HUD, the menus and the title screen draw comes out of this
//! single texture, so the whole UI is a couple of draw calls.

use std::path::Path;

use image::{GenericImageView, Rgba, RgbaImage};

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

    /// Small original bitmap typeface for the launcher when the user has not
    /// prepared their own game resources yet. The launcher must be able to
    /// explain first-run setup without depending on Minecraft's font texture.
    pub fn launcher_fallback() -> Self {
        let mut image = RgbaImage::from_pixel(ATLAS_SIZE, ATLAS_SIZE, Rgba([0, 0, 0, 0]));
        for code in 0u32..=255 {
            let ch = char::from_u32(code).unwrap_or(' ');
            let rendered = if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            let rows = glyph_5x7(rendered);
            let col = code % 16;
            let row = code / 16;
            for (y, bits) in rows.iter().enumerate() {
                for x in 0..5 {
                    if bits & (1 << (4 - x)) != 0 {
                        image.put_pixel(
                            col * 8 + 1 + x,
                            row * 8 + y as u32,
                            Rgba([255, 255, 255, 255]),
                        );
                    }
                }
            }
        }
        let font_sheet = image.view(0, 0, 128, 128).to_image();
        font::set_widths(font::measure(&font_sheet));
        image.put_pixel(WHITE.x, WHITE.y, Rgba([255, 255, 255, 255]));
        Self {
            pixels: image.into_raw(),
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

/// A compact 5x7 uppercase bitmap font. Lowercase ASCII is mapped to the same
/// legible glyphs so every launcher page can be rendered before game assets
/// exist, without embedding or downloading a third-party font.
fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 4, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        ';' => [0, 12, 12, 0, 4, 4, 8],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '=' => [0, 31, 0, 31, 0, 0, 0],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '"' => [10, 10, 0, 0, 0, 0, 0],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '%' => [17, 2, 4, 8, 16, 17, 0],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '*' => [0, 21, 14, 31, 14, 21, 0],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        ' ' => [0; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
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

    #[test]
    fn launcher_fallback_contains_its_original_bitmap_font_and_fill_texel() {
        let atlas = UiAtlas::launcher_fallback();
        let pixels = atlas.pixels();
        let at = |x: usize, y: usize| {
            let index = (y * ATLAS_SIZE as usize + x) * 4;
            &pixels[index..index + 4]
        };
        assert_eq!(at(10, 32)[3], 255, "the A glyph has visible ink");
        assert_eq!(
            at(WHITE.x as usize, WHITE.y as usize),
            &[255, 255, 255, 255]
        );
        assert_eq!(pixels.len(), (ATLAS_SIZE * ATLAS_SIZE * 4) as usize);
    }
}
