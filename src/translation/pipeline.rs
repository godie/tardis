//! CPAL-driven mock translation pipeline.
//!
//! Captures ~`seconds` from the default mic, splits the buffer into
//! `chunk_duration_ms` chunks via the shared `audio::chunker` helpers,
//! routes each chunk through a [`Transcriber`] (mock today), and —
//! when speech is detected — forwards the resulting text through a
//! [`Translator`] (mock today). Prints a transcript line + translation
//! line per chunk, or a silence-skip line if no speech was detected.
//!
//! Both the `Transcriber` and `Translator` traits stay audio-agnostic;
//! only this file imports CPAL. To swap in real impls, change only the
//! `MockTranscriber::new(...)` / `MockTranslator::new()` constructions
//! at the top of `run_mock_translate_test` — the capture loop, drain
//! loop, and print format stay the same.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use crate::audio::chunker::{
    calculate_chunk_size_samples, drain_chunk, has_complete_chunk,
};
use crate::transcription::mock::MockTranscriber;
use crate::transcription::transcriber::Transcriber;
use crate::translation::mock::MockTranslator;
use crate::translation::translator::Translator;

/// Volume threshold for "speech vs. silence" — matches
/// `transcription::pipeline::VOLUME_THRESHOLD` and `transcription::mock`.
const VOLUME_THRESHOLD: f32 = 0.01;

/// Run the (today MockTranscriber + MockTranslator) translation
/// pipeline for `seconds` of mic capture, splitting on
/// `chunk_duration_ms` chunks. Each chunk is first run through the
/// `Transcriber`; if it yields `Some(transcript)`, that text is then
/// passed to the `Translator` and both a transcript line and a
/// translation line are printed. Otherwise a silence-skip line is
/// printed.
pub fn run_mock_translate_test(
    seconds: u64,
    chunk_duration_ms: u64,
    source_language: &str,
    target_language: &str,
) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Mock translate device: {}", device);

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
        "Mock translate: chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch, \
         threshold {VOLUME_THRESHOLD}, {source_language} -> {target_language})"
    );

    let transcriber = MockTranscriber::new(VOLUME_THRESHOLD);
    let translator = MockTranslator::new();

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
    let mut translated_chunks: usize = 0;
    let mut silence_chunks: usize = 0;

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
            let Some(chunk) = drained else { break };

            chunk_index += 1;
            match transcriber.transcribe_chunk(chunk_index, &chunk) {
                Some(result) => {
                    // MockTranscriber guarantees non-empty text and
                    // MockTranslator only returns None for empty /
                    // whitespace input — so a translation must exist.
                    // If a future transcriber yields "", the panic
                    // message names the trait contract that broke.
                    let translation = translator
                        .translate_text(&result.text, source_language, target_language)
                        .expect(
                            "non-empty transcript must yield Some translation \
                             (MockTranslator contract violated)",
                        );
                    translated_chunks += 1;
                    println!("[chunk {}] transcript: {}", chunk_index, result.text);
                    println!(
                        "[chunk {}] translation: {}",
                        chunk_index, translation.translated_text
                    );
                }
                None => {
                    silence_chunks += 1;
                    println!("[chunk {chunk_index}] silence detected, skipping translation...");
                }
            }
            let _ = io::stdout().flush();
        }
    }

    drop(stream);
    println!(
        "Mock translation finished. {chunk_index} chunks processed \
         ({translated_chunks} translated, {silence_chunks} silence)."
    );
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
    let err_fn = |err| eprintln!("translation stream error: {err}");
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
