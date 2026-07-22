//! Remote API types and SHA1↔BLAKE3 hash mapping for Ivaldi VCS.
//!
//! This module provides:
//! - Data types for GitHub/GitLab API interactions
//! - SHA1 ↔ BLAKE3 bidirectional hash mapping (for remote sync only)
//! - Seal ↔ commit conversion metadata
//!
//! IMPORTANT: SHA1 is used ONLY for remote sync mapping.
//! All internal operations use BLAKE3 exclusively.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::B3Hash;

/// A remote repository's metadata.
#[derive(Debug, Clone)]
pub struct RemoteRepo {
    pub owner: String,
    pub repo: String,
    pub default_branch: String,
    pub description: String,
    pub private: bool,
}

/// A remote branch reference.
#[derive(Debug, Clone)]
pub struct RemoteBranch {
    pub name: String,
    pub sha1: String,
}

/// A remote commit (from GitHub/GitLab API).
#[derive(Debug, Clone)]
pub struct RemoteCommit {
    pub sha1: String,
    pub tree_sha1: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub parent_sha1s: Vec<String>,
}

/// A file entry from a remote tree.
#[derive(Debug, Clone)]
pub struct RemoteTreeEntry {
    pub path: String,
    pub mode: String,
    pub entry_type: String, // "blob" or "tree"
    pub sha1: String,
    pub size: Option<u64>,
}

/// Bidirectional SHA1 ↔ BLAKE3 hash mapping.
///
/// Used ONLY during remote sync operations. Never used in internal pipeline.
pub struct HashMapping {
    /// SHA1 → BLAKE3
    sha1_to_blake3: BTreeMap<String, B3Hash>,
    /// BLAKE3 → SHA1
    blake3_to_sha1: BTreeMap<B3Hash, String>,
    /// Persistence path
    map_path: PathBuf,
}

impl HashMapping {
    pub fn new(ivaldi_dir: &Path) -> Self {
        let map_path = ivaldi_dir.join("hash-map");
        let mut mapping = Self {
            sha1_to_blake3: BTreeMap::new(),
            blake3_to_sha1: BTreeMap::new(),
            map_path,
        };
        mapping.load();
        mapping
    }

    /// Map a SHA1 hash to a BLAKE3 hash.
    pub fn insert(&mut self, sha1: &str, blake3: B3Hash) {
        self.sha1_to_blake3.insert(sha1.to_string(), blake3);
        self.blake3_to_sha1.insert(blake3, sha1.to_string());
    }

    /// Look up BLAKE3 hash by SHA1.
    pub fn get_blake3(&self, sha1: &str) -> Option<B3Hash> {
        self.sha1_to_blake3.get(sha1).copied()
    }

    /// Look up SHA1 by BLAKE3 hash.
    pub fn get_sha1(&self, blake3: B3Hash) -> Option<&str> {
        self.blake3_to_sha1.get(&blake3).map(|s| s.as_str())
    }

    /// Remove a mapping by SHA1 key.
    pub fn remove_sha1(&mut self, sha1: &str) {
        if let Some(blake3) = self.sha1_to_blake3.remove(sha1) {
            self.blake3_to_sha1.remove(&blake3);
        }
    }

    /// Number of mappings.
    pub fn len(&self) -> usize {
        self.sha1_to_blake3.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sha1_to_blake3.is_empty()
    }

    /// Save mappings to disk.
    ///
    /// Uses [`atomic_write`](crate::atomic_io::atomic_write) so a crash
    /// mid-save can never leave a truncated SHA1↔BLAKE3 map behind — a torn
    /// map silently breaks duplicate detection on the next import.
    pub fn save(&self) -> Result<(), RemoteError> {
        let mut lines = Vec::with_capacity(self.sha1_to_blake3.len());
        for (sha1, blake3) in &self.sha1_to_blake3 {
            lines.push(format!("{} {}", sha1, blake3.to_hex()));
        }
        crate::atomic_io::atomic_write(&self.map_path, (lines.join("\n") + "\n").as_bytes())
            .map_err(RemoteError::Io)
    }

    /// Load mappings from disk. Malformed lines are counted and surfaced as a
    /// warning instead of being skipped silently — a damaged map means sync
    /// classification (scout/harvest skip checks) can no longer be trusted.
    fn load(&mut self) {
        let content = match fs::read_to_string(&self.map_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let mut malformed = 0usize;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((sha1, blake3_hex)) = line.split_once(' ')
                && let Some(blake3) = B3Hash::from_hex(blake3_hex)
            {
                self.sha1_to_blake3.insert(sha1.to_string(), blake3);
                self.blake3_to_sha1.insert(blake3, sha1.to_string());
            } else {
                malformed += 1;
            }
        }
        if malformed > 0 {
            crate::logging::warn(&format!(
                "{} malformed line(s) in {} — the SHA1↔BLAKE3 map may be damaged; \
                 the next sync may re-import already-synced seals",
                malformed,
                self.map_path.display()
            ));
        }
    }
}

/// Metadata for converting between Ivaldi seals and remote commits.
#[derive(Debug, Clone)]
pub struct SyncMetadata {
    /// Timeline name.
    pub timeline: String,
    /// Last synced BLAKE3 hash (Ivaldi side).
    pub local_hash: Option<B3Hash>,
    /// Last synced SHA1 hash (remote side).
    pub remote_sha1: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("API error: {status} {message}")]
    Api { status: u16, message: String },
    #[error("authentication required")]
    AuthRequired,
    #[error("rate limited, reset at {reset_at}")]
    RateLimited { reset_at: i64 },
    #[error("repository not found: {0}")]
    RepoNotFound(String),
    #[error("branch not found: {0}")]
    BranchNotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_mapping_insert_and_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut mapping = HashMapping::new(&ivaldi_dir);

        let sha1 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let blake3 = B3Hash::digest(b"content");

        mapping.insert(sha1, blake3);

        assert_eq!(mapping.get_blake3(sha1), Some(blake3));
        assert_eq!(mapping.get_sha1(blake3), Some(sha1));
        assert_eq!(mapping.len(), 1);
    }

    #[test]
    fn hash_mapping_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let sha1 = "abc123def456789012345678901234567890abcd";
        let blake3 = B3Hash::digest(b"test");

        {
            let mut mapping = HashMapping::new(&ivaldi_dir);
            mapping.insert(sha1, blake3);
            mapping.save().unwrap();
        }

        // Load in a new instance
        let mapping2 = HashMapping::new(&ivaldi_dir);
        assert_eq!(mapping2.get_blake3(sha1), Some(blake3));
        assert_eq!(mapping2.get_sha1(blake3), Some(sha1));
    }

    #[test]
    fn hash_mapping_tolerates_torn_file_and_keeps_valid_lines() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let good_sha = "abc123def456789012345678901234567890abcd";
        let good_b3 = B3Hash::digest(b"good");
        // Simulate a torn write: one valid line, one truncated, one bogus hex.
        fs::write(
            ivaldi_dir.join("hash-map"),
            format!(
                "{} {}\n{} deadbeef\nffff",
                good_sha,
                good_b3.to_hex(),
                good_sha
            ),
        )
        .unwrap();

        let mapping = HashMapping::new(&ivaldi_dir);
        assert_eq!(mapping.get_blake3(good_sha), Some(good_b3));
        assert_eq!(mapping.len(), 1);
    }

    #[test]
    fn hash_mapping_save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut mapping = HashMapping::new(&ivaldi_dir);
        mapping.insert(
            "abc123def456789012345678901234567890abcd",
            B3Hash::digest(b"x"),
        );
        mapping.save().unwrap();

        let leftovers: Vec<_> = fs::read_dir(&ivaldi_dir)
            .unwrap()
            .filter_map(|e| {
                let name = e.unwrap().file_name().to_string_lossy().into_owned();
                name.contains(".tmp.").then_some(name)
            })
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {:?}", leftovers);
    }

    #[test]
    fn hash_mapping_empty() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mapping = HashMapping::new(&ivaldi_dir);
        assert!(mapping.is_empty());
        assert_eq!(mapping.len(), 0);
        assert!(mapping.get_blake3("nonexistent").is_none());
    }

    #[test]
    fn hash_mapping_multiple_entries() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut mapping = HashMapping::new(&ivaldi_dir);

        for i in 0..10 {
            let sha1 = format!("{:040x}", i);
            let blake3 = B3Hash::digest(format!("content {}", i).as_bytes());
            mapping.insert(&sha1, blake3);
        }

        assert_eq!(mapping.len(), 10);

        // Verify all lookups
        for i in 0..10 {
            let sha1 = format!("{:040x}", i);
            let blake3 = B3Hash::digest(format!("content {}", i).as_bytes());
            assert_eq!(mapping.get_blake3(&sha1), Some(blake3));
            assert_eq!(mapping.get_sha1(blake3), Some(sha1.as_str()));
        }
    }

    #[test]
    fn hash_mapping_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut mapping = HashMapping::new(&ivaldi_dir);
        let sha1 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

        let blake3_v1 = B3Hash::digest(b"v1");
        let blake3_v2 = B3Hash::digest(b"v2");

        mapping.insert(sha1, blake3_v1);
        mapping.insert(sha1, blake3_v2);

        // Latest value wins
        assert_eq!(mapping.get_blake3(sha1), Some(blake3_v2));
    }

    #[test]
    fn remote_commit_structure() {
        let commit = RemoteCommit {
            sha1: "abc123".to_string(),
            tree_sha1: "def456".to_string(),
            message: "Initial commit".to_string(),
            author_name: "Alice".to_string(),
            author_email: "alice@test.com".to_string(),
            timestamp: 1700000000,
            parent_sha1s: vec![],
        };

        assert!(commit.parent_sha1s.is_empty());
        assert_eq!(commit.message, "Initial commit");
    }

    #[test]
    fn remote_branch_structure() {
        let branch = RemoteBranch {
            name: "main".to_string(),
            sha1: "abc123".to_string(),
        };
        assert_eq!(branch.name, "main");
    }

    #[test]
    fn remote_tree_entry() {
        let entry = RemoteTreeEntry {
            path: "src/main.rs".to_string(),
            mode: "100644".to_string(),
            entry_type: "blob".to_string(),
            sha1: "abc123".to_string(),
            size: Some(1024),
        };
        assert_eq!(entry.entry_type, "blob");
        assert_eq!(entry.size, Some(1024));
    }
}
