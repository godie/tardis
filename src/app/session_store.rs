//! File-bound writers for [`crate::app::session::TranscriptSession`].
//!
//! These helpers are the **production write path** for the
//! session export: they take the pure render helpers
//! ([`crate::app::session_export::session_to_json`] and
//! [`crate::app::session_export::session_to_plain_text`]) and
//! write them to a file on disk, creating parent directories
//! as needed.
//!
//! [`export_session`] is the one entry point a frontend calls: it
//! names both files from [`crate::app::session_export::default_session_filename`],
//! writes the JSON and the text rendering, and returns the two paths.
//! [`session_export_dir`] composes the per-user directory under an
//! OS data dir, so the CLI smoke command
//! (`cargo run -- session-export-demo`) never writes into the repo
//! working directory.
//!
//! A session export contains private speech, so the leaf directory is
//! `0700` and the files are `0600` on Unix, independent of the process
//! umask. Missing parents keep their ordinary mode. This follows the
//! owner-only posture tracked in issue #77.
//!
//! File I/O is tested against a temp directory, like
//! [`crate::app::settings_store`]; the render helpers themselves are
//! covered in [`crate::app::session_export`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result as AnyhowResult};

use crate::app::session::TranscriptSession;
use crate::app::session_export::{
    default_session_filename, session_to_json, session_to_plain_text,
};

/// Name of the app's directory inside the OS per-user data directory.
const APP_DATA_DIR_NAME: &str = "tardis";

/// Name of the session-export directory inside the app data directory.
const SESSIONS_DIR_NAME: &str = "sessions";

/// Compose the per-user directory that holds session exports.
///
/// `base_data_dir` is the OS data directory
/// (`$XDG_DATA_HOME`, `~/Library/Application Support`, `%APPDATA%`);
/// the CLI resolves it, the same way the Tauri shell lets
/// `tauri::Manager::path()` resolve its config directory. Pure path
/// composition: no I/O.
pub fn session_export_dir(base_data_dir: &Path) -> PathBuf {
    base_data_dir
        .join(APP_DATA_DIR_NAME)
        .join(SESSIONS_DIR_NAME)
}

/// The two files [`export_session`] wrote.
#[derive(Debug)]
pub struct SessionExportPaths {
    pub json: PathBuf,
    pub text: PathBuf,
}

/// Write `session` under `dir` in both formats and return the paths.
///
/// File names come from
/// [`crate::app::session_export::default_session_filename`], which makes
/// a second export of the same `session_id` replace the first rather than
/// pile up files. Both writers create `dir` (owner-only on Unix) before
/// writing.
pub fn export_session(session: &TranscriptSession, dir: &Path) -> AnyhowResult<SessionExportPaths> {
    let paths = SessionExportPaths {
        json: dir.join(default_session_filename(&session.session_id, "json")),
        text: dir.join(default_session_filename(&session.session_id, "txt")),
    };
    save_session_json_to_file(session, &paths.json)?;
    save_session_text_to_file(session, &paths.text)?;
    Ok(paths)
}

/// Write `session` to `path` as pretty JSON.
///
/// Pipeline:
/// 1. Render via [`session_to_json`] (no FS).
/// 2. Create the parent directory if missing (owner-only on Unix).
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
        create_private_dir(parent)?;
    }
    let mut f = create_private_file(path)?;
    f.write_all(bytes)?;
    // `sync_all` keeps the export readable even if the OS
    // crashes moments later. Mirrors the atomic-write posture
    // of `app::settings_store::save_to_path`.
    f.sync_all()?;
    Ok(())
}

/// Create the leaf directory owner-only on Unix; ordinary mode elsewhere.
///
/// Missing parents keep their default mode, because the per-user data
/// directory above them is not this function's artefact. An existing
/// directory is tightened to the owner, not treated as an error.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        const PRIVATE_DIR_MODE: u32 = 0o700;
        match fs::DirBuilder::new().mode(PRIVATE_DIR_MODE).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !fs::metadata(path)?.is_dir() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("{} exists and is not a directory", path.display()),
                    ));
                }
            }
            Err(e) => return Err(e),
        }
        restrict_mode(path, PRIVATE_DIR_MODE)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Create (or truncate) the export file owner-only on Unix.
fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        const PRIVATE_FILE_MODE: u32 = 0o600;
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(PRIVATE_FILE_MODE)
            .open(path)?;
        // Tighten the handle, not the path: an existing file from an older
        // release may still carry the umask mode.
        file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
    }
}

/// Tighten `path` to `mode` on Unix; a no-op elsewhere.
#[cfg(unix)]
fn restrict_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::{TranscriptSessionEntry, create_session};

    fn fresh_temp_dir(test_name: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("tardis-session-store-{test_name}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).expect("tempdir create");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn sample_session() -> TranscriptSession {
        let mut session = create_session(
            "demo".to_string(),
            "mock-local".to_string(),
            "en".to_string(),
            "es".to_string(),
            "unix:1700000000".to_string(),
        );
        session.ended_at = Some("unix:1700000060".to_string());
        session.entries.push(TranscriptSessionEntry {
            chunk_index: 1,
            transcript_text: Some("hello".to_string()),
            translated_text: Some("hola".to_string()),
            source_language: Some("en".to_string()),
            target_language: Some("es".to_string()),
            provider: Some("mock-local".to_string()),
            is_final: true,
        });
        session
    }

    #[test]
    fn session_export_dir_lives_under_the_app_data_dir() {
        let base = PathBuf::from("/tmp/fake-data");
        assert_eq!(
            session_export_dir(&base),
            PathBuf::from("/tmp/fake-data/tardis/sessions")
        );
    }

    #[test]
    fn export_session_writes_both_formats_and_returns_their_paths() {
        let dir = fresh_temp_dir("both_formats");
        let session = sample_session();

        let paths = export_session(&session, &dir).expect("export must succeed");

        assert_eq!(paths.json, dir.join("session_demo.json"));
        assert_eq!(paths.text, dir.join("session_demo.txt"));
        assert!(paths.json.is_file(), "JSON export must exist");
        assert!(paths.text.is_file(), "text export must exist");
        cleanup(&dir);
    }

    #[test]
    fn exported_json_parses_back_into_the_same_session() {
        // Acceptance: the JSON a caller exports is valid on
        // `serde_json::from_str`, not just valid on the way out.
        let dir = fresh_temp_dir("json_round_trip");
        let session = sample_session();

        let paths = export_session(&session, &dir).expect("export must succeed");
        let body = fs::read_to_string(&paths.json).expect("read JSON export");
        let parsed: TranscriptSession =
            serde_json::from_str(&body).expect("exported JSON must deserialize");

        assert_eq!(parsed, session);
        cleanup(&dir);
    }

    #[test]
    fn exported_text_is_readable_and_carries_the_entries() {
        let dir = fresh_temp_dir("text_readable");
        let paths = export_session(&sample_session(), &dir).expect("export must succeed");

        let body = fs::read_to_string(&paths.text).expect("read text export");
        assert!(body.contains("session_id:           demo"), "got:\n{body}");
        assert!(body.contains("transcript:       hello"), "got:\n{body}");
        assert!(
            body.contains("translation (en -> es): hola"),
            "got:\n{body}"
        );
        cleanup(&dir);
    }

    #[test]
    fn export_session_creates_the_missing_sessions_directory() {
        let dir = fresh_temp_dir("creates_dir");
        let sessions = session_export_dir(&dir);

        let paths = export_session(&sample_session(), &sessions).expect("export must succeed");

        assert_eq!(paths.json, sessions.join("session_demo.json"));
        assert!(sessions.is_dir());
        cleanup(&dir);
    }

    #[test]
    fn a_second_export_replaces_the_first_instead_of_piling_up_files() {
        let dir = fresh_temp_dir("replace");
        let mut first = sample_session();
        first.entries[0].transcript_text = Some("first".to_string());
        let mut second = sample_session();
        second.entries[0].transcript_text = Some("second".to_string());

        export_session(&first, &dir).expect("first export");
        let paths = export_session(&second, &dir).expect("second export");

        let body = fs::read_to_string(&paths.text).expect("read text export");
        assert!(body.contains("second"), "newest export wins:\n{body}");
        assert!(!body.contains("first"), "old content must be gone:\n{body}");
        let count = fs::read_dir(&dir)
            .expect("list exports")
            .filter_map(Result::ok)
            .count();
        assert_eq!(count, 2, "one JSON + one text file, got {count}");
        cleanup(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn exports_are_owner_only_regardless_of_the_umask() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = fresh_temp_dir("owner_only");
        let sessions = session_export_dir(&dir);
        let paths = export_session(&sample_session(), &sessions).expect("export must succeed");

        let dir_mode = fs::metadata(&sessions)
            .expect("sessions dir")
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700, "sessions dir must be 0700");
        for path in [&paths.json, &paths.text] {
            let mode = fs::metadata(path)
                .expect("export file")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{} must be 0600", path.display());
        }
        cleanup(&dir);
    }
}
