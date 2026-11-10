//! File-bound writers for [`crate::app::session::TranscriptSession`].
//!
//! These helpers are the **production write path** for the
//! session export: they take the pure render helpers
//! ([`crate::app::session_export::session_to_json`] and
//! [`crate::app::session_export::session_to_plain_text`]) and
//! write them to a file on disk, creating parent directories
//! as needed.
//!
//! Both functions are **deliberately not unit-tested against
//! the filesystem** — the project's testing contract
//! ([`crate::session_export`] is pure; file I/O is
//! manual-tested via the existing `cargo run -- ...` modes and
//! the Tauri shell). The parent-directory-create behaviour
//! matches the existing
//! [`crate::app::settings_store::save_to_path`] so a
//! reliability-flavoured smoke test against one implies the
//! other.
//!
//! Test surface is `#[allow(dead_code)]`-free: each helper is
//! exercised by a `cargo run -- session-export-demo` smoke
//! command (see README "CLI smoke command" note) and from the
//! Tauri shell's `export_current_session_json` /
//! `export_current_session_text` commands.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result as AnyhowResult};

use crate::app::session::TranscriptSession;
use crate::app::session_export::{session_to_json, session_to_plain_text};

/// Write `session` to `path` as pretty JSON.
///
/// Pipeline:
/// 1. Render via [`session_to_json`] (no FS).
/// 2. Create parent directory if missing (matches
///    [`crate::app::settings_store::save_to_path`] behaviour).
/// 3. Write the bytes (no atomic rename today — the session
///    export is a fresh file, not an in-place overwrite, so a
///    torn-write risk surfaces only for a "export to an
///    existing file" path, which the UI flow does not
///    currently offer).
///
/// Errors are wrapped with context so the operator can tell
/// whether the failure was the formatter (rare), the FS
/// permissions, or a missing parent dir.
pub fn save_session_json_to_file(session: &TranscriptSession, path: &Path) -> AnyhowResult<()> {
    let json = session_to_json(session)
        .context("failed to render TranscriptSession to JSON for export")?;
    write_with_parents(path, json.as_bytes())
        .with_context(|| format!("failed to write JSON export to {}", path.display()))
}

/// Write `session` to `path` as plain text.
///
/// Pipeline mirrors [`save_session_json_to_file`]. Errors are
/// wrapped with context.
pub fn save_session_text_to_file(session: &TranscriptSession, path: &Path) -> AnyhowResult<()> {
    let text = session_to_plain_text(session);
    write_with_parents(path, text.as_bytes())
        .with_context(|| format!("failed to write plain-text export to {}", path.display()))
}

/// Internal: create `path.parent()` if missing, then write `bytes`.
///
/// Split out so both `save_session_*` helpers share the same
/// create-parent-then-write pipeline. Returns `io::Result` so
/// the public wrappers can attach anyhow context.
fn write_with_parents(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // An empty parent means the path is a bare filename: no
    // create-parent-then-write needed.
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    // `sync_all` keeps the export readable even if the OS
    // crashes moments later. Mirrors the atomic-write posture
    // of `app::settings_store::save_to_path`.
    f.sync_all()?;
    Ok(())
}
