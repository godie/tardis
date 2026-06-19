//! Pure chunk classifier that mimics the future transcription-stage
//! decision: "did this chunk contain speech, or is it silence?".

use crate::audio::volume::calculate_average_volume;

/// Decide whether `samples` should be sent to the (yet-to-exist)
/// real transcription API. Returns `Some(transcript_line)` when the
/// chunk is loud enough, `None` when it's silence.
///
/// The contract is intentionally strict (`<= threshold` → silence):
/// equal-to-threshold chunks are skipped, just above the threshold
/// is enough to pass. Future tuning happens here, without touching
/// the capture loop.
pub fn mock_transcribe_chunk(
    chunk_index: usize,
    samples: &[f32],
    volume_threshold: f32,
) -> Option<String> {
    let avg = calculate_average_volume(samples);
    if avg <= volume_threshold {
        return None;
    }
    Some(format!(
        "[chunk {chunk_index}] mock transcript: speech detected, sending to transcription later..."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linspace(start: f32, end: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| start + (end - start) * (i as f32) / (n as f32)).collect()
    }

    // ---- silence / edge cases ------------------------------------------

    #[test]
    fn silent_chunk_returns_none() {
        let r = mock_transcribe_chunk(1, &vec![0.0_f32; 1000], 0.01);
        assert!(r.is_none());
    }

    #[test]
    fn empty_chunk_returns_none() {
        let r = mock_transcribe_chunk(2, &[], 0.01);
        assert!(r.is_none());
    }

    #[test]
    fn low_volume_chunk_returns_none() {
        // avg = 0.005 < 0.01 (threshold)
        let samples = vec![0.005_f32; 1000];
        let r = mock_transcribe_chunk(3, &samples, 0.01);
        assert!(r.is_none());
    }

    #[test]
    fn volume_equal_to_threshold_returns_none() {
        // Single-sample: avg round-trip is exact (no f32 sum drift), so
        // avg == threshold == 0.01 and the boundary must be silence.
        // The multi-sample equivalent (vec![0.01; 1000]) accumulates
        // rounding on each `sum +=` and lands slightly above 0.01, which
        // is why we don't use it here.
        let r = mock_transcribe_chunk(4, &[0.01_f32], 0.01);
        assert!(r.is_none(), "equal-to-threshold must be silence");
    }

    // ---- above-threshold cases -----------------------------------------

    #[test]
    fn voice_like_chunk_above_threshold_returns_some() {
        // avg = 0.05 > 0.01
        let samples = vec![0.05_f32; 1000];
        let r = mock_transcribe_chunk(5, &samples, 0.01);
        assert!(r.is_some());
    }

    #[test]
    fn returned_text_includes_chunk_index() {
        let samples = vec![0.05_f32; 1000];
        let r = mock_transcribe_chunk(7, &samples, 0.01).unwrap();
        assert!(r.contains("chunk 7"), "expected 'chunk 7' in: {r}");
    }

    #[test]
    fn returned_text_includes_speech_detected_phrase() {
        let samples = vec![0.05_f32; 1000];
        let r = mock_transcribe_chunk(8, &samples, 0.01).unwrap();
        assert!(
            r.to_lowercase().contains("speech detected"),
            "expected 'speech detected' in: {r}"
        );
    }

    // ---- extra coverage ------------------------------------------------

    #[test]
    fn ramped_signal_above_threshold_returns_some() {
        // avg ~= 0.025 > 0.01
        let samples = linspace(0.0, 0.05, 1000);
        let r = mock_transcribe_chunk(9, &samples, 0.01);
        assert!(r.is_some());
    }
}
