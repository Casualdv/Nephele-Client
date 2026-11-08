//! The vanilla `sounds.json` event table.
//!
//! Each event maps to one or more Ogg Vorbis files. Paths are relative to the
//! prepared `assets/minecraft` directory, so every asset stays derived from the
//! player's own game installation.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct SoundSample {
    /// Asset name without the `.ogg` suffix, e.g. `dig/stone1`.
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    pub weight: u32,
    pub stream: bool,
}

impl SoundSample {
    pub fn path(&self) -> String {
        if self.name.ends_with(".ogg") {
            format!("sounds/{}", self.name)
        } else {
            format!("sounds/{}.ogg", self.name)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundEvent {
    pub category: String,
    pub sounds: Vec<SoundSample>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundCatalog {
    events: BTreeMap<String, SoundEvent>,
}

impl SoundCatalog {
    pub fn load(assets_root: &Path) -> Result<Self, crate::AudioError> {
        let path = assets_root.join("sounds.json");
        let text = std::fs::read_to_string(&path).map_err(|source| crate::AudioError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, crate::AudioError> {
        let value: Value = serde_json::from_str(text)?;
        let object = value.as_object().ok_or_else(|| {
            crate::AudioError::Catalog("sounds.json root is not an object".into())
        })?;
        let mut events = BTreeMap::new();
        for (name, entry) in object {
            let category = entry
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or("master")
                .to_string();
            let mut sounds = Vec::new();
            if let Some(items) = entry.get("sounds").and_then(Value::as_array) {
                for item in items {
                    if let Some(sample) = parse_sample(item) {
                        sounds.push(sample);
                    }
                }
            }
            if !sounds.is_empty() {
                events.insert(name.clone(), SoundEvent { category, sounds });
            }
        }
        Ok(Self { events })
    }

    pub fn get(&self, event: &str) -> Option<&SoundEvent> {
        self.events.get(event)
    }

    pub fn contains(&self, event: &str) -> bool {
        self.events.contains_key(event)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Resolve an event to one sample. `roll` selects among weighted entries;
    /// the engine supplies a fresh value per playback.
    pub fn choose(&self, event: &str, roll: u32) -> Option<&SoundSample> {
        let event = self.get(event)?;
        let total: u32 = event.sounds.iter().map(|sample| sample.weight.max(1)).sum();
        if total == 0 {
            return None;
        }
        let mut cursor = roll % total;
        for sample in &event.sounds {
            let weight = sample.weight.max(1);
            if cursor < weight {
                return Some(sample);
            }
            cursor -= weight;
        }
        event.sounds.last()
    }
}

fn parse_sample(value: &Value) -> Option<SoundSample> {
    if let Some(name) = value.as_str() {
        return Some(SoundSample {
            name: name.to_string(),
            volume: 1.0,
            pitch: 1.0,
            weight: 1,
            stream: false,
        });
    }
    let object = value.as_object()?;
    let name = object
        .get("name")
        .or_else(|| object.get("sound"))
        .and_then(Value::as_str)?
        .to_string();
    Some(SoundSample {
        name,
        volume: object.get("volume").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        pitch: object.get("pitch").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        weight: object.get("weight").and_then(Value::as_u64).unwrap_or(1) as u32,
        stream: object
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUNDS: &str = r#"{
      "block.stone.break": {
        "category": "block",
        "sounds": [
          "dig/stone1",
          { "name": "dig/stone2", "volume": 0.8, "pitch": 1.1, "weight": 3 }
        ]
      },
      "mob.zombie.say": {
        "category": "hostile",
        "sounds": [ "mob/zombie/say1", "mob/zombie/say2" ]
      }
    }"#;

    #[test]
    fn events_keep_their_category_and_samples() {
        let catalog = SoundCatalog::from_json(SOUNDS).unwrap();
        assert_eq!(catalog.len(), 2);
        let stone = catalog.get("block.stone.break").unwrap();
        assert_eq!(stone.category, "block");
        assert_eq!(stone.sounds.len(), 2);
        assert_eq!(stone.sounds[1].weight, 3);
        assert_eq!(stone.sounds[0].path(), "sounds/dig/stone1.ogg");
    }

    #[test]
    fn weighted_choice_hits_every_sample() {
        let catalog = SoundCatalog::from_json(SOUNDS).unwrap();
        let names: Vec<_> = (0..4)
            .filter_map(|roll| catalog.choose("block.stone.break", roll))
            .map(|sample| sample.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["dig/stone1", "dig/stone2", "dig/stone2", "dig/stone2"]
        );
    }

    #[test]
    fn unknown_events_are_none() {
        let catalog = SoundCatalog::from_json(SOUNDS).unwrap();
        assert!(catalog.choose("nothing", 0).is_none());
        assert!(!catalog.contains("nothing"));
    }
}
