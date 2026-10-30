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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessMode {
    /// Render the utterance in `target_language`. `source_language` is
    /// what the transcriber believes it heard.
    Translate {
        source_language: String,
        target_language: String,
    },
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
}

/// Synchronous per-call processing contract.
///
/// `process` must be pure with respect to `&self`: the same input text +
/// same mode + same impl state must produce the same output.
///
/// `None` is a **normal** outcome, not an error: empty / whitespace-only
/// input, or (in respond mode) a decision rule that says "stay silent".
/// The runner skips the synthesis stage entirely on `None`, so nothing is
/// spoken and no `Speech` event is emitted.
pub trait Processor {
    fn process(&self, text: &str, mode: &ProcessMode) -> Option<Output>;
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
        }
    }
}
