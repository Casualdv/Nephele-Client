//! Where Nephele keeps the files that belong to the user rather than to this
//! checkout: the saved Microsoft session and the server list.
//!
//! A game directory is often read-only, and `.local/` inside it is a
//! per-install location — two installs of the same client would disagree about
//! who you are. Both files therefore live under one per-user directory, with a
//! one-time copy out of `.local/` for people who ran an older build.

use std::path::{Path, PathBuf};

/// Name of the per-user directory, used on every platform.
const APP_DIR: &str = "Nephele";

/// The directory for user files.
///
/// `NEPHELE_DATA_DIR` wins, which is what tests and portable installs use.
/// Otherwise this follows the platform's convention, checked in the order that
/// matters on Windows first.
pub fn user_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NEPHELE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(APP_DIR);
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("nephele");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/nephele");
    }
    // Nothing to go on: the current directory is still a safe place to write.
    PathBuf::from(".nephele")
}

/// Copy `legacy` into `new` the first time the new path is absent.
///
/// Writes only ever go to the user-data path; this exists so a session or a
/// server list made by an older build is not silently abandoned. Returns
/// whether a copy happened.
pub fn migrate_legacy_file(new_path: &Path, legacy_path: &Path) -> std::io::Result<bool> {
    if new_path.exists() || !legacy_path.exists() {
        return Ok(false);
    }
    if let Some(parent) = new_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(legacy_path, new_path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-paths-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_data_directory_is_named_after_the_client() {
        // Whatever the platform, the last component is stable; only the parent
        // changes.
        let dir = user_data_dir();
        let name = dir.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(name == "nephele" || name == ".nephele", "{name}");
    }

    #[test]
    fn a_legacy_file_is_copied_once() {
        let dir = temp_dir("migrate");
        let legacy = dir.join(".local/auth.json");
        let new = dir.join("user/auth.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{\"token\":1}").unwrap();

        assert!(migrate_legacy_file(&new, &legacy).unwrap());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "{\"token\":1}");

        // A second run must not clobber what the new location now holds.
        std::fs::write(&new, "{\"token\":2}").unwrap();
        assert!(!migrate_legacy_file(&new, &legacy).unwrap());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "{\"token\":2}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_legacy_file_is_not_an_error() {
        let dir = temp_dir("missing-legacy");
        let legacy = dir.join(".local/auth.json");
        let new = dir.join("user/auth.json");
        assert!(!migrate_legacy_file(&new, &legacy).unwrap());
        assert!(!new.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
