//! Atomic file replacement for repository metadata files.
//!
//! A plain `fs::write` can leave a truncated file behind if the process
//! crashes mid-write. Every metadata file under `.ivaldi/` (staging area,
//! HEAD, shelves, merge state, config, ...) goes through [`atomic_write`]
//! instead: the bytes are written to a unique temp file in the same
//! directory, fsynced, then renamed over the destination. Readers observe
//! either the old contents or the new contents, never a partial file.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replace `path` with `bytes`.
///
/// Writes to a temp file in the same directory (so the rename cannot cross
/// filesystems), fsyncs it, renames it over `path`, then best-effort fsyncs
/// the parent directory. The parent directory must already exist.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_impl(path, bytes, false)
}

/// Atomically replace a secret file, creating it with owner-only permissions.
///
/// On Unix the temporary file is created as mode 0600, so secret bytes are
/// never visible through a permissive process umask before the final rename.
pub fn atomic_write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_impl(path, bytes, true)
}

fn atomic_write_impl(path: &Path, bytes: &[u8], _secret: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("atomic_write: path has no file name"))?;

    let tmp = parent.join(format!(
        "{}.tmp.{}.{}",
        file_name.to_string_lossy(),
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if _secret {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        replace_file(&tmp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }

    // Make the rename itself durable. Failure here is tolerated (some
    // filesystems reject directory fsync); the rename has already happened.
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers refer to NUL-terminated UTF-16 buffers that remain
    // alive for the duration of the call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_entries(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| {
                let name = e.unwrap().file_name().to_string_lossy().into_owned();
                name.contains(".tmp.").then_some(name)
            })
            .collect()
    }

    #[test]
    fn write_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");

        atomic_write(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");

        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    #[test]
    fn no_temp_files_left_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        atomic_write(&path, b"data").unwrap();
        atomic_write(&path, b"data2").unwrap();
        assert!(tmp_entries(dir.path()).is_empty());
    }

    #[test]
    fn failure_cleans_up_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // Renaming a file over an existing non-empty directory fails.
        let target = dir.path().join("occupied");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("child"), b"x").unwrap();

        assert!(atomic_write(&target, b"data").is_err());
        assert!(tmp_entries(dir.path()).is_empty());
    }

    #[test]
    fn missing_parent_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no/such/dir/state");
        assert!(atomic_write(&path, b"data").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secret_write_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        atomic_write_secret(&path, b"sensitive").unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
