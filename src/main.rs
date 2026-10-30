//! Tardis CLI entrypoint.
//!
//! Modes:
//!   cargo run                -> devices
//!   cargo run -- devices     -> print host + input/output devices and exit
//!   cargo run -- mic         -> capture from default mic until Ctrl+C
//!   cargo run -- mic-5s      -> capture from default mic for 5 seconds and exit
//!   cargo run -- record-5s   -> record 5 s of mic audio to output/mic_test.wav
//!   cargo run -- chunk-test  -> 10 s of mic capture, 1 s chunks, no WAV file
//!   cargo run -- mock-transcribe -> 10 s of mic, mock transcript vs silence per chunk
//!   cargo run -- save-chunks-test -> 10 s of mic, save each 1 s chunk as output/chunks/chunk_NNN.wav
//!   cargo run -- mock-transcribe-file <path> -> read a saved WAV from disk, run MockTranscriber on it
//!   cargo run -- mock-translate        -> 10 s of mic, mock transcript + mock translation per chunk (en -> es)
//!   cargo run -- local-transcribe-file [--provider <name>] <path> -> send a WAV file to a LocalTranscriptionProvider (default local-whisper)
//!   cargo run -- live-local-transcribe [--provider <name>] -> chunk-by-chunk live transcription (default mock-local)
//!   cargo run -- settings-default                           -> print the default AppRuntimeConfig as pretty JSON
//!   cargo run -- settings-validate <path>                   -> load + validate a settings JSON file (exit 0 on success, 1 on error)
//!   cargo run -- app-mock-flow                              -> exercise the mock AppService flow end-to-end (no mic, no Docker)
//!   cargo run -- mock-voice-flow                            -> full five-stage mock loop: transcript -> translation -> speech -> play (no mic, no Docker, no TTS)
//!
//! Only the `mic` mode runs forever; everything else exits on its own.

// Module declarations now live in `src/lib.rs` so this binary and
// the `tardis-ui-shell` desktop crate (`src-tauri`) share the same
// audio + transcription + translation surface through one library.
use std::thread;

use std::time::{Duration, Instant};
use tardis::{
    app::{
        config::AppRuntimeConfig,
        events::{AppEvent, format_app_event_for_console, status_label},
        live_events::{build_output_event, build_speech_event, build_transcript_event},
        service::AppService,
        settings_store,
    },
    audio,
    audio::player::{MockPlayer, Player},
    config, processor,
    processor::processor::ProcessMode,
    processor::{DEFAULT_PROCESSOR_NAME, build_processor},
    synthesis::{MockSynthesizer, Speech, Synthesizer, chunk_duration_ms},
    transcription,
};

use anyhow::{Context, Result, anyhow};
use cpal::traits::StreamTrait;

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("devices") => run_devices(),
        Some("mic") => run_mic_continuous(),
        Some("mic-5s") => run_mic_for(Duration::from_secs(config::DEFAULT_SHORT_CAPTURE_SECONDS)),
        Some("record-5s") => run_record_5s(),
        Some("chunk-test") => run_chunk_test(),
        Some("mock-transcribe") => run_mock_transcribe(),
        Some("save-chunks-test") => run_save_chunks_test(),
        Some("mock-transcribe-file") => run_mock_transcribe_file(),
        Some("mock-translate") => run_mock_translate(),
        Some("local-transcribe-file") => run_local_transcribe_file(),
        Some("live-local-transcribe") => run_live_local_transcribe(),
        Some("app-mock-flow") => run_app_mock_flow(),
        Some("mock-voice-flow") => run_mock_voice_flow(),
        Some("settings-default") => run_settings_default(),
        Some("settings-validate") => run_settings_validate(),
        Some(other) => {
            eprintln!("Unknown mode: {other}");
            eprintln!(
                "Usage: cargo run [-- devices | -- mic | -- mic-5s | -- record-5s | -- chunk-test | -- mock-transcribe | -- save-chunks-test | -- mock-transcribe-file <path> | -- mock-translate | -- local-transcribe-file [--provider <name>] <path> | -- live-local-transcribe [--provider <name>] | -- app-mock-flow | -- mock-voice-flow | -- settings-default | -- settings-validate <path>]"
            );
            std::process::exit(2);
        }
    }
}

fn run_devices() -> Result<()> {
    let host = cpal::default_host();
    audio::devices::print_device_info(&host)
}

fn run_mic_continuous() -> Result<()> {
    // `stream` is held to keep the audio thread alive; the OS handles Ctrl+C
    // by tearing down the process, which drops the stream and stops capture.
    let stream = audio::mic::start_default_mic_capture()?;
    stream.play()?;
    println!("Listening to microphone... Press Ctrl+C to stop.");
    loop {
        thread::sleep(Duration::from_millis(500));
    }
}

fn run_mic_for(duration: Duration) -> Result<()> {
    let stream = audio::mic::start_default_mic_capture()?;
    stream.play()?;
    println!("Capturing for {} seconds...", duration.as_secs());
    let start = Instant::now();
    while start.elapsed() < duration {
        thread::sleep(Duration::from_millis(50));
    }
    println!("Capture finished.");
    Ok(())
}

/// Print the default [`AppRuntimeConfig`] as pretty JSON.
///
/// Mirrors the file the Tauri shell's `save_runtime_settings`
/// command would write for the same default. Useful for:
///
/// - Sanity-checking what the live backend starts with.
/// - Bootstrapping a fresh `runtime.json` (pipe into the
///   `<OS config dir>/tardis/runtime.json` path the
///   `settings-validate` command understands).
/// - Cross-checking the CLI defaults match the UI defaults
///   the Tauri shell pre-populates the settings panel with.
///
/// Pure: no FS, no microphone, no Docker. The output is
/// `serde_json::to_string_pretty` of the normalized default
/// config so it is directly pipe-able into a `runtime.json`
/// file.
fn run_settings_default() -> Result<()> {
    let json = settings_store::serialize_runtime_config(&AppRuntimeConfig::default())?;
    println!("{json}");
    Ok(())
}

/// Load + validate a settings JSON file at `<path>` and print
/// whether it is valid.
///
/// Exit codes:
/// - `0` — file loaded and validated successfully. The
///   resolved values are printed so the operator can confirm
///   which provider / language pair the file pins.
/// - `1` — any error (invalid JSON, validation failure, FS
///   I/O other than "file not found"). The original
///   `SettingsStoreError` is preserved in the error output so
///   the operator can tell parse errors from out-of-bounds
///   threshold errors without re-running the command.
///
/// Mirrors the validation the Tauri shell's
/// `load_runtime_settings` command runs, so a config that
/// passes here will also be accepted by the UI on next
/// launch.
fn run_settings_validate() -> Result<()> {
    let path = std::env::args().nth(2).ok_or_else(|| {
        anyhow!(
            "settings-validate requires a path argument.\n\
             Usage: cargo run -- settings-validate <path-to-runtime.json>"
        )
    })?;
    let path_buf = std::path::PathBuf::from(&path);
    match settings_store::load_from_path_or_default(&path_buf) {
        Ok(cfg) => {
            println!("valid: {path}");
            println!("transcription_provider: {}", cfg.transcription_provider);
            println!("source_language:        {}", cfg.source_language);
            println!("target_language:        {}", cfg.target_language);
            println!("chunk_duration_ms:      {}", cfg.chunk_duration_ms);
            println!("volume_threshold:       {}", cfg.volume_threshold);
            Ok(())
        }
        Err(e) => {
            eprintln!("invalid: {path}");
            // `{e:#}` prints the full anyhow error chain (Display
            // alone only shows the top-level message), so the
            // underlying `serde_json` parse error's line/column
            // and the validator's first-violation message both
            // surface in the operator's terminal.
            eprintln!("reason: {e:#}");
            std::process::exit(1);
        }
    }
}

/// Manual smoke test for the [`tardis::app`] orchestration
/// layer.
///
/// What it does:
/// 1. Builds an [`AppService`] from the default
///    [`AppRuntimeConfig`] (provider `mock-local`, languages
///    `en -> es`).
/// 2. Calls `start_listening_mock`, prints the resulting event.
/// 3. Runs `run_mock_text_flow` with the
///    `"mock transcript: speech detected"` string used by the
///    existing mock UI surface, prints the emitted transcript
///    + translation events.
/// 4. Calls `stop_listening`, prints the resulting event.
/// 5. Prints the final state summary.
///
/// What it deliberately does **not** do (despite the word
/// "listening" in the helper names):
/// - Open the microphone or start a CPAL stream.
/// - Reach the faster-whisper Docker container.
/// - Bind to a Tauri runtime.
/// - Persist anything to disk.
///
/// This command exists for the developer to validate that the
/// `app` module + `MockProcessor` are wired together end-to-end
/// in one short synchronous run.
fn run_app_mock_flow() -> Result<()> {
    println!("=== app-mock-flow (sync smoke for AppService) ===\n");

    let mut service = AppService::new(AppRuntimeConfig::default())
        .map_err(|e| anyhow!("AppService::new with default config failed: {e}"))?;

    // 1. Start listening (one StatusChanged(Listening) event).
    print_step("start_listening_mock");
    print_events(&service.start_listening_mock());

    // 2. Run mock text flow with the canonical UI mock phrase.
    print_step("run_mock_text_flow(\"mock transcript: speech detected\")");
    print_events(&service.run_mock_text_flow("mock transcript: speech detected"));

    // 3. Stop listening (one StatusChanged(Stopped) event).
    print_step("stop_listening");
    print_events(&service.stop_listening());

    // 4. Demonstrate the silent-skip path. `run_mock_text_flow`
    //    returns an empty `Vec<AppEvent>` for empty / whitespace
    //    input so `print_events` shows "(no events)". The UI shell
    //    must not render an "empty transcript" toast on
    //    every capture tick and the CLI follows the same
    //    philosophy.
    print_step("run_mock_text_flow(\"\")  // silent-skip demo");
    print_events(&service.run_mock_text_flow(""));

    // 5. Final summary so the operator can sanity-check the
    //    resulting state without re-reading the event log.
    println!("\n=== summary ===");
    let state = service.state();
    println!("final status:     {}", status_label(state.status));
    println!("last_transcript:  {:?}", state.last_transcript);
    println!("last_output: {:?}", state.last_output);
    println!("\n=== done ===");
    Ok(())
}

/// Manual smoke test for the full five-stage mock loop:
/// transcript -> translation -> synthesis -> playback.
///
/// What it does:
/// 1. Takes the canonical mock phrase as the transcript (the capture
///    and transcribe stages collapse into that literal; both real
///    stages are exercised by their own modes).
/// 2. Runs it through [`MockProcessor`] with the default language
///    pair from [`AppRuntimeConfig::default`].
/// 3. Synthesizes the rendered text with [`MockSynthesizer`] and hands
///    the chunk to [`MockPlayer`], which records instead of playing.
/// 4. Prints one console line per event, using the same
///    [`format_app_event_for_console`] the live pipeline uses.
///
/// What it deliberately does **not** do: open the microphone, reach
/// Docker, call a TTS service, or make a sound. It is the cheapest way
/// to confirm the stage contracts line up (`speech` event included)
/// on a machine with no audio hardware at all.
fn run_mock_voice_flow() -> Result<()> {
    println!("=== mock-voice-flow (five-stage mock loop: text -> speech) ===\n");

    let config = AppRuntimeConfig::default();
    let transcript_text = "mock transcript: speech detected";

    let processor = build_processor(DEFAULT_PROCESSOR_NAME)?;
    let mode = ProcessMode::Translate {
        source_language: config.source_language.clone(),
        target_language: config.target_language.clone(),
    };
    let synthesizer = MockSynthesizer::new();
    let mut player = MockPlayer::new();
    let mut events: Vec<AppEvent> = Vec::with_capacity(3);

    // 1. Transcript (capture + transcribe).
    events.push(build_transcript_event(
        1,
        transcript_text.to_string(),
        config.transcription_provider.clone(),
        true,
    ));

    // 2. Process (translate mode; `build_processor` dispatch).
    let output = processor
        .process(transcript_text, &mode)
        .ok_or_else(|| anyhow!("mock processor returned no output for a non-empty transcript"))?;
    let spoken_text = output.text.clone();
    events.push(build_output_event(
        1,
        output.kind,
        output.source_text,
        output.text,
        output.source_language,
        output.target_language,
        output.is_final,
    ));

    // 3. Synthesize + 4. play. Synthesis returning `None` is a normal
    //    "nothing to say"; for the canonical phrase it must not happen,
    //    so treat it as a contract violation here.
    let speech = Speech {
        text: spoken_text,
        language: config.target_language.clone(),
    };
    let chunk = synthesizer
        .synthesize(&speech)
        .ok_or_else(|| anyhow!("mock synthesizer returned no audio for non-empty speech"))?;
    let duration_ms = chunk_duration_ms(&chunk);
    player
        .play_chunk(&chunk)
        .context("mock player failed to accept the chunk")?;
    events.push(build_speech_event(
        1,
        speech.text,
        speech.language,
        duration_ms,
        chunk.is_final,
    ));

    println!();
    for event in &events {
        println!("{}", format_app_event_for_console(event));
    }

    println!("\n=== summary ===");
    println!("provider:  {}", config.transcription_provider);
    println!(
        "languages: {} -> {}",
        config.source_language, config.target_language
    );
    println!(
        "spoken:    {} chunk(s), {} ms via mock player (nothing audible)",
        player.played_count(),
        player.played_duration_ms(),
    );
    println!("\n=== done ===");
    Ok(())
}

fn print_step(name: &str) {
    println!("\n[step] {name}");
}

fn print_events(events: &[AppEvent]) {
    if events.is_empty() {
        println!("  (no events)");
        return;
    }
    for (i, event) in events.iter().enumerate() {
        // `{:#?}` prints each event on its own block with field
        // labels so the operator can scan transcript vs
        // translation shapes at a glance.
        println!("  [event {i}] {event:#?}");
    }
}

fn run_record_5s() -> Result<()> {
    audio::recorder::record_default_mic_to_wav_for_seconds(
        config::DEFAULT_SHORT_CAPTURE_SECONDS,
        config::DEFAULT_MIC_RECORDING_PATH,
    )
}

fn run_chunk_test() -> Result<()> {
    audio::chunker::run_chunk_test(
        config::DEFAULT_PIPELINE_TEST_SECONDS,
        config::DEFAULT_CHUNK_DURATION_MS,
    )
}

fn run_mock_transcribe() -> Result<()> {
    transcription::pipeline::run_mock_transcription_test(
        config::DEFAULT_PIPELINE_TEST_SECONDS,
        config::DEFAULT_CHUNK_DURATION_MS,
    )
}

fn run_save_chunks_test() -> Result<()> {
    audio::chunk_recorder::run_save_chunks_test(
        config::DEFAULT_PIPELINE_TEST_SECONDS,
        config::DEFAULT_CHUNK_DURATION_MS,
        config::DEFAULT_CHUNKS_DIR,
    )
}

fn run_mock_transcribe_file() -> Result<()> {
    let path = std::env::args().nth(2).ok_or_else(|| {
        anyhow!("mock-transcribe-file requires a path argument.\nUsage: cargo run -- mock-transcribe-file <path-to-wav>")
    })?;
    transcription::file_pipeline::run_mock_transcribe_file(&path)
}

fn run_mock_translate() -> Result<()> {
    processor::pipeline::run_mock_translate_test(
        config::DEFAULT_PIPELINE_TEST_SECONDS,
        config::DEFAULT_CHUNK_DURATION_MS,
        config::DEFAULT_SOURCE_LANGUAGE,
        config::DEFAULT_TARGET_LANGUAGE,
    )
}

/// Send a WAV file on disk to a
/// [`transcription::LocalTranscriptionProvider`] selected by the
/// `--provider` flag and print the plaintext transcript.
///
/// Default provider is `"local-whisper"` (the self-hosted
/// faster-whisper Docker HTTP server). Pass `--provider mock-local`
/// to use the deterministic offline stub instead. It needs no Docker
/// and echoes `"mock transcript for <basename>"` for whatever
/// path is given. Connection-level errors from the Docker provider
/// still carry an explicit "is the Docker container running?"
/// remediation hint.
///
/// Usage:
///   cargo run -- local-transcribe-file [--provider <name>] <path-to-wav>
/// Live chunk-by-chunk local transcription from the default
/// microphone. Default provider is `"mock-local"` (no Docker
/// required); pass `--provider local-whisper` to use the
/// self-hosted faster-whisper server instead.
///
/// This is **not** true streaming: each speech-like chunk is
/// written to a temporary WAV file in `output/live_chunks/`,
/// sent through the selected
/// [`transcription::LocalTranscriptionProvider`], and deleted
/// after transcription. Silence chunks are skipped.
///
/// Usage:
///   cargo run -- live-local-transcribe [--provider <name>]
fn run_live_local_transcribe() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut provider_name = "mock-local".to_string();

    // [0] = binary path, [1] = "live-local-transcribe"; start at [2].
    let mut i = 2;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--provider" {
            i += 1;
            if i >= args.len() {
                return Err(anyhow!(
                    "--provider requires a value (e.g. --provider mock-local)\nUsage: cargo run -- live-local-transcribe [--provider <name>]"
                ));
            }
            provider_name = args[i].clone();
        } else if let Some(value) = arg.strip_prefix("--provider=") {
            provider_name = value.to_string();
        } else {
            return Err(anyhow!(
                "unexpected argument: {arg}\nUsage: cargo run -- live-local-transcribe [--provider <name>]"
            ));
        }
        i += 1;
    }

    transcription::live_local::run_live_local_transcription_test(
        &provider_name,
        config::DEFAULT_PIPELINE_TEST_SECONDS,
        config::DEFAULT_CHUNK_DURATION_MS,
    )
}

fn run_local_transcribe_file() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut provider_name = "local-whisper".to_string();
    let mut path: Option<String> = None;

    // [0] = binary path, [1] = "local-transcribe-file"; start at [2].
    let mut i = 2;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--provider" {
            i += 1;
            if i >= args.len() {
                return Err(anyhow!(
                    "--provider requires a value (e.g. --provider mock-local)\nUsage: cargo run -- local-transcribe-file [--provider <name>] <path-to-wav>"
                ));
            }
            provider_name = args[i].clone();
        } else if let Some(value) = arg.strip_prefix("--provider=") {
            provider_name = value.to_string();
        } else {
            // Positional argument: must be the path. Reject extras
            // rather than silently dropping them so the user can fix
            // the order on their side.
            if path.is_some() {
                return Err(anyhow!(
                    "unexpected extra positional argument: {arg}\nUsage: cargo run -- local-transcribe-file [--provider <name>] <path-to-wav>"
                ));
            }
            path = Some(arg.clone());
        }
        i += 1;
    }

    let path = path.ok_or_else(|| {
        anyhow!("Usage: cargo run -- local-transcribe-file [--provider <name>] <path-to-wav>")
    })?;

    let provider = transcription::build_provider(&provider_name)?;

    println!("file:     {}", path);
    println!("provider: {}", provider.name());
    println!("transcribing...\n");

    match provider.transcribe(&path) {
        Ok(text) => {
            println!("transcript:");
            println!("{}", text);
            Ok(())
        }
        Err(e) => Err(e),
    }
}
