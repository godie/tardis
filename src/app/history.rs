//! Append-only session history, one JSON object per line (JSONL).
//!
//! The shell writes one line per stage output, the transcript it heard and
//! the rendered output it produced, so a session can be re-read after the
//! window is closed: `tail -f`, `grep`, or any JSONL reader. Both kinds live
//! in one file, in the order they happened, because correlating two files by
//! timestamp is work the reader should not have to do.
//!
//! Appending a line never rewrites what is already there, so a crash
//! mid-session loses at most the line in flight.
//!
//! Formatting is pure and table-tested here; the file I/O lives in
//! `src-tauri`, which owns the runtime paths (DEVELOPMENT_RULES rule 1).

use serde::Serialize;

/// One line of the history file.
///
/// Internally tagged with `kind`, so a reader can filter with `grep
/// '"kind":"output"'` without knowing the field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryRecord<'a> {
    /// What the provider heard.
    Transcript {
        /// Unix epoch milliseconds (UTC): sortable, timezone-free, and the
        /// frontend renders the same instant as local wall-clock time.
        at_ms: u64,
        /// Transcript text exactly as the provider returned it. No
        /// translation: this records what was heard, not what was rendered.
        text: &'a str,
        /// Length of the captured audio, in milliseconds.
        duration_ms: u64,
        /// Provider that produced the text (`"mock-local"`, `"local-whisper"`).
        provider: &'a str,
    },
    /// What the process stage rendered for that transcript.
    Output {
        at_ms: u64,
        /// Rendered text, as shown in the Translation panel and spoken.
        text: &'a str,
        source_language: &'a str,
        target_language: &'a str,
        /// Which mode produced the line (`"translate"` or `"respond"`).
        output_kind: &'a str,
    },
}

/// Render one history line, newline included.
///
/// `Err` is only possible if serialization fails, which for a struct of
/// scalars and `&str` means a broken `serde` implementation rather than bad
/// input; the caller logs it and keeps the session running.
pub fn format_record(record: &HistoryRecord<'_>) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(record)?;
    // `serde_json` escapes any newline inside `text`, so one record is
    // always exactly one line even if a provider returns a multi-line
    // transcript. That is what keeps the file greppable and appendable.
    line.push('\n');
    Ok(line)
}

/// True when a stage output is worth recording.
///
/// A provider can return an empty string (silence, wrong language) and the
/// mock processor can return nothing for an empty transcript. Those carry no
/// information and would pad the history with empty rows.
pub fn is_recordable(text: &str) -> bool {
    !text.trim().is_empty()
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    const AT_MS: u64 = 1_767_000_000_000;

    fn transcript<'a>(text: &'a str, duration_ms: u64, provider: &'a str) -> HistoryRecord<'a> {
        HistoryRecord::Transcript {
            at_ms: AT_MS,
            text,
            duration_ms,
            provider,
        }
    }

    fn output<'a>(text: &'a str) -> HistoryRecord<'a> {
        HistoryRecord::Output {
            at_ms: AT_MS,
            text,
            source_language: "en",
            target_language: "es",
            output_kind: "translate",
        }
    }

    fn parse(line: &str) -> serde_json::Value {
        serde_json::from_str(line.trim_end()).expect("valid JSON")
    }

    #[test]
    fn format_record_emits_one_json_line() {
        let line =
            format_record(&transcript("hello world", 4_320, "mock-local")).expect("serializes");
        assert_eq!(
            line.matches('\n').count(),
            1,
            "exactly one newline: {line:?}"
        );
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn transcript_line_carries_every_field() {
        let parsed = parse(&format_record(&transcript("hello", 1_200, "local-whisper")).unwrap());
        assert_eq!(parsed["kind"], "transcript");
        assert_eq!(parsed["at_ms"], AT_MS);
        assert_eq!(parsed["text"], "hello");
        assert_eq!(parsed["duration_ms"], 1_200);
        assert_eq!(parsed["provider"], "local-whisper");
    }

    #[test]
    fn output_line_carries_every_field() {
        let parsed = parse(&format_record(&output("hola")).unwrap());
        assert_eq!(parsed["kind"], "output");
        assert_eq!(parsed["at_ms"], AT_MS);
        assert_eq!(parsed["text"], "hola");
        assert_eq!(parsed["source_language"], "en");
        assert_eq!(parsed["target_language"], "es");
        assert_eq!(parsed["output_kind"], "translate");
    }

    #[test]
    fn the_two_kinds_are_distinguishable_by_line_alone() {
        // One file holds both; `grep '"kind":"output"'` has to be enough to
        // pull the rendered lines out of a week of sessions.
        let t = format_record(&transcript("hi", 900, "p")).unwrap();
        let o = format_record(&output("hola")).unwrap();
        assert!(t.starts_with("{\"kind\":\"transcript\""));
        assert!(o.starts_with("{\"kind\":\"output\""));
        assert!(!t.contains("\"source_language\""));
        assert!(!o.contains("\"provider\""));
    }

    #[test]
    fn a_newline_in_the_text_stays_on_one_record() {
        // Providers do return multi-line text; a raw newline would split the
        // record in two and corrupt every line after it for a JSONL reader.
        for line in [
            format_record(&transcript("first\nsecond", 900, "p")).unwrap(),
            format_record(&output("primero\nsegundo")).unwrap(),
        ] {
            assert_eq!(
                line.matches('\n').count(),
                1,
                "text newline must be escaped"
            );
        }
        let parsed = parse(&format_record(&transcript("first\nsecond", 900, "p")).unwrap());
        assert_eq!(parsed["text"], "first\nsecond");
    }

    #[test]
    fn quotes_and_control_chars_round_trip() {
        let text = "he said \"hi\"\tand left";
        let parsed = parse(&format_record(&transcript(text, 500, "p")).unwrap());
        assert_eq!(parsed["text"], text);
        let parsed = parse(&format_record(&output(text)).unwrap());
        assert_eq!(parsed["text"], text);
    }

    #[test]
    fn empty_text_is_not_recorded() {
        assert!(!is_recordable(""));
        assert!(!is_recordable("   \n\t"));
        assert!(is_recordable("hola"));
    }

    #[test]
    fn zero_duration_is_still_recorded() {
        // Synthetic flows report 0 ms; the text is still real.
        let line = format_record(&transcript("synthetic", 0, "mock-local")).unwrap();
        assert!(line.contains("\"duration_ms\":0"));
    }
}
