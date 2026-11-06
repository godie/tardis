//! Processor abstraction — the stage between transcription and speech.
//!
//! One pipeline, two modes: `translate` renders the utterance in another
//! language, `respond` answers it as the host. They share [`Processor`]
//! and differ only in prompt, output language, and the acting rule, so
//! the runner never grows a second path.
//!
//! The trait is deliberately synchronous, object-safe, and free of CPAL /
//! transcription knowledge: it takes a transcript string plus a
//! [`ProcessMode`] and returns `Option<Output>`. All audio-thread
//! discipline, draining, and source ownership live in the pipeline layer
//! (`processor::pipeline` and `transcription::live_local`), not here.

/// Which mode produced an [`Output`].
///
/// Travels with the result so the event layer never has to infer it from
/// the language pair or from configuration that may have changed since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// Real-time translation into the configured target language.
    Translate,
    /// Answer as the host, in the utterance's own language. Reserved for
    /// the respond-mode PR; no runner emits it yet.
    Respond,
}

impl OutputKind {
    /// Stable lowercase label for logs, payloads, and the UI.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Translate => "translate",
            Self::Respond => "respond",
        }
    }
}

/// How the processor should treat the incoming utterance.
///
/// Passed per call rather than stored in the implementation: the same
/// processor serves every mode, and switching modes becomes a config
/// change instead of a rebuild.
///
/// The languages ride with the mode as they already ride with
/// `Translate`: the config persists the mode discriminant
/// (`app::config::AppRuntimeConfig`'s `mode` field) and the languages
/// stay top-level — this enum carries the pair for the call, plus the
/// respond policy, so the runner does not grow a second path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessMode {
    /// Render the utterance in `target_language`. `source_language` is
    /// what the transcriber believes it heard.
    Translate {
        source_language: String,
        target_language: String,
    },
    /// Answer the utterance as the host would. In respond mode the
    /// output language *is* the host's own `target_language`, so the
    /// pair is the same kind of payload `Translate` takes; [`RespondPolicy`]
    /// carries what the safety layers need.
    Respond {
        source_language: String,
        target_language: String,
        policy: RespondPolicy,
    },
}

/// Acting policy for ([`ProcessMode::Respond`]).
///
/// v1 defaults: conservative. `answer_questions_only` and
/// `require_name` default to `true`, and config validation rejects
/// anything looser until there is evidence to change; `allow_commitments`
/// is a product promise, not a TODO — validation rejects `true` and the
/// decision layer treats every commitment-shaped request as a
/// suggestion regardless of the flag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RespondPolicy {
    /// Who the host is. Matched case-insensitively, on a word boundary.
    pub host_names: Vec<String>,
    /// Only questions get answers. `true` in v1; validation rejects
    /// `false`.
    pub answer_questions_only: bool,
    /// Only utterances that name the host get answered. `true` in v1;
    /// validation rejects `false`.
    pub require_name: bool,
    /// Speaking commitments is deliberately not supported in v1;
    /// `true` is rejected by config validation.
    pub allow_commitments: bool,
}

/// Why a generated reply is blocked (layer b, [`crate::processor::guard`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The request asked for a commitment; the reply is at most a
    /// suggestion, never spoken.
    Commitment,
    /// The reply commits to a date or a booking window.
    Date,
    /// The reply names an amount or a currency.
    Money,
    /// The reply accepts or promises (`claro`, `I'll`, `lo haré`, `deal`).
    Acceptance,
    /// Nothing to say: empty or whitespace.
    Empty,
}

impl BlockReason {
    /// Stable lowercase label for logs, payloads, and the UI.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Commitment => "commitment",
            Self::Date => "date",
            Self::Money => "money",
            Self::Acceptance => "acceptance",
            Self::Empty => "empty",
        }
    }
}

/// One processed utterance.
///
/// Object-safe friendly: only `Clone`-friendly primitives and `String`s.
/// Cloneable so the pipeline can hand it off / log it without lifetime
/// gymnastics.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Which mode produced this line.
    pub kind: OutputKind,
    /// Original transcript text passed in.
    pub source_text: String,
    /// Rendered output (translation today; the host's answer in respond
    /// mode). Verbatim mock today; model-decoded for real impls.
    pub text: String,
    /// Source language code (e.g. `"en"`).
    pub source_language: String,
    /// Output language code (e.g. `"es"`), in both modes.
    pub target_language: String,
    /// `true` for terminal outputs of a call. Real streaming impls may
    /// additionally yield `is_final = false` partials while the utterance
    /// is still being rendered; the mock always emits `is_final = true`.
    pub is_final: bool,
    /// Why the reply must not be spoken, from the safety guard
    /// ([`crate::processor::guard`]) — `None` for an allowed reply.
    /// A blocked reply still reaches the event layer (silent in the
    /// room, visible on screen); `is_final` keeps exactly the meaning
    /// above and is never reused as a safety flag.
    pub blocked: Option<BlockReason>,
}

/// Synchronous per-call processing contract.
///
/// Implementations must be pure with respect to `&self`: no interior
/// mutation of the processor's own state. Determinism is required of the
/// mocks, because tests assert on their exact output; a real provider
/// (`processor::llm`) is exempt, since a model is not reproducible.
///
/// The return type separates silence from failure:
///
/// - `Ok(None)`: empty or whitespace-only input, or a respond-mode decision
///   that says "do not act". The runner skips synthesis and emits nothing.
/// - `Err(..)`: transport, auth, or an empty model reply. The runner emits
///   `AppEvent::Error` and still skips synthesis, so a failed request is
///   not reported as silence.
pub trait Processor {
    fn process(&self, text: &str, mode: &ProcessMode) -> anyhow::Result<Option<Output>>;
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_output() -> Output {
        Output {
            kind: OutputKind::Translate,
            source_text: "hello".to_string(),
            text: "hola".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
            blocked: None,
        }
    }

    #[test]
    fn output_kind_labels_are_stable() {
        assert_eq!(OutputKind::Translate.as_label(), "translate");
        assert_eq!(OutputKind::Respond.as_label(), "respond");
    }

    #[test]
    fn outputs_are_comparable() {
        let a = sample_output();
        let b = a.clone();
        let mut c = a.clone();
        c.kind = OutputKind::Respond;
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn process_mode_carries_both_languages() {
        let mode = ProcessMode::Translate {
            source_language: "en".to_string(),
            target_language: "fr".to_string(),
        };
        match mode {
            ProcessMode::Translate {
                source_language,
                target_language,
            } => {
                assert_eq!(source_language, "en");
                assert_eq!(target_language, "fr");
            }
            ProcessMode::Respond { .. } => unreachable!("the fixture is a Translate mode"),
        }
    }

    #[test]
    fn process_mode_respond_carries_languages_and_policy() {
        let mode = ProcessMode::Respond {
            source_language: "es".to_string(),
            target_language: "es".to_string(),
            policy: RespondPolicy {
                host_names: vec!["Ana".to_string()],
                answer_questions_only: true,
                require_name: true,
                allow_commitments: false,
            },
        };
        match mode {
            ProcessMode::Respond {
                source_language,
                target_language,
                policy,
            } => {
                assert_eq!(source_language, "es");
                assert_eq!(target_language, "es");
                assert_eq!(policy.host_names, vec!["Ana".to_string()]);
                assert!(policy.require_name && policy.answer_questions_only);
                assert!(!policy.allow_commitments);
            }
            ProcessMode::Translate { .. } => unreachable!("the fixture is a Respond mode"),
        }
    }

    #[test]
    fn block_reason_labels_are_stable() {
        for (reason, label) in [
            (BlockReason::Commitment, "commitment"),
            (BlockReason::Date, "date"),
            (BlockReason::Money, "money"),
            (BlockReason::Acceptance, "acceptance"),
            (BlockReason::Empty, "empty"),
        ] {
            assert_eq!(reason.as_label(), label);
        }
    }
}
