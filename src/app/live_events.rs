//! Pure event-construction helpers for live-capture pipelines.
//!
//! Each function accepts the raw data produced by a live capture stage
//! and returns a typed [`crate::app::events::AppEvent`]. Pipelines
//! call these builders instead of constructing events by hand so the
//! field shapes stay consistent across CLI, Tauri, and future
//! consumers.
//!
//! All helpers are pure (no I/O, no CPAL, no network) and
//! unit-testable in isolation.

use crate::app::events::{AppErrorEvent, AppEvent, AppOutputEvent, AppTranscriptEvent};
use crate::processor::processor::OutputKind;

/// Build an [`AppEvent::Transcript`] from the raw fields.
///
/// # Arguments
///
/// * `chunk_index` — 1-based index of the chunk in the capture window.
/// * `text` — recognised transcript text.
/// * `provider` — stable provider identifier (e.g. `"mock-local"`).
/// * `is_final` — `true` for terminal outputs of a chunk.
pub fn build_transcript_event(
    chunk_index: usize,
    text: String,
    provider: String,
    is_final: bool,
) -> AppEvent {
    AppEvent::Transcript(AppTranscriptEvent {
        chunk_index,
        text,
        provider,
        is_final,
    })
}

/// Build an [`AppEvent::Output`] from the raw fields.
///
/// # Arguments
///
/// * `chunk_index` — 1-based index matching the source transcript.
/// * `kind` — which mode produced the line (`translate` / `respond`).
/// * `source_text` — original transcript text.
/// * `text` — rendered output.
/// * `source_language` — BCP-47-ish source code (e.g. `"en"`).
/// * `target_language` — BCP-47-ish output code (e.g. `"es"`).
/// * `is_final` — `true` for terminal outputs of a call.
pub fn build_output_event(
    chunk_index: usize,
    kind: OutputKind,
    source_text: String,
    text: String,
    source_language: String,
    target_language: String,
    is_final: bool,
) -> AppEvent {
    AppEvent::Output(AppOutputEvent {
        chunk_index,
        kind,
        source_text,
        text,
        source_language,
        target_language,
        is_final,
    })
}

/// Build an [`AppEvent::Error`] from an error message.
pub fn build_error_event(message: impl Into<String>) -> AppEvent {
    AppEvent::Error(AppErrorEvent {
        message: message.into(),
    })
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- build_transcript_event ----------------------------------------

    #[test]
    fn build_transcript_event_creates_transcript_variant() {
        let e = build_transcript_event(1, "hello".to_string(), "mock-local".to_string(), true);
        assert!(matches!(e, AppEvent::Transcript(_)));
    }

    #[test]
    fn transcript_event_includes_chunk_index() {
        let e = build_transcript_event(7, "hi".to_string(), "p".to_string(), true);
        if let AppEvent::Transcript(t) = &e {
            assert_eq!(t.chunk_index, 7);
        } else {
            panic!("expected Transcript, got {e:?}");
        }
    }

    #[test]
    fn transcript_event_includes_provider() {
        let e = build_transcript_event(1, "hi".to_string(), "local-whisper".to_string(), true);
        if let AppEvent::Transcript(t) = &e {
            assert_eq!(t.provider, "local-whisper");
        } else {
            panic!("expected Transcript, got {e:?}");
        }
    }

    #[test]
    fn transcript_event_includes_final_flag() {
        let e = build_transcript_event(1, "hi".to_string(), "p".to_string(), false);
        if let AppEvent::Transcript(t) = &e {
            assert!(!t.is_final);
        } else {
            panic!("expected Transcript, got {e:?}");
        }
    }

    // ---- build_output_event ---------------------------------------

    #[test]
    fn build_output_event_creates_translation_variant() {
        let e = build_output_event(
            1,
            OutputKind::Translate,
            "hello".to_string(),
            "hola".to_string(),
            "en".to_string(),
            "es".to_string(),
            true,
        );
        assert!(matches!(e, AppEvent::Output(_)));
    }

    #[test]
    fn translation_event_includes_languages() {
        let e = build_output_event(
            2,
            OutputKind::Translate,
            "hello".to_string(),
            "hola".to_string(),
            "en".to_string(),
            "fr".to_string(),
            true,
        );
        if let AppEvent::Output(t) = &e {
            assert_eq!(t.source_language, "en");
            assert_eq!(t.target_language, "fr");
        } else {
            panic!("expected Translation, got {e:?}");
        }
    }

    // ---- build_error_event ---------------------------------------------

    #[test]
    fn build_error_event_creates_error_variant() {
        let e = build_error_event("something went wrong");
        assert!(matches!(e, AppEvent::Error(_)));
        if let AppEvent::Error(err) = &e {
            assert_eq!(err.message, "something went wrong");
        } else {
            panic!("expected Error, got {e:?}");
        }
    }
}
