//! Unit tests for command implementations.

use super::*;
use crate::fsmerkle::ChangeKind;
use crate::workspace::FileState;

#[test]
fn generic_git_url_parsing() {
    // AUR: single-segment path → host stands in for owner.
    assert_eq!(
        parse_generic_git_url("https://aur.archlinux.org/yay.git"),
        Some((
            "https://aur.archlinux.org/yay.git".into(),
            "aur.archlinux.org".into(),
            "yay".into()
        ))
    );
    // Gitea-style owner/repo, no .git, trailing slash.
    assert_eq!(
        parse_generic_git_url("https://gitea.example.com/foo/bar/"),
        Some((
            "https://gitea.example.com/foo/bar".into(),
            "foo".into(),
            "bar".into()
        ))
    );
    // GitHub/GitLab keep their own path.
    assert_eq!(
        parse_generic_git_url("https://github.com/torvalds/linux"),
        None
    );
    assert_eq!(parse_generic_git_url("https://gitlab.com/foo/bar"), None);
    // Bare shorthand and SSH are handled elsewhere.
    assert_eq!(parse_generic_git_url("owner/repo"), None);
    assert_eq!(parse_generic_git_url("git@example.com:foo/bar.git"), None);
}

mod editor_message {
    use super::super::*;

    #[test]
    fn strips_comment_lines_and_trims_blanks() {
        let raw = "\nFix the parser\n\nHandle empty input\n# Please enter the message\n#\tchanged: src/parse.rs\n";
        assert_eq!(
            strip_message_comments(raw),
            "Fix the parser\n\nHandle empty input"
        );
    }

    #[test]
    fn keeps_hash_not_at_line_start() {
        // A literal '#' mid-line (e.g. an issue ref) is not a comment.
        assert_eq!(
            strip_message_comments("Closes issue #42"),
            "Closes issue #42"
        );
    }

    #[test]
    fn all_comments_yields_empty() {
        assert_eq!(strip_message_comments("# only\n# comments\n"), "");
    }

    #[test]
    fn editor_prefers_visual_then_editor_then_vim() {
        // SAFETY: single-threaded test; we restore the env afterward.
        let saved_visual = std::env::var_os("VISUAL");
        let saved_editor = std::env::var_os("EDITOR");

        unsafe {
            std::env::remove_var("VISUAL");
            std::env::remove_var("EDITOR");
        }
        assert_eq!(resolve_editor(), "vim");

        unsafe { std::env::set_var("EDITOR", "nano") };
        assert_eq!(resolve_editor(), "nano");

        unsafe { std::env::set_var("VISUAL", "code --wait") };
        assert_eq!(resolve_editor(), "code --wait");

        // Whitespace-only values are ignored in favor of the next source.
        unsafe { std::env::set_var("VISUAL", "  ") };
        assert_eq!(resolve_editor(), "nano");

        unsafe {
            match saved_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
            match saved_editor {
                Some(v) => std::env::set_var("EDITOR", v),
                None => std::env::remove_var("EDITOR"),
            }
        }
    }
}

mod patch_session {
    use super::super::*;
    use crate::cas::Cas;
    use crate::fsmerkle::FsStore;
    use crate::workspace::DotfileAllowlist;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    /// Forge a repo whose head seals a.txt with 14 numbered lines, then
    /// modify lines 2 and 12 on disk (two distinct hunks).
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().to_path_buf();
        let ivaldi_dir = work_dir.join(".ivaldi");
        forge::forge(&work_dir).unwrap();
        let mut cfg = Config::new();
        cfg.set("user.name", "Test");
        cfg.set("user.email", "t@ivaldi.dev");
        cfg.save(&ivaldi_dir.join("config")).unwrap();

        let old: Vec<String> = (1..=14).map(|i| format!("line {}", i)).collect();
        let old_content = old.join("\n") + "\n";
        fs::write(work_dir.join("a.txt"), &old_content).unwrap();

        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let mut ws = Workspace::new(&cas, &work_dir, &ivaldi_dir);
        ws.gather(&["a.txt"], &DotfileAllowlist::load(&ivaldi_dir))
            .unwrap();
        let tree = ws.build_seal_tree(None).unwrap();
        let mut repo = Repo::open(&work_dir).unwrap();
        repo.commit(tree, "Test", "initial").unwrap();
        ws.staging.clear();
        ws.save().unwrap();

        let mut new = old.clone();
        new[1] = "line 2 CHANGED".into();
        new[11] = "line 12 CHANGED".into();
        fs::write(work_dir.join("a.txt"), new.join("\n") + "\n").unwrap();

        (dir, work_dir, ivaldi_dir, old_content)
    }

    fn run(
        work_dir: &std::path::Path,
        ivaldi_dir: &std::path::Path,
        answers: &str,
    ) -> (Vec<String>, BTreeMap<String, crate::hash::B3Hash>) {
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let mut ws = Workspace::new(&cas, work_dir, ivaldi_dir);
        let repo = Repo::open(work_dir).unwrap();
        let timeline = repo.current_timeline().unwrap();
        let parent_tree = repo
            .get_timeline_head(&timeline)
            .unwrap()
            .and_then(|idx| repo.get_leaf(idx).unwrap())
            .map(|l| l.tree_root)
            .unwrap();
        let parent_files = ws.list_tree_files(parent_tree).unwrap();
        drop(repo);

        let ignore_cache = ignore::load_pattern_cache(work_dir);
        let mut input = std::io::Cursor::new(answers.as_bytes().to_vec());
        let staged = run_patch_session(
            &mut ws,
            &cas,
            &parent_files,
            &[],
            &ignore_cache,
            &mut input,
            true,
        )
        .unwrap();
        ws.save().unwrap();

        let mut staged_hashes = BTreeMap::new();
        for (path, hash) in ws.staging.staged_files() {
            staged_hashes.insert(path.clone(), *hash);
        }
        (staged, staged_hashes)
    }

    fn blob_text(ivaldi_dir: &std::path::Path, hash: crate::hash::B3Hash) -> String {
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let (_, content) = store.load_blob(hash).unwrap();
        String::from_utf8(content).unwrap()
    }

    #[test]
    fn stage_first_hunk_only() {
        let (_dir, work_dir, ivaldi_dir, _old) = setup();
        let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\nn\n");
        assert_eq!(staged, vec!["a.txt"]);

        let text = blob_text(&ivaldi_dir, hashes["a.txt"]);
        assert!(text.contains("line 2 CHANGED"));
        assert!(!text.contains("line 12 CHANGED"));
        assert!(text.contains("line 12\n"));
    }

    #[test]
    fn quit_stages_nothing() {
        let (_dir, work_dir, ivaldi_dir, _old) = setup();
        let (staged, hashes) = run(&work_dir, &ivaldi_dir, "q\n");
        assert!(staged.is_empty());
        assert!(hashes.is_empty());
    }

    #[test]
    fn all_rest_stages_full_change() {
        let (_dir, work_dir, ivaldi_dir, _old) = setup();
        let (staged, hashes) = run(&work_dir, &ivaldi_dir, "a\n");
        assert_eq!(staged, vec!["a.txt"]);
        let text = blob_text(&ivaldi_dir, hashes["a.txt"]);
        assert!(text.contains("line 2 CHANGED"));
        assert!(text.contains("line 12 CHANGED"));
    }

    #[test]
    fn untracked_file_gets_whole_file_prompt() {
        let (_dir, work_dir, ivaldi_dir, old) = setup();
        // Restore a.txt so only the new untracked file is a candidate.
        fs::write(work_dir.join("a.txt"), &old).unwrap();
        fs::write(work_dir.join("b.txt"), "fresh\n").unwrap();

        let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\n");
        assert_eq!(staged, vec!["b.txt"]);
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        assert!(cas.has(hashes["b.txt"]).unwrap());
    }

    #[test]
    fn binary_file_gets_whole_file_prompt() {
        let (_dir, work_dir, ivaldi_dir, _old) = setup();
        fs::write(work_dir.join("a.txt"), b"bin\x00ary new").unwrap();
        // Seal tree's a.txt is text, disk is binary → whole-file prompt.
        let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\n");
        assert_eq!(staged, vec!["a.txt"]);
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let (_, content) = store.load_blob(hashes["a.txt"]).unwrap();
        assert_eq!(content, b"bin\x00ary new");
    }
}

mod switch_recovery {
    use super::super::*;
    use crate::switch_journal::{self, SwitchJournal};
    use crate::workspace::DotfileAllowlist;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Forge a repo with one seal on `main` (a.txt) and a `feature`
    /// timeline branched from it.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().to_path_buf();
        let ivaldi_dir = work_dir.join(".ivaldi");
        forge::forge(&work_dir).unwrap();
        let mut cfg = Config::new();
        cfg.set("user.name", "Test");
        cfg.set("user.email", "t@ivaldi.dev");
        cfg.save(&ivaldi_dir.join("config")).unwrap();

        fs::write(work_dir.join("a.txt"), "main content").unwrap();
        seal_all(&work_dir, &ivaldi_dir, "initial");

        let repo = Repo::open(&work_dir).unwrap();
        repo.create_timeline("feature", None).unwrap();
        (dir, work_dir, ivaldi_dir)
    }

    fn seal_all(work_dir: &Path, ivaldi_dir: &Path, message: &str) {
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let mut ws = Workspace::new(&cas, work_dir, ivaldi_dir);
        ws.gather(&["a.txt"], &DotfileAllowlist::load(ivaldi_dir))
            .unwrap();
        let mut repo = Repo::open(work_dir).unwrap();
        let timeline = repo.current_timeline().unwrap();
        let parent = repo
            .get_timeline_head(&timeline)
            .unwrap()
            .and_then(|idx| repo.get_leaf(idx).unwrap())
            .map(|l| l.tree_root);
        let tree = ws.build_seal_tree(parent).unwrap();
        repo.commit(tree, "Test <t@ivaldi.dev>", message).unwrap();
        ws.staging.clear();
        ws.save().unwrap();
    }

    fn no_tmp_files(dir: &Path) -> bool {
        !fs::read_dir(dir)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().contains(".tmp."))
    }

    #[test]
    fn happy_path_shelves_and_restores() {
        let (_dir, work_dir, ivaldi_dir) = setup();

        // Dirty the worktree on main.
        fs::write(work_dir.join("a.txt"), "dirty edit").unwrap();
        fs::write(work_dir.join("b.txt"), "untracked").unwrap();

        do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
        assert!(no_tmp_files(&ivaldi_dir));
        let repo = Repo::open(&work_dir).unwrap();
        assert_eq!(repo.current_timeline().unwrap(), "feature");
        // Worktree reverted to the sealed tree; dirt is shelved on main.
        assert_eq!(
            fs::read_to_string(work_dir.join("a.txt")).unwrap(),
            "main content"
        );
        assert!(!work_dir.join("b.txt").exists());
        drop(repo);

        // Switching back restores the shelf.
        do_timeline_switch(&work_dir, &ivaldi_dir, "main", true).unwrap();
        assert_eq!(
            fs::read_to_string(work_dir.join("a.txt")).unwrap(),
            "dirty edit"
        );
        assert_eq!(
            fs::read_to_string(work_dir.join("b.txt")).unwrap(),
            "untracked"
        );
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
    }

    #[test]
    fn resume_completes_interrupted_switch() {
        let (_dir, work_dir, ivaldi_dir) = setup();

        // Simulate a crash right after the journal write: shelve phase
        // done (clean worktree → no shelf), journal present, HEAD and
        // worktree untouched.
        switch_journal::write(
            &ivaldi_dir,
            &SwitchJournal {
                from: "main".into(),
                to: "feature".into(),
                shelf_saved: false,
                started_at: 0,
            },
        )
        .unwrap();

        do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
        let repo = Repo::open(&work_dir).unwrap();
        assert_eq!(repo.current_timeline().unwrap(), "feature");
    }

    #[test]
    fn rollback_returns_to_source() {
        let (_dir, work_dir, ivaldi_dir) = setup();
        switch_journal::write(
            &ivaldi_dir,
            &SwitchJournal {
                from: "main".into(),
                to: "feature".into(),
                shelf_saved: false,
                started_at: 0,
            },
        )
        .unwrap();

        do_timeline_switch(&work_dir, &ivaldi_dir, "main", true).unwrap();
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
        let repo = Repo::open(&work_dir).unwrap();
        assert_eq!(repo.current_timeline().unwrap(), "main");
    }

    #[test]
    fn third_timeline_refused_while_journal_exists() {
        let (_dir, work_dir, ivaldi_dir) = setup();
        let repo = Repo::open(&work_dir).unwrap();
        repo.create_timeline("third", None).unwrap();
        repo.switch_timeline("main").unwrap();
        drop(repo);

        switch_journal::write(
            &ivaldi_dir,
            &SwitchJournal {
                from: "main".into(),
                to: "feature".into(),
                shelf_saved: false,
                started_at: 0,
            },
        )
        .unwrap();

        let err = do_timeline_switch(&work_dir, &ivaldi_dir, "third", true).unwrap_err();
        assert!(err.contains("interrupted timeline switch"));
        // Journal untouched.
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_some());
    }

    #[test]
    fn guard_blocks_mutations_while_journal_exists() {
        let (_dir, _work_dir, ivaldi_dir) = setup();
        assert!(ensure_no_interrupted_switch(&ivaldi_dir).is_ok());

        switch_journal::write(
            &ivaldi_dir,
            &SwitchJournal {
                from: "main".into(),
                to: "feature".into(),
                shelf_saved: true,
                started_at: 0,
            },
        )
        .unwrap();
        let err = ensure_no_interrupted_switch(&ivaldi_dir).unwrap_err();
        assert!(err.contains("timeline switch feature"));
    }

    #[test]
    fn already_on_timeline_still_resumes_when_journal_targets_it() {
        // Crash right after the HEAD write: HEAD already points at the
        // target, but materialize/restore never ran. Re-running the
        // switch must complete it, not early-return "already on".
        let (_dir, work_dir, ivaldi_dir) = setup();
        let repo = Repo::open(&work_dir).unwrap();
        repo.switch_timeline("feature").unwrap();
        drop(repo);
        fs::write(work_dir.join("a.txt"), "mid-transition garbage").unwrap();

        switch_journal::write(
            &ivaldi_dir,
            &SwitchJournal {
                from: "main".into(),
                to: "feature".into(),
                shelf_saved: false,
                started_at: 0,
            },
        )
        .unwrap();

        do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
        assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
        // Materialize ran: the sealed content is back.
        assert_eq!(
            fs::read_to_string(work_dir.join("a.txt")).unwrap(),
            "main content"
        );
    }
}

#[test]
fn status_markers_added() {
    let (marker, _) = state_marker(FileState::Untracked);
    assert_eq!(marker, "++");
}

#[test]
fn status_markers_modified() {
    let (marker, _) = state_marker(FileState::Modified);
    assert_eq!(marker, "~~");
}

#[test]
fn status_markers_deleted() {
    let (marker, _) = state_marker(FileState::Deleted);
    assert_eq!(marker, "--");
}

#[test]
fn status_markers_staged() {
    let (marker, _) = state_marker(FileState::Staged);
    assert_eq!(marker, "++");
}

#[test]
fn change_marker_added() {
    let (marker, _) = change_marker(ChangeKind::Added);
    assert_eq!(marker, "++");
}

#[test]
fn change_marker_deleted() {
    let (marker, _) = change_marker(ChangeKind::Deleted);
    assert_eq!(marker, "--");
}

#[test]
fn change_marker_modified() {
    let (marker, _) = change_marker(ChangeKind::Modified);
    assert_eq!(marker, "~~");
}

#[test]
fn change_marker_typechange() {
    let (marker, _) = change_marker(ChangeKind::TypeChange);
    assert_eq!(marker, "~~");
}

#[test]
fn diff_two_trees_with_markers() {
    use crate::cas::MemoryCas;
    use crate::fsmerkle::{self, FsStore};
    use std::collections::BTreeMap;

    let cas = MemoryCas::new();
    let store = FsStore::new(&cas);

    let mut files_a = BTreeMap::new();
    files_a.insert("kept.txt".into(), b"same".to_vec());
    files_a.insert("changed.txt".into(), b"old content".to_vec());
    files_a.insert("removed.txt".into(), b"gone".to_vec());
    let tree_a = store.build_tree_from_map(&files_a).unwrap();

    let mut files_b = BTreeMap::new();
    files_b.insert("kept.txt".into(), b"same".to_vec());
    files_b.insert("changed.txt".into(), b"new content".to_vec());
    files_b.insert("added.txt".into(), b"fresh".to_vec());
    let tree_b = store.build_tree_from_map(&files_b).unwrap();

    let changes = fsmerkle::diff_trees(tree_a, tree_b, &store).unwrap();
    assert_eq!(changes.len(), 3);

    let mut added = 0;
    let mut modified = 0;
    let mut deleted = 0;
    for c in &changes {
        let (marker, _) = change_marker(c.kind);
        match marker {
            "++" => added += 1,
            "~~" => modified += 1,
            "--" => deleted += 1,
            _ => panic!("unexpected marker"),
        }
    }
    assert_eq!(added, 1);
    assert_eq!(modified, 1);
    assert_eq!(deleted, 1);
}
