//! Ogg Vorbis decoding.
//!
//! kira mixes but deliberately does not decode here: symphonia produces plain
//! interleaved `f32` samples, and [`to_frames`] packs them into the stereo
//! [kira `Frame`](kira::Frame)s a `StaticSoundData` wants. Keeping the two
//! apart means the decoder is testable with no audio device.

use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kira::Frame;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// A decoded sound, still at its file's sample rate.
#[derive(Debug, Clone)]
pub struct DecodedSound {
    pub sample_rate: u32,
    /// Channel count the file had; [`to_frames`] folds it down to stereo.
    pub channels: u16,
    /// Interleaved samples, `channels` per frame.
    pub samples: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("could not open {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a sound file symphonia understands: {message}")]
    Unsupported { path: PathBuf, message: String },
    #[error("{path} has no audio track")]
    NoTrack { path: PathBuf },
    #[error("decode error in {path}: {message}")]
    Decode { path: PathBuf, message: String },
}

impl DecodeError {
    fn unsupported(path: &Path, error: SymphoniaError) -> Self {
        DecodeError::Unsupported {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

/// Decode one Ogg Vorbis file to interleaved `f32` samples.
pub fn decode_ogg(path: &Path) -> Result<DecodedSound, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|error| DecodeError::unsupported(path, error))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| DecodeError::NoTrack {
            path: path.to_path_buf(),
        })?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| DecodeError::unsupported(path, error))?;

    let mut samples = Vec::new();
    let mut sample_rate = 0;
    let mut channels = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // Running out of packets is how a well-formed stream ends.
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(error) => {
                return Err(DecodeError::Decode {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
            }
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                sample_rate = spec.rate;
                channels = spec.channels.count() as u16;
                let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buffer.copy_interleaved_ref(decoded);
                samples.extend_from_slice(buffer.samples());
            }
            // A corrupt packet mid-file should not lose the whole sound.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => {
                return Err(DecodeError::Decode {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
            }
        }
    }

    if samples.is_empty() || channels == 0 {
        return Err(DecodeError::NoTrack {
            path: path.to_path_buf(),
        });
    }
    Ok(DecodedSound {
        sample_rate,
        channels,
        samples,
    })
}

/// Fold a decoded sound down to the stereo frames kira plays.
///
/// Mono is duplicated into both channels; more than two channels keeps the
/// front pair, which is all the mixer uses.
pub fn to_frames(decoded: &DecodedSound) -> Arc<[Frame]> {
    let channels = decoded.channels.max(1) as usize;
    let frames: Vec<Frame> = decoded
        .samples
        .chunks_exact(channels)
        .map(|frame| match channels {
            0 => Frame::ZERO,
            1 => Frame::new(frame[0], frame[0]),
            _ => Frame::new(frame[0], frame[1]),
        })
        .collect();
    frames.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_sound(relative: &str) -> Option<PathBuf> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest
            .join("../../.local/prepared/1.8.9/assets/minecraft")
            .join(relative);
        path.is_file().then_some(path)
    }

    #[test]
    fn mono_is_duplicated_into_both_channels() {
        let decoded = DecodedSound {
            sample_rate: 44_100,
            channels: 1,
            samples: vec![0.5, -0.5],
        };
        let frames = to_frames(&decoded);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Frame::new(0.5, 0.5));
        assert_eq!(frames[1], Frame::new(-0.5, -0.5));
    }

    #[test]
    fn stereo_keeps_its_sides() {
        let decoded = DecodedSound {
            sample_rate: 44_100,
            channels: 2,
            samples: vec![0.1, 0.2, 0.3, 0.4],
        };
        let frames = to_frames(&decoded);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Frame::new(0.1, 0.2));
        assert_eq!(frames[1], Frame::new(0.3, 0.4));
    }

    #[test]
    fn a_prepared_stone_break_decodes_to_audio() {
        let Some(path) = prepared_sound("sounds/dig/stone1.ogg") else {
            eprintln!("skipped: no prepared sounds here; run `nephele-prepare`");
            return;
        };
        let decoded = decode_ogg(&path).expect("stone1.ogg decodes");
        assert!(decoded.sample_rate >= 8_000, "{}", decoded.sample_rate);
        assert!(decoded.channels >= 1);
        assert!(decoded.samples.len() > 1_000);
        assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
        let frames = to_frames(&decoded);
        assert_eq!(
            frames.len(),
            decoded.samples.len() / decoded.channels as usize
        );
    }

    #[test]
    fn a_non_sound_file_is_reported_not_panicked() {
        let dir = std::env::temp_dir().join("nephele-audio-decode-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("not-a-sound.ogg");
        std::fs::write(&path, b"definitely not ogg vorbis").unwrap();
        let error = decode_ogg(&path).expect_err("must refuse");
        assert!(
            !error.to_string().is_empty(),
            "the error should name the problem"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
