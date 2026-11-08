//! Procedural particle sprites.
//!
//! 1.8's particle sheet is outside the block texture array and is not prepared
//! by the resource step. These small generated tiles give smoke, flame, crit,
//! portal, splash and heart particles a distinct look without shipping any
//! Mojang artwork.

use image::{Rgba, RgbaImage};

const TILE: u32 = 16;

/// Stable layer names used by the renderer.
pub const SMOKE: &str = "particle:smoke";
pub const FLAME: &str = "particle:flame";
pub const CRIT: &str = "particle:crit";
pub const PORTAL: &str = "particle:portal";
pub const SPLASH: &str = "particle:splash";
pub const HEART: &str = "particle:heart";

pub fn layers() -> Vec<(&'static str, RgbaImage)> {
    vec![
        (SMOKE, smoke()),
        (FLAME, flame()),
        (CRIT, crit()),
        (PORTAL, portal()),
        (SPLASH, splash()),
        (HEART, heart()),
    ]
}

fn blank() -> RgbaImage {
    RgbaImage::from_pixel(TILE, TILE, Rgba([0, 0, 0, 0]))
}

fn put(image: &mut RgbaImage, x: i32, y: i32, color: [u8; 4]) {
    if (0..TILE as i32).contains(&x) && (0..TILE as i32).contains(&y) {
        image.put_pixel(x as u32, y as u32, Rgba(color));
    }
}

fn radial(center: (f32, f32), radius: f32, inner: [u8; 4], outer: [u8; 4]) -> RgbaImage {
    let mut image = blank();
    for y in 0..TILE {
        for x in 0..TILE {
            let dx = x as f32 + 0.5 - center.0;
            let dy = y as f32 + 0.5 - center.1;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance > radius {
                continue;
            }
            let t = (distance / radius).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t) as u8;
            let alpha = mix(inner[3], outer[3]);
            image.put_pixel(
                x,
                y,
                Rgba([
                    mix(inner[0], outer[0]),
                    mix(inner[1], outer[1]),
                    mix(inner[2], outer[2]),
                    alpha,
                ]),
            );
        }
    }
    image
}

fn smoke() -> RgbaImage {
    let mut image = radial((8.0, 8.0), 6.5, [190, 190, 190, 210], [70, 70, 70, 0]);
    // A few darker puffs keep it from looking like a flat dot.
    for (x, y, a) in [(5, 6, 100), (10, 5, 90), (8, 10, 80)] {
        put(&mut image, x, y, [120, 120, 120, a]);
    }
    image
}

fn flame() -> RgbaImage {
    let mut image = radial((8.0, 9.5), 6.0, [255, 245, 160, 255], [255, 70, 0, 0]);
    for y in 2..11 {
        let width = ((y - 2) as f32 / 9.0 * 3.0) as i32;
        for x in 8 - width..=8 + width {
            put(&mut image, x, y, [255, 190, 40, 230]);
        }
    }
    image
}

fn crit() -> RgbaImage {
    let mut image = blank();
    for i in 2..14 {
        put(&mut image, i, 8, [255, 245, 180, 255]);
        put(&mut image, 8, i, [255, 245, 180, 255]);
        put(&mut image, i, i, [255, 220, 100, 220]);
        put(&mut image, 15 - i, i, [255, 220, 100, 220]);
    }
    image
}

fn portal() -> RgbaImage {
    let mut image = blank();
    for y in 0..TILE {
        for x in 0..TILE {
            let dx = x as f32 + 0.5 - 8.0;
            let dy = y as f32 + 0.5 - 8.0;
            let distance = (dx * dx + dy * dy).sqrt();
            if (3.5..6.5).contains(&distance) {
                let swirl = ((dx.atan2(dy) * 3.0).sin() * 0.5 + 0.5) * 160.0 + 60.0;
                image.put_pixel(x, y, Rgba([150, 60, 210, swirl as u8]));
            }
        }
    }
    image
}

fn splash() -> RgbaImage {
    let mut image = radial((8.0, 10.0), 5.0, [180, 225, 255, 230], [40, 100, 220, 0]);
    for (x, y) in [(4, 4), (8, 2), (12, 5)] {
        put(&mut image, x, y, [210, 240, 255, 220]);
    }
    image
}

fn heart() -> RgbaImage {
    let mut image = blank();
    for y in 0..TILE {
        for x in 0..TILE {
            let dx = (x as f32 + 0.5 - 8.0) / 5.5;
            let dy = (y as f32 + 0.5 - 8.0) / 5.5;
            let value = (dx * dx + dy * dy - 0.45).powi(2) - 0.35 * dx * dx * dy;
            if value <= 0.0 {
                image.put_pixel(x, y, Rgba([235, 60, 90, 230]));
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sprite_is_a_non_empty_16px_tile() {
        for (name, image) in layers() {
            assert_eq!(image.dimensions(), (TILE, TILE), "{name}");
            assert!(
                image.pixels().any(|pixel| pixel.0[3] > 0),
                "{name} is fully transparent"
            );
        }
    }
}
