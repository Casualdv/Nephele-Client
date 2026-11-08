//! `assets/minecraft/sounds.json`: the mapping from a sound *event* name
//! (`dig.stone`, `step.grass`, `game.player.hurt`) to the files that can play
//! for it.
//!
//! 1.8's format is a JSON object keyed by event name. Each event carries a
//! `category` and a `sounds` array whose entries are either a plain string
//! (`"dig/stone1"`) or an object with optional `volume`, `pitch`, `weight` and
//! `type` fields. Names are relative to `sounds/` and carry no extension; every
//! 1.8 sound file is Ogg Vorbis, so a variant resolves to `sounds/<name>.ogg`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One playable file inside a sound event.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundVariant {
    /// Path under `sounds/` without the extension, e.g. `dig/stone1`.
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    /// Relative likelihood when the event has several variants. Never zero.
    pub weight: u32,
    /// 1.8 streams long sounds (music, records) instead of loading them.
    pub stream: bool,
}

impl SoundVariant {
    /// `sounds/<name>.ogg`, relative to the Minecraft asset root.
    pub fn relative_path(&self) -> PathBuf {
        PathBuf::from("sounds").join(format!("{}.ogg", self.name))
    }
}

/// A named sound event.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEvent {
    /// `master`, `block`, `player`, `neutral`, ... (vanilla's sound categories).
    pub category: String,
    pub variants: Vec<SoundVariant>,
}

impl SoundEvent {
    /// Pick a variant for `roll`. Deterministic: the same roll always picks the
    /// same variant, which is what makes this testable without an audio device.
    pub fn variant(&self, roll: u32) -> Option<&SoundVariant> {
        let total: u32 = self.variants.iter().map(|v| v.weight).sum();
        if total == 0 {
            return None;
        }
        let mut remaining = roll % total;
        for variant in &self.variants {
            if remaining < variant.weight {
                return Some(variant);
            }
            remaining -= variant.weight;
        }
        self.variants.last()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("sounds.json is not valid: {0}")]
    Json(#[from] serde_json::Error),
}

/// Every event in `sounds.json`.
#[derive(Debug, Clone, Default)]
pub struct SoundRegistry {
    events: BTreeMap<String, SoundEvent>,
}

impl SoundRegistry {
    /// Reads `<asset root>/sounds.json`.
    pub fn load(asset_root: &Path) -> Result<Self, RegistryError> {
        let path = asset_root.join("sounds.json");
        let text = std::fs::read_to_string(&path).map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        let raw: BTreeMap<String, RawEvent> = serde_json::from_str(text)?;
        let mut events = BTreeMap::new();
        for (name, event) in raw {
            let variants: Vec<SoundVariant> = event
                .sounds
                .into_iter()
                .filter_map(RawSound::into_variant)
                .collect();
            if variants.is_empty() {
                continue;
            }
            events.insert(
                name,
                SoundEvent {
                    category: event.category.unwrap_or_else(|| "master".to_string()),
                    variants,
                },
            );
        }
        Ok(Self { events })
    }

    pub fn event(&self, name: &str) -> Option<&SoundEvent> {
        self.events.get(name)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.events.keys().map(String::as_str)
    }
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    sounds: Vec<RawSound>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSound {
    Named(String),
    Detailed {
        name: String,
        #[serde(default)]
        volume: Option<f32>,
        #[serde(default)]
        pitch: Option<f32>,
        #[serde(default)]
        weight: Option<u32>,
        #[serde(default, rename = "type")]
        kind: Option<String>,
    },
}

impl RawSound {
    fn into_variant(self) -> Option<SoundVariant> {
        match self {
            RawSound::Named(name) => Some(SoundVariant {
                // A bare string still means "add .ogg" unless it says otherwise.
                name: strip_extension(name),
                volume: 1.0,
                pitch: 1.0,
                weight: 1,
                stream: false,
            }),
            RawSound::Detailed {
                name,
                volume,
                pitch,
                weight,
                kind,
            } => Some(SoundVariant {
                name: strip_extension(name),
                volume: volume.unwrap_or(1.0).clamp(0.0, 8.0),
                pitch: pitch.unwrap_or(1.0).clamp(0.05, 8.0),
                // A zero weight would make the variant unreachable; vanilla
                // treats a missing weight as 1 and never ships 0.
                weight: weight.unwrap_or(1).max(1),
                stream: kind.as_deref() == Some("stream"),
            }),
        }
    }
}

fn strip_extension(name: String) -> String {
    name.strip_suffix(".ogg")
        .map(str::to_string)
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "dig.stone": {
            "category": "block",
            "sounds": [ "dig/stone1", "dig/stone2", { "name": "dig/stone3", "volume": 0.5, "pitch": 0.8, "weight": 3 } ]
        },
        "music.game": { "category": "music", "sounds": [ { "name": "music/game/calm1", "type": "stream" } ] },
        "empty.event": { "category": "block", "sounds": [] }
    }"#;

    #[test]
    fn parses_string_and_object_variants() {
        let registry = SoundRegistry::parse(SAMPLE).expect("parses");
        let stone = registry.event("dig.stone").expect("dig.stone exists");
        assert_eq!(stone.category, "block");
        assert_eq!(stone.variants.len(), 3);
        assert_eq!(stone.variants[0].name, "dig/stone1");
        assert_eq!(stone.variants[0].volume, 1.0);
        assert_eq!(stone.variants[2].volume, 0.5);
        assert_eq!(stone.variants[2].pitch, 0.8);
        assert_eq!(stone.variants[2].weight, 3);
    }

    #[test]
    fn an_event_with_no_sounds_is_dropped() {
        let registry = SoundRegistry::parse(SAMPLE).expect("parses");
        assert!(registry.event("empty.event").is_none());
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn stream_type_is_recorded() {
        let registry = SoundRegistry::parse(SAMPLE).expect("parses");
        let music = registry.event("music.game").expect("music.game exists");
        assert!(music.variants[0].stream);
        assert_eq!(music.variants[0].name, "music/game/calm1");
    }

    #[test]
    fn a_variant_name_keeps_its_own_extension() {
        let registry = SoundRegistry::parse(
            r#"{ "x": { "category": "master", "sounds": [ "dig/stone1.ogg" ] } }"#,
        )
        .expect("parses");
        let variant = &registry.event("x").unwrap().variants[0];
        assert_eq!(variant.name, "dig/stone1");
        assert_eq!(
            variant.relative_path(),
            PathBuf::from("sounds").join("dig/stone1.ogg")
        );
    }

    #[test]
    fn variant_choice_follows_weights_and_wraps() {
        let registry = SoundRegistry::parse(SAMPLE).expect("parses");
        let stone = registry.event("dig.stone").expect("exists");
        // Weights are 1, 1, 3: rolls 0..1 -> first, 2..3 -> second, 4..6 -> third.
        assert_eq!(stone.variant(0).unwrap().name, "dig/stone1");
        assert_eq!(stone.variant(1).unwrap().name, "dig/stone2");
        assert_eq!(stone.variant(2).unwrap().name, "dig/stone3");
        assert_eq!(stone.variant(4).unwrap().name, "dig/stone3");
        // Wraps rather than running off the end.
        assert_eq!(stone.variant(5 + 5).unwrap(), stone.variant(0).unwrap());
    }

    #[test]
    fn a_missing_event_is_none() {
        let registry = SoundRegistry::parse(SAMPLE).expect("parses");
        assert!(registry.event("dig.nope").is_none());
    }
}
