//! Process-level repository lock.
//!
//! redb serializes individual store transactions, but multi-step operations
//! (seal, timeline switch, fuse, ...) also touch plain files under
//! `.ivaldi/` (HEAD, staging, shelves) with no coordination. [`RepoLock`]
//! gives mutating commands an exclusive advisory `flock(2)` on
//! `.ivaldi/repo.lock` so two concurrent ivaldi processes can't interleave.
//!
//! The kernel releases the lock when the holding process exits — including
//! on crash — so a stale lock file is never a problem. (This is why an
//! `O_CREAT|O_EXCL` sentinel file was rejected.) Read-only commands take no
//! lock; they still serialize against writers via redb's own file lock.

use std::fs;
use std::io::Write;
use std::path::Path;

use rustix::fs::{FlockOperation, flock};

/// Held for the duration of a mutating command. The flock is released when
/// this struct is dropped (or the process dies).
#[derive(Debug)]
pub struct RepoLock {
    _file: fs::File,
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error(
        "another ivaldi process is operating on this repository \
         (lock held on .ivaldi/repo.lock). Wait for it to finish and retry."
    )]
    Contended,
    #[error("I/O error acquiring repository lock: {0}")]
    Io(#[from] std::io::Error),
}

impl RepoLock {
    /// Open/create `.ivaldi/repo.lock` and take a non-blocking exclusive lock.
    pub fn acquire(ivaldi_dir: &Path) -> Result<RepoLock, LockError> {
        let path = ivaldi_dir.join("repo.lock");
        let mut file = fs::File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)?;

        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            if e == rustix::io::Errno::WOULDBLOCK {
                LockError::Contended
            } else {
                LockError::Io(std::io::Error::from(e))
            }
        })?;

        // Diagnostic only — never read for correctness. Safe: we hold the lock.
        let _ = file.set_len(0);
        let _ = writeln!(file, "{}", std::process::id());

        Ok(RepoLock { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".ivaldi")).unwrap();
        dir
    }

    #[test]
    fn second_acquire_contends() {
        let dir = setup();
        let ivaldi_dir = dir.path().join(".ivaldi");

        // flock is per open file description, so two acquires in one process
        // genuinely contend.
        let first = RepoLock::acquire(&ivaldi_dir).unwrap();
        let second = RepoLock::acquire(&ivaldi_dir);
        assert!(matches!(second, Err(LockError::Contended)));
        let msg = second.unwrap_err().to_string();
        assert!(msg.contains("another ivaldi process"));

        drop(first);
        RepoLock::acquire(&ivaldi_dir).unwrap();
    }

    #[test]
    fn creates_lock_file() {
        let dir = setup();
        let ivaldi_dir = dir.path().join(".ivaldi");
        let _lock = RepoLock::acquire(&ivaldi_dir).unwrap();
        assert!(ivaldi_dir.join("repo.lock").exists());
    }

    #[test]
    fn missing_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = RepoLock::acquire(&dir.path().join("no-such-dir"));
        assert!(matches!(result, Err(LockError::Io(_))));
    }
}
