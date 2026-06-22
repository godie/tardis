//! Transcription abstraction + mock + CPAL pipeline + local providers.
//!
//! `transcriber` defines the per-chunk contract (`TranscriptionResult`,
//! trait `Transcriber`). `mock` is the synchronous test / placeholder
//! impl. `mock_local` is a deterministic stub provider that proves
//! [`LocalTranscriptionProvider`] is an open abstraction (returns
//! `"mock transcript for <basename>"`, never touches the network).
//! `pipeline` drives the CPAL capture loop and routes each chunk
//! through any `Transcriber`. `file_pipeline` reads a previously-saved
//! WAV from disk and routes it through the same `Transcriber` without
//! re-running the microphone capture. `local_whisper` is the **first
//! real local transcription provider** in the crate (HTTP-based,
//! OpenAI-compatible endpoint at
//! `http://localhost:8000/v1/audio/transcriptions`, backed by a local
//! Docker stack defined in `docker/faster-whisper/docker-compose.yml`).
//!
//! [`LocalTranscriptionProvider`] is the provider-agnostic trait
//! future providers (`whisper.cpp` binary, cloud APIs, etc.) implement
//! so CLI and downstream callers stay interchangeable.

pub mod file_pipeline;
pub mod live_local;
pub mod local_whisper;
pub mod mock;
pub mod mock_local;
pub mod pipeline;
pub mod transcriber;

use anyhow::Result;

/// Provider-agnostic local transcription contract.
///
/// The first implementation is [`local_whisper::LocalWhisperClient`]
/// (a self-hosted faster-whisper HTTP server). The deterministic
/// stub [`mock_local::MockLocalProvider`] is the second
/// implementation (used for offline / no-Docker development). Future
/// providers (`whisper.cpp` binary, cloud APIs, etc.) implement this
/// trait and are selected at the call site without changing the CLI
/// via [`ProviderKind::build`] / [`build_provider`].
///
/// **Trait-object Debug bound:** `Box<dyn LocalTranscriptionProvider>`
/// is `!Debug` by default — [.unwrap_err()] on a
/// `Result<Box<dyn LocalTranscriptionProvider>>` will not compile
/// because the panic-handler formats the `Ok` variant with Debug.
/// Tests that need to grab the `Err` arm should call
/// `.err().expect(...)` instead, whose Debug chain only sees the
/// concrete error type (e.g. [`anyhow::Error`]).
///
pub trait LocalTranscriptionProvider {
    /// Stable identifier for diagnostics / logging. Examples:
    /// `"local-whisper"` (faster-whisper Docker) or `"whisper-cpp"`
    /// (binary). Used by future provider-selection flags.
    fn name(&self) -> &'static str;

    /// Transcribe a WAV file on disk and return the plaintext.
    ///
    /// Implementations encapsulate the network / process / cloud
    /// call. Callers (CLI, pipelines, future hot-paths) depend only
    /// on this trait — file in, text out.
    fn transcribe(&self, file_path: &str) -> Result<String>;
}

// ===== Provider dispatch =================================================

/// Canonical key for picking a [`LocalTranscriptionProvider`] at
/// runtime. Stable strings match the `--provider` CLI flag value.
///
/// See [`build_provider`] for the high-level entry point and for
/// the canonical error message when an unknown name is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// Self-hosted faster-whisper Docker HTTP server
    /// (OpenAI-compatible). Real speech-to-text; requires the
    /// Docker container running.
    LocalWhisper,
    /// Deterministic stub that echoes
    /// `"mock transcript for <basename>"`. Never touches the
    /// network. Useful for offline dev / tests.
    MockLocal,
}

impl ProviderKind {
    /// Parse a `--provider` flag value. Returns `None` for unknown
    /// names so the caller can format its own error message listing
    /// the valid options (see [`build_provider`]).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "local-whisper" => Some(Self::LocalWhisper),
            "mock-local" => Some(Self::MockLocal),
            _ => None,
        }
    }

    /// Stable CLI / log identifier. Same string the `--provider`
    /// flag accepts — round-trip holds:
    /// `ProviderKind::from_name(kind.name()) == Some(kind)`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::LocalWhisper => "local-whisper",
            Self::MockLocal => "mock-local",
        }
    }

    /// Construct a boxed provider. Reads [`crate::config::LOCAL_WHISPER_*`]
    /// for the faster-whisper variant; the mock variant needs no
    /// configuration.
    pub fn build(&self) -> Box<dyn LocalTranscriptionProvider> {
        match self {
            Self::LocalWhisper => Box::new(local_whisper::LocalWhisperClient::new(
                crate::config::LOCAL_WHISPER_BASE_URL,
                crate::config::LOCAL_WHISPER_MODEL,
                Some(crate::config::LOCAL_WHISPER_LANGUAGE.to_string()),
            )),
            Self::MockLocal => Box::new(mock_local::MockLocalProvider::new()),
        }
    }
}

/// Parse a `--provider` flag value and construct the matching
/// provider. On an unknown name, returns an `anyhow::Error` that
/// names the offending value AND lists the valid options, so the
/// CLI can surface a self-explaining error to the user.
///
/// The CLI defaults to `provider = "local-whisper"` if no flag is
/// passed; this function does NOT layer a default on top — whatever
/// string the caller passes is what gets parsed.
pub fn build_provider(name: &str) -> Result<Box<dyn LocalTranscriptionProvider>> {
    let kind = ProviderKind::from_name(name).ok_or_else(|| {
        // Keep the valid-options list in sync with `ProviderKind::name()`.
        let valid = [
            ProviderKind::LocalWhisper.name(),
            ProviderKind::MockLocal.name(),
        ];
        anyhow::anyhow!(
            "unknown provider '{}'; valid values are: {}",
            name,
            valid.join(", ")
        )
    })?;
    Ok(kind.build())
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_from_name_local_whisper() {
        assert_eq!(
            ProviderKind::from_name("local-whisper"),
            Some(ProviderKind::LocalWhisper)
        );
    }

    #[test]
    fn provider_kind_from_name_mock_local() {
        assert_eq!(
            ProviderKind::from_name("mock-local"),
            Some(ProviderKind::MockLocal)
        );
    }

    #[test]
    fn provider_kind_from_name_unknown_returns_none() {
        assert_eq!(ProviderKind::from_name("nope"), None);
        assert_eq!(ProviderKind::from_name(""), None);
        // The lookup is case-sensitive; "LOCAL-WHISPER" must not match.
        assert_eq!(ProviderKind::from_name("LOCAL-WHISPER"), None);
    }

    #[test]
    fn provider_kind_name_round_trips() {
        // For every variant: kind.name() -> from_name() -> kind.
        // Catches drift if someone renames a variant (e.g.
        // "local-whisper" -> "faster-whisper") without also updating
        // the string mapping.
        for kind in [ProviderKind::LocalWhisper, ProviderKind::MockLocal] {
            assert_eq!(
                ProviderKind::from_name(kind.name()),
                Some(kind),
                "name() -> from_name() round trip failed for {:?}",
                kind
            );
        }
    }

    #[test]
    fn build_provider_local_whisper_yields_real_provider_name() {
        // We can't reach the Docker container from a unit test, but
        // we can confirm dispatch landed on the real implementation
        // by checking the trait's `name()` (LocalWhisperClient says
        // "local-whisper (faster-whisper Docker HTTP, OpenAI-compatible)").
        let provider = build_provider("local-whisper").unwrap();
        let name = provider.name();
        assert!(
            name.contains("local-whisper"),
            "expected real local-whisper provider, got name: {}",
            name
        );
        // And the constructed client must be a LocalWhisperClient
        // (proves ProviderKind::build's match arm ran the right
        // constructor, not the mock one).
        assert!(
            !name.contains("mock"),
            "expected local-whisper provider, got mock: {}",
            name
        );
    }

    #[test]
    fn build_provider_mock_local_echoes_basename_through_dyn_trait() {
        // End-to-end proof that the trait-object dispatch works:
        // build_provider -> Box<dyn Trait> -> .transcribe() -> "...".
        let provider = build_provider("mock-local").unwrap();
        assert_eq!(provider.name(), "mock-local");
        assert_eq!(
            provider.transcribe("a/b/c.wav").unwrap(),
            "mock transcript for c.wav"
        );
    }

    #[test]
    fn build_provider_unknown_returns_error_listing_valid_names() {
        // Use `.err().expect(...)` instead of `.unwrap_err()` so we
        // don't require `Box<dyn LocalTranscriptionProvider>: Debug`
        // — trait objects aren't `Debug` by default, only types that
        // explicitly opt in via `: Debug` supertrait are. The
        // alternate path (`Result::err`) returns `Option<E>` whose
        // Debug is just on `anyhow::Error`, which std already
        // provides.
        let err = build_provider("nope")
            .err()
            .expect("build_provider(\"nope\") should have returned Err, got Ok");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown provider 'nope'"),
            "error should name the offending value, got: {}",
            msg
        );
        assert!(
            msg.contains("local-whisper") && msg.contains("mock-local"),
            "error should list both valid options, got: {}",
            msg
        );
    }
}
