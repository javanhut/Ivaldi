//! Sync operations for Ivaldi VCS — download, upload, scout, harvest.
//!
//! Bridges Ivaldi's internal BLAKE3-based storage with GitHub's SHA1-based
//! Git objects. SHA1 is used ONLY for API communication — never internally.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::git_remote::{self, SmartHttpClient};
use crate::github::{GitHubClient, GitHubError, TreeEntryCreate, TreeResponse};
use crate::hash::B3Hash;
use crate::ignore;
use crate::leaf::Leaf;
use crate::remote::{HashMapping, RemoteBranch};
use crate::repo::Repo;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTimelineState {
    NotDownloaded,
    UpToDate,
    OutOfSync,
    LocalOnly,
}

#[derive(Debug, Clone)]
pub struct RemoteTimelineInfo {
    pub name: String,
    pub remote_sha: String,
    pub state: RemoteTimelineState,
}

/// Download a repository from GitHub into a local Ivaldi repo.
pub fn download(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    target_dir: &Path,
    branch: Option<&str>,
) -> Result<DownloadResult, SyncError> {
    if target_dir.exists()
        && target_dir
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        return Err(SyncError::Other(format!(
            "directory '{}' already exists and is not empty",
            target_dir.display()
        )));
    }
    let created_target = ensure_download_target(target_dir)?;

    eprintln!("Downloading {}/{}...", owner, repo_name);
    let result = (|| -> Result<DownloadResult, SyncError> {
        let remote = SmartHttpClient::new(client.token())
            .fetch_repo(owner, repo_name, branch)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        crate::forge::forge(target_dir).map_err(|e| SyncError::Other(e.to_string()))?;
        let ivaldi_dir = target_dir.join(".ivaldi");

        let portal_mgr = crate::portal::PortalManager::new(&ivaldi_dir);
        let portal = crate::portal::Portal::parse(&format!("{}/{}", owner, repo_name)).unwrap();
        let _ = portal_mgr.add(&portal);

        let mut cfg = crate::config::Config::new();
        cfg.set("portal.default", &format!("{}/{}", owner, repo_name));
        cfg.save(&ivaldi_dir.join("config")).ok();

        let mut repo = Repo::open(target_dir).map_err(|e| SyncError::Other(e.to_string()))?;
        let import = git_remote::import_fetch_result(&mut repo, &remote)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        // forge() initialised HEAD to a hardcoded "main"; point it at the
        // branch we actually fetched so `whereami` and `timeline list` agree
        // with the working tree. Also materialise the on-disk ref file so the
        // timeline shows up in tools that scan refs/heads.
        let ref_path = ivaldi_dir.join("refs/heads").join(&remote.branch);
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).map_err(|e| SyncError::Other(e.to_string()))?;
        }
        if !ref_path.exists() {
            fs::write(&ref_path, "").map_err(|e| SyncError::Other(e.to_string()))?;
        }
        crate::forge::write_head(
            &ivaldi_dir,
            &crate::forge::HeadRef::Timeline(remote.branch.clone()),
        )
        .map_err(|e| SyncError::Other(e.to_string()))?;

        let cas = FileCas::new(ivaldi_dir.join("objects"))
            .map_err(|e| SyncError::Other(e.to_string()))?;
        let store = FsStore::new(&cas);
        let file_count = if repo
            .get_timeline_head(&remote.branch)
            .map_err(|e| SyncError::Other(e.to_string()))?
            .is_some()
        {
            checkout_tree_to_workspace(&repo, &store, &remote.branch)?
        } else {
            0
        };

        eprintln!(
            "Downloaded {} files, imported {} commits from {}/{}",
            file_count, import.commits_imported, owner, repo_name
        );

        Ok(DownloadResult {
            files_downloaded: file_count,
            commits_imported: import.commits_imported,
            timelines_created: vec![remote.branch],
        })
    })();

    if result.is_err() && created_target {
        cleanup_failed_download_target(target_dir);
    }
    result
}

fn ensure_download_target(target_dir: &Path) -> Result<bool, SyncError> {
    if target_dir.exists() {
        return Ok(false);
    }
    fs::create_dir_all(target_dir).map_err(|e| SyncError::Other(e.to_string()))?;
    Ok(true)
}

fn cleanup_failed_download_target(target_dir: &Path) {
    let _ = fs::remove_dir_all(target_dir);
}

/// Upload blobs in parallel, skipping those already mapped.
///
/// Returns `TreeEntryCreate` entries for the GitHub tree API.
fn upload_blobs_parallel(
    client: &GitHubClient,
    store: &FsStore<'_>,
    files: &BTreeMap<String, B3Hash>,
    hash_mapping: &mut HashMapping,
    owner: &str,
    repo_name: &str,
) -> Result<Vec<TreeEntryCreate>, SyncError> {
    // Defense-in-depth: reject security-blocked files before upload
    for (path, _) in files {
        if crate::ignore::is_security_blocked(path) {
            return Err(SyncError::Other(format!(
                "refusing to upload security-blocked file: {}",
                path
            )));
        }
    }

    // Partition files into already-mapped (skip) and need-upload
    let mut tree_entries = Vec::new();
    let mut to_upload: Vec<(String, B3Hash)> = Vec::new();

    for (path, blob_hash) in files {
        if let Some(sha1) = hash_mapping.get_sha1(*blob_hash) {
            // Already uploaded — reuse SHA1
            tree_entries.push(TreeEntryCreate {
                path: path.clone(),
                mode: "100644".into(),
                entry_type: "blob".into(),
                sha: sha1.to_string(),
            });
        } else {
            to_upload.push((path.clone(), *blob_hash));
        }
    }

    let skipped = files.len() - to_upload.len();
    if skipped > 0 {
        eprintln!("Skipped {} already-uploaded blobs", skipped);
    }

    if to_upload.is_empty() {
        return Ok(tree_entries);
    }

    // Pre-load all blob content (CAS is not Sync, so load before spawning threads)
    let mut upload_items: Vec<(String, B3Hash, Vec<u8>)> = Vec::new();
    for (path, blob_hash) in &to_upload {
        let (_, content) = store
            .load_blob(*blob_hash)
            .map_err(|e| SyncError::Other(e.to_string()))?;
        upload_items.push((path.clone(), *blob_hash, content));
    }

    // Upload in parallel using std::thread::scope (ureq is sync)
    let pb = crate::progress::file_bar(upload_items.len() as u64, "Uploading");
    let results: Vec<Result<(String, String, B3Hash), SyncError>> = std::thread::scope(|s| {
        let chunk_size = (upload_items.len() / 4).max(1);
        let mut handles = Vec::new();

        for chunk in upload_items.chunks(chunk_size) {
            let pb = &pb;
            let handle = s.spawn(move || {
                let mut results = Vec::new();
                for (path, blob_hash, content) in chunk {
                    match client.create_blob(owner, repo_name, content) {
                        Ok(sha) => {
                            pb.inc(1);
                            results.push(Ok((path.clone(), sha, *blob_hash)));
                        }
                        Err(e) => results.push(Err(SyncError::GitHub(e))),
                    }
                }
                results
            });
            handles.push(handle);
        }

        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    pb.finish_with_message(format!("{} blobs uploaded", results.len()));

    for result in results {
        let (path, sha, blob_hash) = result?;
        hash_mapping.insert(&sha, blob_hash);
        tree_entries.push(TreeEntryCreate {
            path,
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha,
        });
    }

    Ok(tree_entries)
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

    let timeline = repo
        .current_timeline()
        .map_err(|e| SyncError::Other(e.to_string()))?;
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

    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);
    let total = files.len();

    // GitHub's Git Data API returns 409 on every endpoint (blobs included) when
    // the repo has no initial commit. Detect that up front and seed the repo
    // via the Contents API so the rest of the upload can proceed.
    let existing_branches = client
        .list_branches(owner, repo_name)
        .map_err(SyncError::GitHub)?;
    let mut bootstrapped = false;
    if existing_branches.is_empty() {
        let default_branch = client
            .get_repo(owner, repo_name)
            .map(|info| info.default_branch)
            .unwrap_or_default();
        let seed_branch = if default_branch.is_empty() {
            branch_name
        } else {
            default_branch.as_str()
        };
        client
            .create_file_contents(
                owner,
                repo_name,
                ".ivaldi-bootstrap",
                seed_branch,
                b"Ivaldi bootstrap placeholder. Safe to remove after first upload.\n",
                "chore: initialize repository for Ivaldi",
            )
            .map_err(SyncError::GitHub)?;
        bootstrapped = true;
    }

    let tree_entries =
        upload_blobs_parallel(client, &store, &files, &mut hash_mapping, owner, repo_name)?;

    // Create tree
    let tree_sha = client
        .create_tree(owner, repo_name, tree_entries, None)
        .map_err(SyncError::GitHub)?;

    // Resolve parent commit SHA for the GitHub commit
    let existing_branch_sha = client
        .list_branches(owner, repo_name)
        .ok()
        .and_then(|branches| {
            branches
                .iter()
                .find(|b| b.name == branch_name)
                .map(|b| b.commit.sha.clone())
        });

    // After a bootstrap, the seed commit on the branch is not a real ancestor
    // of our local history, so treat this like a force-push for parent
    // resolution and ref update to replace the placeholder commit.
    let effective_force = force || bootstrapped;

    let parents = resolve_github_parent(
        repo,
        &head_leaf,
        &hash_mapping,
        existing_branch_sha.as_deref(),
        effective_force,
    );
    let is_new_branch = existing_branch_sha.is_none();

    // Create commit
    let commit_sha = client
        .create_commit(owner, repo_name, &head_leaf.message, &tree_sha, &parents)
        .map_err(SyncError::GitHub)?;

    // Store mapping: GitHub SHA1 → Ivaldi leaf BLAKE3 hash
    hash_mapping.insert(&commit_sha, head_leaf.hash());
    hash_mapping
        .save()
        .map_err(|e| SyncError::Other(e.to_string()))?;

    // Update or create branch ref
    if is_new_branch {
        client
            .create_ref(owner, repo_name, branch_name, &commit_sha)
            .map_err(SyncError::GitHub)?;
    } else {
        client
            .update_ref(owner, repo_name, branch_name, &commit_sha, effective_force)
            .map_err(SyncError::GitHub)?;
    }

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
    SmartHttpClient::new(client.token())
        .list_branches(owner, repo_name)
        .map_err(|e| SyncError::Other(e.to_string()))
}

pub fn scout_with_status(
    client: &GitHubClient,
    repo: &Repo,
    owner: &str,
    repo_name: &str,
) -> Result<Vec<RemoteTimelineInfo>, SyncError> {
    let branches = SmartHttpClient::new(client.token())
        .list_branch_refs(owner, repo_name)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let mapping = HashMapping::new(&repo.ivaldi_dir);

    Ok(branches
        .into_iter()
        .map(|branch| RemoteTimelineInfo {
            name: branch.name.clone(),
            remote_sha: branch.sha1.clone(),
            state: timeline_sync_state(repo, &mapping, &branch),
        })
        .collect())
}

/// Harvest — download specific branches with full history.
pub fn harvest(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline_names: &[String],
) -> Result<Vec<String>, SyncError> {
    let branches = SmartHttpClient::new(client.token())
        .list_branch_refs(owner, repo_name)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let mapping = HashMapping::new(&repo.ivaldi_dir);

    let mut harvested = Vec::new();

    for target_name in timeline_names {
        let branch = branches
            .iter()
            .find(|b| &b.name == target_name)
            .ok_or_else(|| {
                SyncError::Other(format!("remote timeline '{}' not found", target_name))
            })?;

        eprintln!("Harvesting timeline '{}'...", target_name);
        match timeline_sync_state(repo, &mapping, branch) {
            RemoteTimelineState::NotDownloaded => eprintln!("  Local state: not downloaded"),
            RemoteTimelineState::UpToDate => eprintln!("  Local state: up to date"),
            RemoteTimelineState::OutOfSync => eprintln!("  Local state: out of sync"),
            RemoteTimelineState::LocalOnly => eprintln!("  Local state: local only"),
        }

        let fetch = SmartHttpClient::new(client.token())
            .fetch_repo(owner, repo_name, Some(target_name))
            .map_err(|e| SyncError::Other(e.to_string()))?;
        let import = git_remote::import_fetch_result(repo, &fetch)
            .map_err(|e| SyncError::Other(e.to_string()))?;
        if import.commits_skipped > 0 {
            eprintln!(
                "  {} new commits imported ({} already present)",
                import.commits_imported, import.commits_skipped
            );
        } else {
            eprintln!("  {} commits imported", import.commits_imported);
        }

        harvested.push(target_name.clone());
    }

    Ok(harvested)
}

fn timeline_sync_state(
    repo: &Repo,
    mapping: &HashMapping,
    branch: &RemoteBranch,
) -> RemoteTimelineState {
    let Ok(Some(head_idx)) = repo.get_timeline_head(&branch.name) else {
        return RemoteTimelineState::NotDownloaded;
    };
    let Ok(Some(head_leaf)) = repo.get_leaf(head_idx) else {
        return RemoteTimelineState::LocalOnly;
    };

    match mapping.get_sha1(head_leaf.hash()) {
        Some(sha) if sha == branch.sha1 => RemoteTimelineState::UpToDate,
        Some(_) => RemoteTimelineState::OutOfSync,
        None => RemoteTimelineState::LocalOnly,
    }
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

/// Result of a full history import.
#[derive(Debug)]
pub struct ImportResult {
    pub commits_imported: usize,
    pub commits_skipped: usize,
    pub blobs_downloaded: usize,
    pub timeline: String,
}

/// Parse ISO 8601 date string to unix timestamp (no chrono dependency).
///
/// Supports formats: `2024-01-15T10:30:00Z`, `2024-01-15T10:30:00+00:00`,
/// `2024-01-15T10:30:00-05:00`.
pub fn parse_iso8601_to_unix(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }

    // Split date and time at 'T'
    let (date_part, rest) = s.split_once('T')?;
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year: i64 = date_parts[0].parse().ok()?;
    let month: i64 = date_parts[1].parse().ok()?;
    let day: i64 = date_parts[2].parse().ok()?;

    // Parse time, stripping timezone
    let (time_str, tz_offset_secs) = if rest.ends_with('Z') {
        (&rest[..rest.len() - 1], 0i64)
    } else if let Some(plus_pos) = rest[8..].find('+') {
        let idx = 8 + plus_pos;
        let tz = &rest[idx + 1..];
        let tz_parts: Vec<&str> = tz.split(':').collect();
        let hours: i64 = tz_parts.first()?.parse().ok()?;
        let mins: i64 = tz_parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
        (&rest[..idx], hours * 3600 + mins * 60)
    } else if let Some(minus_pos) = rest[8..].find('-') {
        let idx = 8 + minus_pos;
        let tz = &rest[idx + 1..];
        let tz_parts: Vec<&str> = tz.split(':').collect();
        let hours: i64 = tz_parts.first()?.parse().ok()?;
        let mins: i64 = tz_parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
        (&rest[..idx], -(hours * 3600 + mins * 60))
    } else {
        (rest, 0i64)
    };

    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 2 {
        return None;
    }
    let hour: i64 = time_parts[0].parse().ok()?;
    let min: i64 = time_parts[1].parse().ok()?;
    let sec: i64 = time_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // Convert to unix timestamp (days since epoch)
    // Simplified algorithm for dates after 1970
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    let month_days = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for m in 0..(month - 1) as usize {
        if m < 12 {
            days += month_days[m] as i64;
        }
    }
    days += day - 1;

    Some(days * 86400 + hour * 3600 + min * 60 + sec - tz_offset_secs)
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Import full commit history from GitHub into an Ivaldi repo.
///
/// Walks commits oldest-first, creates Ivaldi leaves preserving parent chains,
/// author info, timestamps, and tree content.
///
/// `branch` is the remote GitHub branch name used for API calls.
/// `local_timeline` optionally overrides the local timeline name to store commits under
/// (defaults to `branch` if `None`). This is used when importing into a temp timeline
/// during diverged sync.
pub fn import_full_history(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    branch: &str,
    depth: usize,
) -> Result<ImportResult, SyncError> {
    import_full_history_into(client, repo, owner, repo_name, branch, branch, depth)
}

/// Like `import_full_history` but stores commits under `local_timeline` instead of `branch`.
fn import_full_history_into(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    remote_branch: &str,
    local_timeline: &str,
    depth: usize,
) -> Result<ImportResult, SyncError> {
    // Fetch commits (newest-first from GitHub)
    let mut commits = client
        .list_commits(owner, repo_name, remote_branch, depth)
        .map_err(SyncError::GitHub)?;

    // Reverse to oldest-first for correct parent ordering
    commits.reverse();

    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let store = FsStore::new(&cas);
    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);

    // Track SHA1 → leaf index for parent resolution
    let mut sha_to_idx: HashMap<String, u64> = HashMap::new();
    // Cache tree SHA → Ivaldi tree hash to avoid re-downloading identical trees
    let mut tree_cache: HashMap<String, B3Hash> = HashMap::new();

    let mut commits_imported = 0usize;
    let mut commits_skipped = 0usize;
    let mut blobs_downloaded = 0usize;

    // Ensure timeline exists
    if repo
        .get_timeline_head(local_timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .is_none()
    {
        // Create timeline ref directory/file but no head yet
        let ref_path = repo.ivaldi_dir.join("refs/heads").join(local_timeline);
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&ref_path, "").ok();
    }

    // Identify unskipped commits (those not already mapped)
    let unskipped: Vec<usize> = commits
        .iter()
        .enumerate()
        .filter(|(_, c)| hash_mapping.get_blake3(&c.sha).is_none())
        .map(|(i, _)| i)
        .collect();

    // Collect unique tree SHAs from unskipped commits
    let unique_tree_shas: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        unskipped
            .iter()
            .map(|&i| commits[i].commit.tree.sha.clone())
            .filter(|sha| seen.insert(sha.clone()))
            .collect()
    };

    // --- Phase 2: Parallel tree pre-fetch ---
    let prefetched_trees: HashMap<String, TreeResponse> = if !unique_tree_shas.is_empty() {
        let pb_trees = crate::progress::file_bar(unique_tree_shas.len() as u64, "Fetching trees");
        let results: Vec<(String, Result<TreeResponse, GitHubError>)> = std::thread::scope(|s| {
            let chunk_size = (unique_tree_shas.len() / 8).max(1);
            let mut handles = Vec::new();
            for chunk in unique_tree_shas.chunks(chunk_size) {
                let pb_trees = &pb_trees;
                let handle = s.spawn(move || {
                    let mut results = Vec::new();
                    for sha in chunk {
                        let r = client.get_tree(owner, repo_name, sha);
                        pb_trees.inc(1);
                        results.push((sha.clone(), r));
                    }
                    results
                });
                handles.push(handle);
            }
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        pb_trees.finish_with_message(format!("{} trees fetched", unique_tree_shas.len()));

        let mut map = HashMap::new();
        for (sha, result) in results {
            match result {
                Ok(tree) => {
                    map.insert(sha, tree);
                }
                Err(e) => {
                    crate::logging::warn(&format!("failed to pre-fetch tree {}: {}", sha, e));
                }
            }
        }
        map
    } else {
        HashMap::new()
    };

    // --- Phase 3: Global blob batch download ---
    // Collect all unique blobs from pre-fetched trees that we don't already have
    let mut blobs_to_download: Vec<(String, String, String)> = Vec::new(); // (path, sha1, commit_sha)
    let mut seen_blob_shas: std::collections::HashSet<String> = std::collections::HashSet::new();

    for &idx in &unskipped {
        let commit = &commits[idx];
        let tree_sha = &commit.commit.tree.sha;
        if let Some(tree) = prefetched_trees.get(tree_sha) {
            for entry in &tree.tree {
                if entry.entry_type == "blob"
                    && hash_mapping.get_blake3(&entry.sha).is_none()
                    && seen_blob_shas.insert(entry.sha.clone())
                {
                    blobs_to_download.push((
                        entry.path.clone(),
                        entry.sha.clone(),
                        commit.sha.clone(),
                    ));
                }
            }
        }
    }

    if !blobs_to_download.is_empty() {
        let pb_blobs =
            crate::progress::file_bar(blobs_to_download.len() as u64, "Downloading blobs");
        let blob_results: Vec<Result<(String, String, Vec<u8>), String>> =
            std::thread::scope(|s| {
                let chunk_size = (blobs_to_download.len() / 8).max(1);
                let mut handles = Vec::new();
                for chunk in blobs_to_download.chunks(chunk_size) {
                    let pb_blobs = &pb_blobs;
                    let handle = s.spawn(move || {
                        let mut results = Vec::new();
                        for (path, sha1, commit_sha) in chunk {
                            match client.download_file(owner, repo_name, path, commit_sha) {
                                Ok(content) => {
                                    pb_blobs.inc(1);
                                    results.push(Ok((path.clone(), sha1.clone(), content)));
                                }
                                Err(e) => {
                                    pb_blobs.inc(1);
                                    results
                                        .push(Err(format!("failed to download {}: {}", path, e)));
                                }
                            }
                        }
                        results
                    });
                    handles.push(handle);
                }
                handles
                    .into_iter()
                    .flat_map(|h| h.join().unwrap_or_default())
                    .collect()
            });
        pb_blobs.finish_with_message(format!("{} blobs downloaded", blobs_to_download.len()));

        // Store in CAS and update hash_mapping (serial — CAS is not Sync)
        for result in blob_results {
            match result {
                Ok((_, sha1, content)) => {
                    let (b3_hash, _) = store
                        .put_blob(&content)
                        .map_err(|e| SyncError::Other(e.to_string()))?;
                    hash_mapping.insert(&sha1, b3_hash);
                    blobs_downloaded += 1;
                }
                Err(msg) => {
                    crate::logging::warn(&msg);
                }
            }
        }
    }

    // --- Phase 4: Commit loop using build_tree_from_hash_map ---
    let total = commits.len();
    let pb = crate::progress::file_bar(total as u64, "Importing commits");

    for commit in &commits {
        pb.inc(1);

        // Skip if already mapped
        if hash_mapping.get_blake3(&commit.sha).is_some() {
            // Still populate sha_to_idx from existing data for parent resolution
            if let Some(b3) = hash_mapping.get_blake3(&commit.sha) {
                // Search for leaf with this hash
                for idx in 0..repo.commit_count() {
                    if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                        if leaf.hash() == b3 {
                            sha_to_idx.insert(commit.sha.clone(), idx);
                            break;
                        }
                    }
                }
            }
            commits_skipped += 1;
            continue;
        }

        // Build tree using hash-based approach (Phase 4 optimization)
        let tree_sha = &commit.commit.tree.sha;
        let ivaldi_tree_hash = if let Some(&cached) = tree_cache.get(tree_sha) {
            cached
        } else {
            // Look up pre-fetched tree, fall back to live fetch
            let tree = match prefetched_trees.get(tree_sha) {
                Some(t) => t.clone(),
                None => client
                    .get_tree(owner, repo_name, tree_sha)
                    .map_err(SyncError::GitHub)?,
            };

            // Build hash map from tree entries — pure HashMap lookups, zero disk I/O
            let mut hash_file_map: BTreeMap<String, B3Hash> = BTreeMap::new();
            for entry in &tree.tree {
                if entry.entry_type == "blob" {
                    if let Some(b3) = hash_mapping.get_blake3(&entry.sha) {
                        hash_file_map.insert(entry.path.clone(), b3);
                    }
                    // else: blob wasn't downloaded (error during batch) — skip
                }
            }

            // Build Merkle tree from hashes only — NO blob content reads
            let tree_hash = store
                .build_tree_from_hash_map(&hash_file_map)
                .map_err(|e| SyncError::Other(e.to_string()))?;
            tree_cache.insert(tree_sha.clone(), tree_hash);
            tree_hash
        };

        // Parse author and timestamp
        let author = format!(
            "{} <{}>",
            commit.commit.author.name, commit.commit.author.email
        );
        let time_unix = commit
            .commit
            .author
            .date
            .as_deref()
            .and_then(parse_iso8601_to_unix)
            .unwrap_or(0);

        // Resolve parent indices
        let prev_idx = if !commit.parents.is_empty() {
            sha_to_idx
                .get(&commit.parents[0].sha)
                .copied()
                .unwrap_or(crate::leaf::NO_PARENT)
        } else {
            crate::leaf::NO_PARENT
        };

        let merge_idxs: Vec<u64> = commit
            .parents
            .iter()
            .skip(1)
            .filter_map(|p| sha_to_idx.get(&p.sha).copied())
            .collect();

        // Build leaf
        let mut leaf = Leaf::new(
            ivaldi_tree_hash,
            local_timeline,
            &author,
            time_unix,
            &commit.commit.message,
        );
        leaf.prev_idx = prev_idx;
        leaf.merge_idxs = merge_idxs;

        // Commit raw
        let result = repo
            .commit_raw(leaf, local_timeline)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        // Record mappings
        hash_mapping.insert(&commit.sha, result.hash);
        sha_to_idx.insert(commit.sha.clone(), result.index);
        commits_imported += 1;
    }
    pb.finish_with_message(format!(
        "{} commits imported, {} skipped",
        commits_imported, commits_skipped
    ));

    // Save hash mapping
    hash_mapping
        .save()
        .map_err(|e| SyncError::Other(e.to_string()))?;

    Ok(ImportResult {
        commits_imported,
        commits_skipped,
        blobs_downloaded,
        timeline: local_timeline.to_string(),
    })
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
/// - **Diverged:** both have new commits → auto-fuse (Ivaldi's auto-merge philosophy)
pub fn sync_timeline(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    timeline: &str,
) -> Result<SyncResult, SyncError> {
    let branches = client
        .list_branches(owner, repo_name)
        .map_err(SyncError::GitHub)?;
    let branch = branches
        .iter()
        .find(|b| b.name == timeline)
        .ok_or_else(|| SyncError::Other(format!("remote branch '{}' not found", timeline)))?;

    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);

    // Fetch remote commits
    let remote_commits = client
        .list_commits(owner, repo_name, timeline, 0)
        .map_err(SyncError::GitHub)?;

    if remote_commits.is_empty() {
        return Ok(SyncResult {
            added: vec![],
            modified: vec![],
            deleted: vec![],
            no_changes: true,
            was_fast_forward: false,
            was_fused: false,
            conflicts: vec![],
        });
    }

    // Get local head BEFORE import so we can constrain ancestor search
    let local_head_idx = repo
        .get_timeline_head(timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?;

    // Build set of leaf indices reachable from local timeline head.
    // This prevents matching commits from OTHER timelines that happen to
    // be in the hash_mapping (e.g. after uploading a feature branch whose
    // commits later appear on main via a merge).
    let local_reachable: BTreeSet<u64> = {
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
    };

    // Find common ancestor: walk remote commits newest→oldest, check hash mapping.
    // Only accept leaves that are reachable from the local timeline head.
    let mut common_ancestor_sha: Option<String> = None;
    let mut common_ancestor_idx: Option<u64> = None;
    // Track commits with stale mappings so we skip them on re-search
    let mut stale_shas: BTreeSet<String> = BTreeSet::new();

    for commit in &remote_commits {
        if stale_shas.contains(&commit.sha) {
            continue;
        }
        if let Some(b3) = hash_mapping.get_blake3(&commit.sha) {
            // Find leaf index with this hash
            for idx in 0..repo.commit_count() {
                if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                    if leaf.hash() == b3 && local_reachable.contains(&idx) {
                        common_ancestor_sha = Some(commit.sha.clone());
                        common_ancestor_idx = Some(idx);
                        break;
                    }
                }
            }
            if common_ancestor_idx.is_some() {
                break;
            }
        }
    }

    // Stale-mapping detection: when the common ancestor IS the remote tip
    // (i.e., sync would say "up to date"), verify that the local tree
    // actually matches the remote tree.  A mismatch means a previous buggy
    // sync created a wrong fuse commit and mapped the remote tip to it.
    if common_ancestor_sha.as_ref() == Some(&remote_commits[0].sha) {
        if let Some(ca_idx) = common_ancestor_idx {
            let remote_tip_tree_sha = &remote_commits[0].commit.tree.sha;
            let remote_tree = client
                .get_tree(owner, repo_name, remote_tip_tree_sha)
                .map_err(SyncError::GitHub)?;
            let remote_paths: BTreeSet<&str> = remote_tree
                .tree
                .iter()
                .filter(|e| e.entry_type == "blob")
                .map(|e| e.path.as_str())
                .collect();

            let verify_cas = FileCas::new(repo.ivaldi_dir.join("objects"))
                .map_err(|e| SyncError::Other(e.to_string()))?;
            let verify_store = FsStore::new(&verify_cas);
            let local_files = get_tree_files(repo, &verify_store, ca_idx)?;
            let local_paths: BTreeSet<&str> = local_files.keys().map(|s| s.as_str()).collect();

            if remote_paths != local_paths {
                // Stale mapping: remove it and re-search for the real ancestor
                let stale_sha = remote_commits[0].sha.clone();
                hash_mapping.remove_sha1(&stale_sha);
                hash_mapping
                    .save()
                    .map_err(|e| SyncError::Other(e.to_string()))?;
                stale_shas.insert(stale_sha);
                common_ancestor_sha = None;
                common_ancestor_idx = None;

                for commit in &remote_commits {
                    if stale_shas.contains(&commit.sha) {
                        continue;
                    }
                    if let Some(b3) = hash_mapping.get_blake3(&commit.sha) {
                        for idx in 0..repo.commit_count() {
                            if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                                if leaf.hash() == b3 && local_reachable.contains(&idx) {
                                    common_ancestor_sha = Some(commit.sha.clone());
                                    common_ancestor_idx = Some(idx);
                                    break;
                                }
                            }
                        }
                        if common_ancestor_idx.is_some() {
                            break;
                        }
                    }
                }
            }
        }
    }

    // Count new remote commits (those before the common ancestor in the list)
    let new_remote_count = if let Some(ref ca_sha) = common_ancestor_sha {
        remote_commits
            .iter()
            .take_while(|c| c.sha != *ca_sha)
            .count()
    } else {
        remote_commits.len()
    };
    let new_local_count = match (local_head_idx, common_ancestor_idx) {
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
            count
        }
        (Some(_), None) => {
            // No common ancestor: all local commits are "new"
            let history = repo
                .walk_history(timeline)
                .map_err(|e| SyncError::Other(e.to_string()))?;
            history.len() as u64
        }
        _ => 0,
    };

    if new_remote_count == 0 {
        return Ok(SyncResult {
            added: vec![],
            modified: vec![],
            deleted: vec![],
            no_changes: true,
            was_fast_forward: false,
            was_fused: false,
            conflicts: vec![],
        });
    }

    // Classify: fast-forward or diverged
    let is_fast_forward = new_local_count == 0;

    if is_fast_forward {
        // Fast-forward: import remote commits + update workspace
        let _import = import_full_history(client, repo, owner, repo_name, timeline, 0)?;

        // Compute file changes for the result
        let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
            .map_err(|e| SyncError::Other(e.to_string()))?;
        let store = FsStore::new(&cas);

        let (added, modified, deleted) =
            compute_workspace_delta(repo, &store, timeline, common_ancestor_idx)?;

        // Update workspace files
        checkout_tree_to_workspace(repo, &store, timeline)?;

        return Ok(SyncResult {
            added,
            modified,
            deleted,
            no_changes: false,
            was_fast_forward: true,
            was_fused: false,
            conflicts: vec![],
        });
    }

    // Diverged: import remote commits into temp timeline, then auto-fuse
    let temp_timeline = format!("__sync_{}", timeline);

    // Create temp timeline from common ancestor if known
    if let Some(ancestor_idx) = common_ancestor_idx {
        // Create temp timeline pointing at common ancestor
        let ref_path = repo.ivaldi_dir.join("refs/heads").join(&temp_timeline);
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&ref_path, "").ok();
        repo.store
            .set_timeline_head(&temp_timeline, ancestor_idx)
            .map_err(|e| SyncError::Other(format!("store: {}", e)))?;
    }

    // Import remote history into temp timeline (fetch from real remote branch)
    let _import =
        import_full_history_into(client, repo, owner, repo_name, timeline, &temp_timeline, 0)?;

    // Get file sets for three-way merge
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| SyncError::Other(e.to_string()))?;
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

    let their_head_idx = repo
        .get_timeline_head(&temp_timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let their_files = if let Some(idx) = their_head_idx {
        get_tree_files(repo, &store, idx)?
    } else {
        BTreeMap::new()
    };

    // Auto-fuse
    let fuse_result = crate::fuse::FuseEngine::fuse(
        &base_files,
        &our_files,
        &their_files,
        crate::fuse::Strategy::Auto,
    );

    if fuse_result.success {
        // Build merged tree
        let merged_tree = store
            .build_tree_from_hash_map(&fuse_result.merged_files)
            .map_err(|e| SyncError::Other(e.to_string()))?;

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
            &format!(
                "Fused sync from {}/{} (branch: {})",
                owner, repo_name, timeline
            ),
        );
        fuse_leaf.prev_idx = our_head;
        if their_head != crate::leaf::NO_PARENT {
            fuse_leaf.merge_idxs = vec![their_head];
        }

        repo.commit_raw(fuse_leaf, timeline)
            .map_err(|e| SyncError::Other(e.to_string()))?;

        // Update workspace
        checkout_tree_to_workspace(repo, &store, timeline)?;

        // Map remote tip SHA
        let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);
        hash_mapping.insert(
            &branch.commit.sha,
            repo.get_leaf(
                repo.get_timeline_head(timeline)
                    .map_err(|e| SyncError::Other(e.to_string()))?
                    .unwrap(),
            )
            .map_err(|e| SyncError::Other(e.to_string()))?
            .unwrap()
            .hash(),
        );
        hash_mapping
            .save()
            .map_err(|e| SyncError::Other(e.to_string()))?;

        // Clean up temp timeline
        let _ = repo.store.remove_timeline_head(&temp_timeline);
        let _ = fs::remove_file(repo.ivaldi_dir.join("refs/heads").join(&temp_timeline));

        let (added, modified, deleted) =
            compute_file_changes(&base_files, &fuse_result.merged_files);

        Ok(SyncResult {
            added,
            modified,
            deleted,
            no_changes: false,
            was_fast_forward: false,
            was_fused: true,
            conflicts: vec![],
        })
    } else {
        // Conflicts — save merge state, report
        let conflicts: Vec<String> = fuse_result
            .conflicts
            .iter()
            .map(|c| c.path.clone())
            .collect();

        let merge_state = crate::repo::MergeState {
            source_timeline: temp_timeline.clone(),
            target_timeline: timeline.to_string(),
            strategy: "auto".into(),
            conflicts: conflicts.clone(),
        };
        repo.save_merge_state(&merge_state)
            .map_err(|e| SyncError::Other(e.to_string()))?;

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
}

/// Get the file set (path → B3Hash) from a leaf's tree.
fn get_tree_files(
    repo: &Repo,
    store: &FsStore<'_>,
    leaf_idx: u64,
) -> Result<BTreeMap<String, B3Hash>, SyncError> {
    let leaf = repo
        .get_leaf(leaf_idx)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .ok_or_else(|| SyncError::Other(format!("leaf {} not found", leaf_idx)))?;

    let mut files = BTreeMap::new();
    collect_tree_files(store, leaf.tree_root, "", &mut files)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    Ok(files)
}

/// Compute workspace delta between current head and a prior ancestor.
fn compute_workspace_delta(
    repo: &Repo,
    store: &FsStore<'_>,
    timeline: &str,
    ancestor_idx: Option<u64>,
) -> Result<(Vec<String>, Vec<String>, Vec<String>), SyncError> {
    let old_files = if let Some(idx) = ancestor_idx {
        get_tree_files(repo, store, idx)?
    } else {
        BTreeMap::new()
    };

    let new_head = repo
        .get_timeline_head(timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?;
    let new_files = if let Some(idx) = new_head {
        get_tree_files(repo, store, idx)?
    } else {
        BTreeMap::new()
    };

    Ok(compute_file_changes(&old_files, &new_files))
}

/// Compute added/modified/deleted file lists between two file sets.
fn compute_file_changes(
    old: &BTreeMap<String, B3Hash>,
    new: &BTreeMap<String, B3Hash>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for (path, hash) in new {
        match old.get(path) {
            None => added.push(path.clone()),
            Some(old_hash) if old_hash != hash => modified.push(path.clone()),
            _ => {}
        }
    }
    for path in old.keys() {
        if !new.contains_key(path) {
            deleted.push(path.clone());
        }
    }

    added.sort();
    modified.sort();
    deleted.sort();
    (added, modified, deleted)
}

/// Checkout the tip tree of a timeline to the workspace directory.
///
/// Writes all files from the target tree, deletes workspace files that are
/// no longer in the tree (respecting `.ivaldiignore`), and cleans up empty
/// parent directories left behind.  Returns the number of files in the
/// target tree.
fn checkout_tree_to_workspace(
    repo: &Repo,
    store: &FsStore<'_>,
    timeline: &str,
) -> Result<usize, SyncError> {
    let head_idx = repo
        .get_timeline_head(timeline)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .ok_or_else(|| SyncError::Other("no head to checkout".into()))?;

    let head_leaf = repo
        .get_leaf(head_idx)
        .map_err(|e| SyncError::Other(e.to_string()))?
        .ok_or_else(|| SyncError::Other("corrupt head leaf".into()))?;

    let mut files = BTreeMap::new();
    collect_tree_files(store, head_leaf.tree_root, "", &mut files)
        .map_err(|e| SyncError::Other(e.to_string()))?;

    // Write / update files from the target tree
    for (path, blob_hash) in &files {
        let (_, content) = store
            .load_blob(*blob_hash)
            .map_err(|e| SyncError::Other(e.to_string()))?;
        let file_path = repo.work_dir.join(path);

        let should_write = if file_path.exists() {
            let existing = fs::read(&file_path).map_err(|e| SyncError::Other(e.to_string()))?;
            existing != content
        } else {
            true
        };

        if should_write {
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&file_path, &content).map_err(|e| SyncError::Other(e.to_string()))?;
        }
    }

    // Delete workspace files that are no longer in the target tree
    let ignore_cache = ignore::load_pattern_cache(&repo.work_dir);
    let current_files = scan_workspace_files(&repo.work_dir, "", &ignore_cache);
    let target_set: BTreeSet<&str> = files.keys().map(|s| s.as_str()).collect();

    for path in &current_files {
        if !target_set.contains(path.as_str()) {
            let full_path = repo.work_dir.join(path);
            let _ = fs::remove_file(&full_path);
            // Clean up empty parent directories
            let mut dir = full_path.parent();
            while let Some(d) = dir {
                if d == repo.work_dir {
                    break;
                }
                if fs::read_dir(d)
                    .map(|mut r| r.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = fs::remove_dir(d);
                    dir = d.parent();
                } else {
                    break;
                }
            }
        }
    }

    let count = files.len();
    Ok(count)
}

/// Recursively scan workspace files, respecting ignore patterns and
/// skipping the `.ivaldi/` directory.  Returns sorted relative paths.
fn scan_workspace_files(
    root: &Path,
    prefix: &str,
    ignore_cache: &ignore::PatternCache,
) -> Vec<String> {
    let mut out = Vec::new();
    scan_workspace_dir(root, root, prefix, ignore_cache, &mut out);
    out.sort();
    out
}

fn scan_workspace_dir(
    root: &Path,
    dir: &Path,
    prefix: &str,
    ignore_cache: &ignore::PatternCache,
    out: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };

        // Skip .ivaldi directory
        if rel == ".ivaldi" || rel.starts_with(".ivaldi/") {
            continue;
        }

        if ignore_cache.is_ignored(&rel) {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_dir() {
            scan_workspace_dir(root, &entry.path(), &rel, ignore_cache, out);
        } else if ft.is_file() {
            out.push(rel);
        }
    }
}

/// Resolve the GitHub parent SHA(s) for a new commit.
///
/// Priority:
/// 1. Branch already exists on GitHub AND not force-pushing → use its tip SHA
/// 2. Walk Ivaldi leaf chain via `prev_idx` backwards until a mapped commit is found,
///    AND collect mapped merge parents. Returns ALL resolved parents so GitHub
///    gets correct merge topology.
/// 3. Fallback → no parents mapped → root commit
fn resolve_github_parent(
    repo: &Repo,
    head_leaf: &crate::leaf::Leaf,
    hash_mapping: &HashMapping,
    existing_branch_sha: Option<&str>,
    force: bool,
) -> Vec<String> {
    // Priority 1: branch already exists on GitHub AND not force-pushing
    if !force {
        if let Some(sha) = existing_branch_sha {
            return vec![sha.to_string()];
        }
    }

    let mut parents = Vec::new();

    // Walk prev_idx chain backwards until we find a mapped ancestor
    if head_leaf.has_parent() {
        let mut current_idx = head_leaf.prev_idx;
        let mut depth = 0u32;
        const MAX_WALK_DEPTH: u32 = 1000;

        while depth < MAX_WALK_DEPTH {
            if let Ok(Some(ancestor)) = repo.get_leaf(current_idx) {
                let ancestor_blake3 = ancestor.hash();
                if let Some(sha1) = hash_mapping.get_sha1(ancestor_blake3) {
                    parents.push(sha1.to_string());
                    break;
                }
                // Keep walking if this ancestor also has a parent
                if ancestor.has_parent() {
                    current_idx = ancestor.prev_idx;
                    depth += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    // Also resolve merge parents (from fuse operations)
    for &merge_idx in &head_leaf.merge_idxs {
        // Walk each merge parent chain backwards too
        let mut current_idx = merge_idx;
        let mut depth = 0u32;
        const MAX_WALK_DEPTH: u32 = 1000;

        while depth < MAX_WALK_DEPTH {
            if let Ok(Some(ancestor)) = repo.get_leaf(current_idx) {
                let ancestor_blake3 = ancestor.hash();
                if let Some(sha1) = hash_mapping.get_sha1(ancestor_blake3) {
                    let sha1_str = sha1.to_string();
                    if !parents.contains(&sha1_str) {
                        parents.push(sha1_str);
                    }
                    break;
                }
                if ancestor.has_parent() {
                    current_idx = ancestor.prev_idx;
                    depth += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    if parents.is_empty() && head_leaf.has_parent() {
        eprintln!("Warning: no GitHub SHA1 mapping found in ancestor chain — creating root commit",);
    }

    parents
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
    use crate::cas::Cas;
    use crate::hash::B3Hash;
    use crate::leaf::Leaf;
    use std::fs;

    // -- ISO 8601 parsing tests --

    #[test]
    fn parse_iso8601_utc() {
        let ts = parse_iso8601_to_unix("2024-01-15T10:30:00Z").unwrap();
        assert_eq!(ts, 1705314600);
    }

    #[test]
    fn parse_iso8601_positive_offset() {
        let ts = parse_iso8601_to_unix("2024-01-15T10:30:00+05:30").unwrap();
        // 10:30 at +05:30 = 05:00 UTC → 1705314600 - 5*3600 - 30*60
        assert_eq!(ts, 1705294800);
    }

    #[test]
    fn ensure_download_target_creates_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("clone-target");
        assert!(!target.exists());

        let created = ensure_download_target(&target).unwrap();

        assert!(created);
        assert!(target.exists());
    }

    #[test]
    fn ensure_download_target_keeps_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("existing");
        fs::create_dir_all(&target).unwrap();

        let created = ensure_download_target(&target).unwrap();

        assert!(!created);
        assert!(target.exists());
    }

    #[test]
    fn cleanup_failed_download_target_removes_directory_tree() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("partial");
        fs::create_dir_all(target.join(".ivaldi")).unwrap();
        fs::write(target.join(".ivaldi").join("config"), "partial").unwrap();

        cleanup_failed_download_target(&target);

        assert!(!target.exists());
    }

    #[test]
    fn parse_iso8601_negative_offset() {
        let ts = parse_iso8601_to_unix("2024-01-15T10:30:00-05:00").unwrap();
        // 10:30 at -05:00 = 15:30 UTC → 1705314600 + 5*3600
        assert_eq!(ts, 1705332600);
    }

    #[test]
    fn parse_iso8601_epoch() {
        let ts = parse_iso8601_to_unix("1970-01-01T00:00:00Z").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn parse_iso8601_invalid() {
        assert!(parse_iso8601_to_unix("not a date").is_none());
        assert!(parse_iso8601_to_unix("").is_none());
    }

    // -- ImportResult structure test --

    #[test]
    fn import_result_structure() {
        let r = ImportResult {
            commits_imported: 50,
            commits_skipped: 5,
            blobs_downloaded: 200,
            timeline: "main".into(),
        };
        assert_eq!(r.commits_imported, 50);
        assert_eq!(r.timeline, "main");
    }

    // -- SyncResult new fields --

    #[test]
    fn sync_result_new_fields() {
        let r = SyncResult {
            added: vec![],
            modified: vec![],
            deleted: vec![],
            no_changes: true,
            was_fast_forward: false,
            was_fused: false,
            conflicts: vec![],
        };
        assert!(r.no_changes);
        assert!(!r.was_fast_forward);
        assert!(!r.was_fused);
        assert!(r.conflicts.is_empty());
    }

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

    #[test]
    fn resolve_parent_existing_branch_takes_priority() {
        // When a branch already exists on GitHub, use its tip SHA
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let repo = Repo::open(work_dir).unwrap();

        let leaf = Leaf::new(B3Hash::digest(b"tree"), "feature", "author", 1000, "msg");
        let mapping = HashMapping::new(&repo.ivaldi_dir);

        let parents = resolve_github_parent(
            &repo,
            &leaf,
            &mapping,
            Some("existing_sha_on_github"),
            false,
        );
        assert_eq!(parents, vec!["existing_sha_on_github"]);
    }

    #[test]
    fn resolve_parent_new_branch_with_mapped_parent() {
        // New branch where the parent leaf has a known GitHub SHA mapping
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        // Create a parent commit on main
        let parent_tree = B3Hash::digest(b"parent tree");
        repo.commit(parent_tree, "author", "parent commit").unwrap();

        // Create a child commit (simulating branch)
        let child_tree = B3Hash::digest(b"child tree");
        repo.commit(child_tree, "author", "child commit").unwrap();

        // Get the child leaf
        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();

        // Get the parent leaf and map its BLAKE3 hash to a fake GitHub SHA1
        let parent_leaf = repo.get_leaf(head_leaf.prev_idx).unwrap().unwrap();
        let parent_blake3 = parent_leaf.hash();
        let fake_github_sha = "aabbccdd00112233445566778899aabbccddeeff";

        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        mapping.insert(fake_github_sha, parent_blake3);

        // No existing branch on GitHub (None) → should resolve via hash mapping
        let parents = resolve_github_parent(&repo, &head_leaf, &mapping, None, false);
        assert_eq!(parents, vec![fake_github_sha]);
    }

    #[test]
    fn resolve_parent_new_branch_unmapped_parent_returns_empty() {
        // New branch where the parent leaf has no GitHub SHA mapping → root commit
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        // Two commits on main
        repo.commit(B3Hash::digest(b"t1"), "author", "first")
            .unwrap();
        repo.commit(B3Hash::digest(b"t2"), "author", "second")
            .unwrap();

        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();
        assert!(head_leaf.has_parent());

        // Empty mapping — parent was never uploaded
        let mapping = HashMapping::new(&repo.ivaldi_dir);

        let parents = resolve_github_parent(&repo, &head_leaf, &mapping, None, false);
        assert!(
            parents.is_empty(),
            "should be root commit when parent not mapped"
        );
    }

    #[test]
    fn resolve_parent_no_parent_leaf_returns_empty() {
        // First commit on a timeline (no parent) → root commit
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        repo.commit(B3Hash::digest(b"tree"), "author", "initial")
            .unwrap();

        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();
        assert!(!head_leaf.has_parent());

        let mapping = HashMapping::new(&repo.ivaldi_dir);

        let parents = resolve_github_parent(&repo, &head_leaf, &mapping, None, false);
        assert!(parents.is_empty(), "first commit should have no parent");
    }

    #[test]
    fn resolve_parent_existing_branch_overrides_mapping() {
        // Even if parent leaf is mapped, existing branch SHA takes priority
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        repo.commit(B3Hash::digest(b"t1"), "author", "first")
            .unwrap();
        repo.commit(B3Hash::digest(b"t2"), "author", "second")
            .unwrap();

        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();

        let parent_leaf = repo.get_leaf(head_leaf.prev_idx).unwrap().unwrap();
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        mapping.insert("mapped_parent_sha", parent_leaf.hash());

        // Existing branch SHA should win over the mapping
        let parents =
            resolve_github_parent(&repo, &head_leaf, &mapping, Some("branch_tip_sha"), false);
        assert_eq!(parents, vec!["branch_tip_sha"]);
    }

    #[test]
    fn resolve_parent_force_skips_existing_branch() {
        // When force=true and existing branch SHA is provided, should walk leaf chain instead
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        repo.commit(B3Hash::digest(b"t1"), "author", "first")
            .unwrap();
        repo.commit(B3Hash::digest(b"t2"), "author", "second")
            .unwrap();

        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();

        let parent_leaf = repo.get_leaf(head_leaf.prev_idx).unwrap().unwrap();
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        mapping.insert("mapped_parent_sha", parent_leaf.hash());

        // force=true → should skip existing branch tip and walk leaf chain
        let parents = resolve_github_parent(
            &repo,
            &head_leaf,
            &mapping,
            Some("old_broken_tip_sha"),
            true,
        );
        assert_eq!(
            parents,
            vec!["mapped_parent_sha"],
            "force should skip existing branch tip and use mapped parent"
        );
    }

    #[test]
    fn resolve_parent_force_with_mapped_parent() {
        // force=true + mapped parent → returns parent SHA from mapping, not existing branch tip
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path();
        crate::forge::forge(work_dir).unwrap();
        let mut repo = Repo::open(work_dir).unwrap();

        // Create parent (simulates synced main)
        repo.commit(B3Hash::digest(b"main-tree"), "author", "synced main")
            .unwrap();
        let main_head = repo.get_timeline_head("main").unwrap().unwrap();
        let main_leaf = repo.get_leaf(main_head).unwrap().unwrap();

        // Create child on top (simulates feature after fuse)
        repo.commit(B3Hash::digest(b"feature-tree"), "author", "feature work")
            .unwrap();
        let feature_head = repo.get_timeline_head("main").unwrap().unwrap();
        let feature_leaf = repo.get_leaf(feature_head).unwrap().unwrap();

        // Map main's leaf BLAKE3 → a GitHub SHA (as sync_timeline would do)
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let main_github_sha = "1111111111222222222233333333334444444444";
        mapping.insert(main_github_sha, main_leaf.hash());

        // force=true with an existing broken branch tip
        let parents = resolve_github_parent(
            &repo,
            &feature_leaf,
            &mapping,
            Some("old_broken_branch_tip"),
            true,
        );
        assert_eq!(
            parents,
            vec![main_github_sha],
            "force upload should resolve parent via leaf chain, not existing branch tip"
        );
    }

    #[test]
    fn resolve_parent_walks_backwards_through_unmapped() {
        // Scenario: A → B → C (head), only A is mapped to GitHub
        // Should walk C→B→A and find A's mapping
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();

        // Commit A (will be mapped)
        repo.commit(B3Hash::digest(b"t1"), "author", "commit A")
            .unwrap();
        let a_leaf = repo.get_leaf(0).unwrap().unwrap();

        // Commit B (unmapped)
        repo.commit(B3Hash::digest(b"t2"), "author", "commit B")
            .unwrap();

        // Commit C (unmapped, this is the head)
        repo.commit(B3Hash::digest(b"t3"), "author", "commit C")
            .unwrap();

        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let head_leaf = repo.get_leaf(head_idx).unwrap().unwrap();
        assert_eq!(head_leaf.prev_idx, 1); // points to B

        // Only map A
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let a_sha = "aaaa111122223333444455556666777788889999";
        mapping.insert(a_sha, a_leaf.hash());

        let parents = resolve_github_parent(&repo, &head_leaf, &mapping, None, false);
        assert_eq!(parents, vec![a_sha], "should walk past B to find mapped A");
    }

    #[test]
    fn resolve_parent_merge_returns_both_parents() {
        // Scenario: merge commit with prev_idx=A (mapped) and merge_idx=B (mapped)
        // Should return both SHA1s
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();

        // Branch point
        repo.commit(B3Hash::digest(b"base"), "author", "base")
            .unwrap();

        // "main" commit
        repo.commit(B3Hash::digest(b"main-work"), "author", "main work")
            .unwrap();
        let main_leaf = repo.get_leaf(1).unwrap().unwrap();

        // "feature" commit (simulate by raw commit with prev=base)
        let mut feat_leaf_data =
            Leaf::new(B3Hash::digest(b"feat"), "main", "author", 1000, "feature");
        feat_leaf_data.prev_idx = 0;
        let feat_result = repo.commit_raw(feat_leaf_data, "main").unwrap();
        let feat_leaf = repo.get_leaf(feat_result.index).unwrap().unwrap();

        // Merge commit: prev_idx=main(1), merge_idxs=[feature(2)]
        let mut merge_leaf_data =
            Leaf::new(B3Hash::digest(b"merged"), "main", "author", 2000, "merge");
        merge_leaf_data.prev_idx = 1;
        merge_leaf_data.merge_idxs = vec![feat_result.index];
        let merge_result = repo.commit_raw(merge_leaf_data, "main").unwrap();
        let merge_leaf = repo.get_leaf(merge_result.index).unwrap().unwrap();

        // Map both parents
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let main_sha = "1111111111111111111111111111111111111111";
        let feat_sha = "2222222222222222222222222222222222222222";
        mapping.insert(main_sha, main_leaf.hash());
        mapping.insert(feat_sha, feat_leaf.hash());

        let parents = resolve_github_parent(&repo, &merge_leaf, &mapping, None, false);
        assert_eq!(parents.len(), 2, "merge commit should resolve both parents");
        assert!(parents.contains(&main_sha.to_string()));
        assert!(parents.contains(&feat_sha.to_string()));
    }

    #[test]
    fn resolve_parent_merge_walks_merge_parent_chain() {
        // Scenario: merge commit with merge_idx pointing to unmapped commit
        // whose parent IS mapped → should walk the merge parent chain
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();

        // A (mapped)
        repo.commit(B3Hash::digest(b"a"), "author", "A").unwrap();
        let a_leaf = repo.get_leaf(0).unwrap().unwrap();

        // B (unmapped, parent=A)
        repo.commit(B3Hash::digest(b"b"), "author", "B").unwrap();

        // C (the merge parent, unmapped, parent=B)
        repo.commit(B3Hash::digest(b"c"), "author", "C").unwrap();

        // D (merge commit: prev=C, merge_idxs=[])
        // But we'll make a separate commit to be the "other branch"
        let mut other = Leaf::new(B3Hash::digest(b"other"), "main", "author", 1000, "other");
        other.prev_idx = 0; // parent=A
        let other_result = repo.commit_raw(other, "main").unwrap();

        // Merge: prev=2(C), merge_idxs=[3(other)]
        let mut merge = Leaf::new(B3Hash::digest(b"merge"), "main", "author", 2000, "merge");
        merge.prev_idx = 2;
        merge.merge_idxs = vec![other_result.index];
        let merge_result = repo.commit_raw(merge, "main").unwrap();
        let merge_leaf = repo.get_leaf(merge_result.index).unwrap().unwrap();

        // Only A is mapped
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let a_sha = "aaaa000011112222333344445555666677778888";
        mapping.insert(a_sha, a_leaf.hash());

        let parents = resolve_github_parent(&repo, &merge_leaf, &mapping, None, false);
        // prev chain: C→B→A (mapped) → found
        // merge chain: other→A (mapped) → found, but A is already in parents
        assert_eq!(parents.len(), 1, "both chains converge on A");
        assert_eq!(parents[0], a_sha);
    }

    // -- checkout_tree_to_workspace regression tests --

    /// Helper: build a tree in the CAS from a map of path→content, commit it,
    /// and return the repo + CAS for checkout testing.
    fn setup_checkout_repo(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> (Repo, FileCas) {
        crate::forge::forge(dir).unwrap();
        let mut repo = Repo::open(dir).unwrap();
        let ivaldi_dir = dir.join(".ivaldi");
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let tree_hash = store.build_tree_from_map(files).unwrap();
        repo.commit(tree_hash, "test-author", "test commit")
            .unwrap();
        (repo, cas)
    }

    #[test]
    fn checkout_writes_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"hello a".to_vec());
        files.insert("b.txt".into(), b"hello b".to_vec());
        let (repo, cas) = setup_checkout_repo(dir.path(), &files);
        let store = FsStore::new(&cas);

        let count = checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello a"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "hello b"
        );
    }

    #[test]
    fn checkout_deletes_removed_files() {
        let dir = tempfile::tempdir().unwrap();

        // Initial commit with A, B, C
        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"aaa".to_vec());
        files.insert("b.txt".into(), b"bbb".to_vec());
        files.insert("c.txt".into(), b"ccc".to_vec());
        let (mut repo, cas) = setup_checkout_repo(dir.path(), &files);
        let store = FsStore::new(&cas);

        // Checkout first commit — all three files present
        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert!(dir.path().join("c.txt").exists());

        // Second commit with only A, B (C removed)
        let mut files2 = BTreeMap::new();
        files2.insert("a.txt".into(), b"aaa".to_vec());
        files2.insert("b.txt".into(), b"bbb".to_vec());
        let tree2 = store.build_tree_from_map(&files2).unwrap();
        repo.commit(tree2, "test-author", "remove c").unwrap();

        // Checkout second commit — C should be deleted
        let count = checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert_eq!(count, 2);
        assert!(dir.path().join("a.txt").exists());
        assert!(dir.path().join("b.txt").exists());
        assert!(
            !dir.path().join("c.txt").exists(),
            "c.txt should be deleted"
        );
    }

    #[test]
    fn checkout_handles_modified_files() {
        let dir = tempfile::tempdir().unwrap();

        // Initial commit
        let mut files = BTreeMap::new();
        files.insert("doc.txt".into(), b"version 1".to_vec());
        let (mut repo, cas) = setup_checkout_repo(dir.path(), &files);
        let store = FsStore::new(&cas);
        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "version 1"
        );

        // Second commit with modified content
        let mut files2 = BTreeMap::new();
        files2.insert("doc.txt".into(), b"version 2".to_vec());
        let tree2 = store.build_tree_from_map(&files2).unwrap();
        repo.commit(tree2, "test-author", "update doc").unwrap();

        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "version 2"
        );
    }

    #[test]
    fn checkout_preserves_ignored_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create .ivaldiignore before forging so it's present
        let ignore_path = dir.path().join(".ivaldiignore");
        fs::write(&ignore_path, "secret.key\n").unwrap();

        // Initial commit with one tracked file
        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"tracked".to_vec());
        let (repo, cas) = setup_checkout_repo(dir.path(), &files);
        let store = FsStore::new(&cas);

        // Place an ignored file in the workspace
        fs::write(dir.path().join("secret.key"), "private data").unwrap();

        // Re-write .ivaldiignore (forge may overwrite)
        fs::write(&ignore_path, "secret.key\n").unwrap();

        checkout_tree_to_workspace(&repo, &store, "main").unwrap();

        // Ignored file should still be there
        assert!(
            dir.path().join("secret.key").exists(),
            "ignored file should not be deleted by checkout"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("secret.key")).unwrap(),
            "private data"
        );
    }

    #[test]
    fn checkout_cleans_empty_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();

        // Commit with a file in a subdirectory
        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"root file".to_vec());
        files.insert("sub/deep.txt".into(), b"deep file".to_vec());
        let (mut repo, cas) = setup_checkout_repo(dir.path(), &files);
        let store = FsStore::new(&cas);
        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert!(dir.path().join("sub/deep.txt").exists());

        // Second commit without the subdirectory file
        let mut files2 = BTreeMap::new();
        files2.insert("a.txt".into(), b"root file".to_vec());
        let tree2 = store.build_tree_from_map(&files2).unwrap();
        repo.commit(tree2, "test-author", "remove sub/deep.txt")
            .unwrap();

        checkout_tree_to_workspace(&repo, &store, "main").unwrap();
        assert!(!dir.path().join("sub/deep.txt").exists());
        assert!(
            !dir.path().join("sub").exists(),
            "empty sub/ dir should be cleaned up"
        );
    }

    #[test]
    fn compute_delta_ignores_cross_timeline_ancestors() {
        // Regression: when a feature branch was uploaded and later merged on
        // GitHub, sync_timeline would pick the feature branch commit as the
        // common ancestor.  Because that leaf lives on a different local
        // timeline, the divergence detector would over-count local commits,
        // falsely triggering a fuse that deleted the merged files.
        //
        // The fix constrains the common-ancestor search to leaves reachable
        // from the LOCAL timeline's head.
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();

        // Commit on main (simulates the initial synced state)
        let main_tree = B3Hash::digest(b"main-tree");
        repo.commit(main_tree, "author", "initial main").unwrap();
        let main_head = repo.get_timeline_head("main").unwrap().unwrap();

        // Create a feature timeline with a different commit
        let feat_tree = B3Hash::digest(b"feat-tree");
        let mut feat_leaf = Leaf::new(feat_tree, "feature", "author", 2000, "feat work");
        feat_leaf.prev_idx = crate::leaf::NO_PARENT;
        let feat_result = repo.commit_raw(feat_leaf, "feature").unwrap();

        // Map both to fake GitHub SHAs (simulating upload of both)
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let main_sha = "aaaa111122223333444455556666777788889999";
        let feat_sha = "bbbb111122223333444455556666777788889999";
        let main_leaf = repo.get_leaf(main_head).unwrap().unwrap();
        mapping.insert(main_sha, main_leaf.hash());
        let feat_leaf_stored = repo.get_leaf(feat_result.index).unwrap().unwrap();
        mapping.insert(feat_sha, feat_leaf_stored.hash());

        // Build local_reachable from main's head
        let local_reachable: BTreeSet<u64> = {
            let mut reachable = BTreeSet::new();
            let mut cur = Some(main_head);
            while let Some(idx) = cur {
                reachable.insert(idx);
                if let Ok(Some(leaf)) = repo.get_leaf(idx) {
                    cur = if leaf.has_parent() {
                        Some(leaf.prev_idx)
                    } else {
                        None
                    };
                } else {
                    break;
                }
            }
            reachable
        };

        // Feature leaf should NOT be in main's reachable set
        assert!(
            !local_reachable.contains(&feat_result.index),
            "feature commit must not be reachable from main"
        );
        // Main leaf SHOULD be reachable
        assert!(
            local_reachable.contains(&main_head),
            "main head must be in its own reachable set"
        );
    }

    #[test]
    fn upload_rejects_security_blocked_files() {
        use crate::cas::MemoryCas;

        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        std::fs::create_dir_all(&ivaldi_dir).unwrap();

        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        // Build a file map containing a .env file
        let mut files = BTreeMap::new();
        let content = b"SECRET=abc";
        let canonical = crate::fsmerkle::BlobNode::canonical_bytes(content);
        let hash = B3Hash::digest(&canonical);
        cas.put(hash, &canonical).unwrap();
        files.insert(".env".to_string(), hash);

        let client = GitHubClient::new();
        let mut mapping = HashMapping::new(&ivaldi_dir);

        let result = upload_blobs_parallel(&client, &store, &files, &mut mapping, "owner", "repo");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("security-blocked"),
            "expected security-blocked error, got: {err}"
        );
    }
}
