//! Garbage collection for Ivaldi VCS.
//!
//! Removes orphaned objects from the CAS that are no longer referenced by any
//! commit tree. This reclaims disk space after operations like shift (squash).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::B3Hash;

/// Result of a garbage collection run.
#[derive(Debug)]
pub struct GcResult {
    /// Number of objects scanned.
    pub total_objects: usize,
    /// Number of objects still referenced.
    pub live_objects: usize,
    /// Number of objects removed.
    pub removed_objects: usize,
    /// Bytes freed.
    pub bytes_freed: u64,
}

/// Collect all object hashes from the CAS objects directory.
pub fn scan_all_objects(objects_dir: &Path) -> Result<Vec<(B3Hash, PathBuf, u64)>, GcError> {
    let mut objects = Vec::new();

    let entries = fs::read_dir(objects_dir).map_err(GcError::Io)?;
    for shard_entry in entries.flatten() {
        if !shard_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let shard_name = shard_entry.file_name().to_string_lossy().to_string();
        if shard_name.len() != 2 { continue; }

        let shard_entries = fs::read_dir(shard_entry.path()).map_err(GcError::Io)?;
        for obj_entry in shard_entries.flatten() {
            let file_name = obj_entry.file_name().to_string_lossy().to_string();
            let hex = format!("{}{}", shard_name, file_name);
            if let Some(hash) = B3Hash::from_hex(&hex) {
                let size = obj_entry.metadata().map(|m| m.len()).unwrap_or(0);
                objects.push((hash, obj_entry.path(), size));
            }
        }
    }

    Ok(objects)
}

/// Run garbage collection. Removes objects not in the `live_set`.
pub fn collect_garbage(
    objects_dir: &Path,
    live_set: &BTreeSet<B3Hash>,
    dry_run: bool,
) -> Result<GcResult, GcError> {
    let all_objects = scan_all_objects(objects_dir)?;
    let total = all_objects.len();
    let mut removed = 0;
    let mut freed = 0u64;

    for (hash, path, size) in &all_objects {
        if !live_set.contains(hash) {
            if dry_run {
                removed += 1;
                freed += size;
            } else {
                if fs::remove_file(path).is_ok() {
                    removed += 1;
                    freed += size;
                }
            }
        }
    }

    // Clean up empty shard directories
    if !dry_run {
        if let Ok(entries) = fs::read_dir(objects_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let _ = fs::remove_dir(entry.path()); // only removes if empty
                }
            }
        }
    }

    Ok(GcResult {
        total_objects: total,
        live_objects: total - removed,
        removed_objects: removed,
        bytes_freed: freed,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum GcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{Cas, FileCas};

    #[test]
    fn scan_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let objects_dir = dir.path().join("objects");
        fs::create_dir_all(&objects_dir).unwrap();
        let objs = scan_all_objects(&objects_dir).unwrap();
        assert!(objs.is_empty());
    }

    #[test]
    fn gc_removes_unreferenced() {
        let dir = tempfile::tempdir().unwrap();
        let objects_dir = dir.path().join("objects");
        let cas = FileCas::new(&objects_dir).unwrap();

        // Store some objects
        let h1 = B3Hash::digest(b"live");
        let h2 = B3Hash::digest(b"dead");
        cas.put(h1, b"live").unwrap();
        cas.put(h2, b"dead").unwrap();

        // Only h1 is live
        let mut live = BTreeSet::new();
        live.insert(h1);

        let result = collect_garbage(&objects_dir, &live, false).unwrap();
        assert_eq!(result.total_objects, 2);
        assert_eq!(result.removed_objects, 1);
        assert_eq!(result.live_objects, 1);

        // h1 should still exist, h2 should be gone
        assert!(cas.has(h1).unwrap());
        assert!(!cas.has(h2).unwrap());
    }

    #[test]
    fn gc_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let objects_dir = dir.path().join("objects");
        let cas = FileCas::new(&objects_dir).unwrap();

        let h = B3Hash::digest(b"data");
        cas.put(h, b"data").unwrap();

        let result = collect_garbage(&objects_dir, &BTreeSet::new(), true).unwrap();
        assert_eq!(result.removed_objects, 1);

        // Object should still exist (dry run)
        assert!(cas.has(h).unwrap());
    }
}
