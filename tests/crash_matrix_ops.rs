//! Crash-consistency matrix for the multi-step mutation commands: fuse,
//! weld, undo/pluck, timeline switch (auto-shelving + journal recovery),
//! rewind/reverse, reseal, and gather.
//!
//! Same harness as `tests/crash_matrix.rs`: each case runs a real `ivaldi`
//! child with `IVALDI_FAILPOINT=<name>` (abort at that boundary, simulating
//! power loss), then reopens the repository and asserts:
//!
//! - `verify --full` passes (or the documented recovery error appears),
//! - the operation is old-or-new, never partially visible,
//! - retrying (or the documented recovery command) converges,
//! - no shelved or staged content is lost across the crash.
#![cfg(feature = "failpoints")]

use std::path::Path;
use std::process::{Command, Output};

use ivaldi::repo::Repo;
use ivaldi::workspace::StagingArea;

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

fn history_len(dir: &Path, timeline: &str) -> usize {
    Repo::open(dir)
        .expect("repository must reopen")
        .walk_history(timeline)
        .expect("walk history")
        .len()
}

/// Seal name of the leaf at MMR index `idx`.
fn seal_name_of(dir: &Path, idx: u64) -> String {
    let repo = Repo::open(dir).expect("repository must reopen");
    let leaf = repo.get_leaf(idx).expect("get leaf").expect("leaf exists");
    ivaldi::seal::generate_seal_name(leaf.hash())
}

fn staged_paths(dir: &Path) -> Vec<String> {
    StagingArea::load(&dir.join(".ivaldi"))
        .staged_files()
        .keys()
        .cloned()
        .collect()
}

fn read_file(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
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

fn seal_file(dir: &Path, name: &str, content: &str, message: &str) {
    std::fs::write(dir.join(name), content).unwrap();
    ivaldi_ok(dir, &["gather", name]);
    ivaldi_ok(dir, &["seal", message]);
}

// ---------------------------------------------------------------------------
// fuse
// ---------------------------------------------------------------------------

/// Diverged-but-mergeable pair: `feature` adds feature.txt, `main` adds
/// main.txt after the fork. Returns the repo positioned on `main`.
fn setup_fuse_repo() -> tempfile::TempDir {
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    seal_file(dir.path(), "feature.txt", "from feature\n", "feature work");
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    seal_file(dir.path(), "main.txt", "from main\n", "main work");
    dir
}

#[test]
fn fuse_crash_before_commit_leaves_old_state_and_retry_merges() {
    let dir = setup_fuse_repo();
    let before = commit_count(dir.path());
    let head_before = timeline_head(dir.path(), "main");

    let output = ivaldi(
        dir.path(),
        Some("fuse.before_commit"),
        &["fuse", "feature", "to", "main"],
    );
    assert_aborted(&output, "fuse.before_commit");

    verify_full_ok(dir.path(), "fuse.before_commit");
    assert_eq!(commit_count(dir.path()), before, "merge must be invisible");
    assert_eq!(timeline_head(dir.path(), "main"), head_before);

    // Retry completes the merge and materializes the merged tree.
    ivaldi_ok(dir.path(), &["fuse", "feature", "to", "main"]);
    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(read_file(dir.path(), "feature.txt"), "from feature\n");
    assert_eq!(read_file(dir.path(), "main.txt"), "from main\n");
    verify_full_ok(dir.path(), "fuse retry");
}

#[test]
fn fuse_crash_after_commit_is_fully_visible_and_reverse_rematerializes() {
    let dir = setup_fuse_repo();
    let before = commit_count(dir.path());

    let output = ivaldi(
        dir.path(),
        Some("fuse.after_commit"),
        &["fuse", "feature", "to", "main"],
    );
    assert_aborted(&output, "fuse.after_commit");

    // The merge seal is durable and verify --full passes: the merged tree
    // was flushed to the CAS before the commit record referenced it.
    verify_full_ok(dir.path(), "fuse.after_commit");
    assert_eq!(commit_count(dir.path()), before + 1);
    let repo = Repo::open(dir.path()).unwrap();
    let head = repo.get_timeline_head("main").unwrap().unwrap();
    let head_leaf = repo.get_leaf(head).unwrap().unwrap();
    assert!(head_leaf.is_merge(), "head must be the merge seal");
    drop(repo);

    // Documented window: the working tree still shows pre-merge content.
    assert_eq!(read_file(dir.path(), "feature.txt"), "");

    // Retrying is a clean no-op (source already fused), not a second merge.
    let retry = ivaldi_ok(dir.path(), &["fuse", "feature", "to", "main"]);
    assert!(
        String::from_utf8_lossy(&retry.stdout).contains("already fused"),
        "retry after a landed merge must be a no-op"
    );
    assert_eq!(commit_count(dir.path()), before + 1);

    // `reverse` restores the working tree from the merge seal.
    ivaldi_ok(dir.path(), &["reverse", "--all"]);
    assert_eq!(read_file(dir.path(), "feature.txt"), "from feature\n");
    verify_full_ok(dir.path(), "post-reverse");
}

#[test]
fn fuse_conflict_crash_after_merge_state_blocks_mutations_until_abort() {
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    seal_file(dir.path(), "file.txt", "feature side\n", "feature edit");
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    seal_file(dir.path(), "file.txt", "main side\n", "main edit");
    let before = commit_count(dir.path());

    let output = ivaldi(
        dir.path(),
        Some("fuse.after_merge_state"),
        &["fuse", "feature", "to", "main"],
    );
    assert_aborted(&output, "fuse.after_merge_state");

    verify_full_ok(dir.path(), "fuse.after_merge_state");
    assert_eq!(commit_count(dir.path()), before, "nothing was committed");

    // The persisted merge state gates history-rewriting commands.
    let reseal = ivaldi(dir.path(), None, &["reseal", "-m", "nope"]);
    assert!(!reseal.status.success());
    assert!(
        String::from_utf8_lossy(&reseal.stderr).contains("during a merge"),
        "reseal must refuse while a merge is in progress"
    );

    // --abort clears the state and the repo is fully usable again.
    ivaldi_ok(dir.path(), &["fuse", "--abort"]);
    seal_file(dir.path(), "after.txt", "ok\n", "after abort");
    verify_full_ok(dir.path(), "post-abort seal");
}

// ---------------------------------------------------------------------------
// undo / pluck (pick)
// ---------------------------------------------------------------------------

/// Three seals touching distinct files so undoing the middle one is
/// conflict-free. Returns the middle seal's name.
fn setup_pick_repo() -> (tempfile::TempDir, String) {
    let dir = setup_repo();
    seal_file(dir.path(), "b.txt", "b\n", "add b");
    seal_file(dir.path(), "c.txt", "c\n", "add c");
    let middle = seal_name_of(dir.path(), 1);
    (dir, middle)
}

#[test]
fn undo_crash_before_commit_leaves_old_state_and_retry_succeeds() {
    let (dir, middle) = setup_pick_repo();
    let before = commit_count(dir.path());

    let output = ivaldi(dir.path(), Some("pick.before_commit"), &["undo", &middle]);
    assert_aborted(&output, "pick.before_commit");

    verify_full_ok(dir.path(), "pick.before_commit");
    assert_eq!(commit_count(dir.path()), before);
    assert_eq!(timeline_head(dir.path(), "main"), Some(before - 1));

    ivaldi_ok(dir.path(), &["undo", &middle]);
    assert_eq!(commit_count(dir.path()), before + 1);
    assert!(
        !dir.path().join("b.txt").exists(),
        "retried undo must remove the undone file from the working tree"
    );
    verify_full_ok(dir.path(), "undo retry");
}

#[test]
fn undo_crash_after_commit_is_visible_idempotent_and_reverse_restores() {
    let (dir, middle) = setup_pick_repo();
    let before = commit_count(dir.path());

    let output = ivaldi(dir.path(), Some("pick.after_commit"), &["undo", &middle]);
    assert_aborted(&output, "pick.after_commit");

    // The undo seal is durable and its tree was flushed before the commit.
    verify_full_ok(dir.path(), "pick.after_commit");
    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(timeline_head(dir.path(), "main"), Some(before));

    // Documented window: the working tree was not yet rewritten.
    assert!(dir.path().join("b.txt").exists());

    // Retrying the same undo is a no-op (three-way apply sees no delta).
    ivaldi_ok(dir.path(), &["undo", &middle]);
    assert_eq!(commit_count(dir.path()), before + 1);

    // `reverse` brings the working tree up to the undo seal.
    ivaldi_ok(dir.path(), &["reverse", "--all"]);
    assert!(!dir.path().join("b.txt").exists());
    verify_full_ok(dir.path(), "post-reverse");
}

// ---------------------------------------------------------------------------
// weld
// ---------------------------------------------------------------------------

/// Four seals (indices 0..=3) so a middle-range weld (1..=2) leaves one
/// trailing seal (3) that must be replayed.
fn setup_weld_repo() -> tempfile::TempDir {
    let dir = setup_repo();
    seal_file(dir.path(), "b.txt", "b\n", "add b");
    seal_file(dir.path(), "c.txt", "c\n", "add c");
    seal_file(dir.path(), "d.txt", "d\n", "add d");
    dir
}

#[test]
fn weld_crash_windows_are_old_or_new_never_a_truncated_chain() {
    // The welded seal plus every replayed trailing seal land in ONE store
    // transaction: a crash on either side of it leaves either the original
    // chain fully intact or the rewritten chain fully intact — never a head
    // with the trailing seals silently orphaned.
    for failpoint in [
        "weld.before_commit",
        "store.commit_leaves.before_commit",
        "store.commit_leaves.after_commit",
        "weld.after_commit",
    ] {
        let dir = setup_weld_repo();
        let start = seal_name_of(dir.path(), 1);
        let end = seal_name_of(dir.path(), 2);

        let output = ivaldi(
            dir.path(),
            Some(failpoint),
            &["weld", &start, "to", &end, "-m", "welded"],
        );
        assert_aborted(&output, failpoint);
        verify_full_ok(dir.path(), failpoint);

        let old_state = matches!(
            failpoint,
            "weld.before_commit" | "store.commit_leaves.before_commit"
        );
        if old_state {
            assert_eq!(commit_count(dir.path()), 4, "{failpoint}: old state");
            assert_eq!(timeline_head(dir.path(), "main"), Some(3));
            assert_eq!(history_len(dir.path(), "main"), 4);

            // Retry converges to the rewritten chain.
            ivaldi_ok(dir.path(), &["weld", &start, "to", &end, "-m", "welded"]);
        } else {
            assert_eq!(
                commit_count(dir.path()),
                6,
                "{failpoint}: welded + replayed trailing seal must both be visible"
            );
        }

        // Rewritten chain: baseline -> welded -> replayed trailing seal.
        assert_eq!(commit_count(dir.path()), 6);
        assert_eq!(timeline_head(dir.path(), "main"), Some(5));
        assert_eq!(history_len(dir.path(), "main"), 3);
        let repo = Repo::open(dir.path()).unwrap();
        let replayed = repo.get_leaf(5).unwrap().unwrap();
        assert_eq!(
            replayed.message, "add d",
            "trailing seal must survive the weld as a replay on the new head"
        );
        drop(repo);
        verify_full_ok(dir.path(), "weld converged");
    }
}

// ---------------------------------------------------------------------------
// timeline switch (auto-shelve + journal)
// ---------------------------------------------------------------------------

/// Repo with a `feature` timeline and dirty state on `main`:
/// - staged.txt gathered at "staged-v1\n", then rewritten to "worktree-v2\n"
///   (staged snapshot diverges from the worktree — the lossiest case),
/// - untracked.txt never gathered.
fn setup_switch_repo() -> tempfile::TempDir {
    let dir = setup_repo();
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);

    std::fs::write(dir.path().join("staged.txt"), "staged-v1\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "staged.txt"]);
    std::fs::write(dir.path().join("staged.txt"), "worktree-v2\n").unwrap();
    std::fs::write(dir.path().join("untracked.txt"), "u\n").unwrap();
    dir
}

/// After recovery plus a switch back to main, none of main's dirty state may
/// have been lost.
fn assert_main_dirty_state_intact(dir: &Path) {
    assert_eq!(Repo::open(dir).unwrap().current_timeline().unwrap(), "main");
    assert_eq!(read_file(dir, "staged.txt"), "worktree-v2\n");
    assert_eq!(read_file(dir, "untracked.txt"), "u\n");
    assert_eq!(
        staged_paths(dir),
        vec!["staged.txt".to_string()],
        "the staged entry must survive the crash and recovery"
    );
}

#[test]
fn switch_crash_before_journal_is_old_state_and_retry_converges() {
    // Crash after the shelf write but before the journal: no recovery state
    // exists yet, the worktree and staging are untouched, and a plain retry
    // re-captures and completes.
    let dir = setup_switch_repo();
    let output = ivaldi(
        dir.path(),
        Some("switch.after_shelf_save"),
        &["timeline", "switch", "feature"],
    );
    assert_aborted(&output, "switch.after_shelf_save");

    verify_full_ok(dir.path(), "switch.after_shelf_save");
    assert!(
        !dir.path().join(".ivaldi/SWITCH_IN_PROGRESS").exists(),
        "journal must not exist yet"
    );
    assert_eq!(
        Repo::open(dir.path()).unwrap().current_timeline().unwrap(),
        "main"
    );
    // Worktree and staging untouched by the crash.
    assert_eq!(read_file(dir.path(), "staged.txt"), "worktree-v2\n");
    assert_eq!(staged_paths(dir.path()), vec!["staged.txt".to_string()]);

    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_main_dirty_state_intact(dir.path());
    verify_full_ok(dir.path(), "switch retry roundtrip");
}

#[test]
fn switch_crash_after_journal_blocks_mutations_and_resume_preserves_state() {
    for failpoint in [
        "switch.after_journal",
        "switch.after_materialize",
        "switch.before_journal_clear",
    ] {
        let dir = setup_switch_repo();
        let output = ivaldi(
            dir.path(),
            Some(failpoint),
            &["timeline", "switch", "feature"],
        );
        assert_aborted(&output, failpoint);
        verify_full_ok(dir.path(), failpoint);

        // While the journal exists every other mutating command refuses
        // with the documented recovery instructions.
        let blocked = ivaldi(dir.path(), None, &["seal", "must refuse"]);
        assert!(!blocked.status.success(), "{failpoint}: seal must refuse");
        assert!(
            String::from_utf8_lossy(&blocked.stderr).contains("interrupted timeline switch"),
            "{failpoint}: recovery message expected\nstderr:\n{}",
            String::from_utf8_lossy(&blocked.stderr)
        );

        // Completing the switch, then coming back, restores everything.
        ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
        assert!(!dir.path().join(".ivaldi/SWITCH_IN_PROGRESS").exists());
        ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
        assert_main_dirty_state_intact(dir.path());
        verify_full_ok(dir.path(), "switch resume roundtrip");
    }
}

#[test]
fn switch_repeated_crashes_at_the_journal_converge() {
    // Crash the same interrupted switch resume repeatedly; state must stay
    // recoverable each time and the final resume lands intact.
    let dir = setup_switch_repo();
    let output = ivaldi(
        dir.path(),
        Some("switch.after_journal"),
        &["timeline", "switch", "feature"],
    );
    assert_aborted(&output, "switch.after_journal");

    for _ in 0..2 {
        let again = ivaldi(
            dir.path(),
            Some("switch.after_materialize"),
            &["timeline", "switch", "feature"],
        );
        assert_aborted(&again, "switch.after_materialize");
        verify_full_ok(dir.path(), "repeated switch crash");
    }

    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_main_dirty_state_intact(dir.path());
    verify_full_ok(dir.path(), "converged after repeated crashes");
}

#[test]
fn switch_crash_before_shelf_removal_leaves_only_a_harmless_leftover() {
    // Round-trip: shelve main's dirty state onto `feature`, then crash the
    // switch back to main AFTER the journal clear but BEFORE the restored
    // shelf is removed. The restored staging/worktree must already be
    // complete (the shelf is only removed once it is no longer needed).
    let dir = setup_switch_repo();
    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);

    let output = ivaldi(
        dir.path(),
        Some("switch.before_shelf_remove"),
        &["timeline", "switch", "main"],
    );
    assert_aborted(&output, "switch.before_shelf_remove");

    verify_full_ok(dir.path(), "switch.before_shelf_remove");
    assert!(
        !dir.path().join(".ivaldi/SWITCH_IN_PROGRESS").exists(),
        "journal was already cleared"
    );
    assert_main_dirty_state_intact(dir.path());
    assert!(
        dir.path().join(".ivaldi/shelves/main.shelf").exists(),
        "the leftover shelf is the only residue of this crash window"
    );

    // The leftover shelf must not double-apply or corrupt a later cycle.
    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_main_dirty_state_intact(dir.path());
    verify_full_ok(dir.path(), "post-leftover roundtrip");
}

// ---------------------------------------------------------------------------
// reseal
// ---------------------------------------------------------------------------

#[test]
fn reseal_crash_after_commit_is_visible_and_retry_is_harmless() {
    let dir = setup_repo();
    std::fs::write(dir.path().join("extra.txt"), "extra\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "extra.txt"]);

    let output = ivaldi(
        dir.path(),
        Some("reseal.after_commit"),
        &["reseal", "-m", "fixed"],
    );
    assert_aborted(&output, "reseal.after_commit");

    verify_full_ok(dir.path(), "reseal.after_commit");
    // Replacement, not addition: the timeline still has one seal (the old
    // head is orphaned in the MMR), and it already contains the change.
    assert_eq!(history_len(dir.path(), "main"), 1);
    assert_eq!(commit_count(dir.path()), 2);
    // Documented window: staging was not yet cleared.
    assert_eq!(staged_paths(dir.path()), vec!["extra.txt".to_string()]);

    // Retry folds the identical delta onto the resealed head: same content,
    // one more orphaned leaf, staging finally cleared.
    ivaldi_ok(dir.path(), &["reseal", "-m", "fixed"]);
    assert_eq!(history_len(dir.path(), "main"), 1);
    assert!(staged_paths(dir.path()).is_empty());
    let repo = Repo::open(dir.path()).unwrap();
    let head = repo.get_timeline_head("main").unwrap().unwrap();
    let leaf = repo.get_leaf(head).unwrap().unwrap();
    assert_eq!(leaf.message, "fixed");
    drop(repo);
    verify_full_ok(dir.path(), "reseal retry");
}

#[test]
fn reseal_crash_before_transaction_leaves_old_state() {
    // The same store-transaction failpoints that protect `seal` also cover
    // the reseal path (both funnel through commit_raw).
    let dir = setup_repo();
    std::fs::write(dir.path().join("extra.txt"), "extra\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "extra.txt"]);

    let output = ivaldi(
        dir.path(),
        Some("store.commit_leaf.before_commit"),
        &["reseal", "-m", "fixed"],
    );
    assert_aborted(&output, "store.commit_leaf.before_commit");

    verify_full_ok(dir.path(), "reseal before txn");
    assert_eq!(commit_count(dir.path()), 1, "reseal must be invisible");
    assert_eq!(
        Repo::open(dir.path())
            .unwrap()
            .get_leaf(0)
            .unwrap()
            .unwrap()
            .message,
        "baseline"
    );
    // Staging intact — retry succeeds.
    assert_eq!(staged_paths(dir.path()), vec!["extra.txt".to_string()]);
    ivaldi_ok(dir.path(), &["reseal", "-m", "fixed"]);
    verify_full_ok(dir.path(), "reseal retry after old-state crash");
}

// ---------------------------------------------------------------------------
// rewind / reverse
// ---------------------------------------------------------------------------

#[test]
fn rewind_crash_after_head_move_converges_on_retry() {
    let dir = setup_repo();
    seal_file(dir.path(), "file.txt", "v2\n", "second");
    seal_file(dir.path(), "file.txt", "v3\n", "third");
    let target = seal_name_of(dir.path(), 0);

    // Stale staging gathered against the old head — the hazard rewind must
    // clear even when the head move already happened before the crash.
    std::fs::write(dir.path().join("stale.txt"), "stale\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "stale.txt"]);

    let output = ivaldi(
        dir.path(),
        Some("rewind.after_head"),
        &["rewind", &target, "--discard"],
    );
    assert_aborted(&output, "rewind.after_head");

    verify_full_ok(dir.path(), "rewind.after_head");
    assert_eq!(timeline_head(dir.path(), "main"), Some(0), "head moved");

    // Retrying the same rewind must finish the interrupted steps (clear
    // staging, materialize) even though the head is already at the target.
    ivaldi_ok(dir.path(), &["rewind", &target, "--discard"]);
    assert!(staged_paths(dir.path()).is_empty(), "stale staging cleared");
    assert_eq!(read_file(dir.path(), "file.txt"), "v1\n");
    verify_full_ok(dir.path(), "rewind retry");
}

#[test]
fn rewind_crash_before_materialize_converges_on_retry() {
    let dir = setup_repo();
    seal_file(dir.path(), "file.txt", "v2\n", "second");
    let target = seal_name_of(dir.path(), 0);

    let output = ivaldi(
        dir.path(),
        Some("rewind.after_staging_clear"),
        &["rewind", &target, "--discard"],
    );
    assert_aborted(&output, "rewind.after_staging_clear");

    verify_full_ok(dir.path(), "rewind.after_staging_clear");
    assert_eq!(timeline_head(dir.path(), "main"), Some(0));
    // Documented window: the worktree is not yet rewound.
    assert_eq!(read_file(dir.path(), "file.txt"), "v2\n");

    ivaldi_ok(dir.path(), &["rewind", &target, "--discard"]);
    assert_eq!(read_file(dir.path(), "file.txt"), "v1\n");
    verify_full_ok(dir.path(), "rewind materialize retry");
}

#[test]
fn reverse_crash_between_materialize_and_staging_clear_converges() {
    let dir = setup_repo();
    std::fs::write(dir.path().join("file.txt"), "dirty\n").unwrap();
    ivaldi_ok(dir.path(), &["gather", "file.txt"]);
    std::fs::write(dir.path().join("file.txt"), "dirtier\n").unwrap();

    let output = ivaldi(
        dir.path(),
        Some("reverse.after_materialize"),
        &["reverse", "--all"],
    );
    assert_aborted(&output, "reverse.after_materialize");

    verify_full_ok(dir.path(), "reverse.after_materialize");
    // Worktree restored, staging still stale — the documented window.
    assert_eq!(read_file(dir.path(), "file.txt"), "v1\n");
    assert_eq!(staged_paths(dir.path()), vec!["file.txt".to_string()]);

    // Retry re-runs both steps and converges.
    ivaldi_ok(dir.path(), &["reverse", "--all"]);
    assert_eq!(read_file(dir.path(), "file.txt"), "v1\n");
    assert!(staged_paths(dir.path()).is_empty());
    verify_full_ok(dir.path(), "reverse retry");
}

// ---------------------------------------------------------------------------
// gather
// ---------------------------------------------------------------------------

#[test]
fn gather_crash_around_the_staging_save_is_old_or_new() {
    // Before the atomic staging save: nothing staged, retry stages all.
    let dir = setup_repo();
    std::fs::write(dir.path().join("g1.txt"), "one\n").unwrap();
    std::fs::write(dir.path().join("g2.txt"), "two\n").unwrap();

    let output = ivaldi(
        dir.path(),
        Some("gather.before_stage_save"),
        &["gather", "."],
    );
    assert_aborted(&output, "gather.before_stage_save");
    verify_full_ok(dir.path(), "gather.before_stage_save");
    assert!(
        staged_paths(dir.path()).is_empty(),
        "crash before the staging save must leave nothing staged"
    );

    ivaldi_ok(dir.path(), &["gather", "."]);
    let mut staged = staged_paths(dir.path());
    staged.sort();
    assert_eq!(staged, vec!["g1.txt".to_string(), "g2.txt".to_string()]);

    // After the save: fully staged, seal succeeds directly.
    let output = ivaldi(
        dir.path(),
        Some("gather.after_stage_save"),
        &["gather", "."],
    );
    assert_aborted(&output, "gather.after_stage_save");
    verify_full_ok(dir.path(), "gather.after_stage_save");
    let mut staged = staged_paths(dir.path());
    staged.sort();
    assert_eq!(staged, vec!["g1.txt".to_string(), "g2.txt".to_string()]);

    ivaldi_ok(dir.path(), &["seal", "both files"]);
    assert_eq!(commit_count(dir.path()), 2);
    verify_full_ok(dir.path(), "seal after gather crash");
}
