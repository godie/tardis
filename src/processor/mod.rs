//! Process stage: turn a transcript into the line we will speak.
//!
//! `processor` defines the per-call contract ([`Output`], [`OutputKind`],
//! [`ProcessMode`], trait [`Processor`]). `mock` is the synchronous
//! test/placeholder impl. `pipeline` drives the CPAL capture loop, routes
//! each chunk through any `Transcriber` first, and, when speech is
//! detected, forwards the resulting text through any `Processor` to emit
//! an output line.
//!
//! Translation is one mode of this stage, not a stage of its own: respond
//! mode (answer as the host) plugs into the same trait with a different
//! prompt and acting rule. Both the `Transcriber` and `Processor` traits
//! stay audio-agnostic; only `pipeline` imports CPAL.

pub mod mock;
pub mod pipeline;
// `processor::processor` mirrors `transcription::transcriber`: the contract
// lives in a file named after the trait it defines, and call sites read as
// `processor::processor::Processor`. Renaming the file would break that
// symmetry for a naming lint only.
#[allow(clippy::module_inception)]
pub mod processor;

pub use mock::MockProcessor;
pub use processor::{Output, OutputKind, ProcessMode, Processor};

/// Processor used when the config does not name one. Kept as a constant so
/// the CLI, `AppService`, and the live runner cannot drift apart; a real
/// provider PR replaces the call sites, not the constant.
pub const DEFAULT_PROCESSOR_NAME: &str = "mock";

/// Canonical key for picking a [`Processor`] at runtime, mirroring
/// [`crate::transcription::ProviderKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorKind {
    /// Deterministic placeholder. Does not touch the network; used by the
    /// CLI smoke modes, `AppService`, and the default runtime config.
    Mock,
}

impl ProcessorKind {
    /// Parse a `--processor` / `processor_provider` value. Returns `None`
    /// for unknown names so the caller can format its own error listing
    /// the valid options (see [`build_processor`]).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mock" => Some(Self::Mock),
            _ => None,
        }
    }

    /// Stable CLI / log identifier. Round-trip holds:
    /// `ProcessorKind::from_name(kind.name()) == Some(kind)`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
        }
    }

    /// Construct a boxed processor. The mock variant needs no
    /// configuration; the LLM variant lands in a later PR and will read
    /// its base URL / model from [`crate::config`].
    pub fn build(&self) -> Box<dyn Processor> {
        match self {
            Self::Mock => Box::new(mock::MockProcessor::new()),
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
        let valid = [ProcessorKind::Mock.name()];
        anyhow::anyhow!(
            "unknown processor '{}'; valid values are: {}",
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

    fn translate(source: &str, target: &str) -> ProcessMode {
        ProcessMode::Translate {
            source_language: source.to_string(),
            target_language: target.to_string(),
        }
    }

    #[test]
    fn processor_kind_from_name_mock() {
        assert_eq!(ProcessorKind::from_name("mock"), Some(ProcessorKind::Mock));
    }

    #[test]
    fn processor_kind_from_name_unknown_returns_none() {
        assert_eq!(ProcessorKind::from_name("llm"), None);
        assert_eq!(ProcessorKind::from_name(""), None);
        // Case-sensitive, like the transcription provider lookup.
        assert_eq!(ProcessorKind::from_name("MOCK"), None);
    }

    #[test]
    fn processor_kind_name_round_trips() {
        // Lock-in: every `ProcessorKind` variant must round-trip. Extending
        // the enum means extending this assertion, which is the right
        // friction for adding a processor.
        let kind = ProcessorKind::Mock;
        assert_eq!(
            ProcessorKind::from_name(kind.name()),
            Some(kind),
            "name() -> from_name() round trip failed for {kind:?}"
        );
    }

    #[test]
    fn build_processor_returns_a_working_mock() {
        let processor = build_processor("mock").expect("mock must build");
        let output = processor
            .process("hello", &translate("en", "es"))
            .expect("non-empty text must produce output");
        assert_eq!(output.text, "[mock es] mock translation: \"hello\"");
        assert_eq!(output.kind, OutputKind::Translate);
    }

    #[test]
    fn build_processor_rejects_unknown_names_with_the_valid_list() {
        let err = match build_processor("llm") {
            Ok(_) => panic!("unknown processor name must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("llm"), "error must name the value: {err}");
        assert!(err.contains("mock"), "error must list valid values: {err}");
    }

    #[test]
    fn processor_kind_build_matches_the_trait_contract() {
        let processor = ProcessorKind::Mock.build();
        assert!(processor.process("   ", &translate("en", "es")).is_none());
    }
}
