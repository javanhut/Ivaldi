//! Fuse (merge) engine for Ivaldi VCS.
//!
//! Implements three-way merge with chunk-level intelligence:
//! - Auto-resolves non-conflicting changes using BLAKE3 hashes
//! - Detects identical changes on both sides automatically
//! - Multiple strategies: auto, ours, theirs, union, base
//! - Cleanly merged files never get markers; only the files that genuinely
//!   conflict are written back with `<<<<<<< / ======= / >>>>>>>` regions
//!   (see [`write_conflict_markers`]) so they can be resolved in an editor
//!   and finished with `ivaldi fuse --continue`
//! - Only truly conflicting files require user intervention

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::fsmerkle::{FsMerkleError, FsStore};
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

impl std::str::FromStr for Strategy {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "ours" => Ok(Self::Ours),
            "theirs" => Ok(Self::Theirs),
            "union" => Ok(Self::Union),
            "base" => Ok(Self::Base),
            _ => Err(()),
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
    /// - `store`: blob store, used by the `Union` strategy to materialize
    ///   concatenated blobs for genuine conflicts
    /// - `base`: common ancestor file set
    /// - `ours`: target timeline (left) file set
    /// - `theirs`: source timeline (right) file set
    pub fn fuse(
        store: &FsStore<'_>,
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
                        // Auto never concatenates; it surfaces conflicts instead.
                        MergeDecision::Concat(..) => {
                            unreachable!("auto strategy does not produce Concat decisions")
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
                        MergeDecision::Concat(o_h, t_h) => {
                            // Genuine conflict: combine both versions (ours then
                            // theirs) into a single blob. Fall back to theirs
                            // only if the blobs can't be read (CAS corruption).
                            let hash = concat_blobs(store, &o_h, &t_h).unwrap_or(t_h);
                            merged.insert(path.to_string(), hash);
                        }
                        MergeDecision::Conflict => {
                            // Union shouldn't produce bare conflicts — prefer theirs.
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
    /// Combine both versions: concatenate ours (first) then theirs (second)
    /// into a new blob. Used by the union strategy on genuine conflicts.
    Concat(B3Hash, B3Hash),
    /// Conflict — cannot auto-resolve.
    Conflict,
}

/// Concatenate two blobs (ours first, then theirs, no separator) into a new
/// blob and return its hash. Deterministic and order-fixed so the result is
/// reproducible. Shared by the union strategy and the TUI "Keep BOTH" resolver.
pub(crate) fn concat_blobs(
    store: &FsStore<'_>,
    ours: &B3Hash,
    theirs: &B3Hash,
) -> Result<B3Hash, FsMerkleError> {
    let (_, mut combined) = store.load_blob(*ours)?;
    let (_, theirs_bytes) = store.load_blob(*theirs)?;
    combined.extend_from_slice(&theirs_bytes);
    Ok(store.put_blob(&combined)?.0)
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
                MergeDecision::Concat(o, t) // Both added differently: combine
            }
        }
        (Some(_), None, None) => MergeDecision::Delete,
        (Some(_), Some(&o), None) => MergeDecision::Take(o), // Keep modified
        (Some(_), None, Some(&t)) => MergeDecision::Take(t), // Keep modified
        (Some(&b), Some(&o), Some(&t)) => {
            if o == t {
                MergeDecision::Take(o)
            } else if b == o {
                MergeDecision::Take(t) // Only theirs changed
            } else if b == t {
                MergeDecision::Take(o) // Only ours changed
            } else {
                MergeDecision::Concat(o, t) // Both changed differently: combine
            }
        }
    }
}

// -- Conflict markers -------------------------------------------------------

/// Marker prefix opening the "ours" side of a conflict region.
pub const MARKER_OURS: &str = "<<<<<<<";
/// Marker separating the two sides of a conflict region.
pub const MARKER_SEP: &str = "=======";
/// Marker closing the "theirs" side of a conflict region.
pub const MARKER_THEIRS: &str = ">>>>>>>";

/// True if `content` still holds an unresolved conflict region.
///
/// Used by `fuse --continue` to refuse committing a file the user hasn't
/// finished editing. Only line-leading markers count, so a string literal
/// containing `=======` mid-line doesn't trip it.
pub fn has_conflict_markers(content: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(content) else {
        return false;
    };
    let mut ours = false;
    let mut theirs = false;
    for line in text.lines() {
        if line.starts_with(MARKER_OURS) {
            ours = true;
        } else if line.starts_with(MARKER_THEIRS) {
            theirs = true;
        }
    }
    ours && theirs
}

/// Write every conflicted file back to the workspace with its two sides
/// wrapped in conflict markers, so the user can resolve them in an editor.
///
/// Returns `(marked, skipped)`: paths written with markers, and paths left
/// untouched because at least one side is binary (nothing sensible to
/// interleave — resolve those by choosing a side).
pub fn write_conflict_markers(
    store: &FsStore<'_>,
    work_dir: &Path,
    conflicts: &[Conflict],
    ours_label: &str,
    theirs_label: &str,
) -> (Vec<String>, Vec<String>) {
    let mut marked = Vec::new();
    let mut skipped = Vec::new();

    for c in conflicts {
        // A missing side is a delete/modify conflict: treat it as empty so the
        // marker block shows the deletion as one of the two choices.
        let load = |h: Option<B3Hash>| -> Option<Vec<u8>> {
            match h {
                Some(h) => store.load_blob(h).ok().map(|(_, bytes)| bytes),
                None => Some(Vec::new()),
            }
        };
        let (Some(base), Some(ours), Some(theirs)) = (load(c.base), load(c.ours), load(c.theirs))
        else {
            skipped.push(c.path.clone());
            continue;
        };
        if crate::diff::is_binary(&ours) || crate::diff::is_binary(&theirs) {
            skipped.push(c.path.clone());
            continue;
        }
        let (base, ours, theirs) = (
            String::from_utf8_lossy(&base),
            String::from_utf8_lossy(&ours),
            String::from_utf8_lossy(&theirs),
        );
        let merged = merge3(&base, &ours, &theirs, ours_label, theirs_label);

        let path = work_dir.join(&c.path);
        let wrote = match path.parent() {
            Some(parent) => std::fs::create_dir_all(parent)
                .and_then(|()| crate::atomic_io::atomic_write(&path, merged.as_bytes())),
            None => crate::atomic_io::atomic_write(&path, merged.as_bytes()),
        };
        if wrote.is_ok() {
            marked.push(c.path.clone());
        } else {
            skipped.push(c.path.clone());
        }
    }

    (marked, skipped)
}

/// One side's edit against the merge base: base lines `start..end` are
/// replaced by `lines`. An empty range is a pure insertion.
#[derive(Debug)]
struct Edit {
    start: usize,
    end: usize,
    lines: Vec<String>,
}

/// Diff3-style line merge of a single file.
///
/// Regions only one side touched are taken from that side; regions both sides
/// touched are wrapped in conflict markers. This is what makes a conflicted
/// fuse resolvable in an editor instead of an all-or-nothing choice between
/// two whole file versions.
fn merge3(base: &str, ours: &str, theirs: &str, ours_label: &str, theirs_label: &str) -> String {
    let base_lines: Vec<&str> = base.lines().collect();

    // Both sides' edits in one stream, ordered by the base lines they touch.
    let mut all: Vec<(u8, Edit)> = base_edits(base, ours)
        .into_iter()
        .map(|e| (0u8, e))
        .chain(base_edits(base, theirs).into_iter().map(|e| (1u8, e)))
        .collect();
    all.sort_by_key(|(side, e)| (e.start, e.end, *side));

    // Group the edits into regions of base lines: overlapping or directly
    // adjacent edits belong to the same region. Independent changes a line or
    // more apart stay separate regions and merge cleanly.
    let mut regions: Vec<(usize, usize, usize, usize)> = Vec::new(); // start, end, k, m
    let mut k = 0usize;
    while k < all.len() {
        let start = all[k].1.start;
        let mut end = all[k].1.end;
        let mut m = k + 1;
        while m < all.len() && all[m].1.start <= end {
            end = end.max(all[m].1.end);
            m += 1;
        }
        regions.push((start, end, k, m));
        k = m;
    }

    let is_conflict = |&(_, _, k, m): &(usize, usize, usize, usize)| {
        all[k..m].iter().any(|(s, _)| *s == 0) && all[k..m].iter().any(|(s, _)| *s == 1)
    };

    // Two sides rewriting the same function produce edits that alternate every
    // few lines. Applying those independently splices both rewrites together
    // into code that was never on either side, so a conflict swallows anything
    // within a marker's distance of it — the result is one honest conflict
    // instead of a plausible-looking merge nobody wrote.
    const SETTLE: usize = 7;
    let mut i = 0;
    while i + 1 < regions.len() {
        let gap = regions[i + 1].0.saturating_sub(regions[i].1);
        if gap < SETTLE && (is_conflict(&regions[i]) || is_conflict(&regions[i + 1])) {
            regions[i].1 = regions[i + 1].1;
            regions[i].3 = regions[i + 1].3;
            regions.remove(i + 1);
            continue; // re-test the widened region against what follows
        }
        i += 1;
    }

    let mut out: Vec<String> = Vec::new();
    let mut b = 0usize;
    for region in &regions {
        let (start, end, k, m) = *region;
        while b < start {
            out.push(base_lines[b].to_string());
            b += 1;
        }

        let edits = &all[k..m];
        let ours_side = || edits.iter().filter(|(s, _)| *s == 0).map(|(_, e)| e);
        let theirs_side = || edits.iter().filter(|(s, _)| *s == 1).map(|(_, e)| e);

        if is_conflict(region) {
            out.push(format!("{MARKER_OURS} {ours_label}"));
            out.extend(rebuild(&base_lines, start, end, ours_side()));
            out.push(MARKER_SEP.to_string());
            out.extend(rebuild(&base_lines, start, end, theirs_side()));
            out.push(format!("{MARKER_THEIRS} {theirs_label}"));
        } else {
            out.extend(rebuild(
                &base_lines,
                start,
                end,
                edits.iter().map(|(_, e)| e),
            ));
        }

        b = end;
    }
    while b < base_lines.len() {
        out.push(base_lines[b].to_string());
        b += 1;
    }

    let mut text = out.join("\n");
    // `lines()` drops the terminator; restore it if either side had one.
    if (ours.ends_with('\n') || theirs.ends_with('\n')) && !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Rewrite base lines `start..end` as one side sees them, applying that side's
/// edits and passing untouched base lines through.
fn rebuild<'a>(
    base_lines: &[&str],
    start: usize,
    end: usize,
    edits: impl Iterator<Item = &'a Edit>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut b = start;
    for e in edits {
        while b < e.start {
            out.push(base_lines[b].to_string());
            b += 1;
        }
        out.extend(e.lines.iter().cloned());
        b = b.max(e.end);
    }
    while b < end {
        out.push(base_lines[b].to_string());
        b += 1;
    }
    out
}

/// Convert a line diff of `base` → `side` into replace-this-base-range edits.
fn base_edits(base: &str, side: &str) -> Vec<Edit> {
    use crate::diff::LineOp;

    let mut edits = Vec::new();
    let mut cur: Option<Edit> = None;
    let mut b = 0usize;
    for op in crate::diff::compute_ops(base, side) {
        match op {
            LineOp::Context(_) => {
                if let Some(e) = cur.take() {
                    edits.push(e);
                }
                b += 1;
            }
            LineOp::Remove(_) => {
                let e = cur.get_or_insert(Edit {
                    start: b,
                    end: b,
                    lines: Vec::new(),
                });
                e.end = b + 1;
                b += 1;
            }
            LineOp::Add(line) => {
                cur.get_or_insert(Edit {
                    start: b,
                    end: b,
                    lines: Vec::new(),
                })
                .lines
                .push(line);
            }
        }
    }
    if let Some(e) = cur {
        edits.push(e);
    }
    edits
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

    fn tmp_cas() -> (tempfile::TempDir, crate::cas::FileCas) {
        let dir = tempfile::tempdir().unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join("objects")).unwrap();
        (dir, cas)
    }

    /// Run a fuse against a throwaway store. Adequate for strategies that never
    /// load blob content (auto/ours/theirs/base, and union without conflicts).
    fn fuse_t(
        base: &BTreeMap<String, B3Hash>,
        ours: &BTreeMap<String, B3Hash>,
        theirs: &BTreeMap<String, B3Hash>,
        strategy: Strategy,
    ) -> FuseResult {
        let (_dir, cas) = tmp_cas();
        let store = FsStore::new(&cas);
        FuseEngine::fuse(&store, base, ours, theirs, strategy)
    }

    // ---- Auto strategy ----

    #[test]
    fn auto_identical_trees() {
        let base = files(&[("a.txt", "hello")]);
        let ours = files(&[("a.txt", "hello")]);
        let theirs = files(&[("a.txt", "hello")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 1);
        assert_eq!(result.merged_files["a.txt"], hash("hello"));
    }

    #[test]
    fn auto_only_ours_changed() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "modified")]);
        let theirs = files(&[("a.txt", "base")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    #[test]
    fn auto_only_theirs_changed() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "base")]);
        let theirs = files(&[("a.txt", "modified")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    #[test]
    fn auto_both_same_change() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "same")]);
        let theirs = files(&[("a.txt", "same")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("same"));
    }

    #[test]
    fn auto_both_different_changes_conflict() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(!result.success);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, "a.txt");
    }

    #[test]
    fn auto_added_on_left_only() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "content")]);
        let theirs = files(&[]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["new.txt"], hash("content"));
    }

    #[test]
    fn auto_added_on_right_only() {
        let base = files(&[]);
        let ours = files(&[]);
        let theirs = files(&[("new.txt", "content")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files["new.txt"], hash("content"));
    }

    #[test]
    fn auto_added_both_same() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "same")]);
        let theirs = files(&[("new.txt", "same")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 1);
    }

    #[test]
    fn auto_added_both_different_conflict() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "our version")]);
        let theirs = files(&[("new.txt", "their version")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(!result.success);
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn auto_deleted_both_sides() {
        let base = files(&[("old.txt", "content")]);
        let ours = files(&[]);
        let theirs = files(&[]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
    }

    #[test]
    fn auto_deleted_left_unchanged_right() {
        let base = files(&[("a.txt", "content")]);
        let ours = files(&[]); // deleted
        let theirs = files(&[("a.txt", "content")]); // unchanged

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty()); // accept deletion
    }

    #[test]
    fn auto_deleted_left_modified_right_conflict() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[]); // deleted
        let theirs = files(&[("a.txt", "modified")]); // modified

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
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

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
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

        let result = fuse_t(&base, &ours, &theirs, Strategy::Ours);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("our version"));
    }

    #[test]
    fn ours_deletes_missing_from_ours() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[]);
        let theirs = files(&[("a.txt", "modified")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Ours);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
    }

    // ---- Theirs strategy ----

    #[test]
    fn theirs_always_takes_theirs() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "our version")]);
        let theirs = files(&[("a.txt", "their version")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Theirs);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("their version"));
    }

    // ---- Union strategy ----

    /// Build a `path -> hash` map by storing each content in `store`, so the
    /// union strategy can actually load the blobs to concatenate them.
    fn stored(store: &FsStore<'_>, entries: &[(&str, &[u8])]) -> BTreeMap<String, B3Hash> {
        entries
            .iter()
            .map(|(path, content)| {
                let (h, _) = store.put_blob(content).unwrap();
                (path.to_string(), h)
            })
            .collect()
    }

    #[test]
    fn union_concatenates_all_three_differ() {
        let (_dir, cas) = tmp_cas();
        let store = FsStore::new(&cas);
        let base = stored(&store, &[("a.txt", b"BASE")]);
        let ours = stored(&store, &[("a.txt", b"AAA")]);
        let theirs = stored(&store, &[("a.txt", b"BBB")]);

        let result = FuseEngine::fuse(&store, &base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        let (_, bytes) = store.load_blob(result.merged_files["a.txt"]).unwrap();
        assert_eq!(bytes, b"AAABBB", "ours then theirs, no separator");
    }

    #[test]
    fn union_concatenates_both_added_no_base() {
        let (_dir, cas) = tmp_cas();
        let store = FsStore::new(&cas);
        let base = BTreeMap::new();
        let ours = stored(&store, &[("a.txt", b"AAA")]);
        let theirs = stored(&store, &[("a.txt", b"BBB")]);

        let result = FuseEngine::fuse(&store, &base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        let (_, bytes) = store.load_blob(result.merged_files["a.txt"]).unwrap();
        assert_eq!(bytes, b"AAABBB");
    }

    #[test]
    fn union_clean_resolves_do_not_concat() {
        let (_dir, cas) = tmp_cas();
        let store = FsStore::new(&cas);
        let base = stored(
            &store,
            &[("only_ours.txt", b"O0"), ("only_theirs.txt", b"T0")],
        );
        // only_ours changed on our side; only_theirs changed on theirs.
        let ours = stored(
            &store,
            &[("only_ours.txt", b"O1"), ("only_theirs.txt", b"T0")],
        );
        let theirs = stored(
            &store,
            &[("only_ours.txt", b"O0"), ("only_theirs.txt", b"T1")],
        );

        let result = FuseEngine::fuse(&store, &base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        // Single-sided changes take the changed version verbatim, NOT a concat.
        let (_, a) = store
            .load_blob(result.merged_files["only_ours.txt"])
            .unwrap();
        let (_, b) = store
            .load_blob(result.merged_files["only_theirs.txt"])
            .unwrap();
        assert_eq!(a, b"O1");
        assert_eq!(b, b"T1");
    }

    #[test]
    fn union_concat_order_is_ours_then_theirs() {
        let (_dir, cas) = tmp_cas();
        let store = FsStore::new(&cas);
        let base = stored(&store, &[("a.txt", b"x")]);
        let ours = stored(&store, &[("a.txt", b"<<OURS>>")]);
        let theirs = stored(&store, &[("a.txt", b"<<THEIRS>>")]);

        let result = FuseEngine::fuse(&store, &base, &ours, &theirs, Strategy::Union);
        let (_, bytes) = store.load_blob(result.merged_files["a.txt"]).unwrap();
        assert_eq!(bytes, b"<<OURS>><<THEIRS>>");
    }

    #[test]
    fn union_keeps_modified_over_deleted() {
        let base = files(&[("a.txt", "base")]);
        let ours = files(&[("a.txt", "modified")]);
        let theirs = files(&[]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Union);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("modified"));
    }

    // ---- Base strategy ----

    #[test]
    fn base_reverts_to_ancestor() {
        let base = files(&[("a.txt", "original")]);
        let ours = files(&[("a.txt", "our change")]);
        let theirs = files(&[("a.txt", "their change")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Base);
        assert!(result.success);
        assert_eq!(result.merged_files["a.txt"], hash("original"));
    }

    #[test]
    fn base_drops_files_not_in_base() {
        let base = files(&[]);
        let ours = files(&[("new.txt", "added")]);
        let theirs = files(&[("other.txt", "also added")]);

        let result = fuse_t(&base, &ours, &theirs, Strategy::Base);
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
        assert_eq!("auto".parse::<Strategy>().ok(), Some(Strategy::Auto));
        assert_eq!("ours".parse::<Strategy>().ok(), Some(Strategy::Ours));
        assert_eq!("theirs".parse::<Strategy>().ok(), Some(Strategy::Theirs));
        assert_eq!("union".parse::<Strategy>().ok(), Some(Strategy::Union));
        assert_eq!("base".parse::<Strategy>().ok(), Some(Strategy::Base));
        assert_eq!("invalid".parse::<Strategy>().ok(), None);
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
        let result = fuse_t(&empty, &empty, &empty, Strategy::Auto);
        assert!(result.success);
        assert!(result.merged_files.is_empty());
        assert!(result.conflicts.is_empty());
    }

    // ---- diff3 conflict markers ----

    #[test]
    fn merge3_takes_each_side_when_regions_do_not_overlap() {
        let base = "a\nb\nc\nd\ne\n";
        let ours = "a\nOURS\nc\nd\ne\n";
        let theirs = "a\nb\nc\nTHEIRS\ne\n";

        let merged = merge3(base, ours, theirs, "main", "remote");
        assert_eq!(merged, "a\nOURS\nc\nTHEIRS\ne\n");
        assert!(!has_conflict_markers(merged.as_bytes()));
    }

    #[test]
    fn merge3_marks_the_region_both_sides_changed() {
        let base = "a\nb\nc\n";
        let ours = "a\nOURS\nc\n";
        let theirs = "a\nTHEIRS\nc\n";

        let merged = merge3(base, ours, theirs, "main", "remote");
        assert_eq!(
            merged,
            "a\n<<<<<<< main\nOURS\n=======\nTHEIRS\n>>>>>>> remote\nc\n"
        );
        assert!(has_conflict_markers(merged.as_bytes()));
    }

    #[test]
    fn merge3_keeps_a_distant_clean_hunk_outside_the_conflict() {
        // Both sides rewrite line 2; theirs also appends, well clear of it.
        let base = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let ours = "a\nOURS\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let theirs = "a\nTHEIRS\nc\nd\ne\nf\ng\nh\ni\nj\nappended\n";

        let merged = merge3(base, ours, theirs, "main", "remote");
        assert!(merged.ends_with("appended\n"), "{merged}");
        assert_eq!(merged.matches(MARKER_SEP).count(), 1, "{merged}");
    }

    #[test]
    fn merge3_does_not_splice_two_rewrites_of_the_same_region() {
        // Both sides restructured the same block in different ways. Applying
        // the edits independently would interleave them into code neither
        // side wrote, so the whole run has to come out as one conflict.
        let base = "fn f() {\n    one();\n    two();\n    three();\n}\n";
        let ours = "fn f() {\n    one();\n    OURS_A();\n    three();\n    OURS_B();\n}\n";
        let theirs = "fn f() {\n    THEIRS_A();\n    two();\n    THEIRS_B();\n}\n";

        let merged = merge3(base, ours, theirs, "main", "remote");
        assert_eq!(merged.matches(MARKER_SEP).count(), 1, "{merged}");
        // Every marked line belongs to exactly one side of the one conflict.
        let (ours_part, theirs_part) = merged.split_once(MARKER_SEP).unwrap();
        assert!(
            ours_part.contains("OURS_A") && ours_part.contains("OURS_B"),
            "{merged}"
        );
        assert!(!ours_part.contains("THEIRS_"), "{merged}");
        assert!(
            theirs_part.contains("THEIRS_A") && theirs_part.contains("THEIRS_B"),
            "{merged}"
        );
        assert!(!theirs_part.contains("OURS_"), "{merged}");
    }

    #[test]
    fn merge3_handles_a_side_deleting_the_file() {
        let base = "a\nb\n";
        let merged = merge3(base, "", "a\nCHANGED\n", "main", "remote");
        assert!(has_conflict_markers(merged.as_bytes()), "{merged}");
    }

    #[test]
    fn has_conflict_markers_ignores_lone_or_mid_line_markers() {
        assert!(!has_conflict_markers(b"let sep = \"=======\";\n"));
        assert!(!has_conflict_markers(b"<<<<<<< main\nonly one side\n"));
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

        let result = fuse_t(&base, &ours, &theirs, Strategy::Auto);
        assert!(result.success);
        assert_eq!(result.merged_files.len(), 100);
    }
}
