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
///
/// 2: sound files and `sounds.json` are unpacked from the asset index.
pub const OUTPUT_VERSION: u32 = 2;

/// Name of the checksum pin file, looked up from the working directory upward.
pub const LOCK_FILE_NAME: &str = "research-lock.json";

/// Name of the manifest written next to the prepared files.
pub const MANIFEST_NAME: &str = "prepared.json";

/// The asset subtrees the client reads out of the client jar.
///
/// 1.8.9 ships textures, models and language files inside the jar, but every
/// sound file - and `sounds.json` itself - lives in the *asset object store*
/// instead, addressed by content hash. Those are copied in separately, from
/// the player's own asset index; see [`SoundSource`] and
/// [`prepare_with_sounds`]. A jar-only preparation still succeeds, it just has
/// no audio.
pub const ASSET_PREFIXES: &[&str] = &[
    "minecraft/blockstates/",
    "minecraft/models/",
    "minecraft/textures/blocks/",
    "minecraft/textures/items/",
    "minecraft/textures/gui/",
    "minecraft/textures/font/",
    "minecraft/textures/misc/",
    "minecraft/font/",
    "minecraft/lang/en_US.lang",
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

/// The player's asset object store, plus the 1.8 asset index that names it.
///
/// Sounds are the only 1.8 assets that are not inside the client jar: they are
/// stored once, addressed by the SHA-1 of their contents, and the index maps
/// `minecraft/sounds/dig/stone1.ogg` onto one of those hashes. The index is
/// pinned in `research-lock.json` alongside the jar, so a preparation can
/// refuse an index that is not the 1.8 one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundSource {
    pub index: PathBuf,
    pub objects: PathBuf,
}

impl SoundSource {
    /// Finds `assets/indexes/1.8.json` and `assets/objects` inside a Minecraft
    /// game directory, a launcher root, or the `assets` folder itself.
    pub fn from_minecraft_dir(dir: &Path) -> Option<Self> {
        let candidates = [
            dir.join("assets"),
            dir.join(".minecraft").join("assets"),
            dir.join("minecraft").join("assets"),
            dir.to_path_buf(),
        ];
        for assets in candidates {
            let index = assets.join("indexes").join("1.8.json");
            let objects = assets.join("objects");
            if index.is_file() && objects.is_dir() {
                return Some(Self { index, objects });
            }
        }
        None
    }
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: BTreeMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    #[serde(default)]
    size: u64,
}

/// Copies `sounds.json` and every sound file the index names into
/// `assets/minecraft/`, checking each object against its content hash.
fn extract_sounds(
    assets_dir: &Path,
    source: &SoundSource,
    expected_index_sha1: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<usize, PrepareError> {
    let index_sha1 = sha1_file(&source.index)?;
    if index_sha1 != expected_index_sha1 {
        return Err(PrepareError::Message(format!(
            "{} is not the Minecraft {VERSION} asset index Nephele knows.\n  found    sha1 {index_sha1}\n  expected sha1 {expected_index_sha1}",
            source.index.display()
        )));
    }

    let text = fs::read_to_string(&source.index).map_err(|e| io_err(&source.index, e))?;
    let index: AssetIndex = serde_json::from_str(&text)?;
    let mut copied = 0;

    for (name, object) in &index.objects {
        let Some(relative) = name.strip_prefix("minecraft/") else {
            continue;
        };
        if relative != "sounds.json" && !relative.starts_with("sounds/") {
            continue;
        }
        if object.hash.len() < 2 {
            return Err(PrepareError::Message(format!(
                "the asset index gives {name} a malformed hash {:?}",
                object.hash
            )));
        }

        let origin = source.objects.join(&object.hash[..2]).join(&object.hash);
        if !origin.is_file() {
            return Err(PrepareError::Message(format!(
                "the asset object for {name} is missing from {}\n\
                 Start Minecraft {VERSION} once so your launcher downloads it, then run this again.",
                source.objects.display()
            )));
        }
        // Content-addressed: the file name *is* the hash, so this also catches a
        // store that has been tampered with.
        let digest = sha1_file(&origin)?;
        if digest != object.hash {
            return Err(PrepareError::Message(format!(
                "asset object for {name} is corrupt: expected sha1 {}, found {digest}",
                object.hash
            )));
        }
        if object.size > 0 {
            let size = fs::metadata(&origin).map_err(|e| io_err(&origin, e))?.len();
            if size != object.size {
                return Err(PrepareError::Message(format!(
                    "asset object for {name} is {size} bytes, the index says {}",
                    object.size
                )));
            }
        }

        let target = assets_dir.join("minecraft").join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        fs::copy(&origin, &target).map_err(|e| io_err(&target, e))?;
        files.insert(format!("minecraft/{relative}"), digest);
        copied += 1;
    }

    Ok(copied)
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

/// Verifies `jar` against the lock, extracts the asset subtree into
/// `out_dir/assets/`, and writes `out_dir/prepared.json`.
///
/// Sounds are left out: they are not in the jar. Use
/// [`prepare_with_sounds`] to unpack them from the player's asset store too.
pub fn prepare(jar: &Path, out_dir: &Path, lock: &Lock) -> Result<Prepared, PrepareError> {
    prepare_with_sounds(jar, out_dir, lock, None)
}

/// [`prepare`], plus every sound file from `sounds` when one is given.
///
/// The sounds are additive: a caller that cannot find the player's asset store
/// still gets a usable jar-only preparation, just a silent one.
pub fn prepare_with_sounds(
    jar: &Path,
    out_dir: &Path,
    lock: &Lock,
    sounds: Option<&SoundSource>,
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

    if files.is_empty() {
        return Err(PrepareError::Message(format!(
            "extracted nothing from {}: no assets/{} entries matched the filter",
            jar.display(),
            ASSET_PREFIXES[0]
        )));
    }

    if let Some(source) = sounds {
        let copied = extract_sounds(
            &assets_dir,
            source,
            &lock.minecraft.asset_index.sha1,
            &mut files,
        )?;
        if copied == 0 {
            return Err(PrepareError::Message(format!(
                "{} lists no sound files; is it really the {VERSION} asset index?",
                source.index.display()
            )));
        }
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
        // Sounds are not in the jar at all - they come from the asset index -
        // so the jar filter must not pretend to want them.
        assert!(!is_wanted("minecraft/sounds.json"));
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
    fn sounds_come_out_of_the_asset_store_by_hash() {
        let dir = std::env::temp_dir().join("nephele-prepare-sounds");
        let _ = fs::remove_dir_all(&dir);
        let objects = dir.join("objects");
        let out = dir.join("out");

        let bytes = b"OggS this is not a real vorbis stream";
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        let hash = hex::encode(hasher.finalize());
        let object = objects.join(&hash[..2]).join(&hash);
        fs::create_dir_all(object.parent().unwrap()).unwrap();
        fs::write(&object, bytes).unwrap();

        let index_path = dir.join("1.8.json");
        fs::write(
            &index_path,
            format!(
                r#"{{"objects":{{"minecraft/sounds/dig/stone1.ogg":{{"hash":"{hash}","size":{}}}}}}}"#,
                bytes.len()
            ),
        )
        .unwrap();
        let index_sha1 = sha1_file(&index_path).unwrap();
        let source = SoundSource {
            index: index_path,
            objects,
        };

        let mut files = BTreeMap::new();
        let copied = extract_sounds(&out, &source, &index_sha1, &mut files).unwrap();
        assert_eq!(copied, 1);
        assert!(out.join("minecraft/sounds/dig/stone1.ogg").is_file());
        assert_eq!(
            files
                .get("minecraft/sounds/dig/stone1.ogg")
                .map(String::as_str),
            Some(hash.as_str())
        );

        // A file that no longer matches its hash is refused rather than copied.
        fs::write(&object, b"tampered").unwrap();
        let mut files = BTreeMap::new();
        let error =
            extract_sounds(&out, &source, &index_sha1, &mut files).expect_err("must refuse");
        assert!(error.to_string().contains("corrupt"), "{error}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_asset_index_is_refused() {
        let dir = std::env::temp_dir().join("nephele-prepare-wrong-index");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let index_path = dir.join("1.8.json");
        fs::write(&index_path, r#"{"objects":{}}"#).unwrap();
        let source = SoundSource {
            index: index_path,
            objects: dir.join("objects"),
        };
        let error = extract_sounds(
            &dir.join("out"),
            &source,
            "not-the-pinned-hash",
            &mut BTreeMap::new(),
        )
        .expect_err("must refuse");
        assert!(
            error.to_string().contains("is not the Minecraft"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_asset_store_is_found_under_the_usual_layouts() {
        let dir = std::env::temp_dir().join("nephele-prepare-asset-layout");
        let _ = fs::remove_dir_all(&dir);
        let assets = dir.join(".minecraft").join("assets");
        fs::create_dir_all(assets.join("objects")).unwrap();
        fs::write(assets.join("indexes").join("1.8.json"), "{}").ok();
        fs::create_dir_all(assets.join("indexes")).unwrap();
        fs::write(assets.join("indexes").join("1.8.json"), "{}").unwrap();

        let found = SoundSource::from_minecraft_dir(&dir).expect("found under .minecraft");
        assert_eq!(found.objects, assets.join("objects"));
        let direct =
            SoundSource::from_minecraft_dir(&assets).expect("found when pointed at assets");
        assert_eq!(direct.index, assets.join("indexes").join("1.8.json"));

        let _ = fs::remove_dir_all(&dir);
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
