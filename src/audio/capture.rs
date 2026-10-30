//! Shared CPAL input plumbing.
//!
//! Every chunked capture driver (`chunker`, `chunk_recorder`,
//! `transcription::pipeline`, `processor::pipeline`,
//! `transcription::live_local`) used to resolve the default input device,
//! build the same f32 callback stream, and drain complete chunks out of the
//! same shared buffer. This module owns that machinery; the drivers keep only
//! their own loop shape, banner text, and per-chunk work.
//!
//! Nothing here is unit-tested because it touches CPAL and real hardware.
//! The pure pieces it stands on (`chunker::chunk_size_or_error`,
//! `chunker::has_complete_chunk`, `chunker::drain_chunk`) are covered by
//! their own tests.

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, StreamConfig};

use crate::audio::chunker::{chunk_size_or_error, drain_chunk, has_complete_chunk};

/// Default input device plus its stream config, resolved once.
///
/// The two labels are pre-formatted so each driver can print its own banner
/// without deriving the device/config text again.
pub struct DefaultInput {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: cpal::SampleFormat,
    device_label: String,
    supported_label: String,
}

impl DefaultInput {
    /// Resolve the default input device and its default stream config.
    pub fn resolve() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("No default input device available"))?;
        let supported = device.default_input_config()?;
        Ok(Self {
            device_label: format!("{device}"),
            supported_label: format!("{supported:?}"),
            sample_format: supported.sample_format(),
            config: supported.into(),
            device,
        })
    }

    pub fn device(&self) -> &cpal::Device {
        &self.device
    }

    pub fn config(&self) -> StreamConfig {
        self.config
    }

    pub fn sample_format(&self) -> cpal::SampleFormat {
        self.sample_format
    }

    /// `Display` form of the device, as printed in every driver banner.
    pub fn device_label(&self) -> &str {
        &self.device_label
    }

    /// `Debug` form of the device's default config, as printed in banners.
    pub fn supported_label(&self) -> &str {
        &self.supported_label
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.config.channels
    }
}

/// An unplayed capture stream feeding f32 samples into a shared buffer, plus
/// the input config those samples were captured with.
pub struct MicInput {
    input: DefaultInput,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: cpal::Stream,
}

impl MicInput {
    /// Resolve the default input device and build its capture stream.
    /// `error_prefix` labels stream errors, e.g. `"chunk recorder stream
    /// error: …"`.
    pub fn start(error_prefix: &'static str) -> Result<Self> {
        let input = DefaultInput::resolve()?;
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let stream = match input.sample_format() {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(&input, Arc::clone(&buffer), error_prefix)?
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(&input, Arc::clone(&buffer), error_prefix)?
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(&input, Arc::clone(&buffer), error_prefix)?
            }
            other => return Err(anyhow!("Unsupported sample format: {other:?}")),
        };
        Ok(Self {
            input,
            buffer,
            stream,
        })
    }

    pub fn input(&self) -> &DefaultInput {
        &self.input
    }

    pub fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    pub fn channels(&self) -> u16 {
        self.input.channels()
    }

    /// Samples per chunk for `chunk_duration_ms`, or an error when the device
    /// config makes that zero.
    pub fn chunk_size(&self, chunk_duration_ms: u64) -> Result<usize> {
        chunk_size_or_error(self.sample_rate(), self.channels(), chunk_duration_ms)
    }

    pub fn play(&self) -> Result<()> {
        self.stream.play()?;
        Ok(())
    }

    /// Drain one complete chunk if the buffer holds at least `chunk_size`
    /// samples; leftover samples stay buffered for the next call.
    pub fn next_chunk(&self, chunk_size: usize) -> Option<Vec<f32>> {
        let mut buffer = match self.buffer.lock() {
            Ok(buffer) => buffer,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !has_complete_chunk(buffer.len(), chunk_size) {
            return None;
        }
        drain_chunk(&mut buffer, chunk_size)
    }

    /// Drop everything currently buffered, partial samples included, and
    /// return how many samples were discarded.
    ///
    /// The half-duplex gate calls this the moment playback ends: audio
    /// captured while the player spoke is our own voice, and a leftover
    /// partial chunk would otherwise be glued onto the next utterance.
    /// Draining chunk by chunk cannot do this, because the drain stops as
    /// soon as the reverb tail is consumed and the rest stays queued.
    pub fn clear(&self) -> usize {
        let mut buffer = match self.buffer.lock() {
            Ok(buffer) => buffer,
            Err(poisoned) => poisoned.into_inner(),
        };
        let dropped = buffer.len();
        buffer.clear();
        dropped
    }
}

fn build_stream<T>(
    input: &DefaultInput,
    buffer: Arc<Mutex<Vec<f32>>>,
    error_prefix: &'static str,
) -> Result<cpal::Stream>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    let stream = input.device().build_input_stream(
        input.config(),
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            let mut buffer = match buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            for &sample in data {
                buffer.push(f32::from_sample(sample));
            }
        },
        move |err| eprintln!("{error_prefix} stream error: {err}"),
        None,
    )?;
    Ok(stream)
}
