//! `nephele-assets` - vanilla texture loading.
//!
//! 1.8.9 ships one PNG per block texture inside `client.jar` under
//! `assets/minecraft/textures/blocks` (extracted by `nephele-prepare`, from the
//! player's own copy of the game). Nephele loads those into a GPU texture
//! **array**: one layer per texture, so every block face samples its own layer
//! with no atlas bleeding.
//!
//! The 2D GUI art (bitmap font, HUD icons, widgets) goes into a separate
//! [`UiAtlas`] so the HUD, menus and item icons are a single draw call.

pub mod atlas;
pub mod block_models;
pub mod block_textures;
pub mod skin_atlas;
pub mod title;
pub mod ui_atlas;

pub use atlas::{AssetError, TextureArray, TILE_SIZE};
pub use block_models::{BlockModel, BlockModels, Element, FaceSpec, ModelFace};
pub use block_textures::{face_textures, is_transparent, FaceTextures, MISSING_TEXTURE};
pub use skin_atlas::{default_skin, SkinAtlas, ATLAS_SIZE, SKIN_SIZE, SLOT_SIZE};
pub use title::{Panorama, SplashTexts, PANORAMA_FACES};
pub use ui_atlas::{font, sheets, sprites, SpriteRect, UiAtlas};
