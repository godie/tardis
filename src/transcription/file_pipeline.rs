//! File-based mock transcription pipeline.
//!
//! Reads a previously-saved chunk WAV from disk, converts its i16
//! samples to f32, and routes the result through the existing
//! `MockTranscriber` so a real WAV can be re-classified without
//! re-running the microphone capture.
//!
//! Pure helpers (`i16_sample_to_f32`, `i16_samples_to_f32`) live next
//! to the WAV reader so the conversion math is unit-testable without
//! touching disk or real audio hardware.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use hound::{SampleFormat as HoundSampleFormat, WavReader};

use crate::audio::volume::calculate_average_volume;
use crate::transcription::mock::MockTranscriber;
use crate::transcription::transcriber::Transcriber;

/// Volume threshold for "speech vs. silence" — matches the live mock
/// pipeline (`transcription::pipeline::VOLUME_THRESHOLD`).
const VOLUME_THRESHOLD: f32 = 0.01;

// ---- pure helpers ----------------------------------------------------------

/// Convert a single i16 sample (in standard PCM range) to f32 in
/// `[-1.0, 1.0]`. Symmetric around zero: `0 -> 0.0`,
/// `i16::MAX -> ~1.0`, `i16::MIN -> ~-1.0` (off by ~3e-5 because the
/// symmetric range is -32768..=32767).
pub fn i16_sample_to_f32(sample: i16) -> f32 {
    sample as f32 / i16::MAX as f32
}

/// Convert a slice of i16 samples to f32 via [`i16_sample_to_f32`].
/// Length is preserved; sign + order are preserved per-sample since
/// the in-range mapping is a monotone 1:1.
pub fn i16_samples_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| i16_sample_to_f32(s)).collect()
}

// ---- WAV-driven driver -----------------------------------------------------

/// Read `file_path`, classify it via [`MockTranscriber`] at threshold
/// [`VOLUME_THRESHOLD`], and print a per-file summary. Currently
/// supports 16-bit Int WAV only (the format `chunk_recorder` writes).
pub fn run_mock_transcribe_file(file_path: &str) -> Result<()> {
    let path = Path::new(file_path);
    let mut reader = WavReader::open(path)
        .with_context(|| format!("open WAV file {}", path.display()))?;
    let spec = reader.spec();

    if spec.bits_per_sample != 16 || spec.sample_format != HoundSampleFormat::Int {
        return Err(anyhow!(
            "unsupported WAV format in {}: {}-bit {:?} (file_pipeline supports 16-bit Int only)",
            path.display(),
            spec.bits_per_sample,
            spec.sample_format,
        ));
    }

    let i16_samples: Vec<i16> = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("read samples from {}", path.display()))?;

    let samples_f32 = i16_samples_to_f32(&i16_samples);
    let avg_volume = calculate_average_volume(&samples_f32);
    let transcriber = MockTranscriber::new(VOLUME_THRESHOLD);
    let chunk_index: usize = 1; // a file is one "chunk".

    println!("File: {}", path.display());
    println!("Sample count: {}", i16_samples.len());
    println!("Average volume: {avg_volume:.4}");

    match transcriber.transcribe_chunk(chunk_index, &samples_f32) {
        Some(result) => println!("{}", result.text),
        None => println!("silence detected"),
    }

    Ok(())
}

// ---- unit tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "actual: {actual}, expected: {expected}",
        );
    }

    // ---- i16_sample_to_f32 ---------------------------------------------

    #[test]
    fn i16_sample_to_f32_zero_maps_to_zero() {
        assert_eq!(i16_sample_to_f32(0), 0.0);
    }

    #[test]
    fn i16_sample_to_f32_max_maps_near_one() {
        let v = i16_sample_to_f32(i16::MAX);
        assert_approx(v, 1.0, 1e-6);
    }

    #[test]
    fn i16_sample_to_f32_min_maps_near_neg_one() {
        let v = i16_sample_to_f32(i16::MIN);
        // -32768 / 32767 = -1.00003…; spec only requires "close to -1.0".
        assert_approx(v, -1.0, 1e-4);
    }

    // ---- i16_samples_to_f32 --------------------------------------------

    #[test]
    fn i16_samples_to_f32_preserves_length() {
        let input = vec![0_i16, 100, -100];
        let output = i16_samples_to_f32(&input);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn i16_samples_to_f32_preserves_order() {
        let input = vec![i16::MIN, -100, 0, 100, i16::MAX];
        let output = i16_samples_to_f32(&input);
        assert!(output[0] < output[1], "expected output[0] < output[1]");
        assert!(output[1] < output[2], "expected output[1] < output[2]");
        assert!(output[2] < output[3], "expected output[2] < output[3]");
        assert!(output[3] < output[4], "expected output[3] < output[4]");
    }

    #[test]
    fn i16_samples_to_f32_preserves_sign() {
        let input = vec![-1000_i16, -1, 0, 1, 1000];
        let output = i16_samples_to_f32(&input);
        assert!(output[0] < 0.0, "expected output[0] < 0, got {}", output[0]);
        assert!(output[1] < 0.0, "expected output[1] < 0, got {}", output[1]);
        assert_eq!(output[2], 0.0);
        assert!(output[3] > 0.0, "expected output[3] > 0, got {}", output[3]);
        assert!(output[4] > 0.0, "expected output[4] > 0, got {}", output[4]);
    }

    #[test]
    fn i16_samples_to_f32_empty_input_returns_empty_output() {
        let output = i16_samples_to_f32(&[]);
        assert!(output.is_empty());
    }
}
