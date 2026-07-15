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
/// - **Diverged:** both have new commits → auto-fuse (Ivaldi's auto-merge philosophy)
pub fn sync_timeline(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
) -> Result<SyncResult, SyncError> {
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

    // Create temp timeline pointing at the common ancestor, if known
    if let Some(ancestor_idx) = common_ancestor_idx {
        create_temp_timeline(repo, &temp_timeline, ancestor_idx)?;
    }

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
            .unwrap()
            .as_secs() as i64,
        format!(
            "Fused sync from {}/{} (branch: {})",
            owner, repo_name, timeline
        ),
    );
    fuse_leaf.prev_idx = our_head;
    if their_head != crate::leaf::NO_PARENT {
        fuse_leaf.merge_idxs = vec![their_head];
    }

    repo.commit_raw(fuse_leaf, timeline)?;

    // Update workspace
    checkout_tree_to_workspace(repo, &store, timeline)?;

    // Map remote tip SHA to the fuse commit so the next sync recognizes it
    map_remote_tip_to_head(repo, timeline, remote_tip_sha)?;

    cleanup_temp_timeline(repo, &temp_timeline);

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
