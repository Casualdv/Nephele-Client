//! kira-backed mixer. One `AudioEngine` owns the audio device, the sound
//! registry and a small cache of decoded files.
//!
//! Everything here is fallible and the caller is expected to carry on silently:
//! a laptop with no sound card, or a preparation with no sounds in it, must not
//! stop the game from starting.
//!
//! # Buses
//!
//! The mix follows vanilla's `SoundCategory` idea with fewer categories. Five
//! independent buses, each multiplied by `Master`:
//!
//! | bus | carries | vanilla equivalent |
//! |---|---|---|
//! | `Master` | everything | `MASTER` |
//! | `Music` | menu/background music | `MUSIC` |
//! | `Blocks` | break, place, footsteps | `BLOCK` |
//! | `Weather` | rain loop, thunder | `WEATHER` |
//! | `Mobs` | player hurt today, mob sounds when they exist | `HOSTILE` + `NEUTRAL` + `PLAYER` |
//!
//! Vanilla splits the last one three ways; we ship one entity bus and say so in
//! AGENTS.md. Anything that belongs to an entity belongs on `Mobs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kira::backend::Backend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::FromFileError;
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning, Tween};

use crate::decode::{decode_ogg, to_frames};
use crate::materials::SoundType;
use crate::registry::{RegistryError, SoundRegistry};
use crate::{attenuation, distance, gain_to_decibels, stereo_pan, MIN_GAIN};

/// Decoded sounds kept in memory at once. Vanilla's block sounds are all well
/// under a second, so this is a few megabytes at worst; clearing wholesale
/// beats an LRU for something this small.
const CACHE_LIMIT: usize = 96;

/// How loud the rain loop sits at `Weather = 100`. Vanilla scales the loop by
/// the rain strength; there is no strength in this client yet, so a fixed
/// fraction of the bus keeps it under the block sounds.
const RAIN_GAIN: f32 = 0.35;

/// Seconds between thunder rolls while it is thundering.
const THUNDER_MIN: f32 = 6.0;
const THUNDER_SPAN: f32 = 18.0;

/// Fade applied when a slider moves under a sound that is already playing.
const SLIDER_TWEEN: Duration = Duration::from_millis(80);

/// The five volume buses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    Master,
    Music,
    Blocks,
    Weather,
    Mobs,
}

/// Number of buses; keep in step with [`Bus::ALL`].
pub const BUS_COUNT: usize = 5;

impl Bus {
    pub const ALL: [Bus; BUS_COUNT] = [
        Bus::Master,
        Bus::Music,
        Bus::Blocks,
        Bus::Weather,
        Bus::Mobs,
    ];

    pub const fn index(self) -> usize {
        match self {
            Bus::Master => 0,
            Bus::Music => 1,
            Bus::Blocks => 2,
            Bus::Weather => 3,
            Bus::Mobs => 4,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Bus::Master => "master",
            Bus::Music => "music",
            Bus::Blocks => "blocks",
            Bus::Weather => "weather",
            Bus::Mobs => "mobs",
        }
    }
}

/// Linear gains, `0.0..=1.0`, one per bus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusGains {
    gains: [f32; BUS_COUNT],
}

impl BusGains {
    /// Every bus at `gain`.
    pub fn all(gain: f32) -> Self {
        Self {
            gains: [gain.clamp(0.0, 1.0); BUS_COUNT],
        }
    }

    /// Bus gains straight from the settings sliders, which are 0..=100.
    pub fn from_percent(master: u8, music: u8, blocks: u8, weather: u8, mobs: u8) -> Self {
        Self {
            gains: [
                percent(master),
                percent(music),
                percent(blocks),
                percent(weather),
                percent(mobs),
            ],
        }
    }

    pub fn get(self, bus: Bus) -> f32 {
        self.gains[bus.index()]
    }

    pub fn with(self, bus: Bus, gain: f32) -> Self {
        let mut next = self;
        next.gains[bus.index()] = gain.clamp(0.0, 1.0);
        next
    }
}

impl Default for BusGains {
    fn default() -> Self {
        Self::all(1.0)
    }
}

fn percent(value: u8) -> f32 {
    f32::from(value.min(100)) / 100.0
}

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
}

/// A music track that is streaming from disk.
struct MusicState {
    event: String,
    handle: StreamingSoundHandle<FromFileError>,
}

pub struct AudioEngine<B: Backend = DefaultBackend> {
    manager: AudioManager<B>,
    root: PathBuf,
    registry: SoundRegistry,
    cache: HashMap<PathBuf, (u32, Arc<[Frame]>)>,
    gains: BusGains,
    rng: Rng,
    music: Option<MusicState>,
    /// A music event that could not be opened, so the per-frame tick does not
    /// retry it 60 times a second.
    music_failed: Option<String>,
    rain: Option<StaticSoundHandle>,
    thunder_in: f32,
    played: u64,
    silent_events: u64,
    failed_decodes: u64,
}

impl AudioEngine<DefaultBackend> {
    /// Opens an audio device and loads `<asset_root>/sounds.json`.
    pub fn new(asset_root: &Path, gains: BusGains) -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| AudioError::Device(error.to_string()))?;
        Self::from_manager(manager, asset_root, gains)
    }
}

impl<B: Backend> AudioEngine<B> {
    /// Builds an engine around an already-constructed manager. Tests use this
    /// with kira's renderer capture instead of a real device.
    pub(crate) fn from_manager(
        manager: AudioManager<B>,
        asset_root: &Path,
        gains: BusGains,
    ) -> Result<Self, AudioError> {
        let registry = SoundRegistry::load(asset_root)?;
        Ok(Self {
            manager,
            root: asset_root.to_path_buf(),
            registry,
            cache: HashMap::new(),
            gains,
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
            music: None,
            music_failed: None,
            rain: None,
            thunder_in: THUNDER_MIN,
            played: 0,
            silent_events: 0,
            failed_decodes: 0,
        })
    }

    pub fn registry(&self) -> &SoundRegistry {
        &self.registry
    }

    pub fn gains(&self) -> BusGains {
        self.gains
    }

    /// What a sound on `bus` is actually multiplied by: `Master × bus`. Master
    /// is not applied twice to itself.
    pub fn effective_gain(&self, bus: Bus) -> f32 {
        let master = self.gains.get(Bus::Master);
        if bus == Bus::Master {
            master
        } else {
            master * self.gains.get(bus)
        }
    }

    /// Apply the sliders. Sounds that are already looping (music, rain) are
    /// re-tuned in place rather than restarted.
    pub fn set_gains(&mut self, gains: BusGains) {
        self.gains = gains;
        let music = self.effective_gain(Bus::Music);
        if let Some(state) = self.music.as_mut() {
            state
                .handle
                .set_volume(volume_value(music), Tween::default());
        }
        let rain = self.effective_gain(Bus::Weather) * RAIN_GAIN;
        if let Some(handle) = self.rain.as_mut() {
            handle.set_volume(volume_value(rain), slider_tween());
        }
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

    /// Play `event` on `bus` at a world position, with vanilla's optional
    /// `pitch` multiplier on top of the variant's own pitch.
    ///
    /// Returns whether a sound actually started.
    pub fn play_at(
        &mut self,
        event: &str,
        position: [f32; 3],
        listener: Listener,
        pitch: f32,
        bus: Bus,
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
        let volume = self.effective_gain(bus) * gain;
        self.play_variant(&variant, volume, panning, pitch)
    }

    /// Play `event` at the listener's own position, unpanned.
    pub fn play_local(&mut self, event: &str, pitch: f32, bus: Bus) -> bool {
        let Some(variant) = self.pick(event) else {
            return false;
        };
        let volume = self.effective_gain(bus);
        self.play_variant(&variant, volume, 0.0, pitch)
    }

    /// The local player's hurt sound, on the entity bus.
    ///
    /// Vanilla plays `game.player.hurt` at half volume with a random pitch in
    /// `1.0 +- 0.2`, and never pans it because the listener *is* the player.
    pub fn play_player_hurt(&mut self) -> bool {
        let jitter = (self.rng.next_f32() - self.rng.next_f32()) * 0.2 + 1.0;
        let Some(variant) = self.pick("game.player.hurt") else {
            return false;
        };
        let volume = self.effective_gain(Bus::Mobs) * 0.5;
        self.play_variant(&variant, volume, 0.0, jitter)
    }

    /// A sound made by an entity at a world position. Wired to the `Mobs` bus so
    /// mob sounds land on the right slider the day the entity renderer plays
    /// them; nothing calls it yet.
    pub fn play_mob(&mut self, event: &str, position: [f32; 3], listener: Listener) -> bool {
        self.play_at(event, position, listener, 1.0, Bus::Mobs)
    }

    /// Play a block's break sound, applying vanilla's volume and pitch.
    pub fn play_break(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::break_pitch(type_);
        self.play_at(type_.break_sound, position, listener, pitch, Bus::Blocks)
    }

    pub fn play_place(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::place_pitch(type_);
        self.play_at(type_.place_sound, position, listener, pitch, Bus::Blocks)
    }

    pub fn play_step(&mut self, type_: SoundType, position: [f32; 3], listener: Listener) -> bool {
        let pitch = crate::materials::step_pitch(type_);
        self.play_at(type_.step_sound, position, listener, pitch, Bus::Blocks)
    }

    /// Start a looping music track, streamed from disk so a three-minute ogg
    /// costs a decoder rather than tens of megabytes of samples.
    ///
    /// Idempotent: asking for the track that is already playing does nothing.
    pub fn play_music(&mut self, event: &str) -> bool {
        if self
            .music
            .as_ref()
            .is_some_and(|state| state.event == event)
        {
            return true;
        }
        // A track that failed to open is not retried every frame; the memo is
        // cleared by `stop_music`, so the next menu visit tries once more.
        if self.music_failed.as_deref() == Some(event) {
            return false;
        }
        self.stop_music();
        let Some(variant) = self.pick(event) else {
            return false;
        };
        let path = self.root.join(variant.relative_path());
        let data = match StreamingSoundData::from_file(&path) {
            Ok(data) => data,
            Err(error) => {
                self.failed_decodes += 1;
                tracing::debug!("audio: could not stream {path:?}: {error}");
                self.remember_music_failure(event);
                return false;
            }
        };
        let volume = self.effective_gain(Bus::Music) * variant.volume;
        let data = data
            .loop_region(..)
            .volume(volume_value(volume))
            .playback_rate(f64::from(variant.pitch.clamp(0.05, 8.0)));
        match self.manager.play(data) {
            Ok(handle) => {
                self.played += 1;
                tracing::debug!(event, "music started");
                self.music = Some(MusicState {
                    event: event.to_string(),
                    handle,
                });
                true
            }
            Err(error) => {
                tracing::debug!("audio: could not start music {event}: {error}");
                false
            }
        }
    }

    pub fn stop_music(&mut self) {
        if let Some(mut state) = self.music.take() {
            state.handle.stop(slider_tween());
            tracing::debug!(event = %state.event, "music stopped");
        }
        self.music_failed = None;
    }

    /// Event currently playing as music, if any.
    pub fn music_event(&self) -> Option<&str> {
        self.music.as_ref().map(|state| state.event.as_str())
    }

    fn remember_music_failure(&mut self, event: &str) {
        self.music_failed = Some(event.to_string());
    }

    /// Drive the weather ambience. `delta` is seconds since the last frame.
    ///
    /// Only the debug toggle produces these flags today: `S2B Change Game State`
    /// (the packet a server uses to start rain) is the `net` lane's.
    pub fn update_weather(&mut self, raining: bool, thundering: bool, delta: f32) {
        if raining {
            if self.rain.is_none() {
                self.start_rain();
            }
        } else if let Some(mut handle) = self.rain.take() {
            handle.stop(slider_tween());
            tracing::debug!("rain stopped");
        }

        if thundering {
            self.thunder_in -= delta.max(0.0);
            if self.thunder_in <= 0.0 {
                self.thunder_in = THUNDER_MIN + self.rng.next_f32() * THUNDER_SPAN;
                tracing::debug!("thunder roll");
                self.play_local("ambient.weather.thunder", 1.0, Bus::Weather);
            }
        } else {
            // Roll again soon after the next storm starts.
            self.thunder_in = THUNDER_MIN;
        }
    }

    fn start_rain(&mut self) {
        let Some(variant) = self.pick("ambient.weather.rain") else {
            return;
        };
        let Some((sample_rate, frames)) = self.frames_for(&variant.relative_path()) else {
            self.failed_decodes += 1;
            return;
        };
        let volume = self.effective_gain(Bus::Weather) * RAIN_GAIN * variant.volume;
        let data = StaticSoundData {
            sample_rate,
            frames,
            settings: StaticSoundSettings::new()
                .volume(volume_value(volume))
                .loop_region(..)
                .playback_rate(f64::from(variant.pitch.clamp(0.05, 8.0))),
            slice: None,
        };
        match self.manager.play(data) {
            Ok(handle) => {
                self.played += 1;
                self.rain = Some(handle);
            }
            Err(error) => tracing::debug!("audio: could not start rain: {error}"),
        }
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
        self.play_frames(sample_rate, frames, gain, panning, pitch * variant.pitch)
    }

    /// The one place a static sound reaches the mixer. Both the file path and
    /// the test tone come through here, so a bus test measures the same code the
    /// game runs.
    fn play_frames(
        &mut self,
        sample_rate: u32,
        frames: Arc<[Frame]>,
        gain: f32,
        panning: f32,
        pitch: f32,
    ) -> bool {
        if gain < MIN_GAIN {
            return false;
        }
        let data = StaticSoundData {
            sample_rate,
            frames,
            settings: StaticSoundSettings::new()
                .volume(Decibels(gain_to_decibels(gain)))
                .panning(Panning(panning.clamp(-1.0, 1.0)))
                .playback_rate(f64::from(pitch.clamp(0.05, 8.0))),
            slice: None,
        };
        match self.manager.play(data) {
            Ok(_handle) => {
                self.played += 1;
                true
            }
            Err(error) => {
                tracing::debug!("audio: could not start a sound: {error}");
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

/// A linear gain as a kira volume, with 0 mapped to true silence rather than
/// `-inf`.
fn volume_value(gain: f32) -> Decibels {
    if gain < MIN_GAIN {
        Decibels::SILENCE
    } else {
        Decibels(gain_to_decibels(gain.clamp(0.0, 1.0)))
    }
}

fn slider_tween() -> Tween {
    Tween {
        duration: SLIDER_TWEEN,
        ..Default::default()
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
    use kira::backend::Renderer;
    use std::sync::Mutex;

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

    #[test]
    fn master_multiplies_every_bus_but_not_itself() {
        let gains = BusGains::from_percent(50, 100, 40, 0, 80);
        assert!((gains.get(Bus::Master) - 0.5).abs() < 1e-6);
        assert!((gains.get(Bus::Blocks) - 0.4).abs() < 1e-6);
        // Effective gains are master x bus; master alone is just master.
        let expected = |bus: Bus| gains.get(Bus::Master) * gains.get(bus);
        for bus in Bus::ALL {
            if bus == Bus::Master {
                assert!((gains.get(bus) - 0.5).abs() < 1e-6);
            } else {
                assert!((gains.get(bus) - expected(bus) / 0.5).abs() < 1e-6);
            }
        }
        assert_eq!(Bus::ALL.len(), BUS_COUNT);
    }

    #[test]
    fn percent_sliders_are_clamped() {
        let gains = BusGains::from_percent(255, 0, 100, 50, 101);
        assert_eq!(gains.get(Bus::Master), 1.0);
        assert_eq!(gains.get(Bus::Music), 0.0);
        assert_eq!(gains.get(Bus::Blocks), 1.0);
        assert!((gains.get(Bus::Weather) - 0.5).abs() < 1e-6);
        assert_eq!(gains.get(Bus::Mobs), 1.0);
    }

    // -- mixer capture ------------------------------------------------------
    //
    // kira's own `MockBackend` renders into a buffer it does not expose, so the
    // bus test brings its own backend: it keeps the `Renderer` and hands it back
    // to the test, which then pumps real mixed samples. No device involved.

    struct CaptureBackend {
        slot: Arc<Mutex<Option<Renderer>>>,
    }

    struct CaptureSettings {
        slot: Arc<Mutex<Option<Renderer>>>,
    }

    impl Default for CaptureSettings {
        fn default() -> Self {
            Self {
                slot: Arc::new(Mutex::new(None)),
            }
        }
    }

    impl Backend for CaptureBackend {
        type Settings = CaptureSettings;
        type Error = std::convert::Infallible;

        fn setup(
            settings: Self::Settings,
            _internal_buffer_size: usize,
        ) -> Result<(Self, u32), Self::Error> {
            Ok((
                Self {
                    slot: settings.slot,
                },
                // Anything above MIN_GAIN's limit; the test renders manually.
                44_100,
            ))
        }

        fn start(&mut self, renderer: Renderer) -> Result<(), Self::Error> {
            *self.slot.lock().unwrap() = Some(renderer);
            Ok(())
        }
    }

    fn capture_engine(gains: BusGains) -> (AudioEngine<CaptureBackend>, Renderer) {
        // The registry is loaded from the asset root; the bus test does not use
        // it, so an empty directory is fine.
        let empty = std::env::temp_dir().join("nephele-audio-bus-test");
        let _ = std::fs::create_dir_all(&empty);
        std::fs::write(empty.join("sounds.json"), "{}").expect("empty registry writes");
        capture_engine_from(&empty, gains)
    }

    fn capture_engine_from(
        root: &Path,
        gains: BusGains,
    ) -> (AudioEngine<CaptureBackend>, Renderer) {
        let slot: Arc<Mutex<Option<Renderer>>> = Arc::new(Mutex::new(None));
        let manager = AudioManager::<CaptureBackend>::new(AudioManagerSettings {
            backend_settings: CaptureSettings { slot: slot.clone() },
            ..Default::default()
        })
        .expect("capture backend starts");
        let renderer = slot
            .lock()
            .unwrap()
            .take()
            .expect("the backend keeps the renderer");
        let engine = AudioEngine::from_manager(manager, root, gains).expect("engine builds");
        (engine, renderer)
    }

    /// A music event whose file is missing must fail once, not once per frame.
    #[test]
    fn a_missing_track_is_not_retried_every_frame() {
        let root = std::env::temp_dir().join("nephele-audio-missing-music");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("sounds.json"),
            r#"{ "music.menu": { "category": "music", "sounds": [ "music/menu/menu1" ] } }"#,
        )
        .unwrap();
        let (mut engine, _renderer) = capture_engine_from(&root, BusGains::all(1.0));

        assert!(!engine.play_music("music.menu"));
        assert_eq!(engine.failed_decodes(), 1);
        assert!(!engine.play_music("music.menu"), "memoised");
        assert_eq!(engine.failed_decodes(), 1, "not retried");
        // Leaving the screen clears the memo, so a later visit tries again.
        engine.stop_music();
        assert!(!engine.play_music("music.menu"));
        assert_eq!(engine.failed_decodes(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Root-mean-square of `frames` stereo frames rendered by `renderer`.
    fn render_rms(renderer: &mut Renderer, frames: usize) -> f32 {
        let mut buffer = vec![0.0f32; 256 * 2];
        let mut sum = 0.0f64;
        let mut count = 0usize;
        while count < frames {
            renderer.on_start_processing();
            renderer.process(&mut buffer, 2);
            for sample in &buffer {
                sum += f64::from(*sample) * f64::from(*sample);
                count += 1;
            }
        }
        (sum / count.max(1) as f64).sqrt() as f32
    }

    /// A 440 Hz tone as kira frames, so the test needs no prepared assets.
    fn tone(frames: usize) -> Arc<[Frame]> {
        let rate = 44_100.0f32;
        (0..frames)
            .map(|index| {
                let sample = (index as f32 / rate * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                Frame::new(sample, sample)
            })
            .collect()
    }

    /// The acceptance measurement: with every bus at 100, a tone on one bus is
    /// audible; with *that* bus at 0 it is silent, and muting it changes no
    /// other bus's gain. Rendered through the real mixer, per bus.
    #[test]
    fn each_bus_gain_only_silences_its_own_bus() {
        let (mut engine, mut renderer) = capture_engine(BusGains::all(1.0));
        // Prime the renderer so the first buffer is not a startup artefact.
        render_rms(&mut renderer, 512);

        for bus in [Bus::Blocks, Bus::Music, Bus::Weather, Bus::Mobs] {
            // Full: the tone reaches the output.
            engine.set_gains(BusGains::all(1.0));
            assert!(
                engine.play_frames(44_100, tone(4_410), engine.effective_gain(bus), 0.0, 1.0),
                "{bus:?} should start at full gain"
            );
            let full = render_rms(&mut renderer, 2_048);
            assert!(full > 0.05, "{bus:?} at 100 measured {full}");

            // Let the whole 0.1 s tone leave the pipeline, or the tail is
            // measured as if it were the muted sound.
            render_rms(&mut renderer, 16_384);

            // Muted: nothing of it reaches the output.
            engine.set_gains(BusGains::all(1.0).with(bus, 0.0));
            assert_eq!(engine.effective_gain(bus), 0.0, "{bus:?} gain");
            assert!(
                !engine.play_frames(44_100, tone(4_410), engine.effective_gain(bus), 0.0, 1.0),
                "{bus:?} should not start at zero gain"
            );
            let muted = render_rms(&mut renderer, 2_048);
            assert!(muted < 0.001, "{bus:?} at 0 still measured {muted}");
        }

        // Muting one bus leaves every other bus's gain untouched.
        let muted = BusGains::all(1.0).with(Bus::Mobs, 0.0);
        engine.set_gains(muted);
        for other in Bus::ALL {
            if other == Bus::Mobs || other == Bus::Master {
                continue;
            }
            assert!(
                (engine.effective_gain(other) - 1.0).abs() < 1e-6,
                "{other:?} moved when Mobs was muted"
            );
        }
    }

    /// Master scales everything, including sounds already started.
    #[test]
    fn master_silences_every_bus() {
        let (mut engine, mut renderer) = capture_engine(BusGains::all(1.0));
        render_rms(&mut renderer, 512);
        // One tone at full gain first, then mute while it is in the pipeline.
        assert!(engine.play_frames(44_100, tone(4_410), 0.5, 0.0, 1.0));
        let audible = render_rms(&mut renderer, 1_024);
        assert!(
            audible > 0.05,
            "the tone should be audible first: {audible}"
        );
        render_rms(&mut renderer, 16_384);
        engine.set_gains(BusGains::all(0.0));
        for bus in Bus::ALL {
            assert_eq!(engine.effective_gain(bus), 0.0, "{bus:?}");
        }
        // A bus sound is gated before it reaches the mixer: with master at 0 the
        // caller sees `false` and nothing is started.
        assert!(volume_value(engine.effective_gain(Bus::Blocks)) <= volume_value(0.0));
        let muted = render_rms(&mut renderer, 2_048);
        assert!(muted < 0.001, "master 0 still measured {muted}");
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
        // Quiet, not silent. Bus gains multiply by master, so master alone has
        // to stay above MIN_GAIN for a bus at 100 to be audible.
        let mut engine = AudioEngine::new(&root, BusGains::from_percent(20, 100, 100, 100, 100))
            .expect("audio device opens");
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

        // Music streams and stops; rain loops; thunder is a one-shot.
        assert!(engine.play_music("music.menu"), "menu music streams");
        assert_eq!(engine.music_event(), Some("music.menu"));
        engine.stop_music();
        assert_eq!(engine.music_event(), None);

        engine.update_weather(true, true, 0.0);
        engine.update_weather(true, true, THUNDER_MIN + 1.0);
        engine.update_weather(false, false, 0.0);
        assert_eq!(engine.failed_decodes(), 0);
    }
}
