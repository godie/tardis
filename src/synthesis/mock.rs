//! `MockSynthesizer` implements [`Synthesizer`] offline, deterministically.
//!
//! It renders a fixed-amplitude square wave whose length is a pure
//! function of the text: `ceil(sample_rate * ms_per_char * chars)`.
//! The same text gives byte-identical samples on every run. There is no
//! randomness, no clock, and no allocation beyond the sample buffer, so
//! tests can assert exact lengths and the live loop runs without a TTS
//! service.
//!
//! Empty or whitespace-only text returns `None` (nothing to say),
//! mirroring [`crate::processor::mock::MockProcessor`]'s
//! skip-on-empty contract.

use crate::synthesis::{AudioChunk, Speech, Synthesizer};

/// Sample rate every mock chunk is rendered at (16 kHz mono, the
/// whisper-shaped rate). Also makes the arithmetic exact: one
/// character is 60 ms, i.e. 960 frames.
pub const MOCK_SAMPLE_RATE: u32 = 16_000;

/// Rendered milliseconds per character of trimmed text.
pub const MOCK_MS_PER_CHAR: u32 = 60;

/// Square-wave frequency, in Hz. Any fixed value works; 220 Hz is
/// audible and cheap to compute.
pub const MOCK_TONE_HZ: f32 = 220.0;

/// Peak amplitude. Well below `1.0` so a future real player never
/// clips a mock chunk into a surprise full-scale burst.
pub const MOCK_AMPLITUDE: f32 = 0.25;

/// Mock implementation of [`Synthesizer`].
///
/// Pure, synchronous, object-safe, `Default`-constructible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MockSynthesizer;

impl MockSynthesizer {
    /// Construct a fresh `MockSynthesizer`.
    pub fn new() -> Self {
        Self
    }
}

impl Synthesizer for MockSynthesizer {
    fn synthesize(&self, speech: &Speech) -> Option<AudioChunk> {
        let text = speech.text.trim();
        if text.is_empty() {
            return None;
        }

        let chars = text.chars().count() as u64;
        let frames = mock_frame_count(chars);
        let samples = (0..frames).map(mock_sample).collect();

        Some(AudioChunk {
            samples,
            sample_rate: MOCK_SAMPLE_RATE,
            channels: 1,
            is_final: true,
        })
    }
}

/// Deterministic frame count for `chars` characters of text:
/// `ceil(sample_rate * ms_per_char * chars / 1000)`.
///
/// Pure and public so the length contract is testable without
/// allocating a buffer.
pub fn mock_frame_count(chars: u64) -> u64 {
    let frames = MOCK_SAMPLE_RATE as u64 * MOCK_MS_PER_CHAR as u64 * chars;
    frames.div_ceil(1_000)
}

/// One square-wave sample at `frame`.
///
/// Pure function of its argument, so the whole buffer is reproducible
/// frame by frame.
fn mock_sample(frame: u64) -> f32 {
    let phase = (frame as f32 * MOCK_TONE_HZ / MOCK_SAMPLE_RATE as f32).fract();
    if phase < 0.5 {
        MOCK_AMPLITUDE
    } else {
        -MOCK_AMPLITUDE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthesis::chunk_duration_ms;

    fn speech(text: &str) -> Speech {
        Speech {
            text: text.to_string(),
            language: "es".to_string(),
        }
    }

    // ---- skip-on-empty -------------------------------------------------

    #[test]
    fn empty_text_returns_none() {
        assert!(MockSynthesizer::new().synthesize(&speech("")).is_none());
    }

    #[test]
    fn whitespace_only_text_returns_none() {
        assert!(
            MockSynthesizer::new()
                .synthesize(&speech("   \t\n "))
                .is_none()
        );
    }

    // ---- determinism ----------------------------------------------------

    #[test]
    fn same_text_twice_is_identical() {
        let a = MockSynthesizer::new().synthesize(&speech("hola")).unwrap();
        let b = MockSynthesizer::new().synthesize(&speech("hola")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.samples.len(), b.samples.len());
    }

    #[test]
    fn surrounding_whitespace_does_not_change_length() {
        let a = MockSynthesizer::new().synthesize(&speech("hola")).unwrap();
        let b = MockSynthesizer::new()
            .synthesize(&speech("  hola\n"))
            .unwrap();
        assert_eq!(a.samples.len(), b.samples.len());
    }

    // ---- length contract -------------------------------------------------

    #[test]
    fn two_characters_render_120ms_at_16k_mono() {
        let chunk = MockSynthesizer::new().synthesize(&speech("hi")).unwrap();
        // 2 chars * 60 ms = 120 ms -> 1 920 frames.
        assert_eq!(chunk.samples.len(), 1_920);
        assert_eq!(chunk_duration_ms(&chunk), 120);
        assert_eq!(chunk.sample_rate, MOCK_SAMPLE_RATE);
        assert_eq!(chunk.channels, 1);
        assert!(chunk.is_final);
    }

    #[test]
    fn frame_count_matches_the_documented_formula() {
        assert_eq!(mock_frame_count(0), 0);
        assert_eq!(mock_frame_count(1), 960);
        assert_eq!(mock_frame_count(10), 9_600);
    }

    #[test]
    fn counts_characters_not_bytes() {
        // "ñ" is 2 UTF-8 bytes but 1 character; both must render 60 ms.
        let one = MockSynthesizer::new().synthesize(&speech("ñ")).unwrap();
        let ascii = MockSynthesizer::new().synthesize(&speech("a")).unwrap();
        assert_eq!(one.samples.len(), ascii.samples.len());
    }

    #[test]
    fn samples_stay_within_full_scale() {
        let chunk = MockSynthesizer::new().synthesize(&speech("hola")).unwrap();
        assert!(
            chunk.samples.iter().all(|s| (-1.0..=1.0).contains(s)),
            "mock samples must stay inside -1.0..=1.0"
        );
        assert!(
            chunk.samples.iter().any(|s| *s != 0.0),
            "mock samples must not be silent"
        );
    }
}
