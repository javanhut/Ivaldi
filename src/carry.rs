//! Carrying uncommitted work across a fuse.
//!
//! A fuse merges *seals*. Work that is not sealed yet has no side in that
//! merge, so the fused tree, written over the working directory, would erase
//! it. Refusing to fuse until the user seals is safe but pushes a chore onto
//! them for something they plainly meant: "bring that timeline in, underneath
//! what I am doing".
//!
//! So the fuse does it for them, in three moves:
//!
//! 1. **Set aside** — the uncommitted state is captured as a
//!    [snapshot](crate::snapshot) and the working directory is put back to the
//!    timeline's tip, leaving the fuse a clean tree to work on (including, if
//!    it conflicts, to write conflict markers into and `--continue` from).
//! 2. **Fuse** — exactly as on a clean tree.
//! 3. **Re-apply** — [`reapply`] merges each set-aside change onto the fused
//!    tree. Every change was an edit *of the old tip*, so the old tip is the
//!    merge base, the user's version is one side, and the fused file is the
//!    other. Changes the fuse did not touch drop straight back in; ones it did
//!    are merged line by line, with conflict markers only where both really
//!    collide.
//!
//! The result is left uncommitted, as it was. Nothing here can lose work: the
//! captured content is in the CAS before step 1 touches a file, the same
//! snapshot is what `ivaldi oops` restores, and re-applying never overwrites
//! a user's version without either merging it or saying where it still is.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::hash::B3Hash;
use crate::repo::Repo;
use crate::shelf::WorkspaceChange;
use crate::snapshot::{Snapshot, SnapshotError};
use crate::workspace::Workspace;

/// What became of one set-aside change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Back in the working directory with nothing to look at: either the fuse
    /// never touched the file, or both sets of edits merged without colliding.
    Clean,
    /// The fuse already contains exactly this change; there is nothing left
    /// of it to carry.
    AlreadyFused,
    /// Both sides changed the same lines, and a resolver said which to keep.
    Settled,
    /// Both sides changed the same lines and there was nobody to ask (or they
    /// declined). The file holds conflict markers — harmless here, since it is
    /// an uncommitted working file either way, with no merge left open.
    Conflict,
    /// A binary file changed on both sides. The fused version is on disk; the
    /// user's is still in the snapshot.
    BinaryKeptFused,
    /// The user changed a file the fuse deleted. Their version is kept, and
    /// now shows as a new file.
    KeptDeletedByFuse,
    /// The user deleted a file the fuse changed. The fused version is kept.
    KeptChangedByFuse,
}

impl Outcome {
    /// Whether the user should open this file before sealing.
    pub fn needs_attention(&self) -> bool {
        !matches!(
            self,
            Outcome::Clean | Outcome::Settled | Outcome::AlreadyFused
        )
    }
}

/// Per-path results of [`reapply`], in path order.
#[derive(Debug, Default)]
pub struct CarryReport {
    pub outcomes: Vec<(String, Outcome)>,
    /// Gathered entries that were dropped. They were gathered against the old
    /// tip; kept, the next seal would write those exact blobs over whatever
    /// the fuse brought in for the same paths.
    pub ungathered: usize,
}

impl CarryReport {
    pub fn clean_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| !o.needs_attention())
            .count()
    }

    pub fn attention(&self) -> impl Iterator<Item = &(String, Outcome)> {
        self.outcomes.iter().filter(|(_, o)| o.needs_attention())
    }
}

/// Merge the changes in `carry` onto the timeline's current head.
///
/// Precondition: the working directory matches the current head's tree — the
/// caller has just materialized it. `carry.head` is the tip the changes were
/// made against. `theirs_label` names the fused side in conflict markers.
///
/// Idempotent given that precondition, so an interrupted re-apply is repaired
/// by materializing the head again and calling this again.
pub fn reapply(
    repo: &Repo,
    cas: &FileCas,
    carry: &Snapshot,
    theirs_label: &str,
    mut resolver: Option<&mut (dyn crate::resolve::Resolver + 'static)>,
) -> Result<CarryReport, SnapshotError> {
    const MINE_LABEL: &str = "your uncommitted changes";

    let ws = Workspace::new(cas, &repo.work_dir, &repo.ivaldi_dir);
    let store = FsStore::new(cas);

    let tree_at = |idx: Option<u64>| -> Result<BTreeMap<String, B3Hash>, SnapshotError> {
        match idx {
            Some(idx) => {
                let leaf = repo.get_leaf(idx)?.ok_or(SnapshotError::MissingHead(idx))?;
                Ok(ws.list_tree_files(leaf.tree_root)?)
            }
            None => Ok(BTreeMap::new()),
        }
    };
    let old = tree_at(carry.head)?;
    let new = tree_at(repo.get_timeline_head(&carry.timeline)?)?;

    let load = |hash: B3Hash| -> Result<Vec<u8>, SnapshotError> {
        Ok(store
            .load_blob(hash)
            .map_err(crate::workspace::WorkspaceError::FsMerkle)?
            .1)
    };

    let mut report = CarryReport {
        ungathered: carry.staged_files.len() + carry.staged_deletions.len(),
        ..CarryReport::default()
    };

    for change in &carry.workspace_changes {
        let (path, outcome) = match change {
            WorkspaceChange::Modified { path, hash }
            | WorkspaceChange::Untracked { path, hash } => {
                let full = repo.work_dir.join(path);
                let outcome = match (old.get(path), new.get(path)) {
                    (_, Some(fused)) if fused == hash => Outcome::AlreadyFused,
                    // The fuse left this path exactly as the user found it
                    // (unchanged, or absent before and after).
                    (base, fused) if base == fused => {
                        write(&full, &load(*hash)?)?;
                        Outcome::Clean
                    }
                    (Some(_), None) => {
                        write(&full, &load(*hash)?)?;
                        Outcome::KeptDeletedByFuse
                    }
                    (base, Some(fused)) => {
                        let base = match base {
                            Some(h) => load(*h)?,
                            None => Vec::new(),
                        };
                        let mine = load(*hash)?;
                        let fused = load(*fused)?;
                        if crate::diff::is_binary(&mine) || crate::diff::is_binary(&fused) {
                            Outcome::BinaryKeptFused
                        } else {
                            let merge = crate::fuse::Merge3::new(
                                &String::from_utf8_lossy(&base),
                                &String::from_utf8_lossy(&mine),
                                &String::from_utf8_lossy(&fused),
                            );
                            // The fuse is sealed by now, so declining to answer
                            // cannot undo anything: it just means markers, for
                            // this collision and every later one.
                            let resolutions = match resolver.as_deref_mut() {
                                Some(r) if merge.collisions() > 0 => {
                                    let labels = crate::resolve::Labels {
                                        mine: MINE_LABEL,
                                        theirs: theirs_label,
                                        on_quit: "the fuse is sealed; leaves conflict \
                                                  markers in this file instead",
                                    };
                                    let answers =
                                        crate::resolve::resolve_regions(&merge, path, labels, r);
                                    if answers.is_err() {
                                        resolver = None;
                                    }
                                    answers.ok()
                                }
                                _ => None,
                            };
                            let text = merge.render(
                                resolutions.as_deref().unwrap_or(&[]),
                                MINE_LABEL,
                                theirs_label,
                            );
                            write(&full, text.as_bytes())?;
                            match (merge.collisions(), resolutions) {
                                (0, _) => Outcome::Clean,
                                (_, Some(_)) => Outcome::Settled,
                                (_, None) => Outcome::Conflict,
                            }
                        }
                    }
                    // `base == fused` above already covers (None, None).
                    (None, None) => unreachable!("absent before and after is base == fused"),
                };
                (path, outcome)
            }
            WorkspaceChange::Deleted { path } => {
                let outcome = match (old.get(path), new.get(path)) {
                    (_, None) => Outcome::AlreadyFused,
                    (base, fused) if base == fused => {
                        let _ = fs::remove_file(repo.work_dir.join(path));
                        Outcome::Clean
                    }
                    _ => Outcome::KeptChangedByFuse,
                };
                (path, outcome)
            }
        };
        report.outcomes.push((path.clone(), outcome));
    }

    report.outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(report)
}

fn write(path: &Path, content: &[u8]) -> Result<(), SnapshotError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::atomic_io::atomic_write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A repository whose head moved from `old` to `fused` — all `reapply`
    /// looks at — with the working directory materialized at `fused`.
    /// Returns the repo and the index of the `old` seal.
    fn moved(dir: &Path, old: &[(&str, &[u8])], fused: &[(&str, &[u8])]) -> (Repo, u64) {
        crate::forge::forge(dir).unwrap();
        let mut repo = Repo::open(dir).unwrap();
        let tree = |repo: &Repo, files: &[(&str, &[u8])]| {
            let files: BTreeMap<String, Vec<u8>> = files
                .iter()
                .map(|(path, content)| (path.to_string(), content.to_vec()))
                .collect();
            FsStore::new(&repo.cas).build_tree_from_map(&files).unwrap()
        };

        let old_tree = tree(&repo, old);
        repo.commit(old_tree, "author", "old tip").unwrap();
        let old_idx = repo.get_timeline_head("main").unwrap().unwrap();

        let fused_tree = tree(&repo, fused);
        repo.commit(fused_tree, "author", "fused").unwrap();
        Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir)
            .materialize(fused_tree)
            .unwrap();
        (repo, old_idx)
    }

    fn carry(head: u64, changes: Vec<WorkspaceChange>) -> Snapshot {
        Snapshot {
            id: 0,
            command: "fuse other".into(),
            timeline: "main".into(),
            head: Some(head),
            created_at: 0,
            staged_files: BTreeMap::new(),
            staged_deletions: BTreeSet::new(),
            workspace_changes: changes,
        }
    }

    fn blob(repo: &Repo, content: &[u8]) -> B3Hash {
        FsStore::new(&repo.cas).put_blob(content).unwrap().0
    }

    fn outcome_of(report: &CarryReport, path: &str) -> Outcome {
        report
            .outcomes
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, o)| o.clone())
            .unwrap()
    }

    #[test]
    fn a_change_the_fuse_already_made_is_not_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(dir.path(), &[("a", b"base\n")], &[("a", b"same\n")]);
        let mine = blob(&repo, b"same\n");
        let snapshot = carry(
            old,
            vec![WorkspaceChange::Modified {
                path: "a".into(),
                hash: mine,
            }],
        );
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "a"), Outcome::AlreadyFused);
        assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"same\n");
    }

    #[test]
    fn edit_of_a_file_the_fuse_deleted_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(
            dir.path(),
            &[("a", b"base\n"), ("keep", b"k\n")],
            &[("keep", b"k\n")],
        );
        let mine = blob(&repo, b"edited\n");
        let snapshot = carry(
            old,
            vec![WorkspaceChange::Modified {
                path: "a".into(),
                hash: mine,
            }],
        );
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "a"), Outcome::KeptDeletedByFuse);
        assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"edited\n");
    }

    #[test]
    fn deletion_of_a_file_the_fuse_changed_is_not_carried_out() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(dir.path(), &[("a", b"base\n")], &[("a", b"changed\n")]);
        let snapshot = carry(old, vec![WorkspaceChange::Deleted { path: "a".into() }]);
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "a"), Outcome::KeptChangedByFuse);
        assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"changed\n");
    }

    #[test]
    fn deletion_of_a_file_the_fuse_left_alone_is_carried_out() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(
            dir.path(),
            &[("a", b"base\n"), ("b", b"1\n")],
            &[("a", b"base\n"), ("b", b"2\n")],
        );
        let snapshot = carry(old, vec![WorkspaceChange::Deleted { path: "a".into() }]);
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "a"), Outcome::Clean);
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn binary_changed_on_both_sides_keeps_the_fused_version_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(
            dir.path(),
            &[("img", b"\x00base")],
            &[("img", b"\x00fused")],
        );
        let mine = blob(&repo, b"\x00mine");
        let snapshot = carry(
            old,
            vec![WorkspaceChange::Modified {
                path: "img".into(),
                hash: mine,
            }],
        );
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "img"), Outcome::BinaryKeptFused);
        assert!(outcome_of(&report, "img").needs_attention());
        assert_eq!(fs::read(dir.path().join("img")).unwrap(), b"\x00fused");
        // The user's bytes are not on disk, so they must still be in the CAS.
        assert_eq!(
            FsStore::new(&repo.cas).load_blob(mine).unwrap().1,
            b"\x00mine"
        );
    }

    #[test]
    fn new_file_the_fuse_also_added_differently_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, old) = moved(
            dir.path(),
            &[("k", b"k\n")],
            &[("k", b"k\n"), ("n", b"theirs\n")],
        );
        let mine = blob(&repo, b"mine\n");
        let snapshot = carry(
            old,
            vec![WorkspaceChange::Untracked {
                path: "n".into(),
                hash: mine,
            }],
        );
        let report = reapply(&repo, &repo.cas, &snapshot, "fused", None).unwrap();
        assert_eq!(outcome_of(&report, "n"), Outcome::Conflict);
        let text = fs::read_to_string(dir.path().join("n")).unwrap();
        assert!(text.contains("mine") && text.contains("theirs"), "{text}");
    }
}
