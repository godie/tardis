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

use crate::processor::decision::{Decision, decide_should_respond};
use crate::processor::guard::validate_reply;
use crate::processor::processor::{BlockReason, Output, OutputKind, ProcessMode, Processor};

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

    /// Deterministic respond-mode output, so the offline loop exercises
    /// the same safety path the real provider runs: decide (layer a),
    /// generate, validate (layer b). The suggestion text is a fixed
    /// string so tests can assert on it.
    fn build_respond_output(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
        decision: Decision,
    ) -> Output {
        let (reply, blocked) = match decision {
            Decision::Ignore => unreachable!("the caller skips Ignore before generating"),
            Decision::Suggest { reason } => (
                format!("[respond suggestion] {reason}"),
                Some(BlockReason::Commitment),
            ),
            Decision::Answer => (
                format!("[mock response] aquí está tu respuesta: \"{text}\""),
                None,
            ),
        };
        let guarded = validate_reply(&reply);
        // Layer a already filtered the input; on the Answer path the
        // guard owns the final word, so its first reason wins.
        let blocked = blocked.or_else(|| guarded.reasons.first().copied());
        Output {
            kind: OutputKind::Respond,
            source_text: text.to_string(),
            text: reply,
            source_language: source_language.to_string(),
            target_language: target_language.to_string(),
            is_final: true,
            blocked,
        }
    }
}

impl Processor for MockProcessor {
    fn process(&self, text: &str, mode: &ProcessMode) -> anyhow::Result<Option<Output>> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        match mode {
            ProcessMode::Translate {
                source_language,
                target_language,
            } => Ok(Some(Output {
                kind: OutputKind::Translate,
                source_text: text.to_string(),
                text: format!("[mock {target_language}] mock translation: \"{text}\""),
                source_language: source_language.clone(),
                target_language: target_language.clone(),
                is_final: true,
                blocked: None,
            })),
            ProcessMode::Respond {
                source_language,
                target_language,
                policy,
            } => match decide_should_respond(text, policy) {
                Decision::Ignore => Ok(None),
                decision => Ok(Some(self.build_respond_output(
                    text,
                    source_language,
                    target_language,
                    decision,
                ))),
            },
        }
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

    /// The respond mode the offline loop runs: host Ana, both safety
    /// knobs on, commitments not allowed.
    fn respond_policy() -> crate::processor::processor::RespondPolicy {
        crate::processor::processor::RespondPolicy {
            host_names: vec!["Ana".to_string()],
            answer_questions_only: true,
            require_name: true,
            allow_commitments: false,
        }
    }

    fn respond(source: &str, target: &str, host_names: &[&str]) -> ProcessMode {
        ProcessMode::Respond {
            source_language: source.to_string(),
            target_language: target.to_string(),
            policy: crate::processor::processor::RespondPolicy {
                host_names: host_names.iter().map(|s| (*s).to_string()).collect(),
                ..respond_policy()
            },
        }
    }

    fn process_respond(text: &str, source: &str, target: &str) -> Option<Output> {
        MockProcessor::new()
            .process(text, &respond(source, target, &["Ana"]))
            .expect("MockProcessor never fails")
    }

    fn process(text: &str, source: &str, target: &str) -> Option<Output> {
        MockProcessor::new()
            .process(text, &translate(source, target))
            .expect("MockProcessor never fails")
    }

    // ---- respond mode (layer a + layer b, offline) --------------------

    #[test]
    fn respond_mode_answers_a_named_question_through_the_guard() {
        let output = process_respond("Ana, how long is the line?", "en", "es")
            .expect("a named question gets an answer");
        assert_eq!(output.kind, OutputKind::Respond);
        assert_eq!(output.blocked, None, "a plain answer must not be blocked");
        assert!(output.text.contains("Ana"));
    }

    #[test]
    fn respond_mode_ignores_everything_without_the_host_name_or_a_question() {
        for text in ["Ana", "¿Qué hora es?", "hola buenos días"] {
            assert!(
                process_respond(text, "es", "es").is_none(),
                "{text:?} must stay silent"
            );
        }
    }

    #[test]
    fn respond_mode_never_speaks_a_commitment_request() {
        let output = process_respond("Ana, can you confirm the price by Friday?", "en", "es")
            .expect("a commitment request surfaces as a suggestion");
        assert_eq!(output.kind, OutputKind::Respond);
        assert_eq!(
            output.blocked,
            Some(BlockReason::Commitment),
            "a suggestion is a blocked output: visible, never spoken"
        );
    }

    #[test]
    fn respond_mode_marks_other_names_as_silence() {
        assert!(process_respond("Pedro, ¿cómo va?", "es", "es").is_none());
    }

    #[test]
    fn respond_mode_returns_none_for_empty_input() {
        assert!(process_respond("   ", "es", "es").is_none());
    }

    #[test]
    fn respond_mode_keeps_is_final_meaning_on_a_blocked_reply() {
        let output = process_respond("Ana, mañana te pago", "es", "es")
            .expect("a commitment surfaces as a suggestion");
        assert!(output.is_final, "is_final keeps its terminal meaning");
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
