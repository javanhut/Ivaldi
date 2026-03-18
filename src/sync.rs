//! Sync operations for Ivaldi VCS — download, upload, scout, harvest.
//!
//! Bridges Ivaldi's internal BLAKE3-based storage with GitHub's SHA1-based
//! Git objects. SHA1 is used ONLY for API communication — never internally.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::github::{GitHubClient, GitHubError, TreeEntryCreate};
use crate::hash::B3Hash;
use crate::remote::HashMapping;
use crate::repo::Repo;
use crate::workspace::Workspace;
use crate::ignore;

/// Result of a download (clone) operation.
#[derive(Debug)]
pub struct DownloadResult {
    pub files_downloaded: usize,
    pub commits_imported: usize,
    pub timelines_created: Vec<String>,
}

/// Result of an upload (push) operation.
#[derive(Debug)]
pub struct UploadResult {
    pub files_uploaded: usize,
    pub commit_sha: String,
    pub branch: String,
}

/// Download a repository from GitHub into a local Ivaldi repo.
pub fn download(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    target_dir: &Path,
    branch: Option<&str>,
) -> Result<DownloadResult, SyncError> {
    if !client.is_authenticated() {
        // Try unauthenticated for public repos
    }

    eprintln!("Downloading {}/{}...", owner, repo_name);

    // Get repo info
    let repo_info = client
        .get_repo(owner, repo_name)
        .map_err(SyncError::GitHub)?;
    let default_branch = branch.unwrap_or(&repo_info.default_branch);

    // Initialize Ivaldi repo at target
    crate::forge::forge(target_dir).map_err(|e| SyncError::Other(e.to_string()))?;

    let ivaldi_dir = target_dir.join(".ivaldi");

    // Configure portal
    let portal_mgr = crate::portal::PortalManager::new(&ivaldi_dir);
    let portal = crate::portal::Portal::parse(&format!("{}/{}", owner, repo_name)).unwrap();
    let _ = portal_mgr.add(&portal);

    // Get the tree for the default branch
    let branches = client.list_branches(owner, repo_name).map_err(SyncError::GitHub)?;
    let branch_info = branches
        .iter()
        .find(|b| b.name == default_branch)
        .ok_or_else(|| SyncError::Other(format!("branch '{}' not found", default_branch)))?;

    let tree = client
        .get_tree(owner, repo_name, &branch_info.commit.sha)
        .map_err(SyncError::GitHub)?;

    // Download all blob files
    let cas = FileCas::new(ivaldi_dir.join("objects"))
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let store = FsStore::new(&cas);
    let mut hash_mapping = HashMapping::new(&ivaldi_dir);
    let mut file_count = 0;

    let blob_entries: Vec<_> = tree.tree.iter().filter(|e| e.entry_type == "blob").collect();
    let total = blob_entries.len();
    let pb = crate::progress::file_bar(total as u64, "Downloading");

    for entry in &blob_entries {
        pb.inc(1);

        match client.download_file(owner, repo_name, &entry.path, &branch_info.commit.sha) {
            Ok(content) => {
                let (blob_hash, _) = store
                    .put_blob(&content)
                    .map_err(|e| SyncError::Other(e.to_string()))?;

                hash_mapping.insert(&entry.sha, blob_hash);

                let file_path = target_dir.join(&entry.path);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).ok();
                }
                fs::write(&file_path, &content)
                    .map_err(|e| SyncError::Other(e.to_string()))?;

                file_count += 1;
            }
            Err(e) => {
                crate::logging::warn(&format!("failed to download {}: {}", entry.path, e));
            }
        }
    }
    pb.finish_with_message(format!("{} files downloaded", file_count));

    // Save hash mapping
    hash_mapping.save().map_err(|e| SyncError::Other(e.to_string()))?;

    // Create initial commit from downloaded files
    let mut repo = Repo::open(target_dir).map_err(|e| SyncError::Other(e.to_string()))?;

    // Set up config
    let mut cfg = crate::config::Config::new();
    cfg.set("portal.default", &format!("{}/{}", owner, repo_name));
    cfg.save(&ivaldi_dir.join("config")).ok();

    // Gather and seal all downloaded files
    let ignore_cache = ignore::load_pattern_cache(target_dir);
    let mut ws = Workspace::new(&cas, target_dir, &ivaldi_dir);
    ws.gather_all(&ignore_cache).map_err(|e| SyncError::Other(e.to_string()))?;

    if !ws.staging.is_empty() {
        let tree_hash = ws.build_staged_tree().map_err(|e| SyncError::Other(e.to_string()))?;

        // Commit with repo info
        let author = format!("ivaldi-download <download@ivaldi>");
        let message = format!("Downloaded from {}/{} (branch: {})", owner, repo_name, default_branch);
        repo.commit(tree_hash, &author, &message)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        ws.staging.clear();
        ws.save().map_err(|e| SyncError::Other(e.to_string()))?;
    }

    eprintln!("Downloaded {} files from {}/{}", file_count, owner, repo_name);

    Ok(DownloadResult {
        files_downloaded: file_count,
        commits_imported: 1,
        timelines_created: vec![default_branch.to_string()],
    })
}

/// Upload (push) the current timeline to GitHub.
pub fn upload(
    client: &GitHubClient,
    repo: &Repo,
    owner: &str,
    repo_name: &str,
    branch: Option<&str>,
    force: bool,
) -> Result<UploadResult, SyncError> {
    if !client.is_authenticated() {
        return Err(SyncError::GitHub(GitHubError::AuthRequired));
    }

    let timeline = repo.current_timeline().map_err(|e| SyncError::Other(e.to_string()))?;
    let branch_name = branch.unwrap_or(&timeline);

    // Get head leaf
    let head_idx = repo
        .get_timeline_head(&timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .ok_or_else(|| SyncError::Other("no commits to upload".into()))?;

    let head_leaf = repo
        .get_leaf(head_idx)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .ok_or_else(|| SyncError::Other("corrupt: head leaf not found".into()))?;

    // Build file list from tree
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let store = FsStore::new(&cas);

    let mut files = BTreeMap::new();
    collect_tree_files(&store, head_leaf.tree_root, "", &mut files)
        .map_err(|e| SyncError::Other(e.to_string()))?;

    let mut tree_entries = Vec::new();
    let total = files.len();
    let pb = crate::progress::file_bar(total as u64, "Uploading");

    for (path, blob_hash) in &files {
        pb.inc(1);

        let (_, content) = store
            .load_blob(*blob_hash)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        let sha = client
            .create_blob(owner, repo_name, &content)
            .map_err(SyncError::GitHub)?;

        tree_entries.push(TreeEntryCreate {
            path: path.clone(),
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha,
        });
    }
    pb.finish_with_message(format!("{} blobs uploaded", total));

    // Create tree
    let tree_sha = client
        .create_tree(owner, repo_name, tree_entries, None)
        .map_err(SyncError::GitHub)?;

    // Get parent commit SHA if branch exists
    let mut parents = Vec::new();
    if let Ok(branches) = client.list_branches(owner, repo_name) {
        if let Some(existing) = branches.iter().find(|b| b.name == branch_name) {
            parents.push(existing.commit.sha.clone());
        }
    }

    // Create commit
    let commit_sha = client
        .create_commit(owner, repo_name, &head_leaf.message, &tree_sha, &parents)
        .map_err(SyncError::GitHub)?;

    // Update or create branch ref
    if parents.is_empty() {
        // New branch
        client
            .create_ref(owner, repo_name, branch_name, &commit_sha)
            .map_err(SyncError::GitHub)?;
    } else {
        client
            .update_ref(owner, repo_name, branch_name, &commit_sha, force)
            .map_err(SyncError::GitHub)?;
    }

    eprintln!("Uploaded to {}/{} (branch: {})", owner, repo_name, branch_name);

    Ok(UploadResult {
        files_uploaded: total,
        commit_sha,
        branch: branch_name.to_string(),
    })
}

/// Scout — list remote branches without downloading.
pub fn scout(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
) -> Result<Vec<String>, SyncError> {
    let branches = client
        .list_branches(owner, repo_name)
        .map_err(SyncError::GitHub)?;

    Ok(branches.into_iter().map(|b| b.name).collect())
}

/// Harvest — download specific branches.
pub fn harvest(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline_names: &[String],
) -> Result<Vec<String>, SyncError> {
    let branches = client
        .list_branches(owner, repo_name)
        .map_err(SyncError::GitHub)?;

    let mut harvested = Vec::new();

    for target_name in timeline_names {
        let branch = branches
            .iter()
            .find(|b| b.name == *target_name)
            .ok_or_else(|| {
                SyncError::Other(format!("remote timeline '{}' not found", target_name))
            })?;

        eprintln!("Harvesting timeline '{}'...", target_name);

        // Get the tree
        let tree = client
            .get_tree(owner, repo_name, &branch.commit.sha)
            .map_err(SyncError::GitHub)?;

        // Download blob files into CAS
        let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
            .map_err(|e| SyncError::Other(e.to_string()))?;
        let store = FsStore::new(&cas);
        let mut file_map = BTreeMap::new();

        for entry in &tree.tree {
            if entry.entry_type != "blob" {
                continue;
            }
            match client.download_file(owner, repo_name, &entry.path, &branch.commit.sha) {
                Ok(content) => {
                    let (_hash, _) = store
                        .put_blob(&content)
                        .map_err(|e| SyncError::Other(e.to_string()))?;
                    file_map.insert(entry.path.clone(), content);
                }
                Err(e) => {
                    eprintln!("Warning: skipping {}: {}", entry.path, e);
                }
            }
        }

        // Build tree and create timeline
        let _tree_hash = store
            .build_tree_from_map(
                &file_map
                    .into_iter()
                    .map(|(k, v)| (k, v))
                    .collect(),
            )
            .map_err(|e| SyncError::Other(e.to_string()))?;

        // Create timeline if it doesn't exist
        if repo.get_timeline_head(target_name).map_err(|e| SyncError::Other(e.to_string()))?.is_none() {
            repo.create_timeline(target_name, None)
                .map_err(|e| SyncError::Other(e.to_string()))?;
        }

        harvested.push(target_name.clone());
    }

    Ok(harvested)
}

// Helper to collect files from tree
fn collect_tree_files(
    store: &FsStore<'_>,
    tree_hash: B3Hash,
    prefix: &str,
    files: &mut BTreeMap<String, B3Hash>,
) -> Result<(), crate::fsmerkle::FsMerkleError> {
    let tree = store.load_tree(tree_hash)?;
    for entry in &tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            crate::fsmerkle::NodeKind::Blob => {
                files.insert(path, entry.hash);
            }
            crate::fsmerkle::NodeKind::Tree => {
                collect_tree_files(store, entry.hash, &path, files)?;
            }
        }
    }
    Ok(())
}

/// Result of a sync (delta update) operation.
#[derive(Debug)]
pub struct SyncResult {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub no_changes: bool,
}

/// Sync — incremental delta update of a local timeline from remote.
/// Downloads only changed files compared to local state.
pub fn sync_timeline(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
) -> Result<SyncResult, SyncError> {
    // Get remote tree
    let branches = client.list_branches(owner, repo_name).map_err(SyncError::GitHub)?;
    let branch = branches.iter().find(|b| b.name == timeline)
        .ok_or_else(|| SyncError::Other(format!("remote branch '{}' not found", timeline)))?;

    let remote_tree = client.get_tree(owner, repo_name, &branch.commit.sha)
        .map_err(SyncError::GitHub)?;

    // Build remote file set: path → sha1
    let remote_files: BTreeMap<String, String> = remote_tree.tree.iter()
        .filter(|e| e.entry_type == "blob")
        .map(|e| (e.path.clone(), e.sha.clone()))
        .collect();

    // Build local file set from last commit tree
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let store = FsStore::new(&cas);

    let mut local_files: BTreeMap<String, B3Hash> = BTreeMap::new();
    if let Ok(Some(head_idx)) = repo.get_timeline_head(timeline) {
        if let Ok(Some(leaf)) = repo.get_leaf(head_idx) {
            let _ = collect_tree_files(&store, leaf.tree_root, "", &mut local_files);
        }
    }

    // Compute delta
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    // Check for existing local files by building a hash mapping
    let hash_mapping = crate::remote::HashMapping::new(&repo.ivaldi_dir);

    // Files in remote but not local, or changed
    for (path, remote_sha) in &remote_files {
        let is_new = !local_files.contains_key(path);
        let is_modified = if !is_new {
            // Check if the remote SHA maps to a different BLAKE3 than what we have locally
            match hash_mapping.get_blake3(remote_sha) {
                Some(b3) => local_files.get(path) != Some(&b3),
                None => true, // Unknown mapping = assume changed
            }
        } else {
            false
        };

        if is_new || is_modified {
            if is_new { added.push(path.clone()); } else { modified.push(path.clone()); }
        }
    }

    // Files in local but not remote
    for path in local_files.keys() {
        if !remote_files.contains_key(path) {
            deleted.push(path.clone());
        }
    }

    if added.is_empty() && modified.is_empty() && deleted.is_empty() {
        return Ok(SyncResult { added, modified, deleted, no_changes: true });
    }

    // Download added/modified files
    let download_paths: Vec<String> = added.iter().chain(modified.iter()).cloned().collect();
    let pb = crate::progress::file_bar(download_paths.len() as u64, "Syncing");

    let mut hash_mapping = crate::remote::HashMapping::new(&repo.ivaldi_dir);

    for path in &download_paths {
        pb.inc(1);
        match client.download_file(owner, repo_name, path, &branch.commit.sha) {
            Ok(content) => {
                let (blob_hash, _) = store.put_blob(&content)
                    .map_err(|e| SyncError::Other(e.to_string()))?;

                if let Some(remote_sha) = remote_files.get(path) {
                    hash_mapping.insert(remote_sha, blob_hash);
                }

                // Write to workspace
                let file_path = repo.work_dir.join(path);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&file_path, &content).ok();
            }
            Err(e) => {
                crate::logging::warn(&format!("failed to sync {}: {}", path, e));
            }
        }
    }
    pb.finish_with_message("Sync complete");

    // Delete removed files from workspace
    for path in &deleted {
        let file_path = repo.work_dir.join(path);
        let _ = std::fs::remove_file(&file_path);
    }

    hash_mapping.save().map_err(|e| SyncError::Other(e.to_string()))?;

    // Create commit for the synced state
    let ignore_cache = crate::ignore::load_pattern_cache(&repo.work_dir);
    let mut ws = crate::workspace::Workspace::new(&cas, &repo.work_dir, &repo.ivaldi_dir);
    ws.gather_all(&ignore_cache).map_err(|e| SyncError::Other(e.to_string()))?;

    if !ws.staging.is_empty() {
        let tree_hash = ws.build_staged_tree().map_err(|e| SyncError::Other(e.to_string()))?;
        let msg = format!("Synced from {}/{} (branch: {})", owner, repo_name, timeline);
        repo.commit(tree_hash, "ivaldi-sync", &msg).map_err(|e| SyncError::Other(e.to_string()))?;
        ws.staging.clear();
        ws.save().map_err(|e| SyncError::Other(e.to_string()))?;
    }

    added.sort();
    modified.sort();
    deleted.sort();

    Ok(SyncResult { added, modified, deleted, no_changes: false })
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("GitHub error: {0}")]
    GitHub(#[from] GitHubError),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_result_structure() {
        let r = DownloadResult {
            files_downloaded: 10,
            commits_imported: 1,
            timelines_created: vec!["main".into()],
        };
        assert_eq!(r.files_downloaded, 10);
    }

    #[test]
    fn upload_result_structure() {
        let r = UploadResult {
            files_uploaded: 5,
            commit_sha: "abc123".into(),
            branch: "main".into(),
        };
        assert_eq!(r.branch, "main");
    }
}
