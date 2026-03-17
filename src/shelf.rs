//! Auto-shelving for Ivaldi VCS.
//!
//! When switching timelines, uncommitted changes are automatically saved
//! to a shelf and restored when returning. Shelves are transparent to the
//! user and managed automatically.
//!
//! Storage: `.ivaldi/shelves/<timeline-name>.json`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::B3Hash;

/// A shelved workspace state.
#[derive(Debug, Clone)]
pub struct Shelf {
    /// Timeline this shelf belongs to.
    pub timeline: String,
    /// Staged files at the time of shelving: path → content hash.
    pub staged_files: BTreeMap<String, B3Hash>,
    /// Timestamp when shelf was created (Unix seconds).
    pub created_at: i64,
}

/// Manages auto-shelving operations.
pub struct ShelfManager {
    shelf_dir: PathBuf,
}

impl ShelfManager {
    /// Create a new shelf manager.
    pub fn new(ivaldi_dir: &Path) -> Self {
        let shelf_dir = ivaldi_dir.join("shelves");
        let _ = fs::create_dir_all(&shelf_dir);
        Self { shelf_dir }
    }

    /// Save a shelf for the given timeline.
    pub fn save_shelf(&self, shelf: &Shelf) -> Result<(), ShelfError> {
        let path = self.shelf_path(&shelf.timeline);

        let mut lines = Vec::new();
        lines.push(format!("timeline {}", shelf.timeline));
        lines.push(format!("created_at {}", shelf.created_at));

        for (file_path, hash) in &shelf.staged_files {
            lines.push(format!("staged {} {}", hash, file_path));
        }

        fs::write(&path, lines.join("\n")).map_err(ShelfError::Io)?;
        Ok(())
    }

    /// Load a shelf for the given timeline, if one exists.
    pub fn load_shelf(&self, timeline: &str) -> Result<Option<Shelf>, ShelfError> {
        let path = self.shelf_path(timeline);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ShelfError::Io(e)),
        };

        let mut shelf = Shelf {
            timeline: timeline.to_string(),
            staged_files: BTreeMap::new(),
            created_at: 0,
        };

        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("timeline ") {
                shelf.timeline = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("created_at ") {
                shelf.created_at = rest.parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("staged ") {
                if let Some((hash_str, path)) = rest.split_once(' ') {
                    if let Some(hash) = B3Hash::from_hex(hash_str) {
                        shelf.staged_files.insert(path.to_string(), hash);
                    }
                }
            }
        }

        Ok(Some(shelf))
    }

    /// Remove a shelf for the given timeline.
    pub fn remove_shelf(&self, timeline: &str) -> Result<(), ShelfError> {
        let path = self.shelf_path(timeline);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ShelfError::Io(e)),
        }
    }

    /// Check if a shelf exists for the given timeline.
    pub fn has_shelf(&self, timeline: &str) -> bool {
        self.shelf_path(timeline).exists()
    }

    /// List all timelines that have shelves.
    pub fn list_shelves(&self) -> Result<Vec<String>, ShelfError> {
        let entries = match fs::read_dir(&self.shelf_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ShelfError::Io(e)),
        };

        let mut timelines = Vec::new();
        for entry in entries {
            let entry = entry.map_err(ShelfError::Io)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(timeline) = name.strip_suffix(".shelf") {
                timelines.push(timeline.to_string());
            }
        }
        timelines.sort();
        Ok(timelines)
    }

    fn shelf_path(&self, timeline: &str) -> PathBuf {
        self.shelf_dir.join(format!("{}.shelf", timeline))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShelfError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, ShelfManager) {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();
        let mgr = ShelfManager::new(&ivaldi_dir);
        (dir, mgr)
    }

    #[test]
    fn save_and_load() {
        let (_dir, mgr) = setup();

        let mut shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            created_at: 1700000000,
        };
        shelf
            .staged_files
            .insert("file.txt".into(), B3Hash::digest(b"content"));
        shelf
            .staged_files
            .insert("src/main.rs".into(), B3Hash::digest(b"fn main()"));

        mgr.save_shelf(&shelf).unwrap();
        assert!(mgr.has_shelf("feature"));

        let loaded = mgr.load_shelf("feature").unwrap().unwrap();
        assert_eq!(loaded.timeline, "feature");
        assert_eq!(loaded.created_at, 1700000000);
        assert_eq!(loaded.staged_files.len(), 2);
        assert!(loaded.staged_files.contains_key("file.txt"));
        assert!(loaded.staged_files.contains_key("src/main.rs"));
    }

    #[test]
    fn load_nonexistent() {
        let (_dir, mgr) = setup();
        let result = mgr.load_shelf("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn remove_shelf() {
        let (_dir, mgr) = setup();

        let shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            created_at: 0,
        };
        mgr.save_shelf(&shelf).unwrap();
        assert!(mgr.has_shelf("feature"));

        mgr.remove_shelf("feature").unwrap();
        assert!(!mgr.has_shelf("feature"));
    }

    #[test]
    fn remove_nonexistent_ok() {
        let (_dir, mgr) = setup();
        mgr.remove_shelf("nonexistent").unwrap(); // Should not error
    }

    #[test]
    fn list_shelves() {
        let (_dir, mgr) = setup();

        for name in &["main", "feature", "hotfix"] {
            let shelf = Shelf {
                timeline: name.to_string(),
                staged_files: BTreeMap::new(),
                created_at: 0,
            };
            mgr.save_shelf(&shelf).unwrap();
        }

        let list = mgr.list_shelves().unwrap();
        assert_eq!(list, vec!["feature", "hotfix", "main"]); // sorted
    }

    #[test]
    fn has_shelf() {
        let (_dir, mgr) = setup();
        assert!(!mgr.has_shelf("feature"));

        let shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            created_at: 0,
        };
        mgr.save_shelf(&shelf).unwrap();
        assert!(mgr.has_shelf("feature"));
    }

    #[test]
    fn empty_shelf() {
        let (_dir, mgr) = setup();

        let shelf = Shelf {
            timeline: "empty".into(),
            staged_files: BTreeMap::new(),
            created_at: 0,
        };
        mgr.save_shelf(&shelf).unwrap();

        let loaded = mgr.load_shelf("empty").unwrap().unwrap();
        assert!(loaded.staged_files.is_empty());
    }

    #[test]
    fn overwrite_shelf() {
        let (_dir, mgr) = setup();

        let shelf1 = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::from([("old.txt".into(), B3Hash::digest(b"old"))]),
            created_at: 100,
        };
        mgr.save_shelf(&shelf1).unwrap();

        let shelf2 = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::from([("new.txt".into(), B3Hash::digest(b"new"))]),
            created_at: 200,
        };
        mgr.save_shelf(&shelf2).unwrap();

        let loaded = mgr.load_shelf("feature").unwrap().unwrap();
        assert_eq!(loaded.created_at, 200);
        assert!(loaded.staged_files.contains_key("new.txt"));
        assert!(!loaded.staged_files.contains_key("old.txt"));
    }
}
