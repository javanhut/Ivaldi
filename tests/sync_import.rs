//! Sync-landing correctness: importing a fetched git pack into an Ivaldi
//! repo must translate remote parent SHAs into local leaf indices, be
//! idempotent on retry, leave the old head authoritative when a landing
//! fails, and pass `verify --full` after every successful landing.
//!
//! All tests build a `FetchResult` in-process (no network, no sockets).

use std::collections::HashMap;
use std::path::Path;

use ivaldi::git_remote::{
    FetchResult, GitObject, GitObjectKind, git_object_id, import_fetch_result,
};
use ivaldi::leaf::NO_PARENT;
use ivaldi::repo::Repo;

/// Tiny hex decoder (the `hex` crate is not a dev-dependency).
fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn add_blob(objects: &mut HashMap<String, GitObject>, data: &[u8]) -> String {
    let sha = git_object_id(GitObjectKind::Blob, data);
    objects.insert(
        sha.clone(),
        GitObject {
            kind: GitObjectKind::Blob,
            data: data.to_vec(),
        },
    );
    sha
}

fn add_tree(objects: &mut HashMap<String, GitObject>, entries: &[(&str, &str)]) -> String {
    let mut data = Vec::new();
    for (name, blob_sha) in entries {
        data.extend_from_slice(format!("100644 {}\0", name).as_bytes());
        data.extend_from_slice(&hex_decode(blob_sha));
    }
    let sha = git_object_id(GitObjectKind::Tree, &data);
    objects.insert(
        sha.clone(),
        GitObject {
            kind: GitObjectKind::Tree,
            data,
        },
    );
    sha
}

fn add_commit(
    objects: &mut HashMap<String, GitObject>,
    tree_sha: &str,
    parents: &[&str],
    msg: &str,
) -> String {
    let mut c = format!("tree {}\n", tree_sha);
    for p in parents {
        c.push_str(&format!("parent {}\n", p));
    }
    c.push_str("author Tester <t@x> 1710000000 +0000\n");
    c.push_str("committer Tester <t@x> 1710000000 +0000\n\n");
    c.push_str(msg);
    c.push('\n');
    let data = c.into_bytes();
    let sha = git_object_id(GitObjectKind::Commit, &data);
    objects.insert(
        sha.clone(),
        GitObject {
            kind: GitObjectKind::Commit,
            data,
        },
    );
    sha
}

fn fetch_result(branch: &str, head: &str, objects: HashMap<String, GitObject>) -> FetchResult {
    FetchResult {
        branch: branch.to_string(),
        head_sha: head.to_string(),
        refs: Vec::new(),
        objects,
    }
}

/// Two-commit chain used by several tests: returns (objects, root_sha, tip_sha).
fn two_commit_chain() -> (HashMap<String, GitObject>, String, String) {
    let mut objects = HashMap::new();
    let b1 = add_blob(&mut objects, b"first body");
    let t1 = add_tree(&mut objects, &[("a.txt", &b1)]);
    let c1 = add_commit(&mut objects, &t1, &[], "root");
    let b2 = add_blob(&mut objects, b"second body");
    let t2 = add_tree(&mut objects, &[("a.txt", &b1), ("b.txt", &b2)]);
    let c2 = add_commit(&mut objects, &t2, &[c1.as_str()], "tip");
    (objects, c1, c2)
}

fn verify_full_ok(dir: &Path) {
    let report = ivaldi::verify::verify(dir, true);
    assert!(report.ok, "verify --full failed: {:?}", report.checks);
}

/// Seal one local commit (used to pre-populate unrelated history).
fn seal_local(dir: &Path, name: &str, body: &[u8]) {
    let mut repo = Repo::open(dir).unwrap();
    let cas = ivaldi::cas::FileCas::new(dir.join(".ivaldi/objects")).unwrap();
    let store = ivaldi::fsmerkle::FsStore::new(&cas);
    let (blob, _) = store.put_blob(body).unwrap();
    use ivaldi::fsmerkle::{Entry, MODE_FILE, NodeKind};
    let tree = store
        .put_tree(vec![Entry {
            name: name.into(),
            mode: MODE_FILE,
            kind: NodeKind::Blob,
            hash: blob,
        }])
        .unwrap();
    repo.commit(tree, "local <l@x>", &format!("local {}", name))
        .unwrap();
}

#[test]
fn import_into_empty_repo_builds_correct_chain() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();

    let (objects, _c1, c2) = two_commit_chain();
    let fetch = fetch_result("main", &c2, objects);

    {
        let mut repo = Repo::open(dir.path()).unwrap();
        let result = import_fetch_result(&mut repo, &fetch).unwrap();
        assert_eq!(result.commits_imported, 2);
        assert_eq!(result.commits_skipped, 0);

        let head = repo.get_timeline_head("main").unwrap().unwrap();
        assert_eq!(head, 1);
        let tip = repo.get_leaf(1).unwrap().unwrap();
        assert_eq!(tip.prev_idx, 0);
        assert_eq!(tip.message, "tip\n");
        let root = repo.get_leaf(0).unwrap().unwrap();
        assert_eq!(root.prev_idx, NO_PARENT);
    }
    verify_full_ok(dir.path());
}

#[test]
fn import_into_repo_with_unrelated_history_remaps_parent_indices() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();
    // Local seals occupy indices 0-1 — the imported chain must NOT reference
    // them even though the remote chain "starts at 0" in its own terms.
    seal_local(dir.path(), "local1.txt", b"l1");
    seal_local(dir.path(), "local2.txt", b"l2");

    let (objects, _c1, c2) = two_commit_chain();
    let fetch = fetch_result("feature", &c2, objects);

    {
        let mut repo = Repo::open(dir.path()).unwrap();
        let result = import_fetch_result(&mut repo, &fetch).unwrap();
        assert_eq!(result.commits_imported, 2);

        // Imported leaves land at shifted indices 2-3 with parents remapped.
        let head = repo.get_timeline_head("feature").unwrap().unwrap();
        assert_eq!(head, 3);
        let tip = repo.get_leaf(3).unwrap().unwrap();
        assert_eq!(tip.prev_idx, 2);
        let root = repo.get_leaf(2).unwrap().unwrap();
        assert_eq!(root.prev_idx, NO_PARENT);

        // Local history untouched.
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(1));
    }
    verify_full_ok(dir.path());
}

#[test]
fn import_preserves_merge_parents() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();

    let mut objects = HashMap::new();
    let b = add_blob(&mut objects, b"base");
    let t = add_tree(&mut objects, &[("f.txt", &b)]);
    let a = add_commit(&mut objects, &t, &[], "a");
    let b1 = add_commit(&mut objects, &t, &[a.as_str()], "b");
    let c1 = add_commit(&mut objects, &t, &[a.as_str()], "c");
    let m = add_commit(&mut objects, &t, &[b1.as_str(), c1.as_str()], "merge");
    let fetch = fetch_result("main", &m, objects);

    {
        let mut repo = Repo::open(dir.path()).unwrap();
        import_fetch_result(&mut repo, &fetch).unwrap();

        let head = repo.get_timeline_head("main").unwrap().unwrap();
        let merge_leaf = repo.get_leaf(head).unwrap().unwrap();
        assert_eq!(merge_leaf.message, "merge\n");
        assert!(merge_leaf.is_merge());
        assert_eq!(merge_leaf.merge_idxs.len(), 1);

        // prev and merge parent must be the leaves for "b" and "c".
        let prev = repo.get_leaf(merge_leaf.prev_idx).unwrap().unwrap();
        let merge_parent = repo.get_leaf(merge_leaf.merge_idxs[0]).unwrap().unwrap();
        let mut msgs = vec![prev.message, merge_parent.message];
        msgs.sort();
        assert_eq!(msgs, vec!["b\n".to_string(), "c\n".to_string()]);
        // Both branch commits point back at "a" (the first imported leaf).
        assert_eq!(prev.prev_idx, 0);
        assert_eq!(merge_parent.prev_idx, 0);
        assert_eq!(repo.get_leaf(0).unwrap().unwrap().message, "a\n");
    }
    verify_full_ok(dir.path());
}

#[test]
fn import_retry_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();

    let (objects, _c1, c2) = two_commit_chain();
    let fetch = fetch_result("main", &c2, objects);

    {
        let mut repo = Repo::open(dir.path()).unwrap();
        let first = import_fetch_result(&mut repo, &fetch).unwrap();
        assert_eq!(first.commits_imported, 2);
        assert_eq!(repo.commit_count(), 2);

        let second = import_fetch_result(&mut repo, &fetch).unwrap();
        assert_eq!(
            second.commits_imported, 0,
            "retry must not duplicate history"
        );
        assert_eq!(second.commits_skipped, 2);
        assert_eq!(repo.commit_count(), 2);
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(1));
    }
    verify_full_ok(dir.path());
}

#[test]
fn failed_landing_leaves_existing_state_authoritative_and_retry_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();
    seal_local(dir.path(), "local.txt", b"pre-existing");

    // A truncated fetch: the tip's parent commit object is missing.
    let (full_objects, c1, c2) = two_commit_chain();
    let mut truncated = full_objects.clone();
    truncated.remove(&c1);
    let bad_fetch = fetch_result("feature", &c2, truncated);

    {
        let mut repo = Repo::open(dir.path()).unwrap();
        let before = repo.commit_count();
        let err = import_fetch_result(&mut repo, &bad_fetch)
            .expect_err("truncated fetch must fail loudly");
        assert!(err.to_string().contains("missing commit object"), "{}", err);

        // Old state authoritative: nothing landed, no half-made timeline head.
        assert_eq!(repo.commit_count(), before);
        assert!(repo.get_timeline_head("feature").unwrap().is_none());
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(0));

        // Retry with the complete fetch succeeds and lands exactly once.
        let good_fetch = fetch_result("feature", &c2, full_objects);
        let result = import_fetch_result(&mut repo, &good_fetch).unwrap();
        assert_eq!(result.commits_imported, 2);
        assert_eq!(repo.commit_count(), before + 2);
    }
    verify_full_ok(dir.path());
}

#[test]
fn identical_content_imported_at_different_indices_stays_distinct() {
    // Import the same remote chain into two repos with different amounts of
    // pre-existing history: both must produce self-consistent chains at
    // their own (different) indices.
    let (objects, _c1, c2) = two_commit_chain();

    let plain = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(plain.path()).unwrap();
    {
        let mut repo = Repo::open(plain.path()).unwrap();
        import_fetch_result(&mut repo, &fetch_result("main", &c2, objects.clone())).unwrap();
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(1));
    }
    verify_full_ok(plain.path());

    let shifted = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(shifted.path()).unwrap();
    seal_local(shifted.path(), "x.txt", b"occupies index 0");
    {
        let mut repo = Repo::open(shifted.path()).unwrap();
        import_fetch_result(&mut repo, &fetch_result("feature", &c2, objects)).unwrap();
        let head = repo.get_timeline_head("feature").unwrap().unwrap();
        assert_eq!(head, 2);
        assert_eq!(repo.get_leaf(2).unwrap().unwrap().prev_idx, 1);
        assert_eq!(repo.get_leaf(1).unwrap().unwrap().prev_idx, NO_PARENT);
    }
    verify_full_ok(shifted.path());
}
