//! Upload (push) a local timeline to GitHub.
//!
//! Replays unpushed Ivaldi leaves as individual GitHub commits so local
//! history is preserved on the remote.

use std::collections::BTreeMap;

use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::github::{CommitIdentity, GitHubClient, GitHubError, TreeEntryCreate};
use crate::hash::B3Hash;
use crate::leaf::Leaf;
use crate::remote::HashMapping;
use crate::repo::Repo;

use super::{SyncError, collect_tree_files};

/// Result of an upload (push) operation.
#[derive(Debug)]
pub struct UploadResult {
    pub files_uploaded: usize,
    pub commit_sha: String,
    pub branch: String,
}

/// Upload blobs in parallel, skipping those already mapped.
///
/// Returns `TreeEntryCreate` entries for the GitHub tree API.
pub(super) fn upload_blobs_parallel(
    client: &GitHubClient,
    store: &FsStore<'_>,
    files: &BTreeMap<String, B3Hash>,
    hash_mapping: &mut HashMapping,
    owner: &str,
    repo_name: &str,
) -> Result<Vec<TreeEntryCreate>, SyncError> {
    // Defense-in-depth: reject security-blocked files before upload
    for path in files.keys() {
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
        let (_, content) = store.load_blob(*blob_hash)?;
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
    let (timeline, head_idx) = check_auth_and_timeline(client, repo)?;
    let branch_name = branch.unwrap_or(&timeline);

    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);

    let bootstrapped = bootstrap_if_empty(client, owner, repo_name, branch_name)?;
    let existing_branch_sha = find_existing_branch_sha(client, owner, repo_name, branch_name);

    // After a bootstrap, the seed commit on the branch is not a real ancestor
    // of our local history, so treat this like a force-push for parent
    // resolution and ref update to replace the placeholder commit.
    let effective_force = force || bootstrapped;
    let is_new_branch = existing_branch_sha.is_none();

    // Walk back from head to the deepest already-mapped ancestor (or root) and
    // collect the chronological list of unpushed leaves. Each one is replayed
    // as its own GitHub commit so local history is preserved on the remote.
    let replay_indices = collect_unpushed_leaves(repo, head_idx, &hash_mapping)?;

    if replay_indices.is_empty() {
        // Head is already mapped; nothing to push beyond a possible ref move.
        return mapped_head_result(repo, head_idx, &hash_mapping, branch_name);
    }

    let (last_commit_sha, total_blobs_uploaded) = replay_leaves_to_github(
        client,
        repo,
        owner,
        repo_name,
        &replay_indices,
        &mut hash_mapping,
        ParentResolution {
            existing_branch_sha: existing_branch_sha.as_deref(),
            force: effective_force,
        },
    )?;

    hash_mapping.save()?;

    finalize_upload_ref(
        client,
        owner,
        repo_name,
        branch_name,
        &last_commit_sha,
        is_new_branch,
        effective_force,
    )?;

    Ok(UploadResult {
        files_uploaded: total_blobs_uploaded,
        commit_sha: last_commit_sha,
        branch: branch_name.to_string(),
    })
}

/// Upload step 1: require authentication and resolve the current timeline
/// plus its head leaf index.
fn check_auth_and_timeline(client: &GitHubClient, repo: &Repo) -> Result<(String, u64), SyncError> {
    if !client.is_authenticated() {
        return Err(SyncError::GitHub(GitHubError::AuthRequired));
    }

    let timeline = repo.current_timeline()?;
    let head_idx = repo
        .get_timeline_head(&timeline)?
        .ok_or_else(|| SyncError::Other("no commits to upload".into()))?;
    Ok((timeline, head_idx))
}

/// Upload step 2: GitHub's Git Data API returns 409 on every endpoint (blobs
/// included) when the repo has no initial commit. Detect that up front and
/// seed the repo via the Contents API so the rest of the upload can proceed.
/// Returns `true` when a bootstrap commit was created.
fn bootstrap_if_empty(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    branch_name: &str,
) -> Result<bool, SyncError> {
    let existing_branches = client.list_branches(owner, repo_name)?;
    if !existing_branches.is_empty() {
        return Ok(false);
    }

    let default_branch = client
        .get_repo(owner, repo_name)
        .map(|info| info.default_branch)
        .unwrap_or_default();
    let seed_branch = if default_branch.is_empty() {
        branch_name
    } else {
        default_branch.as_str()
    };
    client.create_file_contents(
        owner,
        repo_name,
        ".ivaldi-bootstrap",
        seed_branch,
        b"Ivaldi bootstrap placeholder. Safe to remove after first upload.\n",
        "chore: initialize repository for Ivaldi",
    )?;
    Ok(true)
}

/// Upload step 3: look up the remote tip SHA of `branch_name`, if the branch
/// already exists on GitHub.
fn find_existing_branch_sha(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    branch_name: &str,
) -> Option<String> {
    client
        .list_branches(owner, repo_name)
        .ok()
        .and_then(|branches| {
            branches
                .iter()
                .find(|b| b.name == branch_name)
                .map(|b| b.commit.sha.clone())
        })
}

/// Build the `UploadResult` for a head that is already mapped on the remote
/// (nothing to push beyond a possible ref move).
fn mapped_head_result(
    repo: &Repo,
    head_idx: u64,
    hash_mapping: &HashMapping,
    branch_name: &str,
) -> Result<UploadResult, SyncError> {
    let head_leaf = repo
        .get_leaf(head_idx)?
        .ok_or_else(|| SyncError::Other("corrupt: head leaf not found".into()))?;
    let head_sha = hash_mapping
        .get_sha1(head_leaf.hash())
        .map(|s| s.to_string())
        .ok_or_else(|| SyncError::Other("head leaf unexpectedly unmapped".into()))?;
    Ok(UploadResult {
        files_uploaded: 0,
        commit_sha: head_sha,
        branch: branch_name.to_string(),
    })
}

/// Parent-resolution inputs for the FIRST replayed leaf — later leaves chain
/// onto the commit created in the previous iteration.
struct ParentResolution<'a> {
    existing_branch_sha: Option<&'a str>,
    force: bool,
}

/// Upload step 4: replay each unpushed leaf as its own GitHub commit
/// (blobs → tree → commit), recording SHA1 mappings as we go. Returns the
/// SHA of the last commit created and the number of blobs uploaded.
fn replay_leaves_to_github(
    client: &GitHubClient,
    repo: &Repo,
    owner: &str,
    repo_name: &str,
    replay_indices: &[u64],
    hash_mapping: &mut HashMapping,
    parent_resolution: ParentResolution<'_>,
) -> Result<(String, usize), SyncError> {
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let store = FsStore::new(&cas);

    let mut last_commit_sha = String::new();
    let mut total_blobs_uploaded = 0usize;

    for (i, &leaf_idx) in replay_indices.iter().enumerate() {
        let leaf = repo
            .get_leaf(leaf_idx)?
            .ok_or_else(|| SyncError::Other(format!("corrupt: leaf {} not found", leaf_idx)))?;

        let mut files = BTreeMap::new();
        collect_tree_files(&store, leaf.tree_root, "", &mut files)?;

        let blobs_before = hash_mapping.len();
        let tree_entries =
            upload_blobs_parallel(client, &store, &files, hash_mapping, owner, repo_name)?;
        total_blobs_uploaded += hash_mapping.len().saturating_sub(blobs_before);

        let tree_sha = client.create_tree(owner, repo_name, tree_entries, None)?;

        // Parents: for the first leaf in the chain, defer to existing parent
        // resolution (existing branch tip / mapped ancestor). For every later
        // leaf, the parent is whatever we just created in the previous
        // iteration plus any merge parents already mapped.
        let parents = if i == 0 {
            resolve_github_parent(
                repo,
                &leaf,
                hash_mapping,
                parent_resolution.existing_branch_sha,
                parent_resolution.force,
            )
        } else {
            let mut p = vec![last_commit_sha.clone()];
            for &midx in &leaf.merge_idxs {
                if let Ok(Some(merge_leaf)) = repo.get_leaf(midx)
                    && let Some(sha) = hash_mapping.get_sha1(merge_leaf.hash())
                {
                    let s = sha.to_string();
                    if !p.contains(&s) {
                        p.push(s);
                    }
                }
            }
            p
        };

        let author_id = identity_for_author(&leaf);
        let committer_id = identity_for_committer(&leaf);

        let commit_sha = client.create_commit(
            owner,
            repo_name,
            &leaf.message,
            &tree_sha,
            &parents,
            Some(&author_id),
            Some(&committer_id),
        )?;

        hash_mapping.insert(&commit_sha, leaf.hash());
        last_commit_sha = commit_sha;
    }

    Ok((last_commit_sha, total_blobs_uploaded))
}

/// Upload step 5: point the remote branch ref at the newly created tip.
fn finalize_upload_ref(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    branch_name: &str,
    last_commit_sha: &str,
    is_new_branch: bool,
    force: bool,
) -> Result<(), SyncError> {
    if is_new_branch {
        client.create_ref(owner, repo_name, branch_name, last_commit_sha)?;
    } else {
        client.update_ref(owner, repo_name, branch_name, last_commit_sha, force)?;
    }
    Ok(())
}

/// Walk from `head_idx` backwards along `prev_idx` and return the chronological
/// list of leaves whose BLAKE3 is NOT yet in `hash_mapping`. The list is empty
/// if the head is already mapped.
pub(super) fn collect_unpushed_leaves(
    repo: &Repo,
    head_idx: u64,
    hash_mapping: &HashMapping,
) -> Result<Vec<u64>, crate::repo::RepoError> {
    let mut chain = Vec::new();
    let mut cur = Some(head_idx);
    while let Some(idx) = cur {
        let leaf = match repo.get_leaf(idx)? {
            Some(l) => l,
            None => break,
        };
        if hash_mapping.get_sha1(leaf.hash()).is_some() {
            // First mapped ancestor — stop here.
            break;
        }
        chain.push(idx);
        cur = if leaf.has_parent() {
            Some(leaf.prev_idx)
        } else {
            None
        };
    }
    chain.reverse();
    Ok(chain)
}

/// Build the `author` identity for a GitHub commit from a leaf.
///
/// Prefers a per-leaf timezone offset stored in `meta["git.author_tz"]` (set
/// during import); falls back to UTC.
pub(super) fn identity_for_author(leaf: &Leaf) -> CommitIdentity {
    let (name, email) = split_author(&leaf.author);
    let tz = leaf
        .meta
        .get("git.author_tz")
        .map(String::as_str)
        .unwrap_or("+0000");
    CommitIdentity {
        name,
        email,
        date: format_rfc3339(leaf.time_unix, tz),
    }
}

/// Build the `committer` identity for a GitHub commit from a leaf.
///
/// Prefers per-leaf committer info stored in `meta` during import; otherwise
/// reuses the author (matches Git's default when the user only sets one).
pub(super) fn identity_for_committer(leaf: &Leaf) -> CommitIdentity {
    if let (Some(committer), Some(time_str)) = (
        leaf.meta.get("git.committer"),
        leaf.meta.get("git.committer_time"),
    ) {
        let time = time_str.parse::<i64>().unwrap_or(leaf.time_unix);
        let tz = leaf
            .meta
            .get("git.committer_tz")
            .map(String::as_str)
            .unwrap_or("+0000");
        let (name, email) = split_author(committer);
        return CommitIdentity {
            name,
            email,
            date: format_rfc3339(time, tz),
        };
    }
    identity_for_author(leaf)
}

/// Split a `"Name <email>"` string. Tolerates malformed input by returning the
/// whole string as the name and an empty email.
pub(super) fn split_author(s: &str) -> (String, String) {
    if let Some(open) = s.rfind(" <")
        && let Some(close) = s[open..].find('>')
    {
        let name = s[..open].trim().to_string();
        let email = s[open + 2..open + close].to_string();
        return (name, email);
    }
    (s.to_string(), String::new())
}

/// Format a unix-second timestamp + git-style timezone offset (e.g. `"+0000"`,
/// `"-0530"`) as RFC 3339 (`YYYY-MM-DDTHH:MM:SS±HH:MM`).
///
/// The offset is applied to the unix instant before splitting into civil
/// time so `1700000000` + `+0100` formats as `2023-11-14T23:13:20+01:00`.
pub(super) fn format_rfc3339(unix_seconds: i64, git_tz: &str) -> String {
    let (sign, hours, minutes) = parse_git_tz(git_tz);
    let offset_seconds = sign * (hours as i64 * 3600 + minutes as i64 * 60);
    let local = unix_seconds + offset_seconds;
    let (y, mo, d, h, mi, s) = civil_from_unix(local);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        y,
        mo,
        d,
        h,
        mi,
        s,
        if sign >= 0 { '+' } else { '-' },
        hours,
        minutes,
    )
}

/// Parse `"+HHMM"` / `"-HHMM"` into (sign, hours, minutes). Defaults to UTC
/// (`+0000`) on any parse error.
fn parse_git_tz(s: &str) -> (i64, u32, u32) {
    let bytes = s.as_bytes();
    if bytes.len() != 5 {
        return (1, 0, 0);
    }
    let sign: i64 = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return (1, 0, 0),
    };
    let h: u32 = std::str::from_utf8(&bytes[1..3])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let m: u32 = std::str::from_utf8(&bytes[3..5])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (sign, h, m)
}

/// Convert a unix-second timestamp into civil (year, month, day, hour, min, sec).
///
/// Implements Howard Hinnant's days_from_civil inverse for portability without
/// pulling in a full date crate.
fn civil_from_unix(unix_seconds: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = unix_seconds.div_euclid(86_400);
    let secs_of_day = unix_seconds.rem_euclid(86_400) as u32;
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;

    // Days since 1970-01-01 → civil date (Hinnant).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (year, m, d, h, mi, s)
}

/// Resolve the GitHub parent SHA(s) for a new commit.
///
/// Priority:
/// 1. Branch already exists on GitHub AND not force-pushing → use its tip SHA
/// 2. Walk Ivaldi leaf chain via `prev_idx` backwards until a mapped commit is found,
///    AND collect mapped merge parents. Returns ALL resolved parents so GitHub
///    gets correct merge topology.
/// 3. Fallback → no parents mapped → root commit
pub(super) fn resolve_github_parent(
    repo: &Repo,
    head_leaf: &crate::leaf::Leaf,
    hash_mapping: &HashMapping,
    existing_branch_sha: Option<&str>,
    force: bool,
) -> Vec<String> {
    // Priority 1: branch already exists on GitHub AND not force-pushing
    if !force && let Some(sha) = existing_branch_sha {
        return vec![sha.to_string()];
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
