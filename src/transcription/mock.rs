//! `MockTranscriber` — pure per-chunk classifier implementing [`Transcriber`].
//!
//! Returns `Some(TranscriptionResult)` with placeholder text whenever
//! the chunk's average volume exceeds the configured threshold, or
//! `None` when it's at-or-below threshold so the pipeline prints a
//! silence line instead.

use crate::audio::activity::{AudioActivity, classify_audio_activity};
use crate::audio::volume::calculate_average_volume;
use crate::transcription::transcriber::{Transcriber, TranscriptionResult};

/// Mock implementation of [`Transcriber`] that emits a placeholder
/// transcript whenever the chunk's average volume exceeds
/// `volume_threshold`. Pure, synchronous, object-safe.
#[derive(Debug, Clone, PartialEq)]
pub struct MockTranscriber {
    pub volume_threshold: f32,
}

impl MockTranscriber {
    /// Construct a `MockTranscriber` with the given speech threshold
    /// (same scale as [`calculate_average_volume`]).
    pub fn new(volume_threshold: f32) -> Self {
        Self { volume_threshold }
    }
}

impl Transcriber for MockTranscriber {
    fn transcribe_chunk(&self, chunk_index: usize, samples: &[f32]) -> Option<TranscriptionResult> {
        let average_volume = calculate_average_volume(samples);
        match classify_audio_activity(average_volume, self.volume_threshold) {
            AudioActivity::Silence => None,
            AudioActivity::SpeechLike => Some(TranscriptionResult {
                chunk_index,
                text: format!("mock transcript for chunk {chunk_index}: speech detected"),
                average_volume,
                is_final: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loose-equality helper for floating-point assertions. f32 sums
    /// drift — callers pick an epsilon sized to the operation's
    /// expected magnitude.
    fn assert_approx_eq(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "actual: {actual}, expected: {expected}",
        );
    }

    fn transcriber() -> MockTranscriber {
        MockTranscriber::new(crate::config::DEFAULT_VOLUME_THRESHOLD)
    }

    // ---- silence / edge cases ------------------------------------------

    #[test]
    fn silent_chunk_returns_none() {
        let r = transcriber().transcribe_chunk(1, &vec![0.0_f32; 1000]);
        assert!(r.is_none());
    }

    #[test]
    fn empty_chunk_returns_none() {
        let r = transcriber().transcribe_chunk(2, &[]);
        assert!(r.is_none());
    }

    #[test]
    fn low_volume_chunk_returns_none() {
        // avg = 0.005 < 0.01 (threshold)
        let samples = vec![0.005_f32; 1000];
        let r = transcriber().transcribe_chunk(3, &samples);
        assert!(r.is_none());
    }

    #[test]
    fn volume_equal_to_threshold_returns_none() {
        // Single-sample: avg round-trip is exact (no f32 sum drift), so
        // avg == threshold == 0.01 and the boundary must be silence.
        // The multi-sample equivalent (`vec![0.01; 1000]`) accumulates
        // rounding on each `sum +=` and lands slightly above 0.01, which
        // is why we don't use it here.
        let r = transcriber().transcribe_chunk(4, &[0.01_f32]);
        assert!(r.is_none(), "equal-to-threshold must be silence");
    }

    // ---- above-threshold cases -----------------------------------------

    #[test]
    fn voice_like_chunk_above_threshold_returns_some() {
        // avg = 0.05 > 0.01
        let samples = vec![0.05_f32; 1000];
        let r = transcriber().transcribe_chunk(5, &samples);
        assert!(r.is_some());
    }

    #[test]
    fn result_includes_correct_chunk_index() {
        let samples = vec![0.05_f32; 1000];
        let r = transcriber().transcribe_chunk(7, &samples).unwrap();
        assert_eq!(r.chunk_index, 7);
    }

    #[test]
    fn result_text_includes_speech_detected() {
        let samples = vec![0.05_f32; 1000];
        let r = transcriber().transcribe_chunk(8, &samples).unwrap();
        assert!(
            r.text.to_lowercase().contains("speech detected"),
            "expected 'speech detected' in: {}",
            r.text,
        );
    }

    #[test]
    fn result_average_volume_greater_than_threshold() {
        let t = transcriber();
        let samples = vec![0.05_f32; 1000];
        let r = t.transcribe_chunk(9, &samples).unwrap();
        assert!(
            r.average_volume > t.volume_threshold,
            "expected avg {} > threshold {}",
            r.average_volume,
            t.volume_threshold,
        );
    }

    #[test]
    fn result_is_final_is_true() {
        let samples = vec![0.05_f32; 1000];
        let r = transcriber().transcribe_chunk(10, &samples).unwrap();
        assert!(r.is_final, "expected is_final = true for mock");
    }

    // ---- struct construction -------------------------------------------

    #[test]
    fn mock_transcriber_new_stores_threshold() {
        let t = MockTranscriber::new(0.42);
        assert_approx_eq(t.volume_threshold, 0.42, f32::EPSILON);
    }
}
