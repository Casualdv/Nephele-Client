use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Preview,
}

impl Channel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stable => "STABLE",
            Self::Preview => "PREVIEW",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub channel: Channel,
    #[serde(default)]
    pub version: Option<String>,
}

impl Profile {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        username: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            username: username.into(),
            online: false,
            channel: Channel::Stable,
            version: None,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 32 {
            return Err("Profile name must be between 1 and 32 characters.");
        }
        let username = self.username.trim();
        if username.is_empty() || username.len() > 16 {
            return Err("Player name must be between 1 and 16 characters.");
        }
        if !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("Player name may contain only letters, numbers and underscores.");
        }
        if self.id.is_empty()
            || self.id.len() > 48
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("Profile id is invalid.");
        }
        if self
            .version
            .as_deref()
            .is_some_and(|version| !valid_version_id(version))
        {
            return Err("Selected version is invalid.");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub profiles: Vec<Profile>,
    pub selected_profile: String,
    pub minecraft_dir: Option<PathBuf>,
    pub minecraft_jar: Option<PathBuf>,
    /// Explicit consent is off by default. Upload requires a second action on
    /// the crash screen even after this setting has been enabled.
    pub crash_upload_opt_in: bool,
    pub auto_update: bool,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        let profile = Profile::new("default", "Default", "Player");
        Self {
            selected_profile: profile.id.clone(),
            profiles: vec![profile],
            minecraft_dir: None,
            minecraft_jar: None,
            crash_upload_opt_in: false,
            auto_update: true,
        }
    }
}

impl LauncherConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("Could not read launcher settings: {err}"))?;
        let mut config: Self = serde_json::from_str(&text)
            .map_err(|err| format!("Could not read launcher settings: {err}"))?;
        config.repair();
        Ok(config)
    }

    fn repair(&mut self) {
        self.profiles.retain(|profile| profile.validate().is_ok());
        self.profiles.truncate(4);
        if self.profiles.is_empty() {
            *self = Self::default();
            return;
        }
        if !self
            .profiles
            .iter()
            .any(|profile| profile.id == self.selected_profile)
        {
            self.selected_profile = self.profiles[0].id.clone();
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Could not create launcher settings folder: {err}"))?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|err| format!("Could not encode launcher settings: {err}"))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)
            .map_err(|err| format!("Could not save launcher settings: {err}"))?;
        std::fs::rename(&temp, path)
            .map_err(|err| format!("Could not replace launcher settings: {err}"))
    }

    pub fn selected_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|profile| profile.id == self.selected_profile)
            .unwrap_or(&self.profiles[0])
    }

    pub fn selected_profile_mut(&mut self) -> &mut Profile {
        let index = self
            .profiles
            .iter()
            .position(|profile| profile.id == self.selected_profile)
            .unwrap_or(0);
        &mut self.profiles[index]
    }

    pub fn add_profile(&mut self) -> &mut Profile {
        if self.profiles.len() >= 4 {
            return self.selected_profile_mut();
        }
        let id = self
            .profiles
            .iter()
            .filter_map(|profile| profile.id.strip_prefix("profile-"))
            .filter_map(|number| number.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let profile = Profile::new(format!("profile-{id}"), format!("Profile {id}"), "Player");
        self.profiles.push(profile);
        self.selected_profile = format!("profile-{id}");
        self.selected_profile_mut()
    }

    pub fn remove_selected_profile(&mut self) {
        if self.profiles.len() <= 1 {
            return;
        }
        let removed = self.selected_profile.clone();
        self.profiles.retain(|profile| profile.id != removed);
        self.selected_profile = self.profiles[0].id.clone();
    }
}

pub fn valid_version_id(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version != "."
        && version != ".."
        && version.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_and_offline_usernames_are_validated() {
        let profile = Profile::new("default", "Main", "Dariu_123");
        assert_eq!(profile.validate(), Ok(()));
        assert!(Profile::new("bad", " ", "Player").validate().is_err());
        assert!(Profile::new("bad", "Main", "two words").validate().is_err());
        assert!(Profile::new("bad", "Main", "01234567890123456")
            .validate()
            .is_err());
    }

    #[test]
    fn config_round_trips_and_defaults_to_no_crash_upload() {
        let path = std::env::temp_dir().join(format!(
            "nephele-launcher-config-{}.json",
            std::process::id()
        ));
        let mut config = LauncherConfig::default();
        config.selected_profile_mut().username = "Tester".to_string();
        config.add_profile().name = "Preview".to_string();
        config.selected_profile_mut().channel = Channel::Preview;
        config.selected_profile_mut().online = true;
        config.save(&path).unwrap();
        config.selected_profile_mut().name = "Preview profile".to_string();
        config.save(&path).unwrap();
        let loaded = LauncherConfig::load(&path).unwrap();
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(loaded.selected_profile().name, "Preview profile");
        assert_eq!(loaded.selected_profile().channel, Channel::Preview);
        assert!(loaded.selected_profile().online);
        assert!(!loaded.crash_upload_opt_in);
        assert!(loaded.auto_update);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn corrupt_or_unsafe_profile_entries_are_repaired() {
        let path = std::env::temp_dir().join(format!(
            "nephele-launcher-corrupt-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"profiles":[{"id":"../bad","name":"Bad","username":"Bad"}],"selected_profile":"missing"}"#,
        )
        .unwrap();
        let config = LauncherConfig::load(&path).unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(
            config.selected_profile(),
            &Profile::new("default", "Default", "Player")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn version_ids_cannot_escape_the_install_directory() {
        assert!(valid_version_id("0.2.0-preview.1"));
        for version in [
            "",
            ".",
            "..",
            "../outside",
            "a/b",
            "a\\b",
            "x".repeat(65).as_str(),
        ] {
            assert!(!valid_version_id(version), "accepted {version:?}");
        }
    }
}
