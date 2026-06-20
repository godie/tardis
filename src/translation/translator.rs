//! Translator abstraction.
//!
//! `TranslationResult` is the canonical per-call output of any
//! `Translator` impl. `Translator` is the trait that the mock and
//! future real impls (DeepL, Google Translate, …) implement.
//!
//! The trait is deliberately synchronous, object-safe, and free of
//! CPAL / transcription knowledge: it takes a transcript string plus
//! source/target language codes and returns `Option<TranslationResult>`.
//! All audio-thread discipline, draining, and source ownership live in
//! the pipeline layer (`translation::pipeline`), not here.

/// Per-call output of a [`Translator`].
///
/// Object-safe friendly: only `Clone`-friendly primitives and a
/// `String`. Cloneable so the pipeline can hand it off / log it
/// without lifetime gymnastics.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslationResult {
    /// Original transcript text passed in.
    pub source_text: String,
    /// Translated text. Verbatim mock today; model-decoded for real impls.
    pub translated_text: String,
    /// Source language code (e.g. `"en"`).
    pub source_language: String,
    /// Target language code (e.g. `"es"`).
    pub target_language: String,
    /// `true` for terminal outputs of a call. Real streaming impls may
    /// additionally yield `is_final = false` partials while a chunk is
    /// still being decoded; the mock always emits `is_final = true`.
    pub is_final: bool,
}

/// Synchronous per-call translation contract.
///
/// `translate_text` must be pure with respect to `&self` — the same
/// input text + same impl state must produce the same output.
/// Returning `None` signals "the input is empty / whitespace-only
/// and not a real translation job; skip it", which the pipeline uses
/// to print nothing instead of a translation line.
pub trait Translator {
    fn translate_text(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Option<TranslationResult>;
}
