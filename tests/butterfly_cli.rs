//! Butterfly timelines, end to end through the CLI: the behaviour
//! `docs/butterfly.md` promises.
//!
//! - `up` merges the butterfly's sealed work into its parent, and `down`
//!   merges the parent's into the butterfly. Neither side loses a seal.
//! - Collisions are settled by the layer rules: the incoming side goes on
//!   top, and an edit beats a deletion.
//! - `rm --cascade` removes a butterfly with everything nested under it;
//!   plain `rm` leaves the children, orphaned and unable to sync.

use std::path::Path;
use std::process::{Command, Output};

fn ivaldi(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(dir)
        .env("NO_COLOR", "1")
        // Nobody to ask: anything that would prompt has to settle by rule.
        .env("IVALDI_INTERACTIVE", "0")
        .args(args)
        .output()
        .expect("run ivaldi binary")
}

fn ivaldi_ok(dir: &Path, args: &[&str]) -> Output {
    let output = ivaldi(dir, args);
    assert!(
        output.status.success(),
        "ivaldi {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn ivaldi_fails(dir: &Path, args: &[&str]) -> String {
    let output = ivaldi(dir, args);
    assert!(
        !output.status.success(),
        "ivaldi {} should have failed\nstdout:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn forged() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ivaldi_ok(dir.path(), &["forge"]);
    ivaldi_ok(dir.path(), &["config", "--set", "user.name", "CLI Test"]);
    ivaldi_ok(
        dir.path(),
        &["config", "--set", "user.email", "cli-test@example.com"],
    );
    dir
}

fn write(dir: &Path, file: &str, content: &str) {
    std::fs::write(dir.join(file), content).unwrap();
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap()
}

fn seal_all(dir: &Path, message: &str) {
    ivaldi_ok(dir, &["gather", "."]);
    ivaldi_ok(dir, &["seal", message]);
}

fn switch(dir: &Path, timeline: &str) {
    ivaldi_ok(dir, &["timeline", "switch", timeline]);
}

/// The working files, `.ivaldi` aside, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != ".ivaldi")
        .collect();
    names.sort();
    names
}

fn status(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&ivaldi_ok(dir, &["status", "--json"]).stdout).unwrap()
}

fn assert_clean(dir: &Path) {
    let status = status(dir);
    assert_eq!(
        status["files"],
        serde_json::json!([]),
        "not clean: {status}"
    );
}

fn timelines(dir: &Path) -> Vec<String> {
    let list: serde_json::Value =
        serde_json::from_slice(&ivaldi_ok(dir, &["timeline", "list", "--json"]).stdout).unwrap();
    let mut names: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn seal_count(dir: &Path) -> usize {
    let log: serde_json::Value =
        serde_json::from_slice(&ivaldi_ok(dir, &["log", "--format", "json"]).stdout).unwrap();
    log.as_array().unwrap().len()
}

/// A repository with `a.txt` sealed on main and a butterfly `exp` made from
/// it, which is current.
fn with_butterfly() -> tempfile::TempDir {
    let dir = forged();
    write(dir.path(), "a.txt", "base\n");
    seal_all(dir.path(), "base");
    ivaldi_ok(dir.path(), &["timeline", "butterfly", "create", "exp"]);
    dir
}

/// Twenty numbered lines, with the given ones replaced.
fn numbered(edits: &[(usize, &str)]) -> String {
    (1..=20)
        .map(|n| {
            edits
                .iter()
                .find(|(line, _)| *line == n)
                .map_or_else(|| format!("line {n}\n"), |(_, text)| format!("{text}\n"))
        })
        .collect()
}

// -- create ------------------------------------------------------------------

#[test]
fn create_switches_to_a_butterfly_of_the_current_timeline() {
    let dir = with_butterfly();
    let d = dir.path();

    assert_eq!(status(d)["timeline"], "exp");
    assert_eq!(timelines(d), ["exp", "main"]);
    assert_eq!(files(d), ["a.txt"]);
}

// -- up ----------------------------------------------------------------------

#[test]
fn up_lands_the_butterflys_seals_on_an_unchanged_parent() {
    let dir = with_butterfly();
    let d = dir.path();
    write(d, "b.txt", "from exp\n");
    seal_all(d, "exp adds b");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    // Still on the butterfly, which up leaves as it was.
    assert_eq!(status(d)["timeline"], "exp");
    assert_eq!(files(d), ["a.txt", "b.txt"]);
    assert_clean(d);

    switch(d, "main");
    assert_eq!(files(d), ["a.txt", "b.txt"]);
    assert_eq!(read(d, "b.txt"), "from exp\n");
}

#[test]
fn up_keeps_what_the_parent_sealed_after_the_butterfly_was_made() {
    let dir = with_butterfly();
    let d = dir.path();
    write(d, "b.txt", "from exp\n");
    seal_all(d, "exp adds b");
    switch(d, "main");
    write(d, "c.txt", "from main\n");
    seal_all(d, "main adds c");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    // up sends the butterfly's work up; it does not pull the parent's down.
    assert_eq!(files(d), ["a.txt", "b.txt"]);
    assert_clean(d);

    switch(d, "main");
    assert_eq!(files(d), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(read(d, "c.txt"), "from main\n");
    assert_clean(d);
}

#[test]
fn up_with_nothing_new_changes_nothing() {
    let dir = with_butterfly();
    let d = dir.path();
    write(d, "b.txt", "from exp\n");
    seal_all(d, "exp adds b");
    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    let before = seal_count(d);
    switch(d, "exp");
    ivaldi_ok(d, &["tl", "bf", "up"]);
    switch(d, "main");
    assert_eq!(seal_count(d), before);
}

#[test]
fn up_leaves_uncommitted_work_on_the_butterfly() {
    let dir = with_butterfly();
    let d = dir.path();
    write(d, "b.txt", "sealed\n");
    seal_all(d, "exp adds b");
    write(d, "draft.txt", "not sealed yet\n");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    assert_eq!(read(d, "draft.txt"), "not sealed yet\n");
    switch(d, "main");
    assert_eq!(files(d), ["a.txt", "b.txt"]);
}

// -- down --------------------------------------------------------------------

#[test]
fn down_merges_the_parents_seals_into_the_butterfly_and_its_files() {
    let dir = with_butterfly();
    let d = dir.path();
    write(d, "e.txt", "from exp\n");
    seal_all(d, "exp adds e");
    switch(d, "main");
    write(d, "d.txt", "from main\n");
    seal_all(d, "main adds d");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "down"]);

    assert_eq!(files(d), ["a.txt", "d.txt", "e.txt"]);
    assert_eq!(read(d, "d.txt"), "from main\n");
    assert_clean(d);

    // Sealed, not just written: it survives leaving and coming back.
    switch(d, "main");
    assert_eq!(files(d), ["a.txt", "d.txt"]);
    switch(d, "exp");
    assert_eq!(files(d), ["a.txt", "d.txt", "e.txt"]);
    assert_clean(d);
}

#[test]
fn down_carries_uncommitted_work_through() {
    let dir = with_butterfly();
    let d = dir.path();
    switch(d, "main");
    write(d, "d.txt", "from main\n");
    seal_all(d, "main adds d");
    switch(d, "exp");
    write(d, "a.txt", "edited, not sealed\n");

    ivaldi_ok(d, &["tl", "bf", "down"]);

    assert_eq!(read(d, "d.txt"), "from main\n");
    assert_eq!(read(d, "a.txt"), "edited, not sealed\n");
    let status = status(d);
    assert_eq!(status["files"][0]["path"], "a.txt", "{status}");
    assert_eq!(status["files"][0]["state"], "modified", "{status}");
}

#[test]
fn down_on_a_timeline_that_is_not_a_butterfly_is_refused() {
    let dir = with_butterfly();
    let d = dir.path();
    switch(d, "main");

    let stderr = ivaldi_fails(d, &["tl", "bf", "down"]);
    assert!(stderr.contains("not a butterfly"), "{stderr}");
}

// -- the layer rules ---------------------------------------------------------

#[test]
fn edits_to_different_lines_of_one_file_both_survive_a_sync() {
    let dir = forged();
    let d = dir.path();
    write(d, "story.txt", &numbered(&[]));
    seal_all(d, "base");
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    write(d, "story.txt", &numbered(&[(9, "exp's ending")]));
    seal_all(d, "exp edits the end");
    switch(d, "main");
    write(d, "story.txt", &numbered(&[(2, "main's opening")]));
    seal_all(d, "main edits the start");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    assert_eq!(
        read(d, "story.txt"),
        numbered(&[(2, "main's opening"), (9, "exp's ending")])
    );
}

#[test]
fn up_puts_the_butterflys_version_on_top_of_a_collision() {
    let dir = forged();
    let d = dir.path();
    write(d, "story.txt", &numbered(&[]));
    seal_all(d, "base");
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    write(d, "story.txt", &numbered(&[(5, "exp's line")]));
    write(d, "new.txt", "exp's new file\n");
    seal_all(d, "exp edits");
    switch(d, "main");
    write(
        d,
        "story.txt",
        &numbered(&[(5, "main's line"), (16, "main's late line")]),
    );
    write(d, "new.txt", "main's new file\n");
    seal_all(d, "main edits");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    assert_eq!(
        read(d, "story.txt"),
        numbered(&[(5, "exp's line"), (16, "main's late line")])
    );
    assert_eq!(read(d, "new.txt"), "exp's new file\n");
    assert_clean(d);
}

#[test]
fn down_puts_the_parents_version_on_top_of_a_collision() {
    let dir = forged();
    let d = dir.path();
    write(d, "story.txt", &numbered(&[]));
    seal_all(d, "base");
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    write(
        d,
        "story.txt",
        &numbered(&[(5, "exp's line"), (18, "exp's late line")]),
    );
    seal_all(d, "exp edits");
    switch(d, "main");
    write(d, "story.txt", &numbered(&[(5, "main's line")]));
    seal_all(d, "main edits");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "down"]);

    assert_eq!(
        read(d, "story.txt"),
        numbered(&[(5, "main's line"), (18, "exp's late line")])
    );
    assert_clean(d);
}

/// The fuse engine takes edits within a few lines of a collision as part of
/// it rather than splice two rewrites of the same code together, so the
/// layer rule settles that whole stretch.
#[test]
fn an_edit_right_next_to_a_collision_goes_with_it() {
    let dir = forged();
    let d = dir.path();
    write(d, "story.txt", &numbered(&[]));
    seal_all(d, "base");
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    write(d, "story.txt", &numbered(&[(5, "exp's line")]));
    seal_all(d, "exp edits");
    switch(d, "main");
    write(
        d,
        "story.txt",
        &numbered(&[(5, "main's line"), (7, "main's nearby line")]),
    );
    seal_all(d, "main edits");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    assert_eq!(read(d, "story.txt"), numbered(&[(5, "exp's line")]));
}

#[test]
fn a_sync_keeps_the_modified_side_of_a_delete_against_an_edit() {
    let dir = forged();
    let d = dir.path();
    write(d, "keep.txt", "base\n");
    write(d, "gone-on-exp.txt", "base\n");
    write(d, "gone-on-main.txt", "base\n");
    seal_all(d, "base");
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    std::fs::remove_file(d.join("gone-on-exp.txt")).unwrap();
    write(d, "gone-on-main.txt", "edited on exp\n");
    seal_all(d, "exp deletes one, edits the other");
    switch(d, "main");
    std::fs::remove_file(d.join("gone-on-main.txt")).unwrap();
    write(d, "gone-on-exp.txt", "edited on main\n");
    seal_all(d, "main deletes one, edits the other");
    switch(d, "exp");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    assert_eq!(read(d, "gone-on-exp.txt"), "edited on main\n");
    assert_eq!(read(d, "gone-on-main.txt"), "edited on exp\n");
}

// -- nesting -----------------------------------------------------------------

#[test]
fn a_nested_butterfly_syncs_with_its_own_parent() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["tl", "bf", "create", "exp2"]);
    write(d, "deep.txt", "from exp2\n");
    seal_all(d, "exp2 adds deep");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "exp");
    assert_eq!(files(d), ["a.txt", "deep.txt"]);
    switch(d, "main");
    assert_eq!(files(d), ["a.txt"]);
}

// -- rm ----------------------------------------------------------------------

#[test]
fn rm_cascade_removes_every_nested_butterfly() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["tl", "bf", "create", "exp2"]);
    ivaldi_ok(d, &["tl", "bf", "create", "exp3"]);
    switch(d, "exp");
    ivaldi_ok(d, &["tl", "bf", "create", "sibling"]);
    switch(d, "main");

    ivaldi_ok(d, &["tl", "bf", "rm", "exp", "--cascade"]);

    assert_eq!(timelines(d), ["main"]);
}

#[test]
fn rm_without_cascade_orphans_the_children_which_cannot_sync() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["tl", "bf", "create", "exp2"]);
    switch(d, "main");

    ivaldi_ok(d, &["tl", "bf", "rm", "exp"]);
    assert_eq!(timelines(d), ["exp2", "main"]);

    // Still a working timeline...
    switch(d, "exp2");
    write(d, "b.txt", "still usable\n");
    seal_all(d, "orphan work");
    // ...that has nothing to sync with.
    let stderr = ivaldi_fails(d, &["tl", "bf", "up"]);
    assert!(stderr.contains("orphaned"), "{stderr}");
    let stderr = ivaldi_fails(d, &["tl", "bf", "down"]);
    assert!(stderr.contains("orphaned"), "{stderr}");
}

#[test]
fn removing_a_parent_with_timeline_remove_orphans_its_butterflies() {
    let dir = forged();
    let d = dir.path();
    write(d, "a.txt", "base\n");
    seal_all(d, "base");
    ivaldi_ok(d, &["timeline", "create", "feature"]);
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    switch(d, "main");
    ivaldi_ok(d, &["timeline", "remove", "feature"]);

    switch(d, "exp");
    let stderr = ivaldi_fails(d, &["tl", "bf", "up"]);
    assert!(stderr.contains("orphaned"), "{stderr}");
}

#[test]
fn rm_refuses_to_remove_the_current_timeline_or_its_ancestors() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["tl", "bf", "create", "exp2"]);

    ivaldi_fails(d, &["tl", "bf", "rm", "exp", "--cascade"]);
    ivaldi_fails(d, &["tl", "bf", "rm", "exp2"]);

    assert_eq!(timelines(d), ["exp", "exp2", "main"]);
    // Refused before anything was touched: exp2 still syncs into exp.
    write(d, "b.txt", "x\n");
    seal_all(d, "exp2 work");
    ivaldi_ok(d, &["tl", "bf", "up"]);
}

#[test]
fn rm_refuses_a_timeline_that_is_not_a_butterfly() {
    let dir = with_butterfly();
    let d = dir.path();
    switch(d, "main");
    ivaldi_ok(d, &["timeline", "create", "feature"]);
    switch(d, "main");

    let stderr = ivaldi_fails(d, &["tl", "bf", "rm", "feature"]);
    assert!(stderr.contains("not a butterfly"), "{stderr}");
    assert_eq!(timelines(d), ["exp", "feature", "main"]);
}

// -- metadata follows the timelines ------------------------------------------

#[test]
fn renaming_a_parent_keeps_its_butterflies_attached() {
    let dir = with_butterfly();
    let d = dir.path();
    switch(d, "main");
    ivaldi_ok(d, &["timeline", "rename", "main", "trunk"]);
    switch(d, "exp");
    write(d, "b.txt", "from exp\n");
    seal_all(d, "exp adds b");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "trunk");
    assert_eq!(files(d), ["a.txt", "b.txt"]);
}

#[test]
fn a_renamed_butterfly_is_still_a_butterfly() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["timeline", "rename", "exp", "experiment"]);
    write(d, "b.txt", "from experiment\n");
    seal_all(d, "experiment adds b");

    ivaldi_ok(d, &["tl", "bf", "up"]);

    switch(d, "main");
    assert_eq!(files(d), ["a.txt", "b.txt"]);
}

#[test]
fn a_new_butterfly_does_not_inherit_a_removed_ones_children() {
    let dir = with_butterfly();
    let d = dir.path();
    ivaldi_ok(d, &["tl", "bf", "create", "exp2"]);
    switch(d, "main");
    ivaldi_ok(d, &["tl", "bf", "rm", "exp"]);
    ivaldi_ok(d, &["tl", "bf", "create", "exp"]);
    switch(d, "main");

    // The old exp2 is an orphan, not a child of the new exp.
    ivaldi_ok(d, &["tl", "bf", "rm", "exp", "--cascade"]);
    assert_eq!(timelines(d), ["exp2", "main"]);
}

#[test]
fn oops_undoes_a_down() {
    let dir = with_butterfly();
    let d = dir.path();
    switch(d, "main");
    write(d, "d.txt", "from main\n");
    seal_all(d, "main adds d");
    switch(d, "exp");
    let before = seal_count(d);
    ivaldi_ok(d, &["tl", "bf", "down"]);

    ivaldi_ok(d, &["oops"]);

    assert_eq!(files(d), ["a.txt"]);
    assert_eq!(seal_count(d), before);
    assert_clean(d);
}
