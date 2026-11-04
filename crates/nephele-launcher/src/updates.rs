use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::profiles::Channel;

const RELEASES_API: &str =
    "https://api.github.com/repos/Casualdv/Nephele-Client/releases?per_page=30";
const MANIFEST_NAME: &str = "update-manifest.json";
const SIGNATURE_NAME: &str = "update-manifest.sig";
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const PINNED_UPDATE_PUBLIC_MODULUS_HEX: &str = "a3bcf92c5ce0e6b57b234bbf5cacaa665c0108f9fa11a2602175a31bbb26b468105325dc045c681ddc9496755dc1fb8c3e1361ae6b1e2b1479431bf57754e99ef2442634a85dacd6303e3d58db4199d61f4f2c729ab87eb5a767d6cd4881a095281a4e7b04856776352324450975d2eb471cd5a2cd7e1fb0e1f5378b6f9be969d43123def9261176a3e9c4371fdcadd79d987dc61354eaf30c98d59902bd56c8425f531d3a255d22aa12b8e229f219ccc0e96e023169a469cb5952f5de028186cc6fca8b47b773b3325ec52ef929023818fc8064423809322da8652b7568f73621b2f4ede6ae05d6ab7ffce9514113d84e23474a2528f79df1d804ba16bc2a91c5f1923f1f4af5aea37ee29fce81c2be9dae7f61cab7ab62fcd2a0b518c5783366ac3ff263aaf2a81489a1abe2c8c51f132022bf6719e5b745132292e9ed362c7a796855d298d9c44436da1c66cc32c9f9ad612ef86cf260d49de0268f8614834f24b9a49772910fb63362774c66a03f67ee5bcd24b1baed2b8a0718335f31f3";
const REQUIRED_FILES: [&str; 4] = [
    "nephele.exe",
    "nephele-launcher.exe",
    "nephele-prepare.exe",
    "research-lock.json",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewsItem {
    pub title: String,
    pub body: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub tag: String,
    pub version: String,
    pub channel: Channel,
    pub released_at: String,
    pub notes: String,
    #[serde(default)]
    pub news: Vec<NewsItem>,
    pub platforms: BTreeMap<String, BTreeMap<String, DownloadAsset>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadAsset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct VerifiedRelease {
    pub manifest: ReleaseManifest,
    pub signed_manifest: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(message) | Self::Invalid(message) | Self::Io(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for UpdateError {}

pub const fn platform_id() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else {
        "unsupported"
    }
}

pub fn check_latest(channel: Channel) -> Result<VerifiedRelease, UpdateError> {
    let agent = format!("NepheleLauncher/{}", env!("CARGO_PKG_VERSION"));
    let response = ureq::get(RELEASES_API)
        .set("User-Agent", &agent)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| UpdateError::Network(format!("Could not check releases: {err}")))?;
    let releases: Vec<GitHubRelease> = response.into_json().map_err(|err| {
        UpdateError::Invalid(format!("GitHub returned an unreadable release list: {err}"))
    })?;
    let release = releases
        .iter()
        .find(|release| !release.draft && release.prerelease == (channel == Channel::Preview))
        .ok_or_else(|| {
            UpdateError::Network(format!(
                "No {} release has been published yet",
                channel.label()
            ))
        })?;
    let manifest_url = release_asset_url(release, MANIFEST_NAME)?;
    let signature_url = release_asset_url(release, SIGNATURE_NAME)?;
    let manifest_bytes = get_limited(&manifest_url, MAX_MANIFEST_BYTES)?;
    let signature = get_limited(&signature_url, MAX_SIGNATURE_BYTES)?;
    verify_signature(&manifest_bytes, &signature)?;

    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes).map_err(|err| {
        UpdateError::Invalid(format!("Signed update manifest is invalid JSON: {err}"))
    })?;
    validate_manifest(&manifest, &release.tag_name, channel)?;

    Ok(VerifiedRelease {
        manifest,
        signed_manifest: manifest_bytes,
        signature,
    })
}

pub fn save_cached_release(data_dir: &Path, release: &VerifiedRelease) -> Result<(), UpdateError> {
    let directory = data_dir.join("updates");
    fs::create_dir_all(&directory)
        .map_err(|err| UpdateError::Io(format!("Could not create update cache: {err}")))?;
    let stem = match release.manifest.channel {
        Channel::Stable => "stable",
        Channel::Preview => "preview",
    };
    let manifest_temp = directory.join(format!("{stem}.json.tmp"));
    let signature_temp = directory.join(format!("{stem}.sig.tmp"));
    fs::write(&manifest_temp, &release.signed_manifest)
        .map_err(|err| UpdateError::Io(format!("Could not cache signed news: {err}")))?;
    fs::write(&signature_temp, &release.signature)
        .map_err(|err| UpdateError::Io(format!("Could not cache update signature: {err}")))?;
    fs::rename(&manifest_temp, directory.join(format!("{stem}.json")))
        .map_err(|err| UpdateError::Io(format!("Could not activate cached manifest: {err}")))?;
    fs::rename(&signature_temp, directory.join(format!("{stem}.sig")))
        .map_err(|err| UpdateError::Io(format!("Could not activate cached signature: {err}")))
}

pub fn load_cached_release(data_dir: &Path, channel: Channel) -> Option<VerifiedRelease> {
    let stem = match channel {
        Channel::Stable => "stable",
        Channel::Preview => "preview",
    };
    let directory = data_dir.join("updates");
    let manifest_bytes = fs::read(directory.join(format!("{stem}.json"))).ok()?;
    let signature = fs::read(directory.join(format!("{stem}.sig"))).ok()?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES || signature.len() > MAX_SIGNATURE_BYTES {
        return None;
    }
    verify_signature(&manifest_bytes, &signature).ok()?;
    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes).ok()?;
    validate_manifest(&manifest, &manifest.tag, channel).ok()?;
    Some(VerifiedRelease {
        manifest,
        signed_manifest: manifest_bytes,
        signature,
    })
}

fn release_asset_url(release: &GitHubRelease, name: &str) -> Result<String, UpdateError> {
    let url = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| UpdateError::Invalid(format!("Release is missing {name}")))?;
    if !url.starts_with("https://github.com/Casualdv/Nephele-Client/releases/download/") {
        return Err(UpdateError::Invalid(format!(
            "Release asset URL is not on the trusted GitHub repository: {url}"
        )));
    }
    Ok(url)
}

fn get_limited(url: &str, max_bytes: usize) -> Result<Vec<u8>, UpdateError> {
    let response = ureq::get(url)
        .set("User-Agent", "NepheleLauncher")
        .call()
        .map_err(|err| UpdateError::Network(format!("Download failed: {err}")))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| UpdateError::Network(format!("Could not read response: {err}")))?;
    if bytes.len() > max_bytes {
        return Err(UpdateError::Invalid(
            "Downloaded metadata is too large".into(),
        ));
    }
    Ok(bytes)
}

fn verify_signature(payload: &[u8], signature: &[u8]) -> Result<(), UpdateError> {
    let modulus = option_env!("NEPHELE_UPDATE_PUBLIC_MODULUS_HEX")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(PINNED_UPDATE_PUBLIC_MODULUS_HEX);
    verify_signature_with_modulus(payload, signature, modulus)
}

fn verify_signature_with_modulus(
    payload: &[u8],
    signature: &[u8],
    modulus_hex: &str,
) -> Result<(), UpdateError> {
    let modulus = BigUint::parse_bytes(modulus_hex.trim().as_bytes(), 16)
        .ok_or_else(|| UpdateError::Invalid("Compiled update key is malformed".into()))?;
    let exponent = BigUint::from(65_537u32);
    let public_key = RsaPublicKey::new(modulus, exponent)
        .map_err(|err| UpdateError::Invalid(format!("Compiled update key is invalid: {err}")))?;
    let signature = Signature::try_from(signature)
        .map_err(|_| UpdateError::Invalid("Update signature is malformed".into()))?;
    VerifyingKey::<Sha256>::new(public_key)
        .verify(payload, &signature)
        .map_err(|_| UpdateError::Invalid("Release signature verification failed".into()))
}

fn validate_manifest(
    manifest: &ReleaseManifest,
    expected_tag: &str,
    expected_channel: Channel,
) -> Result<(), UpdateError> {
    if manifest.schema != 1 {
        return Err(UpdateError::Invalid(format!(
            "Unsupported update manifest schema {}",
            manifest.schema
        )));
    }
    if manifest.tag != expected_tag || manifest.channel != expected_channel {
        return Err(UpdateError::Invalid(
            "Release tag/channel does not match its signed manifest".into(),
        ));
    }
    if !crate::profiles::valid_version_id(&manifest.version) {
        return Err(UpdateError::Invalid("Release version is invalid".into()));
    }
    if manifest.notes.len() > 32_000 || manifest.news.len() > 20 {
        return Err(UpdateError::Invalid(
            "Release notes exceed safe limits".into(),
        ));
    }
    for item in &manifest.news {
        if item.title.len() > 120 || item.body.len() > 2_000 {
            return Err(UpdateError::Invalid(
                "News entry exceeds safe limits".into(),
            ));
        }
        if item
            .url
            .as_deref()
            .is_some_and(|url| !url.starts_with("https://"))
        {
            return Err(UpdateError::Invalid("News links must use HTTPS".into()));
        }
    }

    let files = manifest
        .platforms
        .get(platform_id())
        .ok_or_else(|| UpdateError::Invalid(format!("No build for {}", platform_id())))?;
    for name in REQUIRED_FILES {
        let asset = files
            .get(name)
            .ok_or_else(|| UpdateError::Invalid(format!("Manifest is missing {name}")))?;
        if !asset
            .url
            .starts_with("https://github.com/Casualdv/Nephele-Client/releases/download/")
        {
            return Err(UpdateError::Invalid(format!(
                "Download URL for {name} is not trusted"
            )));
        }
        if asset.size == 0 || asset.size > MAX_ASSET_BYTES || !valid_sha256(&asset.sha256) {
            return Err(UpdateError::Invalid(format!(
                "Download metadata for {name} is invalid"
            )));
        }
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn install_release(
    release: &VerifiedRelease,
    versions_root: &Path,
) -> Result<PathBuf, UpdateError> {
    if !crate::profiles::valid_version_id(&release.manifest.version) {
        return Err(UpdateError::Invalid("Release version is invalid".into()));
    }
    let assets = release
        .manifest
        .platforms
        .get(platform_id())
        .ok_or_else(|| UpdateError::Invalid(format!("No build for {}", platform_id())))?;
    fs::create_dir_all(versions_root)
        .map_err(|err| UpdateError::Io(format!("Could not create versions folder: {err}")))?;
    let target = versions_root.join(&release.manifest.version);
    if target.exists() {
        for name in REQUIRED_FILES {
            let asset = assets
                .get(name)
                .ok_or_else(|| UpdateError::Invalid(format!("Manifest is missing {name}")))?;
            if !verify_asset_file(&target.join(name), asset)? {
                return Err(UpdateError::Invalid(format!(
                    "Version {} is already present but {name} failed verification",
                    release.manifest.version
                )));
            }
        }
        return Ok(target);
    }
    let staging = versions_root.join(format!(
        ".installing-{}-{}",
        release.manifest.version,
        std::process::id()
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|err| UpdateError::Io(format!("Could not clear update staging: {err}")))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|err| UpdateError::Io(format!("Could not stage update: {err}")))?;

    let result = (|| {
        for name in REQUIRED_FILES {
            let asset = assets
                .get(name)
                .ok_or_else(|| UpdateError::Invalid(format!("Manifest is missing {name}")))?;
            download_verified(asset, &staging.join(name))?;
        }
        fs::write(staging.join(MANIFEST_NAME), &release.signed_manifest)
            .map_err(|err| UpdateError::Io(format!("Could not save signed manifest: {err}")))?;
        fs::write(staging.join(SIGNATURE_NAME), &release.signature)
            .map_err(|err| UpdateError::Io(format!("Could not save update signature: {err}")))?;
        fs::rename(&staging, &target).map_err(|err| {
            UpdateError::Io(format!("Could not activate downloaded version: {err}"))
        })?;
        Ok(target.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn download_verified(asset: &DownloadAsset, path: &Path) -> Result<(), UpdateError> {
    if !asset
        .url
        .starts_with("https://github.com/Casualdv/Nephele-Client/releases/download/")
        || asset.size == 0
        || asset.size > MAX_ASSET_BYTES
        || !valid_sha256(&asset.sha256)
    {
        return Err(UpdateError::Invalid(
            "Update file metadata is invalid".into(),
        ));
    }
    let response = ureq::get(&asset.url)
        .set("User-Agent", "NepheleLauncher")
        .call()
        .map_err(|err| UpdateError::Network(format!("Could not download update: {err}")))?;
    if let Some(length) = response.header("Content-Length") {
        if length.parse::<u64>().ok() != Some(asset.size) {
            return Err(UpdateError::Invalid(
                "Update file size does not match manifest".into(),
            ));
        }
    }
    let mut reader = response.into_reader().take(asset.size + 1);
    let mut file = File::create(path)
        .map_err(|err| UpdateError::Io(format!("Could not create update file: {err}")))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| UpdateError::Network(format!("Could not read update: {err}")))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > asset.size {
            return Err(UpdateError::Invalid(
                "Update file is larger than declared".into(),
            ));
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|err| UpdateError::Io(format!("Could not write update file: {err}")))?;
    }
    if total != asset.size || hex::encode(hasher.finalize()) != asset.sha256.to_ascii_lowercase() {
        return Err(UpdateError::Invalid(
            "Update file hash/size verification failed".into(),
        ));
    }
    file.sync_all()
        .map_err(|err| UpdateError::Io(format!("Could not flush update file: {err}")))?;
    set_executable(path)?;
    Ok(())
}

fn verify_asset_file(path: &Path, asset: &DownloadAsset) -> Result<bool, UpdateError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(UpdateError::Io(format!(
                "Could not inspect installed file: {err}"
            )))
        }
    };
    if metadata.len() != asset.size {
        return Ok(false);
    }
    let mut file = File::open(path)
        .map_err(|err| UpdateError::Io(format!("Could not read installed file: {err}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| UpdateError::Io(format!("Could not verify installed file: {err}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()) == asset.sha256.to_ascii_lowercase())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|err| UpdateError::Io(format!("Could not inspect update file: {err}")))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| UpdateError::Io(format!("Could not mark update executable: {err}")))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}

pub fn set_current_version(install_root: &Path, version: &str) -> Result<(), UpdateError> {
    if !crate::profiles::valid_version_id(version) {
        return Err(UpdateError::Invalid("Version id is invalid".into()));
    }
    fs::create_dir_all(install_root)
        .map_err(|err| UpdateError::Io(format!("Could not create install folder: {err}")))?;
    let target = install_root.join("current-version.txt");
    let temp = install_root.join("current-version.tmp");
    fs::write(&temp, version)
        .map_err(|err| UpdateError::Io(format!("Could not write current version: {err}")))?;
    fs::rename(temp, target)
        .map_err(|err| UpdateError::Io(format!("Could not select current version: {err}")))
}

pub fn installed_versions(versions_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(versions_root) else {
        return Vec::new();
    };
    let mut versions = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            (path.is_dir()
                && crate::profiles::valid_version_id(&name)
                && path.join("nephele.exe").is_file())
            .then_some(name)
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| compare_versions(right, left));
    versions
}

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let (left_core, left_pre) = left.split_once('-').unwrap_or((left, ""));
    let (right_core, right_pre) = right.split_once('-').unwrap_or((right, ""));
    let left_parts = version_parts(left_core);
    let right_parts = version_parts(right_core);
    for (left, right) in left_parts.iter().zip(right_parts.iter()) {
        let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    let core_order = left_parts.len().cmp(&right_parts.len());
    if core_order != std::cmp::Ordering::Equal {
        return core_order;
    }
    match (left_pre.is_empty(), right_pre.is_empty()) {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (true, true) => std::cmp::Ordering::Equal,
        (false, false) => compare_identifiers(left_pre, right_pre),
    }
}

fn compare_identifiers(left: &str, right: &str) -> std::cmp::Ordering {
    let left_parts = left.split('.').collect::<Vec<_>>();
    let right_parts = right.split('.').collect::<Vec<_>>();
    for (left, right) in left_parts.iter().zip(right_parts.iter()) {
        let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => left.cmp(right),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left_parts.len().cmp(&right_parts.len())
}

fn version_parts(version: &str) -> Vec<&str> {
    version
        .split(['.', '-', '_'])
        .filter(|part| !part.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;

    #[test]
    fn update_metadata_rejects_wrong_channel_tag_and_bad_hashes() {
        let mut files = BTreeMap::new();
        for name in REQUIRED_FILES {
            files.insert(
                name.to_string(),
                DownloadAsset {
                    url: format!(
                        "https://github.com/Casualdv/Nephele-Client/releases/download/v1/{name}"
                    ),
                    sha256: "a".repeat(64),
                    size: 100,
                },
            );
        }
        let mut platforms = BTreeMap::new();
        platforms.insert(platform_id().to_string(), files);
        let manifest = ReleaseManifest {
            schema: 1,
            tag: "v1".into(),
            version: "1.0.0".into(),
            channel: Channel::Stable,
            released_at: "2026-01-01T00:00:00Z".into(),
            notes: "Release".into(),
            news: Vec::new(),
            platforms,
        };
        assert!(validate_manifest(&manifest, "v1", Channel::Stable).is_ok());
        assert!(validate_manifest(&manifest, "v2", Channel::Stable).is_err());
        assert!(validate_manifest(&manifest, "v1", Channel::Preview).is_err());
        let mut bad_hash = manifest.clone();
        bad_hash
            .platforms
            .get_mut(platform_id())
            .unwrap()
            .get_mut("nephele.exe")
            .unwrap()
            .sha256 = "not-a-hash".into();
        assert!(validate_manifest(&bad_hash, "v1", Channel::Stable).is_err());
        assert!(compare_versions("1.0.0", "1.0.0-rc.1").is_gt());
        assert!(compare_versions("1.0.0-rc.2", "1.0.0-rc.10").is_lt());
    }

    #[test]
    fn version_comparison_orders_numeric_release_components() {
        assert!(compare_versions("0.10.0", "0.9.9").is_gt());
        assert!(compare_versions("1.0.0", "1.0.0").is_eq());
        assert!(!platform_id().is_empty());
    }

    #[test]
    fn signed_manifests_are_verified_with_the_pinned_rsa_key() {
        let private_key = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 1024).unwrap();
        let public_modulus = private_key.to_public_key().n().to_str_radix(16);
        let signing_key = SigningKey::<Sha256>::new(private_key);
        let payload = br#"{"schema":1,"version":"1.0.0"}"#;
        let signature = signing_key.sign(payload);
        assert!(
            verify_signature_with_modulus(payload, &signature.to_vec(), &public_modulus).is_ok()
        );
        assert!(
            verify_signature_with_modulus(b"modified", &signature.to_vec(), &public_modulus)
                .is_err()
        );
    }

    #[test]
    fn current_version_pointer_rejects_traversal() {
        let root =
            std::env::temp_dir().join(format!("nephele-version-pointer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert!(set_current_version(&root, "../outside").is_err());
        set_current_version(&root, "1.2.3").unwrap();
        set_current_version(&root, "1.2.4").unwrap();
        assert_eq!(
            fs::read_to_string(root.join("current-version.txt")).unwrap(),
            "1.2.4"
        );
        let _ = fs::remove_dir_all(root);
    }
}
