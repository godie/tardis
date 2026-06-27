//! Pure helpers for persisting [`AppRuntimeConfig`] to a JSON
//! file on disk.
//!
//! Persistence lives behind two pure helpers
//! ([`save_to_path`] / [`load_from_path_or_default`]) plus a
//! path builder ([`settings_file_path`]) so:
//!
//! * the CLI can target any directory it likes (tests use
//!   `std::env::temp_dir()`)
//! * the Tauri shell targets the canonical OS config dir
//!   resolved through `tauri::app::path::PathResolver`
//! * every layer below the path is pure and unit-testable
//!   without touching CPAL, HTTP, or the Tauri runtime
//!
//! `# Errors`
//!
//! [`SettingsStoreError`] covers the four real failure modes
//! (FS I/O, parse, serialize, atomic rename, and validation)
//! with the underlying std / serde error preserved so callers
//! can format a useful diagnostic. Round-trip is best-effort:
//! `load_from_path_or_default` returns
//! [`AppRuntimeConfig::default`] when the file does not exist so
//! "first run" is not an error.
//!
//! Atomicity is guaranteed by a write-then-rename pattern with
//! an explicit `sync_all()` on the temp file before the rename;
//! an interrupted save can either leave the original file
//! untouched (rename fails) or leave a fully-flushed new file in
//! place (rename succeeds). The temp file is best-effort
//! removed on rename failure so a crash mid-write does not leave
//! clutter behind.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::app::config::{AppRuntimeConfig, normalize_runtime_config, validate_runtime_config};

/// Top-level subdirectory under the OS config dir. Namespaced so
/// a future second-purpose file (eg. `models.json`) does not
/// collide.
pub const SETTINGS_SUBDIR: &str = "tardis";

/// Stable name of this app's settings file within
/// [`SETTINGS_SUBDIR`].
pub const SETTINGS_FILE_NAME: &str = "runtime.json";

/// Construct the canonical settings file path inside `base_dir`:
/// `<base_dir>/<SETTINGS_SUBDIR>/<SETTINGS_FILE_NAME>`.
///
/// Pure — does not touch the filesystem. Callers are free to use
/// `tauri::app::path::PathResolver::config_dir()`,
/// `std::env::var("HOME")`, or a tempdir in tests.
pub fn settings_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(SETTINGS_SUBDIR).join(SETTINGS_FILE_NAME)
}

/// Errors specific to settings persistence.
///
/// Wraps the underlying `std::io::Error` for the FS paths and
/// `serde_json::Error` for parse failures so callers can match on
/// them and re-them through their own error type if needed.
#[derive(Debug)]
pub enum SettingsStoreError {
    /// Filesystem read / write / create_dir_all / rename failure
    /// other than a "not found" read (which load treats as
    /// "no settings yet").
    Io(io::Error),
    /// The file existed but could not be parsed (eg. corrupted
    /// JSON or partial write from a prior crash).
    Parse(serde_json::Error),
    /// Serializing the in-memory config failed — defensive,
    /// because `AppRuntimeConfig` only contains
    /// `String` / `u64` / `f32` fields, all of which `serde_json`
    /// round-trips unconditionally.
    Serialize(serde_json::Error),
    /// Atomic rename of the temp file into place failed. The
    /// original file (if any) remains untouched in that case.
    Rename(io::Error),
    /// Validation failed against the round-tripped config
    /// (eg. on-disk file has an out-of-bounds threshold from an
    /// older app version that the user no longer edited).
    Validation(String),
}

impl std::fmt::Display for SettingsStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "settings file I/O failed: {e}"),
            Self::Parse(e) => write!(f, "settings file is not valid JSON: {e}"),
            Self::Serialize(e) => write!(f, "serializing settings failed: {e}"),
            Self::Rename(e) => write!(f, "atomic rename of settings file failed: {e}"),
            Self::Validation(msg) => write!(f, "loaded settings failed validation: {msg}"),
        }
    }
}

impl std::error::Error for SettingsStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) | Self::Rename(e) => Some(e),
            Self::Parse(e) | Self::Serialize(e) => Some(e),
            Self::Validation(_) => None,
        }
    }
}

impl From<io::Error> for SettingsStoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Atomically write `config` to `path`:
///
/// 1. Validate the config so an in-memory bad value never reaches
///    disk.
/// 2. Normalize, then pretty-print to JSON.
/// 3. Write the JSON to `<path>.tmp`, fsync, and atomically
///    rename over `path`. The rename either succeeds (the new
///    file is in place) or fails (the original is untouched and
///    the temp file is best-effort cleaned up).
///
/// Failures leave neither a corrupted file nor — under normal
/// failure modes — a stray temp file behind.
pub fn save_to_path(path: &Path, config: &AppRuntimeConfig) -> Result<(), SettingsStoreError> {
    validate_runtime_config(config).map_err(SettingsStoreError::Validation)?;
    let normalized = normalize_runtime_config(config.clone());
    let json = serde_json::to_string_pretty(&normalized).map_err(SettingsStoreError::Serialize)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = tmp_path(path);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all().or_else(|e| {
            // Best-effort cleanup of the temp when fsync fails so
            // we don't leak the partial write.
            let _ = fs::remove_file(&tmp);
            Err(e)
        })?;
    }

    fs::rename(&tmp, path).map_err(|rename_err| {
        let _ = fs::remove_file(&tmp);
        SettingsStoreError::Rename(rename_err)
    })?;

    Ok(())
}

/// Read [`AppRuntimeConfig`] from `path`. If the file does not
/// exist, returns `Ok(AppRuntimeConfig::default())` — the
/// "first run" case is **not** an error.
///
/// The parsed config is normalized and validated before being
/// returned. A corrupt or out-of-bounds file surfaces as
/// [`SettingsStoreError::Parse`] or
/// [`SettingsStoreError::Validation`] so the caller can decide
/// whether to fall back to defaults or surface the failure.
pub fn load_from_path_or_default(path: &Path) -> Result<AppRuntimeConfig, SettingsStoreError> {
    let bytes = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(AppRuntimeConfig::default());
        }
        Err(e) => return Err(SettingsStoreError::Io(e)),
    };
    let parsed: AppRuntimeConfig =
        serde_json::from_str(&bytes).map_err(SettingsStoreError::Parse)?;
    let normalized = normalize_runtime_config(parsed);
    validate_runtime_config(&normalized).map_err(SettingsStoreError::Validation)?;
    Ok(normalized)
}

/// `<path>.tmp` — a stable suffix for the staged write so a
/// process crash leaves a recognizable file behind.
fn tmp_path(path: &Path) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

// ===== Unit tests ========================================================
//
// Persistence tests need an isolated directory per test to avoid
// cross-test interference. `Cargo` may run tests in parallel, so
// each test gets a unique subdirectory under
// `std::env::temp_dir()`. Every test cleans up its own dir on
// success; failures leave the dir behind so a developer can
// inspect what survived.

#[cfg(test)]
mod tests {
    use super::*;

    /// Allocate a fresh tempdir for a single test invocation. The
    /// name encodes the test name + pid + nanos so two parallel
    /// tests with the same name never collide.
    fn fresh_temp_dir(test_name: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("tardis-settings-store-{test_name}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).expect("tempdir create");
        dir
    }

    /// Best-effort cleanup; ignore errors so a stray leftover
    /// does not surface as a test failure.
    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    // ---- settings_file_path -------------------------------------------

    #[test]
    fn settings_file_path_joins_subdir_and_filename() {
        let base = PathBuf::from("/tmp/fake-config");
        let p = settings_file_path(&base);
        assert_eq!(p, PathBuf::from("/tmp/fake-config/tardis/runtime.json"));
    }

    // ---- save_to_path + load_from_path_or_default round trip ----------

    #[test]
    fn save_then_load_round_trip_preserves_config() {
        let dir = fresh_temp_dir("round_trip_preserves");
        let path = settings_file_path(&dir);

        let cfg = AppRuntimeConfig {
            transcription_provider: "local-whisper".to_string(),
            source_language: "fr".to_string(),
            target_language: "it".to_string(),
            chunk_duration_ms: 750,
            volume_threshold: 0.04,
        };
        save_to_path(&path, &cfg).expect("save must succeed");
        let loaded = load_from_path_or_default(&path).expect("load must succeed");
        assert_eq!(loaded, cfg);
        cleanup(&dir);
    }

    #[test]
    fn save_creates_parent_dir_when_missing() {
        let dir = fresh_temp_dir("creates_parent");
        let path = settings_file_path(&dir);
        // The subdir does not yet exist; the helper must make it.
        assert!(!path.parent().unwrap().exists());
        save_to_path(&path, &AppRuntimeConfig::default()).expect("save must succeed");
        assert!(path.exists());
        cleanup(&dir);
    }

    #[test]
    fn save_overwrites_existing_file_cleanly() {
        let dir = fresh_temp_dir("overwrites");
        let path = settings_file_path(&dir);

        let first = AppRuntimeConfig::default();
        save_to_path(&path, &first).expect("first save");

        let second = AppRuntimeConfig {
            chunk_duration_ms: 2000,
            ..AppRuntimeConfig::default()
        };
        save_to_path(&path, &second).expect("second save");

        let loaded = load_from_path_or_default(&path).expect("load must succeed");
        assert_eq!(loaded, second);
        cleanup(&dir);
    }

    #[test]
    fn save_rejects_invalid_config_before_writing() {
        // Validation runs before disk I/O so an invalid in-memory
        // value never reaches the user's settings file.
        let dir = fresh_temp_dir("rejects_invalid");
        let path = settings_file_path(&dir);

        let bad = AppRuntimeConfig {
            chunk_duration_ms: 50, // below MIN_CHUNK_DURATION_MS
            ..AppRuntimeConfig::default()
        };
        let err = save_to_path(&path, &bad).expect_err("must reject");
        assert!(
            matches!(err, SettingsStoreError::Validation(_)),
            "expected Validation variant, got: {err:?}"
        );
        assert!(!path.exists(), "rejected save must not touch disk");
        cleanup(&dir);
    }

    #[test]
    fn save_no_tmp_file_left_after_success() {
        let dir = fresh_temp_dir("no_tmp_on_success");
        let path = settings_file_path(&dir);

        save_to_path(&path, &AppRuntimeConfig::default()).expect("save");
        assert!(path.exists(), "final file must exist");
        assert!(
            !tmp_path(&path).exists(),
            "temp file must be cleaned up after rename"
        );
        cleanup(&dir);
    }

    // ---- load_from_path_or_default ------------------------------------

    #[test]
    fn load_returns_default_when_file_missing() {
        let dir = fresh_temp_dir("missing_file");
        let path = settings_file_path(&dir);
        // Never saved; load must still succeed and return defaults.
        let loaded = load_from_path_or_default(&path).expect("missing file is not Err");
        assert_eq!(loaded, AppRuntimeConfig::default());
        cleanup(&dir);
    }

    #[test]
    fn load_rejects_corrupt_json() {
        let dir = fresh_temp_dir("corrupt");
        let path = settings_file_path(&dir);
        // Do not bypass save_to_path (which would trigger
        // validation) — write directly so the helper sees raw
        // bytes it cannot parse.
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ this is not json").unwrap();

        let err = load_from_path_or_default(&path).expect_err("corrupt JSON must error");
        assert!(
            matches!(err, SettingsStoreError::Parse(_)),
            "expected Parse variant, got: {err:?}"
        );
        cleanup(&dir);
    }

    #[test]
    fn load_rejects_known_shape_but_invalid_values() {
        // Valid JSON shape but with an out-of-bounds threshold
        // (eg. left over from an older build that allowed higher
        // values). The post-load validator must surface this as
        // an error rather than silently returning broken
        // defaults — the calling Tauri command can decide
        // whether to fall back to `AppRuntimeConfig::default()`.
        let dir = fresh_temp_dir("invalid_values_on_disk");
        let path = settings_file_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{
  "transcription_provider": "mock-local",
  "source_language": "en",
  "target_language": "es",
  "chunk_duration_ms": 1000,
  "volume_threshold": 1.5
}"#,
        )
        .unwrap();

        let err = load_from_path_or_default(&path).expect_err("out-of-bounds threshold must error");
        assert!(
            matches!(err, SettingsStoreError::Validation(_)),
            "expected Validation variant, got: {err:?}"
        );
        cleanup(&dir);
    }

    #[test]
    fn load_normalizes_whitespace_before_returning() {
        // A user-edited file (or a future `Save As…` path) that
        // padded the provider must still round-trip cleanly
        // because load normalizes post-parse.
        let dir = fresh_temp_dir("normalize_on_load");
        let path = settings_file_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{
  "transcription_provider": "  mock-local  ",
  "source_language": " en ",
  "target_language": "es",
  "chunk_duration_ms": 1000,
  "volume_threshold": 0.01
}"#,
        )
        .unwrap();

        let loaded = load_from_path_or_default(&path).expect("load must succeed");
        assert_eq!(loaded.transcription_provider, "mock-local");
        assert_eq!(loaded.source_language, "en");
        cleanup(&dir);
    }

    // ---- SettingsStoreError: Display + source compatibility ----------

    #[test]
    fn settings_store_error_display_mentions_failure_mode() {
        let io = SettingsStoreError::Io(io::Error::new(io::ErrorKind::PermissionDenied, "x"));
        let s = format!("{io}");
        assert!(s.contains("I/O"), "display must mention I/O, got: {s}");

        let parse =
            SettingsStoreError::Parse(serde_json::from_str::<u32>("not-a-number").unwrap_err());
        let s = format!("{parse}");
        assert!(
            s.contains("not valid JSON"),
            "display must mention parse failure, got: {s}"
        );

        let validation = SettingsStoreError::Validation("bad threshold".to_string());
        let s = format!("{validation}");
        assert!(
            s.contains("validation") && s.contains("bad threshold"),
            "display must include the validation message, got: {s}"
        );
    }
}
