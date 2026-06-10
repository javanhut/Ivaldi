//! Auto-shelving for Ivaldi VCS.
//!
//! When switching timelines, uncommitted changes are automatically saved
//! to a shelf and restored when returning. Shelves are transparent to the
//! user and managed automatically — there is no `stash` command.
//!
//! A shelf captures three kinds of state for a timeline at the moment of
//! switch-away:
//!
//! - **Staged files** — entries in the staging area, by path and blob hash.
//! - **Workspace changes** — modifications, untracked files, and deletions
//!   relative to the timeline's tip tree. Modified/Untracked file content
//!   is hashed into the CAS so the bytes survive even after the working
//!   tree is rewritten by the next timeline's materialize.
//!
//! Storage: `.ivaldi/shelves/<timeline-name>.shelf`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::B3Hash;

/// A single working-tree change captured by the shelf.
///
/// `Modified` and `Untracked` carry a hash because the content lives in the
/// CAS and must be re-applied on switch-back. `Deleted` only needs the path
/// because the tip tree itself supplies the original bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceChange {
    Modified { path: String, hash: B3Hash },
    Untracked { path: String, hash: B3Hash },
    Deleted { path: String },
}

/// A shelved workspace state.
#[derive(Debug, Clone)]
pub struct Shelf {
    /// Timeline this shelf belongs to.
    pub timeline: String,
    /// Staged files at the time of shelving: path → content hash.
    pub staged_files: BTreeMap<String, B3Hash>,
    /// Working-tree changes vs the timeline tip at shelf time.
    pub workspace_changes: Vec<WorkspaceChange>,
    /// Timestamp when shelf was created (Unix seconds).
    pub created_at: i64,
}

impl Shelf {
    /// True if the shelf has nothing to restore.
    pub fn is_empty(&self) -> bool {
        self.staged_files.is_empty() && self.workspace_changes.is_empty()
    }
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

        for change in &shelf.workspace_changes {
            match change {
                WorkspaceChange::Modified { path, hash } => {
                    lines.push(format!("modified {} {}", hash, path));
                }
                WorkspaceChange::Untracked { path, hash } => {
                    lines.push(format!("untracked {} {}", hash, path));
                }
                WorkspaceChange::Deleted { path } => {
                    lines.push(format!("deleted {}", path));
                }
            }
        }

        crate::atomic_io::atomic_write(&path, lines.join("\n").as_bytes())
            .map_err(ShelfError::Io)?;
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
            workspace_changes: Vec::new(),
            created_at: 0,
        };

        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("timeline ") {
                shelf.timeline = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("created_at ") {
                shelf.created_at = rest.parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("staged ") {
                if let Some((hash_str, path)) = rest.split_once(' ')
                    && let Some(hash) = B3Hash::from_hex(hash_str)
                {
                    shelf.staged_files.insert(path.to_string(), hash);
                }
            } else if let Some(rest) = line.strip_prefix("modified ") {
                if let Some((hash_str, path)) = rest.split_once(' ')
                    && let Some(hash) = B3Hash::from_hex(hash_str)
                {
                    shelf.workspace_changes.push(WorkspaceChange::Modified {
                        path: path.to_string(),
                        hash,
                    });
                }
            } else if let Some(rest) = line.strip_prefix("untracked ") {
                if let Some((hash_str, path)) = rest.split_once(' ')
                    && let Some(hash) = B3Hash::from_hex(hash_str)
                {
                    shelf.workspace_changes.push(WorkspaceChange::Untracked {
                        path: path.to_string(),
                        hash,
                    });
                }
            } else if let Some(rest) = line.strip_prefix("deleted ") {
                shelf.workspace_changes.push(WorkspaceChange::Deleted {
                    path: rest.to_string(),
                });
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
    fn load_tolerates_truncated_file() {
        // A truncated shelf file (crash mid-write, pre-atomic-write era)
        // must parse the intact lines without panicking.
        let (_dir, mgr) = setup();
        let hash = B3Hash::digest(b"content");
        let content = format!(
            "timeline feature\ncreated_at 1700000000\nstaged {} file.txt\nmodified abc1",
            hash
        );
        fs::write(mgr.shelf_path("feature"), content).unwrap();

        let loaded = mgr.load_shelf("feature").unwrap().unwrap();
        assert_eq!(loaded.timeline, "feature");
        assert!(loaded.staged_files.contains_key("file.txt"));
    }

    #[test]
    fn save_and_load() {
        let (_dir, mgr) = setup();

        let mut shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            workspace_changes: Vec::new(),
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
    fn workspace_changes_round_trip() {
        let (_dir, mgr) = setup();

        let h_mod = B3Hash::digest(b"modified content");
        let h_unt = B3Hash::digest(b"new file content");
        let shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            workspace_changes: vec![
                WorkspaceChange::Modified {
                    path: "src/main.rs".into(),
                    hash: h_mod,
                },
                WorkspaceChange::Untracked {
                    path: "scratch/notes.md".into(),
                    hash: h_unt,
                },
                WorkspaceChange::Deleted {
                    path: ".gitignore".into(),
                },
            ],
            created_at: 1700000000,
        };
        mgr.save_shelf(&shelf).unwrap();

        let loaded = mgr.load_shelf("feature").unwrap().unwrap();
        assert_eq!(loaded.workspace_changes.len(), 3);
        assert!(loaded
            .workspace_changes
            .iter()
            .any(|c| matches!(c, WorkspaceChange::Modified { path, hash } if path == "src/main.rs" && *hash == h_mod)));
        assert!(loaded
            .workspace_changes
            .iter()
            .any(|c| matches!(c, WorkspaceChange::Untracked { path, hash } if path == "scratch/notes.md" && *hash == h_unt)));
        assert!(
            loaded
                .workspace_changes
                .iter()
                .any(|c| matches!(c, WorkspaceChange::Deleted { path } if path == ".gitignore"))
        );
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
            workspace_changes: Vec::new(),
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
        mgr.remove_shelf("nonexistent").unwrap();
    }

    #[test]
    fn list_shelves() {
        let (_dir, mgr) = setup();

        for name in &["main", "feature", "hotfix"] {
            let shelf = Shelf {
                timeline: name.to_string(),
                staged_files: BTreeMap::new(),
                workspace_changes: Vec::new(),
                created_at: 0,
            };
            mgr.save_shelf(&shelf).unwrap();
        }

        let list = mgr.list_shelves().unwrap();
        assert_eq!(list, vec!["feature", "hotfix", "main"]);
    }

    #[test]
    fn has_shelf() {
        let (_dir, mgr) = setup();
        assert!(!mgr.has_shelf("feature"));

        let shelf = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::new(),
            workspace_changes: Vec::new(),
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
            workspace_changes: Vec::new(),
            created_at: 0,
        };
        mgr.save_shelf(&shelf).unwrap();

        let loaded = mgr.load_shelf("empty").unwrap().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn overwrite_shelf() {
        let (_dir, mgr) = setup();

        let shelf1 = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::from([("old.txt".into(), B3Hash::digest(b"old"))]),
            workspace_changes: Vec::new(),
            created_at: 100,
        };
        mgr.save_shelf(&shelf1).unwrap();

        let shelf2 = Shelf {
            timeline: "feature".into(),
            staged_files: BTreeMap::from([("new.txt".into(), B3Hash::digest(b"new"))]),
            workspace_changes: Vec::new(),
            created_at: 200,
        };
        mgr.save_shelf(&shelf2).unwrap();

        let loaded = mgr.load_shelf("feature").unwrap().unwrap();
        assert_eq!(loaded.created_at, 200);
        assert!(loaded.staged_files.contains_key("new.txt"));
        assert!(!loaded.staged_files.contains_key("old.txt"));
    }
}
