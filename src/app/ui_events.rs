//! UI-safe serializable event payloads for Tauri/frontend.
//!
//! Each [`UiAppEvent`] is a flat struct with optional fields, so
//! the frontend inspects `kind` to decide which fields are
//! populated. [`app_event_to_ui_event`] converts a backend
//! [`crate::app::events::AppEvent`] into this shape.
//!
//! All helpers are pure (no I/O, no CPAL, no network) and
//! unit-testable in isolation.

use crate::app::events::AppEvent;

/// Flat, serializable event payload for the Tauri frontend.
///
/// Every field is `Option` because different event kinds populate
/// different subsets. The frontend inspects `kind` (always
/// present) to decide which optional fields to render.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UiAppEvent {
    /// Always present: `"status"`, `"transcript"`, `"output"`,
    /// `"speech"`, or `"error"`.
    pub kind: String,
    /// Present for `status` events: `"listening"`, `"stopped"`,
    /// `"idle"`, or `"error"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Present for `transcript` and `translation` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_index: Option<usize>,
    /// Present for `transcript` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Present for `transcript`, `output` and `speech` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Present for `translation` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_language: Option<String>,
    /// Present for `output` / `speech` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_language: Option<String>,
    /// Present for `output` events: which mode produced the line
    /// (`"translate"` or `"respond"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_kind: Option<String>,
    /// Present for `error` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Present for `transcript` and `translation` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_final: Option<bool>,
    /// Present for `speech` events: rendered audio length in
    /// milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Convert a backend [`AppEvent`] into a serializable
/// [`UiAppEvent`] payload for the Tauri frontend.
///
/// Pure: no I/O, no allocation beyond the returned struct.
pub fn app_event_to_ui_event(event: &AppEvent) -> UiAppEvent {
    match event {
        AppEvent::StatusChanged(status) => UiAppEvent {
            kind: "status".to_string(),
            status: Some(status.as_label().to_lowercase()),
            chunk_index: None,
            provider: None,
            text: None,
            source_language: None,
            target_language: None,
            output_kind: None,
            message: None,
            is_final: None,
            duration_ms: None,
        },
        AppEvent::Transcript(t) => UiAppEvent {
            kind: "transcript".to_string(),
            status: None,
            chunk_index: Some(t.chunk_index),
            provider: Some(t.provider.clone()),
            text: Some(t.text.clone()),
            source_language: None,
            target_language: None,
            output_kind: None,
            message: None,
            is_final: Some(t.is_final),
            duration_ms: None,
        },
        AppEvent::Output(o) => UiAppEvent {
            kind: "output".to_string(),
            status: None,
            chunk_index: Some(o.chunk_index),
            provider: None,
            text: Some(o.text.clone()),
            source_language: Some(o.source_language.clone()),
            target_language: Some(o.target_language.clone()),
            output_kind: Some(o.kind.as_label().to_string()),
            message: None,
            is_final: Some(o.is_final),
            duration_ms: None,
        },
        AppEvent::Speech(s) => UiAppEvent {
            kind: "speech".to_string(),
            status: None,
            chunk_index: Some(s.chunk_index),
            provider: None,
            text: Some(s.text.clone()),
            source_language: None,
            // `target_language` carries the spoken language so the
            // frontend can reuse the same field it already reads for
            // output events.
            target_language: Some(s.language.clone()),
            output_kind: None,
            message: None,
            is_final: Some(s.is_final),
            duration_ms: Some(s.duration_ms),
        },
        AppEvent::Error(e) => UiAppEvent {
            kind: "error".to_string(),
            status: None,
            chunk_index: None,
            provider: None,
            text: None,
            source_language: None,
            target_language: None,
            output_kind: None,
            message: Some(e.message.clone()),
            is_final: None,
            duration_ms: None,
        },
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::events::{
        AppErrorEvent, AppOutputEvent, AppSpeechEvent, AppStatus, AppTranscriptEvent,
    };
    use crate::processor::processor::OutputKind;

    // ---- app_event_to_ui_event -----------------------------------------

    #[test]
    fn status_event_maps_to_kind_status() {
        let e = AppEvent::StatusChanged(AppStatus::Listening);
        let ui = app_event_to_ui_event(&e);
        assert_eq!(ui.kind, "status");
        assert_eq!(ui.status.as_deref(), Some("listening"));
        // Non-applicable fields are None.
        assert!(ui.text.is_none());
        assert!(ui.provider.is_none());
        assert!(ui.message.is_none());
    }

    #[test]
    fn transcript_event_maps_fields() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 3,
            text: "hello".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        });
        let ui = app_event_to_ui_event(&e);
        assert_eq!(ui.kind, "transcript");
        assert_eq!(ui.chunk_index, Some(3));
        assert_eq!(ui.provider.as_deref(), Some("mock-local"));
        assert_eq!(ui.text.as_deref(), Some("hello"));
        assert_eq!(ui.is_final, Some(true));
        // Non-applicable fields are None.
        assert!(ui.status.is_none());
        assert!(ui.output_kind.is_none());
        assert!(ui.message.is_none());
    }

    #[test]
    fn translation_event_maps_languages() {
        let e = AppEvent::Output(AppOutputEvent {
            chunk_index: 2,
            kind: OutputKind::Translate,
            source_text: "hello".to_string(),
            text: "hola".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        let ui = app_event_to_ui_event(&e);
        assert_eq!(ui.kind, "output");
        assert_eq!(ui.source_language.as_deref(), Some("en"));
        assert_eq!(ui.target_language.as_deref(), Some("es"));
        assert_eq!(ui.text.as_deref(), Some("hola"));
        // Non-applicable fields are None.
        assert!(ui.provider.is_none());
        assert!(ui.message.is_none());
    }

    #[test]
    fn error_event_maps_message() {
        let e = AppEvent::Error(AppErrorEvent {
            message: "connection refused".to_string(),
        });
        let ui = app_event_to_ui_event(&e);
        assert_eq!(ui.kind, "error");
        assert_eq!(ui.message.as_deref(), Some("connection refused"));
        // Non-applicable fields are None.
        assert!(ui.status.is_none());
        assert!(ui.text.is_none());
        assert!(ui.chunk_index.is_none());
    }

    #[test]
    fn speech_event_maps_text_language_and_duration() {
        let e = AppEvent::Speech(AppSpeechEvent {
            chunk_index: 2,
            text: "hola".to_string(),
            language: "es".to_string(),
            duration_ms: 1_240,
            is_final: true,
        });
        let ui = app_event_to_ui_event(&e);
        assert_eq!(ui.kind, "speech");
        assert_eq!(ui.chunk_index, Some(2));
        assert_eq!(ui.text.as_deref(), Some("hola"));
        assert_eq!(ui.target_language.as_deref(), Some("es"));
        assert_eq!(ui.duration_ms, Some(1_240));
        assert_eq!(ui.is_final, Some(true));
        // Non-applicable fields are None.
        assert!(ui.provider.is_none());
        assert!(ui.source_language.is_none());
        assert!(ui.output_kind.is_none());
        assert!(ui.message.is_none());
    }

    #[test]
    fn speech_event_json_has_no_translation_fields() {
        let e = AppEvent::Speech(AppSpeechEvent {
            chunk_index: 1,
            text: "hola".to_string(),
            language: "es".to_string(),
            duration_ms: 60,
            is_final: true,
        });
        let json = serde_json::to_string(&app_event_to_ui_event(&e)).expect("must serialize");
        assert!(json.contains(r#""kind":"speech""#));
        assert!(json.contains(r#""duration_ms":60"#));
        assert!(!json.contains("output_kind"));
        assert!(!json.contains("provider"));
    }

    #[test]
    fn serializable_to_json() {
        let e = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "hi".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        });
        let ui = app_event_to_ui_event(&e);
        let json = serde_json::to_string(&ui).expect("must serialize");
        // Kind is always present.
        assert!(json.contains(r#""kind":"transcript""#));
        // Optional fields only present when populated.
        assert!(json.contains(r#""text":"hi""#));
        // Non-applicable fields are absent from JSON.
        assert!(!json.contains("status"));
        assert!(!json.contains("message"));
    }
}
