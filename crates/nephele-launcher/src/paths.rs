//! Where the launcher keeps its own state.
//!
//! The game's per-user directory is `nephele_cli`'s `paths::user_data_dir`
//! (see `AGENTS.md` §5). The launcher is a separate binary with no library
//! dependency on the CLI crate, so this module carries the same rule for the
//! one path it needs — `%APPDATA%\Nephele` — rather than pulling the whole
//! game crate in for twelve lines. Keep the two in step; a shared
//! `nephele-paths` crate is the right long-term home.

use std::path::PathBuf;

/// Name of the per-user directory, used on every platform.
const APP_DIR: &str = "Nephele";

/// The per-user directory for game files, following `paths::user_data_dir`.
///
/// `NEPHELE_DATA_DIR` wins. This value is the *default* game directory for the
/// default profile; each profile stores its own path in `profiles.json`.
pub fn user_data_dir() -> PathBuf {
    resolve_data_dir(
        std::env::var_os("NEPHELE_DATA_DIR").map(PathBuf::from),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_data_dir(
    explicit: Option<PathBuf>,
    appdata: Option<PathBuf>,
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = explicit {
        return dir;
    }
    if let Some(appdata) = appdata {
        return appdata.join(APP_DIR);
    }
    if let Some(data) = xdg {
        return data.join("nephele");
    }
    if let Some(home) = home {
        return home.join(".local/share/nephele");
    }
    PathBuf::from(".nephele")
}

/// Launcher-owned state: profiles, launcher options, update cache.
///
/// `NEPHELE_LAUNCHER_DIR` exists so a test or a portable copy can keep its
/// state away from the real one.
pub fn launcher_dir() -> PathBuf {
    std::env::var_os("NEPHELE_LAUNCHER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_data_dir().join("launcher"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_order_prefers_appdata_then_xdg_then_home() {
        assert_eq!(
            resolve_data_dir(None, Some(PathBuf::from("C:/Users/x/AppData")), None, None),
            PathBuf::from("C:/Users/x/AppData/Nephele")
        );
        assert_eq!(
            resolve_data_dir(
                None,
                None,
                Some(PathBuf::from("/home/x/.local/share")),
                None
            ),
            PathBuf::from("/home/x/.local/share/nephele")
        );
        assert_eq!(
            resolve_data_dir(None, None, None, Some(PathBuf::from("/home/x"))),
            PathBuf::from("/home/x/.local/share/nephele")
        );
    }

    #[test]
    fn an_explicit_directory_wins() {
        assert_eq!(
            resolve_data_dir(
                Some(PathBuf::from("/portable")),
                Some(PathBuf::from("C:/AppData")),
                None,
                None
            ),
            PathBuf::from("/portable")
        );
    }
}
