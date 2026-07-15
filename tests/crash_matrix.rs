//! Deterministic crash-consistency matrix.
//!
//! Each test runs a real `ivaldi` child process with `IVALDI_FAILPOINT=<name>`
//! set, which aborts the process (no cleanup, simulating power loss) at that
//! mutation boundary — see `src/failpoint.rs`. The test then reopens the
//! repository and asserts the old-or-new atomicity contract:
//!
//! - the repository opens and `verify --full` passes,
//! - the operation is either entirely invisible or entirely visible,
//! - the previous head is still reachable when the operation did not commit,
//! - retrying the operation is safe,
//! - the crashed process's repo lock does not block the retry.
//!
//! Requires the `failpoints` feature (CI runs tests with `--all-features`).
#![cfg(feature = "failpoints")]

use std::path::Path;
use std::process::{Command, Output};

use ivaldi::repo::Repo;

fn ivaldi(dir: &Path, failpoint: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ivaldi"));
    cmd.current_dir(dir).env("NO_COLOR", "1").args(args);
    match failpoint {
        Some(fp) => cmd.env("IVALDI_FAILPOINT", fp),
        None => cmd.env_remove("IVALDI_FAILPOINT"),
    };
    cmd.output().expect("run ivaldi binary")
}

fn ivaldi_ok(dir: &Path, args: &[&str]) -> Output {
    let output = ivaldi(dir, None, args);
    assert!(
        output.status.success(),
        "ivaldi {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

/// The child must have died at the failpoint, not exited cleanly.
fn assert_aborted(output: &Output, failpoint: &str) {
    assert!(
        !output.status.success(),
        "expected abort at failpoint {failpoint}, but the command succeeded"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!("failpoint hit: {failpoint}")),
        "failpoint {failpoint} was never reached\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_full_ok(dir: &Path, context: &str) {
    let output = ivaldi(dir, None, &["verify", "--full"]);
    assert!(
        output.status.success(),
        "verify --full failed after {context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn commit_count(dir: &Path) -> u64 {
    Repo::open(dir)
        .expect("repository must reopen")
        .commit_count()
}

fn timeline_head(dir: &Path, name: &str) -> Option<u64> {
    Repo::open(dir)
        .expect("repository must reopen")
        .get_timeline_head(name)
        .expect("read timeline head")
}

/// Fresh repository with identity configured and one sealed commit on `main`.
fn setup_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ivaldi_ok(dir.path(), &["forge"]);
    ivaldi_ok(dir.path(), &["config", "--set", "user.name", "Crash Test"]);
    ivaldi_ok(
        dir.path(),
        &["config", "--set", "user.email", "crash@example.com"],
    );
    std::fs::write(dir.path().join("file.txt"), "v1\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "baseline"]);
    dir
}

/// Stage a change and run `seal` with the given failpoint armed.
fn seal_with_failpoint(dir: &Path, failpoint: &str) -> Output {
    std::fs::write(dir.join("file.txt"), format!("crash {failpoint}\n")).unwrap();
    ivaldi_ok(dir, &["gather", "."]);
    ivaldi(dir, Some(failpoint), &["seal", "crashing seal"])
}

#[test]
fn seal_crash_before_transaction_leaves_old_state_and_retry_succeeds() {
    for failpoint in [
        "commit.before_marker",
        "commit.after_marker",
        "store.commit_leaf.before_commit",
    ] {
        let dir = setup_repo();
        let output = seal_with_failpoint(dir.path(), failpoint);
        assert_aborted(&output, failpoint);

        verify_full_ok(dir.path(), failpoint);
        assert_eq!(
            commit_count(dir.path()),
            1,
            "crash at {failpoint} must leave the new seal entirely invisible"
        );
        assert_eq!(timeline_head(dir.path(), "main"), Some(0));

        // Recovery is "do nothing": the crashed process's lock died with it,
        // and retrying the same operation must succeed cleanly.
        ivaldi_ok(dir.path(), &["gather", "."]);
        ivaldi_ok(dir.path(), &["seal", "retry"]);
        assert_eq!(commit_count(dir.path()), 2);
        assert_eq!(timeline_head(dir.path(), "main"), Some(1));
        verify_full_ok(dir.path(), "retry");
    }
}

#[test]
fn seal_crash_after_transaction_leaves_new_state_fully_visible() {
    for failpoint in ["store.commit_leaf.after_commit", "commit.after_txn"] {
        let dir = setup_repo();
        let output = seal_with_failpoint(dir.path(), failpoint);
        assert_aborted(&output, failpoint);

        verify_full_ok(dir.path(), failpoint);
        assert_eq!(
            commit_count(dir.path()),
            2,
            "crash at {failpoint} happened after durable commit; the seal must be visible"
        );
        assert_eq!(timeline_head(dir.path(), "main"), Some(1));

        // The repository stays fully usable for new work.
        std::fs::write(dir.path().join("file.txt"), "post-crash\n").unwrap();
        ivaldi_ok(dir.path(), &["gather", "."]);
        ivaldi_ok(dir.path(), &["seal", "after crash"]);
        assert_eq!(commit_count(dir.path()), 3);
        verify_full_ok(dir.path(), "post-crash seal");
    }
}

#[test]
fn seal_crash_inside_atomic_write_is_old_or_new_never_partial() {
    for failpoint in ["atomic_write.before_rename", "atomic_write.after_rename"] {
        let dir = setup_repo();
        let output = seal_with_failpoint(dir.path(), failpoint);
        assert_aborted(&output, failpoint);

        verify_full_ok(dir.path(), failpoint);
        let count = commit_count(dir.path());
        assert!(
            count == 1 || count == 2,
            "crash at {failpoint} left {count} commits; must be old (1) or new (2)"
        );

        // The crash may have landed the seal (post-commit atomic_write), so
        // stage fresh content for the follow-up write.
        std::fs::write(dir.path().join("file.txt"), format!("retry {failpoint}\n")).unwrap();
        ivaldi_ok(dir.path(), &["gather", "."]);
        ivaldi_ok(dir.path(), &["seal", "retry"]);
        verify_full_ok(dir.path(), "retry");
    }
}

#[test]
fn switch_crash_before_head_write_keeps_current_timeline() {
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);

    let output = ivaldi(
        dir.path(),
        Some("head.before_write"),
        &["timeline", "switch", "feature"],
    );
    assert_aborted(&output, "head.before_write");

    verify_full_ok(dir.path(), "head.before_write");
    let repo = Repo::open(dir.path()).unwrap();
    assert_eq!(repo.current_timeline().unwrap(), "main");
    drop(repo);

    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
    assert_eq!(
        Repo::open(dir.path()).unwrap().current_timeline().unwrap(),
        "feature"
    );
}

#[test]
fn timeline_remove_crash_leaves_harmless_marker_and_retry_finishes() {
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);

    let output = ivaldi(
        dir.path(),
        Some("timeline.remove.after_head"),
        &["timeline", "remove", "feature"],
    );
    assert_aborted(&output, "timeline.remove.after_head");

    // Stored head is gone; only the harmless empty marker may remain.
    verify_full_ok(dir.path(), "timeline.remove.after_head");
    assert_eq!(timeline_head(dir.path(), "feature"), None);
    assert!(
        dir.path().join(".ivaldi/refs/heads/feature").exists(),
        "crash between head removal and marker removal must leave the marker"
    );

    ivaldi_ok(dir.path(), &["timeline", "remove", "feature"]);
    assert!(!dir.path().join(".ivaldi/refs/heads/feature").exists());
    verify_full_ok(dir.path(), "remove retry");
}

#[test]
fn timeline_rename_crash_windows_are_old_or_new() {
    // Before the store transaction: rename invisible, retry completes it.
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);

    let output = ivaldi(
        dir.path(),
        Some("timeline.rename.after_marker"),
        &["timeline", "rename", "feature", "renamed"],
    );
    assert_aborted(&output, "timeline.rename.after_marker");
    verify_full_ok(dir.path(), "timeline.rename.after_marker");
    assert!(timeline_head(dir.path(), "feature").is_some());
    assert_eq!(timeline_head(dir.path(), "renamed"), None);
    ivaldi_ok(dir.path(), &["timeline", "rename", "feature", "renamed"]);
    assert!(timeline_head(dir.path(), "renamed").is_some());
    verify_full_ok(dir.path(), "rename retry");

    // After the store transaction: rename fully visible; the leftover old
    // marker is a harmless headless timeline that plain `remove` cleans up.
    for failpoint in [
        "timeline.rename.after_store",
        "timeline.rename.before_old_marker",
    ] {
        let dir = setup_repo();
        ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
        ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);

        let output = ivaldi(
            dir.path(),
            Some(failpoint),
            &["timeline", "rename", "feature", "renamed"],
        );
        assert_aborted(&output, failpoint);
        verify_full_ok(dir.path(), failpoint);
        assert_eq!(timeline_head(dir.path(), "feature"), None);
        assert!(timeline_head(dir.path(), "renamed").is_some());

        ivaldi_ok(dir.path(), &["timeline", "remove", "feature"]);
        assert!(!dir.path().join(".ivaldi/refs/heads/feature").exists());
        verify_full_ok(dir.path(), "orphan marker cleanup");
    }
}

#[test]
fn crashed_writer_never_blocks_the_next_writer() {
    // The aborted child held the exclusive repo lock when it died; the OS
    // must have released it, so an immediate new writer succeeds.
    let dir = setup_repo();
    let output = seal_with_failpoint(dir.path(), "store.commit_leaf.before_commit");
    assert_aborted(&output, "store.commit_leaf.before_commit");

    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "lock released"]);
    assert_eq!(commit_count(dir.path()), 2);
    verify_full_ok(dir.path(), "post-crash lock reacquisition");
}

#[test]
fn crash_recovery_is_idempotent_across_repeated_crashes() {
    // Crash the same operation at the same point repeatedly: state must stay
    // identical old-state each time, and the final retry lands exactly once.
    let dir = setup_repo();
    for _ in 0..3 {
        let output = seal_with_failpoint(dir.path(), "store.commit_leaf.before_commit");
        assert_aborted(&output, "store.commit_leaf.before_commit");
        verify_full_ok(dir.path(), "repeated crash");
        assert_eq!(commit_count(dir.path()), 1);
        assert_eq!(timeline_head(dir.path(), "main"), Some(0));
    }
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "final"]);
    assert_eq!(commit_count(dir.path()), 2);
    verify_full_ok(dir.path(), "final retry");
}
