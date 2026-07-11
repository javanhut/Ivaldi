use std::process::{Command, Output};

fn ivaldi(current_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(current_dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("run ivaldi binary")
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
