//! Full commit-history import from GitHub into an Ivaldi repo.

use std::collections::{BTreeMap, HashMap};
use std::fs;

use crate::atomic_io::atomic_write;
use crate::cas::FileCas;
use crate::fsmerkle::FsStore;
use crate::github::{CommitInfo, GitHubClient, GitHubError, TreeResponse};
use crate::hash::B3Hash;
use crate::leaf::Leaf;
use crate::refname::timeline_ref_path;
use crate::remote::HashMapping;
use crate::repo::Repo;

use super::SyncError;

/// Result of downloading one blob: `(path, sha1, content)` on success, or
/// an error message on failure.
type BlobDownloadResult = Result<(String, String, Vec<u8>), String>;

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
    let (time_str, tz_offset_secs) = if let Some(stripped) = rest.strip_suffix('Z') {
        (stripped, 0i64)
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
    for md in month_days.iter().take((month - 1) as usize) {
        days += *md as i64;
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
pub(super) fn import_full_history_into(
    client: &GitHubClient,
    repo: &mut Repo,
    owner: &str,
    repo_name: &str,
    remote_branch: &str,
    local_timeline: &str,
    depth: usize,
) -> Result<ImportResult, SyncError> {
    // Fetch commits (newest-first from GitHub), then reverse to oldest-first
    // for correct parent ordering.
    let mut commits = client.list_commits(owner, repo_name, remote_branch, depth)?;
    commits.reverse();

    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))?;
    let store = FsStore::new(&cas);
    let mut hash_mapping = HashMapping::new(&repo.ivaldi_dir);

    // --- Phase 1: ensure timeline ref + diff commits against the mapping ---
    ensure_timeline_ref(repo, local_timeline)?;
    let (unskipped, unique_tree_shas) = collect_unimported_commits(&commits, &hash_mapping);

    // --- Phase 2: Parallel tree pre-fetch ---
    let prefetched_trees = prefetch_trees(client, owner, repo_name, &unique_tree_shas);

    // Phases 3 and 4 run inside a closure so the hash mapping is persisted
    // even when the import fails part-way: every blob and commit that did
    // land is durable and a retry skips it instead of redoing the work.
    let import_result: Result<(usize, usize, usize), SyncError> = (|| {
        // --- Phase 3: Global blob batch download ---
        let blobs_to_download =
            collect_blobs_to_download(&commits, &unskipped, &prefetched_trees, &hash_mapping);
        let blobs_downloaded = download_and_store_blobs(
            client,
            owner,
            repo_name,
            &blobs_to_download,
            &store,
            &mut hash_mapping,
        )?;
        // Make the downloaded blobs' directory entries durable before any
        // durable record (leaf transaction, mapping save) references them.
        cas.flush()?;
        crate::failpoint::fail_point("import.api.after_blobs");

        // --- Phase 4: Commit loop using build_tree_from_hash_map ---
        // Track SHA1 → leaf index for parent resolution
        let mut sha_to_idx: HashMap<String, u64> = HashMap::new();
        // Cache tree SHA → Ivaldi tree hash to avoid re-downloading identical trees
        let mut tree_cache: HashMap<String, B3Hash> = HashMap::new();

        let mut commits_imported = 0usize;
        let mut commits_skipped = 0usize;

        // SHAs present in this listing: used to tell "parent beyond the
        // depth-truncated window" (legitimate shallow root) apart from
        // "parent that should have been imported but wasn't" (hard error).
        let listed_shas: std::collections::HashSet<&str> =
            commits.iter().map(|c| c.sha.as_str()).collect();

        let pb = crate::progress::file_bar(commits.len() as u64, "Importing commits");
        for commit in &commits {
            pb.inc(1);

            // Skip only when the mapping resolves to a leaf that actually
            // exists locally — a mapping entry without its leaf is stale and
            // the commit must be re-imported, not silently trusted.
            if let Some(b3) = hash_mapping.get_blake3(&commit.sha)
                && let Some(idx) = find_leaf_idx_by_hash(repo, b3)
            {
                sha_to_idx.insert(commit.sha.clone(), idx);
                commits_skipped += 1;
                continue;
            }

            // Build tree using hash-based approach (Phase 4 optimization)
            let tree_sha = &commit.commit.tree.sha;
            let ivaldi_tree_hash = if let Some(&cached) = tree_cache.get(tree_sha) {
                cached
            } else {
                let tree_hash = ivaldi_tree_for_commit(
                    client,
                    owner,
                    repo_name,
                    &store,
                    tree_sha,
                    &prefetched_trees,
                    &hash_mapping,
                )?;
                tree_cache.insert(tree_sha.clone(), tree_hash);
                tree_hash
            };

            let leaf = build_import_leaf(
                commit,
                ivaldi_tree_hash,
                local_timeline,
                &sha_to_idx,
                &listed_shas,
            )?;
            // Flush this commit's tree nodes before the append-only leaf
            // transaction durably references them.
            cas.flush()?;
            let result = repo.commit_raw(leaf, local_timeline)?;
            crate::failpoint::fail_point("import.api.mid_commits");

            // Record mappings
            hash_mapping.insert(&commit.sha, result.hash);
            sha_to_idx.insert(commit.sha.clone(), result.index);
            commits_imported += 1;
        }
        pb.finish_with_message(format!(
            "{} commits imported, {} skipped",
            commits_imported, commits_skipped
        ));
        Ok((blobs_downloaded, commits_imported, commits_skipped))
    })();

    match hash_mapping.save() {
        Ok(()) => {}
        // Don't let a mapping-save failure mask the original import error.
        Err(e) if import_result.is_ok() => return Err(e.into()),
        Err(e) => crate::logging::warn(&format!("failed to save hash mapping: {}", e)),
    }
    let (blobs_downloaded, commits_imported, commits_skipped) = import_result?;

    Ok(ImportResult {
        commits_imported,
        commits_skipped,
        blobs_downloaded,
        timeline: local_timeline.to_string(),
    })
}

/// Import phase 1a: make sure the on-disk ref marker file for the timeline
/// exists (with no head yet) so it shows up in tools that scan refs/heads.
fn ensure_timeline_ref(repo: &Repo, local_timeline: &str) -> Result<(), SyncError> {
    if repo.get_timeline_head(local_timeline)?.is_none() {
        let ref_path = timeline_ref_path(&repo.ivaldi_dir, local_timeline)
            .map_err(|e| SyncError::Other(e.to_string()))?;
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&ref_path, b"")?;
    }
    Ok(())
}

/// Import phase 1b: identify commits not already in the hash mapping
/// (by list index) and the unique tree SHAs they reference.
fn collect_unimported_commits(
    commits: &[CommitInfo],
    hash_mapping: &HashMapping,
) -> (Vec<usize>, Vec<String>) {
    let unskipped: Vec<usize> = commits
        .iter()
        .enumerate()
        .filter(|(_, c)| hash_mapping.get_blake3(&c.sha).is_none())
        .map(|(i, _)| i)
        .collect();

    let unique_tree_shas: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        unskipped
            .iter()
            .map(|&i| commits[i].commit.tree.sha.clone())
            .filter(|sha| seen.insert(sha.clone()))
            .collect()
    };

    (unskipped, unique_tree_shas)
}

/// Import phase 2: pre-fetch all unique trees in parallel. Failed fetches are
/// logged and fall back to a live fetch (or blob skip) during the commit loop.
fn prefetch_trees(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    unique_tree_shas: &[String],
) -> HashMap<String, TreeResponse> {
    if unique_tree_shas.is_empty() {
        return HashMap::new();
    }

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
}

/// Import phase 3a: collect all unique blobs from pre-fetched trees that we
/// don't already have, as `(path, sha1, commit_sha)` download requests.
fn collect_blobs_to_download(
    commits: &[CommitInfo],
    unskipped: &[usize],
    prefetched_trees: &HashMap<String, TreeResponse>,
    hash_mapping: &HashMapping,
) -> Vec<(String, String, String)> {
    let mut blobs_to_download: Vec<(String, String, String)> = Vec::new();
    let mut seen_blob_shas: std::collections::HashSet<String> = std::collections::HashSet::new();

    for &idx in unskipped {
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

    blobs_to_download
}

/// Import phase 3b: download the requested blobs in parallel, then store them
/// in the CAS and record their SHA1 → BLAKE3 mapping (serial — the CAS is not
/// `Sync`). Failed downloads are logged and skipped. Returns the number of
/// blobs stored.
fn download_and_store_blobs(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    blobs_to_download: &[(String, String, String)],
    store: &FsStore<'_>,
    hash_mapping: &mut HashMapping,
) -> Result<usize, SyncError> {
    if blobs_to_download.is_empty() {
        return Ok(0);
    }

    let pb_blobs = crate::progress::file_bar(blobs_to_download.len() as u64, "Downloading blobs");
    let blob_results: Vec<BlobDownloadResult> = std::thread::scope(|s| {
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
                            results.push(Err(format!("failed to download {}: {}", path, e)));
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

    // Store every successful blob first (so a retry after a partial failure
    // does not re-download them), then fail loudly if anything is missing.
    // Committing a tree with silently absent files would be data loss.
    let mut blobs_downloaded = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for result in blob_results {
        match result {
            Ok((_, sha1, content)) => {
                let (b3_hash, _) = store.put_blob(&content)?;
                hash_mapping.insert(&sha1, b3_hash);
                blobs_downloaded += 1;
            }
            Err(msg) => failures.push(msg),
        }
    }
    if !failures.is_empty() {
        return Err(SyncError::Other(format!(
            "{} of {} blob downloads failed; refusing to import incomplete trees. \
             First failure: {}. Retry the import — completed blobs are cached.",
            failures.len(),
            blobs_to_download.len(),
            failures[0]
        )));
    }
    Ok(blobs_downloaded)
}

/// Linear scan for the leaf whose BLAKE3 hash equals `b3`.
fn find_leaf_idx_by_hash(repo: &Repo, b3: B3Hash) -> Option<u64> {
    (0..repo.commit_count())
        .find(|&idx| matches!(repo.get_leaf(idx), Ok(Some(leaf)) if leaf.hash() == b3))
}

/// Import phase 4 helper: resolve a commit's Git tree to an Ivaldi Merkle
/// tree using only the hash mapping — pure lookups, NO blob content reads.
/// Falls back to a live tree fetch when the pre-fetch failed.
fn ivaldi_tree_for_commit(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    store: &FsStore<'_>,
    tree_sha: &str,
    prefetched_trees: &HashMap<String, TreeResponse>,
    hash_mapping: &HashMapping,
) -> Result<B3Hash, SyncError> {
    // Look up pre-fetched tree, fall back to live fetch
    let tree = match prefetched_trees.get(tree_sha) {
        Some(t) => t.clone(),
        None => client.get_tree(owner, repo_name, tree_sha)?,
    };

    // Build hash map from tree entries — pure HashMap lookups, zero disk I/O.
    // A blob with no local mapping means its download was never scheduled or
    // failed; committing a tree without it would silently drop the file.
    let mut hash_file_map: BTreeMap<String, B3Hash> = BTreeMap::new();
    for entry in &tree.tree {
        if entry.entry_type != "blob" {
            continue;
        }
        match hash_mapping.get_blake3(&entry.sha) {
            Some(b3) => {
                hash_file_map.insert(entry.path.clone(), b3);
            }
            None => {
                return Err(SyncError::Other(format!(
                    "tree {} references blob {} ({}) that was not downloaded; \
                     refusing to import an incomplete tree. Retry the import.",
                    tree_sha, entry.sha, entry.path
                )));
            }
        }
    }

    // Build Merkle tree from hashes only — NO blob content reads
    Ok(store.build_tree_from_hash_map(&hash_file_map)?)
}

/// Import phase 4 helper: build an Ivaldi leaf mirroring a GitHub commit
/// (author, timestamp, parent indices resolved through `sha_to_idx`).
///
/// Parent resolution is strict: a parent that appears in the fetched commit
/// listing (`listed_shas`) but has no resolved index is a hard error — the
/// alternative is silently importing this commit as a fake root, severing
/// its ancestry. A parent *outside* the listing means the listing was
/// depth-truncated (shallow import); that commit legitimately becomes the
/// shallow root, and out-of-window merge parents are dropped with a warning.
fn build_import_leaf(
    commit: &CommitInfo,
    tree_hash: B3Hash,
    local_timeline: &str,
    sha_to_idx: &HashMap<String, u64>,
    listed_shas: &std::collections::HashSet<&str>,
) -> Result<Leaf, SyncError> {
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

    // Resolve parent indices.
    let resolve = |parent_sha: &str| -> Result<Option<u64>, SyncError> {
        if let Some(&idx) = sha_to_idx.get(parent_sha) {
            return Ok(Some(idx));
        }
        if listed_shas.contains(parent_sha) {
            return Err(SyncError::Other(format!(
                "commit {} lists parent {} which was in the fetched history but resolved to \
                 no local seal — refusing to sever ancestry; retry the sync",
                commit.sha, parent_sha
            )));
        }
        // Beyond the depth-truncated window: shallow-import boundary.
        Ok(None)
    };

    let prev_idx = match commit.parents.first() {
        Some(p) => match resolve(&p.sha)? {
            Some(idx) => idx,
            None => {
                crate::logging::warn(&format!(
                    "commit {} becomes the shallow-import root: its parent {} is beyond \
                     the fetched history window",
                    commit.sha, p.sha
                ));
                crate::leaf::NO_PARENT
            }
        },
        None => crate::leaf::NO_PARENT,
    };

    let mut merge_idxs: Vec<u64> = Vec::new();
    for p in commit.parents.iter().skip(1) {
        match resolve(&p.sha)? {
            Some(idx) => merge_idxs.push(idx),
            None => crate::logging::warn(&format!(
                "commit {}: dropping merge parent {} which is beyond the fetched history window",
                commit.sha, p.sha
            )),
        }
    }

    let mut leaf = Leaf::new(
        tree_hash,
        local_timeline,
        &author,
        time_unix,
        &commit.commit.message,
    );
    leaf.prev_idx = prev_idx;
    leaf.merge_idxs = merge_idxs;
    Ok(leaf)
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

    #[test]
    fn parent_outside_fetched_window_becomes_shallow_root() {
        let commit = commit_info("child", &["beyond-window"]);
        let sha_to_idx = HashMap::new();
        let listed: std::collections::HashSet<&str> = ["child"].into();
        let leaf = build_import_leaf(&commit, B3Hash::digest(b"t"), "main", &sha_to_idx, &listed)
            .expect("shallow boundary is legitimate");
        assert_eq!(leaf.prev_idx, crate::leaf::NO_PARENT);
    }

    #[test]
    fn listed_but_unresolvable_parent_is_a_hard_error() {
        // Parent IS in the fetched listing, so it should have been imported —
        // failing to resolve it means severed ancestry, never a silent root.
        let commit = commit_info("child", &["parent"]);
        let sha_to_idx = HashMap::new();
        let listed: std::collections::HashSet<&str> = ["child", "parent"].into();
        let err = build_import_leaf(&commit, B3Hash::digest(b"t"), "main", &sha_to_idx, &listed)
            .expect_err("must refuse to sever ancestry");
        assert!(err.to_string().contains("sever ancestry"), "{}", err);
    }

    #[test]
    fn resolved_parents_populate_prev_and_merge_indices() {
        let commit = commit_info("m", &["p1", "p2"]);
        let mut sha_to_idx = HashMap::new();
        sha_to_idx.insert("p1".to_string(), 4u64);
        sha_to_idx.insert("p2".to_string(), 7u64);
        let listed: std::collections::HashSet<&str> = ["m", "p1", "p2"].into();
        let leaf =
            build_import_leaf(&commit, B3Hash::digest(b"t"), "main", &sha_to_idx, &listed).unwrap();
        assert_eq!(leaf.prev_idx, 4);
        assert_eq!(leaf.merge_idxs, vec![7]);
    }

    #[test]
    fn iso8601_parsing() {
        assert_eq!(
            parse_iso8601_to_unix("2024-01-15T10:30:00Z"),
            parse_iso8601_to_unix("2024-01-15T05:30:00-05:00"),
        );
        assert!(parse_iso8601_to_unix("garbage").is_none());
    }
}
