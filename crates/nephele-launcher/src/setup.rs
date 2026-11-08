//! First-run resource preparation.
//!
//! Nephele ships no game assets. `nephele-prepare` derives them from the
//! player's own 1.8.9 jar, verified against `research-lock.json`. The launcher
//! looks for an existing preparation, and offers a setup screen when there is
//! none — including the output of a failed attempt, because "nothing happened"
//! is the error report players complain about most.
//!
//! The search order mirrors the game (`nephele-cli`'s `minecraft_assets_root`):
//! an explicit `NEPHELE_MINECRAFT_ASSETS`, the checkout's `.local/prepared`,
//! then a per-user preparation under the launcher's data directory. The last
//! one is what an installed build uses, because the install directory may be
//! read-only.

use std::path::{Path, PathBuf};

/// The asset tree the client reads (`.../assets/minecraft`).
pub const ASSET_TAIL: &str = "1.8.9/assets/minecraft";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resources {
    /// `.../assets/minecraft`
    pub assets: PathBuf,
    /// `.../assets/minecraft/textures`, the UI atlas input.
    pub textures: PathBuf,
    /// `.../assets/minecraft/textures/blocks`, the block array input.
    pub blocks: PathBuf,
}

impl Resources {
    pub fn from_assets(assets: PathBuf) -> Self {
        let textures = assets.join("textures");
        let blocks = textures.join("blocks");
        Self {
            assets,
            textures,
            blocks,
        }
    }

    /// Both the font/GUI atlas and the block array must be present. Checking
    /// the directories is cheap and catches a half-copied preparation.
    pub fn ready(&self) -> bool {
        self.textures.is_dir() && self.blocks.is_dir()
    }
}

/// Where a fresh preparation is written for an installed build.
pub fn prepared_out(data_dir: &Path) -> PathBuf {
    data_dir.join("prepared")
}

pub fn assets_under(out: &Path) -> PathBuf {
    out.join(ASSET_TAIL.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Resolve the resources to use. `explicit` is `NEPHELE_MINECRAFT_ASSETS`;
/// `project` is the checkout (or install) root; `data` is the per-user game
/// directory.
///
/// A preparation whose `prepared.json` names a different `outputVersion` is
/// ignored: the file set changed, so the launcher re-runs preparation instead
/// of starting a client that will look for files that are not there.
pub fn discover(explicit: Option<PathBuf>, project: &Path, data: &Path) -> Option<Resources> {
    let explicit_assets = explicit.map(Resources::from_assets);
    let project_out = project.join(".local/prepared");
    let user_out = prepared_out(data);
    let candidates = [
        explicit_assets.map(|resources| (resources, None)),
        Some((
            Resources::from_assets(assets_under(&project_out)),
            Some(project_out),
        )),
        Some((
            Resources::from_assets(assets_under(&user_out)),
            Some(user_out),
        )),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|(resources, out)| resources.ready() && out.as_deref().is_none_or(manifest_current))
        .map(|(resources, _)| resources)
}

/// Whether `out/prepared.json` is absent (a legacy checkout) or matches the
/// preparer this launcher was built against.
pub fn manifest_current(out: &Path) -> bool {
    let path = out
        .join(nephele_prepare::VERSION)
        .join(nephele_prepare::MANIFEST_NAME);
    let Ok(text) = std::fs::read_to_string(path) else {
        return true;
    };
    serde_json::from_str::<nephele_prepare::Prepared>(&text).is_ok_and(|prepared| {
        prepared.output_version == nephele_prepare::OUTPUT_VERSION
            && prepared.minecraft == nephele_prepare::VERSION
    })
}

/// Arguments for `nephele-prepare`. `--out` is always explicit so the launcher
/// never depends on the child's working directory.
pub fn prepare_args(out: &Path, minecraft_dir: Option<&str>) -> Vec<String> {
    let mut args = vec!["--out".to_string(), out.display().to_string()];
    if let Some(dir) = minecraft_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        args.push("--minecraft-dir".to_string());
        args.push(dir.to_string());
    }
    args
}

/// A sensible starting point for the setup screen's path field.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let candidate = appdata.join(".minecraft");
    candidate.is_dir().then_some(candidate)
}

/// How many files preparation has produced so far. Polled at a low rate while
/// the child runs; it is progress feedback, not a correctness check.
pub fn count_files(dir: &Path) -> usize {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            total += count_files(&entry.path());
        } else if kind.is_file() {
            total += 1;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-launcher-setup-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_preparation(root: &Path) -> PathBuf {
        let assets = assets_under(root);
        std::fs::create_dir_all(assets.join("textures/blocks")).unwrap();
        std::fs::create_dir_all(assets.join("textures/font")).unwrap();
        assets
    }

    #[test]
    fn an_explicit_directory_wins_over_the_checkout_and_the_user_copy() {
        let dir = temp_dir("order");
        let explicit = fake_preparation(&dir.join("explicit"));
        fake_preparation(&dir.join("project/.local/prepared"));
        fake_preparation(&dir.join("data/prepared"));
        let found = discover(
            Some(explicit.clone()),
            &dir.join("project"),
            &dir.join("data"),
        )
        .unwrap();
        assert_eq!(found.assets, explicit);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkout_is_preferred_over_the_per_user_copy() {
        let dir = temp_dir("checkout");
        let project = fake_preparation(&dir.join("project/.local/prepared"));
        fake_preparation(&dir.join("data/prepared"));
        let found = discover(None, &dir.join("project"), &dir.join("data")).unwrap();
        assert_eq!(found.assets, project);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_preparation_is_none_not_a_panic() {
        let dir = temp_dir("missing");
        assert_eq!(discover(None, &dir, &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn half_a_preparation_is_not_ready() {
        let dir = temp_dir("half");
        let assets = assets_under(&dir.join("prepared"));
        std::fs::create_dir_all(assets.join("textures")).unwrap();
        assert_eq!(discover(None, &dir.join("project"), &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_output_version_forces_preparation_again() {
        let dir = temp_dir("stale");
        let out = dir.join("project/.local/prepared");
        fake_preparation(&out);
        let manifest = out.join("1.8.9/prepared.json");
        std::fs::write(
            &manifest,
            r#"{
                "minecraft": "1.8.9",
                "outputVersion": 999,
                "jarSha1": "",
                "jarSize": 0,
                "fileCount": 0,
                "digest": "",
                "preparedAtUnix": 0,
                "files": {}
            }"#,
        )
        .unwrap();
        assert!(!manifest_current(&out));
        assert_eq!(discover(None, &dir.join("project"), &dir), None);

        // The version this launcher was built against is accepted.
        std::fs::write(
            &manifest,
            format!(
                r#"{{"minecraft":"{}","outputVersion":{},"jarSha1":"","jarSize":0,"fileCount":0,"digest":"","preparedAtUnix":0,"files":{{}}}}"#,
                nephele_prepare::VERSION,
                nephele_prepare::OUTPUT_VERSION
            ),
        )
        .unwrap();
        assert!(manifest_current(&out));
        assert!(discover(None, &dir.join("project"), &dir).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_arguments_are_explicit_and_optional() {
        let out = PathBuf::from("C:/data/prepared");
        assert_eq!(
            prepare_args(&out, None),
            vec!["--out".to_string(), "C:/data/prepared".to_string()]
        );
        assert_eq!(
            prepare_args(&out, Some("  ")),
            vec!["--out".to_string(), "C:/data/prepared".to_string()]
        );
        assert_eq!(
            prepare_args(&out, Some("C:/Games/.minecraft")),
            vec![
                "--out".to_string(),
                "C:/data/prepared".to_string(),
                "--minecraft-dir".to_string(),
                "C:/Games/.minecraft".to_string()
            ]
        );
    }

    #[test]
    fn counting_files_walks_subdirectories() {
        let dir = temp_dir("count");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("one"), "1").unwrap();
        std::fs::write(dir.join("a/two"), "2").unwrap();
        std::fs::write(dir.join("a/b/three"), "3").unwrap();
        assert_eq!(count_files(&dir), 3);
        assert_eq!(count_files(&dir.join("nope")), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
