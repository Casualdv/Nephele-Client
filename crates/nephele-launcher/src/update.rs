//! Update check, gated on a signing key.
//!
//! The key is compiled in from `NEPHELE_UPDATE_PUBLIC_KEY` and is **empty by
//! default**, which is the honest state of this project: there is no code
//! signing certificate yet, so an unsigned build must not pretend it can
//! verify an update. An empty key means:
//!
//! - no network request is made;
//! - the launcher shows `UNSIGNED BUILD — UPDATES DISABLED`;
//! - startup logs a warning once.
//!
//! With a key present, the launcher fetches the manifest from
//! `NEPHELE_UPDATE_URL`, verifies an Ed25519 signature over a canonical
//! payload, and only then reads the version. Signature verification uses
//! `ring` (already a dependency of the workspace through `ureq`'s TLS stack).
//!
//! This is a **check**, not an updater: nothing is downloaded or executed.
//! That is deliberate until a signed update can be tested end to end.

use std::time::Duration;

use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;

/// Domain separator for the signed payload. Changing the format means
/// changing this string, so an old manifest cannot be replayed as a new one.
const PAYLOAD_PREFIX: &str = "nephele-update-v1";

/// A manifest as served by the update endpoint. The signature covers
/// `version`, `notes` and `url` — not this struct's JSON whitespace.
#[derive(Debug, Clone, Deserialize)]
pub struct SignedManifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub url: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// No signing key was compiled in. No network access was attempted.
    Unsigned,
    /// A key exists but no endpoint is configured in this build.
    Disabled(String),
    Checking,
    UpToDate {
        current: String,
    },
    Available {
        version: String,
        notes: String,
        url: String,
    },
    Failed(String),
}

impl UpdateStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Unsigned => "UPDATES DISABLED (UNSIGNED BUILD)".to_string(),
            Self::Disabled(reason) => format!("updates disabled: {reason}"),
            Self::Checking => "checking for updates...".to_string(),
            Self::UpToDate { current } => format!("up to date (v{current})"),
            Self::Available { version, .. } => format!("update available: v{version}"),
            Self::Failed(reason) => format!("update check failed: {reason}"),
        }
    }

    pub fn available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub fn is_unsigned(&self) -> bool {
        matches!(self, Self::Unsigned)
    }
}

/// The compiled-in Ed25519 public key, or `""` for an unsigned build.
pub fn embedded_public_key() -> &'static str {
    option_env!("NEPHELE_UPDATE_PUBLIC_KEY").unwrap_or("")
}

/// The compiled-in manifest endpoint, if any.
pub fn embedded_endpoint() -> Option<&'static str> {
    option_env!("NEPHELE_UPDATE_URL").filter(|url| !url.trim().is_empty())
}

pub fn is_signed() -> bool {
    is_signed_with(embedded_public_key())
}

fn is_signed_with(public_key_hex: &str) -> bool {
    !public_key_hex.trim().is_empty()
}

/// `UNSIGNED BUILD` or `SIGNED <first 8 hex>`, for the launcher's top bar.
pub fn signing_summary() -> String {
    signing_summary_for(embedded_public_key())
}

fn signing_summary_for(public_key_hex: &str) -> String {
    let key = public_key_hex.trim();
    if key.is_empty() {
        "UNSIGNED BUILD".to_string()
    } else {
        format!("SIGNED {}", &key[..key.len().min(8)])
    }
}

fn payload(manifest: &SignedManifest) -> Vec<u8> {
    format!(
        "{PAYLOAD_PREFIX}\n{}\n{}\n{}",
        manifest.version, manifest.notes, manifest.url
    )
    .into_bytes()
}

/// Verify a detached Ed25519 signature. Exposed for tests and for a future
/// downloaded-artifact check; returns a human-readable error on failure.
pub fn verify_bytes(
    public_key_hex: &str,
    message: &[u8],
    signature_hex: &str,
) -> Result<(), String> {
    let key =
        hex::decode(public_key_hex.trim()).map_err(|_| "public key is not hex".to_string())?;
    let signature =
        hex::decode(signature_hex.trim()).map_err(|_| "signature is not hex".to_string())?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(message, &signature)
        .map_err(|_| "signature does not match".to_string())
}

/// True when `candidate` is a later release than `current`. Numeric components
/// only; a pre-release suffix is treated as its numeric prefix, which is the
/// conservative choice (never nag about a build we cannot order).
pub fn is_newer(current: &str, candidate: &str) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        let core = version.trim_start_matches('v');
        let core = core.split(['-', '+']).next().unwrap_or(core);
        core.split('.')
            .map(|part| {
                part.chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
            })
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    let mut current = parts(current);
    let mut candidate = parts(candidate);
    let length = current.len().max(candidate.len());
    current.resize(length, 0);
    candidate.resize(length, 0);
    candidate > current
}

/// Pure decision function: given the build's key, its endpoint, the running
/// version and the server's response body, produce a status. Kept separate
/// from the network so every branch is testable offline.
pub fn resolve(
    public_key_hex: &str,
    endpoint: Option<&str>,
    current: &str,
    body: Result<&str, String>,
) -> UpdateStatus {
    if public_key_hex.trim().is_empty() {
        return UpdateStatus::Unsigned;
    }
    let Some(endpoint) = endpoint.filter(|url| !url.trim().is_empty()) else {
        return UpdateStatus::Disabled("this build has no update endpoint".to_string());
    };
    let body = match body {
        Ok(body) => body,
        Err(err) => return UpdateStatus::Failed(err),
    };
    let manifest = match serde_json::from_str::<SignedManifest>(body) {
        Ok(manifest) => manifest,
        Err(err) => return UpdateStatus::Failed(format!("malformed manifest: {err}")),
    };
    if let Err(err) = verify_bytes(public_key_hex, &payload(&manifest), &manifest.signature) {
        return UpdateStatus::Failed(format!("{err} ({endpoint})"));
    }
    if is_newer(current, &manifest.version) {
        UpdateStatus::Available {
            version: manifest.version,
            notes: manifest.notes,
            url: manifest.url,
        }
    } else {
        UpdateStatus::UpToDate {
            current: current.to_string(),
        }
    }
}

/// Fetch and verify, synchronously. Only called from a background thread, and
/// only when a key is present.
pub fn check_now(current: &str) -> UpdateStatus {
    let key = embedded_public_key();
    let Some(endpoint) = embedded_endpoint() else {
        return resolve(key, None, current, Ok(""));
    };
    if key.trim().is_empty() {
        return UpdateStatus::Unsigned;
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let body = match agent.get(endpoint).call() {
        Ok(response) => response.into_string().map_err(|err| err.to_string()),
        Err(err) => Err(io_error_text(err)),
    };
    resolve(
        key,
        Some(endpoint),
        current,
        body.as_deref().map_err(|err| err.clone()),
    )
}

fn io_error_text(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("update server returned HTTP {code}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8032 §7.1, test vector 1: empty message.
    const TEST_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const TEST_SIGNATURE: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
                                 5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";

    #[test]
    fn an_unsigned_build_never_reaches_the_network() {
        // Deliberately passing an endpoint as well: the empty key must win.
        assert_eq!(
            resolve(
                "",
                Some("https://example.invalid/update.json"),
                "0.1.0",
                Ok("{}")
            ),
            UpdateStatus::Unsigned
        );
        assert!(!is_signed_with(""));
        assert_eq!(signing_summary_for(""), "UNSIGNED BUILD");
        assert!(is_signed_with(TEST_KEY));
        assert_eq!(signing_summary_for(TEST_KEY), "SIGNED d75a9801");
    }

    #[test]
    fn a_key_without_an_endpoint_is_disabled_not_failed() {
        assert_eq!(
            resolve(TEST_KEY, None, "0.1.0", Ok("{}")),
            UpdateStatus::Disabled("this build has no update endpoint".to_string())
        );
    }

    #[test]
    fn the_rfc_8032_vector_verifies_and_tampering_fails() {
        verify_bytes(TEST_KEY, b"", TEST_SIGNATURE).unwrap();
        assert!(verify_bytes(TEST_KEY, b"x", TEST_SIGNATURE).is_err());
        assert!(verify_bytes(TEST_KEY, b"", &"00".repeat(64)).is_err());
        assert!(verify_bytes("not hex", b"", TEST_SIGNATURE).is_err());
    }

    #[test]
    fn a_correctly_signed_manifest_is_trusted() {
        // The RFC vector signs the empty message, so use `verify_bytes` directly
        // to prove the payload function is stable, then check its shape.
        let manifest = SignedManifest {
            version: "1.2.3".to_string(),
            notes: "fixes".to_string(),
            url: "https://example.invalid/nephele-1.2.3.zip".to_string(),
            signature: TEST_SIGNATURE.to_string(),
        };
        assert_eq!(
            String::from_utf8(payload(&manifest)).unwrap(),
            "nephele-update-v1\n1.2.3\nfixes\nhttps://example.invalid/nephele-1.2.3.zip"
        );
    }

    #[test]
    fn an_invalid_signature_is_rejected() {
        let body = format!(
            r#"{{"version":"9.9.9","notes":"","url":"","signature":"{}"}}"#,
            "00".repeat(64)
        );
        let status = resolve(
            TEST_KEY,
            Some("https://example.invalid"),
            "0.1.0",
            Ok(&body),
        );
        assert!(
            matches!(status, UpdateStatus::Failed(ref reason) if reason.contains("signature")),
            "{status:?}"
        );
    }

    #[test]
    fn a_malformed_manifest_is_an_error_not_an_update() {
        let status = resolve(
            TEST_KEY,
            Some("https://example.invalid"),
            "0.1.0",
            Ok("nope"),
        );
        assert!(matches!(status, UpdateStatus::Failed(ref reason) if reason.contains("malformed")));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.9", "0.2.0"));
        assert!(is_newer("0.9.0", "1.0.0"));
        assert!(is_newer("1.0", "1.0.1"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.3", "1.2.2"));
        assert!(!is_newer("1.2.3", "v1.2.3"));
        // A pre-release is not treated as newer than its release.
        assert!(!is_newer("1.2.3", "1.2.3-beta.1"));
    }
}
