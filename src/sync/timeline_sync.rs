//! Smart incremental sync of a local timeline from a remote branch:
//! up-to-date detection, fast-forward import, and diverged auto-fuse.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::atomic_io::atomic_write;
use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::github::{CommitInfo, GitHubClient};
use crate::leaf::Leaf;
use crate::refname::timeline_ref_path;
use crate::remote::HashMapping;
use crate::repo::Repo;

use super::import::{import_full_history, import_full_history_into};
use super::{
    SyncError, checkout_tree_to_workspace, compute_file_changes, compute_workspace_delta,
    get_tree_files,
};

const SYNC_JOURNAL: &str = "sync-journal.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct SyncJournal {
    timeline: String,
    temp_timeline: String,
    remote_tip_sha: String,
    local_head: u64,
    remote_head: u64,
}

/// Result of a sync (delta update) operation.
#[derive(Debug)]
pub struct SyncResult {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub no_changes: bool,
    pub was_fast_forward: bool,
    pub was_fused: bool,
    pub conflicts: Vec<String>,
}

/// Sync — smart incremental update of a local timeline from remote.
///
/// Detects whether the local and remote have diverged:
/// - **Up to date:** no new remote commits → no-op
/// - **Fast-forward:** remote has new commits, local hasn't diverged → import + advance
/// - **Diverged:** both have new commits → fuse
///
/// Consent-first: fetching and inspecting remote state is free, but once
/// the incoming/local seal counts are known and integration would mutate
/// the timeline, `consent(incoming, local)` is asked exactly once —
/// `false` aborts with [`SyncError::Declined`] and no local mutation.
pub fn sync_timeline(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
    consent: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<SyncResult, SyncError> {
    recover_interrupted_sync(repo)?;
    let branches = client.list_branches(owner, repo_name)?;
    let branch = branches
        .iter()
        .find(|b| b.name == timeline)
        .ok_or_else(|| SyncError::Other(format!("remote branch '{}' not found", timeline)))?;

    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);

    // Fetch remote commits
    let remote_commits = client.list_commits(owner, repo_name, timeline, 0)?;

    if remote_commits.is_empty() {
        return Ok(up_to_date_result());
    }

    // Get local head BEFORE import so we can constrain ancestor search
    let local_head_idx = repo.get_timeline_head(timeline)?;
    let local_reachable = collect_local_reachable(repo, local_head_idx);

    // Track commits with stale mappings so we skip them on re-search
    let mut stale_shas: BTreeSet<String> = BTreeSet::new();

    let (mut common_ancestor_sha, mut common_ancestor_idx) = find_common_ancestor(
        repo,
        &remote_commits,
        &hash_mapping,
        &local_reachable,
        &stale_shas,
    );

    // Stale-mapping detection: when the common ancestor IS the remote tip
    // (i.e., sync would say "up to date"), verify that the local tree
    // actually matches the remote tree.  A mismatch means a previous buggy
    // sync created a wrong fuse commit and mapped the remote tip to it.
    if common_ancestor_sha.as_ref() == Some(&remote_commits[0].sha)
        && let Some(ca_idx) = common_ancestor_idx
        && remote_tip_mapping_is_stale(client, repo, owner, repo_name, &remote_commits[0], ca_idx)?
    {
        // Stale mapping: remove it and re-search for the real ancestor
        let stale_sha = remote_commits[0].sha.clone();
        hash_mapping.remove_sha1(&stale_sha);
        hash_mapping.save()?;
        stale_shas.insert(stale_sha);

        (common_ancestor_sha, common_ancestor_idx) = find_common_ancestor(
            repo,
            &remote_commits,
            &hash_mapping,
            &local_reachable,
            &stale_shas,
        );
    }

    let new_remote_count =
        count_new_remote_commits(&remote_commits, common_ancestor_sha.as_deref());
    let new_local_count =
        count_new_local_commits(repo, timeline, local_head_idx, common_ancestor_idx)?;

    if new_remote_count == 0 {
        return Ok(up_to_date_result());
    }

    // Sync rewrites the workspace to match the incoming tree (overwriting and
    // deleting files), so any uncommitted change would be silently lost.
    // Refuse before touching anything if the working tree is dirty.
    ensure_workspace_clean(repo, local_head_idx)?;

    // Consent gate: everything above only fetched and compared; everything
    // below mutates the user's timeline. Ivaldi never integrates remote
    // changes without permission.
    if !consent(new_remote_count, new_local_count as usize) {
        return Err(SyncError::Declined);
    }

    // Classify: fast-forward or diverged
    if new_local_count == 0 {
        return sync_fast_forward(
            client,
            repo,
            owner,
            repo_name,
            timeline,
            common_ancestor_idx,
        );
    }

    sync_diverged(
        client,
        repo,
        owner,
        repo_name,
        timeline,
        common_ancestor_idx,
        &branch.commit.sha,
    )
}

/// `checkout_tree_to_workspace` (used by every sync path) makes the workspace
/// exactly match the incoming tree: it overwrites modified files and deletes any
/// on-disk file not in that tree, including untracked ones. That is only safe
/// when the working tree already matches HEAD. Enforce that precondition so sync
/// can never destroy uncommitted work.
fn ensure_workspace_clean(repo: &Repo, local_head_idx: Option<u64>) -> Result<(), SyncError> {
    let head_tree = match local_head_idx {
        Some(idx) => repo.get_leaf(idx)?.map(|l| l.tree_root),
        None => None,
    };

    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let ws = crate::workspace::Workspace::new(&cas, &repo.work_dir, &repo.ivaldi_dir);
    let ignore = crate::ignore::load_pattern_cache(&repo.work_dir);
    let dirty: Vec<String> = ws
        .status(head_tree, &ignore)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .into_iter()
        .filter(|f| f.state != crate::workspace::FileState::Unmodified)
        .map(|f| f.path)
        .collect();

    if dirty.is_empty() {
        return Ok(());
    }

    let shown: Vec<String> = dirty.iter().take(10).cloned().collect();
    let more = dirty.len() - shown.len();
    let suffix = if more > 0 {
        format!("\n  ... and {} more", more)
    } else {
        String::new()
    };
    Err(SyncError::Other(format!(
        "you have uncommitted changes that sync would overwrite:\n  {}{}\n\n\
         Seal them ('ivaldi seal') or throw them away ('ivaldi discard') before syncing.",
        shown.join("\n  "),
        suffix
    )))
}

/// A `SyncResult` for the nothing-to-do case.
fn up_to_date_result() -> SyncResult {
    SyncResult {
        added: vec![],
        modified: vec![],
        deleted: vec![],
        no_changes: true,
        was_fast_forward: false,
        was_fused: false,
        conflicts: vec![],
    }
}

/// Sync step 1: build the set of leaf indices reachable from the local
/// timeline head. This prevents matching commits from OTHER timelines that
/// happen to be in the hash_mapping (e.g. after uploading a feature branch
/// whose commits later appear on main via a merge).
fn collect_local_reachable(repo: &Repo, local_head_idx: Option<u64>) -> BTreeSet<u64> {
    let mut reachable = BTreeSet::new();
    if let Some(head) = local_head_idx {
        let mut cur = Some(head);
        while let Some(idx) = cur {
            reachable.insert(idx);
            if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                // Follow both prev_idx and merge parents
                for &midx in &leaf.merge_idxs {
                    // Shallow: just add direct merge parents
                    reachable.insert(midx);
                }
                cur = if leaf.has_parent() {
                    Some(leaf.prev_idx)
                } else {
                    None
                };
            } else {
                break;
            }
        }
    }
    reachable
}

/// Sync step 2: find the common ancestor — walk remote commits
/// newest→oldest, check the hash mapping, and only accept leaves that are
/// reachable from the local timeline head. Commits in `stale_shas` are
/// skipped (their mappings were found to be wrong).
fn find_common_ancestor(
    repo: &Repo,
    remote_commits: &[CommitInfo],
    hash_mapping: &HashMapping,
    local_reachable: &BTreeSet<u64>,
    stale_shas: &BTreeSet<String>,
) -> (Option<String>, Option<u64>) {
    for commit in remote_commits {
        if stale_shas.contains(&commit.sha) {
            continue;
        }
        if let Some(b3) = hash_mapping.get_blake3(&commit.sha) {
            // Find leaf index with this hash
            for idx in 0..repo.commit_count() {
                if let Ok(Some(leaf)) = repo.get_leaf(idx)
                    && leaf.hash() == b3
                    && local_reachable.contains(&idx)
                {
                    return (Some(commit.sha.clone()), Some(idx));
                }
            }
        }
    }
    (None, None)
}

/// Sync step 3: compare the remote tip tree's path set against the local
/// tree at the supposed common ancestor. A mismatch means a previous buggy
/// sync created a wrong fuse commit and mapped the remote tip to it.
fn remote_tip_mapping_is_stale(
    client: &GitHubClient,
    repo: &Repo,
    owner: &str,
    repo_name: &str,
    remote_tip: &CommitInfo,
    ca_idx: u64,
) -> Result<bool, SyncError> {
    if repo
        .get_leaf(ca_idx)?
        .and_then(|leaf| leaf.meta.get("sync.remote_tip").cloned())
        .as_deref()
        == Some(remote_tip.sha.as_str())
    {
        // A fuse intentionally contains local paths that are absent remotely.
        // Its authenticated marker distinguishes that from an accidental
        // mapping of an ordinary imported commit to the wrong tree.
        return Ok(false);
    }
    let remote_tree = client.get_tree(owner, repo_name, &remote_tip.commit.tree.sha)?;
    let remote_paths: BTreeSet<&str> = remote_tree
        .tree
        .iter()
        .filter(|e| e.entry_type == "blob")
        .map(|e| e.path.as_str())
        .collect();

    let verify_cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let verify_store = FsStore::new(&verify_cas);
    let local_files = get_tree_files(repo, &verify_store, ca_idx)?;
    let local_paths: BTreeSet<&str> = local_files.keys().map(|s| s.as_str()).collect();

    Ok(remote_paths != local_paths)
}

/// Sync step 4: count new remote commits (those before the common ancestor
/// in the newest-first list).
fn count_new_remote_commits(remote_commits: &[CommitInfo], ca_sha: Option<&str>) -> usize {
    match ca_sha {
        Some(ca_sha) => remote_commits
            .iter()
            .take_while(|c| c.sha != ca_sha)
            .count(),
        None => remote_commits.len(),
    }
}

/// Sync step 5: count local commits since the common ancestor.
fn count_new_local_commits(
    repo: &Repo,
    timeline: &str,
    local_head_idx: Option<u64>,
    common_ancestor_idx: Option<u64>,
) -> Result<u64, SyncError> {
    match (local_head_idx, common_ancestor_idx) {
        (Some(head), Some(ancestor)) => {
            // Walk from head back to ancestor, counting steps
            let mut count = 0u64;
            let mut cur = Some(head);
            while let Some(idx) = cur {
                if idx == ancestor {
                    break;
                }
                if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                    count += 1;
                    cur = if leaf.has_parent() {
                        Some(leaf.prev_idx)
                    } else {
                        None
                    };
                } else {
                    break;
                }
            }
            Ok(count)
        }
        (Some(_), None) => {
            // No common ancestor: all local commits are "new"
            Ok(repo.walk_history(timeline)?.len() as u64)
        }
        _ => Ok(0),
    }
}

/// Sync fast-forward path: import remote commits and advance the workspace.
fn sync_fast_forward(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
    common_ancestor_idx: Option<u64>,
) -> Result<SyncResult, SyncError> {
    let _import = import_full_history(client, repo, owner, repo_name, timeline, 0)?;

    // Compute file changes for the result
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let store = FsStore::new(&cas);

    let (added, modified, deleted) =
        compute_workspace_delta(repo, &store, timeline, common_ancestor_idx)?;

    // Update workspace files
    checkout_tree_to_workspace(repo, &store, timeline)?;

    Ok(SyncResult {
        added,
        modified,
        deleted,
        no_changes: false,
        was_fast_forward: true,
        was_fused: false,
        conflicts: vec![],
    })
}

/// Sync diverged path: import remote commits into a temp timeline, three-way
/// fuse against the common ancestor, then clean up the temp timeline.
fn sync_diverged(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
    common_ancestor_idx: Option<u64>,
    remote_tip_sha: &str,
) -> Result<SyncResult, SyncError> {
    // Same value as captured before the ancestor search: nothing between
    // there and here mutates this timeline's head.
    let local_head_idx = repo.get_timeline_head(timeline)?;

    let temp_timeline = format!("__sync_{}", timeline);

    // A previous sync that crashed after its fuse commit leaves the temp
    // timeline stranded (cleanup is best-effort). Remove any leftover before
    // reuse so stale state can't leak into this sync.
    cleanup_temp_timeline(repo, &temp_timeline);

    // Create temp timeline pointing at the common ancestor, if known
    if let Some(ancestor_idx) = common_ancestor_idx {
        create_temp_timeline(repo, &temp_timeline, ancestor_idx)?;
    }
    crate::failpoint::fail_point("sync.after_temp_timeline");

    // Import remote history into temp timeline (fetch from real remote branch)
    let _import =
        import_full_history_into(client, repo, owner, repo_name, timeline, &temp_timeline, 0)?;

    // Get file sets for three-way merge
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let store = FsStore::new(&cas);

    let base_files = if let Some(ancestor_idx) = common_ancestor_idx {
        get_tree_files(repo, &store, ancestor_idx)?
    } else {
        BTreeMap::new()
    };

    let our_files = if let Some(head_idx) = local_head_idx {
        get_tree_files(repo, &store, head_idx)?
    } else {
        BTreeMap::new()
    };

    let their_head_idx = repo.get_timeline_head(&temp_timeline)?;
    let their_files = if let Some(idx) = their_head_idx {
        get_tree_files(repo, &store, idx)?
    } else {
        BTreeMap::new()
    };

    // Auto-fuse
    let fuse_result = crate::fuse::FuseEngine::fuse(
        &store,
        &base_files,
        &our_files,
        &their_files,
        crate::fuse::Strategy::Auto,
    );

    if !fuse_result.success {
        // Conflicts — save merge state, report
        let conflicts: Vec<String> = fuse_result
            .conflicts
            .iter()
            .map(|c| c.path.clone())
            .collect();
        return save_sync_conflicts(repo, &temp_timeline, timeline, conflicts);
    }

    // Build merged tree
    let merged_tree = store.build_tree_from_hash_map(&fuse_result.merged_files)?;

    // Create fuse commit
    let our_head = local_head_idx.unwrap_or(crate::leaf::NO_PARENT);
    let their_head = their_head_idx.unwrap_or(crate::leaf::NO_PARENT);

    let mut fuse_leaf = Leaf::new(
        merged_tree,
        timeline,
        "ivaldi-sync",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        format!(
            "Fused sync from {}/{} (branch: {})",
            owner, repo_name, timeline
        ),
    );
    fuse_leaf.prev_idx = our_head;
    if their_head != crate::leaf::NO_PARENT {
        fuse_leaf.merge_idxs = vec![their_head];
    }
    fuse_leaf
        .meta
        .insert("sync.remote_tip".into(), remote_tip_sha.to_string());

    // The merged tree nodes were written to the CAS without their directory
    // entries being durable; flush before the commit record references them.
    cas.flush()?;
    let journal = SyncJournal {
        timeline: timeline.to_string(),
        temp_timeline: temp_timeline.clone(),
        remote_tip_sha: remote_tip_sha.to_string(),
        local_head: our_head,
        remote_head: their_head,
    };
    let journal_bytes = serde_json::to_vec(&journal)
        .map_err(|e| SyncError::Other(format!("cannot encode sync journal: {e}")))?;
    atomic_write(&repo.ivaldi_dir.join(SYNC_JOURNAL), &journal_bytes)?;
    repo.commit_raw(fuse_leaf, timeline)?;
    crate::failpoint::fail_point("sync.after_fuse_commit");

    // Update workspace
    checkout_tree_to_workspace(repo, &store, timeline)?;

    // Map remote tip SHA to the fuse commit so the next sync recognizes it
    crate::failpoint::fail_point("sync.before_tip_remap");
    map_remote_tip_to_head(repo, timeline, remote_tip_sha)?;

    cleanup_temp_timeline(repo, &temp_timeline);
    let _ = fs::remove_file(repo.ivaldi_dir.join(SYNC_JOURNAL));
    crate::failpoint::fail_point("sync.after_cleanup");

    let (added, modified, deleted) = compute_file_changes(&base_files, &fuse_result.merged_files);

    Ok(SyncResult {
        added,
        modified,
        deleted,
        no_changes: false,
        was_fast_forward: false,
        was_fused: true,
        conflicts: vec![],
    })
}

/// Finish or roll back the small cross-file window around a diverged sync.
/// The append-only fuse transaction is authoritative only when its parents
/// exactly match the journal; otherwise no fuse landed and the temp state is
/// discarded. Repeating this recovery is harmless.
fn recover_interrupted_sync(repo: &mut Repo) -> Result<(), SyncError> {
    let path = repo.ivaldi_dir.join(SYNC_JOURNAL);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let journal: SyncJournal = serde_json::from_slice(&bytes)
        .map_err(|e| SyncError::Other(format!("corrupt sync journal: {e}")))?;
    crate::refname::validate_timeline_name(&journal.timeline)
        .map_err(|e| SyncError::Other(format!("unsafe sync journal timeline: {e}")))?;
    crate::refname::validate_timeline_name(&journal.temp_timeline)
        .map_err(|e| SyncError::Other(format!("unsafe sync journal temp timeline: {e}")))?;

    let landed = repo
        .get_timeline_head(&journal.timeline)?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten().map(|leaf| (idx, leaf)))
        .filter(|(_, leaf)| {
            leaf.prev_idx == journal.local_head && leaf.merge_idxs.contains(&journal.remote_head)
        });
    if landed.is_some() {
        map_remote_tip_to_head(repo, &journal.timeline, &journal.remote_tip_sha)?;
        let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
        let store = FsStore::new(&cas);
        checkout_tree_to_workspace(repo, &store, &journal.timeline)?;
    }
    cleanup_temp_timeline(repo, &journal.temp_timeline);
    fs::remove_file(&path)?;
    Ok(())
}

/// Create the temp sync timeline pointing at the common ancestor.
fn create_temp_timeline(
    repo: &Repo,
    temp_timeline: &str,
    ancestor_idx: u64,
) -> Result<(), SyncError> {
    let ref_path = timeline_ref_path(&repo.ivaldi_dir, temp_timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    if let Some(parent) = ref_path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(&ref_path, b"")?;
    repo.store.set_timeline_head(temp_timeline, ancestor_idx)?;
    Ok(())
}

/// Best-effort removal of the temp sync timeline (head entry + ref file).
fn cleanup_temp_timeline(repo: &Repo, temp_timeline: &str) {
    let _ = repo.store.remove_timeline_head(temp_timeline);
    if let Ok(path) = timeline_ref_path(&repo.ivaldi_dir, temp_timeline) {
        let _ = fs::remove_file(path);
    }
}

/// Map the remote tip SHA to the freshly created fuse commit at the timeline
/// head.
fn map_remote_tip_to_head(
    repo: &Repo,
    timeline: &str,
    remote_tip_sha: &str,
) -> Result<(), SyncError> {
    let head_idx = repo
        .get_timeline_head(timeline)?
        .ok_or_else(|| SyncError::Other("timeline head missing after merge".into()))?;
    let merged_leaf = repo
        .get_leaf(head_idx)?
        .ok_or_else(|| SyncError::Other("merged leaf missing after merge".into()))?;
    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);
    hash_mapping.insert(remote_tip_sha, merged_leaf.hash());
    hash_mapping.save()?;
    Ok(())
}

/// Save merge state for a conflicted sync so the user can resolve and
/// continue.
fn save_sync_conflicts(
    repo: &Repo,
    temp_timeline: &str,
    timeline: &str,
    conflicts: Vec<String>,
) -> Result<SyncResult, SyncError> {
    let merge_state = crate::repo::MergeState {
        source_timeline: temp_timeline.to_string(),
        target_timeline: timeline.to_string(),
        strategy: "auto".into(),
        conflicts: conflicts.clone(),
    };
    repo.save_merge_state(&merge_state)?;

    Ok(SyncResult {
        added: vec![],
        modified: vec![],
        deleted: vec![],
        no_changes: false,
        was_fast_forward: false,
        was_fused: false,
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{AuthorInfo, CommitDetail, ParentRef, TreeRef};

    fn commit_info(sha: &str, parents: &[&str]) -> CommitInfo {
        CommitInfo {
            sha: sha.to_string(),
            commit: CommitDetail {
                message: "msg".into(),
                author: AuthorInfo {
                    name: "A".into(),
                    email: "a@x".into(),
                    date: None,
                },
                tree: TreeRef { sha: "t".into() },
            },
            parents: parents
                .iter()
                .map(|p| ParentRef { sha: p.to_string() })
                .collect(),
        }
    }

    /// Forge a repo with a 3-seal chain on main; returns the repo.
    fn repo_with_chain() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();
        let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = FsStore::new(&cas);
        for i in 0..3u8 {
            let (blob, _) = store.put_blob(&[i]).unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree = store
                .put_tree(vec![Entry {
                    name: "f".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob,
                }])
                .unwrap();
            repo.commit(tree, "t <t@x>", &format!("c{}", i)).unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn count_new_remote_commits_stops_at_common_ancestor() {
        let commits = vec![
            commit_info("tip", &["mid"]),
            commit_info("mid", &["base"]),
            commit_info("base", &[]),
        ];
        assert_eq!(count_new_remote_commits(&commits, Some("mid")), 1);
        assert_eq!(count_new_remote_commits(&commits, Some("tip")), 0);
        assert_eq!(count_new_remote_commits(&commits, None), 3);
        assert_eq!(count_new_remote_commits(&commits, Some("unknown")), 3);
    }

    #[test]
    fn collect_local_reachable_walks_prev_chain() {
        let (_dir, repo) = repo_with_chain();
        let reachable = collect_local_reachable(&repo, Some(2));
        assert!(reachable.contains(&0) && reachable.contains(&1) && reachable.contains(&2));
        assert_eq!(collect_local_reachable(&repo, None).len(), 0);
    }

    #[test]
    fn count_new_local_commits_counts_steps_to_ancestor() {
        let (_dir, repo) = repo_with_chain();
        assert_eq!(
            count_new_local_commits(&repo, "main", Some(2), Some(0)).unwrap(),
            2
        );
        assert_eq!(
            count_new_local_commits(&repo, "main", Some(2), Some(2)).unwrap(),
            0
        );
        // No common ancestor: every local commit is new.
        assert_eq!(
            count_new_local_commits(&repo, "main", Some(2), None).unwrap(),
            3
        );
    }

    #[test]
    fn find_common_ancestor_skips_stale_and_unreachable() {
        let (_dir, repo) = repo_with_chain();
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let leaf1 = repo.get_leaf(1).unwrap().unwrap();
        mapping.insert("remote-mid", leaf1.hash());
        // Also map the tip to a leaf NOT reachable from the searched head.
        let leaf2 = repo.get_leaf(2).unwrap().unwrap();
        mapping.insert("remote-tip", leaf2.hash());

        let commits = vec![
            commit_info("remote-tip", &["remote-mid"]),
            commit_info("remote-mid", &["remote-base"]),
        ];
        // Reachable set limited to leaves 0-1: the tip's mapping (leaf 2) is
        // outside it, so the mid commit must win.
        let reachable: BTreeSet<u64> = [0u64, 1].into();
        let (sha, idx) =
            find_common_ancestor(&repo, &commits, &mapping, &reachable, &BTreeSet::new());
        assert_eq!(sha.as_deref(), Some("remote-mid"));
        assert_eq!(idx, Some(1));

        // Marking the mid stale skips it entirely.
        let stale: BTreeSet<String> = ["remote-mid".to_string(), "remote-tip".to_string()].into();
        let (sha, idx) = find_common_ancestor(&repo, &commits, &mapping, &reachable, &stale);
        assert_eq!(sha, None);
        assert_eq!(idx, None);
    }

    /// Sync must refuse when the workspace has uncommitted changes, since the
    /// checkout would overwrite modified files and delete untracked ones.
    #[test]
    fn ensure_workspace_clean_refuses_uncommitted_changes() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();
        let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = FsStore::new(&cas);

        let mut files = BTreeMap::new();
        files.insert("a.txt".to_string(), b"hello".to_vec());
        let tree = store.build_tree_from_map(&files).unwrap();
        repo.commit(tree, "author", "init").unwrap();
        let head = repo.get_timeline_head("main").unwrap();

        // Make the workspace match HEAD, then it must pass.
        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert!(ensure_workspace_clean(&repo, head).is_ok());

        // An uncommitted edit must block sync, naming the file.
        fs::write(dir.path().join("a.txt"), b"local edit").unwrap();
        let err = ensure_workspace_clean(&repo, head).unwrap_err();
        assert!(
            err.to_string().contains("a.txt"),
            "error should name the dirty file: {err}"
        );

        // An untracked file must also block sync (it would be deleted).
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::write(dir.path().join("new.txt"), b"local only").unwrap();
        assert!(ensure_workspace_clean(&repo, head).is_err());
    }

    /// A stranded `__sync_<name>` timeline from a crashed sync must be
    /// removable, and cleanup must leave the repo fully consistent.
    #[test]
    fn stranded_temp_timeline_cleanup_is_safe() {
        let (dir, repo) = repo_with_chain();
        create_temp_timeline(&repo, "__sync_main", 1).unwrap();
        assert_eq!(repo.get_timeline_head("__sync_main").unwrap(), Some(1));

        cleanup_temp_timeline(&repo, "__sync_main");
        assert!(repo.get_timeline_head("__sync_main").unwrap().is_none());
        assert!(
            !timeline_ref_path(&repo.ivaldi_dir, "__sync_main")
                .unwrap()
                .exists()
        );
        drop(repo);
        let report = crate::verify::verify(dir.path(), true);
        assert!(report.ok, "verify --full failed: {:?}", report.checks);
    }
}
