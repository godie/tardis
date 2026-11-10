//! Transcript session model — pure aggregation of transcript /
//! translation [`crate::app::events::AppEvent`]s into a single
//! exportable value.
//!
//! A [`TranscriptSession`] is the canonical "what happened this
//! run" surface for:
//! - the Tauri shell's "Export JSON" / "Export Text" buttons
//!   (read once at the moment the user clicks)
//! - the future `cargo run -- session-export-demo` smoke command
//!
//! It is intentionally **not** the live-capture state machine
//! — that lives in [`crate::app::state::AppState`] and on
//! `start_live_transcription`'s on-screen panels. This module
//! is the **aggregation sink**: events get folded into it via
//! [`apply_app_event_to_session`] / [`merge_output_into_entries`]
//! and the resulting struct is round-tripped through
//! [`serde_json`] (or pretty-printed text) only at export time.
//!
//! All helpers are pure (no I/O, no CPAL, no Tauri runtime) and
//! unit-tested in isolation per the
//! [`crate::app`] / AGENTS.md testing contract. Tests live in
//! `#[cfg(test)] mod tests` at the bottom of this file.

use crate::app::events::{AppEvent, AppOutputEvent, AppTranscriptEvent};

/// A single per-chunk entry inside a [`TranscriptSession`].
///
/// One entry covers a chunk index and carries whichever fields
/// are populated at the moment of fold-in. Both lifecycle
/// ordering (transcript first, translation second, or
/// translation-only when a translation arrives without a prior
/// transcript event) are representable:
///
/// - Transcript-then-output: both `transcript_text`
///   and `translated_text` populated.
/// - Output-only: only `translated_text` populated;
///   `transcript_text` stays `None`. Useful for tests and for
///   respond-mode lines that arrive without a prior transcript
///   event.
///
/// All five optional fields are `Option<String>` / `Option<bool>`
/// so a future structured serialisation round-trips cleanly
/// without placeholder defaults.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSessionEntry {
    /// 1-based chunk index inside the current capture window.
    pub chunk_index: usize,
    /// Recognised source text. `None` if only a translation
    /// event was folded into this entry.
    pub transcript_text: Option<String>,
    /// Translated output. `None` until a matching translation
    /// event arrives.
    pub translated_text: Option<String>,
    /// Source language tag the translation was computed for.
    /// Mirrors [`crate::app::events::AppOutputEvent::source_language`].
    pub source_language: Option<String>,
    /// Target language tag the translation was computed for.
    /// Mirrors [`crate::app::events::AppOutputEvent::target_language`].
    pub target_language: Option<String>,
    /// Provider identifier that produced the transcript (when
    /// this entry carries one). Mirrors
    /// [`crate::app::events::AppTranscriptEvent::provider`].
    pub provider: Option<String>,
    /// `true` for terminal outputs of a chunk. Real streaming
    /// implementations may additionally yield `is_final = false`
    /// partials.
    pub is_final: bool,
}

/// A complete transcript session — the export-ready shape
/// handed to [`crate::app::session_export::session_to_json`]
/// and [`crate::app::session_export::session_to_plain_text`].
///
/// `started_at` / `ended_at` are plain `String` (typically
/// RFC-3339-ish or simply `"current"`) so the model stays
/// dependency-free — the project does not depend on `chrono`
/// or `time`, so a caller-supplied timestamp string is the
/// lightest contract that still round-trips through serde.
///
/// `entries` are intentionally append-only: every translate
/// after the matching transcript mutates the same entry in
/// place via [`merge_output_into_entries`] rather than
/// appending a duplicate, so consumer code (Tauri UI list
/// views, future session history browsers) can rely on
/// `entries[i].chunk_index` being monotonic with `i`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSession {
    /// Stable session identifier. The Tauri shell uses
    /// `"current"`; the CLI smoke command uses `"demo"`.
    pub session_id: String,
    /// Provider name from the active [`crate::app::config::AppRuntimeConfig`].
    pub provider: String,
    /// Source language from the active [`crate::app::config::AppRuntimeConfig`].
    pub source_language: String,
    /// Target language from the active [`crate::app::config::AppRuntimeConfig`].
    pub target_language: String,
    /// Caller-supplied timestamp string captured at session
    /// creation.
    pub started_at: String,
    /// `Some(_)` once the session is closed (e.g. `stop_live_transcription`
    /// in the Tauri shell); `None` while a session is still open.
    pub ended_at: Option<String>,
    /// Per-chunk transcript / translation entries in arrival
    /// order. Mutated in place by [`apply_app_event_to_session`]
    /// and [`merge_output_into_entries`].
    pub entries: Vec<TranscriptSessionEntry>,
}

/// Build a fresh [`TranscriptSession`] with empty `entries`
/// and `ended_at = None`.
///
/// Pure — no I/O. Test-friendly by construction: every field
/// the caller supplies lands verbatim on the resulting struct.
pub fn create_session(
    session_id: String,
    provider: String,
    source_language: String,
    target_language: String,
    started_at: String,
) -> TranscriptSession {
    TranscriptSession {
        session_id,
        provider,
        source_language,
        target_language,
        started_at,
        ended_at: None,
        entries: Vec::new(),
    }
}

/// Map an [`AppTranscriptEvent`] onto a [`TranscriptSessionEntry`].
///
/// The `source_language` / `target_language` fields stay `None`
/// because `AppTranscriptEvent` does not carry per-chunk language
/// metadata — that arrives with the matching `AppTranslationEvent`
/// and is folded in via [`merge_output_into_entries`].
pub fn transcript_entry_from_event(event: &AppTranscriptEvent) -> TranscriptSessionEntry {
    TranscriptSessionEntry {
        chunk_index: event.chunk_index,
        transcript_text: Some(event.text.clone()),
        translated_text: None,
        source_language: None,
        target_language: None,
        provider: Some(event.provider.clone()),
        is_final: event.is_final,
    }
}

/// Fold an [`AppTranscriptEvent`] into `entries` mirroring
/// [`merge_output_into_entries`]'s merge-first policy.
///
/// Three outcomes, in order:
///
/// 1. **No matching entry** → append a fresh entry via
///    [`transcript_entry_from_event`] (the standard path).
/// 2. **Matching entry exists with `transcript_text.is_none()`**
///    (i.e. a translation arrived earlier for the same chunk):
///    fill in `transcript_text` / `provider` / `is_final` in
///    place. The translation fields set by the prior event are
///    retained — out-of-order events do not produce duplicate
///    entries.
/// 3. **Matching entry already has a transcript_text** (the
///    common case where a duplicate transcript event arrives
///    after the canonical transcript + translation pair have
///    been recorded): **no-op**. Producing a second entry would
///    create a duplicate `chunk_index` row in the export, and
///    overwriting the existing transcript_text would silently
///    drop the canonical translation's source. Practically no
///    pipeline emits twice, but the trade-off is locked here so
///    a future audio resend cannot corrupt a recorded entry.
///
/// Pure — `&mut Vec<...>` only.
pub fn merge_transcript_into_entries(
    entries: &mut Vec<TranscriptSessionEntry>,
    transcript: &AppTranscriptEvent,
) {
    match entries
        .iter_mut()
        .find(|e| e.chunk_index == transcript.chunk_index)
    {
        // Case 2: translation-only entry exists — fill in place.
        Some(existing) if existing.transcript_text.is_none() => {
            existing.transcript_text = Some(transcript.text.clone());
            existing.provider = Some(transcript.provider.clone());
            existing.is_final = transcript.is_final;
        }
        // Case 3: full entry already exists — leave it untouched.
        Some(_) => {
            // Intentionally ignore the duplicate transcript
            // event. See the case-discussion note above.
        }
        // Case 1: no matching entry — append a fresh one.
        None => {
            entries.push(transcript_entry_from_event(transcript));
        }
    }
}

/// Map an [`AppTranslationEvent`] onto a
/// [`TranscriptSessionEntry`] — translation-only.
///
/// Used by [`merge_output_into_entries`] when no matching
/// transcript entry yet exists. The result is structurally a
/// [`TranscriptSessionEntry`] with `transcript_text = None` so
/// the downstream consumer can distinguish "translation
/// arrived first" from "transcript arrived first".
pub fn output_entry_from_event(event: &AppOutputEvent) -> TranscriptSessionEntry {
    TranscriptSessionEntry {
        chunk_index: event.chunk_index,
        transcript_text: None,
        translated_text: Some(event.text.clone()),
        source_language: Some(event.source_language.clone()),
        target_language: Some(event.target_language.clone()),
        provider: None,
        is_final: event.is_final,
    }
}

/// Fold an [`AppTranslationEvent`] into `entries`.
///
/// - If an entry with the same `chunk_index` exists, mutate it
///   in place: copy `translated_text`, `source_language`,
///   `target_language`, and `is_final` from the translation
///   event. `transcript_text` and `provider` are left as-is
///   (they were set by the prior transcript event).
/// - Otherwise, push a translation-only entry to the back of
///   `entries` via [`output_entry_from_event`].
///
/// Pure — `&mut Vec<...>` only.
pub fn merge_output_into_entries(
    entries: &mut Vec<TranscriptSessionEntry>,
    output: &AppOutputEvent,
) {
    if let Some(existing) = entries
        .iter_mut()
        .find(|e| e.chunk_index == output.chunk_index)
    {
        existing.translated_text = Some(output.text.clone());
        existing.source_language = Some(output.source_language.clone());
        existing.target_language = Some(output.target_language.clone());
        existing.is_final = output.is_final;
    } else {
        entries.push(output_entry_from_event(output));
    }
}

/// Apply a backend [`AppEvent`] to a `session`.
///
/// Routing:
/// - `AppEvent::Transcript(_)`: merge into the matching
///   output-only entry when one exists (e.g. if an
///   output event for the same chunk_index arrived first),
///   otherwise append a fresh transcript entry. Uses
///   [`merge_transcript_into_entries`] so an out-of-order
///   pipeline (translation-first then transcript) does not
///   produce a duplicate entry.
/// - `AppEvent::Output(_)`: merge into the matching
///   transcript entry when one exists, otherwise append an
///   output-only entry (via [`merge_output_into_entries`]).
/// - `AppEvent::StatusChanged(_)`: ignored — status lifecycle
///   is tracked separately on the [`crate::app::state::AppStatus`]
///   field, not per-chunk on the session export.
/// - `AppEvent::Error(_)`: ignored — errors are surfaced to
///   the user via the Tauri `error` event payload, and
///   deliberately not embedded in the export (so a corrupt
///   `runtime.json` from a mid-session failure still produces
///   a valid export surface).
///
/// Pure — no I/O, no side effects beyond mutating `session`.
pub fn apply_app_event_to_session(session: &mut TranscriptSession, event: &AppEvent) {
    match event {
        AppEvent::Transcript(t) => {
            merge_transcript_into_entries(&mut session.entries, t);
        }
        AppEvent::Output(t) => {
            merge_output_into_entries(&mut session.entries, t);
        }
        AppEvent::StatusChanged(_) | AppEvent::Error(_) | AppEvent::Speech(_) => {
            // Status / error events do not contribute to the
            // exportable transcript surface. See module docs.
        }
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::events::{AppErrorEvent, AppStatus};
    use crate::processor::processor::OutputKind;

    fn sample_transcript_event(chunk: usize, text: &str) -> AppTranscriptEvent {
        AppTranscriptEvent {
            chunk_index: chunk,
            text: text.to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
            duration_ms: 1_200,
        }
    }

    fn sample_output_event(chunk: usize, source_text: &str, translated: &str) -> AppOutputEvent {
        AppOutputEvent {
            chunk_index: chunk,
            kind: OutputKind::Translate,
            source_text: source_text.to_string(),
            text: translated.to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
            blocked: None,
        }
    }

    // ---- create_session ----------------------------------------------

    #[test]
    fn create_session_initializes_metadata_and_empty_entries() {
        let s = create_session(
            "abc123".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "2026-06-27T12:00:00Z".to_string(),
        );
        assert_eq!(s.session_id, "abc123");
        assert_eq!(s.provider, "mock-local");
        assert_eq!(s.source_language, "en");
        assert_eq!(s.target_language, "es");
        assert_eq!(s.started_at, "2026-06-27T12:00:00Z");
        assert!(s.ended_at.is_none());
        assert!(s.entries.is_empty());
    }

    // ---- transcript_entry_from_event ---------------------------------

    #[test]
    fn transcript_entry_from_event_maps_chunk_index() {
        let event = sample_transcript_event(7, "hello");
        let entry = transcript_entry_from_event(&event);
        assert_eq!(entry.chunk_index, 7);
    }

    #[test]
    fn transcript_entry_from_event_maps_text() {
        let event = sample_transcript_event(1, "hello world");
        let entry = transcript_entry_from_event(&event);
        assert_eq!(entry.transcript_text.as_deref(), Some("hello world"));
    }

    #[test]
    fn transcript_entry_from_event_maps_provider() {
        let event = AppTranscriptEvent {
            chunk_index: 1,
            text: "x".to_string(),
            provider: "local-whisper".to_string(),
            is_final: true,
            duration_ms: 1_200,
        };
        let entry = transcript_entry_from_event(&event);
        assert_eq!(entry.provider.as_deref(), Some("local-whisper"));
    }

    #[test]
    fn transcript_entry_from_event_maps_final_flag() {
        let event = AppTranscriptEvent {
            chunk_index: 1,
            text: "x".to_string(),
            provider: "p".to_string(),
            is_final: false,
            duration_ms: 900,
        };
        let entry = transcript_entry_from_event(&event);
        assert!(!entry.is_final);
    }

    #[test]
    fn transcript_entry_from_event_leaves_translation_fields_empty() {
        let event = sample_transcript_event(1, "hello");
        let entry = transcript_entry_from_event(&event);
        assert!(entry.translated_text.is_none());
        assert!(entry.source_language.is_none());
        assert!(entry.target_language.is_none());
    }

    // ---- output_entry_from_event --------------------------------

    #[test]
    fn output_entry_from_event_maps_chunk_index() {
        let event = sample_output_event(4, "hi", "hola");
        let entry = output_entry_from_event(&event);
        assert_eq!(entry.chunk_index, 4);
    }

    #[test]
    fn output_entry_from_event_maps_source_language() {
        let event = sample_output_event(1, "hi", "hola");
        let entry = output_entry_from_event(&event);
        assert_eq!(entry.source_language.as_deref(), Some("en"));
    }

    #[test]
    fn output_entry_from_event_maps_target_language() {
        let event = sample_output_event(1, "hi", "hola");
        let entry = output_entry_from_event(&event);
        assert_eq!(entry.target_language.as_deref(), Some("es"));
    }

    #[test]
    fn output_entry_from_event_maps_translated_text() {
        let event = sample_output_event(1, "hi", "[mock es] hola");
        let entry = output_entry_from_event(&event);
        assert_eq!(entry.translated_text.as_deref(), Some("[mock es] hola"));
    }

    #[test]
    fn output_entry_from_event_maps_final_flag() {
        let event = AppOutputEvent {
            chunk_index: 1,
            kind: OutputKind::Translate,
            source_text: "x".to_string(),
            text: "y".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: false,
            blocked: None,
        };
        let entry = output_entry_from_event(&event);
        assert!(!entry.is_final);
    }

    #[test]
    fn output_entry_from_event_leaves_transcript_text_empty() {
        let event = sample_output_event(1, "hi", "hola");
        let entry = output_entry_from_event(&event);
        assert!(entry.transcript_text.is_none());
        assert!(entry.provider.is_none());
    }

    // ---- merge_output_into_entries -----------------------------

    #[test]
    fn merge_output_into_entries_updates_existing_matching_entry() {
        // Set up an entry from a transcript-only event.
        let transcript = sample_transcript_event(1, "hello");
        let mut entries = vec![transcript_entry_from_event(&transcript)];

        // Now fold in a translation for the same chunk — the
        // existing entry is mutated in place.
        let output = AppOutputEvent {
            chunk_index: 1,
            kind: OutputKind::Translate,
            source_text: "hello".to_string(),
            text: "[mock es] hello".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
            blocked: None,
        };
        merge_output_into_entries(&mut entries, &output);

        assert_eq!(entries.len(), 1, "must not append a duplicate entry");
        let entry = &entries[0];
        assert_eq!(entry.chunk_index, 1);
        assert_eq!(
            entry.transcript_text.as_deref(),
            Some("hello"),
            "previous transcript text must remain"
        );
        assert_eq!(entry.translated_text.as_deref(), Some("[mock es] hello"));
        assert_eq!(entry.source_language.as_deref(), Some("en"));
        assert_eq!(entry.target_language.as_deref(), Some("es"));
        assert!(entry.is_final);
        // Provider comes from the transcript event, not the
        // translation event — must be preserved.
        assert_eq!(entry.provider.as_deref(), Some("mock-local"));
    }

    #[test]
    fn merge_output_into_entries_preserves_existing_provider_on_update() {
        // Lock-in: the merge path must not clobber the
        // `provider` field set by the prior transcript event
        // (the translation event does not carry a provider).
        let mut entries = vec![transcript_entry_from_event(&sample_transcript_event(
            1, "hi",
        ))];
        entries[0].provider = Some("local-whisper".to_string());

        merge_output_into_entries(&mut entries, &sample_output_event(1, "hi", "hola"));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider.as_deref(), Some("local-whisper"));
    }

    #[test]
    fn merge_output_into_entries_adds_new_entry_if_missing() {
        // Set up an entry for chunk 1 (transcript-only).
        let mut entries = vec![transcript_entry_from_event(&sample_transcript_event(
            1, "first",
        ))];

        // Fold in a translation for chunk 5 with no matching
        // entry — must append a translation-only entry.
        let output = sample_output_event(5, "fifth", "quinto");
        merge_output_into_entries(&mut entries, &output);

        assert_eq!(entries.len(), 2);
        // Original entry untouched.
        assert_eq!(entries[0].chunk_index, 1);
        assert_eq!(entries[0].transcript_text.as_deref(), Some("first"));
        assert!(entries[0].translated_text.is_none());

        // New translation-only entry.
        assert_eq!(entries[1].chunk_index, 5);
        assert!(entries[1].transcript_text.is_none());
        assert_eq!(entries[1].translated_text.as_deref(), Some("quinto"));
        assert_eq!(entries[1].source_language.as_deref(), Some("en"));
        assert_eq!(entries[1].target_language.as_deref(), Some("es"));
    }

    #[test]
    fn merge_output_into_entries_on_empty_appends_translation_only() {
        let mut entries: Vec<TranscriptSessionEntry> = Vec::new();
        merge_output_into_entries(&mut entries, &sample_output_event(1, "hi", "hola"));
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.chunk_index, 1);
        assert!(entry.transcript_text.is_none());
        assert!(entry.provider.is_none());
        assert_eq!(entry.translated_text.as_deref(), Some("hola"));
    }

    // ---- apply_app_event_to_session ----------------------------------

    #[test]
    fn apply_app_event_to_session_appends_transcript_event() {
        let mut s = create_session(
            "s1".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "2026-06-27T12:00:00Z".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "hello")),
        );
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.entries[0].transcript_text.as_deref(), Some("hello"));
    }

    #[test]
    fn apply_app_event_to_session_merges_translation_event_into_prior_transcript() {
        // Mirror the live-pipeline order: transcript first,
        // translation second — both for chunk 1.
        let mut s = create_session(
            "s1".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "hello")),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(1, "hello", "[mock es] hello")),
        );
        assert_eq!(s.entries.len(), 1, "must fold into the existing entry");
        let entry = &s.entries[0];
        assert_eq!(entry.transcript_text.as_deref(), Some("hello"));
        assert_eq!(entry.translated_text.as_deref(), Some("[mock es] hello"));
        assert_eq!(entry.provider.as_deref(), Some("mock-local"));
        assert_eq!(entry.source_language.as_deref(), Some("en"));
        assert_eq!(entry.target_language.as_deref(), Some("es"));
    }

    #[test]
    fn apply_app_event_to_session_appends_translation_only_entry_when_no_prior_transcript() {
        let mut s = create_session(
            "s1".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(2, "hi", "[mock es] hola")),
        );
        assert_eq!(s.entries.len(), 1);
        let entry = &s.entries[0];
        assert_eq!(entry.chunk_index, 2);
        assert!(entry.transcript_text.is_none());
        assert_eq!(entry.translated_text.as_deref(), Some("[mock es] hola"));
    }

    #[test]
    fn apply_app_event_to_session_ignores_status_event() {
        let mut s = create_session(
            "s".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(&mut s, &AppEvent::StatusChanged(AppStatus::Listening));
        assert!(
            s.entries.is_empty(),
            "status events must not contribute to export entries"
        );
    }

    #[test]
    fn apply_app_event_to_session_ignores_error_event() {
        let mut s = create_session(
            "s".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Error(AppErrorEvent {
                message: "boom".to_string(),
            }),
        );
        assert!(
            s.entries.is_empty(),
            "error events must not contribute to export entries"
        );
        // Session metadata untouched.
        assert_eq!(s.entries.len(), 0);
        assert!(
            s.ended_at.is_none(),
            "errors must not auto-close the session"
        );
    }

    #[test]
    fn apply_app_event_to_session_out_of_order_transcript_after_translation_merges() {
        // Out-of-order pipe: translation arrives first for
        // chunk 1, transcript arrives later. The transcript
        // must fold into the existing translation-only entry
        // rather than producing a second entry for the same
        // chunk_index.
        let mut s = create_session(
            "s".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(1, "hello", "[mock es] hello")),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "hello")),
        );
        assert_eq!(
            s.entries.len(),
            1,
            "must not produce a duplicate entry for chunk 1"
        );
        let entry = &s.entries[0];
        assert_eq!(entry.chunk_index, 1);
        assert_eq!(entry.transcript_text.as_deref(), Some("hello"));
        assert_eq!(
            entry.translated_text.as_deref(),
            Some("[mock es] hello"),
            "pre-existing translation must be preserved"
        );
        assert_eq!(entry.provider.as_deref(), Some("mock-local"));
        assert_eq!(entry.source_language.as_deref(), Some("en"));
        assert_eq!(entry.target_language.as_deref(), Some("es"));
    }

    #[test]
    fn apply_app_event_to_session_repeat_transcript_does_not_overwrite_translation() {
        // Lock-in: a duplicate transcript event for a chunk
        // that already has both transcript and translation must
        // be ignored — pushing a fresh entry would create a
        // duplicate `chunk_index` row, overwriting would silently
        // drop the canonical translation source. See
        // [`merge_transcript_into_entries`]'s case-3 branch.
        let mut s = create_session(
            "s".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "first-transcript")),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(
                1,
                "first-transcript",
                "[mock es] first",
            )),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "duplicate-transcript")),
        );
        assert_eq!(
            s.entries.len(),
            1,
            "duplicate transcript event must not produce a second entry"
        );
        let entry = &s.entries[0];
        assert_eq!(
            entry.transcript_text.as_deref(),
            Some("first-transcript"),
            "first transcript text must survive a duplicate transcript event"
        );
        assert_eq!(
            entry.translated_text.as_deref(),
            Some("[mock es] first"),
            "translation must survive a duplicate transcript event"
        );
    }

    #[test]
    fn apply_app_event_to_session_full_sequence_keeps_entries_compact() {
        let mut s = create_session(
            "seq".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "t0".to_string(),
        );
        // Chunk 1: transcript + translation.
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(1, "one")),
        );
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(1, "one", "[mock es] uno")),
        );
        // Chunk 2: transcript-only (no translation yet).
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Transcript(sample_transcript_event(2, "two")),
        );
        // Status changes are ignored, error events are ignored.
        apply_app_event_to_session(&mut s, &AppEvent::StatusChanged(AppStatus::Listening));
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Error(AppErrorEvent {
                message: "ignored".to_string(),
            }),
        );
        // Chunk 2: translation arrives later — merges into the
        // existing entry rather than appending.
        apply_app_event_to_session(
            &mut s,
            &AppEvent::Output(sample_output_event(2, "two", "[mock es] dos")),
        );

        assert_eq!(s.entries.len(), 2, "each chunk stays a single entry");

        // Chunk 1 fully populated.
        let e1 = &s.entries[0];
        assert_eq!(e1.chunk_index, 1);
        assert_eq!(e1.transcript_text.as_deref(), Some("one"));
        assert_eq!(e1.translated_text.as_deref(), Some("[mock es] uno"));

        // Chunk 2 fully populated after the late translation.
        let e2 = &s.entries[1];
        assert_eq!(e2.chunk_index, 2);
        assert_eq!(e2.transcript_text.as_deref(), Some("two"));
        assert_eq!(e2.translated_text.as_deref(), Some("[mock es] dos"));
    }
}
