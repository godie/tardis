//! Runtime configuration for the orchestration layer.
//!
//! [`AppRuntimeConfig`] packages user-facing knobs (provider,
//! language pair, chunk size, speech threshold) into a single
//! value the app layer can hold, validate, and pass to
//! downstream services. Raw defaults still live in
//! [`crate::config`] (the centralised constants module); this
//! struct's `Default` reads from there so swapping a default at
//! the factory level propagates to the app layer automatically.
//!
//! [`validate_runtime_config`] is the pure helper called by
//! [`crate::app::service::AppService::new`] before constructing
//! state. It is also unit-testable: callers can build a custom
//! config and check `validate_runtime_config(&cfg)` directly.

use crate::config as central;

/// User-facing knobs owned by the orchestration layer.
///
/// `&AppRuntimeConfig` flows into [`crate::app::state::AppState`]
/// and is copied out into every emitted [`crate::app::events::AppEvent`].
#[derive(Debug, Clone, PartialEq)]
pub struct AppRuntimeConfig {
    /// Stable identifier for the active `Transcriber`
    /// implementation (e.g. `"mock-local"`, `"local-whisper"`).
    pub transcription_provider: String,
    /// BCP-47-ish source language code (e.g. `"en"`).
    pub source_language: String,
    /// BCP-47-ish target language code (e.g. `"es"`).
    pub target_language: String,
    /// Per-chunk duration, in milliseconds, that the capture
    /// pipeline chunks audio into before handing it to the
    /// `Transcriber`. Mirrors [`central::DEFAULT_CHUNK_DURATION_MS`].
    pub chunk_duration_ms: u64,
    /// Speech-vs-silence decision threshold on the same scale
    /// as [`crate::audio::volume::calculate_average_volume`]'s
    /// output. Mirrors [`central::DEFAULT_VOLUME_THRESHOLD`].
    pub volume_threshold: f32,
}

impl Default for AppRuntimeConfig {
    fn default() -> Self {
        Self {
            transcription_provider: "mock-local".to_string(),
            source_language: central::DEFAULT_SOURCE_LANGUAGE.to_string(),
            target_language: central::DEFAULT_TARGET_LANGUAGE.to_string(),
            chunk_duration_ms: central::DEFAULT_CHUNK_DURATION_MS,
            volume_threshold: central::DEFAULT_VOLUME_THRESHOLD,
        }
    }
}

/// Validate an [`AppRuntimeConfig`].
///
/// Returns `Ok(())` on success or `Err(String)` with a
/// user-facing message explaining the **first** violation
/// found. The validator deliberately fails fast (does not
/// collect every error) so the message reads as a single
/// actionable line for the UI.
///
/// Pure — no FS, no HTTP, no CPAL.
pub fn validate_runtime_config(config: &AppRuntimeConfig) -> Result<(), String> {
    if config.transcription_provider.trim().is_empty() {
        return Err("transcription_provider must not be empty or whitespace".to_string());
    }
    if config.source_language.trim().is_empty() {
        return Err("source_language must not be empty or whitespace".to_string());
    }
    if config.target_language.trim().is_empty() {
        return Err("target_language must not be empty or whitespace".to_string());
    }
    if config.chunk_duration_ms == 0 {
        return Err("chunk_duration_ms must be greater than 0".to_string());
    }
    if config.volume_threshold < 0.0 {
        return Err("volume_threshold must be greater than or equal to 0.0".to_string());
    }
    Ok(())
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Default ------------------------------------------------------

    #[test]
    fn default_uses_mock_local_provider() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.transcription_provider, "mock-local");
    }

    #[test]
    fn default_pulls_languages_from_centralised_constants() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.source_language, central::DEFAULT_SOURCE_LANGUAGE);
        assert_eq!(cfg.target_language, central::DEFAULT_TARGET_LANGUAGE);
    }

    #[test]
    fn default_pulls_chunk_duration_from_centralised_constants() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.chunk_duration_ms, central::DEFAULT_CHUNK_DURATION_MS);
    }

    #[test]
    fn default_pulls_threshold_from_centralised_constants() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.volume_threshold, central::DEFAULT_VOLUME_THRESHOLD);
    }

    #[test]
    fn default_is_valid() {
        // The validator must accept the `Default::default()` value
        // so `AppService::new(AppRuntimeConfig::default())` works
        // out of the box.
        let cfg = AppRuntimeConfig::default();
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    // ---- Empty / whitespace language / provider ----------------------

    #[test]
    fn empty_provider_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.transcription_provider = String::new();
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("transcription_provider"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_provider_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.transcription_provider = "   \t\n".to_string();
        assert!(validate_runtime_config(&cfg).is_err());
    }

    #[test]
    fn empty_source_language_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.source_language = String::new();
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("source_language"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_source_language_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.source_language = "   ".to_string();
        assert!(validate_runtime_config(&cfg).is_err());
    }

    #[test]
    fn empty_target_language_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.target_language = String::new();
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("target_language"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_target_language_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.target_language = " ".to_string();
        assert!(validate_runtime_config(&cfg).is_err());
    }

    // ---- chunk_duration_ms -------------------------------------------

    #[test]
    fn zero_chunk_duration_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = 0;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("chunk_duration_ms"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn positive_chunk_duration_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = 500; // 500 ms
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    // ---- volume_threshold --------------------------------------------

    #[test]
    fn negative_threshold_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = -0.0001;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("volume_threshold"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn zero_threshold_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = 0.0;
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "zero threshold must be accepted — it just means 'always speech'"
        );
    }

    #[test]
    fn default_threshold_is_valid() {
        let cfg = AppRuntimeConfig::default();
        assert!(cfg.volume_threshold > 0.0);
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    /// `f32::EPSILON` is the smallest positive `f32`. Boundary:
    /// `cfg.volume_threshold < 0.0` must reject, `>= 0.0` must
    /// accept.
    #[test]
    fn epsilon_threshold_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = f32::EPSILON;
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    /// Failing fast: the first violation in field order is the
    /// one surfaced. We assert it here so the contract is locked
    /// in — if validator order is ever reshuffled, this test
    /// will flag the drift.
    #[test]
    fn validator_fails_fast_on_first_violation() {
        let mut cfg = AppRuntimeConfig::default();
        // Three violations stacked; provider comes first.
        cfg.transcription_provider = String::new();
        cfg.source_language = String::new();
        cfg.volume_threshold = -1.0;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("transcription_provider"),
            "first violation must be reported, got: {err}"
        );
    }
}
