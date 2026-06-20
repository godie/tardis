//! `MockTranslator` — pure per-call translator implementing [`Translator`].
//!
//! Returns `Some(TranslationResult)` whose `translated_text` embeds a
//! `[mock <target>]` tag and the (trimmed) original text whenever the
//! input has visible content, or `None` when the input is empty or
//! whitespace-only so the pipeline can short-circuit it.

use crate::translation::translator::{Translator, TranslationResult};

/// Mock implementation of [`Translator`] that emits a placeholder
/// translation whenever the input text is non-empty after trimming.
/// Pure, synchronous, object-safe, `Default`-constructible.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MockTranslator;

impl MockTranslator {
    /// Construct a fresh `MockTranslator`.
    pub fn new() -> Self {
        Self
    }
}

impl Translator for MockTranslator {
    fn translate_text(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Option<TranslationResult> {
        if text.trim().is_empty() {
            return None;
        }
        Some(TranslationResult {
            source_text: text.to_string(),
            translated_text: format!(
                "[mock {target_language}] mock translation: \"{text}\""
            ),
            source_language: source_language.to_string(),
            target_language: target_language.to_string(),
            is_final: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_none() {
        let r = MockTranslator::new().translate_text("", "en", "es");
        assert!(r.is_none(), "empty text must return None");
    }

    #[test]
    fn whitespace_only_input_returns_none() {
        let r = MockTranslator::new().translate_text("   \t\n  ", "en", "es");
        assert!(r.is_none(), "whitespace-only text must return None");
    }

    #[test]
    fn non_empty_input_returns_some() {
        let r = MockTranslator::new().translate_text("speech detected", "en", "es");
        assert!(r.is_some(), "non-empty text must return Some");
    }

    #[test]
    fn result_preserves_source_text() {
        let r = MockTranslator::new()
            .translate_text("hello world", "en", "es")
            .unwrap();
        assert_eq!(r.source_text, "hello world");
    }

    #[test]
    fn result_includes_correct_source_language() {
        let r = MockTranslator::new()
            .translate_text("hello", "en", "es")
            .unwrap();
        assert_eq!(r.source_language, "en");
    }

    #[test]
    fn result_includes_correct_target_language() {
        let r = MockTranslator::new()
            .translate_text("hello", "en", "es")
            .unwrap();
        assert_eq!(r.target_language, "es");
    }

    #[test]
    fn translated_text_includes_target_language() {
        let r = MockTranslator::new()
            .translate_text("hello", "en", "es")
            .unwrap();
        assert!(
            r.translated_text.contains("es"),
            "expected 'es' in translated_text: {}",
            r.translated_text,
        );
    }

    #[test]
    fn translated_text_includes_original_text() {
        let r = MockTranslator::new()
            .translate_text("hello world", "en", "es")
            .unwrap();
        assert!(
            r.translated_text.contains("hello world"),
            "expected original text in translated_text: {}",
            r.translated_text,
        );
    }

    #[test]
    fn is_final_is_true() {
        let r = MockTranslator::new()
            .translate_text("hello", "en", "es")
            .unwrap();
        assert!(r.is_final, "expected is_final = true for mock");
    }

    #[test]
    fn mock_translator_new_works() {
        // Default-fabricated and `new()`-fabricated instances must be
        // equal because MockTranslator is a unit struct.
        assert_eq!(MockTranslator::new(), MockTranslator::default());
    }
}
