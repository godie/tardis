//! App-facing event stream.
//!
//! Types in this module are the canonical surface the
//! orchestration layer emits to its consumers (today: the CLI
//! smoke command; tomorrow: the Tauri webview). They are all
//! `Clone + PartialEq` so callers can drain them into a sink
//! without lifetime gymnastics and so unit tests can compare
//! emitted events to expected fixtures directly.
//!
//! Tests live in `#[cfg(test)] mod tests` per AGENTS.md
//! conventions; the helpers here are pure (no I/O, no CPAL).

/// Coarse-grained lifecycle state of the app.
///
/// `Stopped` and `Error` are terminal sinks; `Idle` and
/// `Listening` are non-terminal. [`is_terminal_status`] is the
/// pure helper for callers that need to short-circuit on
/// terminal variants.
///
/// Distinct from the private `AppStatus` in `src-tauri` so the
/// Tauri shell cannot accidentally couple to the orchestration
/// layer's richer variant set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppStatus {
    Idle,
    Listening,
    Stopped,
    Error,
}

impl AppStatus {
    /// Canonical string label for UI display.
    ///
    /// Kept as a method (not just a free function) so callers
    /// with `&AppStatus` can read it without copying the
    /// variant.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Listening => "Listening",
            Self::Stopped => "Stopped",
            Self::Error => "Error",
        }
    }
}

/// Pure helper: canonical label for an [`AppStatus`] variant.
///
/// Mirrors [`AppStatus::as_label`]; kept as a free function so
/// callers that hold `AppStatus` by copy can still look the
/// label up without taking a reference.
pub fn status_label(status: AppStatus) -> &'static str {
    status.as_label()
}

/// Pure helper: is `status` a terminal sink?
///
/// Terminal means the orchestration has stopped producing
/// non-error events: a future live pipeline can use this to
/// shortcut the capture loop without special-casing each
/// terminal variant.
pub fn is_terminal_status(status: AppStatus) -> bool {
    matches!(status, AppStatus::Stopped | AppStatus::Error)
}

/// Pure helper: stable kind tag for an [`AppEvent`] variant.
///
/// Returns one of `"status"`, `"transcript"`, `"translation"`,
/// or `"error"`. Useful for prefixing console output without
/// matching on every variant at every call site.
pub fn app_event_kind(event: &AppEvent) -> &'static str {
    match event {
        AppEvent::StatusChanged(_) => "status",
        AppEvent::Transcript(_) => "transcript",
        AppEvent::Translation(_) => "translation",
        AppEvent::Error(_) => "error",
    }
}

/// Format an [`AppEvent`] as a single-line console string.
///
/// Pure — no I/O, no allocation beyond the returned `String`.
/// Used by the CLI and eventually by the Tauri shell log sink.
pub fn format_app_event_for_console(event: &AppEvent) -> String {
    match event {
        AppEvent::StatusChanged(status) => {
            format!("[status] {}", status_label(*status).to_lowercase())
        }
        AppEvent::Transcript(t) => {
            format!(
                "[chunk {}][transcript][provider={}] {}",
                t.chunk_index, t.provider, t.text
            )
        }
        AppEvent::Translation(t) => {
            format!(
                "[chunk {}][translation {}->{}] {}",
                t.chunk_index, t.source_language, t.target_language, t.translated_text
            )
        }
        AppEvent::Error(e) => {
            format!("[error] {}", e.message)
        }
    }
}

/// Per-chunk transcript event emitted after the active
/// `Transcriber` produced a transcript.
///
/// Today `AppService::run_mock_text_flow` synthesises one of
/// these per non-empty call; tomorrow the live capture pipeline
/// will emit one per detected chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTranscriptEvent {
    /// 1-based chunk index inside the current capture window.
    /// Today fixed at `1` for synthetic flows.
    pub chunk_index: usize,
    /// Recognised text. Verbatim for the mock; model-decoded
    /// for real impls.
    pub text: String,
    /// Stable provider identifier (e.g. `"mock-local"`,
    /// `"local-whisper"`). Stored as owned `String` so future
    /// runtime-configured providers (custom URLs, dynamic
    /// binary paths) can flow through without `'static`
    /// coupling.
    pub provider: String,
    /// `true` for terminal outputs of a chunk. Real streaming
    /// impls may additionally yield `is_final = false` partials
    /// while the chunk is still being decoded.
    pub is_final: bool,
}

/// Per-call translation event emitted after a successful
/// [`crate::translation::Translator::translate_text`] roundtrip.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTranslationEvent {
    pub chunk_index: usize,
    pub source_text: String,
    pub translated_text: String,
    pub source_language: String,
    pub target_language: String,
    pub is_final: bool,
}

/// Generic error event surfaced when the orchestration layer
/// cannot continue without intervention.
///
/// Today the only emitter is the future CLI / UI layer; the
/// runtime config validator returns `Result<(), String>` rather
/// than emit `AppErrorEvent`. The single `message` field keeps
/// the event shape minimal — a future `AppErrorKind` enum is
/// deliberately out of scope until a second error path lands.
#[derive(Debug, Clone, PartialEq)]
pub struct AppErrorEvent {
    pub message: String,
}

/// Canonical app-facing event stream item.
///
/// `AppState`'s mutators return a single [`AppEvent`] per call;
/// `AppService` returns `Vec<AppEvent>` so a multi-step
/// operation (`run_mock_text_flow`) can emit a transcript and a
/// translation event from one invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    StatusChanged(AppStatus),
    Transcript(AppTranscriptEvent),
    Translation(AppTranslationEvent),
    Error(AppErrorEvent),
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- status_label / AppStatus::as_label ---------------------------

    #[test]
    fn idle_label() {
        assert_eq!(status_label(AppStatus::Idle), "Idle");
        assert_eq!(AppStatus::Idle.as_label(), "Idle");
    }

    #[test]
    fn listening_label() {
        assert_eq!(status_label(AppStatus::Listening), "Listening");
        assert_eq!(AppStatus::Listening.as_label(), "Listening");
    }

    #[test]
    fn stopped_label() {
        assert_eq!(status_label(AppStatus::Stopped), "Stopped");
        assert_eq!(AppStatus::Stopped.as_label(), "Stopped");
    }

    #[test]
    fn error_label() {
        assert_eq!(status_label(AppStatus::Error), "Error");
        assert_eq!(AppStatus::Error.as_label(), "Error");
    }

    // ---- is_terminal_status -------------------------------------------

    #[test]
    fn idle_is_not_terminal() {
        assert!(!is_terminal_status(AppStatus::Idle));
    }

    #[test]
    fn listening_is_not_terminal() {
        assert!(!is_terminal_status(AppStatus::Listening));
    }

    #[test]
    fn stopped_is_terminal() {
        assert!(is_terminal_status(AppStatus::Stopped));
    }

    #[test]
    fn error_is_terminal() {
        assert!(is_terminal_status(AppStatus::Error));
    }

    // ---- PartialEq for event structs ----------------------------------

    #[test]
    fn transcript_events_are_comparable() {
        let a = AppTranscriptEvent {
            chunk_index: 1,
            text: "hello".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        };
        let b = AppTranscriptEvent {
            chunk_index: 1,
            text: "hello".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        };
        let c = AppTranscriptEvent {
            chunk_index: 1,
            text: "hello".to_string(),
            provider: "local-whisper".to_string(),
            is_final: true,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn translation_events_are_comparable() {
        let a = AppTranslationEvent {
            chunk_index: 1,
            source_text: "hello".to_string(),
            translated_text: "[mock es] mock translation: \"hello\"".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        };
        let b = a.clone();
        let mut c = a.clone();
        c.target_language = "fr".to_string();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn error_events_are_comparable() {
        let a = AppErrorEvent {
            message: "boom".to_string(),
        };
        let b = AppErrorEvent {
            message: "boom".to_string(),
        };
        let c = AppErrorEvent {
            message: "different".to_string(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn app_event_variants_are_distinct() {
        // Two AppEvent values built from different variants must
        // not compare equal even if their inner payloads were
        // miraculously identical — the variant tag is part of
        // equality.
        let status = AppEvent::StatusChanged(AppStatus::Idle);
        let error = AppEvent::Error(AppErrorEvent {
            message: "x".to_string(),
        });
        assert_ne!(status, error);

        let transcript = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "x".to_string(),
            provider: "p".to_string(),
            is_final: true,
        });
        let translation = AppEvent::Translation(AppTranslationEvent {
            chunk_index: 1,
            source_text: "x".to_string(),
            translated_text: "y".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        assert_ne!(transcript, translation);
    }

    // ---- app_event_kind -----------------------------------------------

    #[test]
    fn app_event_kind_status() {
        let e = AppEvent::StatusChanged(AppStatus::Listening);
        assert_eq!(app_event_kind(&e), "status");
    }

    #[test]
    fn app_event_kind_transcript() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "hi".to_string(),
            provider: "p".to_string(),
            is_final: true,
        });
        assert_eq!(app_event_kind(&e), "transcript");
    }

    #[test]
    fn app_event_kind_translation() {
        let e = AppEvent::Translation(AppTranslationEvent {
            chunk_index: 1,
            source_text: "hi".to_string(),
            translated_text: "hola".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        assert_eq!(app_event_kind(&e), "translation");
    }

    #[test]
    fn app_event_kind_error() {
        let e = AppEvent::Error(AppErrorEvent {
            message: "boom".to_string(),
        });
        assert_eq!(app_event_kind(&e), "error");
    }

    // ---- format_app_event_for_console ---------------------------------

    #[test]
    fn format_status_event_is_readable() {
        let e = AppEvent::StatusChanged(AppStatus::Listening);
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("listening"),
            "status event should contain 'listening', got: {s}"
        );
    }

    #[test]
    fn format_transcript_event_includes_chunk_index() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 3,
            text: "hello".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("chunk 3"),
            "transcript event should include chunk index, got: {s}"
        );
    }

    #[test]
    fn format_transcript_event_includes_provider() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "hi".to_string(),
            provider: "local-whisper".to_string(),
            is_final: true,
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("local-whisper"),
            "transcript event should include provider, got: {s}"
        );
    }

    #[test]
    fn format_transcript_event_includes_text() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "hello world".to_string(),
            provider: "p".to_string(),
            is_final: true,
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("hello world"),
            "transcript event should include text, got: {s}"
        );
    }

    #[test]
    fn format_translation_event_includes_languages() {
        let e = AppEvent::Translation(AppTranslationEvent {
            chunk_index: 2,
            source_text: "hello".to_string(),
            translated_text: "hola".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("en->es"),
            "translation event should include language pair, got: {s}"
        );
    }

    #[test]
    fn format_translation_event_includes_translated_text() {
        let e = AppEvent::Translation(AppTranslationEvent {
            chunk_index: 1,
            source_text: "hello".to_string(),
            translated_text: "[mock es] mock translation: \"hello\"".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("mock translation"),
            "translation event should include translated text, got: {s}"
        );
    }

    #[test]
    fn format_error_event_includes_message() {
        let e = AppEvent::Error(AppErrorEvent {
            message: "connection refused".to_string(),
        });
        let s = format_app_event_for_console(&e);
        assert!(
            s.contains("connection refused"),
            "error event should include message, got: {s}"
        );
    }
}
