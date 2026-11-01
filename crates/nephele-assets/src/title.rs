//! Title-screen assets: the six-faced panorama cube and the splash texts.

use std::path::Path;

use image::RgbaImage;

use crate::atlas::AssetError;

/// Number of faces in a panorama (`panorama_0` .. `panorama_5`).
pub const PANORAMA_FACES: usize = 6;

/// The title screen's background cube: six 256x256 faces.
#[derive(Debug, Clone)]
pub struct Panorama {
    pub size: u32,
    /// Face 0..=3 are the sides, 4 is the top and 5 the bottom.
    pub faces: Vec<RgbaImage>,
}

impl Panorama {
    pub fn load(textures_dir: &Path) -> Result<Self, AssetError> {
        let mut faces = Vec::with_capacity(PANORAMA_FACES);
        for index in 0..PANORAMA_FACES {
            let path = textures_dir
                .join("gui/title/background")
                .join(format!("panorama_{index}.png"));
            if !path.is_file() {
                return Err(AssetError::MissingDirectory(path.display().to_string()));
            }
            let image = image::open(&path).map_err(|source| AssetError::Image {
                path: path.display().to_string(),
                source,
            })?;
            faces.push(image.to_rgba8());
        }
        let size = faces[0].width().max(faces[0].height());
        Ok(Self { size, faces })
    }

    /// RGBA bytes for the whole panorama, face after face.
    pub fn pixels(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.faces.len() * (self.size * self.size * 4) as usize);
        for face in &self.faces {
            // Normalise every face to size x size.
            if face.width() == self.size && face.height() == self.size {
                out.extend_from_slice(face.as_raw());
            } else {
                let resized = image::imageops::resize(
                    face,
                    self.size,
                    self.size,
                    image::imageops::FilterType::Nearest,
                );
                out.extend_from_slice(resized.as_raw());
            }
        }
        out
    }
}

/// The yellow rotating splash line under the logo.
#[derive(Debug, Clone, Default)]
pub struct SplashTexts {
    pub lines: Vec<String>,
}

impl SplashTexts {
    pub fn load(texts_dir: &Path) -> Result<Self, AssetError> {
        let path = texts_dir.join("splashes.txt");
        let text = std::fs::read_to_string(&path)
            .map_err(|_| AssetError::MissingDirectory(path.display().to_string()))?;
        let lines: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        if lines.is_empty() {
            return Err(AssetError::NoTextures(path.display().to_string()));
        }
        Ok(Self { lines })
    }

    /// A stable choice for a given seed (so the splash doesn't flicker).
    pub fn pick(&self, seed: u64) -> &str {
        if self.lines.is_empty() {
            return "";
        }
        let index = (splitmix64(seed) % self.lines.len() as u64) as usize;
        &self.lines[index]
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn minecraft() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/vanilla/1.8.9/assets/minecraft")
    }

    #[test]
    fn loads_the_six_panorama_faces() {
        let panorama = Panorama::load(&minecraft().join("textures")).expect("panorama");
        assert_eq!(panorama.faces.len(), 6);
        assert_eq!(panorama.size, 256);
        assert_eq!(panorama.pixels().len(), 6 * 256 * 256 * 4, "six RGBA faces");
    }

    #[test]
    fn loads_splashes_and_picks_stably() {
        let splashes = SplashTexts::load(&minecraft().join("texts")).expect("splashes");
        assert!(splashes.len() > 100, "{} splashes", splashes.len());
        assert_eq!(splashes.pick(42), splashes.pick(42));
        assert!(!splashes.pick(1).is_empty());
    }
}
