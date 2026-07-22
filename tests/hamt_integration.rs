//! End-to-end integration of the HAMT directory encoding through the real
//! `ivaldi` binary: a format-2 repository with a large directory must seal,
//! status, diff, verify, and rescue correctly with its root stored as a HAMT
//! — while a format-1 repository must never receive a HAMT object.

use std::path::Path;
use std::process::{Command, Output};

use ivaldi::cas::Cas;
use ivaldi::repo::Repo;

/// One entry over the threshold, so the top directory becomes a HAMT root.
const N_FILES: usize = ivaldi::fsmerkle::HAMT_DIR_THRESHOLD + 44;

fn ivaldi_ok(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("run ivaldi binary");
    assert!(
        output.status.success(),
        "ivaldi {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn setup_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ivaldi_ok(dir.path(), &["forge"]);
    ivaldi_ok(dir.path(), &["config", "--set", "user.name", "Hamt Test"]);
    ivaldi_ok(
        dir.path(),
        &["config", "--set", "user.email", "hamt@example.com"],
    );
    dir
}

fn write_big_dir(dir: &Path) {
    for i in 0..N_FILES {
        std::fs::write(
            dir.join(format!("file_{:04}.txt", i)),
            format!("content {}\n", i),
        )
        .unwrap();
    }
}

fn head_tree_root(dir: &Path) -> ivaldi::hash::B3Hash {
    let repo = Repo::open(dir).unwrap();
    let timeline = repo.current_timeline().unwrap();
    let head = repo
        .get_timeline_head(&timeline)
        .unwrap()
        .expect("timeline head");
    repo.get_leaf(head).unwrap().expect("head leaf").tree_root
}

#[test]
fn format2_repo_lifecycle_with_hamt_directory() {
    let dir = setup_repo();

    // New repositories are stamped format 2.
    let format = std::fs::read_to_string(dir.path().join(".ivaldi/FORMAT")).unwrap();
    assert!(format.contains("format = 2"), "FORMAT was:\n{format}");

    write_big_dir(dir.path());
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "big directory"]);

    // The sealed root must actually be a HAMT node, not a fsmerkle tree.
    let root = head_tree_root(dir.path());
    let repo = Repo::open(dir.path()).unwrap();
    let bytes = repo.cas.get(root).unwrap();
    assert!(
        ivaldi::hamt::is_hamt_node(&bytes),
        "root of a {N_FILES}-entry directory should be a HAMT node"
    );

    // Transparent read-back: the flattened directory holds every file.
    let store = ivaldi::fsmerkle::FsStore::new(&repo.cas);
    assert_eq!(store.load_tree(root).unwrap().entries.len(), N_FILES);
    drop(repo);

    // Status must be clean right after sealing (workspace walk reads HAMT).
    let status = ivaldi_ok(dir.path(), &["status"]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        !stdout.contains("modified") && !stdout.contains("untracked file"),
        "status not clean after seal:\n{stdout}"
    );

    // Full verification walks HAMT interior nodes.
    ivaldi_ok(dir.path(), &["verify", "--full"]);

    // Change one file and seal again; diff between the two seals must report
    // exactly that file (exercises the HAMT structural diff).
    std::fs::write(dir.path().join("file_0100.txt"), "changed\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "one change"]);
    ivaldi_ok(dir.path(), &["verify", "--full"]);

    // Diff the two seals' roots: the HAMT structural diff must report
    // exactly the one changed file.
    let repo = Repo::open(dir.path()).unwrap();
    let timeline = repo.current_timeline().unwrap();
    let head = repo.get_timeline_head(&timeline).unwrap().unwrap();
    let new_leaf = repo.get_leaf(head).unwrap().unwrap();
    let old_leaf = repo.get_leaf(new_leaf.prev_idx).unwrap().unwrap();
    let store = ivaldi::fsmerkle::FsStore::new(&repo.cas);
    let changes =
        ivaldi::fsmerkle::diff_trees(old_leaf.tree_root, new_leaf.tree_root, &store).unwrap();
    assert_eq!(changes.len(), 1, "changes: {:?}", changes);
    assert_eq!(changes[0].path, "file_0100.txt");
    assert_eq!(changes[0].kind, ivaldi::fsmerkle::ChangeKind::Modified);
}

#[test]
fn rescue_recovers_hamt_directories() {
    let dir = setup_repo();
    write_big_dir(dir.path());
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "to be rescued"]);

    let out = tempfile::tempdir().unwrap();
    let out_arg = out.path().to_str().unwrap();
    ivaldi_ok(dir.path(), &["rescue", "--out", out_arg]);

    // Every file of the HAMT-encoded directory must come back.
    let snapshot = std::fs::read_dir(out.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "orphans"))
        .expect("rescue produced a snapshot dir");
    let rescued = std::fs::read_dir(&snapshot).unwrap().flatten().count();
    assert_eq!(rescued, N_FILES, "rescue must recover all files");
    let content = std::fs::read_to_string(snapshot.join("file_0042.txt")).unwrap();
    assert_eq!(content, "content 42\n");
}

#[test]
fn format1_repo_never_writes_hamt_objects() {
    let dir = setup_repo();

    // Downgrade the repository to format 1 before any content exists —
    // simulates a repo created by an older ivaldi.
    std::fs::write(
        dir.path().join(".ivaldi/FORMAT"),
        "format = 1\nmin_ivaldi = 0.1.1\nfeatures =\n",
    )
    .unwrap();

    write_big_dir(dir.path());
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "big directory, old format"]);

    let root = head_tree_root(dir.path());
    let repo = Repo::open(dir.path()).unwrap();
    let bytes = repo.cas.get(root).unwrap();
    assert!(
        !ivaldi::hamt::is_hamt_node(&bytes),
        "a format-1 repository must never contain HAMT objects"
    );
    let store = ivaldi::fsmerkle::FsStore::new(&repo.cas);
    assert_eq!(store.load_tree(root).unwrap().entries.len(), N_FILES);
    drop(repo);

    ivaldi_ok(dir.path(), &["verify", "--full"]);
}
