//! End-to-end integration of chunked large-file storage through the real
//! `ivaldi` binary: in a format-3 repository a large file must seal as a
//! chunk tree, stay clean in status, store only changed chunks on edit,
//! materialize on timeline switch, verify, and rescue — and a large file
//! sealed whole before a format 2 → 3 migration must not show as modified.

use std::path::Path;
use std::process::{Command, Output};

use ivaldi::cas::Cas;
use ivaldi::filechunk::{CHUNK_SIZE, CHUNKED_FILE_THRESHOLD};
use ivaldi::fsmerkle::{BlobNode, FsStore};
use ivaldi::hash::B3Hash;
use ivaldi::repo::Repo;

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
    ivaldi_ok(dir.path(), &["config", "--set", "user.name", "Chunk Test"]);
    ivaldi_ok(
        dir.path(),
        &["config", "--set", "user.email", "chunk@example.com"],
    );
    dir
}

/// Distinct chunks, a partial last chunk, just over the threshold.
fn large_content(seed: u8) -> Vec<u8> {
    let len = CHUNKED_FILE_THRESHOLD as usize + 2 * CHUNK_SIZE + 4321;
    (0..len)
        .map(|i| (i % 251) as u8 ^ (i / CHUNK_SIZE) as u8 ^ seed)
        .collect()
}

fn head_file_hash(dir: &Path, name: &str) -> B3Hash {
    let repo = Repo::open(dir).unwrap();
    let timeline = repo.current_timeline().unwrap();
    let head = repo.get_timeline_head(&timeline).unwrap().expect("head");
    let root = repo.get_leaf(head).unwrap().expect("leaf").tree_root;
    FsStore::new(&repo.cas)
        .load_tree(root)
        .unwrap()
        .find_entry(name)
        .expect("entry")
        .hash
}

fn object_count(dir: &Path) -> usize {
    ivaldi::gc::scan_all_objects(&dir.join(".ivaldi/objects"))
        .unwrap()
        .len()
}

fn assert_clean(dir: &Path) {
    let status = ivaldi_ok(dir, &["status"]);
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        !stdout.contains("modified") && !stdout.contains("untracked file"),
        "status not clean:\n{stdout}"
    );
}

#[test]
fn format3_repo_lifecycle_with_large_file() {
    let dir = setup_repo();
    let format = std::fs::read_to_string(dir.path().join(".ivaldi/FORMAT")).unwrap();
    assert!(format.contains("format = 3"), "FORMAT was:\n{format}");

    let v1 = large_content(1);
    std::fs::write(dir.path().join("big.bin"), &v1).unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "large file"]);

    let first = head_file_hash(dir.path(), "big.bin");
    {
        let repo = Repo::open(dir.path()).unwrap();
        let store = FsStore::new(&repo.cas);
        assert!(store.is_chunked_blob(first).unwrap());
        assert_eq!(store.load_blob(first).unwrap().1, v1);
    }
    assert_clean(dir.path());
    ivaldi_ok(dir.path(), &["verify", "--full"]);

    // Keep v1 on another timeline, then edit one byte on main.
    ivaldi_ok(dir.path(), &["timeline", "create", "before-edit"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    let mut v2 = v1.clone();
    v2[CHUNK_SIZE + 7] ^= 0xFF;
    std::fs::write(dir.path().join("big.bin"), &v2).unwrap();
    let before = object_count(dir.path());
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "one byte"]);
    // One chunk, the chunk root, the directory: not another whole copy.
    assert_eq!(object_count(dir.path()) - before, 3);
    assert_clean(dir.path());
    ivaldi_ok(dir.path(), &["verify", "--full"]);

    // Switching timelines streams each version back out.
    ivaldi_ok(dir.path(), &["timeline", "switch", "before-edit"]);
    assert_eq!(std::fs::read(dir.path().join("big.bin")).unwrap(), v1);
    assert_clean(dir.path());
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_eq!(std::fs::read(dir.path().join("big.bin")).unwrap(), v2);

    // Rescue reassembles chunked files from raw objects.
    let out = tempfile::tempdir().unwrap();
    ivaldi_ok(
        dir.path(),
        &["rescue", "--out", out.path().to_str().unwrap()],
    );
    let rescued: Vec<Vec<u8>> = std::fs::read_dir(out.path())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name() != "orphans")
        .map(|e| std::fs::read(e.path().join("big.bin")).unwrap())
        .collect();
    assert!(rescued.contains(&v1) && rescued.contains(&v2));
}

#[test]
fn migrated_repo_keeps_whole_blobs_until_content_changes() {
    let dir = setup_repo();
    // Simulate a repository created by a format-2 binary.
    std::fs::write(
        dir.path().join(".ivaldi/FORMAT"),
        "format = 2\nmin_ivaldi = 0.1.2\nfeatures =\n",
    )
    .unwrap();
    let content = large_content(2);
    std::fs::write(dir.path().join("big.bin"), &content).unwrap();
    std::fs::write(dir.path().join("note.txt"), "one\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "format 2"]);
    let whole = head_file_hash(dir.path(), "big.bin");
    assert_eq!(whole, BlobNode::hash_content(&content));

    ivaldi_ok(dir.path(), &["migrate"]);
    let format = std::fs::read_to_string(dir.path().join(".ivaldi/FORMAT")).unwrap();
    assert!(format.contains("format = 3"), "FORMAT was:\n{format}");
    {
        let repo = Repo::open(dir.path()).unwrap();
        assert!(repo.cas.chunked_files());
    }

    // Unchanged large file: clean, and sealing another change keeps its hash.
    assert_clean(dir.path());
    std::fs::write(dir.path().join("note.txt"), "two\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "unrelated change"]);
    assert_eq!(head_file_hash(dir.path(), "big.bin"), whole);
    ivaldi_ok(dir.path(), &["verify", "--full"]);

    // Real edit: now it is stored chunked.
    let mut edited = content.clone();
    edited[3] ^= 1;
    std::fs::write(dir.path().join("big.bin"), &edited).unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "edit"]);
    let chunked = head_file_hash(dir.path(), "big.bin");
    let repo = Repo::open(dir.path()).unwrap();
    let store = FsStore::new(&repo.cas);
    assert!(store.is_chunked_blob(chunked).unwrap());
    assert_eq!(store.load_blob(chunked).unwrap().1, edited);
}
