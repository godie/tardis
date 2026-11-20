//! Owner-only permissions for the artefacts a session writes to disk.
//!
//! Session history and live-utterance WAVs contain private speech, so they
//! must not depend on the process umask: a common `022` leaves a newly
//! created file world-readable. Every directory this module creates is
//! `0700` and every file `0600` on Unix; other platforms keep the default,
//! where the per-user profile directory already carries the access control.
//!
//! The permission is set twice on purpose. The `mode` passed to
//! [`std::fs::OpenOptions`] applies at creation (the umask can only clear
//! bits), and the `set_permissions` call tightens a file or directory left
//! behind by an older build that created it with the default mode.

use std::fs;
use std::io;
use std::path::Path;

/// Create `path` and any missing parents, restricted to the owner on Unix.
///
/// The creation mode is applied by the builder, so the directory is never
/// briefly world-traversable while it is empty. Calling this on an existing
/// directory is a repair, not an error: a `0755` directory from an older
/// release becomes `0700`.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        const OWNER_ONLY_DIR: u32 = 0o700;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(OWNER_ONLY_DIR)
            .create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    restrict_dir(path)
}

/// Create (or truncate) `path` owner-only and return it for writing.
pub fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    open_restricted(options, path)
}

/// Open `path` owner-only for reading and appending, creating it if needed.
///
/// Read access is part of the contract: the history writer walks the tail
/// of the file to repair a record left unterminated by a crash.
pub fn open_private_append(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).append(true).create(true);
    open_restricted(options, path)
}

/// Apply the creation mode before the file exists, then tighten the handle
/// for the case where the file already existed.
fn open_restricted(options: fs::OpenOptions, path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut options = options;
        options.mode(OWNER_ONLY_FILE);
        options
    };
    let file = options.open(path)?;
    restrict_handle(&file)?;
    Ok(file)
}

#[cfg(unix)]
const OWNER_ONLY_FILE: u32 = 0o600;

#[cfg(unix)]
fn restrict_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    const OWNER_ONLY_DIR: u32 = 0o700;
    fs::set_permissions(path, fs::Permissions::from_mode(OWNER_ONLY_DIR))
}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_handle(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(fs::Permissions::from_mode(OWNER_ONLY_FILE))
}

#[cfg(not(unix))]
fn restrict_handle(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::path::PathBuf;

    /// Allocate a fresh tempdir for a single test invocation (same pattern
    /// as `app::settings_store`'s tests).
    fn fresh_temp_dir(test_name: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("tardis-private-fs-{test_name}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).expect("tempdir create");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn create_private_dir_makes_nested_directories() {
        let root = fresh_temp_dir("dir_nested");
        let nested = root.join("app-data").join("live_chunks");

        create_private_dir(&nested).expect("nested dir must be created");

        assert!(nested.is_dir());
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn create_private_dir_restricts_a_new_directory_to_the_owner() {
        let root = fresh_temp_dir("dir_mode");
        let target = root.join("history");

        create_private_dir(&target).expect("dir created");

        assert_eq!(mode(&target), 0o700);
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn create_private_dir_tightens_an_existing_directory() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = fresh_temp_dir("dir_repair");
        let target = root.join("output");
        fs::create_dir_all(&target).expect("seed dir");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("loosen");

        create_private_dir(&target).expect("repair");

        assert_eq!(mode(&target), 0o700);
        cleanup(&root);
    }

    #[test]
    fn create_private_file_writes_the_content() {
        let root = fresh_temp_dir("file_write");
        let path = root.join("utterance.wav");

        {
            let mut file = create_private_file(&path).expect("file created");
            file.write_all(b"RIFF").expect("write");
        }

        let mut body = Vec::new();
        fs::File::open(&path)
            .expect("open")
            .read_to_end(&mut body)
            .expect("read");
        assert_eq!(body, b"RIFF");
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn create_private_file_is_owner_only() {
        let root = fresh_temp_dir("file_mode");
        let path = root.join("utterance.wav");

        let file = create_private_file(&path).expect("file created");
        drop(file);

        assert_eq!(mode(&path), 0o600);
        cleanup(&root);
    }

    #[test]
    fn open_private_append_creates_and_appends() {
        let root = fresh_temp_dir("append");
        let path = root.join("history.jsonl");

        {
            let mut file = open_private_append(&path).expect("create");
            file.write_all(b"one\n").expect("first write");
        }
        {
            let mut file = open_private_append(&path).expect("reopen");
            file.write_all(b"two\n").expect("second write");
        }

        assert_eq!(fs::read_to_string(&path).expect("read"), "one\ntwo\n");
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn open_private_append_tightens_an_existing_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = fresh_temp_dir("append_repair");
        let path = root.join("history.jsonl");
        fs::write(&path, "one\n").expect("seed file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("loosen");

        open_private_append(&path).expect("reopen");

        assert_eq!(mode(&path), 0o600);
        cleanup(&root);
    }
}
