//! Ogg Vorbis decoding through Symphonia.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone)]
pub struct DecodedSound {
    /// Interleaved f32 samples in `[-1, 1]`.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn decode_ogg(path: &Path) -> Result<DecodedSound, crate::AudioError> {
    let file = File::open(path).map_err(|source| crate::AudioError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| crate::AudioError::Decode(error.to_string()))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .filter(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| crate::AudioError::Decode("no decodable audio track".into()))?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let channels = track
        .codec_params
        .channels
        .map(|channels| channels.count() as u16)
        .unwrap_or(2);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| crate::AudioError::Decode(error.to_string()))?;

    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(error) => return Err(crate::AudioError::Decode(error.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let mut buffer =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                buffer.copy_interleaved_ref(decoded);
                samples.extend_from_slice(buffer.samples());
            }
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(error) => return Err(crate::AudioError::Decode(error.to_string())),
        }
    }
    if samples.is_empty() {
        return Err(crate::AudioError::Decode(format!(
            "{} decoded no samples",
            path.display()
        )));
    }
    Ok(DecodedSound {
        samples,
        sample_rate,
        channels: channels.max(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_prepared_stone_sound_when_available() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/prepared/1.8.9/assets/minecraft/sounds/dig/stone1.ogg");
        if !path.is_file() {
            eprintln!("skipping: run `nephele-prepare` to prepare sounds");
            return;
        }
        let sound = decode_ogg(&path).expect("stone sound decodes");
        assert!(sound.sample_rate > 0);
        assert!(sound.channels >= 1);
        assert!(!sound.samples.is_empty());
    }
}
