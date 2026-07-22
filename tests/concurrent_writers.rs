//! Process-level concurrent-writer safety.
//!
//! Spawns real `ivaldi` processes racing on the same repository and asserts
//! the exclusive repo lock serializes them: every writer either succeeds or
//! fails with a clean, actionable error — never corruption, never a partial
//! commit. Runs without any feature flags.

use std::path::Path;
use std::process::{Command, Output, Stdio};

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
    ivaldi_ok(dir.path(), &["config", "--set", "user.name", "Race Test"]);
    ivaldi_ok(
        dir.path(),
        &["config", "--set", "user.email", "race@example.com"],
    );
    std::fs::write(dir.path().join("file.txt"), "base\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "."]);
    ivaldi_ok(dir.path(), &["seal", "baseline"]);
    dir
}

fn verify_full_ok(dir: &Path) {
    ivaldi_ok(dir, &["verify", "--full"]);
}

/// A racing writer must either succeed or fail with one of the documented
/// clean errors; anything else (panic, corruption message) fails the test.
fn assert_clean_outcome(output: &Output) -> bool {
    if output.status.success() {
        return true;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("another ivaldi process")
            || stderr.contains("no changes staged")
            || stderr.contains("in use by another ivaldi process"),
        "racing writer failed with an unexpected error:\n{stderr}"
    );
    false
}

#[test]
fn two_processes_sealing_concurrently_never_corrupt_the_repository() {
    let dir = setup_repo();

    for round in 0..5 {
        let before = Repo::open(dir.path()).unwrap().commit_count();
        std::fs::write(dir.path().join("file.txt"), format!("round {round}\n")).unwrap();
        ivaldi_ok(dir.path(), &["gather", "."]);

        let spawn = |msg: &str| {
            Command::new(env!("CARGO_BIN_EXE_ivaldi"))
                .current_dir(dir.path())
                .env("NO_COLOR", "1")
                .args(["seal", msg])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn ivaldi seal")
        };
        let first = spawn("racer one");
        let second = spawn("racer two");
        let first = first.wait_with_output().unwrap();
        let second = second.wait_with_output().unwrap();

        let successes = assert_clean_outcome(&first) as u64 + assert_clean_outcome(&second) as u64;
        assert!(successes >= 1, "at least one racing seal must win");

        let after = Repo::open(dir.path()).unwrap().commit_count();
        assert_eq!(
            after,
            before + successes,
            "commit count must advance by exactly the number of successful seals"
        );
        verify_full_ok(dir.path());
    }
}

#[test]
fn two_processes_creating_the_same_timeline_race_cleanly() {
    let dir = setup_repo();

    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_ivaldi"))
            .current_dir(dir.path())
            .env("NO_COLOR", "1")
            .args(["timeline", "create", "contested"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ivaldi timeline create")
    };
    let first = spawn();
    let second = spawn();
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    // Each racer must succeed or fail cleanly (lock contention or "already
    // exists"); afterwards exactly one usable timeline exists and the
    // repository verifies.
    for output in [&first, &second] {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("another ivaldi process")
                    || stderr.contains("already exists")
                    || stderr.contains("in use by another ivaldi process"),
                "racing create failed with an unexpected error:\n{stderr}"
            );
        }
    }
    assert!(first.status.success() || second.status.success());
    verify_full_ok(dir.path());
}
