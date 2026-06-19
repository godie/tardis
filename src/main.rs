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
//!
//! Only the `mic` mode runs forever; everything else exits on its own.

mod transcription;

mod audio;

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cpal::traits::StreamTrait;

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("devices") => run_devices(),
        Some("mic") => run_mic_continuous(),
        Some("mic-5s") => run_mic_for(Duration::from_secs(5)),
        Some("record-5s") => run_record_5s(),
        Some("chunk-test") => run_chunk_test(),
        Some("mock-transcribe") => run_mock_transcribe(),
        Some("save-chunks-test") => run_save_chunks_test(),
        Some("mock-transcribe-file") => run_mock_transcribe_file(),
        Some(other) => {
            eprintln!("Unknown mode: {other}");
            eprintln!("Usage: cargo run [-- devices | -- mic | -- mic-5s | -- record-5s | -- chunk-test | -- mock-transcribe | -- save-chunks-test | -- mock-transcribe-file <path>]");
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

fn run_record_5s() -> Result<()> {
    audio::recorder::record_default_mic_to_wav_for_seconds(5, "output/mic_test.wav")
}

fn run_chunk_test() -> Result<()> {
    audio::chunker::run_chunk_test(10, 1000)
}

fn run_mock_transcribe() -> Result<()> {
    transcription::pipeline::run_mock_transcription_test(10, 1000)
}

fn run_save_chunks_test() -> Result<()> {
    audio::chunk_recorder::run_save_chunks_test(10, 1000, "output/chunks")
}

fn run_mock_transcribe_file() -> Result<()> {
    let path = std::env::args().nth(2).ok_or_else(|| {
        anyhow!("mock-transcribe-file requires a path argument.\nUsage: cargo run -- mock-transcribe-file <path-to-wav>")
    })?;
    transcription::file_pipeline::run_mock_transcribe_file(&path)
}
