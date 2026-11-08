//! `nephele-prepare` - build Nephele's resources from the player's own copy of
//! Minecraft: Java Edition 1.8.9.
//!
//! Nephele contains no Minecraft code, assets or data. Everything the client
//! loads is derived on the player's machine, from the client jar the player
//! already owns, and verified against the checksums pinned in
//! `research-lock.json` before a single byte is extracted.
//!
//! The lock pins the jar itself rather than the version JSON. A launcher may
//! rewrite or republish its metadata, but the jar's hash is the thing that
//! actually decides whether the bytes are the 1.8.9 client, so that is the only
//! checksum this crate needs.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// The Minecraft release Nephele speaks.
pub const VERSION: &str = "1.8.9";

/// Bumped whenever the set or shape of prepared files changes, so a launcher
/// can tell a stale preparation from a current one without rehashing it.
pub const OUTPUT_VERSION: u32 = 2;

/// Name of the checksum pin file, looked up from the working directory upward.
pub const LOCK_FILE_NAME: &str = "research-lock.json";

/// Asset index id for the pinned 1.8.9 assets.
pub const ASSET_INDEX_ID: &str = "1.8";

/// Name of the manifest written next to the prepared files.
pub const MANIFEST_NAME: &str = "prepared.json";

/// The asset subtrees the client reads. 1.8.9 ships most of these inside the
/// client jar; sounds live in the asset object store, which
/// [`prepare_with_assets`] copies when the player has one.
pub const ASSET_PREFIXES: &[&str] = &[
    "minecraft/blockstates/",
    "minecraft/models/",
    "minecraft/textures/blocks/",
    "minecraft/textures/items/",
    "minecraft/textures/entity/",
    "minecraft/textures/gui/",
    "minecraft/textures/font/",
    "minecraft/textures/misc/",
    "minecraft/texts/",
    "minecraft/font/",
    "minecraft/lang/en_US.lang",
    "minecraft/sounds.json",
];

/// One file's published content hash.
#[derive(Debug, Clone, Deserialize)]
pub struct LockFile {
    pub sha1: String,
    pub size: u64,
}

/// The pinned release.
#[derive(Debug, Clone, Deserialize)]
pub struct LockMinecraft {
    pub version: String,
    pub client: LockFile,
    #[serde(rename = "assetIndex")]
    pub asset_index: LockFile,
}

/// The preparer's own output contract.
#[derive(Debug, Clone, Deserialize)]
pub struct LockPreparer {
    #[serde(rename = "outputVersion")]
    pub output_version: u32,
}

/// `research-lock.json` in full.
#[derive(Debug, Clone, Deserialize)]
pub struct Lock {
    pub minecraft: LockMinecraft,
    pub preparer: LockPreparer,
}

/// What `prepare` produced and, on later runs, what is on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub minecraft: String,
    pub output_version: u32,
    pub jar_sha1: String,
    pub jar_size: u64,
    pub file_count: usize,
    pub digest: String,
    pub prepared_at_unix: u64,
    pub files: BTreeMap<String, String>,
}

/// Outcome of `--verify-only`.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub checked: usize,
    pub mismatched: Vec<String>,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.mismatched.is_empty() && self.missing.is_empty() && self.extra.is_empty()
    }

    pub fn problems(&self) -> usize {
        self.mismatched.len() + self.missing.len() + self.extra.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    Message(String),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

fn io_err(path: &Path, source: io::Error) -> PrepareError {
    PrepareError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// SHA-1 of a file, streamed so a jar is never held in memory.
pub fn sha1_file(path: &Path) -> Result<String, PrepareError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let mut reader = io::BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha1::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| io_err(path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

impl Lock {
    pub fn load(path: &Path) -> Result<Lock, PrepareError> {
        let text = fs::read_to_string(path).map_err(|e| io_err(path, e))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Looks for `research-lock.json` in `start` and each of its parents.
    pub fn find(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .map(|dir| dir.join(LOCK_FILE_NAME))
            .find(|candidate| candidate.is_file())
    }
}

/// Where a launcher of each family keeps `1.8.9.jar` under `dir`.
///
/// Covers a standard game directory, a Prism/MultiMC instance (which nests
/// everything under `minecraft/`) and a launcher root holding `.minecraft/`.
pub fn client_jar_candidates(dir: &Path) -> Vec<PathBuf> {
    let name = format!("{VERSION}.jar");
    vec![
        dir.join("versions").join(VERSION).join(&name),
        dir.join("minecraft")
            .join("versions")
            .join(VERSION)
            .join(&name),
        dir.join(".minecraft")
            .join("versions")
            .join(VERSION)
            .join(&name),
    ]
}

fn is_wanted(virtual_path: &str) -> bool {
    ASSET_PREFIXES.iter().any(|prefix| {
        virtual_path.starts_with(prefix)
            || (prefix.ends_with('/') && virtual_path == prefix.trim_end_matches('/'))
    })
}

/// Folds a file map into one digest, so a launcher can compare a preparation
/// across releases without hashing every file.
pub fn digest_of(files: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha1::new();
    for (path, hash) in files {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

/// Copy every `minecraft/sounds/**` object named by the asset index.
///
/// 1.8.9 does not put Ogg files in the client jar, so this is the only source
/// for the game's sound effects. Each object is addressed by its SHA-1 and
/// copied to the prepared tree under its virtual asset path.
fn copy_sound_objects(
    assets_root: &Path,
    assets_dir: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), PrepareError> {
    let index_path = assets_root
        .join("indexes")
        .join(format!("{ASSET_INDEX_ID}.json"));
    if !index_path.is_file() {
        eprintln!(
            "  sounds    skipped: {} does not exist (pass --assets-dir to enable audio)",
            index_path.display()
        );
        return Ok(());
    }
    let text = fs::read_to_string(&index_path).map_err(|e| io_err(&index_path, e))?;
    let index: serde_json::Value = serde_json::from_str(&text)?;
    let Some(objects) = index.get("objects").and_then(|value| value.as_object()) else {
        return Err(PrepareError::Message(format!(
            "{} has no objects map",
            index_path.display()
        )));
    };
    let objects_root = assets_root.join("objects");
    let mut copied = 0usize;
    let mut missing = 0usize;
    for (virtual_path, value) in objects {
        if virtual_path != "minecraft/sounds.json" && !virtual_path.starts_with("minecraft/sounds/")
        {
            continue;
        }
        let Some(hash) = value.get("hash").and_then(|value| value.as_str()) else {
            continue;
        };
        if hash.len() < 2 {
            continue;
        }
        let source = objects_root.join(&hash[..2]).join(hash);
        if !source.is_file() {
            missing += 1;
            continue;
        }
        let target = assets_dir.join(virtual_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        fs::copy(&source, &target).map_err(|e| io_err(&target, e))?;
        files.insert(virtual_path.clone(), sha1_file(&target)?);
        copied += 1;
    }
    println!("  sounds    {copied} object(s) copied");
    if missing > 0 {
        eprintln!(
            "  sounds    {missing} object(s) absent from {}; run the launcher once to populate it",
            objects_root.display()
        );
    }
    Ok(())
}

/// Verifies `jar` against the lock, extracts the asset subtree into
/// `out_dir/assets/`, and writes `out_dir/prepared.json`.
pub fn prepare(jar: &Path, out_dir: &Path, lock: &Lock) -> Result<Prepared, PrepareError> {
    prepare_with_assets(jar, out_dir, lock, None)
}

/// [`prepare`] plus sound objects from an extracted Minecraft `assets` folder
/// (the one containing `indexes/` and `objects/`). A missing or incomplete
/// object store is reported but does not make a texture-only preparation fail.
pub fn prepare_with_assets(
    jar: &Path,
    out_dir: &Path,
    lock: &Lock,
    assets_root: Option<&Path>,
) -> Result<Prepared, PrepareError> {
    if lock.minecraft.version != VERSION {
        return Err(PrepareError::Message(format!(
            "research-lock.json pins Minecraft {}, but this preparer speaks {VERSION}",
            lock.minecraft.version
        )));
    }

    let size = fs::metadata(jar).map_err(|e| io_err(jar, e))?.len();
    let jar_sha1 = sha1_file(jar)?;
    if jar_sha1 != lock.minecraft.client.sha1 || size != lock.minecraft.client.size {
        return Err(PrepareError::Message(format!(
            "{} is not a Minecraft {VERSION} client jar Nephele knows.\n  found    sha1 {jar_sha1} ({size} bytes)\n  expected sha1 {} ({} bytes)\n\
             Download the release again, or point --jar at the right file. Change research-lock.json only after confirming the new jar really is 1.8.9.",
            jar.display(),
            lock.minecraft.client.sha1,
            lock.minecraft.client.size
        )));
    }

    let assets_dir = out_dir.join("assets");
    if assets_dir.exists() {
        fs::remove_dir_all(&assets_dir).map_err(|e| io_err(&assets_dir, e))?;
    }
    fs::create_dir_all(&assets_dir).map_err(|e| io_err(&assets_dir, e))?;

    let mut files = BTreeMap::new();
    let mut archive = zip::ZipArchive::new(File::open(jar).map_err(|e| io_err(jar, e))?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = name.to_string_lossy().replace('\\', "/");
        let Some(virtual_path) = name.strip_prefix("assets/") else {
            continue;
        };
        if !is_wanted(virtual_path) {
            continue;
        }
        let target = assets_dir.join(virtual_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        let mut out = File::create(&target).map_err(|e| io_err(&target, e))?;
        io::copy(&mut entry, &mut out).map_err(|e| io_err(&target, e))?;
        files.insert(virtual_path.to_string(), sha1_file(&target)?);
    }

    if let Some(assets_root) = assets_root {
        copy_sound_objects(assets_root, &assets_dir, &mut files)?;
    }

    if files.is_empty() {
        return Err(PrepareError::Message(format!(
            "extracted nothing from {}: no assets/{} entries matched the filter",
            jar.display(),
            ASSET_PREFIXES[0]
        )));
    }

    let prepared = Prepared {
        minecraft: VERSION.to_string(),
        output_version: lock.preparer.output_version,
        jar_sha1,
        jar_size: size,
        file_count: files.len(),
        digest: digest_of(&files),
        prepared_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        files,
    };

    let manifest_path = out_dir.join(MANIFEST_NAME);
    let text = serde_json::to_string_pretty(&prepared)?;
    fs::write(&manifest_path, text).map_err(|e| io_err(&manifest_path, e))?;
    Ok(prepared)
}

/// Reads `out_dir/prepared.json` and rehashes every file it names.
pub fn verify(out_dir: &Path, lock: &Lock) -> Result<(Prepared, VerifyReport), PrepareError> {
    let manifest_path = out_dir.join(MANIFEST_NAME);
    let text = fs::read_to_string(&manifest_path).map_err(|e| io_err(&manifest_path, e))?;
    let prepared: Prepared = serde_json::from_str(&text)?;
    let mut report = VerifyReport::default();

    if prepared.output_version != lock.preparer.output_version {
        return Err(PrepareError::Message(format!(
            "prepared for output version {}, but this preparer writes {}. Prepare again.",
            prepared.output_version, lock.preparer.output_version
        )));
    }

    let assets_dir = out_dir.join("assets");
    for (path, expected) in &prepared.files {
        let full = assets_dir.join(path);
        if !full.is_file() {
            report.missing.push(path.clone());
            continue;
        }
        report.checked += 1;
        if &sha1_file(&full)? != expected {
            report.mismatched.push(path.clone());
        }
    }

    if let Ok(walk) = fs::read_dir(&assets_dir) {
        let mut known: BTreeMap<&str, ()> =
            prepared.files.keys().map(|k| (k.as_str(), ())).collect();
        let mut stack: Vec<PathBuf> = walk.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        while let Some(dir) = stack.pop() {
            if dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&dir) {
                    stack.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
                }
                continue;
            }
            let Ok(relative) = dir.strip_prefix(&assets_dir) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if known.remove(relative.as_str()).is_none() {
                report.extra.push(relative);
            }
        }
    }

    if report.ok() && digest_of(&prepared.files) != prepared.digest {
        return Err(PrepareError::Message(
            "prepared.json's digest does not match its own file list; it was edited by hand"
                .to_string(),
        ));
    }
    Ok((prepared, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_json() -> &'static str {
        r#"{
            "minecraft": {
                "version": "1.8.9",
                "client": { "sha1": "3870888a6c3d349d3771a3e9d16c9bf5e076b908", "size": 8461484 },
                "assetIndex": { "id": "1.8", "sha1": "f6ad102bcaa53b1a58358f16e376d548d44933ec", "size": 78494 }
            },
            "preparer": { "outputVersion": 2 }
        }"#
    }

    fn lock() -> Lock {
        serde_json::from_str(lock_json()).expect("lock parses")
    }

    #[test]
    fn lock_parses_the_pinned_release() {
        let lock = lock();
        assert_eq!(lock.minecraft.version, VERSION);
        assert_eq!(lock.minecraft.client.size, 8_461_484);
        assert_eq!(lock.preparer.output_version, OUTPUT_VERSION);
    }

    #[test]
    fn asset_filter_takes_only_what_the_client_reads() {
        assert!(is_wanted("minecraft/textures/blocks/stone.png"));
        assert!(is_wanted("minecraft/textures/gui/widgets.png"));
        assert!(is_wanted("minecraft/lang/en_US.lang"));
        assert!(is_wanted("minecraft/textures/entity/zombie/zombie.png"));
        assert!(is_wanted("minecraft/texts/splashes.txt"));
        assert!(is_wanted("minecraft/sounds.json"));
        // Sound objects live in the asset index, not the jar; copy_sound_objects
        // handles them separately.
        assert!(!is_wanted("minecraft/sounds/random/click.ogg"));
        assert!(!is_wanted("minecraft/lang/de_DE.lang"));
        assert!(!is_wanted("minecraft/icon/icon_16x16.png"));
    }

    #[test]
    fn filter_does_not_match_a_prefix_by_accident() {
        assert!(!is_wanted("minecraft/fontx/unifont.ttf"));
        assert!(!is_wanted("minecraft/textures/gui2/widgets.png"));
    }

    #[test]
    fn digest_is_order_independent_but_content_sensitive() {
        let mut a = BTreeMap::new();
        a.insert("b".to_string(), "2".to_string());
        a.insert("a".to_string(), "1".to_string());
        let mut b = BTreeMap::new();
        b.insert("a".to_string(), "1".to_string());
        b.insert("b".to_string(), "2".to_string());
        assert_eq!(digest_of(&a), digest_of(&b));

        b.insert("b".to_string(), "3".to_string());
        assert_ne!(digest_of(&a), digest_of(&b));
    }

    #[test]
    fn digest_is_stable() {
        let mut files = BTreeMap::new();
        files.insert(
            "minecraft/textures/blocks/stone.png".to_string(),
            "deadbeef".to_string(),
        );
        assert_eq!(digest_of(&files), digest_of(&files));
        assert_eq!(digest_of(&files).len(), 40);
    }

    #[test]
    fn jar_candidates_cover_launcher_layouts() {
        let dir = Path::new("C:/game");
        let names: Vec<String> = client_jar_candidates(dir)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(names.contains(&"C:/game/versions/1.8.9/1.8.9.jar".to_string()));
        assert!(names.contains(&"C:/game/minecraft/versions/1.8.9/1.8.9.jar".to_string()));
        assert!(names.contains(&"C:/game/.minecraft/versions/1.8.9/1.8.9.jar".to_string()));
    }

    #[test]
    fn a_wrong_jar_is_refused_before_anything_is_written() {
        let dir = std::env::temp_dir().join("nephele-prepare-wrong-jar");
        let _ = fs::create_dir_all(&dir);
        let jar = dir.join("not-minecraft.bin");
        fs::write(&jar, b"definitely not a jar").unwrap();
        let out = dir.join("out");
        let err = prepare(&jar, &out, &lock()).expect_err("must refuse");
        let text = err.to_string();
        assert!(
            text.contains("is not a Minecraft 1.8.9 client jar"),
            "{text}"
        );
        assert!(!out.join("assets").exists(), "no assets written on refusal");
        let _ = fs::remove_dir_all(&dir);
    }
}
