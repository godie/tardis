//! Pure render helpers for [`crate::app::session::TranscriptSession`].
//!
//! Two output formats:
//!
//! 1. **Pretty JSON** — round-trippable through
//!    [`crate::app::session`] and stable across versions because
//!    the struct is `#[derive(serde::Serialize, serde::Deserialize)]`.
//!    Hand-written by [`session_to_json`] to guarantee it uses
//!    `serde_json::to_string_pretty` so the on-disk file is
//!    diff-friendly and operator-readable.
//!
//! 2. **Plain text** — a stable, human-readable summary that
//!    includes the session metadata block (session_id,
//!    provider, source / target language, started_at,
//!    ended_at) followed by one block per
//!    [`crate::app::session::TranscriptSessionEntry`] carrying
//!    transcript text (when present) and translation text
//!    (when present). See [`session_to_plain_text`].
//!
//! The module is **deliberately pure**: no FS, no CPAL, no HTTP.
//! File-bound writers live in
//! [`crate::app::session_store`]. Tests live in
//! `#[cfg(test)] mod tests` at the bottom per the
//! AGENTS.md testing contract.

use crate::app::session::TranscriptSession;

/// Render a [`TranscriptSession`] as pretty JSON.
///
/// Uses [`serde_json::to_string_pretty`] so the result is
/// diff-able and operator-readable. Field order is guaranteed
/// by `serde_json`'s struct declaration order — adding a new
/// field to [`crate::app::session::TranscriptSession`]
/// automatically appends it to the export without breaking
/// existing consumers.
///
/// Errors are surfaced as `anyhow::Error` for parity with the
/// file-bound helpers in [`crate::app::session_store`].
///
/// Pure — no FS access, no logging, no allocation beyond the
/// returned `String`.
pub fn session_to_json(session: &TranscriptSession) -> anyhow::Result<String> {
    serde_json::to_string_pretty(session)
        .map_err(|e| anyhow::anyhow!("failed to serialize TranscriptSession: {e}"))
}

/// Render a [`TranscriptSession`] as a stable, human-readable
/// plain-text block.
///
/// Layout (stable, see unit tests):
///
/// ```text
/// session_id:           <session_id>
/// provider:             <provider>
/// source_language:      <source>
/// target_language:      <target>
/// started_at:           <started_at>
/// ended_at:             <ended or "—">
///
/// entries: <count>
///   [chunk <i>]
///     transcript:       <text or omitted>
///     translation (<src> -> <tgt>): <text or omitted>
///     is_final:         true | false
/// ```
///
/// The format is intentionally **compact** and **scannable**:
/// one line per scalar field, one block per entry. Anything
/// missing from an entry (e.g. `transcript_text` is `None`
/// because only a translation arrived) is omitted rather
/// than printed as `—` so empty rows do not visually pollute
/// exports.
///
/// Pure — no FS, no allocation beyond the returned `String`.
pub fn session_to_plain_text(session: &TranscriptSession) -> String {
    let mut out = String::new();

    out.push_str(&format!("session_id:           {}\n", session.session_id));
    out.push_str(&format!("provider:             {}\n", session.provider));
    out.push_str(&format!(
        "source_language:      {}\n",
        session.source_language
    ));
    out.push_str(&format!(
        "target_language:      {}\n",
        session.target_language
    ));
    out.push_str(&format!("started_at:           {}\n", session.started_at));
    match session.ended_at.as_deref() {
        Some(ended) => out.push_str(&format!("ended_at:             {ended}\n")),
        None => out.push_str("ended_at:             —\n"),
    }
    out.push('\n');

    out.push_str(&format!("entries: {}\n", session.entries.len()));
    for entry in &session.entries {
        out.push_str(&format!("  [chunk {}]\n", entry.chunk_index));
        if let Some(text) = entry.transcript_text.as_deref() {
            out.push_str(&format!("    transcript:       {text}\n"));
        }
        if let Some(translated) = entry.translated_text.as_deref() {
            let src = entry.source_language.as_deref().unwrap_or("?");
            let tgt = entry.target_language.as_deref().unwrap_or("?");
            out.push_str(&format!("    translation ({src} -> {tgt}): {translated}\n"));
        }
        out.push_str(&format!("    is_final:         {}\n", entry.is_final));
    }

    out
}

/// Build a default filename for a session export.
///
/// Behavior:
/// - Prefix: `"session_"`.
/// - Body: `session_id` after simple whitespace
///   sanitisation (each ASCII space `' '` is replaced with
///   `_` so the result survives a shell round-trip).
/// - Suffix: `'.' + extension` if `extension` does not
///   already start with a leading dot; otherwise
///   `extension` verbatim. Empty `extension` is treated as a
///   missing extension and produces no suffix.
///
/// Examples:
///
/// | input                                  | output                              |
/// |----------------------------------------|-------------------------------------|
/// | `("abc123", "json")`                   | `"session_abc123.json"`             |
/// | `("abc123", ".json")`                  | `"session_abc123.json"`             |
/// | `("session one", "txt")`               | `"session_session_one.txt"`         |
/// | `("abc", "")`                          | `"session_abc"`                     |
///
/// Pure — no FS, no allocation beyond the returned `String`.
pub fn default_session_filename(session_id: &str, extension: &str) -> String {
    let sanitised: String = session_id
        .chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .collect();
    let suffix = if extension.is_empty() {
        String::new()
    } else if extension.starts_with('.') {
        extension.to_string()
    } else {
        format!(".{extension}")
    };
    format!("session_{sanitised}{suffix}")
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::{TranscriptSessionEntry, create_session};

    fn sample_session() -> TranscriptSession {
        let mut s = create_session(
            "abc123".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "2026-06-27T12:00:00Z".to_string(),
        );
        s.ended_at = Some("2026-06-27T12:01:00Z".to_string());
        s.entries.push(TranscriptSessionEntry {
            chunk_index: 1,
            transcript_text: Some("hello".to_string()),
            translated_text: Some("[mock es] hello".to_string()),
            source_language: Some("en".to_string()),
            target_language: Some("es".to_string()),
            provider: Some("mock-local".to_string()),
            is_final: true,
        });
        s
    }

    // ---- session_to_json ---------------------------------------------

    #[test]
    fn session_to_json_includes_session_id() {
        let s = sample_session();
        let json = session_to_json(&s).expect("serialize must succeed");
        assert!(
            json.contains("\"session_id\""),
            "JSON must include session_id key, got: {json}"
        );
        assert!(
            json.contains("\"abc123\""),
            "JSON must include the session_id value, got: {json}"
        );
    }

    #[test]
    fn session_to_json_includes_provider() {
        // Provider key + value both land in JSON because the
        // struct is plain `serde_json` round-trip.
        let s = sample_session();
        let json = session_to_json(&s).expect("serialize must succeed");
        assert!(json.contains("\"provider\""));
        assert!(json.contains("\"mock-local\""));
    }

    #[test]
    fn session_to_json_includes_entries() {
        let s = sample_session();
        let json = session_to_json(&s).expect("serialize must succeed");
        assert!(
            json.contains("\"entries\""),
            "JSON must include entries key, got: {json}"
        );
        assert!(
            json.contains("\"chunk_index\""),
            "JSON must include chunk_index in entries, got: {json}"
        );
        assert!(
            json.contains("\"transcript_text\""),
            "JSON must include transcript_text field inside entries"
        );
        assert!(
            json.contains("\"hello\""),
            "JSON must echo the transcript text content"
        );
        assert!(
            json.contains("\"[mock es] hello\""),
            "JSON must echo the translated text content"
        );
    }

    #[test]
    fn session_to_json_is_pretty_printed() {
        // Pretty JSON produces newlines between fields; raw
        // `serde_json::to_string` does not. We anchor on a
        // multi-line shape so the format choice shows up in
        // the assertion.
        let s = sample_session();
        let json = session_to_json(&s).expect("serialize must succeed");
        assert!(
            json.contains("{\n"),
            "pretty JSON opens the object on its own line"
        );
        assert!(
            json.contains('\n'),
            "pretty JSON contains at least one newline"
        );
    }

    // ---- session_to_plain_text ---------------------------------------

    #[test]
    fn session_to_plain_text_includes_session_id() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("abc123"),
            "plain text must include session_id, got:\n{text}"
        );
        assert!(
            text.contains("session_id:"),
            "plain text must label session_id, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_provider() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("mock-local"),
            "plain text must include provider, got:\n{text}"
        );
        assert!(
            text.contains("provider:"),
            "plain text must label provider, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_languages() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(text.contains("source_language:      en"));
        assert!(text.contains("target_language:      es"));
    }

    #[test]
    fn session_to_plain_text_includes_transcript_text() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("transcript:       hello"),
            "plain text must include transcript line, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_translated_text() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("[mock es] hello"),
            "plain text must include translated text, got:\n{text}"
        );
        assert!(
            text.contains("translation (en -> es)"),
            "plain text must include the language pair, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_chunk_index() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("[chunk 1]"),
            "plain text must include chunk index, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_ended_at_when_present() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("2026-06-27T12:01:00Z"),
            "ended_at must be included when Some, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_emits_dash_when_ended_at_missing() {
        // Lock-in: missing `ended_at` renders as `—` so the
        // metadata block stays aligned for both open and
        // closed sessions.
        let mut s = sample_session();
        s.ended_at = None;
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("ended_at:             —"),
            "missing ended_at must render as em-dash, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_includes_entry_count() {
        let s = sample_session();
        let text = session_to_plain_text(&s);
        assert!(
            text.contains("entries: 1"),
            "plain text must report entry count, got:\n{text}"
        );
    }

    #[test]
    fn session_to_plain_text_omits_lines_for_missing_optional_fields() {
        // Translation-only entry (transcript_text = None):
        // the transcript line must NOT be emitted at all,
        // otherwise empty rows visually pollute the export.
        let mut s = sample_session();
        s.entries.clear();
        s.entries.push(TranscriptSessionEntry {
            chunk_index: 2,
            transcript_text: None,
            translated_text: Some("only translation".to_string()),
            source_language: Some("en".to_string()),
            target_language: Some("es".to_string()),
            provider: None,
            is_final: true,
        });
        let text = session_to_plain_text(&s);
        assert!(text.contains("[chunk 2]"));
        assert!(text.contains("only translation"));
        assert!(
            !text.contains("transcript:       only"),
            "translation-only entry must not emit a transcript line, got:\n{text}"
        );
    }

    // ---- default_session_filename ------------------------------------

    #[test]
    fn default_filename_works_with_extension_without_dot() {
        // Spec-mandated: `"abc123", "json" -> "session_abc123.json"`.
        assert_eq!(
            default_session_filename("abc123", "json"),
            "session_abc123.json"
        );
    }

    #[test]
    fn default_filename_works_with_extension_with_dot() {
        // Spec-mandated: leading dot is allowed and must NOT
        // double up ("session_abc123.json.json" would be a
        // bug).
        assert_eq!(
            default_session_filename("abc123", ".json"),
            "session_abc123.json"
        );
    }

    #[test]
    fn default_filename_replaces_spaces_in_session_id() {
        // Spec-mandated: spaces become underscores so the
        // result survives a shell round-trip.
        assert_eq!(
            default_session_filename("session one", "txt"),
            "session_session_one.txt"
        );
    }

    #[test]
    fn default_filename_replaces_multiple_spaces() {
        // Every single space is replaced — only the literal
        // ASCII space is in scope; tabs / other whitespace are
        // preserved (out of scope per spec).
        assert_eq!(
            default_session_filename("a b c", "txt"),
            "session_a_b_c.txt"
        );
    }

    #[test]
    fn default_filename_with_empty_extension_drops_suffix() {
        // Empty extension means "no extension at all" — a
        // future shell-history display might want exactly
        // this.
        assert_eq!(default_session_filename("abc", ""), "session_abc");
    }

    #[test]
    fn default_filename_preserves_other_whitespace() {
        // Only ASCII space is in-scope for sanitisation. A tab
        // character is left untouched — out of scope.
        // This is a lock-in test against a future expansion of
        // the sanitisation surface.
        assert_eq!(
            default_session_filename("a\tb", "json"),
            "session_a\tb.json"
        );
    }
}
