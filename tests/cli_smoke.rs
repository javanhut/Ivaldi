use std::process::{Command, Output};

fn ivaldi(current_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(current_dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("run ivaldi binary")
}

fn ivaldi_ok(current_dir: &std::path::Path, args: &[&str]) -> Output {
    let output = ivaldi(current_dir, args);
    assert!(
        output.status.success(),
        "ivaldi {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn forge_with_identity(dir: &std::path::Path) {
    ivaldi_ok(dir, &["forge"]);
    ivaldi_ok(dir, &["config", "--set", "user.name", "CLI Test"]);
    ivaldi_ok(
        dir,
        &["config", "--set", "user.email", "cli-test@example.com"],
    );
}

fn seal_all(dir: &std::path::Path, message: &str) {
    ivaldi_ok(dir, &["gather", "."]);
    ivaldi_ok(dir, &["seal", message]);
}

#[test]
fn forge_status_and_timeline_work_as_a_cli() {
    let dir = tempfile::tempdir().unwrap();

    let forged = ivaldi(dir.path(), &["forge"]);
    assert!(
        forged.status.success(),
        "{}",
        String::from_utf8_lossy(&forged.stderr)
    );
    assert!(dir.path().join(".ivaldi/HEAD").is_file());

    let status = ivaldi(dir.path(), &["status", "--json"]);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["timeline"], "main");

    let timeline = ivaldi(dir.path(), &["timeline", "list", "--json"]);
    assert!(
        timeline.status.success(),
        "{}",
        String::from_utf8_lossy(&timeline.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&timeline.stdout).unwrap();
    assert!(value.as_array().is_some_and(|entries| !entries.is_empty()));
}

#[test]
fn command_errors_have_a_nonzero_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let output = ivaldi(dir.path(), &["status"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not an Ivaldi repository"));
}

#[test]
fn malformed_head_is_reported_without_damaging_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    forge_with_identity(dir.path());

    let head = dir.path().join(".ivaldi/HEAD");
    std::fs::write(&head, [0xff, 0xfe]).unwrap();

    let create = ivaldi(dir.path(), &["timeline", "create", "feature"]);
    assert!(!create.status.success());
    let stderr = String::from_utf8_lossy(&create.stderr);
    assert!(stderr.contains("valid UTF-8"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");

    // Restoring the one damaged metadata file is enough to reopen the repo;
    // the failed read must not have mutated any other repository state.
    std::fs::write(&head, "ref: refs/heads/main\n").unwrap();
    let status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["timeline"], "main");

    let timelines = ivaldi_ok(dir.path(), &["timeline", "list", "--json"]);
    let timelines: serde_json::Value = serde_json::from_slice(&timelines.stdout).unwrap();
    assert_eq!(timelines.as_array().unwrap().len(), 1);
    assert_eq!(timelines[0]["name"], "main");
}

#[test]
fn dirty_work_is_shelved_and_restored_per_timeline() {
    let dir = tempfile::tempdir().unwrap();
    forge_with_identity(dir.path());

    std::fs::write(dir.path().join("story.txt"), "base\n").unwrap();
    seal_all(dir.path(), "base");

    // `timeline create` switches to the new timeline. Give it committed work
    // that must be rematerialized every time we return to it.
    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    std::fs::write(dir.path().join("story.txt"), "feature\n").unwrap();
    seal_all(dir.path(), "feature version");

    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("story.txt")).unwrap(),
        "base\n"
    );

    // This unsealed edit belongs to main. Switching away must preserve it
    // without leaking it into feature, and switching back must restore it.
    std::fs::write(dir.path().join("story.txt"), "unfinished main work\n").unwrap();
    ivaldi_ok(dir.path(), &["timeline", "switch", "feature"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("story.txt")).unwrap(),
        "feature\n"
    );

    let feature_status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let feature_status: serde_json::Value = serde_json::from_slice(&feature_status.stdout).unwrap();
    assert_eq!(feature_status["timeline"], "feature");
    assert_eq!(feature_status["files"], serde_json::json!([]));

    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("story.txt")).unwrap(),
        "unfinished main work\n"
    );

    let main_status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let main_status: serde_json::Value = serde_json::from_slice(&main_status.stdout).unwrap();
    assert_eq!(main_status["timeline"], "main");
    assert_eq!(main_status["files"][0]["path"], "story.txt");
    assert_eq!(main_status["files"][0]["state"], "modified");
}

#[test]
fn divergent_timelines_merge_and_persist_through_cli_processes() {
    let dir = tempfile::tempdir().unwrap();
    forge_with_identity(dir.path());

    std::fs::write(dir.path().join("base.txt"), "shared base\n").unwrap();
    seal_all(dir.path(), "base");

    ivaldi_ok(dir.path(), &["timeline", "create", "feature"]);
    std::fs::write(dir.path().join("feature.txt"), "from feature\n").unwrap();
    seal_all(dir.path(), "feature work");

    ivaldi_ok(dir.path(), &["timeline", "switch", "main"]);
    std::fs::write(dir.path().join("main.txt"), "from main\n").unwrap();
    seal_all(dir.path(), "main work");

    ivaldi_ok(dir.path(), &["fuse", "feature", "to", "main"]);

    assert_eq!(
        std::fs::read_to_string(dir.path().join("base.txt")).unwrap(),
        "shared base\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("feature.txt")).unwrap(),
        "from feature\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("main.txt")).unwrap(),
        "from main\n"
    );

    // Every call above opened the repository in a fresh process. Reopen it
    // again and verify both the clean workspace and the persisted merge DAG.
    let status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["timeline"], "main");
    assert_eq!(status["files"], serde_json::json!([]));

    let log = ivaldi_ok(dir.path(), &["log", "--format", "json"]);
    let log: serde_json::Value = serde_json::from_slice(&log.stdout).unwrap();
    let entries = log.as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["message"], "Fuse feature into main");
    assert_eq!(entries[0]["timeline"], "main");
    assert_eq!(entries[0]["is_merge"], true);
    assert!(
        entries
            .iter()
            .any(|entry| entry["message"] == "feature work")
    );
    assert!(entries.iter().any(|entry| entry["message"] == "main work"));
    assert!(entries.iter().any(|entry| entry["message"] == "base"));
}

#[test]
fn skip_excludes_paths_from_staging_until_unskip() {
    let dir = tempfile::tempdir().unwrap();
    forge_with_identity(dir.path());

    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    std::fs::write(dir.path().join("debug.log"), "base\n").unwrap();
    seal_all(dir.path(), "base");

    // Mark both a tracked file and a would-be test-output file as skipped.
    ivaldi_ok(dir.path(), &["skip", "a.txt", "debug.log"]);

    let list = ivaldi_ok(dir.path(), &["skip", "--list"]);
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("a.txt"), "{list_stdout}");
    assert!(list_stdout.contains("debug.log"), "{list_stdout}");

    // Bulk gather must not stage a skipped file, whether it was modified on
    // disk or (for a tracked file) appears missing from the filtered scan —
    // it must never turn into a staged deletion either.
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    std::fs::write(dir.path().join("debug.log"), "test output\n").unwrap();
    let gather = ivaldi_ok(dir.path(), &["gather", "."]);
    let gather_stdout = String::from_utf8_lossy(&gather.stdout);
    assert!(
        gather_stdout.contains("Nothing to gather"),
        "{gather_stdout}"
    );

    let status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["files"], serde_json::json!([]));
    assert_eq!(status["staged_deletions"], serde_json::json!([]));

    // With nothing staged, sealing must refuse.
    let seal = ivaldi(dir.path(), &["seal", "should fail"]);
    assert!(!seal.status.success());

    // Naming a skipped file explicitly warns and still does not stage it.
    let explicit = ivaldi_ok(dir.path(), &["gather", "a.txt"]);
    let explicit_stderr = String::from_utf8_lossy(&explicit.stderr);
    assert!(explicit_stderr.contains("skipped"), "{explicit_stderr}");
    let status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["files"], serde_json::json!([]));

    // Unskip restores normal staging behavior.
    ivaldi_ok(dir.path(), &["unskip", "a.txt"]);
    let gather = ivaldi_ok(dir.path(), &["gather", "."]);
    let gather_stdout = String::from_utf8_lossy(&gather.stdout);
    assert!(gather_stdout.contains("modified: a.txt"), "{gather_stdout}");
    ivaldi_ok(dir.path(), &["seal", "update a"]);

    let log = ivaldi_ok(dir.path(), &["log", "--format", "json"]);
    let log: serde_json::Value = serde_json::from_slice(&log.stdout).unwrap();
    assert_eq!(log.as_array().unwrap().len(), 2);

    // debug.log stayed skipped: it was never sealed into any tree.
    let status = ivaldi_ok(dir.path(), &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["files"], serde_json::json!([]));
}

/// A conflicted fuse must leave a resolvable state, and `--continue` must
/// finish it without dropping the side that merged cleanly.
#[test]
fn conflicted_fuse_is_reported_resolvable_and_completable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    forge_with_identity(path);

    std::fs::write(path.join("shared.txt"), "a\nb\nc\n").unwrap();
    std::fs::write(path.join("only_theirs.txt"), "base\n").unwrap();
    seal_all(path, "base");

    // Source side: rewrite line 2 of shared.txt, and change a file main never
    // touches — that second change must survive the merge.
    ivaldi_ok(path, &["timeline", "create", "feature"]);
    std::fs::write(path.join("shared.txt"), "a\nTHEIRS\nc\n").unwrap();
    std::fs::write(path.join("only_theirs.txt"), "changed by feature\n").unwrap();
    seal_all(path, "feature edit");

    // Target side: rewrite the same line differently.
    ivaldi_ok(path, &["timeline", "switch", "main"]);
    std::fs::write(path.join("shared.txt"), "a\nOURS\nc\n").unwrap();
    seal_all(path, "main edit");

    // With --markers the fuse stops, says so, and writes both sides into the
    // file for resolving by hand.
    let fuse = ivaldi(path, &["fuse", "feature", "--markers"]);
    assert!(!fuse.status.success(), "a conflicted fuse must not exit 0");
    let out = String::from_utf8_lossy(&fuse.stdout);
    assert!(out.contains("CONFLICT: shared.txt"), "{out}");
    let conflicted = std::fs::read_to_string(path.join("shared.txt")).unwrap();
    assert!(conflicted.contains("<<<<<<<"), "{conflicted}");
    assert!(
        conflicted.contains("OURS") && conflicted.contains("THEIRS"),
        "{conflicted}"
    );

    // Status must not call this clean.
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["merge"]["source_timeline"], "feature");
    assert_eq!(
        status["merge"]["conflicts"],
        serde_json::json!(["shared.txt"])
    );

    // Continuing before resolving is refused.
    let early = ivaldi(path, &["fuse", "--continue"]);
    assert!(!early.status.success());

    std::fs::write(path.join("shared.txt"), "a\nRESOLVED\nc\n").unwrap();
    ivaldi_ok(path, &["fuse", "--continue"]);

    // The resolution and the cleanly-merged file both landed, and the merge
    // state is gone.
    assert_eq!(
        std::fs::read_to_string(path.join("shared.txt")).unwrap(),
        "a\nRESOLVED\nc\n"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("only_theirs.txt")).unwrap(),
        "changed by feature\n"
    );
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["merge"].is_null(), "{status}");

    // The merge seal records both parents, which is what lets a diverged
    // upload fast-forward again.
    let log = ivaldi_ok(path, &["log", "--format", "json"]);
    let log: serde_json::Value = serde_json::from_slice(&log.stdout).unwrap();
    assert_eq!(log[0]["is_merge"], true, "{log}");
}

// ---------------------------------------------------------------------------
// Uncommitted work vs. commands that rewrite the working directory
// ---------------------------------------------------------------------------

/// Twenty numbered lines, with `edits` applied as (line number, replacement).
/// Long enough that edits at opposite ends merge without touching.
fn numbered(edits: &[(usize, &str)]) -> String {
    (1..=20)
        .map(|n| match edits.iter().find(|(line, _)| *line == n) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("line {n}\n"),
        })
        .collect()
}

fn read(path: &std::path::Path, file: &str) -> String {
    std::fs::read_to_string(path.join(file)).unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn last_seal_message(path: &std::path::Path) -> String {
    let log = ivaldi_ok(path, &["log", "--format", "json"]);
    let log: serde_json::Value = serde_json::from_slice(&log.stdout).unwrap();
    log[0]["message"].as_str().unwrap().to_string()
}

/// `main` has moved on in `a.txt` (line 2) and `b.txt`; the current timeline,
/// `feature`, is left with nothing sealed of its own.
fn diverged_with_feature_current(path: &std::path::Path) {
    forge_with_identity(path);
    std::fs::write(path.join("a.txt"), numbered(&[])).unwrap();
    std::fs::write(path.join("b.txt"), "base\n").unwrap();
    std::fs::write(path.join("doomed.txt"), "base\n").unwrap();
    seal_all(path, "base");

    ivaldi_ok(path, &["timeline", "create", "feature"]);
    ivaldi_ok(path, &["timeline", "switch", "main"]);
    std::fs::write(path.join("a.txt"), numbered(&[(2, "MAIN")])).unwrap();
    std::fs::write(path.join("b.txt"), "changed on main\n").unwrap();
    seal_all(path, "main edit");
    ivaldi_ok(path, &["timeline", "switch", "feature"]);
}

/// The shape that used to lose work: uncommitted edits on a timeline, then
/// `fuse main`. They must come out the other side merged onto the fused tree
/// and still uncommitted — modified, new and deleted files alike.
#[test]
fn fuse_carries_uncommitted_changes_through() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);

    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();
    std::fs::write(path.join("new.txt"), "untracked\n").unwrap();
    std::fs::remove_file(path.join("doomed.txt")).unwrap();

    let out = stdout(&ivaldi_ok(path, &["fuse", "main"]));
    assert!(out.contains("Carrying 3 uncommitted change(s)"), "{out}");
    assert!(out.contains("back on top"), "{out}");

    // Same file edited on both sides, in different places: both edits land.
    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MAIN"), (19, "MINE")]));
    assert_eq!(read(path, "b.txt"), "changed on main\n");
    assert_eq!(read(path, "new.txt"), "untracked\n");
    assert!(!path.join("doomed.txt").exists());

    // The fuse was sealed; the carried work was not.
    assert_eq!(last_seal_message(path), "Fuse main into feature");
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status = String::from_utf8_lossy(&status.stdout).into_owned();
    for file in ["a.txt", "new.txt", "doomed.txt"] {
        assert!(
            status.contains(file),
            "{file} should still be uncommitted: {status}"
        );
    }
    assert!(!status.contains("b.txt"), "{status}");
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
}

/// Where the carried edit and the fuse really collide, the file gets conflict
/// markers — but the fuse itself still succeeds and nothing is dropped.
#[test]
fn fuse_marks_a_carried_change_that_collides() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);

    std::fs::write(path.join("a.txt"), numbered(&[(2, "MINE")])).unwrap();

    let out = stdout(&ivaldi_ok(path, &["fuse", "main"]));
    assert!(out.contains("1 need a look"), "{out}");
    assert!(out.contains("a.txt"), "{out}");

    let merged = read(path, "a.txt");
    assert!(
        merged.contains("<<<<<<< your uncommitted changes"),
        "{merged}"
    );
    assert!(
        merged.contains("MINE") && merged.contains("MAIN"),
        "{merged}"
    );
    assert!(merged.contains(">>>>>>> fused from main"), "{merged}");
    assert_eq!(last_seal_message(path), "Fuse main into feature");
    // Markers in an uncommitted working file leave no merge open.
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["merge"].is_null(), "{status}");
}

/// Gathered entries name blobs made against the old tip; sealed after the
/// fuse they would silently undo what it brought in. They are un-gathered,
/// and their content carried like any other edit.
#[test]
fn fuse_ungathers_but_keeps_gathered_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);

    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();
    ivaldi_ok(path, &["gather", "a.txt"]);

    let out = stdout(&ivaldi_ok(path, &["fuse", "main"]));
    assert!(out.contains("un-gathered"), "{out}");
    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MAIN"), (19, "MINE")]));
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    let a = status["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "a.txt")
        .unwrap_or_else(|| panic!("a.txt should still be uncommitted: {status}"));
    assert_eq!(a["state"], "modified", "{status}");
}

/// A fuse left open with `--markers` holds set-aside work until it is settled.
/// It comes back merged on `--continue`...
#[test]
fn conflicted_fuse_returns_carried_changes_on_continue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("b.txt"), "changed on feature\n").unwrap();
    seal_all(path, "feature edit");

    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();

    let fuse = ivaldi(path, &["fuse", "main", "--markers"]);
    assert!(!fuse.status.success());
    let out = stdout(&fuse);
    assert!(out.contains("CONFLICT: b.txt"), "{out}");
    assert!(out.contains("set aside"), "{out}");
    // Set aside means the fuse got a clean tree to resolve in.
    assert_eq!(read(path, "a.txt"), numbered(&[]));

    std::fs::write(path.join("b.txt"), "resolved\n").unwrap();
    ivaldi_ok(path, &["fuse", "--continue"]);

    assert_eq!(read(path, "b.txt"), "resolved\n");
    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MAIN"), (19, "MINE")]));
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
}

/// ...and untouched on `--abort`, which also clears the conflict markers.
#[test]
fn conflicted_fuse_returns_carried_changes_on_abort() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("b.txt"), "changed on feature\n").unwrap();
    seal_all(path, "feature edit");

    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();
    assert!(
        !ivaldi(path, &["fuse", "main", "--markers"])
            .status
            .success()
    );
    assert!(read(path, "b.txt").contains("<<<<<<<"));

    ivaldi_ok(path, &["fuse", "--abort"]);
    assert_eq!(read(path, "a.txt"), numbered(&[(19, "MINE")]));
    assert_eq!(read(path, "b.txt"), "changed on feature\n");
    assert_eq!(last_seal_message(path), "feature edit");
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
}

/// `oops` takes back a whole fuse — merge seal, files, carried work — and is
/// its own inverse.
#[test]
fn oops_undoes_and_redoes_a_fuse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();
    std::fs::write(path.join("new.txt"), "untracked\n").unwrap();

    ivaldi_ok(path, &["fuse", "main"]);
    assert_eq!(last_seal_message(path), "Fuse main into feature");

    let out = stdout(&ivaldi_ok(path, &["oops"]));
    assert!(out.contains("fuse main"), "{out}");
    assert_eq!(last_seal_message(path), "base");
    assert_eq!(read(path, "a.txt"), numbered(&[(19, "MINE")]));
    assert_eq!(read(path, "b.txt"), "base\n");
    assert_eq!(read(path, "new.txt"), "untracked\n");

    ivaldi_ok(path, &["oops"]);
    assert_eq!(last_seal_message(path), "Fuse main into feature");
    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MAIN"), (19, "MINE")]));
    assert_eq!(read(path, "b.txt"), "changed on main\n");
    assert_eq!(read(path, "new.txt"), "untracked\n");
}

/// A bare `oops` right after a fuse left open with `--markers` aborts it.
#[test]
fn oops_aborts_a_conflicted_fuse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("b.txt"), "changed on feature\n").unwrap();
    seal_all(path, "feature edit");
    std::fs::write(path.join("a.txt"), numbered(&[(19, "MINE")])).unwrap();
    assert!(
        !ivaldi(path, &["fuse", "main", "--markers"])
            .status
            .success()
    );

    let out = stdout(&ivaldi_ok(path, &["oops"]));
    assert!(out.contains("Fuse aborted"), "{out}");
    assert_eq!(read(path, "a.txt"), numbered(&[(19, "MINE")]));
    assert_eq!(read(path, "b.txt"), "changed on feature\n");
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["merge"].is_null(), "{status}");
}

/// `reverse --all` exists to destroy uncommitted work, so it is the command
/// most worth being able to take back — staging included.
#[test]
fn oops_brings_back_what_reverse_threw_away() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    forge_with_identity(path);
    std::fs::write(path.join("a.txt"), "base\n").unwrap();
    std::fs::write(path.join("gone.txt"), "base\n").unwrap();
    seal_all(path, "base");

    std::fs::write(path.join("a.txt"), "hours of work\n").unwrap();
    std::fs::write(path.join("new.txt"), "untracked\n").unwrap();
    std::fs::remove_file(path.join("gone.txt")).unwrap();
    ivaldi_ok(path, &["gather", "new.txt"]);

    let out = stdout(&ivaldi_ok(path, &["reverse", "--all"]));
    assert!(out.contains("ivaldi oops"), "{out}");
    assert_eq!(read(path, "a.txt"), "base\n");
    assert!(!path.join("new.txt").exists());

    ivaldi_ok(path, &["oops"]);
    assert_eq!(read(path, "a.txt"), "hours of work\n");
    assert_eq!(read(path, "new.txt"), "untracked\n");
    assert!(!path.join("gone.txt").exists());
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status = String::from_utf8_lossy(&status.stdout).into_owned();
    assert!(status.contains("new.txt"), "{status}");

    let list = stdout(&ivaldi_ok(path, &["oops", "--list"]));
    assert!(list.contains("oops (undo of 'reverse --all')"), "{list}");
}

#[test]
fn oops_with_nothing_to_undo_says_so() {
    let dir = tempfile::tempdir().unwrap();
    forge_with_identity(dir.path());
    let oops = ivaldi(dir.path(), &["oops"]);
    assert!(!oops.status.success());
    assert!(String::from_utf8_lossy(&oops.stderr).contains("nothing to undo"));
}

// ---------------------------------------------------------------------------
// Fuse settles what it can, and asks about the rest — in one command
// ---------------------------------------------------------------------------

/// Run with answers on stdin, as if at a terminal.
fn ivaldi_answering(
    dir: &std::path::Path,
    args: &[&str],
    answers: &str,
    envs: &[(&str, &str)],
) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IVALDI_INTERACTIVE", "1")
        .envs(envs.iter().copied())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run ivaldi binary");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answers.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Both timelines have sealed edits to `a.txt`: they collide on lines 2 and
/// 10, and each also has an edit of its own (feature: 19, main: 28). `b.txt`
/// is changed on main only. Current timeline is `feature`.
fn colliding_timelines(path: &std::path::Path) {
    forge_with_identity(path);
    std::fs::write(path.join("a.txt"), numbered30(&[])).unwrap();
    std::fs::write(path.join("b.txt"), "base\n").unwrap();
    seal_all(path, "base");

    ivaldi_ok(path, &["timeline", "create", "feature"]);
    std::fs::write(
        path.join("a.txt"),
        numbered30(&[(2, "FEATURE 2"), (10, "FEATURE 10"), (19, "FEATURE ONLY")]),
    )
    .unwrap();
    seal_all(path, "feature edit");

    ivaldi_ok(path, &["timeline", "switch", "main"]);
    std::fs::write(
        path.join("a.txt"),
        numbered30(&[(2, "MAIN 2"), (10, "MAIN 10"), (28, "MAIN ONLY")]),
    )
    .unwrap();
    std::fs::write(path.join("b.txt"), "changed on main\n").unwrap();
    seal_all(path, "main edit");
    ivaldi_ok(path, &["timeline", "switch", "feature"]);
}

fn numbered30(edits: &[(usize, &str)]) -> String {
    (1..=30)
        .map(|n| match edits.iter().find(|(line, _)| *line == n) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("line {n}\n"),
        })
        .collect()
}

/// Two timelines editing the same file is not a conflict; editing the same
/// *lines* is. The first needs nobody.
#[test]
fn fuse_merges_one_file_changed_in_different_places_without_asking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    forge_with_identity(path);
    std::fs::write(path.join("a.txt"), numbered30(&[])).unwrap();
    seal_all(path, "base");
    ivaldi_ok(path, &["timeline", "create", "feature"]);
    std::fs::write(path.join("a.txt"), numbered30(&[(3, "FEATURE")])).unwrap();
    seal_all(path, "feature edit");
    ivaldi_ok(path, &["timeline", "switch", "main"]);
    std::fs::write(path.join("a.txt"), numbered30(&[(27, "MAIN")])).unwrap();
    seal_all(path, "main edit");

    // No terminal, no --prefer: it must not need either.
    ivaldi_ok(path, &["fuse", "feature"]);
    assert_eq!(
        read(path, "a.txt"),
        numbered30(&[(3, "FEATURE"), (27, "MAIN")])
    );
    assert_eq!(last_seal_message(path), "Fuse feature into main");
}

/// With collisions and nobody to ask, picking a side by rule would be a
/// guess. The fuse refuses — and refusing must leave no trace at all.
#[test]
fn fuse_with_nobody_to_ask_refuses_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    colliding_timelines(path);
    std::fs::write(path.join("b.txt"), "unsealed\n").unwrap();

    let fuse = ivaldi(path, &["fuse", "main"]);
    assert!(!fuse.status.success());
    let err = String::from_utf8_lossy(&fuse.stderr);
    assert!(err.contains("a.txt  (2 collision(s))"), "{err}");
    assert!(err.contains("--prefer mine|theirs|both"), "{err}");
    assert!(err.contains("Nothing was changed"), "{err}");

    assert_eq!(
        read(path, "a.txt"),
        numbered30(&[(2, "FEATURE 2"), (10, "FEATURE 10"), (19, "FEATURE ONLY")])
    );
    assert_eq!(read(path, "b.txt"), "unsealed\n");
    assert_eq!(last_seal_message(path), "feature edit");
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["merge"].is_null(), "{status}");
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
    assert!(!ivaldi(path, &["oops"]).status.success(), "nothing to undo");
}

/// `--prefer` settles the collisions and *only* the collisions: each side's
/// own edits to the same file still both land.
#[test]
fn fuse_prefer_settles_only_the_collisions() {
    for (prefer, two, ten) in [
        ("mine", "FEATURE 2", "FEATURE 10"),
        ("theirs", "MAIN 2", "MAIN 10"),
        ("both", "FEATURE 2\nMAIN 2", "FEATURE 10\nMAIN 10"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        colliding_timelines(path);

        ivaldi_ok(path, &["fuse", "main", "--prefer", prefer]);
        assert_eq!(
            read(path, "a.txt"),
            numbered30(&[(2, two), (10, ten), (19, "FEATURE ONLY"), (28, "MAIN ONLY")]),
            "--prefer {prefer}"
        );
        assert_eq!(read(path, "b.txt"), "changed on main\n");
        assert_eq!(last_seal_message(path), "Fuse main into feature");
        assert!(!read(path, "a.txt").contains("<<<<<<<"));
    }
}

/// At a terminal each collision is its own question, and the fuse is sealed
/// by the same command that asked.
#[test]
fn fuse_asks_about_each_collision_and_finishes_in_one_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    colliding_timelines(path);

    let fuse = ivaldi_answering(path, &["fuse", "main"], "t\nm\n", &[]);
    assert!(
        fuse.status.success(),
        "{}",
        String::from_utf8_lossy(&fuse.stderr)
    );
    let out = stdout(&fuse);
    assert!(
        out.contains("a.txt — collision 1 of 2, around line 2"),
        "{out}"
    );
    assert!(
        out.contains("a.txt — collision 2 of 2, around line 10"),
        "{out}"
    );
    assert!(
        out.contains("mine (feature)") && out.contains("theirs (main)"),
        "{out}"
    );
    assert!(out.contains("ivaldi oops"), "{out}");

    assert_eq!(
        read(path, "a.txt"),
        numbered30(&[
            (2, "MAIN 2"),
            (10, "FEATURE 10"),
            (19, "FEATURE ONLY"),
            (28, "MAIN ONLY")
        ])
    );
    assert_eq!(last_seal_message(path), "Fuse main into feature");
    let status = ivaldi_ok(path, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["merge"].is_null(), "{status}");
    assert_eq!(
        status["files"].as_array().map_or(0, Vec::len),
        0,
        "{status}"
    );
}

/// Quitting — even after answering some — is free: nothing was written yet.
#[test]
fn quitting_the_questions_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    colliding_timelines(path);
    std::fs::write(path.join("b.txt"), "unsealed\n").unwrap();

    let fuse = ivaldi_answering(path, &["fuse", "main"], "t\nq\n", &[]);
    assert!(!fuse.status.success());
    assert!(String::from_utf8_lossy(&fuse.stderr).contains("nothing was changed"));
    assert_eq!(read(path, "b.txt"), "unsealed\n");
    assert_eq!(last_seal_message(path), "feature edit");
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
}

/// When the right answer is neither side — the usual case being both edits
/// combined on one line — `edit` opens just that region.
#[cfg(unix)]
#[test]
fn edit_settles_a_collision_with_lines_neither_side_had() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    colliding_timelines(path);

    let editor = dir.path().join("editor.sh");
    std::fs::write(
        &editor,
        "#!/bin/sh\n\
         grep -v -e '^<<<<<<<' -e '^=======' -e '^>>>>>>>' -e '^MAIN' \"$1\" \
           | sed 's/^FEATURE \\(.*\\)$/FEATURE \\1 \\&\\& MAIN \\1/' > \"$1.new\"\n\
         mv \"$1.new\" \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fuse = ivaldi_answering(
        path,
        &["fuse", "main"],
        "e\nt\n",
        &[("VISUAL", editor.to_str().unwrap())],
    );
    assert!(
        fuse.status.success(),
        "{}",
        String::from_utf8_lossy(&fuse.stderr)
    );
    assert_eq!(
        read(path, "a.txt"),
        numbered30(&[
            (2, "FEATURE 2 && MAIN 2"),
            (10, "MAIN 10"),
            (19, "FEATURE ONLY"),
            (28, "MAIN ONLY")
        ])
    );
}

/// Carried uncommitted work that collides with the fuse is asked about the
/// same way, instead of getting markers.
#[test]
fn carried_collisions_are_asked_about_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("a.txt"), numbered(&[(2, "MINE")])).unwrap();

    let fuse = ivaldi_answering(path, &["fuse", "main"], "b\n", &[]);
    assert!(
        fuse.status.success(),
        "{}",
        String::from_utf8_lossy(&fuse.stderr)
    );
    let out = stdout(&fuse);
    assert!(out.contains("mine (your uncommitted changes)"), "{out}");
    assert!(out.contains("back on top"), "{out}");
    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MINE\nMAIN")]));
}

/// Carried collisions are asked *before* the fuse is sealed, so quitting at
/// one is as free as quitting at any other question.
#[test]
fn quitting_at_a_carried_collision_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    diverged_with_feature_current(path);
    std::fs::write(path.join("a.txt"), numbered(&[(2, "MINE")])).unwrap();

    let fuse = ivaldi_answering(path, &["fuse", "main"], "q\n", &[]);
    assert!(!fuse.status.success());
    let out = stdout(&fuse);
    assert!(out.contains("mine (your uncommitted changes)"), "{out}");
    assert!(
        out.contains("cancels the fuse; nothing has been changed"),
        "{out}"
    );
    assert!(String::from_utf8_lossy(&fuse.stderr).contains("nothing was changed"));

    assert_eq!(read(path, "a.txt"), numbered(&[(2, "MINE")]));
    assert_eq!(
        read(path, "b.txt"),
        "base\n",
        "main's change was not brought in"
    );
    assert_eq!(last_seal_message(path), "base");
    assert!(!path.join(".ivaldi/fuse-carry.snap").exists());
    assert!(!ivaldi(path, &["oops"]).status.success(), "nothing to undo");
}

/// A wrong answer costs one command: `oops` takes the whole fuse back, and
/// the question can be answered differently.
#[test]
fn oops_lets_a_collision_be_answered_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    colliding_timelines(path);

    ivaldi_ok(path, &["fuse", "main", "--prefer", "theirs"]);
    assert!(read(path, "a.txt").contains("MAIN 2"));

    ivaldi_ok(path, &["oops"]);
    assert_eq!(last_seal_message(path), "feature edit");
    assert_eq!(
        read(path, "a.txt"),
        numbered30(&[(2, "FEATURE 2"), (10, "FEATURE 10"), (19, "FEATURE ONLY")])
    );

    ivaldi_ok(path, &["fuse", "main", "--prefer", "mine"]);
    assert!(read(path, "a.txt").contains("FEATURE 2"));
    assert!(read(path, "a.txt").contains("MAIN ONLY"));
}
