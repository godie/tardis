//! Records audio from the default microphone into a 16-bit PCM WAV file.
//!
//! The audio callback pushes converted int16 samples into a shared
//! `Arc<Mutex<Vec<i16>>>` so the writer can drain it after capture ends.
//! The stream is explicitly dropped before the buffer is locked so the
//! callback cannot race with the file write.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use hound::{SampleFormat as HoundSampleFormat, WavSpec};

/// Records `seconds` of audio from the default microphone and writes a
/// 16-bit PCM WAV file to `output_path`. Creates any missing parent
/// directories. Channel count and sample rate mirror the device's default
/// input config.
pub fn record_default_mic_to_wav_for_seconds(seconds: u64, output_path: &str) -> Result<()> {
    // Ensure target directory exists. Skip when the caller passed a bare
    // filename (`create_dir_all("")` would fail on most platforms).
    let out_path = Path::new(output_path);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating output directory {:?}", parent))?;
        }
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Recording device: {}", device);

    let supported = device.default_input_config()?;
    println!("Default input config: {:?}", supported);
    let config: StreamConfig = supported.into();
    let sample_format = supported.sample_format();

    let samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));

    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, Arc::clone(&samples))?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config, Arc::clone(&samples))?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config, Arc::clone(&samples))?,
        other => return Err(anyhow!("Unsupported sample format: {:?}", other)),
    };

    stream.play()?;
    println!("Recording for {} seconds...", seconds);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        thread::sleep(Duration::from_millis(50));
    }
    // Explicit drop ensures no callback can fire once we take the buffer
    // lock for the WAV write below.
    drop(stream);
    println!("Recording finished.");

    let spec = WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate,
        bits_per_sample: 16,
        sample_format: HoundSampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output_path, spec)
        .with_context(|| format!("creating WAV at {output_path}"))?;

    let collected = samples
        .lock()
        .map_err(|e| anyhow!("poisoned sample buffer: {e}"))?;
    for &s in collected.iter() {
        writer.write_sample(s)?;
    }
    drop(collected);

    writer.finalize()?;
    println!("Saved WAV to {output_path}");
    Ok(())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    samples: Arc<Mutex<Vec<i16>>>,
) -> Result<cpal::Stream>
where
    T: Sample + SizedSample,
    i16: FromSample<T>,
{
    let err_fn = |err| eprintln!("recorder stream error: {err}");
    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            // Recover from poisoning rather than panicking — losing a small
            // slice of audio is preferable to aborting the capture.
            let mut buf = match samples.lock() {
                Ok(b) => b,
                Err(p) => p.into_inner(),
            };
            for &s in data {
                buf.push(i16::from_sample(s));
            }
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}
