//! Player-facing video/input settings, and the rules for nudging them.
//!
//! Settings persist to `settings.json` in the per-user data directory
//! ([`crate::paths::user_data_dir`]), beside `servers.json` and `auth.json`.
//! Every field is `#[serde(default)]`, so a file written by an older or newer
//! build still loads: unknown keys are ignored and missing ones keep their
//! default. Values are clamped on load rather than trusted, because the file is
//! hand-editable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hud_modules::{ModuleConfig, ModuleId};
use crate::paths;

/// One adjustable row in the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    Vsync,
    Sensitivity,
    Fov,
    GuiScale,
    ViewDistance,
}

impl SettingId {
    pub const ALL: [SettingId; 5] = [
        SettingId::Vsync,
        SettingId::Sensitivity,
        SettingId::Fov,
        SettingId::GuiScale,
        SettingId::ViewDistance,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SettingId::Vsync => "VSync",
            SettingId::Sensitivity => "Mouse Sensitivity",
            SettingId::Fov => "Field of View",
            SettingId::GuiScale => "GUI Scale",
            SettingId::ViewDistance => "Render Distance",
        }
    }

    /// Whether the value can be nudged in this direction.
    pub fn can_adjust(self, settings: &Settings, direction: i32) -> bool {
        let mut probe = *settings;
        probe.adjust(self, direction)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub vsync: bool,
    /// Multiplier on the base mouse sensitivity.
    pub sensitivity: f32,
    pub fov: f32,
    /// 0 = automatic, 1..=4 = fixed.
    pub gui_scale: u8,
    pub view_distance: u8,
    /// HUD module toggles and positions.
    pub modules: ModuleConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vsync: true,
            sensitivity: 1.0,
            fov: 70.0,
            gui_scale: 0,
            view_distance: 8,
            modules: ModuleConfig::default(),
        }
    }
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
    gui_scale: Option<u8>,
    #[serde(default)]
    view_distance: Option<u8>,
    /// Keyed by [`ModuleId::key`]; a map rather than a list so one missing
    /// module does not shift the rest.
    #[serde(default)]
    modules: BTreeMap<String, StoredModule>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredModule {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    anchor: Option<[f32; 2]>,
}

impl Settings {
    /// `"ON"` / `"OFF"`, `"70"`, `"8 chunks"`, ...
    pub fn value_label(&self, id: SettingId) -> String {
        match id {
            SettingId::Vsync => if self.vsync { "ON" } else { "OFF" }.to_string(),
            SettingId::Sensitivity => format!("{:.0}%", self.sensitivity * 100.0),
            SettingId::Fov => format!("{:.0}", self.fov),
            SettingId::GuiScale => {
                if self.gui_scale == 0 {
                    "Auto".to_string()
                } else {
                    self.gui_scale.to_string()
                }
            }
            SettingId::ViewDistance => format!("{} chunks", self.view_distance),
        }
    }

    /// Apply a `-1`/`+1` nudge. Returns `true` when the setting changed.
    pub fn adjust(&mut self, id: SettingId, direction: i32) -> bool {
        let before = *self;
        match id {
            SettingId::Vsync => self.vsync = !self.vsync,
            SettingId::Sensitivity => {
                self.sensitivity = (self.sensitivity + 0.1 * direction as f32).clamp(0.4, 3.0);
            }
            SettingId::Fov => {
                self.fov = (self.fov + 5.0 * direction as f32).clamp(50.0, 110.0);
            }
            SettingId::GuiScale => {
                let scale = self.gui_scale as i32 + direction;
                self.gui_scale = scale.rem_euclid(5) as u8;
            }
            SettingId::ViewDistance => {
                let distance = self.view_distance as i32 + direction;
                self.view_distance = distance.clamp(2, 16) as u8;
            }
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

    fn to_stored(self) -> StoredSettings {
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
        StoredSettings {
            vsync: Some(self.vsync),
            sensitivity: Some(self.sensitivity),
            fov: Some(self.fov),
            gui_scale: Some(self.gui_scale),
            view_distance: Some(self.view_distance),
            modules,
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
            settings.fov = fov.clamp(50.0, 110.0);
        }
        if let Some(scale) = stored.gui_scale {
            settings.gui_scale = scale.min(4);
        }
        if let Some(distance) = stored.view_distance {
            settings.view_distance = distance.clamp(2, 16);
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
        settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsync_toggles() {
        let mut settings = Settings::default();
        assert!(settings.vsync);
        assert!(settings.adjust(SettingId::Vsync, 1));
        assert!(!settings.vsync);
        assert_eq!(settings.value_label(SettingId::Vsync), "OFF");
    }

    #[test]
    fn fov_and_sensitivity_clamp() {
        let mut settings = Settings::default();
        for _ in 0..50 {
            settings.adjust(SettingId::Fov, 1);
            settings.adjust(SettingId::Sensitivity, 1);
        }
        assert_eq!(settings.fov, 110.0);
        assert_eq!(settings.sensitivity, 3.0);
        for _ in 0..50 {
            settings.adjust(SettingId::Fov, -1);
            settings.adjust(SettingId::Sensitivity, -1);
        }
        assert_eq!(settings.fov, 50.0);
        assert_eq!(settings.sensitivity, 0.4);
    }

    #[test]
    fn gui_scale_wraps_through_auto() {
        let mut settings = Settings::default();
        assert_eq!(settings.value_label(SettingId::GuiScale), "Auto");
        settings.adjust(SettingId::GuiScale, 1);
        assert_eq!(settings.gui_scale, 1);
        for _ in 0..4 {
            settings.adjust(SettingId::GuiScale, 1);
        }
        assert_eq!(settings.gui_scale, 0, "wraps back to Auto");
    }

    #[test]
    fn render_distance_is_bounded() {
        let mut settings = Settings::default();
        for _ in 0..40 {
            settings.adjust(SettingId::ViewDistance, 1);
        }
        assert_eq!(settings.view_distance, 16);
        assert!(!settings.adjust(SettingId::ViewDistance, 1));
        for _ in 0..40 {
            settings.adjust(SettingId::ViewDistance, -1);
        }
        assert_eq!(settings.view_distance, 2);
        assert!(!settings.adjust(SettingId::ViewDistance, -1));
    }

    #[test]
    fn can_adjust_reports_the_limits() {
        let settings = Settings {
            fov: 110.0,
            ..Settings::default()
        };
        assert!(!SettingId::Fov.can_adjust(&settings, 1));
        assert!(SettingId::Fov.can_adjust(&settings, -1));
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-settings-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
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
            ..Settings::default()
        };
        settings.modules.toggle(ModuleId::Cps);
        settings
            .modules
            .set_anchor(ModuleId::Keystrokes, [0.25, 0.75]);
        settings.save_to(&path).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, settings);
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
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hand_edited_values_are_clamped_on_load() {
        let path = temp_file("clamped");
        std::fs::write(
            &path,
            r#"{"fov": 500.0, "sensitivity": -1.0, "view_distance": 99, "gui_scale": 9,
                "modules": {"fps": {"anchor": [-2.0, 4.0]}}}"#,
        )
        .unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.fov, 110.0);
        assert_eq!(loaded.sensitivity, 0.4);
        assert_eq!(loaded.view_distance, 16);
        assert_eq!(loaded.gui_scale, 4);
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
}
