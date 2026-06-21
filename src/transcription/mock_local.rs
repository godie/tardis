//! Pure stub for the [`LocalTranscriptionProvider`] trait.
//!
//! Deterministic fake transcripts — never touches the network, the
//! filesystem, or any external service. Used in unit tests and
//! reserved for a future `--provider` flag (a CLI option that lets
//! callers pick between [`crate::transcription::local_whisper::LocalWhisperClient`]
//! and this stub without changing call sites).
//!
//! Exists primarily to prove that [`crate::transcription::LocalTranscriptionProvider`]
//! is genuinely an open abstraction: any type that impls the trait
//! drops in alongside [`crate::transcription::local_whisper::LocalWhisperClient`]
//! and is reachable from a `Box<dyn LocalTranscriptionProvider>` or
//! any `T: LocalTranscriptionProvider` generic bound — no call site
//! has to distinguish between providers unless it asks for `name()`
//! or matches on the concrete type explicitly.

use std::path::Path;

use anyhow::Result;

use crate::transcription::LocalTranscriptionProvider;

/// Deterministic stub provider.
///
/// * `name()` returns the stable string `"mock-local"`.
/// * `transcribe(file_path)` returns
///   `"mock transcript for <basename>"`, where `<basename>` is
///   [`std::path::Path::file_name`] of `file_path`. If the path has
///   no filename component (e.g. `""` or `"/"`) the literal input
///   string is used as the fallback so the output is still well
///   shaped and the unit tests can compare exact strings.
///
/// Reached from production code by
/// [`crate::transcription::ProviderKind::build`] when the CLI
/// passes `--provider mock-local`. The unit tests below still
/// exercise it directly for trait-object dispatch proof.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MockLocalProvider;

impl MockLocalProvider {
    /// Build a stub provider. Unit struct, so all instances are
    /// equivalent; this constructor exists for symmetry with
    /// [`crate::transcription::local_whisper::LocalWhisperClient::new`]
    /// (so `--provider` dispatch in
    /// [`crate::transcription::ProviderKind::build`] can build both
    /// through the same call shape).
    pub fn new() -> Self {
        Self
    }
}

impl LocalTranscriptionProvider for MockLocalProvider {
    fn name(&self) -> &'static str {
        "mock-local"
    }

    fn transcribe(&self, file_path: &str) -> Result<String> {
        let basename = Path::new(file_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_path.to_string());
        Ok(format!("mock transcript for {}", basename))
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_name_is_mock_local() {
        let p = MockLocalProvider::new();
        assert_eq!(p.name(), "mock-local");
    }

    #[test]
    fn transcribe_echoes_basename_for_relative_path() {
        let p = MockLocalProvider::new();
        let out = p.transcribe("output/chunks/chunk_001.wav").unwrap();
        assert_eq!(out, "mock transcript for chunk_001.wav");
    }

    #[test]
    fn transcribe_echoes_basename_for_absolute_path() {
        let p = MockLocalProvider::new();
        let out = p.transcribe("/var/tardis/chunks/chunk_007.wav").unwrap();
        assert_eq!(out, "mock transcript for chunk_007.wav");
    }

    #[test]
    fn transcribe_handles_directory_path_with_trailing_slash() {
        // `output/chunks/` -> file_name == "chunks" (trailing slash
        // is treated as a directory separator, not as part of the
        // last component).
        let p = MockLocalProvider::new();
        let out = p.transcribe("output/chunks/").unwrap();
        assert_eq!(out, "mock transcript for chunks");
    }

    #[test]
    fn dispatches_through_dyn_trait_object() {
        // The whole point of the trait: a caller holding a
        // `Box<dyn LocalTranscriptionProvider>` can invoke `name`
        // and `transcribe` without knowing the concrete type. If the
        // trait-method signatures ever drift, every future provider
        // consumer breaks too — so this test is the canary for the
        // "openness" guarantee.
        let provider: Box<dyn LocalTranscriptionProvider> = Box::new(MockLocalProvider::new());
        assert_eq!(provider.name(), "mock-local");
        assert_eq!(
            provider.transcribe("a/b/c.wav").unwrap(),
            "mock transcript for c.wav"
        );
    }
}
