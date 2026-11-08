//! Player-facing options, and the rules for nudging them.
//!
//! Every row of the options screens is backed by a field here, persisted to
//! `settings.json` in the per-user data directory ([`crate::paths::user_data_dir`]).
//! Every field is optional in the stored form, so a file written by an older or
//! newer build still loads: unknown keys are ignored and missing ones keep their
//! default. Values are clamped on load rather than trusted, because the file is
//! hand-editable.
//!
//! Not every option has a renderer behind it yet. `Graphics`, `SmoothLighting`,
//! `DynamicLights` and `AlternateBlocks` are the ones that need `nephele-render`
//! changes (mesher flags / lightmap); they persist and display, and the screen
//! marks them as pending so they cannot pass for working controls.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hud_modules::{ModuleConfig, ModuleId};
use crate::keybinds::{Action as BindableAction, Bind, Keybinds};
use crate::paths;

/// World difficulty. Single-player has nothing to spawn yet, so this is stored
/// and shown rather than acted on; a server owns it in multiplayer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 4] = [
        Difficulty::Peaceful,
        Difficulty::Easy,
        Difficulty::Normal,
        Difficulty::Hard,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Difficulty::Peaceful => "Peaceful",
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Difficulty::Peaceful => "peaceful",
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hard => "hard",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn step(self, direction: i32) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(2) as i32;
        Self::ALL[(index + direction).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

/// `Video Settings` graphics mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Graphics {
    Fast,
    Fancy,
}

impl Graphics {
    pub const fn label(self) -> &'static str {
        match self {
            Graphics::Fast => "Fast",
            Graphics::Fancy => "Fancy",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Graphics::Fast => "fast",
            Graphics::Fancy => "fancy",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "fast" => Some(Graphics::Fast),
            "fancy" => Some(Graphics::Fancy),
            _ => None,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Graphics::Fast => Graphics::Fancy,
            Graphics::Fancy => Graphics::Fast,
        }
    }
}

/// Vanilla's brightness slider, in five detents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brightness {
    Moody,
    Dark,
    Normal,
    Bright,
    Brightest,
}

impl Brightness {
    pub const ALL: [Brightness; 5] = [
        Brightness::Moody,
        Brightness::Dark,
        Brightness::Normal,
        Brightness::Bright,
        Brightness::Brightest,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Brightness::Moody => "Moody",
            Brightness::Dark => "Dark",
            Brightness::Normal => "Normal",
            Brightness::Bright => "Bright",
            Brightness::Brightest => "Brightest",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Brightness::Moody => "moody",
            Brightness::Dark => "dark",
            Brightness::Normal => "normal",
            Brightness::Bright => "bright",
            Brightness::Brightest => "brightest",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn step(self, direction: i32) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(2) as i32;
        Self::ALL[(index + direction).clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    /// Map one stored 0..=15 light value for this brightness.
    ///
    /// `Normal` is the identity so changing nothing keeps the reference look;
    /// the two darker steps scale the light down and the two brighter ones lift
    /// the floor towards full daylight.
    pub fn adjust_light(self, level: u8) -> u8 {
        let level = level.min(15) as f32;
        let out = match self {
            Brightness::Moody => level * 0.70,
            Brightness::Dark => level * 0.86,
            Brightness::Normal => level,
            Brightness::Bright => level + (15.0 - level) * 0.25,
            Brightness::Brightest => level + (15.0 - level) * 0.50,
        };
        out.round().clamp(0.0, 15.0) as u8
    }
}

/// Frame cap choices, in the order the button cycles them. `0` is unlimited.
pub const MAX_FRAMERATES: [u16; 9] = [0, 30, 60, 75, 90, 120, 144, 165, 240];

/// Chat look, applied by `ui::draw_chat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatOptions {
    /// Background opacity, percent.
    pub opacity: u8,
    /// Text scale, percent.
    pub scale: u8,
    /// Line width limit, percent of the window.
    pub width: u8,
    pub links: bool,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            opacity: 100,
            scale: 100,
            width: 100,
            links: true,
        }
    }
}

impl ChatOptions {
    pub const OPACITY_STEP: u8 = 10;
    pub const SCALE_STEP: u8 = 10;
    pub const WIDTH_STEP: u8 = 10;

    pub fn adjust(&mut self, row: ChatRow, direction: i32) {
        let apply = |value: &mut u8, min: u8, max: u8, step: u8| {
            *value = (*value as i32 + direction * step as i32).clamp(min as i32, max as i32) as u8;
        };
        match row {
            ChatRow::Opacity => apply(&mut self.opacity, 10, 100, Self::OPACITY_STEP),
            ChatRow::Scale => apply(&mut self.scale, 50, 150, Self::SCALE_STEP),
            ChatRow::Width => apply(&mut self.width, 30, 100, Self::WIDTH_STEP),
            ChatRow::Links => self.links = !self.links,
        }
    }

    pub fn label(&self, row: ChatRow) -> String {
        match row {
            ChatRow::Opacity => format!("{}%", self.opacity),
            ChatRow::Scale => format!("{}%", self.scale),
            ChatRow::Width => format!("{}%", self.width),
            ChatRow::Links => if self.links { "ON" } else { "OFF" }.to_string(),
        }
    }
}

/// One row of the chat settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRow {
    Opacity,
    Scale,
    Width,
    Links,
}

impl ChatRow {
    pub const ALL: [ChatRow; 4] = [
        ChatRow::Opacity,
        ChatRow::Scale,
        ChatRow::Width,
        ChatRow::Links,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ChatRow::Opacity => "Chat Opacity",
            ChatRow::Scale => "Chat Text Size",
            ChatRow::Width => "Chat Width",
            ChatRow::Links => "Chat Links",
        }
    }
}

/// Volume sliders, percent. Nothing consumes these yet â€” the audio lane owns
/// the sound engine â€” but they are stored, shown and changed like any other
/// option, so the engine has one place to read its mix from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundOptions {
    pub master: u8,
    pub music: u8,
    pub blocks: u8,
    pub weather: u8,
}

impl Default for SoundOptions {
    fn default() -> Self {
        Self {
            master: 100,
            music: 100,
            blocks: 100,
            weather: 100,
        }
    }
}

/// One row of the sound screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundRow {
    Master,
    Music,
    Blocks,
    Weather,
}

impl SoundRow {
    pub const ALL: [SoundRow; 4] = [
        SoundRow::Master,
        SoundRow::Music,
        SoundRow::Blocks,
        SoundRow::Weather,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SoundRow::Master => "Master Volume",
            SoundRow::Music => "Music",
            SoundRow::Blocks => "Blocks",
            SoundRow::Weather => "Weather",
        }
    }
}

impl SoundOptions {
    pub fn get(&self, row: SoundRow) -> u8 {
        match row {
            SoundRow::Master => self.master,
            SoundRow::Music => self.music,
            SoundRow::Blocks => self.blocks,
            SoundRow::Weather => self.weather,
        }
    }

    pub fn set(&mut self, row: SoundRow, value: u8) {
        let value = value.min(100);
        match row {
            SoundRow::Master => self.master = value,
            SoundRow::Music => self.music = value,
            SoundRow::Blocks => self.blocks = value,
            SoundRow::Weather => self.weather = value,
        }
    }

    pub fn adjust(&mut self, row: SoundRow, direction: i32) {
        let value = self.get(row) as i32 + direction * 10;
        self.set(row, value.clamp(0, 100) as u8);
    }
}

/// Which skin parts to draw. Stored for the entity renderer; nothing reads them
/// yet, because skin part visibility lives in `nephele-render::entity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinParts {
    pub cape: bool,
    pub jacket: bool,
    pub left_sleeve: bool,
    pub right_sleeve: bool,
    pub left_pants: bool,
    pub right_pants: bool,
    pub hat: bool,
}

impl Default for SkinParts {
    fn default() -> Self {
        Self {
            cape: true,
            jacket: true,
            left_sleeve: true,
            right_sleeve: true,
            left_pants: true,
            right_pants: true,
            hat: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinPart {
    Cape,
    Jacket,
    LeftSleeve,
    RightSleeve,
    LeftPants,
    RightPants,
    Hat,
}

impl SkinPart {
    pub const ALL: [SkinPart; 7] = [
        SkinPart::Cape,
        SkinPart::Jacket,
        SkinPart::LeftSleeve,
        SkinPart::RightSleeve,
        SkinPart::LeftPants,
        SkinPart::RightPants,
        SkinPart::Hat,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SkinPart::Cape => "Cape",
            SkinPart::Jacket => "Jacket",
            SkinPart::LeftSleeve => "Left Sleeve",
            SkinPart::RightSleeve => "Right Sleeve",
            SkinPart::LeftPants => "Left Pants",
            SkinPart::RightPants => "Right Pants",
            SkinPart::Hat => "Hat",
        }
    }
}

impl SkinParts {
    pub fn get(&self, part: SkinPart) -> bool {
        match part {
            SkinPart::Cape => self.cape,
            SkinPart::Jacket => self.jacket,
            SkinPart::LeftSleeve => self.left_sleeve,
            SkinPart::RightSleeve => self.right_sleeve,
            SkinPart::LeftPants => self.left_pants,
            SkinPart::RightPants => self.right_pants,
            SkinPart::Hat => self.hat,
        }
    }

    pub fn toggle(&mut self, part: SkinPart) {
        let slot = match part {
            SkinPart::Cape => &mut self.cape,
            SkinPart::Jacket => &mut self.jacket,
            SkinPart::LeftSleeve => &mut self.left_sleeve,
            SkinPart::RightSleeve => &mut self.right_sleeve,
            SkinPart::LeftPants => &mut self.left_pants,
            SkinPart::RightPants => &mut self.right_pants,
            SkinPart::Hat => &mut self.hat,
        };
        *slot = !*slot;
    }
}

/// The language list. One entry today: there is no translation table, and a
/// fake second language would be worse than an honest single choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    EnglishUs,
}

impl Language {
    pub const ALL: [Language; 1] = [Language::EnglishUs];

    pub const fn label(self) -> &'static str {
        match self {
            Language::EnglishUs => "English (US)",
        }
    }

    pub const fn key(self) -> &'static str {
        "en_us"
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// Installed resource packs. The client ships none and cannot load a pack yet,
/// so the list has the built-in one; the selection is still remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePack {
    Vanilla,
}

impl ResourcePack {
    pub const ALL: [ResourcePack; 1] = [ResourcePack::Vanilla];

    pub const fn label(self) -> &'static str {
        match self {
            ResourcePack::Vanilla => "Default",
        }
    }

    pub const fn key(self) -> &'static str {
        "vanilla"
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// One adjustable row across the options screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionRow {
    Fov,
    Difficulty,
    Graphics,
    SmoothLighting,
    SmoothLightingLevel,
    GuiScale,
    Brightness,
    DynamicLights,
    RenderDistance,
    MaxFramerate,
    ViewBobbing,
    Vsync,
    AlternateBlocks,
    Chat(ChatRow),
    Volume(SoundRow),
}

impl OptionRow {
    /// Rows that change immediately; the rest persist and display, waiting on a
    /// renderer feature. `screen_note` names the missing piece.
    pub fn takes_effect(self) -> bool {
        !matches!(
            self,
            OptionRow::Graphics
                | OptionRow::SmoothLighting
                | OptionRow::SmoothLightingLevel
                | OptionRow::DynamicLights
                | OptionRow::AlternateBlocks
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub vsync: bool,
    /// Multiplier on the base mouse sensitivity.
    pub sensitivity: f32,
    pub fov: f32,
    /// 0 = automatic, 1..=4 = fixed.
    pub gui_scale: u8,
    pub view_distance: u8,
    pub difficulty: Difficulty,
    pub graphics: Graphics,
    pub smooth_lighting: bool,
    /// Percent, 0..=100.
    pub smooth_lighting_level: u8,
    pub brightness: Brightness,
    pub dynamic_lights: bool,
    /// 0 = unlimited.
    pub max_framerate: u16,
    pub view_bobbing: bool,
    pub alternate_blocks: bool,
    pub chat: ChatOptions,
    pub sounds: SoundOptions,
    pub language: Language,
    pub resource_pack: ResourcePack,
    pub skin_parts: SkinParts,
    /// HUD module toggles and positions.
    pub modules: ModuleConfig,
    pub keys: Keybinds,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vsync: true,
            sensitivity: 1.0,
            fov: 70.0,
            gui_scale: 0,
            view_distance: 8,
            difficulty: Difficulty::Normal,
            graphics: Graphics::Fast,
            smooth_lighting: true,
            smooth_lighting_level: 100,
            brightness: Brightness::Normal,
            dynamic_lights: false,
            max_framerate: 0,
            view_bobbing: true,
            alternate_blocks: false,
            chat: ChatOptions::default(),
            sounds: SoundOptions::default(),
            language: Language::EnglishUs,
            resource_pack: ResourcePack::Vanilla,
            skin_parts: SkinParts::default(),
            modules: ModuleConfig::default(),
            keys: Keybinds::default(),
        }
    }
}

impl Settings {
    /// Value shown on the right of a row.
    pub fn value_label(&self, row: OptionRow) -> String {
        match row {
            OptionRow::Fov => format!("{:.0}", self.fov),
            OptionRow::Difficulty => self.difficulty.label().to_string(),
            OptionRow::Graphics => self.graphics.label().to_string(),
            OptionRow::SmoothLighting => on_off(self.smooth_lighting),
            OptionRow::SmoothLightingLevel => format!("{}%", self.smooth_lighting_level),
            OptionRow::GuiScale => {
                if self.gui_scale == 0 {
                    "Auto".to_string()
                } else {
                    self.gui_scale.to_string()
                }
            }
            OptionRow::Brightness => self.brightness.label().to_string(),
            OptionRow::DynamicLights => on_off(self.dynamic_lights),
            OptionRow::RenderDistance => format!("{} chunks", self.view_distance),
            OptionRow::MaxFramerate => {
                if self.max_framerate == 0 {
                    "Unlimited".to_string()
                } else {
                    format!("{} fps", self.max_framerate)
                }
            }
            OptionRow::ViewBobbing => on_off(self.view_bobbing),
            OptionRow::Vsync => on_off(self.vsync),
            OptionRow::AlternateBlocks => on_off(self.alternate_blocks),
            OptionRow::Chat(row) => self.chat.label(row),
            OptionRow::Volume(row) => format!("{}%", self.sounds.get(row)),
        }
    }

    /// Apply a `-1`/`+1` nudge. Returns `true` when something changed.
    ///
    /// `Range` rows step; `Toggle` rows flip and ignore the direction.
    pub fn adjust(&mut self, row: OptionRow, direction: i32) -> bool {
        let before = self.clone();
        match row {
            OptionRow::Fov => self.fov = (self.fov + 5.0 * direction as f32).clamp(30.0, 110.0),
            OptionRow::Difficulty => self.difficulty = self.difficulty.step(direction),
            OptionRow::Graphics => self.graphics = self.graphics.toggled(),
            OptionRow::SmoothLighting => self.smooth_lighting = !self.smooth_lighting,
            OptionRow::SmoothLightingLevel => {
                self.smooth_lighting_level =
                    (self.smooth_lighting_level as i32 + direction * 10).clamp(0, 100) as u8;
            }
            OptionRow::GuiScale => {
                self.gui_scale = (self.gui_scale as i32 + direction).rem_euclid(5) as u8;
            }
            OptionRow::Brightness => self.brightness = self.brightness.step(direction),
            OptionRow::DynamicLights => self.dynamic_lights = !self.dynamic_lights,
            OptionRow::RenderDistance => {
                self.view_distance = (self.view_distance as i32 + direction).clamp(2, 16) as u8;
            }
            OptionRow::MaxFramerate => {
                let index = MAX_FRAMERATES
                    .iter()
                    .position(|value| *value == self.max_framerate)
                    .unwrap_or(0) as i32;
                let next = (index + direction).rem_euclid(MAX_FRAMERATES.len() as i32) as usize;
                self.max_framerate = MAX_FRAMERATES[next];
            }
            OptionRow::ViewBobbing => self.view_bobbing = !self.view_bobbing,
            OptionRow::Vsync => self.vsync = !self.vsync,
            OptionRow::AlternateBlocks => self.alternate_blocks = !self.alternate_blocks,
            OptionRow::Chat(row) => self.chat.adjust(row, direction),
            OptionRow::Volume(row) => self.sounds.adjust(row, direction),
        }
        *self != before
    }

    /// Set a slider from a `0..=1` position.
    pub fn set_slider(&mut self, row: OptionRow, fraction: f32) -> bool {
        let before = self.clone();
        let fraction = fraction.clamp(0.0, 1.0);
        match row {
            OptionRow::Fov => {
                self.fov = (30.0 + fraction * 80.0) / 5.0 * 5.0;
            }
            OptionRow::Brightness => {
                let index = (fraction * (Brightness::ALL.len() - 1) as f32).round() as i32;
                self.brightness = Brightness::ALL[index.clamp(0, 4) as usize];
            }
            OptionRow::SmoothLightingLevel => {
                self.smooth_lighting_level = (fraction * 100.0).round() as u8;
            }
            OptionRow::Volume(row) => {
                let value = (fraction * 100.0).round() as u8;
                self.sounds.set(row, value);
            }
            OptionRow::Chat(ChatRow::Opacity) => {
                self.chat.opacity = (fraction * 100.0).round().clamp(10.0, 100.0) as u8;
            }
            OptionRow::Chat(ChatRow::Scale) => {
                self.chat.scale = (50.0 + fraction * 100.0).round() as u8;
            }
            OptionRow::Chat(ChatRow::Width) => {
                self.chat.width = (30.0 + fraction * 70.0).round() as u8;
            }
            _ => {}
        }
        *self != before
    }

    /// `settings.json` in the per-user data directory.
    pub fn path() -> PathBuf {
        paths::user_data_dir().join("settings.json")
    }

    /// Load the saved settings, falling back to defaults for a missing,
    /// unreadable or malformed file. Never fails: a client that cannot read its
    /// own settings should still start.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<StoredSettings>(&text) {
            Ok(stored) => Self::from_stored(stored),
            Err(err) => {
                eprintln!("settings: {path:?} is not valid settings JSON ({err}); using defaults");
                Self::default()
            }
        }
    }

    /// Best-effort save; a read-only profile directory must not break play.
    pub fn save(&self) {
        if let Err(err) = self.save_to(&Self::path()) {
            eprintln!("settings: could not save {:?}: {err}", Self::path());
        }
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.to_stored())
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        std::fs::write(path, text)
    }

    fn to_stored(&self) -> StoredSettings {
        let mut modules = BTreeMap::new();
        for id in ModuleId::ALL {
            let entry = self.modules.get(id);
            modules.insert(
                id.key().to_string(),
                StoredModule {
                    enabled: Some(entry.enabled),
                    anchor: Some(entry.anchor),
                },
            );
        }
        let mut keys = BTreeMap::new();
        for action in BindableAction::ALL {
            keys.insert(action.id().to_string(), self.keys.get(action).encode());
        }
        StoredSettings {
            vsync: Some(self.vsync),
            sensitivity: Some(self.sensitivity),
            fov: Some(self.fov),
            gui_scale: Some(self.gui_scale as u32),
            view_distance: Some(self.view_distance as u32),
            difficulty: Some(self.difficulty.key().to_string()),
            graphics: Some(self.graphics.key().to_string()),
            smooth_lighting: Some(self.smooth_lighting),
            smooth_lighting_level: Some(self.smooth_lighting_level as u32),
            brightness: Some(self.brightness.key().to_string()),
            dynamic_lights: Some(self.dynamic_lights),
            max_framerate: Some(self.max_framerate as u32),
            view_bobbing: Some(self.view_bobbing),
            alternate_blocks: Some(self.alternate_blocks),
            chat: Some(StoredChat {
                opacity: Some(self.chat.opacity as u32),
                scale: Some(self.chat.scale as u32),
                width: Some(self.chat.width as u32),
                links: Some(self.chat.links),
            }),
            sounds: Some(StoredSounds {
                master: Some(self.sounds.master as u32),
                music: Some(self.sounds.music as u32),
                blocks: Some(self.sounds.blocks as u32),
                weather: Some(self.sounds.weather as u32),
            }),
            language: Some(self.language.key().to_string()),
            resource_pack: Some(self.resource_pack.key().to_string()),
            skin_parts: Some(StoredSkinParts {
                cape: Some(self.skin_parts.cape),
                jacket: Some(self.skin_parts.jacket),
                left_sleeve: Some(self.skin_parts.left_sleeve),
                right_sleeve: Some(self.skin_parts.right_sleeve),
                left_pants: Some(self.skin_parts.left_pants),
                right_pants: Some(self.skin_parts.right_pants),
                hat: Some(self.skin_parts.hat),
            }),
            modules,
            keys,
        }
    }

    fn from_stored(stored: StoredSettings) -> Self {
        let mut settings = Self::default();
        if let Some(vsync) = stored.vsync {
            settings.vsync = vsync;
        }
        if let Some(sensitivity) = stored.sensitivity {
            settings.sensitivity = sensitivity.clamp(0.4, 3.0);
        }
        if let Some(fov) = stored.fov {
            settings.fov = fov.clamp(30.0, 110.0);
        }
        if let Some(scale) = stored.gui_scale {
            settings.gui_scale = scale.min(4) as u8;
        }
        if let Some(distance) = stored.view_distance {
            settings.view_distance = distance.clamp(2, 16) as u8;
        }
        if let Some(value) = stored.difficulty.as_deref().and_then(Difficulty::from_key) {
            settings.difficulty = value;
        }
        if let Some(value) = stored.graphics.as_deref().and_then(Graphics::from_key) {
            settings.graphics = value;
        }
        if let Some(value) = stored.smooth_lighting {
            settings.smooth_lighting = value;
        }
        if let Some(value) = stored.smooth_lighting_level {
            settings.smooth_lighting_level = value.min(100) as u8;
        }
        if let Some(value) = stored.brightness.as_deref().and_then(Brightness::from_key) {
            settings.brightness = value;
        }
        if let Some(value) = stored.dynamic_lights {
            settings.dynamic_lights = value;
        }
        if let Some(value) = stored.max_framerate {
            settings.max_framerate =
                if value <= u16::MAX as u32 && MAX_FRAMERATES.contains(&(value as u16)) {
                    value as u16
                } else {
                    0
                };
        }
        if let Some(value) = stored.view_bobbing {
            settings.view_bobbing = value;
        }
        if let Some(value) = stored.alternate_blocks {
            settings.alternate_blocks = value;
        }
        if let Some(chat) = stored.chat {
            if let Some(value) = chat.opacity {
                settings.chat.opacity = value.clamp(10, 100) as u8;
            }
            if let Some(value) = chat.scale {
                settings.chat.scale = value.clamp(50, 150) as u8;
            }
            if let Some(value) = chat.width {
                settings.chat.width = value.clamp(30, 100) as u8;
            }
            if let Some(value) = chat.links {
                settings.chat.links = value;
            }
        }
        if let Some(sounds) = stored.sounds {
            if let Some(value) = sounds.master {
                settings.sounds.master = value.min(100) as u8;
            }
            if let Some(value) = sounds.music {
                settings.sounds.music = value.min(100) as u8;
            }
            if let Some(value) = sounds.blocks {
                settings.sounds.blocks = value.min(100) as u8;
            }
            if let Some(value) = sounds.weather {
                settings.sounds.weather = value.min(100) as u8;
            }
        }
        if let Some(value) = stored.language.as_deref().and_then(Language::from_key) {
            settings.language = value;
        }
        if let Some(value) = stored
            .resource_pack
            .as_deref()
            .and_then(ResourcePack::from_key)
        {
            settings.resource_pack = value;
        }
        if let Some(parts) = stored.skin_parts {
            let target = &mut settings.skin_parts;
            target.cape = parts.cape.unwrap_or(target.cape);
            target.jacket = parts.jacket.unwrap_or(target.jacket);
            target.left_sleeve = parts.left_sleeve.unwrap_or(target.left_sleeve);
            target.right_sleeve = parts.right_sleeve.unwrap_or(target.right_sleeve);
            target.left_pants = parts.left_pants.unwrap_or(target.left_pants);
            target.right_pants = parts.right_pants.unwrap_or(target.right_pants);
            target.hat = parts.hat.unwrap_or(target.hat);
        }
        for (key, module) in stored.modules {
            let Some(id) = ModuleId::from_key(&key) else {
                continue;
            };
            let entry = settings.modules.get_mut(id);
            if let Some(enabled) = module.enabled {
                entry.enabled = enabled;
            }
            if let Some(anchor) = module.anchor {
                entry.anchor = crate::hud_modules::ModuleEntry::clamp_anchor(anchor);
            }
        }
        for (key, bind) in stored.keys {
            let Some(action) = BindableAction::from_id(&key) else {
                continue;
            };
            settings.keys.set(action, Bind::decode(&bind));
        }
        settings
    }
}

fn on_off(value: bool) -> String {
    if value { "ON" } else { "OFF" }.to_string()
}

/// The on-disk form. Separate from [`Settings`] so the file stays readable and
/// stable even if the in-memory shape changes.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredSettings {
    #[serde(default)]
    vsync: Option<bool>,
    #[serde(default)]
    sensitivity: Option<f32>,
    #[serde(default)]
    fov: Option<f32>,
    #[serde(default)]
    gui_scale: Option<u32>,
    #[serde(default)]
    view_distance: Option<u32>,
    #[serde(default)]
    difficulty: Option<String>,
    #[serde(default)]
    graphics: Option<String>,
    #[serde(default)]
    smooth_lighting: Option<bool>,
    #[serde(default)]
    smooth_lighting_level: Option<u32>,
    #[serde(default)]
    brightness: Option<String>,
    #[serde(default)]
    dynamic_lights: Option<bool>,
    #[serde(default)]
    max_framerate: Option<u32>,
    #[serde(default)]
    view_bobbing: Option<bool>,
    #[serde(default)]
    alternate_blocks: Option<bool>,
    #[serde(default)]
    chat: Option<StoredChat>,
    #[serde(default)]
    sounds: Option<StoredSounds>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    resource_pack: Option<String>,
    #[serde(default)]
    skin_parts: Option<StoredSkinParts>,
    /// Keyed by [`ModuleId::key`]; a map rather than a list so one missing
    /// module does not shift the rest.
    #[serde(default)]
    modules: BTreeMap<String, StoredModule>,
    /// Keyed by [`BindableAction::id`].
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredModule {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    anchor: Option<[f32; 2]>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredChat {
    #[serde(default)]
    opacity: Option<u32>,
    #[serde(default)]
    scale: Option<u32>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    links: Option<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredSounds {
    #[serde(default)]
    master: Option<u32>,
    #[serde(default)]
    music: Option<u32>,
    #[serde(default)]
    blocks: Option<u32>,
    #[serde(default)]
    weather: Option<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredSkinParts {
    #[serde(default)]
    cape: Option<bool>,
    #[serde(default)]
    jacket: Option<bool>,
    #[serde(default)]
    left_sleeve: Option<bool>,
    #[serde(default)]
    right_sleeve: Option<bool>,
    #[serde(default)]
    left_pants: Option<bool>,
    #[serde(default)]
    right_pants: Option<bool>,
    #[serde(default)]
    hat: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{Action as BindableAction, Bind};
    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-options-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn vsync_toggles() {
        let mut settings = Settings::default();
        assert!(settings.vsync);
        assert!(settings.adjust(OptionRow::Vsync, 1));
        assert!(!settings.vsync);
        assert_eq!(settings.value_label(OptionRow::Vsync), "OFF");
    }

    #[test]
    fn fov_clamps_both_ways() {
        let mut settings = Settings::default();
        for _ in 0..30 {
            settings.adjust(OptionRow::Fov, 1);
        }
        assert_eq!(settings.fov, 110.0);
        for _ in 0..40 {
            settings.adjust(OptionRow::Fov, -1);
        }
        assert_eq!(settings.fov, 30.0);
    }

    #[test]
    fn gui_scale_wraps_through_auto() {
        let mut settings = Settings::default();
        assert_eq!(settings.value_label(OptionRow::GuiScale), "Auto");
        settings.adjust(OptionRow::GuiScale, 1);
        assert_eq!(settings.gui_scale, 1);
        for _ in 0..4 {
            settings.adjust(OptionRow::GuiScale, 1);
        }
        assert_eq!(settings.gui_scale, 0, "wraps back to Auto");
    }

    #[test]
    fn render_distance_is_bounded() {
        let mut settings = Settings::default();
        for _ in 0..40 {
            settings.adjust(OptionRow::RenderDistance, 1);
        }
        assert_eq!(settings.view_distance, 16);
        assert!(!settings.adjust(OptionRow::RenderDistance, 1));
        for _ in 0..40 {
            settings.adjust(OptionRow::RenderDistance, -1);
        }
        assert_eq!(settings.view_distance, 2);
    }

    #[test]
    fn framerate_cycles_through_the_list_and_wraps() {
        let mut settings = Settings::default();
        assert_eq!(settings.value_label(OptionRow::MaxFramerate), "Unlimited");
        settings.adjust(OptionRow::MaxFramerate, 1);
        assert_eq!(settings.max_framerate, 30);
        settings.adjust(OptionRow::MaxFramerate, -1);
        assert_eq!(settings.max_framerate, 0);
        for _ in 0..MAX_FRAMERATES.len() {
            settings.adjust(OptionRow::MaxFramerate, 1);
        }
        assert_eq!(
            settings.max_framerate, 0,
            "a full cycle returns to unlimited"
        );
    }

    #[test]
    fn toggles_ignore_the_direction_and_flip() {
        for row in [
            OptionRow::ViewBobbing,
            OptionRow::SmoothLighting,
            OptionRow::DynamicLights,
            OptionRow::AlternateBlocks,
            OptionRow::Vsync,
        ] {
            let mut settings = Settings::default();
            let before = settings.value_label(row);
            settings.adjust(row, -1);
            assert_ne!(settings.value_label(row), before, "{row:?}");
            settings.adjust(row, 1);
            assert_eq!(settings.value_label(row), before, "{row:?}");
        }
    }

    #[test]
    fn difficulty_wraps_through_all_four() {
        let mut settings = Settings::default();
        assert_eq!(settings.difficulty, Difficulty::Normal);
        let mut seen = vec![settings.difficulty];
        for _ in 0..3 {
            settings.adjust(OptionRow::Difficulty, 1);
            seen.push(settings.difficulty);
        }
        let mut keys: Vec<&str> = seen.iter().map(|value| value.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 4, "one step each, wrapping: {seen:?}");
        settings.adjust(OptionRow::Difficulty, 1);
        assert_eq!(settings.difficulty, Difficulty::Normal, "four steps wrap");
    }

    #[test]
    fn sliders_map_a_fraction_onto_their_range() {
        let mut settings = Settings::default();
        assert!(settings.set_slider(OptionRow::Volume(SoundRow::Master), 0.3));
        assert_eq!(settings.sounds.master, 30);
        settings.set_slider(OptionRow::Fov, 0.0);
        assert_eq!(settings.fov, 30.0);
        settings.set_slider(OptionRow::Fov, 1.0);
        assert_eq!(settings.fov, 110.0);
        settings.set_slider(OptionRow::Brightness, 1.0);
        assert_eq!(settings.brightness, Brightness::Brightest);
        settings.set_slider(OptionRow::Chat(ChatRow::Opacity), 0.0);
        assert_eq!(settings.chat.opacity, 10, "never fully invisible");
    }

    #[test]
    fn the_rows_that_need_the_renderer_are_marked() {
        assert!(OptionRow::RenderDistance.takes_effect());
        assert!(OptionRow::Vsync.takes_effect());
        assert!(OptionRow::ViewBobbing.takes_effect());
        assert!(!OptionRow::Graphics.takes_effect());
        assert!(!OptionRow::SmoothLighting.takes_effect());
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let path = temp_file("round-trip");
        let mut settings = Settings {
            vsync: false,
            sensitivity: 1.7,
            fov: 95.0,
            gui_scale: 2,
            view_distance: 12,
            difficulty: Difficulty::Hard,
            graphics: Graphics::Fancy,
            smooth_lighting: false,
            smooth_lighting_level: 40,
            brightness: Brightness::Bright,
            dynamic_lights: true,
            max_framerate: 144,
            view_bobbing: false,
            alternate_blocks: true,
            chat: ChatOptions {
                opacity: 60,
                scale: 120,
                width: 70,
                links: false,
            },
            sounds: SoundOptions {
                master: 80,
                music: 20,
                blocks: 100,
                weather: 0,
            },
            language: Language::EnglishUs,
            resource_pack: ResourcePack::Vanilla,
            skin_parts: SkinParts {
                cape: false,
                ..SkinParts::default()
            },
            ..Settings::default()
        };
        settings.modules.toggle(ModuleId::Cps);
        settings
            .modules
            .set_anchor(ModuleId::Keystrokes, [0.25, 0.75]);
        settings
            .keys
            .assign(BindableAction::Attack, Bind::Key(KeyCode::KeyR));

        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, settings);
        assert_eq!(
            loaded.keys.get(BindableAction::Attack),
            Bind::Key(KeyCode::KeyR)
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_file_is_the_defaults() {
        let path = temp_file("missing");
        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_it_omits() {
        let path = temp_file("partial");
        std::fs::write(
            &path,
            r#"{"fov": 100.0, "modules": {"fps": {"enabled": false}}}"#,
        )
        .unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.fov, 100.0);
        assert!(!loaded.modules.is_enabled(ModuleId::Fps));
        assert!(loaded.modules.is_enabled(ModuleId::Ping), "untouched");
        assert_eq!(loaded.view_distance, Settings::default().view_distance);
        assert_eq!(loaded.keys, Keybinds::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hand_edited_values_are_clamped_on_load() {
        let path = temp_file("clamped");
        std::fs::write(
            &path,
            r#"{"fov": 500.0, "sensitivity": -1.0, "view_distance": 99, "gui_scale": 9,
                "smooth_lighting_level": 250, "max_framerate": 37,
                "chat": {"opacity": 0, "scale": 999},
                "sounds": {"master": 200},
                "language": "klingon", "difficulty": "nightmare",
                "keys": {"attack": "key:NotAKey", "forward": "key:Q"},
                "modules": {"fps": {"anchor": [-2.0, 4.0]}}}"#,
        )
        .unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.fov, 110.0);
        assert_eq!(loaded.sensitivity, 0.4);
        assert_eq!(loaded.view_distance, 16);
        assert_eq!(loaded.gui_scale, 4);
        assert_eq!(loaded.smooth_lighting_level, 100);
        assert_eq!(
            loaded.max_framerate, 0,
            "an unknown cap falls back to unlimited"
        );
        assert_eq!(loaded.chat.opacity, 10);
        assert_eq!(loaded.chat.scale, 150);
        assert_eq!(loaded.sounds.master, 100);
        assert_eq!(loaded.language, Language::EnglishUs);
        assert_eq!(loaded.difficulty, Difficulty::Normal);
        assert_eq!(loaded.keys.get(BindableAction::Attack), Bind::Unbound);
        assert_eq!(
            loaded.keys.get(BindableAction::Forward),
            Bind::Key(KeyCode::KeyQ)
        );
        assert_eq!(loaded.modules.get(ModuleId::Fps).anchor, [0.0, 1.0]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults() {
        let path = temp_file("corrupt");
        std::fs::write(&path, "{ not json at all").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unknown_module_key_is_ignored_not_fatal() {
        let path = temp_file("unknown-module");
        std::fs::write(&path, r#"{"modules": {"armour": {"enabled": true}}}"#).unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn every_default_binding_encodes_and_decodes() {
        let binds = Keybinds::default();
        for action in BindableAction::ALL {
            let bind = binds.get(action);
            assert_eq!(Bind::decode(&bind.encode()), bind, "{action:?}");
        }
        assert_eq!(
            binds.get(BindableAction::Attack),
            Bind::Mouse(MouseButton::Left)
        );
    }

    #[test]
    fn brightness_leaves_the_default_look_alone() {
        // Normal must be the identity: it is the default, and a "settings" that
        // silently darkens everything on first launch is a bug.
        for level in 0..=15u8 {
            assert_eq!(Brightness::Normal.adjust_light(level), level);
        }
        assert!(Brightness::Moody.adjust_light(15) < 15);
        assert!(Brightness::Dark.adjust_light(8) < 8);
        assert!(Brightness::Bright.adjust_light(4) > 4);
        assert!(Brightness::Brightest.adjust_light(4) > Brightness::Bright.adjust_light(4));
        assert_eq!(Brightness::Brightest.adjust_light(15), 15, "capped");
        assert_eq!(Brightness::Moody.adjust_light(0), 0, "still dark");
    }

    #[test]
    fn chat_and_sound_rows_are_all_addressable() {
        // Several of these start at their maximum, so "nudge and expect a
        // change" has to try the other direction too.
        for row in ChatRow::ALL {
            let mut settings = Settings::default();
            let before = settings.value_label(OptionRow::Chat(row));
            let moved = settings.adjust(OptionRow::Chat(row), 1)
                || settings.adjust(OptionRow::Chat(row), -1);
            assert!(moved, "{row:?} cannot move in either direction");
            assert_ne!(
                settings.value_label(OptionRow::Chat(row)),
                before,
                "{row:?}"
            );
        }
        for row in SoundRow::ALL {
            let mut settings = Settings::default();
            settings.adjust(OptionRow::Volume(row), -1);
            assert_eq!(settings.sounds.get(row), 90, "{row:?}");
            settings.adjust(OptionRow::Volume(row), 5);
            assert_eq!(settings.sounds.get(row), 100, "{row:?} clamps at 100");
        }
    }

    #[test]
    fn skin_parts_toggle_independently() {
        let mut parts = SkinParts::default();
        parts.toggle(SkinPart::Hat);
        assert!(!parts.get(SkinPart::Hat));
        assert!(parts.get(SkinPart::Cape));
    }
}
