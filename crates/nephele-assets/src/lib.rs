//! `nephele-assets` - vanilla texture loading.
//!
//! 1.8.9 ships one PNG per block texture inside `client.jar` under
//! `assets/minecraft/textures/blocks` (extracted by `nephele-prepare`, from the
//! player's own copy of the game). Nephele loads those into a GPU texture
//! **array**: one layer per texture, so every block face samples its own layer
//! with no atlas bleeding. The same array also holds the supported, namespaced
//! `textures/items` layers used for inventory icons and dropped items.
//!
//! The 2D GUI art (bitmap font, HUD icons, widgets) goes into a separate
//! [`UiAtlas`] so the HUD, menus and item icons are a single draw call.

pub mod atlas;
pub mod block_models;
pub mod block_textures;
pub mod item_textures;
pub mod particle_textures;
pub mod skin_atlas;
pub mod title;
pub mod ui_atlas;

pub use atlas::{AssetError, TextureAnimation, TextureArray, TILE_SIZE};
pub use block_models::{BlockModel, BlockModels, Element, FaceSpec, ModelFace};
pub use block_textures::{face_textures, is_transparent, FaceTextures, MISSING_TEXTURE};
pub use item_textures::{all_item_texture_layers, item_texture_name};
pub use skin_atlas::{default_skin, SkinAtlas, ATLAS_SIZE, SKIN_SIZE, SLOT_SIZE};
pub use title::{Panorama, SplashTexts, PANORAMA_FACES};
pub use ui_atlas::{font, sheets, sprites, SpriteRect, UiAtlas};

/// Locates the player's own copy of the game for the tests that read it.
///
/// The repository ships no Minecraft files, so a fresh clone has none. Those
/// tests step aside rather than fail, which keeps `cargo test` meaningful for
/// anyone browsing the source. Run `nephele-prepare` to exercise them.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    pub(crate) fn minecraft() -> Option<PathBuf> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for candidate in [
            manifest.join("../../assets/vanilla/1.8.9/assets/minecraft"),
            manifest.join("../../.local/prepared/1.8.9/assets/minecraft"),
        ] {
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
        eprintln!(
            "skipped: no Minecraft 1.8.9 assets here; run `cargo run -p nephele-prepare` to enable this test"
        );
        None
    }
}
