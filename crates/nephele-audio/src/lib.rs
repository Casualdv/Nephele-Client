//! Nephele's sound engine.
//!
//! Two jobs, deliberately kept apart:
//!
//! - **Decoding** ([`decode`]) turns the player's own Ogg Vorbis files into
//!   plain `f32` samples with symphonia. No audio device is needed, so it is
//!   testable.
//! - **Mixing** ([`engine`]) hands those samples to kira and applies the game's
//!   own volume and panning rules.
//!
//! Nothing here ships a sound: `sounds.json` names the events and
//! `nephele-prepare` copies the files out of the player's own 1.8.9 install.

pub mod decode;
pub mod engine;
pub mod materials;
pub mod registry;

pub use decode::{decode_ogg, DecodeError, DecodedSound};
pub use engine::{AudioEngine, AudioError, Bus, BusGains, Listener};
pub use materials::SoundType;
pub use registry::{SoundEvent, SoundRegistry, SoundVariant};

/// How far a block sound carries. Vanilla cuts normal sounds off at 16 blocks;
/// past this the client does not even send them.
pub const MAX_DISTANCE: f32 = 16.0;

/// Below this the caller should not bother starting a sound at all. It is well
/// under audibility, but keeps a mutex lock and a mixer voice out of the way.
pub const MIN_GAIN: f32 = 0.01;

/// Straight-line distance between two points.
pub fn distance(from: [f32; 3], to: [f32; 3]) -> f32 {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Vanilla-ish distance attenuation: full volume at the source, silent at
/// [`MAX_DISTANCE`].
///
/// OpenAL's real model is inverse-distance with a rolloff factor; a linear ramp
/// over the 16 blocks the client already uses as its cutoff lands close enough
/// and is trivial to reason about.
pub fn attenuation(distance: f32) -> f32 {
    (1.0 - distance / MAX_DISTANCE).clamp(0.0, 1.0)
}

/// Stereo placement from how far along the listener's right axis the source
/// sits: `-1` hard left, `0` centred, `1` hard right.
pub fn stereo_pan(right_distance: f32) -> f32 {
    (right_distance / MAX_DISTANCE).clamp(-1.0, 1.0)
}

/// Linear gain as decibels, which is what kira's mixer wants. Gain 0 is
/// reported as silence rather than negative infinity.
pub fn gain_to_decibels(gain: f32) -> f32 {
    if gain <= 0.0 {
        return -60.0;
    }
    20.0 * gain.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_falls_to_silence_at_sixteen_blocks() {
        assert!((attenuation(0.0) - 1.0).abs() < 1e-6);
        assert!((attenuation(8.0) - 0.5).abs() < 1e-6);
        assert!(attenuation(MAX_DISTANCE) <= 1e-6);
        assert_eq!(attenuation(64.0), 0.0);
    }

    #[test]
    fn a_sound_behind_the_listener_is_not_panned() {
        assert_eq!(stereo_pan(0.0), 0.0);
    }

    #[test]
    fn panning_saturates_instead_of_overflowing() {
        assert_eq!(stereo_pan(4.0), 0.25);
        assert_eq!(stereo_pan(-4.0), -0.25);
        assert_eq!(stereo_pan(100.0), 1.0);
        assert_eq!(stereo_pan(-100.0), -1.0);
    }

    #[test]
    fn distance_is_euclidean() {
        assert!((distance([0.0; 3], [3.0, 4.0, 0.0]) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn gain_and_decibels_round_trip() {
        assert!((gain_to_decibels(1.0)).abs() < 1e-6);
        assert!((gain_to_decibels(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(gain_to_decibels(0.0), -60.0);
    }
}
