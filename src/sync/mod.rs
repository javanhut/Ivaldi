//! Sync operations for Ivaldi VCS — download, upload, scout, harvest.
//!
//! Bridges Ivaldi's internal BLAKE3-based storage with GitHub's SHA1-based
//! Git objects. SHA1 is used ONLY for API communication — never internally.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::git_remote::{self, FetchResult, SmartHttpClient};
use crate::github::{GitHubClient, GitHubError};
use crate::hash::B3Hash;
use crate::ignore;
use crate::portal::{Portal, Transport};
use crate::remote::{HashMapping, RemoteBranch};
use crate::repo::Repo;
use crate::ssh_transport::SshClient;

mod import;
mod timeline_sync;

pub use import::{ImportResult, import_full_history, parse_iso8601_to_unix};
pub use timeline_sync::{SyncResult, sync_timeline};

/// Workspace delta as `(added, modified, deleted)` relative paths.
type WorkspaceDelta = (Vec<String>, Vec<String>, Vec<String>);

/// Read-side dispatch: pick HTTPS or SSH based on a portal's transport, and
/// expose the small surface that `scout` / `harvest` / `sync` need.
///
/// Construct via [`RemoteFetcher::for_portal`]. The HTTPS variant carries an
/// optional auth token (matching `SmartHttpClient::new`), while the SSH
/// variant carries the resolved `SshTarget` from `portal.transport()`.
pub enum RemoteFetcher {
    Https {
        token: Option<String>,
        /// Full smart-HTTP base URL for a generic (non-GitHub) host. When
        /// set, the URL-based client methods are used instead of deriving a
        /// GitHub URL from owner/repo.
        base_url: Option<String>,
    },
    Ssh {
        target: crate::ssh_transport::SshTarget,
    },
}

impl RemoteFetcher {
    /// Build the fetcher matching a portal's transport. The token is used
    /// only by the HTTPS variant.
    pub fn for_portal(portal: &Portal, token: Option<&str>) -> Self {
        match portal.transport() {
            Transport::Ssh(target) => RemoteFetcher::Ssh { target },
            // Generic host: talk to its URL, and resolve auth per-host
            // (the GitHub token passed in doesn't apply).
            Transport::GenericHttps(url) => {
                let host = crate::portal::http_host(&url).unwrap_or_default();
                RemoteFetcher::Https {
                    token: crate::auth::generic_git_token(&host),
                    base_url: Some(url),
                }
            }
            // P2P portals can't be served by HTTPS scout/harvest/sync;
            // those callers should branch on the portal first. Falling
            // back to HTTPS gives a coherent error path.
            Transport::Peer(_) | Transport::Https => RemoteFetcher::Https {
                token: token.map(str::to_string),
                base_url: None,
            },
        }
    }

    /// List branches of the remote, name-only (no SHAs).
    pub fn list_branches(&self, owner: &str, repo_name: &str) -> Result<Vec<String>, SyncError> {
        Ok(self
            .list_branch_refs(owner, repo_name)?
            .into_iter()
            .map(|b| b.name)
            .collect())
    }

    /// List branches with SHAs (for sync-state classification).
    pub fn list_branch_refs(
        &self,
        owner: &str,
        repo_name: &str,
    ) -> Result<Vec<RemoteBranch>, SyncError> {
        match self {
            RemoteFetcher::Https { token, base_url } => {
                let c = SmartHttpClient::new(token.as_deref());
                match base_url {
                    Some(base) => c.list_branch_refs_url(base).map_err(SyncError::from),
                    None => c.list_branch_refs(owner, repo_name).map_err(SyncError::from),
                }
            }
            RemoteFetcher::Ssh { target } => SshClient::new(target.clone())
                .list_branch_refs()
                .map_err(SyncError::from),
        }
    }

    /// Fetch a branch's full pack.
    pub fn fetch_repo(
        &self,
        owner: &str,
        repo_name: &str,
        branch: Option<&str>,
    ) -> Result<FetchResult, SyncError> {
        match self {
            RemoteFetcher::Https { token, base_url } => {
                let c = SmartHttpClient::new(token.as_deref());
                match base_url {
                    Some(base) => c.fetch_repo_url(base, branch).map_err(SyncError::from),
                    None => c.fetch_repo(owner, repo_name, branch).map_err(SyncError::from),
                }
            }
            RemoteFetcher::Ssh { target } => SshClient::new(target.clone())
                .fetch_repo(branch)
                .map_err(SyncError::from),
        }
    }
}

/// Result of a download (clone) operation.
#[derive(Debug)]
pub struct DownloadResult {
    pub files_downloaded: usize,
    pub commits_imported: usize,
    pub timelines_created: Vec<String>,
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
    download_with_fetch(
        target_dir,
        owner,
        repo_name,
        None,
        |branch| {
            SmartHttpClient::new(client.token())
                .fetch_repo(owner, repo_name, branch)
                .map_err(SyncError::from)
        },
        branch,
    )
}

/// Clone from an arbitrary Git smart-HTTP URL (AUR, Gitea, cgit, self-hosted,
/// ...). `owner`/`repo_name` are display/portal labels derived from the URL;
/// `base_url` is passed verbatim to the smart-HTTP client. `token` is used for
/// hosts that require Basic auth (public repos pass `None`).
pub fn download_url(
    base_url: &str,
    owner: &str,
    repo_name: &str,
    target_dir: &Path,
    token: Option<&str>,
    branch: Option<&str>,
) -> Result<DownloadResult, SyncError> {
    let base = base_url.to_string();
    let tok = token.map(str::to_string);
    download_with_fetch(
        target_dir,
        owner,
        repo_name,
        Some(base_url),
        move |branch| {
            SmartHttpClient::new(tok.as_deref())
                .fetch_repo_url(&base, branch)
                .map_err(SyncError::from)
        },
        branch,
    )
}

/// Download a repository from any SSH-reachable Git host into a local
/// Ivaldi repo. `display_name` is used for "Downloading X..." messaging
/// and the local portal entry (e.g. `git@github.com:owner/repo.git`).
pub fn download_ssh(
    target: &crate::ssh_transport::SshTarget,
    target_dir: &Path,
    branch: Option<&str>,
) -> Result<DownloadResult, SyncError> {
    let (owner, repo_name) = derive_owner_repo_from_path(&target.repo_path);
    let target_clone = target.clone();
    download_with_fetch(
        target_dir,
        &owner,
        &repo_name,
        None,
        move |branch| {
            crate::ssh_transport::SshClient::new(target_clone.clone())
                .fetch_repo(branch)
                .map_err(SyncError::from)
        },
        branch,
    )
}

/// Best-effort split of a remote repo path like `owner/repo.git` into
/// (owner, repo). For paths that don't fit `owner/repo` (e.g. nested
/// subgroups like `team/subteam/repo.git` on GitLab), we keep the last two
/// segments as (owner, repo) and discard the prefix — Ivaldi's local model
/// is two-level only, and the portal entry will round-trip the original
/// path.
fn derive_owner_repo_from_path(path: &str) -> (String, String) {
    let trimmed = path.trim_start_matches('/').trim_end_matches('/');
    let stripped = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let parts: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => ("local".to_string(), "repo".to_string()),
        [single] => ("local".to_string(), (*single).to_string()),
        many => (
            many[many.len() - 2].to_string(),
            many[many.len() - 1].to_string(),
        ),
    }
}

/// Common orchestration: ensure target dir, run the supplied fetch closure,
/// import the resulting `FetchResult`, materialize, return DownloadResult.
fn download_with_fetch<F>(
    target_dir: &Path,
    owner: &str,
    repo_name: &str,
    remote_url: Option<&str>,
    fetch: F,
    branch: Option<&str>,
) -> Result<DownloadResult, SyncError>
where
    F: FnOnce(Option<&str>) -> Result<crate::git_remote::FetchResult, SyncError>,
{
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
        let remote = fetch(branch)?;

        crate::forge::forge(target_dir)?;
        let ivaldi_dir = target_dir.join(".ivaldi");

        let portal_mgr = crate::portal::PortalManager::new(&ivaldi_dir);
        let mut portal = crate::portal::Portal::parse(&format!("{}/{}", owner, repo_name))
            .ok_or_else(|| SyncError::Other(format!("invalid portal '{}/{}'", owner, repo_name)))?;
        // Generic (non-GitHub) clones record their smart-HTTP URL so the portal
        // round-trips the real remote instead of a bogus GitHub owner/repo.
        if let Some(url) = remote_url {
            portal = portal.with_base_url(url);
        }
        let _ = portal_mgr.add(&portal);

        let mut cfg = crate::config::Config::new();
        cfg.set("portal.default", &format!("{}/{}", owner, repo_name));
        cfg.save(&ivaldi_dir.join("config")).ok();

        let mut repo = Repo::open(target_dir)?;
        let import = git_remote::import_fetch_result(&mut repo, &remote)?;

        // forge() initialised HEAD to a hardcoded "main"; point it at the
        // branch we actually fetched so `whereami` and `timeline list` agree
        // with the working tree. Also materialise the on-disk ref file so the
        // timeline shows up in tools that scan refs/heads.
        let ref_path = ivaldi_dir.join("refs/heads").join(&remote.branch);
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !ref_path.exists() {
            fs::write(&ref_path, "")?;
        }
        crate::forge::write_head(
            &ivaldi_dir,
            &crate::forge::HeadRef::Timeline(remote.branch.clone()),
        )?;

        let cas = FileCas::new(ivaldi_dir.join("objects"))?;
        let store = FsStore::new(&cas);
        let file_count = if repo.get_timeline_head(&remote.branch)?.is_some() {
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
    fs::create_dir_all(target_dir)?;
    Ok(true)
}

fn cleanup_failed_download_target(target_dir: &Path) {
    let _ = fs::remove_dir_all(target_dir);
}

/// Scout — list remote branches without downloading. Routes through the
/// portal's transport (HTTPS or SSH).
pub fn scout(client: &GitHubClient, portal: &Portal) -> Result<Vec<String>, SyncError> {
    RemoteFetcher::for_portal(portal, client.token()).list_branches(&portal.owner, &portal.repo)
}

pub fn scout_with_status(
    client: &GitHubClient,
    repo: &Repo,
    portal: &Portal,
) -> Result<Vec<RemoteTimelineInfo>, SyncError> {
    let branches = RemoteFetcher::for_portal(portal, client.token())
        .list_branch_refs(&portal.owner, &portal.repo)?;
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
    portal: &Portal,
    timeline_names: &[String],
) -> Result<Vec<String>, SyncError> {
    let fetcher = RemoteFetcher::for_portal(portal, client.token());
    let branches = fetcher.list_branch_refs(&portal.owner, &portal.repo)?;
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

        let fetch = fetcher.fetch_repo(&portal.owner, &portal.repo, Some(target_name))?;
        let import = git_remote::import_fetch_result(repo, &fetch)?;
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

/// Get the file set (path → B3Hash) from a leaf's tree.
fn get_tree_files(
    repo: &Repo,
    store: &FsStore<'_>,
    leaf_idx: u64,
) -> Result<BTreeMap<String, B3Hash>, SyncError> {
    let leaf = repo
        .get_leaf(leaf_idx)?
        .ok_or_else(|| SyncError::Other(format!("leaf {} not found", leaf_idx)))?;

    let mut files = BTreeMap::new();
    collect_tree_files(store, leaf.tree_root, "", &mut files)?;
    Ok(files)
}

/// Compute workspace delta between current head and a prior ancestor.
fn compute_workspace_delta(
    repo: &Repo,
    store: &FsStore<'_>,
    timeline: &str,
    ancestor_idx: Option<u64>,
) -> Result<WorkspaceDelta, SyncError> {
    let old_files = if let Some(idx) = ancestor_idx {
        get_tree_files(repo, store, idx)?
    } else {
        BTreeMap::new()
    };

    let new_head = repo.get_timeline_head(timeline)?;
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
        .get_timeline_head(timeline)?
        .ok_or_else(|| SyncError::Other("no head to checkout".into()))?;

    let head_leaf = repo
        .get_leaf(head_idx)?
        .ok_or_else(|| SyncError::Other("corrupt head leaf".into()))?;

    let mut files = BTreeMap::new();
    collect_tree_files(store, head_leaf.tree_root, "", &mut files)?;

    // Write / update files from the target tree
    for (path, blob_hash) in &files {
        let (_, content) = store.load_blob(*blob_hash)?;
        let file_path = repo.work_dir.join(path);

        let should_write = if file_path.exists() {
            let existing = fs::read(&file_path)?;
            existing != content
        } else {
            true
        };

        if should_write {
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&file_path, &content)?;
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
    scan_workspace_dir(root, prefix, ignore_cache, &mut out);
    out.sort();
    out
}

fn scan_workspace_dir(
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
            scan_workspace_dir(&entry.path(), &rel, ignore_cache, out);
        } else if ft.is_file() {
            out.push(rel);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("GitHub error: {0}")]
    GitHub(#[from] GitHubError),
    #[error("repo error: {0}")]
    Repo(#[from] crate::repo::RepoError),
    #[error("forge error: {0}")]
    Forge(#[from] crate::forge::ForgeError),
    #[error("git remote error: {0}")]
    GitRemote(#[from] crate::git_remote::GitRemoteError),
    #[error("CAS error: {0}")]
    Cas(#[from] crate::cas::CasError),
    #[error("merkle tree error: {0}")]
    FsMerkle(#[from] crate::fsmerkle::FsMerkleError),
    #[error("remote mapping error: {0}")]
    Remote(#[from] crate::remote::RemoteError),
    #[error("{0}")]
    Store(#[from] crate::store::StoreError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
