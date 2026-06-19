//! Default microphone capture with simple volume activity logging.
//!
//! `start_default_mic_capture` builds the input stream, plays it, and returns
//! it. The caller holds the returned `Stream` to keep capture alive; dropping
//! it stops the stream.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

/// Minimum average absolute amplitude that triggers a volume log line.
const VOLUME_THRESHOLD: f32 = 0.01;
/// Minimum spacing between consecutive volume log lines from the audio callback.
const LOG_COOLDOWN: Duration = Duration::from_millis(100);

/// Starts capturing audio from the default microphone and returns the active
/// `Stream`. Drop the returned `Stream` to stop capture.
pub fn start_default_mic_capture() -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Mic device: {}", device);

    let supported = device.default_input_config()?;
    println!("Default input config: {:?}", supported);

    let config: StreamConfig = supported.into();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, &config)?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config)?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config)?,
        other => return Err(anyhow!("Unsupported sample format: {:?}", other)),
    };

    Ok(stream)
}

fn build_stream<T>(device: &cpal::Device, config: &StreamConfig) -> Result<cpal::Stream>
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    // `Instant::now()` is not a const fn, so this can't live in a module-level
    // `const`. Set the timestamp to (now - cooldown) so the first event above
    // the threshold is logged immediately.
    let last_log: Mutex<Instant> = Mutex::new(Instant::now() - LOG_COOLDOWN);

    let err_fn = |err| eprintln!("mic stream error: {}", err);

    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            let avg = average_amplitude(data);
            if avg < VOLUME_THRESHOLD {
                return;
            }
            if let Ok(mut last) = last_log.lock() {
                if last.elapsed() >= LOG_COOLDOWN {
                    println!("[mic] vol={:.3}", avg);
                    *last = Instant::now();
                }
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

/// Average absolute amplitude across all samples in the buffer.
/// Returns 0.0 for an empty buffer.
fn average_amplitude<T>(data: &[T]) -> f32
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    if data.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for &sample in data {
        sum += f32::from_sample(sample).abs();
    }
    sum / data.len() as f32
}
