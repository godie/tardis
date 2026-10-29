//! CPAL-driven transcription pipeline.
//!
//! Captures ~`seconds` from the default mic, splits the buffer into
//! `chunk_duration_ms` chunks via the shared `audio::chunker` helpers,
//! and routes each chunk through a [`Transcriber`] impl to decide
//! whether to print a transcript line or a silence skip line.
//!
//! Today the pipeline uses `MockTranscriber`; once a real API impl
//! lands behind a feature flag, only this file changes; the trait,
//! the pure logic, and the chunking layer stay untouched. The capture
//! stream comes from `audio::capture`; the `Transcriber` trait itself never
//! imports CPAL.

use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::audio::capture::MicInput;
use crate::config::DEFAULT_VOLUME_THRESHOLD;
use crate::transcription::mock::MockTranscriber;
use crate::transcription::transcriber::Transcriber;

/// Run the (today MockTranscriber-based) transcription pipeline for
/// `seconds` of mic capture, routing each completed
/// `chunk_duration_ms`-equivalent chunk through the trait.
pub fn run_mock_transcription_test(seconds: u64, chunk_duration_ms: u64) -> Result<()> {
    let mic = MicInput::start("transcription")?;
    println!("Mock transcription device: {}", mic.input().device_label());
    println!("Default input config: {}", mic.input().supported_label());
    let sample_rate = mic.sample_rate();
    let channels = mic.channels();
    let chunk_size = mic.chunk_size(chunk_duration_ms)?;
    println!(
        "Mock pipeline: chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch, \
         threshold {DEFAULT_VOLUME_THRESHOLD})"
    );

    let transcriber = MockTranscriber::new(DEFAULT_VOLUME_THRESHOLD);

    mic.play()?;
    println!("Capturing for {seconds} seconds...");

    let start = Instant::now();
    let total_duration = Duration::from_secs(seconds);
    let tick = Duration::from_millis(chunk_duration_ms);
    let mut chunk_index: usize = 0;
    let mut speech_chunks: usize = 0;
    let mut silence_chunks: usize = 0;

    while start.elapsed() < total_duration {
        thread::sleep(tick);

        while let Some(chunk) = mic.next_chunk(chunk_size) {
            chunk_index += 1;
            match transcriber.transcribe_chunk(chunk_index, &chunk) {
                Some(result) => {
                    speech_chunks += 1;
                    println!(
                        "[chunk {}] {} | volume: {:.4}",
                        result.chunk_index, result.text, result.average_volume,
                    );
                }
                None => {
                    silence_chunks += 1;
                    println!("[chunk {chunk_index}] silence detected, skipping...");
                }
            }
            let _ = io::stdout().flush();
        }
    }

    drop(mic);
    println!(
        "Mock transcription finished. {chunk_index} chunks processed \
         ({speech_chunks} speech, {silence_chunks} silence)."
    );
    Ok(())
}
