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
/// Which process mode the app runs, as configuration stores it.
///
/// A lean discriminant: the languages stay top-level on
/// [`AppRuntimeConfig`] and are carried into the per-call
/// [`crate::processor::processor::ProcessMode`] by the runner, so there
/// is one source of truth per piece of data. Serializes with a `kind`
/// tag; a `runtime.json` written before this field existed, or with an
/// explicit `null`, loads as translate — today's behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AppMode {
    /// Translate every speech segment into the target language.
    #[default]
    Translate,
    /// Answer as the host, under the two-layer commitment safety.
    Respond {
        /// What the decision step needs: who the host is and what never
        /// gets spoken. Defaults on a respond payload that omits it;
        /// validation then rejects what v1 does not support.
        #[serde(default)]
        policy: crate::processor::processor::RespondPolicy,
    },
}

/// Accepts a `null` mode the way a missing one is accepted: a file may
/// say `"mode": null` and still load as translate, but an unknown
/// `kind` deserializing fails so a corrupt file is reported, not
/// silently demoted.
fn deserialize_mode_or_null<'de, D>(deserializer: D) -> Result<AppMode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Deserialize;
    Option::<AppMode>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// struct through its IPC boundary; the frontend sends the same
/// shape and the backend validates before using it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppRuntimeConfig {
    /// Stable identifier for the active `Transcriber`
    /// implementation (e.g. `"mock-local"`, `"local-whisper"`).
    pub transcription_provider: String,
    /// Stable identifier for the active `Processor` implementation:
    /// `"mock"` (deterministic, offline) or `"llm"` (OpenAI-compatible
    /// chat endpoint). Deserialized with a default so a `runtime.json`
    /// written before the LLM processor existed still loads as the mock.
    #[serde(default = "default_processor_provider")]
    pub processor_provider: String,
    /// Which process mode the session runs. `serde` defaults it so a
    /// `runtime.json` written before it existed, or an explicit `null`,
    /// still loads as translate; an unknown `kind` is a parse error, not
    /// a silent fallback.
    #[serde(default, deserialize_with = "deserialize_mode_or_null")]
    pub mode: AppMode,
    /// Never persisted and never sent by the UI: the CLI's
    /// `--dry-run` turns playback into a counted, silent drain so a
    /// respond session reveals its decisions without a sound. Sessions
    /// started anywhere else are not dry runs.
    #[serde(skip)]
    pub dry_run: bool,
    /// Stable identifier for the active `Synthesizer` implementation:
    /// `"mock"` (deterministic offline tone, the default) or `"piper"`
    /// (the local Docker TTS server). `serde` defaults it: a
    /// `runtime.json` written before the real synthesizer existed
    /// still loads, on the mock.
    #[serde(default = "default_synthesizer_provider")]
    pub synthesizer_provider: String,
    /// Output device preference for the real player, matched
    /// case-insensitively against the device label, in the same
    /// `"<substring>"` form. `None` means "the OS default output
    /// device". Only read when a real player is active (the `piper`
    /// path); the always-on mocks have no device, which is what keeps
    /// CI device-free.
    #[serde(default)]
    pub player_device: Option<String>,
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
    /// Non-speech time, in milliseconds, tolerated inside one
    /// utterance before it closes. Mirrors
    /// [`central::DEFAULT_HANGOVER_MS`]. `serde` defaults it so a
    /// `runtime.json` written before endpointing existed still
    /// loads.
    #[serde(default = "default_hangover_ms")]
    pub hangover_ms: u64,
    /// Shortest burst kept as an utterance, in milliseconds;
    /// anything shorter is noise. Mirrors
    /// [`central::DEFAULT_MIN_SPEECH_MS`].
    #[serde(default = "default_min_speech_ms")]
    pub min_speech_ms: u64,
    /// Burst cap, in milliseconds: an utterance is flushed at this
    /// length even if the speaker never paused. Mirrors
    /// [`central::DEFAULT_MAX_UTTERANCE_MS`].
    #[serde(default = "default_max_utterance_ms")]
    pub max_utterance_ms: u64,
    /// Stable `cpal` device id of the input device to capture from, in the
    /// `"<host>:<device>"` form [`crate::audio::capture::list_input_devices`]
    /// reports and `parse_device_id` parses. `None` means "whichever device
    /// the OS considers default", which is what every capture driver did
    /// before the selector existed. `serde` defaults it so a `runtime.json`
    /// written before the selector still loads.
    ///
    /// The value is an id, not a name: CPAL's enumeration order and, on some
    /// hosts, a device's display name are not stable across replugs and
    /// reboots. An id that no longer resolves (device unplugged, host
    /// changed) falls back to the default with a printed warning instead of
    /// failing the session. Hand-editing a device *name* into `runtime.json`
    /// therefore does nothing but trigger that fallback.
    #[serde(default)]
    pub input_device: Option<String>,
}

fn default_hangover_ms() -> u64 {
    central::DEFAULT_HANGOVER_MS
}

fn default_processor_provider() -> String {
    crate::processor::DEFAULT_PROCESSOR_NAME.to_string()
}

fn default_synthesizer_provider() -> String {
    "mock".to_string()
}

fn default_min_speech_ms() -> u64 {
    central::DEFAULT_MIN_SPEECH_MS
}

fn default_max_utterance_ms() -> u64 {
    central::DEFAULT_MAX_UTTERANCE_MS
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

/// Upper bound on `AppRuntimeConfig::hangover_ms`. A hangover longer
/// than this would keep an utterance open across whole seconds of
/// silence, merging separate turns.
pub const MAX_HANGOVER_MS: u64 = 5_000;

/// Upper bound on `AppRuntimeConfig::min_speech_ms`. Above this almost
/// every real utterance would be discarded as noise.
pub const MAX_MIN_SPEECH_MS: u64 = 5_000;

/// Upper bound on `AppRuntimeConfig::max_utterance_ms`. The cap exists
/// to bound memory; a minute is already generous for one turn.
pub const MAX_MAX_UTTERANCE_MS: u64 = 60_000;

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
/// all return `false`. Comparison is case-sensitive: the live
/// runner validates against the same predicate, and the CLI's
/// `--provider` flag expects the exact string, so anything else
/// is a typo a user-friendly error should surface.
pub fn is_supported_transcription_provider(provider: &str) -> bool {
    supported_transcription_providers().contains(&provider)
}

/// Return the canonical list of supported process-stage
/// ("processor") names.
///
/// Kept next to the transcription list so the UI can rebuild both
/// dropdowns from the backend instead of hardcoding either.
pub fn supported_processor_providers() -> Vec<&'static str> {
    vec!["mock", "llm"]
}

/// Membership test for [`supported_processor_providers`].
///
/// Same contract as [`is_supported_transcription_provider`]:
/// exact, case-sensitive match; empty and unknown names fail.
pub fn is_supported_processor_provider(provider: &str) -> bool {
    supported_processor_providers().contains(&provider)
}

/// Return the canonical list of supported synthesizer provider names.
///
/// `"mock"` is the offline stage that ships with the loop; `"piper"`
/// talks to the local Docker TTS server. Kept next to the other lists
/// so the validator (and a future UI dropdown) rebuild from one place.
pub fn supported_synthesizer_providers() -> Vec<&'static str> {
    vec!["mock", "piper"]
}

/// Membership test for [`supported_synthesizer_providers`].
///
/// Same contract as [`is_supported_transcription_provider`]:
/// exact, case-sensitive match; empty and unknown names fail.
pub fn is_supported_synthesizer_provider(provider: &str) -> bool {
    supported_synthesizer_providers().contains(&provider)
}

/// Trim leading / trailing whitespace from the
/// [`AppRuntimeConfig`] string fields.
///
/// Numeric fields pass through unchanged. The returned config
/// is a fresh value so callers that need to keep the original
/// (eg UI bindings) can do so without aliasing.
///
/// Pure: no I/O, no validation. Run this *before*
/// [`validate_runtime_config`] so a `"  "` provider becomes
/// `""` and is caught by the empty-string check rather than
/// silently passing the supported-provider check via a
/// whitespace-prefixed "mock-local".
pub fn normalize_runtime_config(config: AppRuntimeConfig) -> AppRuntimeConfig {
    AppRuntimeConfig {
        transcription_provider: config.transcription_provider.trim().to_string(),
        processor_provider: config.processor_provider.trim().to_string(),
        synthesizer_provider: config.synthesizer_provider.trim().to_string(),
        source_language: config.source_language.trim().to_string(),
        target_language: config.target_language.trim().to_string(),
        chunk_duration_ms: config.chunk_duration_ms,
        volume_threshold: config.volume_threshold,
        hangover_ms: config.hangover_ms,
        min_speech_ms: config.min_speech_ms,
        max_utterance_ms: config.max_utterance_ms,
        // A whitespace-only id is the UI's empty option in disguise; treat
        // it as "system default" rather than as a device id that can never
        // be resolved.
        input_device: normalize_optional_device(config.input_device),
        player_device: normalize_optional_device(config.player_device),
        mode: normalize_mode(config.mode),
        dry_run: config.dry_run,
    }
}

/// Trims and de-duplicates the respond policy's host names,
/// case-insensitively, keeping the first casing ("  Ana " + "ana" →
/// "Ana"). Nothing else about the mode is rewritten: the flags are
/// validated, not normalized, so a config shipping v1's conservative
/// defaults stays exactly that.
fn normalize_mode(mode: AppMode) -> AppMode {
    match mode {
        AppMode::Translate => AppMode::Translate,
        AppMode::Respond { policy } => {
            let mut seen: Vec<String> = Vec::new();
            let host_names = policy
                .host_names
                .iter()
                .filter_map(|name| {
                    let trimmed = name.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    let key = trimmed.to_lowercase();
                    if seen.contains(&key) {
                        return None;
                    }
                    seen.push(key);
                    Some(trimmed.to_string())
                })
                .collect();
            AppMode::Respond {
                policy: crate::processor::processor::RespondPolicy {
                    host_names,
                    ..policy
                },
            }
        }
    }
}

/// `Some("")` and padded values collapse the way the UI sends its
/// "system default" option: blank becomes `None`, everything else is
/// trimmed.
fn normalize_optional_device(device: Option<String>) -> Option<String> {
    device.and_then(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

impl Default for AppRuntimeConfig {
    fn default() -> Self {
        Self {
            transcription_provider: "mock-local".to_string(),
            processor_provider: crate::processor::DEFAULT_PROCESSOR_NAME.to_string(),
            synthesizer_provider: "mock".to_string(),
            source_language: central::DEFAULT_SOURCE_LANGUAGE.to_string(),
            target_language: central::DEFAULT_TARGET_LANGUAGE.to_string(),
            chunk_duration_ms: central::DEFAULT_CHUNK_DURATION_MS,
            volume_threshold: central::DEFAULT_VOLUME_THRESHOLD,
            hangover_ms: central::DEFAULT_HANGOVER_MS,
            min_speech_ms: central::DEFAULT_MIN_SPEECH_MS,
            max_utterance_ms: central::DEFAULT_MAX_UTTERANCE_MS,
            // `None` = the OS default input device.
            input_device: None,
            // `None` = the OS default output device, and the default
            // synthesizer is the mock, which never opens a device at all.
            player_device: None,
            // Translate is the shipped mode; respond never loads from a
            // legacy file.
            mode: AppMode::Translate,
            // A plain config never dry-runs; only the CLI flag sets it.
            dry_run: false,
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
/// 3. `processor_provider` non-empty (after trimming).
/// 4. `processor_provider` is one of
///    [`supported_processor_providers`].
/// 5. `synthesizer_provider` non-empty (after trimming).
/// 6. `synthesizer_provider` is one of
///    [`supported_synthesizer_providers`]; and `player_device`, when
///    present, is not empty or whitespace
///    ([`normalize_runtime_config`] collapses a blank one to `None`;
///    a whitespace-only value that survives means a malformed
///    config).
/// 7. translate mode has nothing more to check; a respond `mode` must
///    name at least one non-blank host and never err looser than v1
///    (`allow_commitments` must stay `false`, while
///    `answer_questions_only` and `require_name` must stay `true`).
/// 8. `source_language` non-empty.
/// 8. `target_language` non-empty.
/// 9. `chunk_duration_ms > 0` (avoid divide-by-zero in
///    [`crate::audio::chunker::calculate_chunk_size_samples`]).
/// 10. `chunk_duration_ms` within [`MIN_CHUNK_DURATION_MS`] ..
///     [`MAX_CHUNK_DURATION_MS`].
/// 11. `volume_threshold >= 0.0` and `<=` [`MAX_VOLUME_THRESHOLD`].
/// 13. `hangover_ms <=` [`MAX_HANGOVER_MS`] (0 is allowed: close on the
///     first non-speech frame).
/// 14. `min_speech_ms <=` [`MAX_MIN_SPEECH_MS`].
/// 15. `max_utterance_ms > 0`, `<=` [`MAX_MAX_UTTERANCE_MS`] and at
///     least `min_speech_ms`, so a valid utterance can never exceed the
///     cap before it reaches the minimum.
///
/// *Note:* this validator expects the caller to have already
/// run [`normalize_runtime_config`] on the config so the
/// supported-provider check is not bypassed via whitespace
/// padding. The help message explicitly mentions the supported
/// list so the UI can correct typo'd providers without grepping
/// the source.
///
/// Pure: no FS, no HTTP, no CPAL.
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
    let trimmed_processor = config.processor_provider.trim();
    if trimmed_processor.is_empty() {
        return Err("processor_provider must not be empty or whitespace".to_string());
    }
    if !is_supported_processor_provider(trimmed_processor) {
        return Err(format!(
            "processor_provider \"{}\" is not supported; valid values are: {}",
            trimmed_processor,
            supported_processor_providers().join(", ")
        ));
    }
    let trimmed_synthesizer = config.synthesizer_provider.trim();
    if trimmed_synthesizer.is_empty() {
        return Err("synthesizer_provider must not be empty or whitespace".to_string());
    }
    if !is_supported_synthesizer_provider(trimmed_synthesizer) {
        return Err(format!(
            "synthesizer_provider \"{}\" is not supported; valid values are: {}",
            trimmed_synthesizer,
            supported_synthesizer_providers().join(", ")
        ));
    }
    if let Some(device) = config.player_device.as_deref()
        && device.trim().is_empty()
    {
        return Err(
            "player_device must not be empty or whitespace; omit it to select the system default"
                .to_string(),
        );
    }
    if let AppMode::Respond { policy } = &config.mode {
        let host_count = policy
            .host_names
            .iter()
            .filter(|name| !name.trim().is_empty())
            .count();
        if host_count == 0 {
            return Err(
                "mode.policy.host_names must name at least one host in respond mode".to_string(),
            );
        }
        if policy.host_names.iter().any(|name| name.trim().is_empty()) {
            return Err(
                "mode.policy.host_names must not contain empty or whitespace names".to_string(),
            );
        }
        if policy.allow_commitments {
            return Err(
                "allow_commitments is not supported yet; respond mode never speaks commitments"
                    .to_string(),
            );
        }
        if !policy.answer_questions_only {
            return Err(
                "answer_questions_only=false is not supported yet; v1 only answers questions"
                    .to_string(),
            );
        }
        if !policy.require_name {
            return Err(
                "require_name=false is not supported yet; v1 only answers the host".to_string(),
            );
        }
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
    if config.hangover_ms > MAX_HANGOVER_MS {
        return Err(format!(
            "hangover_ms ({}) is above the maximum of {} ms",
            config.hangover_ms, MAX_HANGOVER_MS
        ));
    }
    if config.min_speech_ms > MAX_MIN_SPEECH_MS {
        return Err(format!(
            "min_speech_ms ({}) is above the maximum of {} ms",
            config.min_speech_ms, MAX_MIN_SPEECH_MS
        ));
    }
    if config.max_utterance_ms == 0 {
        return Err("max_utterance_ms must be greater than 0".to_string());
    }
    if config.max_utterance_ms > MAX_MAX_UTTERANCE_MS {
        return Err(format!(
            "max_utterance_ms ({}) is above the maximum of {} ms",
            config.max_utterance_ms, MAX_MAX_UTTERANCE_MS
        ));
    }
    if config.max_utterance_ms < config.min_speech_ms {
        return Err(format!(
            "max_utterance_ms ({}) is below min_speech_ms ({})",
            config.max_utterance_ms, config.min_speech_ms
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
        let cfg = AppRuntimeConfig {
            transcription_provider: String::new(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("transcription_provider"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_provider_is_invalid() {
        let cfg = AppRuntimeConfig {
            transcription_provider: "   \t\n".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_err());
    }

    // ---- Processor provider -------------------------------------------

    #[test]
    fn default_uses_mock_processor() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.processor_provider, "mock");
    }

    #[test]
    fn empty_processor_provider_is_invalid() {
        let cfg = AppRuntimeConfig {
            processor_provider: "  \t".to_string(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("processor_provider"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn unknown_processor_provider_is_invalid_and_lists_the_valid_values() {
        let cfg = AppRuntimeConfig {
            processor_provider: "gpt".to_string(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(err.contains("gpt"), "error must name the value: {err}");
        assert!(err.contains("mock"), "error must list valid values: {err}");
        assert!(err.contains("llm"), "error must list valid values: {err}");
    }

    #[test]
    fn llm_processor_provider_is_valid() {
        let cfg = AppRuntimeConfig {
            processor_provider: "llm".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    #[test]
    fn supported_processor_providers_lists_mock_and_llm() {
        assert_eq!(supported_processor_providers(), vec!["mock", "llm"]);
        assert!(is_supported_processor_provider("mock"));
        assert!(is_supported_processor_provider("llm"));
        assert!(!is_supported_processor_provider("Mock"));
        assert!(!is_supported_processor_provider(""));
    }

    #[test]
    fn normalize_trims_processor_provider() {
        let cfg = AppRuntimeConfig {
            processor_provider: "  llm  ".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert_eq!(normalize_runtime_config(cfg).processor_provider, "llm");
    }

    #[test]
    fn empty_source_language_is_invalid() {
        let cfg = AppRuntimeConfig {
            source_language: String::new(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("source_language"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_source_language_is_invalid() {
        let cfg = AppRuntimeConfig {
            source_language: "   ".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_err());
    }

    #[test]
    fn empty_target_language_is_invalid() {
        let cfg = AppRuntimeConfig {
            target_language: String::new(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("target_language"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn whitespace_target_language_is_invalid() {
        let cfg = AppRuntimeConfig {
            target_language: " ".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_err());
    }

    // ---- chunk_duration_ms -------------------------------------------

    #[test]
    fn zero_chunk_duration_is_invalid() {
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: 0,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("chunk_duration_ms"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn positive_chunk_duration_is_valid() {
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: 500,
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    // ---- volume_threshold --------------------------------------------

    #[test]
    fn negative_threshold_is_invalid() {
        let cfg = AppRuntimeConfig {
            volume_threshold: -0.0001,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("volume_threshold"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn zero_threshold_is_valid() {
        let cfg = AppRuntimeConfig {
            volume_threshold: 0.0,
            ..AppRuntimeConfig::default()
        };
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
        let cfg = AppRuntimeConfig {
            volume_threshold: f32::EPSILON,
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    /// Failing fast: the first violation in field order is the
    /// one surfaced. We assert it here so the contract is locked
    /// in. If validator order is ever reshuffled, this test
    /// will flag the drift.
    #[test]
    fn validator_fails_fast_on_first_violation() {
        // Three violations stacked; provider comes first.
        let cfg = AppRuntimeConfig {
            transcription_provider: String::new(),
            source_language: String::new(),
            volume_threshold: -1.0,
            ..AppRuntimeConfig::default()
        };
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
    fn normalize_trims_input_device() {
        let cfg = AppRuntimeConfig {
            input_device: Some("  coreaudio:builtin  ".to_string()),
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.input_device.as_deref(), Some("coreaudio:builtin"));
    }

    #[test]
    fn normalize_turns_a_blank_input_device_into_none() {
        // A blank name is the UI's "System default" option in disguise, not
        // a device called " " that could never be opened.
        for blank in ["", "   ", "\n\t"] {
            let cfg = AppRuntimeConfig {
                input_device: Some(blank.to_string()),
                ..AppRuntimeConfig::default()
            };
            assert_eq!(normalize_runtime_config(cfg).input_device, None);
        }
    }

    #[test]
    fn default_config_uses_the_system_input_device() {
        assert_eq!(AppRuntimeConfig::default().input_device, None);
    }

    // ---- synthesizer_provider / player_device -------------------------

    #[test]
    fn default_uses_mock_synthesizer_and_no_player_device() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.synthesizer_provider, "mock");
        assert_eq!(cfg.player_device, None);
    }

    #[test]
    fn piper_synthesizer_provider_is_valid() {
        let cfg = AppRuntimeConfig {
            synthesizer_provider: "piper".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    #[test]
    fn empty_synthesizer_provider_is_invalid() {
        let cfg = AppRuntimeConfig {
            synthesizer_provider: "  \t".to_string(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("synthesizer_provider"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn blank_player_device_is_invalid_and_omit_selects_the_system_default() {
        let cfg = AppRuntimeConfig {
            player_device: Some("   ".to_string()),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("player_device"),
            "error should name the offending field, got: {err}"
        );
        assert!(validate_runtime_config(&AppRuntimeConfig::default()).is_ok());
    }

    #[test]
    fn unknown_synthesizer_provider_is_invalid_and_lists_the_valid_values() {
        let cfg = AppRuntimeConfig {
            synthesizer_provider: "kitten".to_string(),
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(err.contains("kitten"), "error must name the value: {err}");
        assert!(err.contains("mock"), "error must list valid values: {err}");
        assert!(err.contains("piper"), "error must list valid values: {err}");
    }

    #[test]
    fn normalize_trims_synthesizer_provider() {
        let cfg = AppRuntimeConfig {
            synthesizer_provider: "  piper  ".to_string(),
            ..AppRuntimeConfig::default()
        };
        assert_eq!(normalize_runtime_config(cfg).synthesizer_provider, "piper");
    }

    #[test]
    fn supported_synthesizer_providers_lists_mock_and_piper() {
        assert_eq!(supported_synthesizer_providers(), vec!["mock", "piper"]);
        assert!(is_supported_synthesizer_provider("mock"));
        assert!(is_supported_synthesizer_provider("piper"));
        assert!(!is_supported_synthesizer_provider("Piper"));
        assert!(!is_supported_synthesizer_provider(""));
    }

    #[test]
    fn normalize_trims_player_device() {
        let cfg = AppRuntimeConfig {
            player_device: Some("  macbook  ".to_string()),
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.player_device.as_deref(), Some("macbook"));
    }

    #[test]
    fn normalize_turns_a_blank_player_device_into_none() {
        // Same contract as `input_device`: a blank value is the UI's
        // "System default" option, not a device named " ".
        for blank in ["", "   ", "\n\t"] {
            let cfg = AppRuntimeConfig {
                player_device: Some(blank.to_string()),
                ..AppRuntimeConfig::default()
            };
            assert_eq!(normalize_runtime_config(cfg).player_device, None);
        }
    }

    #[test]
    fn default_config_uses_the_system_player_device() {
        assert_eq!(AppRuntimeConfig::default().player_device, None);
    }

    /// A `runtime.json` written before the synthesizer fields existed
    /// must still load, on the mock provider with no player device.
    /// Mirrors the `deserialize_ignores_unknown_fields` / legacy test
    /// pattern in `settings_store.rs`.
    #[test]
    fn legacy_config_without_synthesizer_fields_defaults_to_mock() {
        let json = r#"{
            "transcription_provider": "mock-local",
            "source_language": "en",
            "target_language": "es",
            "chunk_duration_ms": 1000,
            "volume_threshold": 0.01
        }"#;
        let cfg: AppRuntimeConfig = serde_json::from_str(json).expect("legacy JSON must load");
        assert_eq!(cfg.synthesizer_provider, "mock");
        assert_eq!(cfg.player_device, None);
        assert!(validate_runtime_config(&cfg).is_ok());
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
        let cfg = AppRuntimeConfig {
            transcription_provider: "openai-cloud".to_string(),
            ..AppRuntimeConfig::default()
        };
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
        // normalized input; both must validate.
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
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: MIN_CHUNK_DURATION_MS - 1,
            ..AppRuntimeConfig::default()
        };
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
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: MAX_CHUNK_DURATION_MS + 1,
            ..AppRuntimeConfig::default()
        };
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
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: MIN_CHUNK_DURATION_MS,
            ..AppRuntimeConfig::default()
        };
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "chunk_duration_ms exactly at MIN_CHUNK_DURATION_MS must validate"
        );
    }

    #[test]
    fn chunk_duration_at_max_boundary_is_valid() {
        let cfg = AppRuntimeConfig {
            chunk_duration_ms: MAX_CHUNK_DURATION_MS,
            ..AppRuntimeConfig::default()
        };
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "chunk_duration_ms exactly at MAX_CHUNK_DURATION_MS must validate"
        );
    }

    // ---- validate_runtime_config: threshold upper bound --------------

    #[test]
    fn threshold_greater_than_one_is_invalid() {
        let cfg = AppRuntimeConfig {
            volume_threshold: 1.5,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("above the maximum"),
            "threshold-exceeds-1.0 error must mention maximum, got: {err}"
        );
    }

    #[test]
    fn threshold_at_one_boundary_is_valid() {
        let cfg = AppRuntimeConfig {
            volume_threshold: 1.0,
            ..AppRuntimeConfig::default()
        };
        assert!(
            validate_runtime_config(&cfg).is_ok(),
            "volume_threshold exactly at MAX_VOLUME_THRESHOLD must validate"
        );
    }

    // ---- endpointing fields ------------------------------------------

    #[test]
    fn default_pulls_endpointing_fields_from_centralised_constants() {
        let cfg = AppRuntimeConfig::default();
        assert_eq!(cfg.hangover_ms, central::DEFAULT_HANGOVER_MS);
        assert_eq!(cfg.min_speech_ms, central::DEFAULT_MIN_SPEECH_MS);
        assert_eq!(cfg.max_utterance_ms, central::DEFAULT_MAX_UTTERANCE_MS);
    }

    #[test]
    fn hangover_at_max_boundary_is_valid() {
        let cfg = AppRuntimeConfig {
            hangover_ms: MAX_HANGOVER_MS,
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    #[test]
    fn hangover_above_max_is_invalid() {
        let cfg = AppRuntimeConfig {
            hangover_ms: MAX_HANGOVER_MS + 1,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("hangover_ms") && err.contains(&MAX_HANGOVER_MS.to_string()),
            "error must name the field and the bound, got: {err}"
        );
    }

    #[test]
    fn zero_hangover_is_valid() {
        // 0 means "close on the first non-speech frame", a legitimate
        // setting for a push-to-talk style session.
        let cfg = AppRuntimeConfig {
            hangover_ms: 0,
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    #[test]
    fn min_speech_above_max_is_invalid() {
        let cfg = AppRuntimeConfig {
            min_speech_ms: MAX_MIN_SPEECH_MS + 1,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("min_speech_ms"),
            "error must name the field, got: {err}"
        );
    }

    #[test]
    fn zero_max_utterance_is_invalid() {
        let cfg = AppRuntimeConfig {
            max_utterance_ms: 0,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("max_utterance_ms"),
            "error must name the field, got: {err}"
        );
    }

    #[test]
    fn max_utterance_above_max_is_invalid() {
        let cfg = AppRuntimeConfig {
            max_utterance_ms: MAX_MAX_UTTERANCE_MS + 1,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("max_utterance_ms") && err.contains(&MAX_MAX_UTTERANCE_MS.to_string()),
            "error must name the field and the bound, got: {err}"
        );
    }

    #[test]
    fn max_utterance_below_min_speech_is_invalid() {
        // The cap would flush before the burst could ever qualify, so every
        // utterance would be discarded: reject the pair instead of silently
        // producing no output.
        let cfg = AppRuntimeConfig {
            min_speech_ms: 1_000,
            max_utterance_ms: 500,
            ..AppRuntimeConfig::default()
        };
        let err = validate_runtime_config(&cfg).unwrap_err();
        assert!(
            err.contains("max_utterance_ms") && err.contains("min_speech_ms"),
            "error must name both fields, got: {err}"
        );
    }

    #[test]
    fn max_utterance_equal_to_min_speech_is_valid() {
        let cfg = AppRuntimeConfig {
            min_speech_ms: 500,
            max_utterance_ms: 500,
            ..AppRuntimeConfig::default()
        };
        assert!(validate_runtime_config(&cfg).is_ok());
    }

    #[test]
    fn normalize_preserves_endpointing_fields() {
        let cfg = AppRuntimeConfig {
            hangover_ms: 0,
            min_speech_ms: 300,
            max_utterance_ms: 20_000,
            ..AppRuntimeConfig::default()
        };
        let out = normalize_runtime_config(cfg);
        assert_eq!(out.hangover_ms, 0);
        assert_eq!(out.min_speech_ms, 300);
        assert_eq!(out.max_utterance_ms, 20_000);
    }
}
