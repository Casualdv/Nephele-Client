//! Skin resolution.
//!
//! The only place a skin texture is ever described is the `textures` property
//! on `S38` (Player List Item). It is a base64-encoded JSON document shaped like
//! `{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/<hash>"}}}`.
//! This module decodes that, extracts the hash and remembers it per player.

use std::collections::HashMap;

/// A player's appearance, as advertised in the tab list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkinInfo {
    pub name: String,
    /// Texture hash from the skin URL, if the account has a custom skin.
    pub hash: Option<String>,
    /// The `metadata.model` field: `true` for the 3px-wide "slim" arms.
    pub slim: bool,
    /// UUID of the owning account, kept for `GET /session/minecraft/profile`.
    pub uuid: [u8; 16],
}

impl SkinInfo {
    /// Canonical CDN URL, or `None` when the player uses the default skin.
    pub fn texture_url(&self) -> Option<String> {
        self.hash
            .as_ref()
            .map(|hash| format!("https://textures.minecraft.net/texture/{hash}"))
    }
}

/// Every player's advertised skin, keyed by UUID.
#[derive(Debug, Default, Clone)]
pub struct Skins {
    entries: HashMap<[u8; 16], SkinInfo>,
}

impl Skins {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, uuid: &[u8; 16]) -> Option<&SkinInfo> {
        self.entries.get(uuid)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8; 16], &SkinInfo)> {
        self.entries.iter()
    }

    /// Record one tab-list entry. Entries that carry no `textures` property are
    /// still stored, so the name is known even when the skin isn't.
    pub fn record(&mut self, entry: &nephele_protocol::packets::play::PlayerListEntry) {
        let (hash, slim) = entry
            .properties
            .iter()
            .find(|property| property.name == "textures")
            .and_then(|property| skin_from_property(&property.value))
            .unwrap_or((None, false));
        self.entries.insert(
            entry.uuid,
            SkinInfo {
                name: entry.name.clone(),
                hash,
                slim,
                uuid: entry.uuid,
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Extract the skin hash out of the base64 `textures` property value.
pub fn skin_hash_from_property(value: &str) -> Option<String> {
    skin_from_property(value).and_then(|(hash, _)| hash)
}

/// Extract the skin hash and the slim/classic model from a `textures` property.
///
/// The model is `"slim"` for Alex-shaped skins and absent (classic) for Steve.
/// Unknown values are treated as classic so a malformed profile still renders.
pub fn skin_from_property(value: &str) -> Option<(Option<String>, bool)> {
    let decoded = base64_decode(value)?;
    let json = std::str::from_utf8(&decoded).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
    let skin = parsed.get("textures")?.get("SKIN")?;
    let hash = skin
        .get("url")
        .and_then(|url| url.as_str())
        .and_then(texture_hash_from_url);
    let slim = skin
        .get("metadata")
        .and_then(|metadata| metadata.get("model"))
        .and_then(|model| model.as_str())
        .is_some_and(|model| model.eq_ignore_ascii_case("slim"));
    Some((hash, slim))
}

/// `http(s)://textures.minecraft.net/texture/<hash>` -> `<hash>`.
pub fn texture_hash_from_url(url: &str) -> Option<String> {
    let hash = url.rsplit('/').next()?;
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hash.to_string())
}

/// Standard base64, tolerant of padding and of the URL-safe alphabet.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            // Skip whitespace and newlines; reject anything else.
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real-shaped textures property: 64x64 skin for the default skin.
    const PROPERTY: &str = "eyJ0aW1lc3RhbXAiOjE1MDAwMDAwMDAwMDAsInByb2ZpbGVJZCI6ImFiY2QiLCJwcm9maWxlTmFtZSI6Ik5vdGNoIiwidGV4dHVyZXMiOnsiU0tJTiI6eyJ1cmwiOiJodHRwOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlLzZhY2UxM2Q5NWUxM2Y3YzFhZmYwMWE2NTI0MjZmMWYifX19";

    #[test]
    fn extracts_the_skin_hash_from_a_textures_property() {
        assert_eq!(
            skin_hash_from_property(PROPERTY).as_deref(),
            Some("6ace13d95e13f7c1aff01a652426f1f")
        );
    }

    #[test]
    fn slim_metadata_selects_the_slim_model() {
        // {"textures":{"SKIN":{"url":".../abc123","metadata":{"model":"slim"}}}}
        let value = "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHA6Ly90ZXh0dXJlcy5taW5lY3JhZnQubmV0L3RleHR1cmUvYWJjMTIzIiwibWV0YWRhdGEiOnsibW9kZWwiOiJzbGltIn19fX0=";
        assert_eq!(
            skin_from_property(value),
            Some((Some("abc123".to_string()), true))
        );
    }

    #[test]
    fn missing_model_metadata_is_classic() {
        assert_eq!(
            skin_from_property(PROPERTY),
            Some((Some("6ace13d95e13f7c1aff01a652426f1f".to_string()), false))
        );
    }

    #[test]
    fn property_without_skin_is_none() {
        // {"textures":{"CAPE":{"url":"http://textures.minecraft.net/texture/abc"}}}
        let value = "eyJ0ZXh0dXJlcyI6eyJDQVBFIjp7InVybCI6Imh0dHA6Ly90ZXh0dXJlcy5taW5lY3JhZnQubmV0L3RleHR1cmUvYWJjIn19fQ==";
        assert_eq!(skin_hash_from_property(value), None);
    }

    #[test]
    fn rejects_non_hash_urls() {
        assert_eq!(
            texture_hash_from_url("https://textures.minecraft.net/texture/not-a-hash"),
            None
        );
        assert_eq!(
            texture_hash_from_url("https://textures.minecraft.net/texture/6ace13d9"),
            Some("6ace13d9".to_string())
        );
    }

    #[test]
    fn base64_round_trips_known_vectors() {
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode("TQ==").unwrap(), b"M");
        assert_eq!(base64_decode("").unwrap(), b"");
        // URL-safe alphabet.
        assert_eq!(base64_decode("-_8=").unwrap(), vec![0xfb, 0xff]);
        assert!(base64_decode("****").is_none());
    }

    #[test]
    fn records_entries_and_exposes_the_cdn_url() {
        let mut skins = Skins::new();
        let uuid = [9u8; 16];
        skins.record(&nephele_protocol::packets::play::PlayerListEntry {
            uuid,
            name: "Notch".into(),
            properties: vec![nephele_protocol::packets::play::PlayerProperty {
                name: "textures".into(),
                value: PROPERTY.into(),
                signature: None,
            }],
        });
        let info = skins.get(&uuid).expect("recorded");
        assert_eq!(info.name, "Notch");
        assert_eq!(
            info.texture_url().as_deref(),
            Some("https://textures.minecraft.net/texture/6ace13d95e13f7c1aff01a652426f1f")
        );
    }

    #[test]
    fn entries_without_textures_are_still_remembered() {
        let mut skins = Skins::new();
        let uuid = [3u8; 16];
        skins.record(&nephele_protocol::packets::play::PlayerListEntry {
            uuid,
            name: "Alex".into(),
            properties: Vec::new(),
        });
        let info = skins.get(&uuid).unwrap();
        assert_eq!(info.name, "Alex");
        assert_eq!(info.texture_url(), None);
    }
}
