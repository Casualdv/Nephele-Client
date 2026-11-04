//! Nephele's sound engine.
//!
//! Symphonia decodes the Ogg Vorbis files prepared from the player's own 1.8.9
//! assets. cpal supplies the platform output stream. Playback is mixed on the
//! audio thread with distance attenuation and stereo panning relative to the
//! camera listener.

pub mod catalog;
pub mod decode;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, StreamConfig};
use tracing::{debug, warn};

pub use catalog::{SoundCatalog, SoundEvent, SoundSample};
pub use decode::{decode_ogg, DecodedSound};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sound catalog: {0}")]
    Catalog(String),
    #[error("audio decode: {0}")]
    Decode(String),
    #[error("audio output: {0}")]
    Output(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    /// Camera yaw in radians (0 looks towards -Z, positive turns right).
    pub yaw: f32,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            yaw: 0.0,
        }
    }
}

struct Voice {
    sound: Arc<DecodedSound>,
    cursor: f64,
    volume: f32,
    pitch: f32,
    position: Option<[f32; 3]>,
}

struct Mixer {
    output_rate: u32,
    channels: usize,
    listener: Listener,
    voices: Vec<Voice>,
}

impl Mixer {
    fn new(output_rate: u32, channels: usize) -> Self {
        Self {
            output_rate: output_rate.max(1),
            channels: channels.max(1),
            listener: Listener::default(),
            voices: Vec::new(),
        }
    }

    fn push(&mut self, voice: Voice) {
        // Keep a bounded voice pool: a particle storm must not starve the
        // output callback.
        const MAX_VOICES: usize = 64;
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(voice);
    }

    fn fill(&mut self, output: &mut [f32], master: f32) {
        output.fill(0.0);
        let frames = output.len() / self.channels;
        let right = [self.listener.yaw.cos(), 0.0, self.listener.yaw.sin()];
        let mut finished = false;

        for voice in &mut self.voices {
            let source_channels = voice.sound.channels as usize;
            let source_frames = voice.sound.samples.len() / source_channels;
            if source_frames == 0 {
                finished = true;
                continue;
            }
            let (attenuation, left_pan, right_pan) = if let Some(position) = voice.position {
                let delta = [
                    position[0] - self.listener.position[0],
                    position[1] - self.listener.position[1],
                    position[2] - self.listener.position[2],
                ];
                let distance =
                    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
                let attenuation = (1.0 - distance / 32.0).clamp(0.0, 1.0);
                let pan = if distance > 0.001 {
                    (delta[0] * right[0] + delta[2] * right[2]) / distance
                } else {
                    0.0
                };
                let pan = pan.clamp(-1.0, 1.0);
                (
                    attenuation * attenuation,
                    ((1.0 - pan) * 0.5).sqrt(),
                    ((1.0 + pan) * 0.5).sqrt(),
                )
            } else {
                (1.0, 1.0, 1.0)
            };
            let gain = voice.volume * master * attenuation;
            let step =
                voice.sound.sample_rate as f64 / self.output_rate as f64 * voice.pitch as f64;
            for frame in 0..frames {
                if voice.cursor >= source_frames as f64 {
                    finished = true;
                    break;
                }
                let source_frame = voice.cursor.floor() as usize;
                let fraction = (voice.cursor - source_frame as f64) as f32;
                let next_frame = (source_frame + 1).min(source_frames - 1);
                for channel in 0..self.channels {
                    let source_channel = channel.min(source_channels - 1);
                    let a = voice.sound.samples[source_frame * source_channels + source_channel];
                    let b = voice.sound.samples[next_frame * source_channels + source_channel];
                    let sample = a + (b - a) * fraction;
                    let pan = match channel {
                        0 => left_pan,
                        1 => right_pan,
                        _ => 1.0,
                    };
                    output[frame * self.channels + channel] += sample * gain * pan;
                }
                voice.cursor += step;
            }
            if voice.cursor >= source_frames as f64 {
                finished = true;
            }
        }

        if finished {
            self.voices.retain(|voice| {
                voice.cursor < (voice.sound.samples.len() / voice.sound.channels as usize) as f64
            });
        }
        for sample in output {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

struct PlayRequest {
    event: String,
    position: Option<[f32; 3]>,
    volume: f32,
    pitch: f32,
    roll: u32,
}

/// A live audio device plus its decoder worker. Constructing this never fails:
/// a machine without an output device simply gets an engine that drops sounds.
pub struct AudioEngine {
    catalog: Arc<SoundCatalog>,
    mixer: Arc<Mutex<Mixer>>,
    request_tx: Sender<PlayRequest>,
    master: Arc<AtomicU32>,
    next_roll: AtomicU32,
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn new(assets_root: &Path) -> Self {
        let catalog = match SoundCatalog::load(assets_root) {
            Ok(catalog) => Arc::new(catalog),
            Err(error) => {
                warn!("audio disabled: {error}");
                Arc::new(SoundCatalog::default())
            }
        };
        let master = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let mut stream = None;
        let mixer = match default_output() {
            Ok((device, config)) => {
                let mixer = Arc::new(Mutex::new(Mixer::new(
                    config.sample_rate.0,
                    config.channels as usize,
                )));
                match build_stream(&device, &config, Arc::clone(&mixer), Arc::clone(&master)) {
                    Ok(built) => {
                        stream = Some(built);
                    }
                    Err(error) => warn!("audio output unavailable: {error}"),
                }
                mixer
            }
            Err(error) => {
                warn!("audio output unavailable: {error}");
                Arc::new(Mutex::new(Mixer::new(44_100, 2)))
            }
        };

        let (request_tx, request_rx) = channel();
        let worker = spawn_decoder_worker(
            Arc::clone(&catalog),
            assets_root.to_path_buf(),
            request_rx,
            Arc::clone(&mixer),
        );

        Self {
            catalog,
            mixer,
            request_tx,
            master,
            next_roll: AtomicU32::new(1),
            stream,
            worker,
        }
    }

    pub fn is_available(&self) -> bool {
        self.stream.is_some() && !self.catalog.is_empty()
    }

    pub fn catalog_len(&self) -> usize {
        self.catalog.len()
    }

    pub fn set_listener(&self, position: [f32; 3], yaw: f32) {
        self.mixer.lock().unwrap().listener = Listener { position, yaw };
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.master
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn master_volume(&self) -> f32 {
        f32::from_bits(self.master.load(Ordering::Relaxed))
    }

    /// Play a named `sounds.json` event. `position` is world-space; `None`
    /// plays non-positionally (UI, music, local player).
    pub fn play(&self, event: &str, position: Option<[f32; 3]>, volume: f32, pitch: f32) {
        if self.stream.is_none() || !self.catalog.contains(event) {
            return;
        }
        let roll = self.next_roll.fetch_add(1, Ordering::Relaxed);
        let _ = self.request_tx.send(PlayRequest {
            event: event.to_string(),
            position,
            volume: volume.max(0.0),
            pitch: pitch.clamp(0.25, 4.0),
            roll,
        });
    }

    pub fn stop_all(&self) {
        self.mixer.lock().unwrap().voices.clear();
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stream = None;
        if let Some(worker) = self.worker.take() {
            // The request sender is still alive until after the fields drop,
            // so ask the worker to stop explicitly through a sentinel event.
            let _ = self.request_tx.send(PlayRequest {
                event: String::new(),
                position: None,
                volume: 0.0,
                pitch: 1.0,
                roll: 0,
            });
            let _ = worker.join();
        }
    }
}

fn spawn_decoder_worker(
    catalog: Arc<SoundCatalog>,
    assets_root: PathBuf,
    requests: std::sync::mpsc::Receiver<PlayRequest>,
    mixer: Arc<Mutex<Mixer>>,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("nephele-audio".into())
        .spawn(move || {
            let mut cache: HashMap<String, Arc<DecodedSound>> = HashMap::new();
            while let Ok(request) = requests.recv() {
                if request.event.is_empty() {
                    break;
                }
                let Some(sample) = catalog.choose(&request.event, request.roll) else {
                    continue;
                };
                let path = assets_root.join(sample.path());
                let sound = if let Some(sound) = cache.get(&sample.name) {
                    Arc::clone(sound)
                } else {
                    match decode_ogg(&path) {
                        Ok(sound) => {
                            let sound = Arc::new(sound);
                            cache.insert(sample.name.clone(), Arc::clone(&sound));
                            sound
                        }
                        Err(error) => {
                            debug!("could not decode {}: {error}", path.display());
                            continue;
                        }
                    }
                };
                mixer.lock().unwrap().push(Voice {
                    sound,
                    cursor: 0.0,
                    volume: request.volume * sample.volume,
                    pitch: request.pitch * sample.pitch,
                    position: request.position,
                });
            }
        })
        .ok()
}

fn default_output() -> Result<(cpal::Device, StreamConfig), AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| AudioError::Output("no default output device".into()))?;
    let supported = device
        .default_output_config()
        .map_err(|error| AudioError::Output(error.to_string()))?;
    Ok((device, supported.config()))
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    master: Arc<AtomicU32>,
) -> Result<cpal::Stream, AudioError> {
    let supported = device
        .default_output_config()
        .map_err(|error| AudioError::Output(error.to_string()))?;
    let sample_format = supported.sample_format();
    let error_callback = |error| warn!("audio stream error: {error}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            build_typed::<f32>(device, config, mixer, master, error_callback)
        }
        cpal::SampleFormat::I16 => {
            build_typed::<i16>(device, config, mixer, master, error_callback)
        }
        cpal::SampleFormat::U16 => {
            build_typed::<u16>(device, config, mixer, master, error_callback)
        }
        other => Err(AudioError::Output(format!(
            "unsupported sample format {other:?}"
        ))),
    }?;
    stream
        .play()
        .map_err(|error| AudioError::Output(error.to_string()))?;
    Ok(stream)
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    master: Arc<AtomicU32>,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch = vec![0.0f32; 2048 * channels];
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                if scratch.len() < output.len() {
                    scratch.resize(output.len(), 0.0);
                }
                let mut mixer = mixer.lock().unwrap();
                let master = f32::from_bits(master.load(Ordering::Relaxed));
                mixer.fill(&mut scratch[..output.len()], master);
                for (target, sample) in output.iter_mut().zip(scratch.iter()) {
                    *target = T::from_sample(*sample);
                }
            },
            error_callback,
            None,
        )
        .map_err(|error| AudioError::Output(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_positional_voice_reaches_the_output_buffer() {
        let mut mixer = Mixer::new(44_100, 2);
        mixer.push(Voice {
            sound: Arc::new(DecodedSound {
                samples: vec![0.5; 64],
                sample_rate: 44_100,
                channels: 1,
            }),
            cursor: 0.0,
            volume: 1.0,
            pitch: 1.0,
            position: Some([0.0, 0.0, 0.0]),
        });
        let mut output = [0.0f32; 64];
        mixer.fill(&mut output, 1.0);
        assert!(output.iter().any(|sample| *sample > 0.0));
    }

    #[test]
    fn listener_panning_puts_a_left_source_in_the_left_channel() {
        let mut mixer = Mixer::new(44_100, 2);
        mixer.listener = Listener {
            position: [0.0; 3],
            yaw: 0.0,
        };
        mixer.push(Voice {
            sound: Arc::new(DecodedSound {
                samples: vec![1.0; 64],
                sample_rate: 44_100,
                channels: 1,
            }),
            cursor: 0.0,
            volume: 1.0,
            pitch: 1.0,
            position: Some([-4.0, 0.0, 0.0]),
        });
        let mut output = [0.0f32; 64];
        mixer.fill(&mut output, 1.0);
        assert!(output[0] > output[1]);
    }
}
