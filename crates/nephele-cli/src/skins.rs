//! Downloads player skins from Mojang's texture CDN.
//!
//! Fetching happens on a worker thread so a slow or unreachable CDN can never
//! stall the renderer; results are polled once a frame.

use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

use image::RgbaImage;

/// Timeout for one skin download.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Vanilla skins are always 64×64 (the 64×32 legacy shape is padded).
const SKIN_SIZE: u32 = 64;

pub struct SkinLoader {
    requests: Sender<String>,
    results: Receiver<(String, RgbaImage)>,
    /// Hashes already requested, so a repeat join doesn't refetch.
    requested: HashSet<String>,
    /// Hashes that failed; they are not retried until the next session.
    failed: HashSet<String>,
}

impl Default for SkinLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinLoader {
    pub fn new() -> Self {
        let (request_tx, request_rx) = channel::<String>();
        let (result_tx, result_rx) = channel::<(String, RgbaImage)>();
        let request_rx = std::sync::Arc::new(std::sync::Mutex::new(request_rx));

        // Two workers: a join can announce a dozen players at once.
        for index in 0..2 {
            let request_rx = std::sync::Arc::clone(&request_rx);
            let result_tx = result_tx.clone();
            thread::Builder::new()
                .name(format!("skin-{index}"))
                .spawn(move || loop {
                    let hash = {
                        let rx = request_rx.lock().unwrap_or_else(|err| err.into_inner());
                        rx.recv()
                    };
                    let Ok(hash) = hash else { break };
                    // Offline (or a hash that is no longer on the CDN) still
                    // needs a distinct, recognisable skin; a lobby of identical
                    // stand-in Steves is not a useful render test.
                    let image = download(&hash).unwrap_or_else(|| synthetic_skin(&hash));
                    if result_tx.send((hash, image)).is_err() {
                        break;
                    }
                })
                .expect("spawn skin worker");
        }

        Self {
            requests: request_tx,
            results: result_rx,
            requested: HashSet::new(),
            failed: HashSet::new(),
        }
    }

    /// Ask for a skin by texture hash. Silently ignored if already requested.
    pub fn request(&mut self, hash: &str) {
        if self.requested.contains(hash) || self.failed.contains(hash) {
            return;
        }
        self.requested.insert(hash.to_string());
        if self.requests.send(hash.to_string()).is_err() {
            self.failed.insert(hash.to_string());
        }
    }

    /// One freshly downloaded skin, if any finished.
    pub fn try_recv(&mut self) -> Option<(String, RgbaImage)> {
        self.results.try_recv().ok()
    }
}

/// `https://textures.minecraft.net/texture/<hash>` as a 64×64 RGBA image.
fn download(hash: &str) -> Option<RgbaImage> {
    let agent = ureq::AgentBuilder::new().timeout(FETCH_TIMEOUT).build();
    let url = format!("https://textures.minecraft.net/texture/{hash}");
    let response = agent.get(&url).call().ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes).ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some(normalise(decoded))
}

/// A deterministic Steve-shaped skin for a hash we could not download. The
/// shirt and trousers are tinted by the hash, so every bot in a lobby reads as
/// a different player even without network access.
pub fn synthetic_skin(hash: &str) -> RgbaImage {
    let mut image = nephele_assets::default_skin();
    let mut seed: u32 = 2_166_136_261;
    for byte in hash.bytes() {
        seed = seed.wrapping_mul(16_777_619) ^ u32::from(byte);
    }
    let hue = seed % 360;
    let shirt = hsv(hue, 0.65, 0.85);
    let pants = hsv((hue + 150) % 360, 0.55, 0.65);
    let fill = |image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, colour: [u8; 4]| {
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, image::Rgba(colour));
            }
        }
    };
    fill(&mut image, 16, 16, 40, 32, shirt);
    fill(&mut image, 40, 16, 56, 24, shirt);
    fill(&mut image, 32, 48, 48, 56, shirt);
    fill(&mut image, 0, 16, 16, 24, pants);
    fill(&mut image, 16, 48, 32, 56, pants);
    // A slightly darker second layer so the jacket/sleeves/trousers are
    // visibly a layer rather than an invisible copy of the base.
    let jacket = shade(shirt, 0.55);
    let trouser_layer = shade(pants, 0.55);
    fill(&mut image, 16, 32, 40, 48, jacket);
    fill(&mut image, 40, 32, 56, 48, jacket);
    fill(&mut image, 48, 48, 64, 64, jacket);
    fill(&mut image, 0, 32, 16, 48, trouser_layer);
    fill(&mut image, 0, 48, 16, 64, trouser_layer);
    image
}

fn shade(colour: [u8; 4], factor: f32) -> [u8; 4] {
    [
        (colour[0] as f32 * factor).min(255.0) as u8,
        (colour[1] as f32 * factor).min(255.0) as u8,
        (colour[2] as f32 * factor).min(255.0) as u8,
        colour[3],
    ]
}

fn hsv(hue: u32, saturation: f32, value: f32) -> [u8; 4] {
    let h = (hue % 360) as f32 / 60.0;
    let c = value * saturation;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
        255,
    ]
}

/// Pad or crop to the canonical 64×64 skin so UV maths never has to special-case
/// legacy 64×32 skins.
pub fn normalise(image: RgbaImage) -> RgbaImage {
    if image.width() == SKIN_SIZE && image.height() == SKIN_SIZE {
        return image;
    }
    let mut out = RgbaImage::from_pixel(SKIN_SIZE, SKIN_SIZE, image::Rgba([0, 0, 0, 0]));
    let width = image.width().min(SKIN_SIZE);
    let height = image.height().min(SKIN_SIZE);
    for y in 0..height {
        for x in 0..width {
            out.put_pixel(x, y, *image.get_pixel(x, y));
        }
    }
    // Legacy 64×32 skins keep their limbs mirrored; copy the right-hand limbs
    // into the left-hand slots the 64×64 layout uses.
    if image.height() <= 32 {
        mirror_legacy_limbs(&mut out);
    }
    out
}

fn mirror_legacy_limbs(skin: &mut RgbaImage) {
    let copies = [
        // (source x, source y, width, height, dest x, dest y)
        (0, 16, 16, 16, 16, 48),  // right leg -> left leg
        (40, 16, 16, 16, 32, 48), // right arm -> left arm
    ];
    for (sx, sy, w, h, dx, dy) in copies {
        for y in 0..h {
            for x in 0..w {
                let pixel = *skin.get_pixel(sx + x, sy + y);
                skin.put_pixel(dx + x, dy + y, pixel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_skins_are_padded_and_mirrored() {
        let mut legacy = RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 0, 0]));
        legacy.put_pixel(0, 16, image::Rgba([10, 20, 30, 255]));
        let out = normalise(legacy);
        assert_eq!(out.dimensions(), (64, 64));
        // Copied into the left-leg slot at (16,48).
        assert_eq!(*out.get_pixel(16, 48), image::Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn synthetic_skins_differ_by_hash_and_keep_the_face() {
        let a = synthetic_skin("aaaaaaaa");
        let b = synthetic_skin("bbbbbbbb");
        assert_eq!(a.dimensions(), (64, 64));
        assert_ne!(a.get_pixel(20, 20), b.get_pixel(20, 20));
        // The face is still skin-coloured, not tinted by the shirt.
        assert_eq!(
            *a.get_pixel(10, 10),
            *nephele_assets::default_skin().get_pixel(10, 10)
        );
    }

    #[test]
    fn modern_skins_pass_through_untouched() {
        let mut skin = RgbaImage::from_pixel(64, 64, image::Rgba([1, 2, 3, 255]));
        skin.put_pixel(5, 5, image::Rgba([9, 9, 9, 255]));
        let out = normalise(skin);
        assert_eq!(*out.get_pixel(5, 5), image::Rgba([9, 9, 9, 255]));
        assert_eq!(out.dimensions(), (64, 64));
    }

    #[test]
    fn oversized_images_are_cropped() {
        let big = RgbaImage::from_pixel(128, 128, image::Rgba([4, 4, 4, 255]));
        let out = normalise(big);
        assert_eq!(out.dimensions(), (64, 64));
    }
}
