//! Crash journal for timeline switches.
//!
//! A timeline switch is multi-step: shelve the current timeline's dirty
//! state, rewrite HEAD, materialize the target tree, restore the target's
//! shelf. A crash mid-sequence used to leave the working tree half
//! transitioned with nothing recording that fact. The journal file
//! (`.ivaldi/SWITCH_IN_PROGRESS`) is written after the shelve phase (the
//! only non-idempotent part) and removed after the switch completes; while
//! it exists, mutating commands refuse to run and `timeline switch` offers
//! to complete or roll back the transition.

use std::path::Path;

pub const JOURNAL_FILE: &str = "SWITCH_IN_PROGRESS";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwitchJournal {
    /// Timeline the interrupted switch was leaving.
    pub from: String,
    /// Timeline the interrupted switch was entering.
    pub to: String,
    /// Whether a shelf for `from` was saved (vs. removed-as-clean).
    pub shelf_saved: bool,
    /// Unix time the switch started (diagnostic only).
    pub started_at: i64,
}

pub fn write(ivaldi_dir: &Path, journal: &SwitchJournal) -> std::io::Result<()> {
    let data =
        serde_json::to_string_pretty(journal).map_err(|e| std::io::Error::other(e.to_string()))?;
    crate::atomic_io::atomic_write(&ivaldi_dir.join(JOURNAL_FILE), data.as_bytes())
}

/// Load the journal if one exists. Corrupt JSON is an error (conservative:
/// the user should inspect rather than have it silently ignored).
pub fn load(ivaldi_dir: &Path) -> std::io::Result<Option<SwitchJournal>> {
    let path = ivaldi_dir.join(JOURNAL_FILE);
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data)
            .map(Some)
            .map_err(|e| std::io::Error::other(format!("corrupt {}: {}", JOURNAL_FILE, e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn clear(ivaldi_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(ivaldi_dir.join(JOURNAL_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());

        let journal = SwitchJournal {
            from: "main".into(),
            to: "feature".into(),
            shelf_saved: true,
            started_at: 1700000000,
        };
        write(dir.path(), &journal).unwrap();

        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.from, "main");
        assert_eq!(loaded.to, "feature");
        assert!(loaded.shelf_saved);

        clear(dir.path()).unwrap();
        assert!(load(dir.path()).unwrap().is_none());
        // Clearing again is fine.
        clear(dir.path()).unwrap();
    }

    #[test]
    fn corrupt_journal_is_an_error_not_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(JOURNAL_FILE), "{truncated").unwrap();
        assert!(load(dir.path()).is_err());
    }
}
