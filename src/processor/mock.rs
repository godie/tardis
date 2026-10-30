//! `MockProcessor` — pure per-call processor implementing [`Processor`].
//!
//! Returns `Some(Output)` whose `text` embeds a `[mock <target>]` tag and
//! the original text whenever the input has visible content, or `None`
//! when the input is empty or whitespace-only so the pipeline can
//! short-circuit it.
//!
//! The rendered string is **frozen**: PR 1 of the core-pipeline plan is a
//! rename with no behaviour change, so `text` must stay byte-identical to
//! what `MockTranslator` produced before it, and only the real-provider PR
//! may change what the user sees.

use crate::processor::processor::{Output, OutputKind, ProcessMode, Processor};

/// Mock implementation of [`Processor`] that emits a placeholder
/// translation whenever the input text is non-empty after trimming.
/// Pure, synchronous, object-safe, `Default`-constructible.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MockProcessor;

impl MockProcessor {
    /// Construct a fresh `MockProcessor`.
    pub fn new() -> Self {
        Self
    }
}

impl Processor for MockProcessor {
    fn process(&self, text: &str, mode: &ProcessMode) -> Option<Output> {
        if text.trim().is_empty() {
            return None;
        }
        let ProcessMode::Translate {
            source_language,
            target_language,
        } = mode;
        Some(Output {
            kind: OutputKind::Translate,
            source_text: text.to_string(),
            text: format!("[mock {target_language}] mock translation: \"{text}\""),
            source_language: source_language.clone(),
            target_language: target_language.clone(),
            is_final: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(source: &str, target: &str) -> ProcessMode {
        ProcessMode::Translate {
            source_language: source.to_string(),
            target_language: target.to_string(),
        }
    }

    fn process(text: &str, source: &str, target: &str) -> Option<Output> {
        MockProcessor::new().process(text, &translate(source, target))
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(
            process("", "en", "es").is_none(),
            "empty text must return None"
        );
    }

    #[test]
    fn whitespace_only_input_returns_none() {
        assert!(
            process("   \t\n  ", "en", "es").is_none(),
            "whitespace-only text must return None"
        );
    }

    #[test]
    fn non_empty_input_returns_some() {
        assert!(
            process("speech detected", "en", "es").is_some(),
            "non-empty text must return Some"
        );
    }

    #[test]
    fn result_preserves_source_text() {
        let r = process("hello world", "en", "es").unwrap();
        assert_eq!(r.source_text, "hello world");
    }

    #[test]
    fn result_is_tagged_as_translate() {
        let r = process("hello", "en", "es").unwrap();
        assert_eq!(r.kind, OutputKind::Translate);
    }

    #[test]
    fn result_includes_correct_source_language() {
        let r = process("hello", "en", "es").unwrap();
        assert_eq!(r.source_language, "en");
    }

    #[test]
    fn result_includes_correct_target_language() {
        let r = process("hello", "en", "es").unwrap();
        assert_eq!(r.target_language, "es");
    }

    #[test]
    fn text_includes_target_language() {
        let r = process("hello", "en", "es").unwrap();
        assert!(
            r.text.contains("es"),
            "expected 'es' in output text: {}",
            r.text,
        );
    }

    #[test]
    fn text_includes_original_text() {
        let r = process("hello world", "en", "es").unwrap();
        assert!(
            r.text.contains("hello world"),
            "expected original text in output: {}",
            r.text,
        );
    }

    #[test]
    fn output_string_is_frozen_from_the_translator_era() {
        // Lock-in: renaming Translator -> Processor must not change what
        // the user reads. If this assertion ever needs updating, the
        // change belongs to a provider PR with a visible-output diff.
        let r = process("hello", "en", "es").unwrap();
        assert_eq!(r.text, "[mock es] mock translation: \"hello\"");
    }

    #[test]
    fn is_final_is_true() {
        let r = process("hello", "en", "es").unwrap();
        assert!(r.is_final, "expected is_final = true for mock");
    }

    #[test]
    fn mock_processor_new_works() {
        // Both constructors must produce the same value because
        // MockProcessor is a unit struct, whose only inhabitant is itself.
        assert_eq!(MockProcessor::new(), MockProcessor);
    }
}
