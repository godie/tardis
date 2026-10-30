//! Live utterance-by-utterance local transcription pipeline.
//!
//! Captures audio from the default microphone in fixed windows, classifies
//! each window as silence or speech-like, and runs the windows through
//! [`crate::audio::endpoint`] so whole utterances — not clock chunks — reach
//! the provider. Silence only closes an utterance (after `hangover_ms`); it
//! never becomes a provider call. Each utterance is written to a temporary
//! WAV file because the provider interface is file-based. Each transcript is
//! then processed, synthesized by [`crate::synthesis::MockSynthesizer`], and
//! handed to [`crate::audio::player::CountingPlayer`], which counts instead
//! of playing: the whole five-stage loop runs with no TTS service, no output
//! device, and no permissions. It counts rather than records because an
//! indefinite session would otherwise hold every rendered sample in memory.
//!
//! This is **not** true streaming: it is utterance-by-utterance live
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
//! Pure helpers (`format_live_utterance_filename`,
//! `should_transcribe_chunk`, `live_utterance_status_message`) live in this
//! module so they can be unit-tested without touching CPAL, Docker, or
//! filesystem I/O.

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
use crate::app::live_events::{
    build_error_event, build_output_event, build_speech_event, build_transcript_event,
};
use crate::audio::activity;
use crate::audio::capture::MicInput;
use crate::audio::chunk_recorder::write_chunk_wav;
use crate::audio::endpoint::{self, EndpointConfig, EndpointDecision, EndpointState};
use crate::audio::gate::{HalfDuplexGate, playback_backlog};
use crate::audio::player::{CountingPlayer, Player};
use crate::audio::volume::calculate_average_volume;
use crate::config;
use crate::processor::processor::{ProcessMode, Processor};
use crate::processor::{DEFAULT_PROCESSOR_NAME, build_processor};
use crate::synthesis::{AudioChunk, MockSynthesizer, Speech, Synthesizer, chunk_duration_ms};
use crate::transcription;
use crate::transcription::LocalTranscriptionProvider;

// ===== Pure helpers (unit-tested) ========================================

/// Format a live-utterance WAV filename with a `live_` prefix and
/// zero-padded 3-digit index. Indices past 999 grow the width
/// naturally (e.g. `live_utterance_1234.wav`).
///
/// Examples:
/// - `1` → `"live_utterance_001.wav"`
/// - `12` → `"live_utterance_012.wav"`
/// - `123` → `"live_utterance_123.wav"`
pub fn format_live_utterance_filename(utterance_index: usize) -> String {
    format!("live_utterance_{utterance_index:03}.wav")
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

/// Format a readable status line containing the utterance index and a
/// human-readable status string. Callers append the result to their
/// log / console output.
///
/// Example: `"[utterance 2] speech-like"`
pub fn live_utterance_status_message(utterance_index: usize, status: &str) -> String {
    format!("[utterance {utterance_index}] {status}")
}

/// What to do with the audio still buffered when capture ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopFlush {
    /// Close the open utterance and run it through the stages as a turn.
    Speak { frames: usize },
    /// Nothing worth keeping: drop the buffered samples.
    Drop,
}

/// Decide what happens to the pending utterance when capture stops.
///
/// Capture can be interrupted mid-utterance (the stop signal, the CLI
/// timer), and the samples buffered for it are real user audio: dropping
/// them loses the last turn. That flush cannot be covered by a hardware-free
/// test while it lives inside the CPAL loop, so the decision is pure here and
/// the runner only acts on it. [`endpoint::finish`] owns the timing rule
/// (`min_speech_ms`); this adds the "is there any audio at all" guard, so an
/// open utterance holding zero samples never reaches the provider.
pub fn stop_flush(
    state: &EndpointState,
    config: &EndpointConfig,
    pending_samples: usize,
) -> StopFlush {
    match endpoint::finish(state, config) {
        EndpointDecision::Utterance { frames } if pending_samples > 0 => {
            StopFlush::Speak { frames }
        }
        EndpointDecision::Utterance { .. }
        | EndpointDecision::Discard
        | EndpointDecision::Continue => StopFlush::Drop,
    }
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
///   `Translation`, `Speech`, `Error`). Must be `Send + Sync + 'static`
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
    // What the hardware reports, before the stream folds it down to mono.
    // BlackHole 16ch/64ch say 16/64 here while the buffer holds one channel.
    let device_channels = mic.device_channels();
    println!("input:   {device_channels} ch -> mono (channels carrying energy)");
    let chunk_size = mic.chunk_size(config.chunk_duration_ms)?;
    println!(
        "chunk:   {} ms ({chunk_size} samples, {sample_rate} Hz, {channels} ch)",
        config.chunk_duration_ms
    );
    // Endpointing is frame-counted, so report what the configured values
    // actually resolve to: a 400 ms hangover on 1 s windows closes after a
    // full window of silence, and the log should say so.
    println!(
        "utter.:  closes after {} ms of silence ({} frame(s)), needs {} ms of speech ({} frame(s)), cap {} ms ({} frame(s))",
        config.hangover_ms,
        endpoint::frames_for_ms(config.hangover_ms, config.chunk_duration_ms),
        config.min_speech_ms,
        endpoint::frames_for_ms(config.min_speech_ms, config.chunk_duration_ms),
        config.max_utterance_ms,
        endpoint::frames_for_ms(config.max_utterance_ms, config.chunk_duration_ms),
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
    let mut utterance_index: usize = 0;
    let threshold = config.volume_threshold;
    // Endpointing turns fixed capture windows into whole utterances, so the
    // provider sees one call per turn instead of one per clock chunk.
    let endpoint_config = EndpointConfig {
        chunk_duration_ms: config.chunk_duration_ms,
        hangover_ms: config.hangover_ms,
        min_speech_ms: config.min_speech_ms,
        max_utterance_ms: config.max_utterance_ms,
    };
    let mut endpoint_state = EndpointState::new();
    let mut pending_samples: Vec<f32> = Vec::new();
    // The process stage is built, not constructed inline: the same
    // `build_processor` dispatch the service uses, so a real processor
    // slots in without touching this loop.
    let processor = build_processor(DEFAULT_PROCESSOR_NAME)?;
    let process_mode = ProcessMode::Translate {
        source_language: config.source_language.clone(),
        target_language: config.target_language.clone(),
    };
    let synthesizer = MockSynthesizer::new();
    let mut player = CountingPlayer::new();
    let mut gate = HalfDuplexGate::with_default_tail();
    // Duration of one drained/captured frame batch, for the gate's tail.
    let captured_ms = chunk_size as u64 * 1_000 / (sample_rate as u64 * channels as u64);
    let target_lang = config.target_language.clone();
    let has_stop_signal = should_stop.is_some();

    // Playback plumbing and the per-utterance stages are grouped so the loop
    // can flush an utterance both mid-session and at teardown without
    // repeating the stage sequence.
    let mut speaker = Speaker {
        player: &mut player,
        gate: &mut gate,
        mic: &mic,
        chunk_size,
        tick,
        captured_ms,
    };
    let stages = UtteranceStages {
        provider: provider.as_ref(),
        provider_name: &provider_name,
        processor: processor.as_ref(),
        process_mode: &process_mode,
        synthesizer: &synthesizer,
        on_event: &on_event,
        output_dir,
        sample_rate,
        channels,
        chunk_duration_ms: config.chunk_duration_ms,
        target_language: &target_lang,
        has_stop_signal,
    };

    loop {
        // Exit conditions: either the timer expires (CLI mode) or
        // the stop signal was set (Tauri mode). When `seconds` is
        // `None`, the time path is skipped and only the stop signal
        // can break the loop; when `should_stop` is `Some`, the
        // time path is bypassed so a long-running Tauri session
        // with both parameters exits exclusively on the stop flag.
        if !has_stop_signal
            && let Some(d) = total_duration
            && start.elapsed() >= d
        {
            break;
        }
        if let Some(ref flag) = should_stop
            && flag.load(Ordering::Relaxed)
        {
            break;
        }

        thread::sleep(tick);

        while let Some(samples) = mic.next_chunk(chunk_size) {
            let avg_volume = calculate_average_volume(&samples);
            let is_speech = should_transcribe_chunk(avg_volume, threshold);

            // The frame that closes an utterance belongs to it, so buffer
            // before deciding. A non-speech frame outside an utterance is
            // never buffered: silence alone must not accumulate.
            if endpoint_state.is_open() || is_speech {
                pending_samples.extend_from_slice(&samples);
            }

            match endpoint::advance(&mut endpoint_state, is_speech, &endpoint_config) {
                EndpointDecision::Continue => {}
                EndpointDecision::Discard => {
                    // A blip below `min_speech_ms` is noise: no event, no
                    // WAV, no provider call. Print once so the CLI is not
                    // silent about having thrown audio away.
                    pending_samples.clear();
                    if !has_stop_signal {
                        println!(
                            "{} discarded (below {} ms of speech)",
                            live_utterance_status_message(utterance_index + 1, "burst"),
                            endpoint_config.min_speech_ms,
                        );
                        let _ = io::stdout().flush();
                    }
                }
                EndpointDecision::Utterance { frames } => {
                    utterance_index += 1;
                    let utterance = std::mem::take(&mut pending_samples);
                    if stages.run(utterance_index, frames, &utterance, &mut speaker)? {
                        // Playback happened: the audio captured meanwhile was
                        // our own voice, so an open utterance cannot span it.
                        endpoint_state.reset();
                        pending_samples.clear();
                    }
                }
            }
        }
    }

    // Capture can stop mid-utterance (the stop signal, the CLI timer). Flush
    // what was captured so the last turn is not silently dropped.
    match stop_flush(&endpoint_state, &endpoint_config, pending_samples.len()) {
        StopFlush::Speak { frames } => {
            utterance_index += 1;
            let utterance = std::mem::take(&mut pending_samples);
            stages.run(utterance_index, frames, &utterance, &mut speaker)?;
        }
        StopFlush::Drop => pending_samples.clear(),
    }

    drop(mic);

    // Emit the final event: capture has stopped.
    on_event(AppEvent::StatusChanged(AppStatus::Stopped));
    println!(
        "\nLive local transcription finished. {utterance_index} utterance(s) processed, {} spoken ({} ms, mock player).",
        player.played_count(),
        player.played_duration_ms(),
    );

    // Clean up the output directory if empty.
    let _ = fs::remove_dir(output_dir);

    Ok(())
}

/// Playback plumbing for one session: the half-duplex gate, the player, and
/// the capture buffer they clear.
///
/// Grouped so the per-utterance stages can speak without owning the loop's
/// state (the capture loop keeps the endpointing state itself).
struct Speaker<'a> {
    player: &'a mut CountingPlayer,
    gate: &'a mut HalfDuplexGate,
    mic: &'a MicInput,
    chunk_size: usize,
    tick: Duration,
    captured_ms: u64,
}

impl Speaker<'_> {
    /// Speak one synthesized chunk, then drop the audio captured meanwhile.
    ///
    /// The result is returned rather than handled here: a failed playback
    /// must not end the session, so the caller reports it and keeps
    /// transcribing.
    fn speak(
        &mut self,
        utterance_index: usize,
        chunk: &AudioChunk,
        has_stop_signal: bool,
    ) -> Result<()> {
        // Half duplex: stop listening while we speak. Playback blocks, so
        // the ring buffer fills with our own voice; clearing it drops that
        // whole backlog at once, partial chunk included.
        self.gate.begin_speaking();
        let playback = self.player.play_chunk(chunk);
        self.gate.end_speaking();
        let dropped = self.mic.clear();

        let backlog = playback_backlog(dropped, self.chunk_size);
        if !has_stop_signal && backlog.requires_clear() {
            println!(
                "{} dropped buffered playback audio ({} chunk(s), {} partial sample(s))",
                live_utterance_status_message(utterance_index, "half-duplex"),
                backlog.chunks,
                backlog.partial_samples,
            );
        }

        // Only the reverb tail is left to discard.
        discard_blocked_capture(
            self.mic,
            self.chunk_size,
            self.tick,
            self.captured_ms,
            self.gate,
        );
        playback
    }
}

/// The stages every closed utterance goes through: one WAV, one provider
/// call, one process call, one synthesis, one playback.
///
/// Owns nothing; the runner owns the handles. Extracted so the capture loop
/// can flush an utterance both mid-session and at teardown without repeating
/// the sequence.
struct UtteranceStages<'a, F> {
    provider: &'a dyn LocalTranscriptionProvider,
    provider_name: &'a str,
    processor: &'a dyn Processor,
    process_mode: &'a ProcessMode,
    synthesizer: &'a dyn Synthesizer,
    on_event: &'a F,
    output_dir: &'a str,
    sample_rate: u32,
    channels: u16,
    chunk_duration_ms: u64,
    target_language: &'a str,
    has_stop_signal: bool,
}

impl<F> UtteranceStages<'_, F>
where
    F: Fn(AppEvent) + Send + Sync + 'static,
{
    /// Run the whole utterance through the pipeline.
    ///
    /// Returns `true` when audio was played, which tells the caller to drop
    /// any utterance that opened while we were speaking.
    fn run(
        &self,
        utterance_index: usize,
        frames: usize,
        samples: &[f32],
        speaker: &mut Speaker<'_>,
    ) -> Result<bool> {
        let filename = format_live_utterance_filename(utterance_index);
        let wav_path = Path::new(self.output_dir).join(&filename);

        write_chunk_wav(&wav_path, samples, self.channels, self.sample_rate)
            .with_context(|| format!("write live utterance WAV {}", wav_path.display()))?;

        if !self.has_stop_signal {
            println!(
                "{} ({} ms) | saved {}",
                live_utterance_status_message(utterance_index, "speech"),
                endpoint::frames_to_ms(frames, self.chunk_duration_ms),
                wav_path.display(),
            );
        }

        // The transcript and output events use the configured provider name
        // verbatim so the UI shows exactly what the user selected, and the
        // configured language pair (no hardcoded en/es).
        let spoke = self.transcribe_process_and_speak(utterance_index, &wav_path, speaker);

        let _ = io::stdout().flush();

        // Delete the temporary WAV whatever the provider did with it.
        if let Err(e) = fs::remove_file(&wav_path) {
            eprintln!(
                "{} warning: could not delete temp WAV {}: {e}",
                live_utterance_status_message(utterance_index, "cleanup"),
                wav_path.display(),
            );
        }

        Ok(spoke)
    }

    fn transcribe_process_and_speak(
        &self,
        utterance_index: usize,
        wav_path: &Path,
        speaker: &mut Speaker<'_>,
    ) -> bool {
        let wav_path_str = wav_path.to_string_lossy().to_string();
        match self.provider.transcribe(&wav_path_str) {
            Ok(text) => {
                (self.on_event)(build_transcript_event(
                    utterance_index,
                    text.clone(),
                    self.provider_name.to_string(),
                    true,
                ));

                let Some(output) = self.processor.process(&text, self.process_mode) else {
                    return false;
                };

                // The rendered text is both the UI payload and the thing to
                // speak, so keep a copy before the event takes ownership.
                let spoken_text = output.text.clone();
                (self.on_event)(build_output_event(
                    utterance_index,
                    output.kind,
                    output.source_text,
                    output.text,
                    output.source_language,
                    output.target_language,
                    output.is_final,
                ));

                // `None` from synthesis means "nothing to say": emit no
                // speech event and do not touch the player, so silence never
                // reaches the wire as a zero-length utterance.
                let speech = Speech {
                    text: spoken_text,
                    language: self.target_language.to_string(),
                };
                let Some(chunk) = self.synthesizer.synthesize(&speech) else {
                    return false;
                };

                let duration_ms = chunk_duration_ms(&chunk);
                (self.on_event)(build_speech_event(
                    utterance_index,
                    speech.text,
                    speech.language,
                    duration_ms,
                    chunk.is_final,
                ));

                if let Err(e) = speaker.speak(utterance_index, &chunk, self.has_stop_signal) {
                    (self.on_event)(build_error_event(format!(
                        "[utterance {utterance_index}] playback error: {e}"
                    )));
                }
                true
            }
            Err(e) => {
                (self.on_event)(build_error_event(format!(
                    "[utterance {utterance_index}] transcription error: {e}"
                )));
                false
            }
        }
    }
}

/// Discard capture until the gate allows draining again.
///
/// Runs **after** playback returned (`end_speaking`) and after the caller
/// cleared the buffer, so the only thing still blocking is the reverb tail.
/// Anything captured during this window is thrown away too: it can be room
/// reverb or the device's own output latency. Draining the playback backlog
/// is deliberately not this function's job — that is `MicInput::clear`,
/// because a single chunk can outlast the whole tail and the leftovers
/// would then be transcribed as our own voice. The pure decision lives in
/// [`crate::audio::gate::HalfDuplexGate::drain_allowed`].
fn discard_blocked_capture(
    mic: &MicInput,
    chunk_size: usize,
    tick: Duration,
    captured_ms: u64,
    gate: &mut HalfDuplexGate,
) {
    while !gate.drain_allowed() {
        if mic.next_chunk(chunk_size).is_some() {
            gate.advance(captured_ms);
        } else {
            // Nothing buffered: wait one tick and let the tail elapse.
            // `captured_ms` is 0 for a degenerate config, so fall back to
            // the tick length to guarantee progress.
            thread::sleep(tick);
            gate.advance(captured_ms.max(tick.as_millis() as u64));
        }
    }
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
        hangover_ms: config::DEFAULT_HANGOVER_MS,
        min_speech_ms: config::DEFAULT_MIN_SPEECH_MS,
        max_utterance_ms: config::DEFAULT_MAX_UTTERANCE_MS,
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

    // ---- format_live_utterance_filename --------------------------------

    #[test]
    fn format_live_utterance_filename_001() {
        assert_eq!(format_live_utterance_filename(1), "live_utterance_001.wav");
    }

    #[test]
    fn format_live_utterance_filename_012() {
        assert_eq!(format_live_utterance_filename(12), "live_utterance_012.wav");
    }

    #[test]
    fn format_live_utterance_filename_123() {
        assert_eq!(
            format_live_utterance_filename(123),
            "live_utterance_123.wav"
        );
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

    // ---- live_utterance_status_message ----------------------------------

    #[test]
    fn status_message_contains_utterance_index() {
        let msg = live_utterance_status_message(5, "speech");
        assert!(
            msg.contains("5"),
            "message should contain utterance index 5, got: {msg}"
        );
    }

    #[test]
    fn status_message_contains_status() {
        let msg = live_utterance_status_message(3, "discarded");
        assert!(
            msg.contains("discarded"),
            "message should contain status 'discarded', got: {msg}"
        );
    }

    #[test]
    fn status_message_is_readable() {
        let msg = live_utterance_status_message(2, "speech");
        assert!(
            msg.starts_with("[utterance "),
            "message should start with '[utterance ', got: {msg}"
        );
    }

    // ---- stop_flush -----------------------------------------------------
    //
    // The bug these pin down: stopping a session while an utterance was open
    // used to drop `pending_samples`, so the last turn never reached the
    // provider. `advance` is driven directly here instead of the capture loop
    // so the decision runs without CPAL.

    fn endpoint_config(
        chunk_duration_ms: u64,
        hangover_ms: u64,
        min_speech_ms: u64,
    ) -> EndpointConfig {
        EndpointConfig {
            chunk_duration_ms,
            hangover_ms,
            min_speech_ms,
            max_utterance_ms: 15_000,
        }
    }

    #[test]
    fn stop_flush_speaks_the_open_utterance() {
        let cfg = endpoint_config(1_000, 400, 200);
        let mut state = EndpointState::new();
        assert_eq!(
            endpoint::advance(&mut state, true, &cfg),
            EndpointDecision::Continue
        );
        assert!(state.is_open());

        // One 1 s window of speech is a kept utterance; the samples buffered
        // for it must reach the stages rather than be thrown away.
        assert_eq!(
            stop_flush(&state, &cfg, 16_000),
            StopFlush::Speak { frames: 1 }
        );
    }

    #[test]
    fn stop_flush_counts_trailing_silence_in_the_frame_total() {
        let cfg = endpoint_config(1_000, 3_000, 200);
        let mut state = EndpointState::new();
        endpoint::advance(&mut state, true, &cfg);
        endpoint::advance(&mut state, true, &cfg);
        // One silent frame is inside the 3 s hangover, so the utterance stays
        // open and the stop-time flush must still include it.
        assert_eq!(
            endpoint::advance(&mut state, false, &cfg),
            EndpointDecision::Continue
        );

        assert_eq!(
            stop_flush(&state, &cfg, 48_000),
            StopFlush::Speak { frames: 3 }
        );
    }

    #[test]
    fn stop_flush_drops_a_burst_below_min_speech() {
        // 250 ms of speech needs 3 frames of 100 ms, so a single speech frame
        // is noise: no event, no WAV, no provider call.
        let cfg = endpoint_config(100, 400, 250);
        let mut state = EndpointState::new();
        assert_eq!(
            endpoint::advance(&mut state, true, &cfg),
            EndpointDecision::Continue
        );
        assert!(state.is_open());

        assert_eq!(stop_flush(&state, &cfg, 1_600), StopFlush::Drop);
    }

    #[test]
    fn stop_flush_drops_when_no_utterance_is_open() {
        let cfg = endpoint_config(1_000, 400, 200);
        let state = EndpointState::new();

        assert_eq!(stop_flush(&state, &cfg, 0), StopFlush::Drop);
    }

    #[test]
    fn stop_flush_drops_an_open_utterance_holding_no_samples() {
        // Defensive: an open utterance with an empty buffer would otherwise
        // write a zero-length WAV and call the provider on it.
        let cfg = endpoint_config(1_000, 400, 200);
        let mut state = EndpointState::new();
        endpoint::advance(&mut state, true, &cfg);

        assert_eq!(stop_flush(&state, &cfg, 0), StopFlush::Drop);
    }
}
