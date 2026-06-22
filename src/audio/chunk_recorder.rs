//! Per-chunk WAV recorder.
//!
//! Captures ~`seconds` from the default mic, splits the buffer into
//! `chunk_duration_ms` chunks via the shared `audio::chunker` helpers,
//! converts each f32 chunk to 16-bit Int PCM, and writes a separate
//! WAV file per chunk into `output_dir` (created if missing).
//!
//! Pure helpers (`format_chunk_filename`, `f32_sample_to_i16`,
//! `f32_samples_to_i16`) live next to the CPAL driver so the formatting
//! and conversion math can be unit-tested without touching real
//! hardware.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use hound::{SampleFormat as HoundSampleFormat, WavSpec, WavWriter};

use crate::audio::chunker::{calculate_chunk_size_samples, drain_chunk, has_complete_chunk};
use crate::audio::volume::calculate_average_volume;
use crate::config::WAV_BITS_PER_SAMPLE;

// ---- pure helpers ----------------------------------------------------------

/// Zero-padded chunk filename. Minimum width is 3 digits; indices past
/// 999 grow the width naturally (e.g. `chunk_1234.wav`).
pub fn format_chunk_filename(chunk_index: usize) -> String {
    format!("chunk_{chunk_index:03}.wav")
}

/// Convert a single f32 sample (assumed in -1.0..=1.0) to i16, clamping
/// out-of-range inputs.
///
/// Boundary values are exact: `±1.0` map directly to `i16::MAX` /
/// `i16::MIN` rather than `±32767` (the multiplicative round
/// alternative would give -32767 at exact -1.0). Out-of-range samples
/// (`±2.0`) clamp to the same extremes.
pub fn f32_sample_to_i16(sample: f32) -> i16 {
    if sample >= 1.0 {
        return i16::MAX;
    }
    if sample <= -1.0 {
        return i16::MIN;
    }
    (sample * i16::MAX as f32) as i16
}

/// Convert a slice of f32 samples to i16 via [`f32_sample_to_i16`].
/// Length is preserved; sign and order are preserved per-sample since
/// the in-range conversion is a monotone 1:1 map.
pub fn f32_samples_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|&s| f32_sample_to_i16(s)).collect()
}

// ---- CPAL driver -----------------------------------------------------------

/// Capture `seconds` from the default mic, write one WAV file per
/// completed chunk of size `chunk_duration_ms` into `output_dir`.
/// Mirrors channels + sample rate from the device's default input
/// config into each WAV header.
pub fn run_save_chunks_test(seconds: u64, chunk_duration_ms: u64, output_dir: &str) -> Result<()> {
    fs::create_dir_all(output_dir).with_context(|| format!("create output dir {output_dir}"))?;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Chunk recorder device: {device}");

    let supported = device.default_input_config()?;
    println!("Default input config: {supported:?}");
    let config: StreamConfig = supported.into();
    let sample_format = supported.sample_format();
    let sample_rate = config.sample_rate;
    let channels = config.channels;
    let chunk_size = calculate_chunk_size_samples(sample_rate, channels, chunk_duration_ms);
    if chunk_size == 0 {
        return Err(anyhow!(
            "computed chunk_size is 0 (sample_rate={sample_rate}, channels={channels}, \
             chunk_duration_ms={chunk_duration_ms})"
        ));
    }
    println!(
        "Chunk recorder: chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch); \
         saving to {output_dir}/"
    );

    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config, Arc::clone(&buffer))?,
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    };

    stream.play()?;
    println!("Capturing for {seconds} seconds...");

    let start = Instant::now();
    let total_duration = Duration::from_secs(seconds);
    let tick = Duration::from_millis(chunk_duration_ms);
    let mut chunk_index: usize = 0;

    while start.elapsed() < total_duration {
        thread::sleep(tick);

        loop {
            let drained: Option<Vec<f32>> = {
                let mut buf = match buffer.lock() {
                    Ok(b) => b,
                    Err(p) => p.into_inner(),
                };
                if !has_complete_chunk(buf.len(), chunk_size) {
                    None
                } else {
                    drain_chunk(&mut buf, chunk_size)
                }
            };
            let Some(samples) = drained else { break };

            chunk_index += 1;
            let avg_volume = calculate_average_volume(&samples);
            let filename = format_chunk_filename(chunk_index);
            let path = Path::new(output_dir).join(&filename);
            write_chunk_wav(&path, &samples, channels, sample_rate)?;

            println!(
                "[chunk {chunk_index}] saved to {} | samples: {chunk_size} | volume: {avg_volume:.4}",
                path.display(),
            );
            let _ = io::stdout().flush();
        }
    }

    drop(stream);
    println!("Chunk recorder finished. {chunk_index} chunks written to {output_dir}/");
    Ok(())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    buffer: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream>
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    let err_fn = |err| eprintln!("chunk recorder stream error: {err}");
    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            let mut buf = match buffer.lock() {
                Ok(b) => b,
                Err(p) => p.into_inner(),
            };
            for &s in data {
                buf.push(f32::from_sample(s));
            }
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

/// Write a slice of f32 samples to a 16-bit Int PCM WAV file at `path`.
/// Reused by chunk recorders and live-transcription pipelines.
pub fn write_chunk_wav(
    path: &Path,
    samples_f32: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Result<()> {
    let spec = WavSpec {
        channels: channels as u16,
        sample_rate,
        bits_per_sample: WAV_BITS_PER_SAMPLE,
        sample_format: HoundSampleFormat::Int,
    };
    let mut writer =
        WavWriter::create(path, spec).with_context(|| format!("create {}", path.display()))?;

    let i16_samples = f32_samples_to_i16(samples_f32);
    for s in i16_samples {
        writer
            .write_sample(s)
            .with_context(|| format!("write sample to {}", path.display()))?;
    }
    writer
        .finalize()
        .with_context(|| format!("finalize {}", path.display()))?;
    Ok(())
}

// ---- unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- format_chunk_filename -----------------------------------------

    #[test]
    fn format_chunk_filename_001() {
        assert_eq!(format_chunk_filename(1), "chunk_001.wav");
    }

    #[test]
    fn format_chunk_filename_012() {
        assert_eq!(format_chunk_filename(12), "chunk_012.wav");
    }

    #[test]
    fn format_chunk_filename_123() {
        assert_eq!(format_chunk_filename(123), "chunk_123.wav");
    }

    // ---- f32_sample_to_i16 ---------------------------------------------

    #[test]
    fn f32_sample_to_i16_zero_maps_to_zero() {
        assert_eq!(f32_sample_to_i16(0.0), 0);
    }

    #[test]
    fn f32_sample_to_i16_one_maps_to_max() {
        assert_eq!(f32_sample_to_i16(1.0), i16::MAX);
    }

    #[test]
    fn f32_sample_to_i16_neg_one_maps_to_min() {
        assert_eq!(f32_sample_to_i16(-1.0), i16::MIN);
    }

    #[test]
    fn f32_sample_to_i16_positive_overflow_clamps() {
        assert_eq!(f32_sample_to_i16(2.0), i16::MAX);
    }

    #[test]
    fn f32_sample_to_i16_negative_overflow_clamps() {
        assert_eq!(f32_sample_to_i16(-2.0), i16::MIN);
    }

    // ---- f32_samples_to_i16 --------------------------------------------

    #[test]
    fn f32_samples_to_i16_preserves_length() {
        let input = vec![-0.5_f32, 0.0, 0.5];
        let output = f32_samples_to_i16(&input);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn f32_samples_to_i16_preserves_order_and_sign() {
        let input = vec![-1.0_f32, -0.5, 0.0, 0.5, 1.0];
        let output = f32_samples_to_i16(&input);
        assert_eq!(output.len(), 5);
        // Negative samples stay negative.
        assert!(output[0] < 0, "expected output[0] < 0, got {}", output[0]);
        assert!(output[1] < 0, "expected output[1] < 0, got {}", output[1]);
        // Zero maps to zero.
        assert_eq!(output[2], 0);
        // Positive samples stay positive.
        assert!(output[3] > 0, "expected output[3] > 0, got {}", output[3]);
        assert!(output[4] > 0, "expected output[4] > 0, got {}", output[4]);
        // Boundary values are exact.
        assert_eq!(output[0], i16::MIN);
        assert_eq!(output[4], i16::MAX);
    }
}
