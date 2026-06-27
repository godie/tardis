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
//! [`crate::app::service::AppService::new`] and by the Tauri
//! `start_live_transcription` command before any state
//! mutation. It is also unit-testable: callers can build a
//! custom config and check `validate_runtime_config(&cfg)`
//! directly.
//!
//! The `serde` derives let the same struct round-trip through the
//! Tauri IPC boundary so the frontend can present and edit the
//! exact same fields the backend validates and consumes.

use crate::config as central;

/// User-facing knobs owned by the orchestration layer.
///
/// `&AppRuntimeConfig` flows into [`crate::app::state::AppState`]
/// and is copied out into every emitted [`crate::app::events::AppEvent`].
///
/// The `serde` derives let the Tauri shell round-trip the same
/// struct through its IPC boundary; the frontend sends the same
/// shape and the backend validates before using it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Lower bound on `AppRuntimeConfig::chunk_duration_ms`. Below
/// this the CPAL chunk buffer rounds to zero samples on common
/// audio configurations, so the live runner cannot drain a
/// chunk at all. Mirrors the value the spec uses as the
/// recommended minimum for the UI.
pub const MIN_CHUNK_DURATION_MS: u64 = 250;

/// Upper bound on `AppRuntimeConfig::chunk_duration_ms`. This
/// is a sanity ceiling that prevents the UI from accidentally
/// sending a multi-second chunk that would defeat the
/// "chunk-by-chunk live" intent of the live pipeline. Higher
/// values do still build; this is a UX guard, not a hard
/// correctness limit.
pub const MAX_CHUNK_DURATION_MS: u64 = 5000;

/// Upper bound on `AppRuntimeConfig::volume_threshold`. The
/// underlying [`crate::audio::volume::calculate_average_volume`]
/// returns values in `0.0..=1.0` from normalised samples, so
/// anything above `1.0` is meaningless and almost certainly a
/// UI input bug.
pub const MAX_VOLUME_THRESHOLD: f32 = 1.0;

/// Return the canonical list of supported transcription
/// provider names.
///
/// `Vec<&'static str>` so the strings can be borrowed without
/// allocation when the caller just wants to test membership;
/// the Tauri command wrapper collects them into owned
/// `String`s for IPC.
pub fn supported_transcription_providers() -> Vec<&'static str> {
    vec!["mock-local", "local-whisper"]
}

/// Membership test for [`supported_transcription_providers`].
///
/// Returns `true` only when `provider` is exactly one of the
/// supported names. Empty, whitespace-only, or unknown names
/// all return `false`. Comparison is case-sensitive — the live
/// runner validates against the same predicate, and the CLI's
/// `--provider` flag expects the exact string, so anything else
/// is a typo a user-friendly error should surface.
pub fn is_supported_transcription_provider(provider: &str) -> bool {
    supported_transcription_providers().contains(&provider)
}

/// Trim leading / trailing whitespace from the
/// [`AppRuntimeConfig`] string fields.
///
/// Numeric fields pass through unchanged. The returned config
/// is a fresh value so callers that need to keep the original
/// (eg UI bindings) can do so without aliasing.
///
/// Pure — no I/O, no validation. Run this *before*
/// [`validate_runtime_config`] so a `"  "` provider becomes
/// `""` and is caught by the empty-string check rather than
/// silently passing the supported-provider check via a
/// whitespace-prefixed "mock-local".
pub fn normalize_runtime_config(config: AppRuntimeConfig) -> AppRuntimeConfig {
    AppRuntimeConfig {
        transcription_provider: config.transcription_provider.trim().to_string(),
        source_language: config.source_language.trim().to_string(),
        target_language: config.target_language.trim().to_string(),
        chunk_duration_ms: config.chunk_duration_ms,
        volume_threshold: config.volume_threshold,
    }
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
/// Order of checks:
/// 1. `transcription_provider` non-empty (after trimming).
/// 2. `transcription_provider` is one of
///    [`supported_transcription_providers`].
/// 3. `source_language` non-empty.
/// 4. `target_language` non-empty.
/// 5. `chunk_duration_ms > 0` (avoid divide-by-zero in
///    [`crate::audio::chunker::calculate_chunk_size_samples`]).
/// 6. `chunk_duration_ms` within [`MIN_CHUNK_DURATION_MS`] ..
///    [`MAX_CHUNK_DURATION_MS`].
/// 7. `volume_threshold >= 0.0` and `<=` [`MAX_VOLUME_THRESHOLD`].
///
/// *Note:* this validator expects the caller to have already
/// run [`normalize_runtime_config`] on the config so the
/// supported-provider check is not bypassed via whitespace
/// padding. The help message explicitly mentions the supported
/// list so the UI can correct typo'd providers without grepping
/// the source.
///
/// Pure — no FS, no HTTP, no CPAL.
pub fn validate_runtime_config(config: &AppRuntimeConfig) -> Result<(), String> {
    let trimmed_provider = config.transcription_provider.trim();
    if trimmed_provider.is_empty() {
        return Err("transcription_provider must not be empty or whitespace".to_string());
    }
    if !is_supported_transcription_provider(trimmed_provider) {
        return Err(format!(
            "transcription_provider \"{}\" is not supported; valid values are: {}",
            trimmed_provider,
            supported_transcription_providers().join(", ")
        ));
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
    if config.chunk_duration_ms < MIN_CHUNK_DURATION_MS {
        return Err(format!(
            "chunk_duration_ms ({}) is below the minimum of {} ms",
            config.chunk_duration_ms, MIN_CHUNK_DURATION_MS
        ));
    }
    if config.chunk_duration_ms > MAX_CHUNK_DURATION_MS {
        return Err(format!(
            "chunk_duration_ms ({}) is above the maximum of {} ms",
            config.chunk_duration_ms, MAX_CHUNK_DURATION_MS
        ));
    }
    if config.volume_threshold < 0.0 {
        return Err("volume_threshold must be greater than or equal to 0.0".to_string());
    }
    if config.volume_threshold > MAX_VOLUME_THRESHOLD {
        return Err(format!(
            "volume_threshold ({}) is above the maximum of {}",
            config.volume_threshold, MAX_VOLUME_THRESHOLD
        ));
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

    // ---- supported_transcription_providers / supported-membership ----

    #[test]
    fn supported_providers_include_mock_local() {
        let providers = supported_transcription_providers();
        assert!(
            providers.contains(&"mock-local"),
            "supported providers must include 'mock-local', got: {:?}",
            providers
        );
    }

    #[test]
    fn supported_providers_include_local_whisper() {
        let providers = supported_transcription_providers();
        assert!(
            providers.contains(&"local-whisper"),
            "supported providers must include 'local-whisper', got: {:?}",
            providers
        );
    }

    #[test]
    fn supported_providers_lists_exactly_two_entries() {
        // Lock-in test: reordering or adding a provider requires
        // touching this assertion and the matching Tauri command,
        // which is the right friction for adding a provider.
        assert_eq!(
            supported_transcription_providers().len(),
            2,
            "supported_transcription_providers must have exactly 2 entries (mock-local, local-whisper)"
        );
    }

    #[test]
    fn is_supported_accepts_exact_provider_names() {
        assert!(is_supported_transcription_provider("mock-local"));
        assert!(is_supported_transcription_provider("local-whisper"));
    }

    #[test]
    fn is_supported_rejects_empty_and_whitespace() {
        assert!(!is_supported_transcription_provider(""));
        assert!(!is_supported_transcription_provider("   "));
        assert!(!is_supported_transcription_provider("\t"));
    }

    #[test]
    fn is_supported_rejects_unknown_name() {
        assert!(!is_supported_transcription_provider("openai-cloud"));
        assert!(!is_supported_transcription_provider("whisper-local"));
        // Case-sensitive: the live runner keys on the exact string.
        assert!(!is_supported_transcription_provider("MOCK-LOCAL"));
    }

    // ---- normalize_runtime_config ------------------------------------

    #[test]
    fn normalize_trims_provider() {
        let cfg = AppRuntimeConfig {
            transcription_provider: "  mock-local  ".to_string(),
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.transcription_provider, "mock-local");
    }

    #[test]
    fn normalize_trims_languages() {
        let cfg = AppRuntimeConfig {
            source_language: " en\t".to_string(),
            target_language: "\nes ".to_string(),
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.source_language, "en");
        assert_eq!(out.target_language, "es");
    }

    #[test]
    fn normalize_preserves_numeric_fields() {
        // Local vars keep the original numeric values; trimming
        // must not corrupt chunk_duration_ms / volume_threshold.
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: 1234,
            volume_threshold: 0.42,
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.chunk_duration_ms, 1234);
        // f32 equality is exact here because we didn't touch the value.
        assert_eq!(out.volume_threshold, 0.42);
    }

    #[test]
    fn normalize_empty_provider_stays_empty() {
        // The validator separately rejects "empty/whitespace";
        // normalize just trims and lets the validator explain.
        let cfg = AppRuntimeConfig {
            transcription_provider: "   ".to_string(),
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.transcription_provider, "");
        assert!(validate_runtime_config(&out).is_err());
    }

    // ---- validate_runtime_config: supported provider -----------------

    #[test]
    fn unsupported_provider_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.transcription_provider = "openai-cloud".to_string();
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("not supported"),
            "unsupported provider error must say 'not supported', got: {err}"
        );
        // Help message lists the valid options so the user can
        // correct a typo without consulting the source.
        assert!(
            err.contains("mock-local") && err.contains("local-whisper"),
            "unsupported-provider error must list valid options, got: {err}"
        );
    }

    #[test]
    fn padded_supported_provider_is_trimmed_then_valid() {
        // Lock-in: confirm the validator's defensive trim
        // makes the unnormalized path forgiving too. A padded
        // "  mock-local  " is identical to the canonical
        // normalized input — both must validate.
        let cfg = AppRuntimeConfig {
            transcription_provider: "  mock-local  ".to_string(),
            ..AppRuntimeConfig::default()
        };
        let result = validate_runtime_config(&cfg);
        assert!(
            result.is_ok(),
            "padded supported name must validate (validator trims internally), got: {:?}",
            result
        );
        let normalized = normalize_runtime_config(cfg);
        assert_eq!(normalized.transcription_provider, "mock-local");
        assert!(validate_runtime_config(&normalized).is_ok());
    }

    // ---- validate_runtime_config: chunk bounds -----------------------

    #[test]
    fn too_small_chunk_duration_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = MIN_CHUNK_DURATION_MS - 1;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("below the minimum"),
            "too-small chunk error must mention minimum, got: {err}"
        );
        assert!(
            err.contains(&MIN_CHUNK_DURATION_MS.to_string()),
            "too-small chunk error must include the bound value, got: {err}"
        );
    }

    #[test]
    fn too_large_chunk_duration_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = MAX_CHUNK_DURATION_MS + 1;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("above the maximum"),
            "too-large chunk error must mention maximum, got: {err}"
        );
        assert!(
            err.contains(&MAX_CHUNK_DURATION_MS.to_string()),
            "too-large chunk error must include the bound value, got: {err}"
        );
    }

    #[test]
    fn chunk_duration_at_min_boundary_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = MIN_CHUNK_DURATION_MS;
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "chunk_duration_ms exactly at MIN_CHUNK_DURATION_MS must validate"
        );
    }

    #[test]
    fn chunk_duration_at_max_boundary_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = MAX_CHUNK_DURATION_MS;
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "chunk_duration_ms exactly at MAX_CHUNK_DURATION_MS must validate"
        );
    }

    // ---- validate_runtime_config: threshold upper bound --------------

    #[test]
    fn threshold_greater_than_one_is_invalid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = 1.5;
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("above the maximum"),
            "threshold-exceeds-1.0 error must mention maximum, got: {err}"
        );
    }

    #[test]
    fn threshold_at_one_boundary_is_valid() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = 1.0;
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "volume_threshold exactly at MAX_VOLUME_THRESHOLD must validate"
        );
    }
}
