//! Fuse (merge) engine for Ivaldi VCS.
//!
//! Implements three-way merge with chunk-level intelligence:
//! - Auto-resolves non-conflicting changes using BLAKE3 hashes
//! - Detects identical changes on both sides automatically
//! - Multiple strategies: auto, ours, theirs, union, base
//! - NO conflict markers written to workspace files (workspace always clean)
//! - Only truly conflicting files require user intervention

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::hash::B3Hash;

/// Merge strategy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Intelligent three-way merge (default). Auto-resolves non-conflicting changes.
    Auto,
    /// Keep all target timeline (left/ours) versions.
    Ours,
    /// Accept all source timeline (right/theirs) versions.
    Theirs,
    /// Combine both versions (useful for append-only files).
    Union,
    /// Revert to common ancestor version.
    Base,
}

impl Strategy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "ours" => Some(Self::Ours),
            "theirs" => Some(Self::Theirs),
            "union" => Some(Self::Union),
            "base" => Some(Self::Base),
            _ => None,
        }
    }
}

impl fmt::Display for Strategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Strategy::Auto => write!(f, "auto"),
            Strategy::Ours => write!(f, "ours"),
            Strategy::Theirs => write!(f, "theirs"),
            Strategy::Union => write!(f, "union"),
            Strategy::Base => write!(f, "base"),
        }
    }
}

/// A file version identified by its content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    pub path: String,
    pub hash: B3Hash,
}

/// A conflict on a single file.
#[derive(Debug, Clone)]
pub struct Conflict {
    pub path: String,
    pub base: Option<B3Hash>,
    pub ours: Option<B3Hash>,
    pub theirs: Option<B3Hash>,
}

/// Result of a fuse (merge) operation.
#[derive(Debug)]
pub struct FuseResult {
    /// Whether the merge completed without conflicts.
    pub success: bool,
    /// Merged files: path → content hash.
    pub merged_files: BTreeMap<String, B3Hash>,
    /// Unresolved conflicts.
    pub conflicts: Vec<Conflict>,
}

/// The fuse engine performs three-way merges on file sets.
///
/// Files are represented as `BTreeMap<String, B3Hash>` (path → content hash).
/// The engine compares hashes to determine changes, avoiding false conflicts.
pub struct FuseEngine;

impl FuseEngine {
    /// Perform a three-way merge with the given strategy.
    ///
    /// - `base`: common ancestor file set
    /// - `ours`: target timeline (left) file set
    /// - `theirs`: source timeline (right) file set
    pub fn fuse(
        base: &BTreeMap<String, B3Hash>,
        ours: &BTreeMap<String, B3Hash>,
        theirs: &BTreeMap<String, B3Hash>,
        strategy: Strategy,
    ) -> FuseResult {
        // Collect all unique paths
        let all_paths: BTreeSet<&str> = base
            .keys()
            .chain(ours.keys())
            .chain(theirs.keys())
            .map(|s| s.as_str())
            .collect();

        let mut merged = BTreeMap::new();
        let mut conflicts = Vec::new();

        for path in all_paths {
            let b = base.get(path);
            let o = ours.get(path);
            let t = theirs.get(path);

            match strategy {
                Strategy::Auto => {
                    match merge_file_auto(b, o, t) {
                        MergeDecision::Take(hash) => {
                            merged.insert(path.to_string(), hash);
                        }
                        MergeDecision::Delete => {
                            // File removed — don't include
                        }
                        MergeDecision::Conflict => {
                            conflicts.push(Conflict {
                                path: path.to_string(),
                                base: b.copied(),
                                ours: o.copied(),
                                theirs: t.copied(),
                            });
                        }
                    }
                }
                Strategy::Ours => {
                    if let Some(&hash) = o {
                        merged.insert(path.to_string(), hash);
                    }
                    // If not in ours, file is deleted in our version
                }
                Strategy::Theirs => {
                    if let Some(&hash) = t {
                        merged.insert(path.to_string(), hash);
                    }
                }
                Strategy::Union => {
                    match merge_file_union(b, o, t) {
                        MergeDecision::Take(hash) => {
                            merged.insert(path.to_string(), hash);
                        }
                        MergeDecision::Delete => {}
                        MergeDecision::Conflict => {
                            // Union shouldn't produce conflicts — prefer theirs for tie-breaking
                            if let Some(&hash) = t {
                                merged.insert(path.to_string(), hash);
                            } else if let Some(&hash) = o {
                                merged.insert(path.to_string(), hash);
                            }
                        }
                    }
                }
                Strategy::Base => {
                    if let Some(&hash) = b {
                        merged.insert(path.to_string(), hash);
                    }
                }
            }
        }

        FuseResult {
            success: conflicts.is_empty(),
            merged_files: merged,
            conflicts,
        }
    }

    /// Check if a merge would be a fast-forward (target is ancestor of source).
    pub fn is_fast_forward(
        ours: &BTreeMap<String, B3Hash>,
        theirs: &BTreeMap<String, B3Hash>,
        base: &BTreeMap<String, B3Hash>,
    ) -> bool {
        // Fast-forward if base == ours (target hasn't changed since divergence)
        base == ours && ours != theirs
    }
}

/// Decision for a single file in the merge.
enum MergeDecision {
    /// Take this hash as the merged result.
    Take(B3Hash),
    /// Delete the file.
    Delete,
    /// Conflict — cannot auto-resolve.
    Conflict,
}

/// Three-way merge logic for a single file (auto strategy).
fn merge_file_auto(
    base: Option<&B3Hash>,
    ours: Option<&B3Hash>,
    theirs: Option<&B3Hash>,
) -> MergeDecision {
    match (base, ours, theirs) {
        // File doesn't exist anywhere
        (None, None, None) => MergeDecision::Delete,

        // Added on one side only
        (None, Some(&hash), None) => MergeDecision::Take(hash),
        (None, None, Some(&hash)) => MergeDecision::Take(hash),

        // Added on both sides
        (None, Some(&o), Some(&t)) => {
            if o == t {
                MergeDecision::Take(o) // Same content
            } else {
                MergeDecision::Conflict
            }
        }

        // Deleted on both sides
        (Some(_), None, None) => MergeDecision::Delete,

        // Modified on left, deleted on right
        (Some(&b), Some(&o), None) => {
            if b == o {
                MergeDecision::Delete // Unchanged on left, accept deletion
            } else {
                MergeDecision::Conflict // Modified vs deleted
            }
        }

        // Deleted on left, modified on right
        (Some(&b), None, Some(&t)) => {
            if b == t {
                MergeDecision::Delete // Unchanged on right, accept deletion
            } else {
                MergeDecision::Conflict // Deleted vs modified
            }
        }

        // Exists in all three
        (Some(&b), Some(&o), Some(&t)) => {
            if o == t {
                MergeDecision::Take(o) // Both made same change (or no change)
            } else if b == o {
                MergeDecision::Take(t) // Only theirs changed
            } else if b == t {
                MergeDecision::Take(o) // Only ours changed
            } else {
                MergeDecision::Conflict // Both changed differently
            }
        }

    }
}

/// Union merge: prefer including both sides, avoid conflicts.
fn merge_file_union(
    base: Option<&B3Hash>,
    ours: Option<&B3Hash>,
    theirs: Option<&B3Hash>,
) -> MergeDecision {
    match (base, ours, theirs) {
        (None, None, None) => MergeDecision::Delete,
        (None, Some(&hash), None) | (None, None, Some(&hash)) => MergeDecision::Take(hash),
        (None, Some(&o), Some(&t)) => {
            if o == t {
                MergeDecision::Take(o)
            } else {
                MergeDecision::Take(t) // Union prefers theirs for new files
            }
        }
        (Some(_), None, None) => MergeDecision::Delete,
        (Some(_), Some(&o), None) => MergeDecision::Take(o), // Keep modified
        (Some(_), None, Some(&t)) => MergeDecision::Take(t), // Keep modified
        (Some(&b), Some(&o), Some(&t)) => {
            if o == t {
                MergeDecision::Take(o)
            } else if b == o {
                MergeDecision::Take(t)
            } else if b == t {
                MergeDecision::Take(o)
            } else {
                MergeDecision::Take(t) // Union prefers theirs for tie-breaking
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FuseError {
    #[error("unknown strategy: {0}")]
    UnknownStrategy(String),
    #[error("merge in progress")]
    MergeInProgress,
    #[error("no merge in progress")]
    NoMergeInProgress,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &str)]) -> BTreeMap<String, B3Hash> {
        entries
            .iter()
            .map(|(path, content)| (path.to_string(), B3Hash::digest(content.as_bytes())))
            .collect()
    }

    fn hash(s: &str) -> B3Hash {
        B3Hash::digest(s.as_bytes())
    }

    // ---- Auto strategy ----

    #[test]
    fn auto_identical_trees() {
        let base = files(&[("a.txt", "hello")]);
        let ours = files(&[("a.txt", "hello")]);
        let theirs = files(&[("a.txt", "hello")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 1);
        assert_eq!(result.merged_files["a.txt"], hash("hello"));
    }

    #[test]
    fn auto_only_ours_changed() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "modified")]);
        let theirs = files(&[("a.txt", "base")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    #[test]
    fn auto_only_theirs_changed() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "base")]);
        let theirs = files(&[("a.txt", "modified")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    #[test]
    fn auto_both_same_change() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "same")]);
        let theirs = files(&[("a.txt", "same")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("same"));
    }

    #[test]
    fn auto_both_different_changes_conflict() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(!result.success);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, "a.txt");
    }

    #[test]
    fn auto_added_on_left_only() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "content")]);
        let theirs = files(&[]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["new.txt"], hash("content"));
    }

    #[test]
    fn auto_added_on_right_only() {
        let base = files(&[]);
        let ours = files(&[]);
        let theirs = files(&[("new.txt", "content")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["new.txt"], hash("content"));
    }

    #[test]
    fn auto_added_both_same() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "same")]);
        let theirs = files(&[("new.txt", "same")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 1);
    }

    #[test]
    fn auto_added_both_different_conflict() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "our version")]);
        let theirs = files(&[("new.txt", "their version")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(!result.success);
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn auto_deleted_both_sides() {
        let base = files(&[("old.txt", "content")]);
        let ours = files(&[]);
        let theirs = files(&[]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
    }

    #[test]
    fn auto_deleted_left_unchanged_right() {
        let base = files(&[("a.txt", "content")]);
        let ours = files(&[]); // deleted
        let theirs = files(&[("a.txt", "content")]); // unchanged

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty()); // accept deletion
    }

    #[test]
    fn auto_deleted_left_modified_right_conflict() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[]); // deleted
        let theirs = files(&[("a.txt", "modified")]); // modified

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(!result.success);
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn auto_multiple_files_mixed() {
        let base = files(&[
            ("keep.txt", "keep"),
            ("modify.txt", "base"),
            ("delete.txt", "gone"),
        ]);
        let ours = files(&[
            ("keep.txt", "keep"),
            ("modify.txt", "base"), // unchanged
            // delete.txt removed
        ]);
        let theirs = files(&[
            ("keep.txt", "keep"),
            ("modify.txt", "changed"), // modified
            // delete.txt removed
            ("new.txt", "added"),
        ]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 3); // keep + modify(changed) + new
        assert_eq!(result.merged_files["modify.txt"], hash("changed"));
        assert_eq!(result.merged_files["new.txt"], hash("added"));
        assert!(!result.merged_files.contains_key("delete.txt"));
    }

    // ---- Ours strategy ----

    #[test]
    fn ours_always_takes_ours() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our version")]);
        let theirs = files(&[("a.txt", "their version")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Ours);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("our version"));
    }

    #[test]
    fn ours_deletes_missing_from_ours() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[]);
        let theirs = files(&[("a.txt", "modified")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Ours);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
    }

    // ---- Theirs strategy ----

    #[test]
    fn theirs_always_takes_theirs() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our version")]);
        let theirs = files(&[("a.txt", "their version")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Theirs);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("their version"));
    }

    // ---- Union strategy ----

    #[test]
    fn union_no_conflicts() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        // Union takes theirs for tie-breaking
        assert!(result.merged_files.contains_key("a.txt"));
    }

    #[test]
    fn union_keeps_modified_over_deleted() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "modified")]);
        let theirs = files(&[]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    // ---- Base strategy ----

    #[test]
    fn base_reverts_to_ancestor() {
        let base = files(&[("a.txt", "original")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Base);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("original"));
    }

    #[test]
    fn base_drops_files_not_in_base() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "added")]);
        let theirs = files(&[("other.txt", "also added")]);

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Base);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
    }

    // ---- Fast-forward detection ----

    #[test]
    fn fast_forward_detected() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "base")]); // same as base
        let theirs = files(&[("a.txt", "advanced")]);

        assert!(FuseEngine::is_fast_forward(&ours, &theirs, &base));
    }

    #[test]
    fn no_fast_forward_when_both_changed() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        assert!(!FuseEngine::is_fast_forward(&ours, &theirs, &base));
    }

    // ---- Strategy parsing ----

    #[test]
    fn strategy_from_str() {
        assert_eq!(Strategy::from_str("auto"), Some(Strategy::Auto));
        assert_eq!(Strategy::from_str("ours"), Some(Strategy::Ours));
        assert_eq!(Strategy::from_str("theirs"), Some(Strategy::Theirs));
        assert_eq!(Strategy::from_str("union"), Some(Strategy::Union));
        assert_eq!(Strategy::from_str("base"), Some(Strategy::Base));
        assert_eq!(Strategy::from_str("invalid"), None);
    }

    #[test]
    fn strategy_display() {
        assert_eq!(format!("{}", Strategy::Auto), "auto");
        assert_eq!(format!("{}", Strategy::Theirs), "theirs");
    }

    // ---- Edge cases ----

    #[test]
    fn empty_merge() {
        let empty = BTreeMap::new();
        let result = FuseEngine::fuse(&empty, &empty, &empty, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn large_merge_no_conflicts() {
        let mut base = BTreeMap::new();
        let mut ours = BTreeMap::new();
        let mut theirs = BTreeMap::new();

        // 100 files, only a few changed
        for i in 0..100 {
            let path = format!("file_{}.txt", i);
            let content = format!("content {}", i);
            base.insert(path.clone(), hash(&content));
            ours.insert(path.clone(), hash(&content));
            theirs.insert(path.clone(), hash(&content));
        }

        // Ours changes files 0-4
        for i in 0..5 {
            let path = format!("file_{}.txt", i);
            ours.insert(path, hash(&format!("our change {}", i)));
        }

        // Theirs changes files 50-54
        for i in 50..55 {
            let path = format!("file_{}.txt", i);
            theirs.insert(path, hash(&format!("their change {}", i)));
        }

        let result = FuseEngine::fuse(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 100);
    }
}
