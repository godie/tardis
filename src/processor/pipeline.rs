//! CPAL-driven mock translation pipeline.
//!
//! Captures ~`seconds` from the default mic, splits the buffer into
//! `chunk_duration_ms` chunks via the shared `audio::chunker` helpers,
//! routes each chunk through a [`Transcriber`] (mock today), and,
//! when speech is detected, forwards the resulting text through a
//! [`Processor`] (mock today) in [`ProcessMode::Translate`]. Prints a
//! transcript line + output line per chunk, or a silence-skip line if no
//! speech was detected.
//!
//! Both the `Transcriber` and `Processor` traits stay audio-agnostic; the
//! capture stream comes from `audio::capture`. To swap in real impls,
//! change only the `MockTranscriber::new(...)` / `MockProcessor::new()`
//! constructions at the top of `run_mock_translate_test`. The capture loop,
//! drain loop, and print format stay the same.

use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::audio::capture::MicInput;
use crate::config::DEFAULT_VOLUME_THRESHOLD;
use crate::processor::MockProcessor;
use crate::processor::processor::{ProcessMode, Processor};
use crate::transcription::mock::MockTranscriber;
use crate::transcription::transcriber::Transcriber;

/// Run the (today MockTranscriber + MockProcessor) translation pipeline for
/// `seconds` of mic capture, splitting on `chunk_duration_ms` chunks. Each
/// chunk is first run through the `Transcriber`; if it yields
/// `Some(transcript)`, that text is then passed to the `Processor` and both
/// a transcript line and a translation line are printed. Otherwise a
/// silence-skip line is printed.
pub fn run_mock_translate_test(
    seconds: u64,
    chunk_duration_ms: u64,
    source_language: &str,
    target_language: &str,
) -> Result<()> {
    let mic = MicInput::start("translation")?;
    println!("Mock translate device: {}", mic.input().device_label());
    println!("Default input config: {}", mic.input().supported_label());
    let sample_rate = mic.sample_rate();
    let channels = mic.channels();
    let chunk_size = mic.chunk_size(chunk_duration_ms)?;
    println!(
        "Mock translate: chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch, \
         threshold {DEFAULT_VOLUME_THRESHOLD}, {source_language} -> {target_language})"
    );

    let transcriber = MockTranscriber::new(DEFAULT_VOLUME_THRESHOLD);
    let processor = MockProcessor::new();
    let mode = ProcessMode::Translate {
        source_language: source_language.to_string(),
        target_language: target_language.to_string(),
    };

    mic.play()?;
    println!("Capturing for {seconds} seconds...");

    let start = Instant::now();
    let total_duration = Duration::from_secs(seconds);
    let tick = Duration::from_millis(chunk_duration_ms);
    let mut chunk_index: usize = 0;
    let mut translated_chunks: usize = 0;
    let mut silence_chunks: usize = 0;

    while start.elapsed() < total_duration {
        thread::sleep(tick);

        while let Some(chunk) = mic.next_chunk(chunk_size) {
            chunk_index += 1;
            match transcriber.transcribe_chunk(chunk_index, &chunk) {
                Some(result) => {
                    // MockTranscriber guarantees non-empty text and
                    // MockProcessor only returns None for empty /
                    // whitespace input, so an output must exist.
                    // If a future transcriber yields "", the panic
                    // message names the trait contract that broke.
                    let output = processor.process(&result.text, &mode).expect(
                        "non-empty transcript must yield Some output \
                         (MockProcessor contract violated)",
                    );
                    translated_chunks += 1;
                    println!("[chunk {}] transcript: {}", chunk_index, result.text);
                    println!("[chunk {}] translation: {}", chunk_index, output.text);
                }
                None => {
                    silence_chunks += 1;
                    println!("[chunk {chunk_index}] silence detected, skipping translation...");
                }
            }
            let _ = io::stdout().flush();
        }
    }

    drop(mic);
    println!(
        "Mock translation finished. {chunk_index} chunks processed \
         ({translated_chunks} translated, {silence_chunks} silence)."
    );
    Ok(())
}
