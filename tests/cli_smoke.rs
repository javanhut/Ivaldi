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
