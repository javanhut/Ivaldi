//! Process-abort matrix for import landing and maintenance mutations.
//!
//! Every parent test kills this test binary at a real production failpoint,
//! inspects the bytes/state left by the dead process, retries the same public
//! operation, and asserts exact convergence. These are not error-return mocks:
//! `abort()` skips Rust destructors exactly as an abrupt process loss would.
#![cfg(feature = "failpoints")]

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ivaldi::cas::{Cas, FileCas};
use ivaldi::git_remote::{
    FetchResult, GitObject, GitObjectKind, git_object_id, import_fetch_result,
};
use ivaldi::hash::B3Hash;
use ivaldi::repo::Repo;

const CHILD_ENV: &str = "IVALDI_MAINTENANCE_CRASH_CHILD";
const OP_ENV: &str = "IVALDI_MAINTENANCE_CRASH_OP";
const DIR_ENV: &str = "IVALDI_MAINTENANCE_CRASH_DIR";
const BASE_COMMIT: &str = "1111111111111111111111111111111111111111";
const REMOTE_TIP: &str = "2222222222222222222222222222222222222222";
const BASE_TREE: &str = "3333333333333333333333333333333333333333";
const TIP_TREE: &str = "4444444444444444444444444444444444444444";
const BASE_BLOB: &str = "5555555555555555555555555555555555555555";
const REMOTE_BLOB: &str = "6666666666666666666666666666666666666666";

fn child(dir: &Path, operation: &str, failpoint: &str) -> Output {
    Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("crash_child")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env(OP_ENV, operation)
        .env(DIR_ENV, dir)
        .env("IVALDI_FAILPOINT", failpoint)
        .output()
        .expect("run crash child")
}

fn assert_aborted(output: &Output, failpoint: &str) {
    assert!(
        !output.status.success(),
        "{failpoint} unexpectedly succeeded"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!("failpoint hit: {failpoint}")),
        "child failed without reaching {failpoint}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_full_ok(dir: &Path) {
    let report = ivaldi::verify::verify(dir, true);
    assert!(report.ok, "verify --full failed: {:?}", report.checks);
}

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
        data.extend_from_slice(format!("100644 {name}\0").as_bytes());
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
    message: &str,
) -> String {
    let mut body = format!("tree {tree_sha}\n");
    for parent in parents {
        body.push_str(&format!("parent {parent}\n"));
    }
    body.push_str("author Crash Test <crash@example.com> 1710000000 +0000\n");
    body.push_str("committer Crash Test <crash@example.com> 1710000000 +0000\n\n");
    body.push_str(message);
    body.push('\n');
    let data = body.into_bytes();
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

fn import_fixture() -> FetchResult {
    let mut objects = HashMap::new();
    let first_blob = add_blob(&mut objects, b"first\n");
    let first_tree = add_tree(&mut objects, &[("first.txt", &first_blob)]);
    let root = add_commit(&mut objects, &first_tree, &[], "root");
    let second_blob = add_blob(&mut objects, b"second\n");
    let second_tree = add_tree(
        &mut objects,
        &[("first.txt", &first_blob), ("second.txt", &second_blob)],
    );
    let tip = add_commit(&mut objects, &second_tree, &[&root], "tip");
    FetchResult {
        branch: "imported".into(),
        head_sha: tip,
        refs: Vec::new(),
        objects,
    }
}

fn run_import(dir: &Path) {
    let mut repo = Repo::open(dir).unwrap();
    import_fetch_result(&mut repo, &import_fixture()).unwrap();
}

fn write_pack(dir: &Path) {
    let mut writer = ivaldi::pack::PackWriter::new();
    for data in [
        b"alpha alpha alpha".as_slice(),
        b"alpha alpha beta".as_slice(),
        b"completely different".as_slice(),
    ] {
        writer.add(B3Hash::digest(data), data.to_vec());
    }
    writer.write_delta(dir).unwrap();
}

fn extract_pack(dir: &Path) {
    let reader = ivaldi::pack::PackReader::new(&dir.join("packs"));
    let cas = FileCas::new(dir.join("objects")).unwrap();
    reader.extract_to_cas(&cas).unwrap();
}

fn run_gc(dir: &Path) {
    let live = B3Hash::digest(b"live-object");
    ivaldi::gc::collect_garbage(dir, &BTreeSet::from([live]), false).unwrap();
}

fn mock_response(path: &str) -> (&'static str, String) {
    if path.contains("/branches?") {
        return (
            "application/json",
            format!(r#"[{{"name":"main","commit":{{"sha":"{REMOTE_TIP}"}}}}]"#),
        );
    }
    if path.contains("/commits?") {
        return (
            "application/json",
            format!(
                r#"[
{{"sha":"{REMOTE_TIP}","commit":{{"message":"remote change","author":{{"name":"Remote","email":"r@example.com","date":"2024-01-02T00:00:00Z"}},"tree":{{"sha":"{TIP_TREE}"}}}},"parents":[{{"sha":"{BASE_COMMIT}"}}]}},
{{"sha":"{BASE_COMMIT}","commit":{{"message":"base","author":{{"name":"Remote","email":"r@example.com","date":"2024-01-01T00:00:00Z"}},"tree":{{"sha":"{BASE_TREE}"}}}},"parents":[]}}
]"#
            ),
        );
    }
    if path.contains(&format!("/git/trees/{TIP_TREE}")) {
        return (
            "application/json",
            format!(
                r#"{{"sha":"{TIP_TREE}","truncated":false,"tree":[
{{"path":"base.txt","mode":"100644","type":"blob","size":5,"sha":"{BASE_BLOB}"}},
{{"path":"remote.txt","mode":"100644","type":"blob","size":7,"sha":"{REMOTE_BLOB}"}}
]}}"#
            ),
        );
    }
    if path.contains("/o/r/") && path.ends_with("/remote.txt") {
        return ("application/octet-stream", "remote\n".into());
    }
    (
        "application/json",
        format!(r#"{{"message":"unexpected mock path: {path}"}}"#),
    )
}

fn serve_connection(mut stream: TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut request = [0u8; 8192];
    let read = stream.read(&mut request).unwrap_or(0);
    let first = String::from_utf8_lossy(&request[..read]);
    let path = first
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let (content_type, body) = mock_response(path);
    let status = if body.contains("unexpected mock path") {
        "404 Not Found"
    } else {
        "200 OK"
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn run_sync(dir: &Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let server_done = Arc::clone(&done);
    let server = std::thread::spawn(move || {
        while !server_done.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => serve_connection(stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("mock server failed: {error}"),
            }
        }
    });
    let base = format!("http://{address}");
    let client = ivaldi::github::GitHubClient::with_base_urls(&base, &base);
    let mut repo = Repo::open(dir).unwrap();
    let result = ivaldi::sync::sync_timeline(
        &client,
        &mut repo,
        "o",
        "r",
        "main",
        &mut |_, _| true,
        false,
    );
    done.store(true, Ordering::Release);
    server.join().unwrap();
    result.unwrap();
}

/// Child entry point invoked by the parent tests above. With no marker it is a
/// harmless normal test; with the marker it runs exactly one real operation.
#[test]
fn crash_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(DIR_ENV).expect("child directory"));
    match std::env::var(OP_ENV).as_deref() {
        Ok("import") => run_import(&dir),
        Ok("recover") => {
            let _ = ivaldi::recover::recover(&dir, false);
        }
        Ok("gc") => run_gc(&dir),
        Ok("pack") => write_pack(&dir),
        Ok("pack-extract") => extract_pack(&dir),
        Ok("sync") => run_sync(&dir),
        other => panic!("unknown child operation: {other:?}"),
    }
}

fn setup_diverged_sync_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();
    let mut repo = Repo::open(dir.path()).unwrap();
    let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
    let store = ivaldi::fsmerkle::FsStore::new(&cas);
    let (base_blob, _) = store.put_blob(b"base\n").unwrap();
    let base_tree = store
        .put_tree(vec![ivaldi::fsmerkle::Entry {
            name: "base.txt".into(),
            mode: ivaldi::fsmerkle::MODE_FILE,
            kind: ivaldi::fsmerkle::NodeKind::Blob,
            hash: base_blob,
        }])
        .unwrap();
    let mut base_leaf = ivaldi::leaf::Leaf::new(base_tree, "main", "Remote", 1, "base");
    base_leaf.meta.insert("git.sha1".into(), BASE_COMMIT.into());
    cas.flush().unwrap();
    let base_result = repo.commit_raw(base_leaf, "main").unwrap();

    let (local_blob, _) = store.put_blob(b"local\n").unwrap();
    let local_tree = store
        .put_tree(vec![
            ivaldi::fsmerkle::Entry {
                name: "base.txt".into(),
                mode: ivaldi::fsmerkle::MODE_FILE,
                kind: ivaldi::fsmerkle::NodeKind::Blob,
                hash: base_blob,
            },
            ivaldi::fsmerkle::Entry {
                name: "local.txt".into(),
                mode: ivaldi::fsmerkle::MODE_FILE,
                kind: ivaldi::fsmerkle::NodeKind::Blob,
                hash: local_blob,
            },
        ])
        .unwrap();
    cas.flush().unwrap();
    repo.commit(local_tree, "Local", "local change").unwrap();
    let mut mapping = ivaldi::remote::HashMapping::new(&repo.ivaldi_dir);
    mapping.insert(BASE_COMMIT, base_result.hash);
    mapping.insert(BASE_BLOB, base_blob);
    mapping.save().unwrap();
    drop(repo);
    std::fs::write(dir.path().join("base.txt"), b"base\n").unwrap();
    std::fs::write(dir.path().join("local.txt"), b"local\n").unwrap();
    dir
}

#[test]
fn sync_never_claims_or_deletes_a_user_owned_temporary_namespace_timeline() {
    let dir = setup_diverged_sync_repo();
    let protected_name = "__sync_main";
    let protected_marker = dir.path().join(".ivaldi/refs/heads/__sync_main");

    // This is a genuine user timeline accepted by the public repository API,
    // not debris manufactured by a previous sync. Give it its own head so
    // replacing or deleting its authority cannot be mistaken for harmless
    // cleanup of an empty marker.
    let mut repo = Repo::open(dir.path()).unwrap();
    repo.create_timeline(protected_name, Some("main")).unwrap();
    let main_head = repo.get_timeline_head("main").unwrap().unwrap();
    let main_leaf = repo.get_leaf(main_head).unwrap().unwrap();
    let mut protected_leaf = ivaldi::leaf::Leaf::new(
        main_leaf.tree_root,
        protected_name,
        "Timeline Owner",
        3,
        "user-owned protected timeline",
    );
    protected_leaf.prev_idx = main_head;
    let protected_head = repo
        .commit_raw(protected_leaf, protected_name)
        .unwrap()
        .index;
    assert_eq!(
        repo.get_timeline_head(protected_name).unwrap(),
        Some(protected_head)
    );
    assert!(protected_marker.exists());
    drop(repo);

    let output = child(dir.path(), "sync", "sync.after_temp_timeline");
    assert_aborted(&output, "sync.after_temp_timeline");
    verify_full_ok(dir.path());

    // Even while the process-owned alternate scratch timeline is stranded,
    // the colliding user timeline remains authoritative. The journal must
    // identify a different name so recovery has explicit cleanup authority.
    let interrupted = Repo::open(dir.path()).unwrap();
    assert_eq!(
        interrupted.get_timeline_head(protected_name).unwrap(),
        Some(protected_head)
    );
    assert!(protected_marker.exists());
    drop(interrupted);
    let journal = std::fs::read_to_string(dir.path().join(".ivaldi/sync-journal.json")).unwrap();
    assert!(journal.contains(r#""temp_timeline":"__sync_main_1""#));

    run_sync(dir.path());
    run_sync(dir.path());
    verify_full_ok(dir.path());

    // Sync may complete, refuse the collision, or choose another internal
    // namespace. It may never reinterpret a valid user timeline as its own
    // scratch state. Both pieces of timeline authority must survive exactly.
    let reopened = Repo::open(dir.path()).unwrap();
    assert_eq!(
        reopened.get_timeline_head(protected_name).unwrap(),
        Some(protected_head),
        "sync deleted or replaced a user-owned timeline head"
    );
    assert!(
        protected_marker.exists(),
        "sync deleted a user-owned timeline ref marker"
    );
    let protected_leaf = reopened.get_leaf(protected_head).unwrap().unwrap();
    assert_eq!(protected_leaf.message, "user-owned protected timeline");
    assert_eq!(
        reopened.list_timelines().unwrap(),
        vec![(protected_name.into(), protected_head), ("main".into(), 4)],
        "sync left a scratch timeline authoritative after avoiding the collision"
    );
    assert!(!dir.path().join(".ivaldi/sync-journal.json").exists());
}

#[test]
fn diverged_sync_crashes_finalize_one_fuse_and_clean_temporary_authority() {
    for failpoint in [
        "sync.after_temp_timeline",
        "sync.after_fuse_commit",
        "sync.before_tip_remap",
        "sync.after_cleanup",
    ] {
        let dir = setup_diverged_sync_repo();
        let output = child(dir.path(), "sync", failpoint);
        assert_aborted(&output, failpoint);
        verify_full_ok(dir.path());
        if dir.path().join(".ivaldi/sync-journal.json").exists() {
            let blocked = Command::new(env!("CARGO_BIN_EXE_ivaldi"))
                .current_dir(dir.path())
                .args(["timeline", "create", "must-not-land"])
                .output()
                .unwrap();
            assert!(!blocked.status.success());
            assert!(
                String::from_utf8_lossy(&blocked.stderr).contains("interrupted sync"),
                "unrelated mutation was not blocked after {failpoint}"
            );
            assert!(!dir.path().join(".ivaldi/refs/heads/must-not-land").exists());
        }

        run_sync(dir.path());
        run_sync(dir.path());
        verify_full_ok(dir.path());
        let repo = Repo::open(dir.path()).unwrap();
        assert_eq!(
            repo.commit_count(),
            4,
            "sync retry at {failpoint} duplicated history"
        );
        let head = repo.get_timeline_head("main").unwrap().unwrap();
        let leaf = repo.get_leaf(head).unwrap().unwrap();
        assert_eq!(
            leaf.meta.get("sync.remote_tip").map(String::as_str),
            Some(REMOTE_TIP)
        );
        assert_eq!(repo.get_timeline_head("__sync_main").unwrap(), None);
        assert!(!dir.path().join(".ivaldi/refs/heads/__sync_main").exists());
        assert!(!dir.path().join(".ivaldi/sync-journal.json").exists());
        assert_eq!(
            std::fs::read(dir.path().join("base.txt")).unwrap(),
            b"base\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("local.txt")).unwrap(),
            b"local\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("remote.txt")).unwrap(),
            b"remote\n"
        );
    }
}

#[test]
fn sync_import_phase_crashes_reuse_landed_remote_leaves_and_finish_fusion() {
    for (failpoint, landed_prefix) in [("import.api.after_blobs", 2), ("import.api.mid_commits", 3)]
    {
        let dir = setup_diverged_sync_repo();
        let output = child(dir.path(), "sync", failpoint);
        assert_aborted(&output, failpoint);
        verify_full_ok(dir.path());

        let interrupted = Repo::open(dir.path()).unwrap();
        assert_eq!(
            interrupted.commit_count(),
            landed_prefix,
            "unexpected durable prefix at {failpoint}"
        );
        drop(interrupted);

        // This death happens before the outer sync journal is published. The
        // retry must reconstruct Git SHA mappings from authenticated leaves,
        // repoint the fresh temporary timeline at an already-landed remote
        // tip, and create exactly one fusion commit.
        run_sync(dir.path());
        run_sync(dir.path());
        verify_full_ok(dir.path());

        let repo = Repo::open(dir.path()).unwrap();
        assert_eq!(
            repo.commit_count(),
            4,
            "retry after {failpoint} duplicated the remote tip or fusion"
        );
        let head = repo.get_timeline_head("main").unwrap().unwrap();
        let leaf = repo.get_leaf(head).unwrap().unwrap();
        assert_eq!(
            leaf.meta.get("sync.remote_tip").map(String::as_str),
            Some(REMOTE_TIP)
        );
        assert_eq!(repo.get_timeline_head("__sync_main").unwrap(), None);
        assert!(!dir.path().join(".ivaldi/refs/heads/__sync_main").exists());
        assert!(!dir.path().join(".ivaldi/sync-journal.json").exists());
        assert_eq!(
            std::fs::read(dir.path().join("base.txt")).unwrap(),
            b"base\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("local.txt")).unwrap(),
            b"local\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("remote.txt")).unwrap(),
            b"remote\n"
        );
    }
}

#[test]
fn import_crashes_before_or_after_mapping_publication_converge_exactly_once() {
    for failpoint in [
        "import.after_blob_prefetch",
        "import.before_mapping_save",
        "import.after_mapping_save",
    ] {
        let dir = tempfile::tempdir().unwrap();
        ivaldi::forge::forge(dir.path()).unwrap();
        let output = child(dir.path(), "import", failpoint);
        assert_aborted(&output, failpoint);

        // Every landed prefix is a valid append-only repository. Retry must
        // produce the same two-leaf chain, never a duplicate or severed root.
        verify_full_ok(dir.path());
        run_import(dir.path());
        verify_full_ok(dir.path());
        let repo = Repo::open(dir.path()).unwrap();
        assert_eq!(
            repo.commit_count(),
            2,
            "retry after {failpoint} duplicated history"
        );
        let head = repo.get_timeline_head("imported").unwrap().unwrap();
        let tip = repo.get_leaf(head).unwrap().unwrap();
        assert_eq!(tip.prev_idx, 0);
        assert_eq!(tip.meta.get("git.sha1"), Some(&import_fixture().head_sha));
    }
}

#[test]
fn repeated_crashes_in_commit_mapping_window_do_not_accumulate_orphans() {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();

    // First death lands root before its mapping; second retry must discover
    // that root from authenticated leaf metadata and lands only the tip.
    for expected_count in [1, 2] {
        let output = child(dir.path(), "import", "import.mid_commit_loop");
        assert_aborted(&output, "import.mid_commit_loop");
        verify_full_ok(dir.path());
        assert_eq!(
            Repo::open(dir.path()).unwrap().commit_count(),
            expected_count
        );
    }
    run_import(dir.path());
    run_import(dir.path());
    verify_full_ok(dir.path());
    assert_eq!(Repo::open(dir.path()).unwrap().commit_count(), 2);
}

fn setup_repo_with_commit() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ivaldi::forge::forge(dir.path()).unwrap();
    let mut repo = Repo::open(dir.path()).unwrap();
    let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
    let store = ivaldi::fsmerkle::FsStore::new(&cas);
    let (blob, _) = store.put_blob(b"preserved\n").unwrap();
    let tree = store
        .put_tree(vec![ivaldi::fsmerkle::Entry {
            name: "preserved.txt".into(),
            mode: ivaldi::fsmerkle::MODE_FILE,
            kind: ivaldi::fsmerkle::NodeKind::Blob,
            hash: blob,
        }])
        .unwrap();
    cas.flush().unwrap();
    repo.commit(tree, "Crash Test", "baseline").unwrap();
    drop(repo);
    dir
}

#[test]
fn recover_ref_recreation_is_old_or_new_and_retry_converges() {
    for failpoint in ["recover.before_ref_write", "recover.after_ref_write"] {
        let dir = setup_repo_with_commit();
        let marker = dir.path().join(".ivaldi/refs/heads/main");
        std::fs::remove_file(&marker).unwrap();
        assert!(!ivaldi::verify::verify(dir.path(), true).ok);

        let output = child(dir.path(), "recover", failpoint);
        assert_aborted(&output, failpoint);
        if failpoint == "recover.before_ref_write" {
            assert!(!marker.exists());
        } else {
            assert!(marker.exists());
            verify_full_ok(dir.path());
        }

        let _ = ivaldi::recover::recover(dir.path(), false);
        assert!(marker.exists());
        verify_full_ok(dir.path());
    }
}

#[test]
fn recover_checkpoint_transaction_is_old_or_new_and_retry_converges() {
    for failpoint in ["recover.before_checkpoint", "recover.after_checkpoint"] {
        let dir = setup_repo_with_commit();
        let store = ivaldi::store::Store::open(&dir.path().join(".ivaldi/store.db")).unwrap();
        store.remove_meta(ivaldi::store::MMR_SIZE_KEY).unwrap();
        store.remove_meta(ivaldi::store::MMR_ROOT_KEY).unwrap();
        drop(store);

        let output = child(dir.path(), "recover", failpoint);
        assert_aborted(&output, failpoint);
        let store = ivaldi::store::Store::open(&dir.path().join(".ivaldi/store.db")).unwrap();
        let present = store
            .get_meta(ivaldi::store::MMR_SIZE_KEY)
            .unwrap()
            .is_some();
        assert_eq!(present, failpoint == "recover.after_checkpoint");
        drop(store);

        let _ = ivaldi::recover::recover(dir.path(), false);
        verify_full_ok(dir.path());
    }
}

#[test]
fn recover_quarantine_never_loses_corrupt_evidence() {
    for failpoint in ["recover.before_quarantine", "recover.after_quarantine"] {
        let dir = setup_repo_with_commit();
        let claimed = B3Hash::digest(b"claimed-content");
        let hex = claimed.to_hex();
        let source = dir
            .path()
            .join(".ivaldi/objects")
            .join(&hex[..2])
            .join(&hex[2..]);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"actual corrupt evidence").unwrap();
        let destination = dir
            .path()
            .join(".ivaldi/quarantine")
            .join(&hex[..2])
            .join(&hex[2..]);

        let output = child(dir.path(), "recover", failpoint);
        assert_aborted(&output, failpoint);
        assert!(source.exists() ^ destination.exists());
        let evidence = if source.exists() {
            &source
        } else {
            &destination
        };
        assert_eq!(std::fs::read(evidence).unwrap(), b"actual corrupt evidence");

        let _ = ivaldi::recover::recover(dir.path(), false);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"actual corrupt evidence"
        );
        verify_full_ok(dir.path());
    }
}

#[test]
fn gc_crash_deletes_only_dead_objects_and_retry_finishes() {
    for failpoint in ["gc.after_object_remove", "gc.after_shard_cleanup"] {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let live = B3Hash::digest(b"live-object");
        cas.put(live, b"live-object").unwrap();
        let dead = [
            b"dead-one".as_slice(),
            b"dead-two".as_slice(),
            b"dead-three".as_slice(),
        ]
        .map(|data| {
            let hash = B3Hash::digest(data);
            cas.put(hash, data).unwrap();
            hash
        });

        let output = child(dir.path(), "gc", failpoint);
        assert_aborted(&output, failpoint);
        assert!(
            cas.has(live).unwrap(),
            "GC removed a reachable object at {failpoint}"
        );
        run_gc(dir.path());
        assert!(cas.has(live).unwrap());
        assert!(dead.into_iter().all(|hash| !cas.has(hash).unwrap()));
    }
}

#[test]
fn pack_and_index_publication_are_atomic_and_idempotent() {
    let expected = [
        b"alpha alpha alpha".as_slice(),
        b"alpha alpha beta".as_slice(),
        b"completely different".as_slice(),
    ];
    for failpoint in ["pack.after_pack_publish", "pack.after_index_publish"] {
        let dir = tempfile::tempdir().unwrap();
        let output = child(dir.path(), "pack", failpoint);
        assert_aborted(&output, failpoint);

        // The pack is already a complete self-indexed object. A missing sidecar
        // index is recoverable by the idempotent retry; no truncated pack is
        // ever visible because publication uses atomic replacement.
        let reader = ivaldi::pack::PackReader::new(dir.path());
        assert_eq!(reader.list_packs().len(), 1);
        for data in expected {
            assert_eq!(reader.get_object(B3Hash::digest(data)).unwrap(), data);
        }
        write_pack(dir.path());
        assert_eq!(reader.list_packs().len(), 1);
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "idx"))
                .count(),
            1
        );
    }
}

#[test]
fn interrupted_pack_receive_exposes_only_verified_objects_and_retry_completes() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = dir.path().join("packs");
    write_pack(&pack_dir);
    let expected = [
        b"alpha alpha alpha".as_slice(),
        b"alpha alpha beta".as_slice(),
        b"completely different".as_slice(),
    ];

    let output = child(dir.path(), "pack-extract", "pack.after_object_extract");
    assert_aborted(&output, "pack.after_object_extract");
    let cas = FileCas::new(dir.path().join("objects")).unwrap();
    let landed = expected
        .iter()
        .filter(|data| cas.has(B3Hash::digest(data)).unwrap())
        .count();
    assert_eq!(
        landed, 1,
        "failpoint must kill after exactly one verified CAS put"
    );

    extract_pack(dir.path());
    extract_pack(dir.path());
    for data in expected {
        let hash = B3Hash::digest(data);
        assert_eq!(cas.get(hash).unwrap(), data);
    }
}
