//! Shared three-way "apply a delta as a new seal" engine for `undo` and
//! `pluck` (cherry-pick).
//!
//! Both commands are the same operation with different inputs: run the fuse
//! engine with ours = the current head tree and a (base, theirs) pair that
//! encodes the delta to apply, then seal the merged tree as a plain
//! (non-merge) seal.
//!
//! - undo SEAL:    base = SEAL's tree,        theirs = SEAL's parent tree
//! - pluck SEAL:   base = SEAL's parent tree, theirs = SEAL's tree
//!
//! The fuse engine works at file-hash granularity (no line-level merging),
//! so any file touched by both the delta and later history conflicts. In
//! that case the operation refuses and reports the paths — nothing is
//! committed and the working tree is untouched.

use std::collections::BTreeMap;

use crate::cas::Cas;
use crate::fsmerkle::{FsStore, NodeKind};
use crate::fuse::{FuseEngine, Strategy};
use crate::hash::B3Hash;
use crate::repo::{CommitResult, Repo};

/// Outcome of a three-way apply.
#[derive(Debug)]
pub enum ApplyOutcome {
    /// A new seal was created with the merged tree.
    Applied(CommitResult),
    /// The delta touches files that later history also changed.
    Conflicts(Vec<String>),
    /// The merged tree equals the current head tree — nothing to do.
    NoChanges,
}

/// Recursively collect `path → blob hash` for every file under a tree.
pub fn collect_tree_blobs(
    store: &FsStore<'_>,
    tree_hash: B3Hash,
    prefix: &str,
    files: &mut BTreeMap<String, B3Hash>,
) -> Result<(), String> {
    let tree = store.load_tree(tree_hash).map_err(|e| e.to_string())?;
    for entry in &tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            NodeKind::Blob => {
                files.insert(path, entry.hash);
            }
            NodeKind::Tree => {
                collect_tree_blobs(store, entry.hash, &path, files)?;
            }
        }
    }
    Ok(())
}

/// File map for an optional tree root (`None` → empty map, e.g. the parent
/// of a timeline's first seal).
pub fn tree_files(
    store: &FsStore<'_>,
    tree: Option<B3Hash>,
) -> Result<BTreeMap<String, B3Hash>, String> {
    let mut files = BTreeMap::new();
    if let Some(root) = tree {
        collect_tree_blobs(store, root, "", &mut files)?;
    }
    Ok(files)
}

/// Three-way merge `(base → theirs)` onto the current head and seal the
/// result. Returns without committing on conflicts or when the merge is a
/// no-op. The caller materializes the new tree to the working directory.
pub fn three_way_seal(
    repo: &mut Repo,
    cas: &dyn Cas,
    base: &BTreeMap<String, B3Hash>,
    theirs: &BTreeMap<String, B3Hash>,
    author: &str,
    message: &str,
) -> Result<ApplyOutcome, String> {
    let store = FsStore::new(cas);

    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
    let head_tree = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten())
        .map(|l| l.tree_root);
    let ours = tree_files(&store, head_tree)?;

    let result = FuseEngine::fuse(&store, base, &ours, theirs, Strategy::Auto);
    if !result.success {
        return Ok(ApplyOutcome::Conflicts(
            result.conflicts.into_iter().map(|c| c.path).collect(),
        ));
    }
    if result.merged_files == ours {
        return Ok(ApplyOutcome::NoChanges);
    }

    let merged_tree = store
        .build_tree_from_hash_map(&result.merged_files)
        .map_err(|e| e.to_string())?;
    let commit = repo
        .commit(merged_tree, author, message)
        .map_err(|e| e.to_string())?;
    Ok(ApplyOutcome::Applied(commit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::FileCas;
    use crate::config::Config;
    use crate::forge;
    use std::path::Path;

    fn setup() -> (tempfile::TempDir, Repo, FileCas) {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();
        let mut cfg = Config::new();
        cfg.set("user.name", "Test");
        cfg.set("user.email", "t@ivaldi.dev");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();
        let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let repo = Repo::open(dir.path()).unwrap();
        (dir, repo, cas)
    }

    /// Build and commit a tree from `path → content` pairs; returns the result.
    fn seal_tree(
        repo: &mut Repo,
        cas: &FileCas,
        files: &[(&str, &str)],
        msg: &str,
    ) -> CommitResult {
        let store = FsStore::new(cas);
        let mut map = BTreeMap::new();
        for (path, content) in files {
            map.insert(path.to_string(), content.as_bytes().to_vec());
        }
        let tree = store.build_tree_from_map(&map).unwrap();
        repo.commit(tree, "Test", msg).unwrap()
    }

    fn head_files(repo: &Repo, cas: &FileCas, _work: &Path) -> BTreeMap<String, B3Hash> {
        let store = FsStore::new(cas);
        let timeline = repo.current_timeline().unwrap();
        let tree = repo
            .get_timeline_head(&timeline)
            .unwrap()
            .and_then(|idx| repo.get_leaf(idx).unwrap())
            .map(|l| l.tree_root);
        tree_files(&store, tree).unwrap()
    }

    fn leaf_tree_files(repo: &Repo, cas: &FileCas, idx: u64) -> BTreeMap<String, B3Hash> {
        let store = FsStore::new(cas);
        let leaf = repo.get_leaf(idx).unwrap().unwrap();
        tree_files(&store, Some(leaf.tree_root)).unwrap()
    }

    fn parent_tree_files(repo: &Repo, cas: &FileCas, idx: u64) -> BTreeMap<String, B3Hash> {
        let store = FsStore::new(cas);
        let leaf = repo.get_leaf(idx).unwrap().unwrap();
        let parent_tree = if leaf.has_parent() {
            repo.get_leaf(leaf.prev_idx).unwrap().map(|l| l.tree_root)
        } else {
            None
        };
        tree_files(&store, parent_tree).unwrap()
    }

    #[test]
    fn undo_middle_seal_restores_prior_content() {
        let (dir, mut repo, cas) = setup();
        seal_tree(&mut repo, &cas, &[("a.txt", "v1"), ("b.txt", "b")], "C1");
        let c2 = seal_tree(&mut repo, &cas, &[("a.txt", "v2"), ("b.txt", "b")], "C2");
        seal_tree(
            &mut repo,
            &cas,
            &[("a.txt", "v2"), ("b.txt", "b"), ("c.txt", "c")],
            "C3",
        );

        // Undo C2: base = C2's tree, theirs = C1's tree.
        let base = leaf_tree_files(&repo, &cas, c2.index);
        let theirs = parent_tree_files(&repo, &cas, c2.index);
        let outcome = three_way_seal(&mut repo, &cas, &base, &theirs, "Test", "Undo C2").unwrap();

        match outcome {
            ApplyOutcome::Applied(_) => {}
            other => panic!("expected Applied, got {:?}", other),
        }
        let files = head_files(&repo, &cas, dir.path());
        let store = FsStore::new(&cas);
        let (_, content) = store.load_blob(files["a.txt"]).unwrap();
        assert_eq!(content, b"v1");
        // C3's addition is untouched.
        assert!(files.contains_key("c.txt"));
    }

    #[test]
    fn undo_first_seal_deletes_its_files() {
        let (dir, mut repo, cas) = setup();
        let c1 = seal_tree(&mut repo, &cas, &[("a.txt", "v1")], "C1");
        seal_tree(&mut repo, &cas, &[("a.txt", "v1"), ("b.txt", "b")], "C2");

        let base = leaf_tree_files(&repo, &cas, c1.index);
        let theirs = parent_tree_files(&repo, &cas, c1.index); // empty
        let outcome = three_way_seal(&mut repo, &cas, &base, &theirs, "Test", "Undo C1").unwrap();

        assert!(matches!(outcome, ApplyOutcome::Applied(_)));
        let files = head_files(&repo, &cas, dir.path());
        assert!(!files.contains_key("a.txt"));
        assert!(files.contains_key("b.txt"));
    }

    #[test]
    fn conflicting_undo_is_refused() {
        let (_dir, mut repo, cas) = setup();
        seal_tree(&mut repo, &cas, &[("a.txt", "v1")], "C1");
        let c2 = seal_tree(&mut repo, &cas, &[("a.txt", "v2")], "C2");
        // C3 also touches a.txt → undoing C2 conflicts.
        seal_tree(&mut repo, &cas, &[("a.txt", "v3")], "C3");

        let base = leaf_tree_files(&repo, &cas, c2.index);
        let theirs = parent_tree_files(&repo, &cas, c2.index);
        let outcome = three_way_seal(&mut repo, &cas, &base, &theirs, "Test", "Undo C2").unwrap();

        match outcome {
            ApplyOutcome::Conflicts(paths) => assert_eq!(paths, vec!["a.txt".to_string()]),
            other => panic!("expected Conflicts, got {:?}", other),
        }
        // Nothing committed.
        assert_eq!(repo.walk_history("main").unwrap().len(), 3);
    }

    #[test]
    fn pluck_applies_only_the_delta() {
        let (dir, mut repo, cas) = setup();
        seal_tree(&mut repo, &cas, &[("a.txt", "base")], "C1");

        // Build the picked seal on a side timeline.
        repo.create_timeline("side", None).unwrap();
        repo.switch_timeline("side").unwrap();
        let picked = seal_tree(
            &mut repo,
            &cas,
            &[("a.txt", "base"), ("fix.txt", "the fix")],
            "Fix",
        );
        seal_tree(
            &mut repo,
            &cas,
            &[
                ("a.txt", "base"),
                ("fix.txt", "the fix"),
                ("extra.txt", "x"),
            ],
            "Extra",
        );
        repo.switch_timeline("main").unwrap();

        // Pluck: base = picked's parent tree, theirs = picked's tree.
        let base = parent_tree_files(&repo, &cas, picked.index);
        let theirs = leaf_tree_files(&repo, &cas, picked.index);
        let outcome = three_way_seal(&mut repo, &cas, &base, &theirs, "Test", "Fix").unwrap();

        assert!(matches!(outcome, ApplyOutcome::Applied(_)));
        let files = head_files(&repo, &cas, dir.path());
        assert!(files.contains_key("fix.txt"));
        // The later "Extra" seal's file did NOT come along.
        assert!(!files.contains_key("extra.txt"));
    }

    #[test]
    fn pluck_already_applied_is_noop() {
        let (_dir, mut repo, cas) = setup();
        seal_tree(&mut repo, &cas, &[("a.txt", "v1")], "C1");
        let c2 = seal_tree(&mut repo, &cas, &[("a.txt", "v1"), ("b.txt", "b")], "C2");

        // Plucking C2 onto a head that already contains it.
        let base = parent_tree_files(&repo, &cas, c2.index);
        let theirs = leaf_tree_files(&repo, &cas, c2.index);
        let outcome = three_way_seal(&mut repo, &cas, &base, &theirs, "Test", "again").unwrap();
        assert!(matches!(outcome, ApplyOutcome::NoChanges));
        assert_eq!(repo.walk_history("main").unwrap().len(), 2);
    }
}
