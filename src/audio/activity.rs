//! Pure-logic audio-activity classifier.
//!
//! Decides whether a pre-computed `f32` volume represents silence or
//! speech-like activity against a configurable `f32` threshold.
//!
//! Independent of CPAL, audio capture, the device layer, and the
//! transcript / translation pipeline — only consumes a derived
//! `average_volume` and a `volume_threshold` so it can be unit-tested
//! in isolation.
//!
//! Today the boundary is a single `volume > threshold` check, which
//! matches the contract [`crate::transcription::mock::MockTranscriber`]
//! (and every caller that delegates to it) already exercises.
//! Future revisions can swap in real voice-activity detection (VAD)
//! behind the same [`AudioActivity`] enum + helpers without disturbing
//! call sites.

/// Coarse audio-activity classification for a single pre-computed
/// volume sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioActivity {
    /// Volume is at or below the configured threshold — treat as
    /// silence. Mirrors the mock-transcription `average_volume <=
    /// threshold` boundary.
    Silence,
    /// Volume is strictly above the configured threshold — treat as
    /// speech-like activity.
    SpeechLike,
}

/// Is `volume` strictly above `threshold`?
///
/// Equal-to-threshold returns `false`, matching the boundary every
/// `MockTranscriber`-driven pipeline uses today. Pure, no side
/// effects, no CPAL / audio-import dependencies.
pub fn is_speech_like(volume: f32, threshold: f32) -> bool {
    volume > threshold
}

/// Classify a pre-computed `volume` against `threshold`. Boundary is
/// the same as [`is_speech_like`]: `<= threshold` is [`AudioActivity::Silence`],
/// `> threshold` is [`AudioActivity::SpeechLike`].
pub fn classify_audio_activity(volume: f32, threshold: f32) -> AudioActivity {
    if is_speech_like(volume, threshold) {
        AudioActivity::SpeechLike
    } else {
        AudioActivity::Silence
    }
}

/// Human-readable label for an [`AudioActivity`] variant. Useful in
/// log lines and CLI output. Annotated `#[allow(dead_code)]` because no
/// production call site reaches it yet — it's exercised by unit tests
/// today and reserved for future VAD / activity-aware logging changes.
#[allow(dead_code)]
pub fn activity_label(activity: AudioActivity) -> &'static str {
    match activity {
        AudioActivity::Silence => "silence",
        AudioActivity::SpeechLike => "speech-like",
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_speech_like ------------------------------------------------

    #[test]
    fn is_speech_like_below_threshold_is_false() {
        // 0.005 < 0.01; should classify as silence.
        assert!(!is_speech_like(0.005_f32, 0.01_f32));
    }

    #[test]
    fn is_speech_like_equal_to_threshold_is_false() {
        // Boundary: equal-to-threshold must NOT be speech-like — matches
        // the mock transcription contract `average_volume <= threshold`.
        assert!(!is_speech_like(0.01_f32, 0.01_f32));
    }

    #[test]
    fn is_speech_like_above_threshold_is_true() {
        // 0.02 > 0.01; should classify as speech-like.
        assert!(is_speech_like(0.02_f32, 0.01_f32));
    }

    // ---- classify_audio_activity --------------------------------------

    #[test]
    fn classify_below_threshold_returns_silence() {
        assert_eq!(
            classify_audio_activity(0.005_f32, 0.01_f32),
            AudioActivity::Silence,
        );
    }

    #[test]
    fn classify_equal_threshold_returns_silence() {
        // Boundary: equal-to-threshold must classify as Silence.
        assert_eq!(
            classify_audio_activity(0.01_f32, 0.01_f32),
            AudioActivity::Silence,
        );
    }

    #[test]
    fn classify_above_threshold_returns_speech_like() {
        assert_eq!(
            classify_audio_activity(0.02_f32, 0.01_f32),
            AudioActivity::SpeechLike,
        );
    }

    // ---- activity_label -----------------------------------------------

    #[test]
    fn activity_label_silence_returns_silence() {
        assert_eq!(activity_label(AudioActivity::Silence), "silence");
    }

    #[test]
    fn activity_label_speech_like_returns_speech_like() {
        assert_eq!(activity_label(AudioActivity::SpeechLike), "speech-like");
    }
}
