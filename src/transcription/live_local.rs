//! Live chunk-by-chunk local transcription pipeline.
//!
//! Captures audio from the default microphone, splits it into chunks,
//! classifies each chunk as silence or speech-like, and sends
//! speech-like chunks through a selected
//! [`crate::transcription::LocalTranscriptionProvider`] (writing each
//! chunk to a temporary WAV file first because the provider interface
//! is file-based).
//!
//! This is **not** true streaming: it is chunk-by-chunk live
//! transcription via temporary WAV files. The default provider is
//! `mock-local` so the command works without Docker; pass
//! `--provider local-whisper` to use the self-hosted faster-whisper
//! server instead.
//!
//! Two entry points share the same underlying capture loop:
//!
//! - [`run_live_local_transcription_with_config_and_events`] — the
//!   canonical driver. Accepts a fully-formed
//!   [`AppRuntimeConfig`] so the Tauri shell's UI-supplied
//!   provider / language pair / chunk size / threshold flow
//!   through end-to-end. Translation uses
//!   `config.source_language` / `config.target_language` (no
//!   hardcoded en/es).
//! - [`run_live_local_transcription_with_events`] — back-compat
//!   thin wrapper that builds an `AppRuntimeConfig` from
//!   individual CLI args (defaulting the language pair and
//!   threshold from centralised [`config`] constants). Used by
//!   `cargo run -- live-local-transcribe --provider mock-local`.
//!
//! Pure helpers (`format_live_chunk_filename`,
//! `should_transcribe_chunk`, `live_transcription_status_message`)
//! live in this module so they can be unit-tested without touching
//! CPAL, Docker, or filesystem I/O.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::app::config::AppRuntimeConfig;
use crate::app::events::{AppEvent, AppStatus, format_app_event_for_console};
use crate::app::live_events::{build_error_event, build_transcript_event, build_translation_event};
use crate::audio::activity;
use crate::audio::capture::MicInput;
use crate::audio::chunk_recorder::write_chunk_wav;
use crate::audio::volume::calculate_average_volume;
use crate::config;
use crate::transcription;
use crate::translation::mock::MockTranslator;
use crate::translation::translator::Translator;

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

/// Run live chunk-by-chunk local transcription, routing each
/// [`AppEvent`] through an external callback.
///
/// This is the canonical driver of the live pipeline; the
/// per-arg [`run_live_local_transcription_with_events`] is now
/// a back-compat shim around this function for the CLI.
///
/// # Arguments
///
/// * `config` — a fully-formed [`AppRuntimeConfig`]. The
///   caller is responsible for running
///   [`crate::app::config::normalize_runtime_config`] and
///   [`crate::app::config::validate_runtime_config`] before
///   calling here; this function does NOT revalidate to avoid
///   double work, but a malformed config surfaces downstream
///   as a provider error or chunk-size error and the help
///   message is informative. The Tauri command validates ahead
///   of state mutation; the CLI's wrapper builds the config
///   from its defaulted fields.
/// * `seconds` — total capture window, in seconds. `Some(n)`
///   means "exit after `n` seconds if no stop signal arrives
///   sooner." `None` means "run until `should_stop` is set."
///   When `should_stop` is `Some`, the timer is bypassed (the
///   existing CLI / Tauri contract) so a long-running Tauri
///   session can pass `None` while a finite CLI smoke test
///   passes `Some(10)`.
/// * `should_stop` — optional shared stop flag. When `Some`,
///   the loop checks this flag each tick and exits cleanly
///   when it becomes `true`. When `None`, the loop exits via
///   the elapsed-time check (which is itself gated by
///   `seconds`).
/// * `on_event` — callback invoked for every [`AppEvent`] the
///   runner produces (`StatusChanged`, `Transcript`,
///   `Translation`, `Error`). Must be `Send + Sync + 'static`
///   so it can be called from the capture thread when invoked
///   from a Tauri context where the closure's captured
///   `AppHandle` is also `Send + Sync`.
pub fn run_live_local_transcription_with_config_and_events<F>(
    config: AppRuntimeConfig,
    seconds: Option<u64>,
    should_stop: Option<Arc<AtomicBool>>,
    on_event: F,
) -> Result<()>
where
    F: Fn(AppEvent) + Send + Sync + 'static,
{
    let provider_name = config.transcription_provider.clone();

    let output_dir = config::LIVE_CHUNKS_DIR;
    fs::create_dir_all(output_dir).with_context(|| format!("create output dir {output_dir}"))?;

    let provider = transcription::build_provider(&provider_name)?;
    println!("provider: {}", provider.name());

    let mic = MicInput::start("live-local")?;
    println!("device:  {}", mic.input().device_label());
    println!("config:  {}", mic.input().supported_label());
    let sample_rate = mic.sample_rate();
    let channels = mic.channels();
    let chunk_size = mic.chunk_size(config.chunk_duration_ms)?;
    println!(
        "chunk:   {} ms ({chunk_size} samples, {sample_rate} Hz, {channels} ch)",
        config.chunk_duration_ms
    );
    println!("output:  {output_dir}/");
    match seconds {
        Some(s) => println!("duration: {s} s\n"),
        None => println!("duration: until stopped\n"),
    }

    mic.play()?;

    // Emit the first event: capture has started.
    on_event(AppEvent::StatusChanged(AppStatus::Listening));

    let start = Instant::now();
    let total_duration = seconds.map(Duration::from_secs);
    let tick = Duration::from_millis(config.chunk_duration_ms);
    let mut chunk_index: usize = 0;
    let threshold = config.volume_threshold;
    let translator = MockTranslator::new();
    let source_lang = config.source_language.clone();
    let target_lang = config.target_language.clone();
    let has_stop_signal = should_stop.is_some();

    loop {
        // Exit conditions: either the timer expires (CLI mode) or
        // the stop signal was set (Tauri mode). When `seconds` is
        // `None`, the time path is skipped and only the stop signal
        // can break the loop; when `should_stop` is `Some`, the
        // time path is bypassed so a long-running Tauri session
        // with both parameters exits exclusively on the stop flag.
        if !has_stop_signal {
            if let Some(d) = total_duration {
                if start.elapsed() >= d {
                    break;
                }
            }
        }
        if let Some(ref flag) = should_stop {
            if flag.load(Ordering::Relaxed) {
                break;
            }
        }

        thread::sleep(tick);

        while let Some(samples) = mic.next_chunk(chunk_size) {
            chunk_index += 1;
            let avg_volume = calculate_average_volume(&samples);

            if !should_transcribe_chunk(avg_volume, threshold) {
                // Silence chunks are not events; they would spam
                // the event stream. Print directly for CLI; skip
                // entirely for Tauri (the UI shows the last
                // transcript/translation, not a silence log).
                if !has_stop_signal {
                    println!(
                        "{} detected, skipping...",
                        live_transcription_status_message(chunk_index, "silence")
                    );
                    let _ = io::stdout().flush();
                }
                continue;
            }

            // Write speech-like chunk to a temporary WAV.
            let filename = format_live_chunk_filename(chunk_index);
            let wav_path = Path::new(output_dir).join(&filename);

            write_chunk_wav(&wav_path, &samples, channels, sample_rate)
                .with_context(|| format!("write live chunk WAV {}", wav_path.display()))?;

            if !has_stop_signal {
                println!(
                    "{} | saved {}",
                    live_transcription_status_message(chunk_index, "speech-like"),
                    wav_path.display(),
                );
            }

            // Transcribe through the selected provider.
            let wav_path_str = wav_path.to_string_lossy().to_string();
            match provider.transcribe(&wav_path_str) {
                Ok(text) => {
                    // Transcript event. Uses the configured
                    // provider name verbatim so the UI displays
                    // exactly what the user selected.
                    on_event(build_transcript_event(
                        chunk_index,
                        text.clone(),
                        provider_name.clone(),
                        true,
                    ));

                    // Mock translation uses config.source_language /
                    // config.target_language so the user-selected
                    // language pair reaches the translation panel,
                    // not the hardcoded en/es.
                    if let Some(tr) = translator.translate_text(&text, &source_lang, &target_lang) {
                        on_event(build_translation_event(
                            chunk_index,
                            tr.source_text,
                            tr.translated_text,
                            tr.source_language,
                            tr.target_language,
                            tr.is_final,
                        ));
                    }
                }
                Err(e) => {
                    on_event(build_error_event(format!(
                        "[chunk {chunk_index}] transcription error: {e}"
                    )));
                }
            }

            let _ = io::stdout().flush();

            // Delete the temporary WAV.
            if let Err(e) = fs::remove_file(&wav_path) {
                eprintln!(
                    "{} warning: could not delete temp WAV {}: {e}",
                    live_transcription_status_message(chunk_index, "cleanup"),
                    wav_path.display(),
                );
            }
        }
    }

    drop(mic);

    // Emit the final event: capture has stopped.
    on_event(AppEvent::StatusChanged(AppStatus::Stopped));
    println!("\nLive local transcription finished. {chunk_index} chunks processed.");

    // Clean up the output directory if empty.
    let _ = fs::remove_dir(output_dir);

    Ok(())
}

/// Back-compat wrapper around
/// [`run_live_local_transcription_with_config_and_events`] for the
/// CLI's `--provider` flow.
///
/// Builds an [`AppRuntimeConfig`] from individual `&str` / `u64`
/// args, defaulting the language pair and threshold from the
/// centralised [`crate::config`] constants so the CLI's
/// behaviour is unchanged when called via
/// `cargo run -- live-local-transcribe --provider mock-local`.
/// Keep this signature stable: the CLI in `src/main.rs` calls
/// it directly. The Tauri shell never reaches this wrapper; it
/// uses the config-based function above so the UI can pass a
/// full [`AppRuntimeConfig`].
pub fn run_live_local_transcription_with_events<F>(
    provider_name: &str,
    seconds: u64,
    chunk_duration_ms: u64,
    should_stop: Option<Arc<AtomicBool>>,
    on_event: F,
) -> Result<()>
where
    F: Fn(AppEvent) + Send + Sync + 'static,
{
    let config = AppRuntimeConfig {
        transcription_provider: provider_name.to_string(),
        chunk_duration_ms,
        source_language: config::DEFAULT_SOURCE_LANGUAGE.to_string(),
        target_language: config::DEFAULT_TARGET_LANGUAGE.to_string(),
        volume_threshold: config::DEFAULT_VOLUME_THRESHOLD,
    };
    run_live_local_transcription_with_config_and_events(
        config,
        Some(seconds),
        should_stop,
        on_event,
    )
}

/// CLI-facing wrapper that delegates to
/// [`run_live_local_transcription_with_events`] with a
/// console-formatting callback and no stop signal.
///
/// Preserves the exact same output behaviour the CLI expects.
pub fn run_live_local_transcription_test(
    provider_name: &str,
    seconds: u64,
    chunk_duration_ms: u64,
) -> Result<()> {
    run_live_local_transcription_with_events(
        provider_name,
        seconds,
        chunk_duration_ms,
        None,
        |event| {
            println!("{}", format_app_event_for_console(&event));
        },
    )
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
