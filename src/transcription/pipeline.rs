//! CPAL-driven mock pipeline.
//!
//! Captures ~`seconds` from the default mic, splits the buffer into
//! `chunk_duration_ms` chunks via the shared `audio::chunker` helpers,
//! and routes each chunk through `mock_transcribe_chunk` to decide
//! whether to print a fake transcript line or a silence skip line.
//!
//! The mock data path is the scaffolding for tomorrow's real
//! transcription API: only `mock_transcribe_chunk` changes when we
//! wire in the real model.

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
use crate::transcription::mock::mock_transcribe_chunk;

/// Volume threshold for "speech vs. silence" decisions. Matches the
/// value used by the live volume log in `audio::mic`.
const VOLUME_THRESHOLD: f32 = 0.01;

/// Run the mock transcription pipeline for `seconds` of mic capture,
/// routing each completed 1-second-equivalent chunk through
/// `mock_transcribe_chunk`.
pub fn run_mock_transcription_test(seconds: u64, chunk_duration_ms: u64) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No default input device available"))?;
    println!("Mock transcription device: {}", device);

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
        "Mock pipeline: chunking every {chunk_duration_ms} ms \
         ({chunk_size} samples per chunk, {sample_rate} Hz, {channels} ch, \
         threshold {VOLUME_THRESHOLD})"
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
    let mut speech_chunks: usize = 0;
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
            match mock_transcribe_chunk(chunk_index, &chunk, VOLUME_THRESHOLD) {
                Some(transcript) => {
                    speech_chunks += 1;
                    println!("{transcript}");
                }
                None => {
                    silence_chunks += 1;
                    println!("[chunk {chunk_index}] silence detected, skipping...");
                }
            }
            let _ = io::stdout().flush();
        }
    }

    drop(stream);
    println!(
        "Mock transcription finished. {chunk_index} chunks processed \
         ({speech_chunks} speech, {silence_chunks} silence)."
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
    let err_fn = |err| eprintln!("transcription stream error: {err}");
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
