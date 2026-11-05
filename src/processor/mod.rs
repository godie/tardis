//! Process stage: turn a transcript into the line we will speak.
//!
//! `processor` defines the per-call contract ([`Output`], [`OutputKind`],
//! [`ProcessMode`], trait [`Processor`]). `mock` is the synchronous
//! test/placeholder impl; `llm` is the real one, over an OpenAI-compatible
//! chat endpoint. `pipeline` drives the CPAL capture loop, routes each
//! chunk through any `Transcriber` first, and, when speech is detected,
//! forwards the resulting text through any `Processor` to emit an output
//! line.
//!
//! Translation is one mode of this stage, not a stage of its own: respond
//! mode (answer as the host) plugs into the same trait with a different
//! prompt and acting rule. Both the `Transcriber` and `Processor` traits
//! stay audio-agnostic; only `pipeline` imports CPAL.

pub mod llm;
pub mod mock;
pub mod pipeline;
pub mod prompt;
// `processor::processor` mirrors `transcription::transcriber`: the contract
// lives in a file named after the trait it defines, and call sites read as
// `processor::processor::Processor`. Renaming the file would break that
// symmetry for a naming lint only.
#[allow(clippy::module_inception)]
pub mod processor;

pub use mock::MockProcessor;
pub use processor::{Output, OutputKind, ProcessMode, Processor};

/// Processor used when the config does not name one. Kept as a constant so
/// the CLI, `AppService`, and the live runner cannot drift apart.
pub const DEFAULT_PROCESSOR_NAME: &str = "mock";

/// Canonical key for picking a [`Processor`] at runtime, mirroring
/// [`crate::transcription::ProviderKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorKind {
    /// Deterministic placeholder. Does not touch the network; used by the
    /// CLI smoke modes, `AppService`, and the default runtime config.
    Mock,
    /// Real model over an OpenAI-compatible `/chat/completions` endpoint.
    /// Reads `TARDIS_LLM_API_KEY`; the base URL and model come from
    /// [`crate::config::LLM_*`].
    Llm,
}

impl ProcessorKind {
    /// Parse a `--processor` / `processor_provider` value. Returns `None`
    /// for unknown names so the caller can format its own error listing
    /// the valid options (see [`build_processor`]).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mock" => Some(Self::Mock),
            "llm" => Some(Self::Llm),
            _ => None,
        }
    }

    /// Stable CLI / log identifier. Round-trip holds:
    /// `ProcessorKind::from_name(kind.name()) == Some(kind)`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Llm => "llm",
        }
    }

    /// Construct a boxed processor.
    ///
    /// Fallible because the LLM variant builds an HTTP client; the mock
    /// variant cannot fail. Nothing is sent over the network here: the
    /// server is contacted on the first `process` call, so building a
    /// processor never turns a missing Ollama into a startup error.
    pub fn build(&self) -> anyhow::Result<Box<dyn Processor>> {
        match self {
            Self::Mock => Ok(Box::new(mock::MockProcessor::new())),
            Self::Llm => Ok(Box::new(llm::LlmProcessor::from_env()?)),
        }
    }
}

/// Parse a processor name and construct the matching implementation.
///
/// On an unknown name, returns an `anyhow::Error` that names the offending
/// value AND lists the valid options, so the CLI can surface a
/// self-explaining error.
pub fn build_processor(name: &str) -> anyhow::Result<Box<dyn Processor>> {
    let kind = ProcessorKind::from_name(name).ok_or_else(|| {
        // Keep the valid-options list in sync with `ProcessorKind::name()`.
        let valid = [ProcessorKind::Mock.name(), ProcessorKind::Llm.name()];
        anyhow::anyhow!(
            "unknown processor '{}'; valid values are: {}",
            name,
            valid.join(", ")
        )
    })?;
    kind.build()
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(source: &str, target: &str) -> ProcessMode {
        ProcessMode::Translate {
            source_language: source.to_string(),
            target_language: target.to_string(),
        }
    }

    #[test]
    fn processor_kind_from_name_mock_and_llm() {
        assert_eq!(ProcessorKind::from_name("mock"), Some(ProcessorKind::Mock));
        assert_eq!(ProcessorKind::from_name("llm"), Some(ProcessorKind::Llm));
    }

    #[test]
    fn processor_kind_from_name_unknown_returns_none() {
        assert_eq!(ProcessorKind::from_name("gpt"), None);
        assert_eq!(ProcessorKind::from_name(""), None);
        // Case-sensitive, like the transcription provider lookup.
        assert_eq!(ProcessorKind::from_name("MOCK"), None);
    }

    #[test]
    fn processor_kind_name_round_trips() {
        // Lock-in: every `ProcessorKind` variant must round-trip. Extending
        // the enum means extending this assertion, which is the right
        // friction for adding a processor.
        for kind in [ProcessorKind::Mock, ProcessorKind::Llm] {
            assert_eq!(
                ProcessorKind::from_name(kind.name()),
                Some(kind),
                "name() -> from_name() round trip failed for {kind:?}"
            );
        }
    }

    #[test]
    fn build_processor_returns_a_working_mock() {
        let processor = build_processor("mock").expect("mock must build");
        let output = processor
            .process("hello", &translate("en", "es"))
            .expect("mock never fails")
            .expect("non-empty text must produce output");
        assert_eq!(output.text, "[mock es] mock translation: \"hello\"");
        assert_eq!(output.kind, OutputKind::Translate);
    }

    #[test]
    fn build_processor_builds_the_llm_variant_without_network() {
        // Construction only builds the HTTP client; no request is made
        // until `process`, so this stays offline with no server running.
        let processor = build_processor("llm").expect("the llm processor must build offline");
        assert!(
            processor
                .process("   ", &translate("en", "es"))
                .expect("processor must not fail on blank input")
                .is_none()
        );
    }

    #[test]
    fn build_processor_rejects_unknown_names_with_the_valid_list() {
        let err = match build_processor("gpt") {
            Ok(_) => panic!("unknown processor name must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("gpt"), "error must name the value: {err}");
        assert!(err.contains("mock"), "error must list valid values: {err}");
        assert!(err.contains("llm"), "error must list valid values: {err}");
    }

    #[test]
    fn processor_kind_build_matches_the_trait_contract() {
        let processor = ProcessorKind::Mock.build().expect("mock must build");
        assert!(
            processor
                .process("   ", &translate("en", "es"))
                .expect("processor must not fail on blank input")
                .is_none()
        );
    }
}
