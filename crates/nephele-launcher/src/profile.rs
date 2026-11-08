//! Launcher profiles.
//!
//! A profile is the pair the task actually needs: **a game directory** (the
//! `NEPHELE_DATA_DIR` handed to the child, holding saves, the auth session,
//! servers and settings) and **a window size** for the game. The launcher's
//! own state lives elsewhere (see [`crate::paths::launcher_dir`]).
//!
//! Profiles persist as JSON. The default profile deliberately points at the
//! existing `%APPDATA%\Nephele` directory so an upgrade does not move anybody's
//! saves or sign them out; additional profiles get their own subdirectory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MIN_WINDOW: (u32, u32) = (640, 480);
pub const MAX_WINDOW: (u32, u32) = (7680, 4320);

/// Window sizes offered by the launcher's cycle button, in order.
pub const WINDOW_PRESETS: [(u32, u32); 4] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

impl WindowSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Clamp to the range a desktop window can plausibly use. A profile file
    /// is user-editable, so nothing downstream may assume it is sane.
    pub fn sanitized(self) -> Self {
        Self {
            width: self.width.clamp(MIN_WINDOW.0, MAX_WINDOW.0),
            height: self.height.clamp(MIN_WINDOW.1, MAX_WINDOW.1),
        }
    }

    pub fn label(self) -> String {
        format!("{} x {}", self.width, self.height)
    }

    /// The next preset after this size, wrapping around.
    pub fn next(self) -> Self {
        let current = WINDOW_PRESETS
            .iter()
            .position(|&(w, h)| w == self.width && h == self.height);
        let next = match current {
            Some(index) => (index + 1) % WINDOW_PRESETS.len(),
            // A hand-edited size that is not a preset starts at the first one.
            None => 0,
        };
        let (width, height) = WINDOW_PRESETS[next];
        Self::new(width, height)
    }

    /// `WxH`, the form the game reads from `NEPHELE_WINDOW_SIZE`.
    pub fn env_value(self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub game_dir: PathBuf,
    #[serde(default)]
    pub window: WindowSize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profiles {
    #[serde(default)]
    active: usize,
    #[serde(default)]
    profiles: Vec<Profile>,
}

impl Profiles {
    /// One profile pointing at the existing per-user game directory.
    pub fn default_at(base: &Path) -> Self {
        Self {
            active: 0,
            profiles: vec![Profile {
                name: "Default".to_string(),
                game_dir: base.to_path_buf(),
                window: WindowSize::default(),
            }],
        }
    }

    /// Read `path`, or start from `default_at(default_game_dir)` when it is
    /// missing or unreadable. A corrupt file must never prevent the launcher
    /// from starting — the worst case is one fresh default profile.
    pub fn load(path: &Path, default_game_dir: &Path) -> Self {
        let mut profiles = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_else(|| Self::default_at(default_game_dir));
        profiles.repair(default_game_dir);
        profiles
    }

    /// Make a hand-edited or upgraded file safe to use.
    fn repair(&mut self, default_game_dir: &Path) {
        if self.profiles.is_empty() {
            self.profiles
                .push(Self::fresh_profile("Default", default_game_dir));
        }
        if self.active >= self.profiles.len() {
            self.active = self.profiles.len() - 1;
        }
        for profile in &mut self.profiles {
            if profile.name.trim().is_empty() {
                profile.name = "Profile".to_string();
            }
            if profile.game_dir.as_os_str().is_empty() {
                profile.game_dir = default_game_dir.to_path_buf();
            }
            profile.window = profile.window.sanitized();
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text)?;
        fs::rename(temporary, path)
    }

    pub fn all(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Profile {
        &self.profiles[self.active.min(self.profiles.len() - 1)]
    }

    pub fn active_mut(&mut self) -> &mut Profile {
        let index = self.active.min(self.profiles.len() - 1);
        &mut self.profiles[index]
    }

    /// Move the selection by one, wrapping. Returns whether it moved.
    pub fn cycle(&mut self, direction: i32) -> bool {
        if self.profiles.len() < 2 {
            return false;
        }
        let len = self.profiles.len() as i32;
        self.active = (self.active as i32 + direction).rem_euclid(len) as usize;
        true
    }

    /// Add a profile with a unique name and its own directory under `base`.
    pub fn add(&mut self, base: &Path) {
        let mut number = self.profiles.len() + 1;
        let name = loop {
            let candidate = format!("Profile {number}");
            if !self
                .profiles
                .iter()
                .any(|profile| profile.name == candidate)
            {
                break candidate;
            }
            number += 1;
        };
        let mut profile = Self::fresh_profile(&name, base);
        profile.game_dir = base.join("profiles").join(slug(&name));
        self.profiles.push(profile);
        self.active = self.profiles.len() - 1;
    }

    /// Duplicate the active profile (same window, new directory) and select it.
    pub fn duplicate_active(&mut self, base: &Path) {
        let source = self.active().clone();
        self.add(base);
        let index = self.active;
        self.profiles[index].window = source.window;
        self.profiles[index].name = format!("{} copy", source.name);
        self.profiles[index].game_dir =
            base.join("profiles").join(slug(&self.profiles[index].name));
        // `add` already picked a unique directory for the generated name; the
        // rename above can collide with it, so make this one unique too.
        let mut suffix = 2;
        while self
            .profiles
            .iter()
            .enumerate()
            .any(|(i, profile)| i != index && profile.game_dir == self.profiles[index].game_dir)
        {
            self.profiles[index].name = format!("{} copy {suffix}", source.name);
            self.profiles[index].game_dir =
                base.join("profiles").join(slug(&self.profiles[index].name));
            suffix += 1;
        }
    }

    /// Remove the active profile. The last profile is never removed.
    pub fn remove_active(&mut self) -> bool {
        if self.profiles.len() < 2 {
            return false;
        }
        self.profiles.remove(self.active);
        if self.active >= self.profiles.len() {
            self.active = self.profiles.len() - 1;
        }
        true
    }

    fn fresh_profile(name: &str, game_dir: &Path) -> Profile {
        Profile {
            name: name.to_string(),
            game_dir: game_dir.to_path_buf(),
            window: WindowSize::default(),
        }
    }
}

/// A filesystem-safe directory name for a profile.
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-launcher-profile-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_default_profile_keeps_the_existing_game_directory() {
        let base = PathBuf::from("C:/Users/x/AppData/Roaming/Nephele");
        let profiles = Profiles::default_at(&base);
        assert_eq!(profiles.active().game_dir, base);
        assert_eq!(profiles.active().window, WindowSize::new(1280, 720));
    }

    #[test]
    fn window_sizes_are_clamped_and_cycle() {
        assert_eq!(
            WindowSize::new(1, 99999).sanitized(),
            WindowSize::new(640, 4320)
        );
        let mut size = WindowSize::default();
        for expected in [(1600, 900), (1920, 1080), (2560, 1440), (1280, 720)] {
            size = size.next();
            assert_eq!(size, WindowSize::new(expected.0, expected.1));
        }
        // A hand-edited size starts the cycle from the first preset.
        assert_eq!(
            WindowSize::new(1000, 700).next(),
            WindowSize::new(1280, 720)
        );
    }

    #[test]
    fn the_last_profile_cannot_be_removed() {
        let base = PathBuf::from("C:/Nephele");
        let mut profiles = Profiles::default_at(&base);
        assert!(!profiles.remove_active());
        profiles.add(&base);
        profiles.add(&base);
        assert_eq!(profiles.all().len(), 3);
        assert!(profiles.remove_active());
        assert_eq!(profiles.all().len(), 2);
    }

    #[test]
    fn added_profiles_get_unique_names_and_directories() {
        let base = PathBuf::from("C:/Nephele");
        let mut profiles = Profiles::default_at(&base);
        profiles.add(&base);
        profiles.add(&base);
        let names: Vec<&str> = profiles.all().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Default", "Profile 2", "Profile 3"]);
        let dirs: Vec<&Path> = profiles
            .all()
            .iter()
            .map(|p| p.game_dir.as_path())
            .collect();
        assert!(dirs[0].ends_with("Nephele"));
        assert!(dirs[1].ends_with("profiles/profile-2"));
        assert!(dirs[2].ends_with("profiles/profile-3"));
    }

    #[test]
    fn a_profile_round_trips_through_json() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("profiles.json");
        let base = dir.join("game");
        let mut profiles = Profiles::default_at(&base);
        profiles.add(&base);
        profiles.active_mut().window = WindowSize::new(1920, 1080);
        profiles.save(&path).unwrap();

        let loaded = Profiles::load(&path, &base);
        assert_eq!(loaded.all().len(), 2);
        assert_eq!(loaded.active().window, WindowSize::new(1920, 1080));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_becomes_one_default_profile() {
        let dir = temp_dir("corrupt");
        let path = dir.join("profiles.json");
        fs::write(&path, "{ not json").unwrap();
        let base = dir.join("game");
        let loaded = Profiles::load(&path, &base);
        assert_eq!(loaded.all().len(), 1);
        assert_eq!(loaded.active().game_dir, base);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_active_index_is_repaired() {
        let dir = temp_dir("empty-list");
        let path = dir.join("profiles.json");
        fs::write(&path, r#"{"active": 9, "profiles": []}"#).unwrap();
        let base = dir.join("game");
        let loaded = Profiles::load(&path, &base);
        assert_eq!(loaded.active_index(), 0);
        assert_eq!(loaded.all().len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_game_reads_a_plain_environment_value() {
        assert_eq!(WindowSize::new(1600, 900).env_value(), "1600x900");
    }
}
