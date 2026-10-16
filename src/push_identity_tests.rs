//! Upload must never re-mint history the Git side already has.
//!
//! These tests drive the real push planner against a local bare repository
//! through `git receive-pack --stateless-rpc`, the same protocol a Git host
//! speaks, so the server's own connectivity and fsck checks judge the pack.
//!
//! The fixture history carries `.gitignore`, `.gitattributes` and nested
//! dotfiles. Import's dotfile policy rewrites those trees, so any seal that
//! export re-translates instead of reusing comes back as a different commit —
//! exactly the duplicated history these tests guard against.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::git_remote::{
    AdvertisedRef, FetchResult, GitObject, GitObjectKind, PushReport, import_fetch_result,
    parse_report_status, plan_push, receive_pack_commands,
};
use crate::remote::HashMapping;
use crate::repo::Repo;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_AUTHOR_DATE", "1710000000 +0000")
        .env("GIT_COMMITTER_DATE", "1710000000 +0000")
        .output()
        .expect("git is installed");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn have_git() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn commit_all(dir: &Path, msg: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A Git project whose history has dotfiles from the second commit on, plus a
/// bare clone acting as the portal. Returns `(work, bare, [c1, c2, c3])`.
fn git_fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, [String; 3]) {
    git_fixture_with(root, true)
}

fn git_fixture_with(
    root: &Path,
    dotfiles: bool,
) -> (std::path::PathBuf, std::path::PathBuf, [String; 3]) {
    let work = root.join("work");
    let bare = root.join("remote.git");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);

    write(&work, "README.md", "hello\n");
    let c1 = commit_all(&work, "Initial Commit");

    if dotfiles {
        write(&work, ".gitignore", "target/\n");
        write(&work, ".gitattributes", "*.bin binary\n");
        write(&work, ".dockerignore", "target\n");
        write(&work, "root/.zshrc", "export X=1\n");
    }
    write(&work, "src/lib.rs", "fn a() {}\n");
    let c2 = commit_all(&work, "Adding git lfs and things for deployment");

    write(&work, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let c3 = commit_all(&work, "add b");

    git(
        root,
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    git(&bare, &["config", "receive.fsckObjects", "true"]);
    (work, bare, [c1, c2, c3])
}

/// Every object of the work repository, in the shape a fetch hands import.
fn fetch_all(work: &Path, branch: &str) -> FetchResult {
    let out = Command::new("git")
        .arg("-C")
        .arg(work)
        .args(["cat-file", "--batch-all-objects", "--batch"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(out.status.success());
    let data = out.stdout;
    let mut objects = HashMap::new();
    let mut pos = 0;
    while pos < data.len() {
        let nl = pos + data[pos..].iter().position(|&b| b == b'\n').unwrap();
        let header = std::str::from_utf8(&data[pos..nl]).unwrap();
        let mut parts = header.split(' ');
        let sha = parts.next().unwrap().to_string();
        let kind = match parts.next().unwrap() {
            "blob" => GitObjectKind::Blob,
            "tree" => GitObjectKind::Tree,
            "commit" => GitObjectKind::Commit,
            "tag" => GitObjectKind::Tag,
            other => panic!("unexpected object type {other}"),
        };
        let size: usize = parts.next().unwrap().parse().unwrap();
        let body = data[nl + 1..nl + 1 + size].to_vec();
        pos = nl + 1 + size + 1;
        objects.insert(sha, GitObject { kind, data: body });
    }
    FetchResult {
        branch: branch.to_string(),
        head_sha: git(work, &["rev-parse", branch]),
        refs: Vec::new(),
        objects,
        pack: None,
        shallow: BTreeSet::new(),
    }
}

/// Upload `branch` to the bare repository the way `ivaldi upload` does:
/// advertisement → `plan_push` → pack → receive-pack → record the new tip.
fn upload(repo: &Repo, branch: &str, bare: &Path) -> Result<PushReport, String> {
    let advertised: Vec<AdvertisedRef> =
        git(bare, &["for-each-ref", "--format=%(objectname) %(refname)"])
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(id, name)| AdvertisedRef {
                id: id.to_string(),
                name: name.to_string(),
            })
            .collect();
    let head_idx = repo.get_timeline_head(branch).unwrap().unwrap();
    let mapping = HashMapping::new(&repo.ivaldi_dir);
    let plan = plan_push(repo, branch, head_idx, &advertised, &mapping, false, false)
        .map_err(|e| e.to_string())?;
    if plan.updates.is_empty() {
        return Err("nothing to push".into());
    }

    let mut objects: Vec<&crate::git_export::GitObject> = plan.objects.values().collect();
    objects.sort_by_key(|o| o.sha1);
    let mut body = receive_pack_commands(&plan.updates);
    body.extend(crate::git_pack_writer::write_pack(&objects).unwrap());

    let mut child = Command::new("git")
        .args(["receive-pack", "--stateless-rpc"])
        .arg(bare)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().unwrap();
    let report = parse_report_status(&out.stdout).map_err(|e| e.to_string())?;
    if !report.unpack_ok || report.refs.iter().any(|r| r.error.is_some()) {
        return Err(format!(
            "receive-pack refused the upload: {:?} / {}",
            report,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let new_tip = plan
        .updates
        .iter()
        .find(|(name, _, _)| *name == format!("refs/heads/{branch}"))
        .map(|(_, _, new)| new.clone())
        .unwrap();
    crate::git_remote::record_pushed_tip(repo, head_idx, &new_tip);
    Ok(report)
}

/// Seal `path = body` on the current timeline on top of its head's tree.
fn seal_change(repo: &mut Repo, path: &str, body: &[u8], msg: &str) -> u64 {
    let ws = crate::workspace::Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir);
    let head = repo
        .get_timeline_head(&repo.current_timeline().unwrap())
        .unwrap()
        .unwrap();
    let tree = repo.get_leaf(head).unwrap().unwrap().tree_root;
    let mut files = ws.list_tree_files(tree).unwrap();
    let store = crate::fsmerkle::FsStore::new(&repo.cas);
    let (blob, _) = store.put_blob(body).unwrap();
    files.insert(path.to_string(), blob);
    let tree = store.build_tree_from_hash_map(&files).unwrap();
    repo.commit(tree, "Tester <t@example.com>", msg)
        .unwrap()
        .index
}

/// Leaf hash → every Git SHA-1 the persisted map assigns to it.
fn shas_per_leaf(repo: &Repo) -> BTreeMap<crate::hash::B3Hash, Vec<String>> {
    let text = std::fs::read_to_string(repo.ivaldi_dir.join("hash-map")).unwrap();
    let mut out: BTreeMap<_, Vec<String>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (sha, b3) = line.split_once(' ').unwrap();
        out.entry(crate::hash::B3Hash::from_hex(b3).unwrap())
            .or_default()
            .push(sha.to_string());
    }
    out
}

/// Every commit leaf maps to exactly the given number of SHA-1s, and an
/// imported one's identity is its authenticated `git.sha1`.
fn assert_commit_identities(repo: &Repo, expect_per_leaf: usize) {
    let map = shas_per_leaf(repo);
    for idx in 0..repo.commit_count() {
        let leaf = repo.get_leaf(idx).unwrap().unwrap();
        let shas = map.get(&leaf.hash()).cloned().unwrap_or_default();
        if let Some(git_sha) = leaf.meta.get("git.sha1") {
            assert!(
                shas.contains(git_sha),
                "seal {idx} lost its imported identity"
            );
            assert_eq!(
                shas.len(),
                expect_per_leaf,
                "seal {idx} ({}) has git identities {shas:?}",
                leaf.message
            );
        } else {
            assert!(
                shas.len() <= 1,
                "native seal {idx} ({}) was given several git identities: {shas:?}",
                leaf.message
            );
        }
    }
}

/// Forge, import `main`, fork `feature` at the second commit (the one that
/// introduced the dotfiles), seal one change there and fuse `main` back in.
fn imported_repo_with_fused_feature(dir: &Path, work: &Path) -> (Repo, u64, u64) {
    crate::forge::forge(dir).unwrap();
    let mut repo = Repo::open(dir).unwrap();
    import_fetch_result(&mut repo, &fetch_all(work, "main")).unwrap();

    let main_head = repo.get_timeline_head("main").unwrap().unwrap();
    let c2_idx = repo.get_leaf(main_head).unwrap().unwrap().prev_idx;
    repo.create_timeline("feature", Some("main")).unwrap();
    repo.set_timeline_head("feature", c2_idx).unwrap();
    repo.switch_timeline("feature").unwrap();
    let seal = seal_change(
        &mut repo,
        "feature.txt",
        b"feature\n",
        "feat: added updates",
    );

    let crate::fuse_op::Planned::Plan(plan) =
        crate::fuse_op::plan(&repo, "main", crate::fuse::Strategy::Auto).unwrap()
    else {
        panic!("main should not already be fused into feature");
    };
    assert!(plan.conflicts.is_empty());
    let fuse = crate::fuse_op::seal(&mut repo, &plan).unwrap().index;
    (repo, seal, fuse)
}

/// Commits on the remote's `feature` that `main` does not have.
fn ahead_of_main(bare: &Path) -> Vec<String> {
    git(bare, &["rev-list", "feature", "^main"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn parents(bare: &Path, sha: &str) -> Vec<String> {
    git(bare, &["rev-list", "--parents", "-n", "1", sha])
        .split(' ')
        .skip(1)
        .map(str::to_string)
        .collect()
}

#[test]
fn upload_reuses_imported_shas_when_remote_main_moved_on() {
    if !have_git() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, [c1, c2, c3]) = git_fixture(tmp.path());
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    let (repo, seal, fuse) = imported_repo_with_fused_feature(&local, &work);

    // Someone else pushes to main; this repository never downloads it, so the
    // advertised main tip maps to nothing here. This is what left the export
    // with no proof the server had the imported history, and it re-minted it.
    write(&work, "src/other.rs", "fn c() {}\n");
    let c4 = commit_all(&work, "someone else's work");
    git(&work, &["push", "-q", bare.to_str().unwrap(), "main"]);

    upload(&repo, "feature", &bare).unwrap();

    let ahead = ahead_of_main(&bare);
    assert_eq!(
        ahead.len(),
        2,
        "only the new seal and its fuse may be new on the remote, got {ahead:?}"
    );
    let feature_tip = git(&bare, &["rev-parse", "feature"]);
    let tip_parents = parents(&bare, &feature_tip);
    assert_eq!(tip_parents.len(), 2, "the fuse stays a merge");
    assert_eq!(tip_parents[1], c3, "the fuse merges main's real commit");
    assert_eq!(
        parents(&bare, &tip_parents[0]),
        vec![c2.clone()],
        "the new seal sits on the imported commit, not a re-minted copy"
    );
    let history: BTreeSet<String> = git(&bare, &["rev-list", "feature"])
        .lines()
        .map(str::to_string)
        .collect();
    assert!(history.contains(&c1) && history.contains(&c2) && history.contains(&c3));
    assert!(!history.contains(&c4));

    assert_commit_identities(&repo, 1);
    let map = HashMapping::new(&repo.ivaldi_dir);
    for idx in [seal, fuse] {
        let leaf = repo.get_leaf(idx).unwrap().unwrap();
        assert!(
            map.get_sha1(leaf.hash()).is_some(),
            "uploaded seal {idx} is mapped"
        );
    }

    // Uploading again is a no-op, not a second copy.
    assert_eq!(
        upload(&repo, "feature", &bare).unwrap_err(),
        "nothing to push"
    );
    assert_eq!(ahead_of_main(&bare).len(), 2);
}

#[test]
fn upload_with_many_to_one_map_reuses_original_shas() {
    if !have_git() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, [_c1, c2, c3]) = git_fixture(tmp.path());
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    let (repo, _seal, _fuse) = imported_repo_with_fused_feature(&local, &work);

    // Damage the map the way an affected repository has it: every imported
    // seal also answers to a second, re-minted SHA. `ff…` sorts after every
    // real id, so a reload makes the copy win the reverse lookup.
    let mut map = HashMapping::new(&repo.ivaldi_dir);
    for idx in 0..repo.commit_count() {
        let leaf = repo.get_leaf(idx).unwrap().unwrap();
        if leaf.meta.contains_key("git.sha1") {
            map.insert(&format!("ff{:038x}", idx), leaf.hash());
        }
    }
    map.save().unwrap();
    assert_commit_identities(&repo, 2);

    upload(&repo, "feature", &bare).unwrap();

    let ahead = ahead_of_main(&bare);
    assert_eq!(
        ahead.len(),
        2,
        "no imported commit may be re-minted: {ahead:?}"
    );
    let feature_tip = git(&bare, &["rev-parse", "feature"]);
    let tip_parents = parents(&bare, &feature_tip);
    assert_eq!(tip_parents[1], c3);
    assert_eq!(parents(&bare, &tip_parents[0]), vec![c2]);
    // The damage is not made worse: no imported seal gained a third identity.
    assert_commit_identities(&repo, 2);
}

#[test]
fn upload_to_an_empty_portal_refuses_history_it_cannot_reproduce() {
    if !have_git() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (work, _bare, _) = git_fixture(tmp.path());
    let empty = tmp.path().join("empty.git");
    git(
        tmp.path(),
        &["init", "-q", "--bare", empty.to_str().unwrap()],
    );
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    let (repo, _seal, _fuse) = imported_repo_with_fused_feature(&local, &work);

    let err = upload(&repo, "feature", &empty).unwrap_err();
    assert!(
        err.contains("cannot be reproduced"),
        "a lossy imported commit must not be re-minted for a new portal: {err}"
    );
    assert_eq!(
        git(&empty, &["for-each-ref"]),
        "",
        "nothing reached the portal"
    );
    assert_commit_identities(&repo, 1);
}

#[test]
fn upload_to_an_empty_portal_mirrors_an_exact_history_unchanged() {
    if !have_git() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (work, _bare, [c1, c2, c3]) = git_fixture_with(tmp.path(), false);
    let empty = tmp.path().join("empty.git");
    git(
        tmp.path(),
        &["init", "-q", "--bare", empty.to_str().unwrap()],
    );
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&local).unwrap();
    let (repo, _seal, _fuse) = imported_repo_with_fused_feature(&local, &work);

    // With nothing lost on import, every imported seal rebuilds its original
    // commit byte for byte, so a new portal receives the very same history.
    upload(&repo, "feature", &empty).unwrap();
    let history: Vec<String> = git(&empty, &["rev-list", "feature"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        history.len(),
        5,
        "c1..c3, the seal and the fuse: {history:?}"
    );
    for original in [&c1, &c2, &c3] {
        assert!(history.contains(original), "{original} was re-minted");
    }
    assert_commit_identities(&repo, 1);
}
