//! kira-backed mixer. One `AudioEngine` owns the audio device, the sound
//! registry and a small cache of decoded files.
//!
//! Everything here is fallible and the caller is expected to carry on silently:
//! a laptop with no sound card, or a preparation with no sounds in it, must not
//! stop the game from starting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning};

use crate::decode::{decode_ogg, to_frames};
use crate::materials::SoundType;
use crate::registry::{RegistryError, SoundRegistry};
use crate::{attenuation, distance, gain_to_decibels, stereo_pan, MIN_GAIN};

/// Decoded sounds kept in memory at once. Vanilla's block sounds are all well
/// under a second, so this is a few megabytes at worst; clearing wholesale
/// beats an LRU for something this small.
const CACHE_LIMIT: usize = 96;

/// Where the player is and which way is right, so a sound can be placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    /// Unit vector pointing out of the player's right ear.
    pub right: [f32; 3],
}

impl Listener {
    /// Builds a listener from the camera's yaw, matching the client's own
    /// convention (`forward = (sin yaw, 0, -cos yaw)`).
    pub fn from_yaw(position: [f32; 3], yaw: f32) -> Self {
        let (sin, cos) = yaw.sin_cos();
        Self {
            position,
            right: [cos, 0.0, sin],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio device: {0}")]
    Device(String),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("could not play {event}: {message}")]
    Play { event: String, message: String },
}

pub struct AudioEngine {
    manager: AudioManager<DefaultBackend>,
    root: PathBuf,
    registry: SoundRegistry,
    cache: HashMap<PathBuf, (u32, Arc<[Frame]>)>,
    master: f32,
    rng: Rng,
    played: u64,
    silent_events: u64,
    failed_decodes: u64,
}

impl AudioEngine {
    /// Opens an audio device and loads `<asset_root>/sounds.json`.
    ///
    /// `master` is a linear gain, 0.0..=1.0.
    pub fn new(asset_root: &Path, master: f32) -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| AudioError::Device(error.to_string()))?;
        let registry = SoundRegistry::load(asset_root)?;
        Ok(Self {
            manager,
            root: asset_root.to_path_buf(),
            registry,
            cache: HashMap::new(),
            master: master.clamp(0.0, 1.0),
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
            played: 0,
            silent_events: 0,
            failed_decodes: 0,
        })
    }

    pub fn registry(&self) -> &SoundRegistry {
        &self.registry
    }

    pub fn master_volume(&self) -> f32 {
        self.master
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master = volume.clamp(0.0, 1.0);
    }

    /// Sounds started, for diagnostics.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// Events the registry has no entry for (vanilla ships a few of these:
    /// `step.anvil`, `dig.ladder`). They stay silent, exactly like vanilla.
    pub fn silent_events(&self) -> u64 {
        self.silent_events
    }

    pub fn failed_decodes(&self) -> u64 {
        self.failed_decodes
    }

    /// Play `event` at a world position, with vanilla's optional `pitch`
    /// multiplier on top of the variant's own pitch.
    ///
    /// Returns whether a sound actually started.
    pub fn play_at(
        &mut self,
        event: &str,
        position: [f32; 3],
        listener: Listener,
        pitch: f32,
    ) -> bool {
        let Some(variant) = self.pick(event) else {
            return false;
        };
        let distance = distance(listener.position, position);
        let gain = attenuation(distance);
        if gain < MIN_GAIN {
            return false;
        }

        let right = (position[0] - listener.position[0]) * listener.right[0]
            + (position[1] - listener.position[1]) * listener.right[1]
            + (position[2] - listener.position[2]) * listener.right[2];
        let panning = stereo_pan(right);
        let volume = self.master * gain;
        self.play_variant(&variant, volume, panning, pitch)
    }

    /// Play `event` at the listener's own position, unpanned. Used for sounds
    /// that belong to the player rather than to the world.
    pub fn play_local(&mut self, event: &str, pitch: f32) -> bool {
        let Some(variant) = self.pick(event) else {
            return false;
        };
        let volume = self.master;
        self.play_variant(&variant, volume, 0.0, pitch)
    }

    /// The local player's hurt sound.
    ///
    /// Vanilla plays `game.player.hurt` at half volume with a random pitch in
    /// `1.0 +- 0.2`, and never pans it because the listener *is* the player.
    pub fn play_player_hurt(&mut self) -> bool {
        let jitter = (self.rng.next_f32() - self.rng.next_f32()) * 0.2 + 1.0;
        let Some(variant) = self.pick("game.player.hurt") else {
            return false;
        };
        let volume = self.master * 0.5;
        self.play_variant(&variant, volume, 0.0, jitter)
    }

    /// Play a block's break sound, applying vanilla's volume and pitch.
    pub fn play_break(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::break_pitch(type_);
        self.play_at(type_.break_sound, position, listener, pitch)
    }

    pub fn play_place(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::place_pitch(type_);
        self.play_at(type_.place_sound, position, listener, pitch)
    }

    pub fn play_step(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::step_pitch(type_);
        self.play_at(type_.step_sound, position, listener, pitch)
    }

    fn pick(&mut self, event: &str) -> Option<crate::registry::SoundVariant> {
        let roll = self.rng.next_u32();
        let Some(entry) = self.registry.event(event) else {
            self.silent_events += 1;
            return None;
        };
        // Cloned so the `&mut self` borrow for the cache stays free; a variant
        // is a name plus four numbers.
        entry.variant(roll).cloned()
    }

    fn play_variant(
        &mut self,
        variant: &crate::registry::SoundVariant,
        volume: f32,
        panning: f32,
        pitch: f32,
    ) -> bool {
        let gain = volume * variant.volume;
        if gain < MIN_GAIN {
            return false;
        }
        let relative = variant.relative_path();
        let Some((sample_rate, frames)) = self.frames_for(&relative) else {
            self.failed_decodes += 1;
            return false;
        };
        let data = StaticSoundData {
            sample_rate,
            frames,
            settings: StaticSoundSettings::new()
                .volume(Decibels(gain_to_decibels(gain)))
                .panning(Panning(panning.clamp(-1.0, 1.0)))
                .playback_rate(f64::from((pitch * variant.pitch).clamp(0.05, 8.0))),
            slice: None,
        };
        match self.manager.play(data) {
            Ok(_handle) => {
                self.played += 1;
                true
            }
            Err(error) => {
                tracing::debug!("audio: could not start {relative:?}: {error}");
                false
            }
        }
    }

    fn frames_for(&mut self, relative: &Path) -> Option<(u32, Arc<[Frame]>)> {
        if let Some((rate, frames)) = self.cache.get(relative) {
            return Some((*rate, Arc::clone(frames)));
        }
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        let path = self.root.join(relative);
        match decode_ogg(&path) {
            Ok(decoded) => {
                let rate = decoded.sample_rate;
                let frames = to_frames(&decoded);
                self.cache
                    .insert(relative.to_path_buf(), (rate, Arc::clone(&frames)));
                Some((rate, frames))
            }
            Err(error) => {
                tracing::debug!("audio: {error}");
                None
            }
        }
    }
}

/// xorshift64*, so choosing among a block's four break sounds needs no
/// dependency and stays reproducible in tests.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// A roll in `0.0..1.0`, matching vanilla's `rand.nextFloat()`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rng_is_deterministic_by_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let first: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let second: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn the_rng_does_not_repeat_immediately() {
        let mut rng = Rng::new(7);
        let rolls: Vec<u32> = (0..16).map(|_| rng.next_u32()).collect();
        assert!(rolls.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn a_listener_facing_north_has_right_due_east() {
        // yaw 0 looks down -Z, so the player's right hand is +X.
        let listener = Listener::from_yaw([0.0; 3], 0.0);
        assert!((listener.right[0] - 1.0).abs() < 1e-6);
        assert!(listener.right[2].abs() < 1e-6);
    }

    #[test]
    fn a_listener_facing_east_has_right_due_south() {
        let listener = Listener::from_yaw([0.0; 3], std::f32::consts::FRAC_PI_2);
        assert!(listener.right[0].abs() < 1e-6);
        assert!((listener.right[2] - 1.0).abs() < 1e-6);
    }

    /// Ignored by default: it opens the real sound device. Run it with
    /// `cargo test -p nephele-audio -- --ignored --nocapture` on a machine with
    /// audio and a prepared 1.8.9 asset tree to prove the whole chain -
    /// registry, decode, mixer - actually starts sounds.
    #[test]
    #[ignore = "opens the audio device"]
    fn the_real_device_starts_the_four_launch_sounds() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../.local/prepared/1.8.9/assets/minecraft");
        if !root.join("sounds.json").is_file() {
            eprintln!("skipped: no prepared sounds; run `nephele-prepare`");
            return;
        }
        // Quiet, not silent: a zero master volume is itself a reason not to
        // start a sound, so it would short-circuit before the mixer.
        let mut engine = AudioEngine::new(&root, 0.05).expect("audio device opens");
        assert!(
            engine.registry().len() > 200,
            "{} events",
            engine.registry().len()
        );

        let listener = Listener::from_yaw([0.0, 64.0, 0.0], 0.0);
        let near = [2.0, 64.0, -1.0];
        assert!(engine.play_break(crate::materials::sound_type(1).unwrap(), near, listener));
        assert!(engine.play_place(crate::materials::sound_type(2).unwrap(), near, listener));
        assert!(engine.play_step(crate::materials::sound_type(3).unwrap(), near, listener));
        assert!(engine.play_player_hurt());
        assert_eq!(engine.played(), 4);
        assert_eq!(engine.failed_decodes(), 0);

        // Beyond the 16-block cutoff nothing is even decoded or started.
        assert!(!engine.play_break(
            crate::materials::sound_type(1).unwrap(),
            [90.0, 64.0, 90.0],
            listener
        ));
        assert_eq!(engine.played(), 4);
    }
}
