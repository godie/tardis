//! Transcriber abstraction.
//!
//! `TranscriptionResult` is the canonical per-chunk output of any
//! `Transcriber` impl. `Transcriber` is the trait that the mock and
//! future real impls (whisper-local, remote-http, …) implement.
//!
//! The trait is deliberately synchronous, object-safe, and free of
//! CPAL knowledge: it takes `&[f32]` and returns
//! `Option<TranscriptionResult>`. All audio-thread discipline, draining,
//! and source ownership live in the pipeline layer
//! (`transcription::pipeline`), not here.

/// Per-chunk output of a [`Transcriber`].
///
/// Object-safe friendly: only `#[non_exhaustive]`-friendly primitives
/// and a `String`. Cloneable so the pipeline can hand it off / log it
/// without lifetime gymnastics.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionResult {
    /// 1-based chunk index inside the current capture window.
    pub chunk_index: usize,
    /// Recognised text. Verbatim for the mock; model-decoded for real impls.
    pub text: String,
    /// Average amplitude of the chunk on the same scale as
    /// `audio::volume::calculate_average_volume`'s threshold. Useful for
    /// routing or UI display.
    pub average_volume: f32,
    /// `true` for terminal outputs of a chunk. Real streaming impls
    /// may additionally yield `is_final = false` partials while a chunk
    /// is still being decoded; the mock always emits `is_final = true`.
    pub is_final: bool,
}

/// Synchronous per-chunk transcription contract.
///
/// `transcribe_chunk` should be pure with respect to `&self` — the same
/// input chunk + the same impl state must produce the same output.
/// Returning `None` signals "this chunk is silence / no audio usable;
/// skip it", which the pipeline uses to print a silence line instead
/// of a transcript one.
pub trait Transcriber {
    fn transcribe_chunk(&self, chunk_index: usize, samples: &[f32]) -> Option<TranscriptionResult>;
}
