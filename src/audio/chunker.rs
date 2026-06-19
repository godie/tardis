//! Real-time audio chunking test driver.
//!
//! Captures ~`seconds` of audio from the default mic, splits it into
//! `chunk_duration_ms`-millisecond chunks, and prints per-chunk stats:
//! chunk number, sample count, approximate duration, and average volume.
//!
//! Pure helpers (`calculate_chunk_size_samples`, `has_complete_chunk`,
//! `drain_chunk`) live in this module so they can be exercised by unit
//! tests without touching CPAL or any audio device.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use super::volume::calculate_average_volume;

// ===== Pure helpers (unit-tested) ========================================

/// Number of samples in one chunk of `chunk_duration_ms` at `sample_rate`
/// with `channels`. Multiplication is computed in `u128` to dodge the
/// 32-bit overflow that would happen at high rates × long windows.
pub fn calculate_chunk_size_samples(
    sample_rate: u32,
    channels: u16,
    chunk_duration_ms: u64,
) -> usize {
    let n = (sample_rate as u128) * (channels as u128) * (chunk_duration_ms as u128) / 1000;
    n as usize
}

/// Has the buffer accumulated at least `chunk_size` samples?
/// Returns false when `chunk_size == 0` so callers don't infinite-drain.
pub fn has_complete_chunk(buffer_len: usize, chunk_size: usize) -> bool {
    chunk_size > 0 && buffer_len >= chunk_size
}

/// Drain exactly `chunk_size` samples from the front of `buffer`.
/// Returns `None` if a complete chunk isn't available or `chunk_size == 0`.
/// Any leftover samples stay in the buffer.
pub fn drain_chunk(buffer: &mut Vec<f32>, chunk_size: usize) -> Option<Vec<f32>> {
    if chunk_size == 0 || buffer.len() < chunk_size {
        return None;
    }
    Some(buffer.drain(..chunk_size).collect())
}

// ===== CPAL-driven loop =================================================

/// Capture `seconds` of mic audio, splitting it into `chunk_duration_ms`
/// chunks and printing stats for each. Returns when the window ends.
pub fn run_chunk_test(seconds: u64, chunk_duration_ms: u64) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Chunk test device: {}", device);

    let supported = device.default_input_config()?;
    println!("Default input config: {:?}", supported);
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
        "Chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch)"
    );

    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config, Arc::clone(&buffer))?,
        other => return Err(anyhow!("Unsupported sample format: {:?}", other)),
    };

    stream.play()?;
    println!("Capturing for {seconds} seconds...");

    let start = Instant::now();
    let total_duration = Duration::from_secs(seconds);
    let tick = Duration::from_millis(chunk_duration_ms);
    let mut chunk_index: usize = 0;

    while start.elapsed() < total_duration {
        thread::sleep(tick);

        // Drain every complete chunk that accumulated during the tick so
        // we don't slowly fall behind when a callback delivers >1 chunk
        // worth of samples between sleeps.
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
            let Some(chunk) = drained else { break };

            chunk_index += 1;
            let samples = chunk.len();
            let approx_duration_ms = (samples as u128 * 1000)
                / ((sample_rate as u128) * (channels as u128));
            let avg = calculate_average_volume(&chunk);
            println!(
                "[chunk #{chunk_index:>3}] samples={samples:>6} ~{approx_duration_ms:>5} ms vol={avg:.3}"
            );
            // Flush so live monitors (piped to a file or remote sink)
            // see chunk lines as they arrive instead of in bursts.
            let _ = io::stdout().flush();
        }
    }

    drop(stream);
    println!("Chunk test finished. {chunk_index} chunks printed.");
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
    let err_fn = |err| eprintln!("chunker stream error: {err}");
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

// ===== Unit tests =======================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- calculate_chunk_size_samples ----------------------------------

    #[test]
    fn chunk_size_16k_mono_1000ms_is_16000() {
        assert_eq!(calculate_chunk_size_samples(16_000, 1, 1000), 16_000);
    }

    #[test]
    fn chunk_size_48k_stereo_1000ms_is_96000() {
        assert_eq!(calculate_chunk_size_samples(48_000, 2, 1000), 96_000);
    }

    #[test]
    fn chunk_size_48k_stereo_500ms_is_48000() {
        assert_eq!(calculate_chunk_size_samples(48_000, 2, 500), 48_000);
    }

    // ---- has_complete_chunk --------------------------------------------

    #[test]
    fn incomplete_buffer_returns_false() {
        assert!(!has_complete_chunk(100, 1_000));
    }

    #[test]
    fn exact_buffer_size_returns_true() {
        assert!(has_complete_chunk(1_000, 1_000));
    }

    #[test]
    fn larger_buffer_returns_true() {
        assert!(has_complete_chunk(1_500, 1_000));
    }

    #[test]
    fn zero_chunk_size_never_completes() {
        assert!(!has_complete_chunk(1_000, 0));
        assert!(!has_complete_chunk(0, 0));
    }

    // ---- drain_chunk ---------------------------------------------------

    #[test]
    fn drain_returns_none_if_not_enough_samples() {
        let mut buf: Vec<f32> = vec![0.0; 50];
        assert!(drain_chunk(&mut buf, 100).is_none());
        assert_eq!(buf.len(), 50); // unchanged
    }

    #[test]
    fn drain_returns_exact_chunk_size_from_front() {
        let mut buf: Vec<f32> = (0..200).map(|i| i as f32).collect();
        let drained = drain_chunk(&mut buf, 100).unwrap();
        assert_eq!(drained.len(), 100);
        assert_eq!(drained[0], 0.0);
        assert_eq!(drained[99], 99.0);
    }

    #[test]
    fn drain_leaves_remaining_samples_in_buffer() {
        let mut buf: Vec<f32> = (0..150).map(|i| i as f32).collect();
        let _ = drain_chunk(&mut buf, 100).unwrap();
        assert_eq!(buf.len(), 50);
        assert_eq!(buf[0], 100.0); // first remaining
        assert_eq!(buf[49], 149.0); // last remaining
    }

    #[test]
    fn drain_returns_none_if_chunk_size_zero() {
        let mut buf: Vec<f32> = vec![1.0; 100];
        assert!(drain_chunk(&mut buf, 0).is_none());
        assert_eq!(buf.len(), 100); // unchanged
    }
}
