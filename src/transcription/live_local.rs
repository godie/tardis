//! Live chunk-by-chunk local transcription pipeline.
//!
//! Captures audio from the default microphone, splits it into chunks,
//! classifies each chunk as silence or speech-like, and sends
//! speech-like chunks through a selected
//! [`crate::transcription::LocalTranscriptionProvider`] (writing each
//! chunk to a temporary WAV file first because the provider interface
//! is file-based).
//!
//! This is **not** true streaming — it is chunk-by-chunk live
//! transcription via temporary WAV files. The default provider is
//! `mock-local` so the command works without Docker; pass
//! `--provider local-whisper` to use the self-hosted faster-whisper
//! server instead.
//!
//! Pure helpers (`format_live_chunk_filename`,
//! `should_transcribe_chunk`, `live_transcription_status_message`)
//! live in this module so they can be unit-tested without touching
//! CPAL, Docker, or filesystem I/O.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use crate::app::events::AppTranscriptEvent;
use crate::audio::activity;
use crate::audio::chunk_recorder::write_chunk_wav;
use crate::audio::chunker::{calculate_chunk_size_samples, drain_chunk, has_complete_chunk};
use crate::audio::volume::calculate_average_volume;
use crate::config;
use crate::transcription;

// ===== Pure helpers (unit-tested) ========================================

/// Format a live-chunk WAV filename with a `live_` prefix and
/// zero-padded 3-digit index. Indices past 999 grow the width
/// naturally (e.g. `live_chunk_1234.wav`).
///
/// Examples:
/// - `1` → `"live_chunk_001.wav"`
/// - `12` → `"live_chunk_012.wav"`
/// - `123` → `"live_chunk_123.wav"`
pub fn format_live_chunk_filename(chunk_index: usize) -> String {
    format!("live_chunk_{chunk_index:03}.wav")
}

/// Should a chunk with the given `volume` be sent to the transcription
/// provider?
///
/// Delegates to [`crate::audio::activity::is_speech_like`]: returns
/// `true` only when `volume` is strictly above `threshold`.
/// Equal-to-threshold returns `false`.
pub fn should_transcribe_chunk(volume: f32, threshold: f32) -> bool {
    activity::is_speech_like(volume, threshold)
}

/// Format a readable status line containing the chunk index and a
/// human-readable status string. Callers append the result to their
/// log / console output.
///
/// Example: `"[chunk 2] speech-like"`
pub fn live_transcription_status_message(chunk_index: usize, status: &str) -> String {
    format!("[chunk {chunk_index}] {status}")
}

// ===== CPAL-driven runner ================================================

/// Run a live chunk-by-chunk local transcription test.
///
/// Opens the default microphone, captures for `seconds`, splits the
/// input into `chunk_duration_ms`-millisecond chunks, and routes each
/// speech-like chunk through the selected
/// [`crate::transcription::LocalTranscriptionProvider`].
///
/// # Arguments
///
/// * `provider_name` — value for the `--provider` flag (e.g.
///   `"mock-local"` or `"local-whisper"`). Parsed by
///   [`crate::transcription::build_provider`].
/// * `seconds` — total capture window, in seconds.
/// * `chunk_duration_ms` — per-chunk interval, in milliseconds.
///
/// # Errors
///
/// Returns an error if no default input device is available, the
/// sample format is unsupported, or the provider cannot be
/// constructed / contacted.
pub fn run_live_local_transcription_test(
    provider_name: &str,
    seconds: u64,
    chunk_duration_ms: u64,
) -> Result<()> {
    let output_dir = config::LIVE_CHUNKS_DIR;
    fs::create_dir_all(output_dir).with_context(|| format!("create output dir {output_dir}"))?;

    let provider = transcription::build_provider(provider_name)?;
    println!("provider: {}", provider.name());

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("device:  {device}");

    let supported = device.default_input_config()?;
    println!("config:  {supported:?}");
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
        "chunk:   {chunk_duration_ms} ms ({chunk_size} samples, {sample_rate} Hz, {channels} ch)"
    );
    println!("output:  {output_dir}/");
    println!("duration: {seconds} s\n");

    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config, Arc::clone(&buffer))?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config, Arc::clone(&buffer))?,
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    };

    stream.play()?;

    let start = Instant::now();
    let total_duration = Duration::from_secs(seconds);
    let tick = Duration::from_millis(chunk_duration_ms);
    let mut chunk_index: usize = 0;
    let threshold = config::DEFAULT_VOLUME_THRESHOLD;

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

            if !should_transcribe_chunk(avg_volume, threshold) {
                println!(
                    "{} detected, skipping...",
                    live_transcription_status_message(chunk_index, "silence")
                );
                let _ = io::stdout().flush();
                continue;
            }

            // Speech-like: write chunk to a temporary WAV, send to
            // the selected provider, print the result, then delete
            // the WAV.
            let filename = format_live_chunk_filename(chunk_index);
            let wav_path = Path::new(output_dir).join(&filename);

            write_chunk_wav(&wav_path, &samples, channels, sample_rate)
                .with_context(|| format!("write live chunk WAV {}", wav_path.display()))?;

            println!(
                "{} | saved {}",
                live_transcription_status_message(chunk_index, "speech-like"),
                wav_path.display(),
            );

            let wav_path_str = wav_path.to_string_lossy().to_string();
            match provider.transcribe(&wav_path_str) {
                Ok(text) => {
                    // Build a transcript event to demonstrate the future
                    // Tauri event shape — same fields the app layer will
                    // emit once the Tauri shell wires into live capture.
                    let _event = AppTranscriptEvent {
                        chunk_index,
                        text: text.clone(),
                        provider: provider_name.to_string(),
                        is_final: true,
                    };
                    println!(
                        "{}provider={} transcript=\"{}\"",
                        live_transcription_status_message(chunk_index, ""),
                        provider_name,
                        text,
                    );
                }
                Err(e) => {
                    eprintln!(
                        "{} transcription error: {e}",
                        live_transcription_status_message(chunk_index, "error"),
                    );
                }
            }
            let _ = io::stdout().flush();

            // Delete the temporary WAV after successful or failed
            // transcription — the chunk has been consumed.
            if let Err(e) = fs::remove_file(&wav_path) {
                eprintln!(
                    "{} warning: could not delete temp WAV {}: {e}",
                    live_transcription_status_message(chunk_index, "cleanup"),
                    wav_path.display(),
                );
            }
        }
    }

    drop(stream);
    println!("\nLive local transcription finished. {chunk_index} chunks processed.");

    // Clean up the output directory if empty.
    let _ = fs::remove_dir(output_dir);

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
    let err_fn = |err| eprintln!("live-local stream error: {err}");
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

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- format_live_chunk_filename ------------------------------------

    #[test]
    fn format_live_chunk_filename_001() {
        assert_eq!(format_live_chunk_filename(1), "live_chunk_001.wav");
    }

    #[test]
    fn format_live_chunk_filename_012() {
        assert_eq!(format_live_chunk_filename(12), "live_chunk_012.wav");
    }

    #[test]
    fn format_live_chunk_filename_123() {
        assert_eq!(format_live_chunk_filename(123), "live_chunk_123.wav");
    }

    // ---- should_transcribe_chunk ---------------------------------------

    #[test]
    fn should_transcribe_below_threshold_returns_false() {
        assert!(!should_transcribe_chunk(0.005_f32, 0.01_f32));
    }

    #[test]
    fn should_transcribe_equal_to_threshold_returns_false() {
        assert!(!should_transcribe_chunk(0.01_f32, 0.01_f32));
    }

    #[test]
    fn should_transcribe_above_threshold_returns_true() {
        assert!(should_transcribe_chunk(0.02_f32, 0.01_f32));
    }

    // ---- live_transcription_status_message -----------------------------

    #[test]
    fn status_message_contains_chunk_index() {
        let msg = live_transcription_status_message(5, "speech-like");
        assert!(
            msg.contains("5"),
            "message should contain chunk index 5, got: {msg}"
        );
    }

    #[test]
    fn status_message_contains_status() {
        let msg = live_transcription_status_message(3, "silence");
        assert!(
            msg.contains("silence"),
            "message should contain status 'silence', got: {msg}"
        );
    }

    #[test]
    fn status_message_is_readable() {
        let msg = live_transcription_status_message(2, "speech-like");
        assert!(
            msg.starts_with("[chunk "),
            "message should start with '[chunk ', got: {msg}"
        );
    }
}
