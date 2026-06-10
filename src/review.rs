//! Local code review system for Ivaldi VCS.
//!
//! Provides offline review workflows: create reviews, comment on code,
//! approve/reject, and merge — all working entirely without a remote.
//!
//! Reviews are stored as JSON files in `.ivaldi/reviews/`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config;
use crate::fsmerkle::{self, FsStore, NodeKind};
use crate::fuse::{FuseEngine, Strategy};
use crate::hash::B3Hash;
use crate::repo::{Repo, RepoError};

/// Current status of a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewStatus {
    Open,
    Approved,
    ChangesRequested,
    Merged,
    Closed,
}

impl std::fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewStatus::Open => write!(f, "open"),
            ReviewStatus::Approved => write!(f, "approved"),
            ReviewStatus::ChangesRequested => write!(f, "changes-requested"),
            ReviewStatus::Merged => write!(f, "merged"),
            ReviewStatus::Closed => write!(f, "closed"),
        }
    }
}

impl std::str::FromStr for ReviewStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "approved" => Ok(Self::Approved),
            "changes-requested" => Ok(Self::ChangesRequested),
            "merged" => Ok(Self::Merged),
            "closed" => Ok(Self::Closed),
            _ => Err(()),
        }
    }
}

impl ReviewStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            ReviewStatus::Open => "O",
            ReviewStatus::Approved => "+",
            ReviewStatus::ChangesRequested => "!",
            ReviewStatus::Merged => "M",
            ReviewStatus::Closed => "X",
        }
    }
}

/// A comment on a review, optionally attached to a file and line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: u64,
    pub path: String,
    pub line: Option<u64>,
    pub author: String,
    pub time_unix: i64,
    pub body: String,
    pub reply_to: Option<u64>,
}

/// A verdict (approve or request changes) on a review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub author: String,
    pub time_unix: i64,
    pub status: ReviewStatus,
    pub body: String,
}

/// A local code review linking a source timeline to a target timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub source_timeline: String,
    pub target_timeline: String,
    pub source_head_seal: String,
    pub target_head_seal: String,
    pub author: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: ReviewStatus,
    pub comments: Vec<ReviewComment>,
    pub verdicts: Vec<ReviewVerdict>,
    pub fuse_strategy: String,
    pub merge_seal: Option<String>,
}

/// Optional filter for listing reviews.
#[derive(Debug, Clone, Default)]
pub struct ReviewFilter {
    pub status: Option<ReviewStatus>,
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// Create a new review.
pub fn create_review(
    repo: &Repo,
    title: &str,
    description: &str,
    source: &str,
    target: &str,
    strategy: &str,
) -> Result<Review, RepoError> {
    // Validate timelines exist
    let source_head = repo
        .get_timeline_head(source)?
        .ok_or_else(|| RepoError::Other(format!("timeline '{}' has no commits", source)))?;
    let target_head = repo
        .get_timeline_head(target)?
        .ok_or_else(|| RepoError::Other(format!("timeline '{}' has no commits", target)))?;

    if source == target {
        return Err(RepoError::Other(
            "source and target timelines must differ".into(),
        ));
    }

    // Get seal names for head commits
    let source_leaf = repo
        .get_leaf(source_head)?
        .ok_or_else(|| RepoError::Other("corrupt source head".into()))?;
    let target_leaf = repo
        .get_leaf(target_head)?
        .ok_or_else(|| RepoError::Other("corrupt target head".into()))?;

    let source_seal = crate::seal::generate_seal_name(source_leaf.hash());
    let target_seal = crate::seal::generate_seal_name(target_leaf.hash());

    let cfg = config::load_config(&repo.ivaldi_dir);
    let author = cfg.author().unwrap_or_else(|| "unknown <unknown>".into());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let id = repo.next_review_id()?;

    let review = Review {
        id,
        title: title.to_string(),
        description: description.to_string(),
        source_timeline: source.to_string(),
        target_timeline: target.to_string(),
        source_head_seal: source_seal,
        target_head_seal: target_seal,
        author,
        created_at: now,
        updated_at: now,
        status: ReviewStatus::Open,
        comments: Vec::new(),
        verdicts: Vec::new(),
        fuse_strategy: strategy.to_string(),
        merge_seal: None,
    };

    repo.save_review(&review)?;
    Ok(review)
}

/// List reviews, optionally filtered by status.
pub fn list_reviews(repo: &Repo, filter: &ReviewFilter) -> Result<Vec<Review>, RepoError> {
    let mut reviews = repo.list_reviews()?;
    if let Some(status) = filter.status {
        reviews.retain(|r| r.status == status);
    }
    reviews.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    Ok(reviews)
}

/// Add a comment to a review.
pub fn add_comment(
    repo: &Repo,
    review_id: u64,
    path: &str,
    line: Option<u64>,
    body: &str,
    reply_to: Option<u64>,
) -> Result<Review, RepoError> {
    let mut review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    if review.status == ReviewStatus::Merged || review.status == ReviewStatus::Closed {
        return Err(RepoError::Other(format!(
            "cannot comment on {} review",
            review.status
        )));
    }

    let cfg = config::load_config(&repo.ivaldi_dir);
    let author = cfg.author().unwrap_or_else(|| "unknown <unknown>".into());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let comment_id = review.comments.len() as u64 + 1;
    review.comments.push(ReviewComment {
        id: comment_id,
        path: path.to_string(),
        line,
        author,
        time_unix: now,
        body: body.to_string(),
        reply_to,
    });
    review.updated_at = now;

    repo.save_review(&review)?;
    Ok(review)
}

/// Submit a verdict (approve or request changes).
pub fn submit_verdict(
    repo: &Repo,
    review_id: u64,
    status: ReviewStatus,
    body: &str,
) -> Result<Review, RepoError> {
    if status != ReviewStatus::Approved && status != ReviewStatus::ChangesRequested {
        return Err(RepoError::Other(
            "verdict must be Approved or ChangesRequested".into(),
        ));
    }

    let mut review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    if review.status == ReviewStatus::Merged || review.status == ReviewStatus::Closed {
        return Err(RepoError::Other(format!(
            "cannot submit verdict on {} review",
            review.status
        )));
    }

    let cfg = config::load_config(&repo.ivaldi_dir);
    let author = cfg.author().unwrap_or_else(|| "unknown <unknown>".into());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    review.verdicts.push(ReviewVerdict {
        author,
        time_unix: now,
        status,
        body: body.to_string(),
    });
    review.status = status;
    review.updated_at = now;

    repo.save_review(&review)?;
    Ok(review)
}

/// Merge a review. Requires Approved status.
pub fn merge_review(repo: &mut Repo, review_id: u64) -> Result<Review, RepoError> {
    let mut review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    if review.status != ReviewStatus::Approved {
        return Err(RepoError::Other(format!(
            "review must be approved before merge (current: {})",
            review.status
        )));
    }

    let strategy = review
        .fuse_strategy
        .parse::<Strategy>()
        .unwrap_or(Strategy::Auto);

    // Get current head trees for both timelines
    let source_head = repo
        .get_timeline_head(&review.source_timeline)?
        .ok_or_else(|| {
            RepoError::Other(format!(
                "source timeline '{}' has no commits",
                review.source_timeline
            ))
        })?;
    let target_head = repo
        .get_timeline_head(&review.target_timeline)?
        .ok_or_else(|| {
            RepoError::Other(format!(
                "target timeline '{}' has no commits",
                review.target_timeline
            ))
        })?;

    let source_leaf = repo
        .get_leaf(source_head)?
        .ok_or_else(|| RepoError::Other("corrupt source head".into()))?;
    let target_leaf = repo
        .get_leaf(target_head)?
        .ok_or_else(|| RepoError::Other("corrupt target head".into()))?;

    // Build file maps
    let store = FsStore::new(&repo.cas);
    let base_files = BTreeMap::new();
    let mut ours_files = BTreeMap::new();
    let mut theirs_files = BTreeMap::new();
    collect_blob_hashes(&store, target_leaf.tree_root, "", &mut ours_files)?;
    collect_blob_hashes(&store, source_leaf.tree_root, "", &mut theirs_files)?;

    let result = FuseEngine::fuse(&store, &base_files, &ours_files, &theirs_files, strategy);

    if !result.success {
        let conflict_paths: Vec<String> = result.conflicts.iter().map(|c| c.path.clone()).collect();
        return Err(RepoError::Other(format!(
            "merge has {} conflict(s): {}",
            conflict_paths.len(),
            conflict_paths.join(", ")
        )));
    }

    // Build merged tree and commit
    let merged_tree = store
        .build_tree_from_hash_map(&result.merged_files)
        .map_err(|e| RepoError::Other(e.to_string()))?;

    let cfg = config::load_config(&repo.ivaldi_dir);
    let author = cfg.author().unwrap_or_else(|| "unknown <unknown>".into());
    let message = format!(
        "Review #{}: {} (fuse {} into {})",
        review.id, review.title, review.source_timeline, review.target_timeline
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut fuse_leaf =
        crate::leaf::Leaf::new(merged_tree, &review.target_timeline, &author, now, &message);
    fuse_leaf.prev_idx = target_head;
    fuse_leaf.merge_idxs = vec![source_head];

    let commit_result = repo.commit_raw(fuse_leaf, &review.target_timeline)?;

    review.status = ReviewStatus::Merged;
    review.merge_seal = Some(commit_result.seal_name.clone());
    review.updated_at = now;
    repo.save_review(&review)?;

    Ok(review)
}

/// Close a review without merging.
pub fn close_review(repo: &Repo, review_id: u64) -> Result<Review, RepoError> {
    let mut review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    if review.status == ReviewStatus::Merged {
        return Err(RepoError::Other("cannot close a merged review".into()));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    review.status = ReviewStatus::Closed;
    review.updated_at = now;
    repo.save_review(&review)?;
    Ok(review)
}

/// Reopen a closed review.
pub fn reopen_review(repo: &Repo, review_id: u64) -> Result<Review, RepoError> {
    let mut review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    if review.status != ReviewStatus::Closed {
        return Err(RepoError::Other(format!(
            "can only reopen closed reviews (current: {})",
            review.status
        )));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    review.status = ReviewStatus::Open;
    review.updated_at = now;
    repo.save_review(&review)?;
    Ok(review)
}

/// Get the diff between source and target timelines for a review.
pub fn review_diff(repo: &Repo, review_id: u64) -> Result<Vec<fsmerkle::Change>, RepoError> {
    let review = repo
        .load_review(review_id)?
        .ok_or_else(|| RepoError::Other(format!("review #{} not found", review_id)))?;

    let source_head = repo
        .get_timeline_head(&review.source_timeline)?
        .ok_or_else(|| {
            RepoError::Other(format!(
                "source timeline '{}' has no commits",
                review.source_timeline
            ))
        })?;
    let target_head = repo
        .get_timeline_head(&review.target_timeline)?
        .ok_or_else(|| {
            RepoError::Other(format!(
                "target timeline '{}' has no commits",
                review.target_timeline
            ))
        })?;

    let source_leaf = repo
        .get_leaf(source_head)?
        .ok_or_else(|| RepoError::Other("corrupt source head".into()))?;
    let target_leaf = repo
        .get_leaf(target_head)?
        .ok_or_else(|| RepoError::Other("corrupt target head".into()))?;

    let store = FsStore::new(&repo.cas);
    fsmerkle::diff_trees(target_leaf.tree_root, source_leaf.tree_root, &store)
        .map_err(|e| RepoError::Other(e.to_string()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_blob_hashes(
    store: &FsStore<'_>,
    tree_hash: B3Hash,
    prefix: &str,
    files: &mut BTreeMap<String, B3Hash>,
) -> Result<(), RepoError> {
    let tree = store
        .load_tree(tree_hash)
        .map_err(|e| RepoError::Other(e.to_string()))?;
    for entry in &tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            NodeKind::Blob => {
                files.insert(path, entry.hash);
            }
            NodeKind::Tree => {
                collect_blob_hashes(store, entry.hash, &path, files)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::forge;

    fn setup_repo_with_branches() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        let mut cfg = Config::new();
        cfg.set("user.name", "Test User");
        cfg.set("user.email", "test@ivaldi.dev");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        // Ensure reviews dir exists
        std::fs::create_dir_all(dir.path().join(".ivaldi/reviews")).unwrap();

        let mut repo = Repo::open(dir.path()).unwrap();

        // Create initial commit on main
        let tree = B3Hash::digest(b"initial tree");
        repo.commit(tree, "Test User <test@ivaldi.dev>", "Initial commit")
            .unwrap();

        // Create feature branch with a different commit
        repo.create_timeline("feature", None).unwrap();
        repo.switch_timeline("feature").unwrap();
        let tree2 = B3Hash::digest(b"feature tree");
        repo.commit(tree2, "Test User <test@ivaldi.dev>", "Feature work")
            .unwrap();

        repo.switch_timeline("main").unwrap();
        (dir, repo)
    }

    #[test]
    fn review_status_display() {
        assert_eq!(format!("{}", ReviewStatus::Open), "open");
        assert_eq!(format!("{}", ReviewStatus::Approved), "approved");
        assert_eq!(
            format!("{}", ReviewStatus::ChangesRequested),
            "changes-requested"
        );
        assert_eq!(format!("{}", ReviewStatus::Merged), "merged");
        assert_eq!(format!("{}", ReviewStatus::Closed), "closed");
    }

    #[test]
    fn review_status_from_str() {
        assert_eq!(
            "open".parse::<ReviewStatus>().ok(),
            Some(ReviewStatus::Open)
        );
        assert_eq!(
            "approved".parse::<ReviewStatus>().ok(),
            Some(ReviewStatus::Approved)
        );
        assert_eq!(
            "changes-requested".parse::<ReviewStatus>().ok(),
            Some(ReviewStatus::ChangesRequested)
        );
        assert_eq!(
            "merged".parse::<ReviewStatus>().ok(),
            Some(ReviewStatus::Merged)
        );
        assert_eq!(
            "closed".parse::<ReviewStatus>().ok(),
            Some(ReviewStatus::Closed)
        );
        assert_eq!("invalid".parse::<ReviewStatus>().ok(), None);
    }

    #[test]
    fn serialization_roundtrip() {
        let review = Review {
            id: 1,
            title: "Test review".into(),
            description: "Description".into(),
            source_timeline: "feature".into(),
            target_timeline: "main".into(),
            source_head_seal: "crimson-forge".into(),
            target_head_seal: "azure-peak".into(),
            author: "Alice <a@b.com>".into(),
            created_at: 1700000000,
            updated_at: 1700000000,
            status: ReviewStatus::Open,
            comments: vec![ReviewComment {
                id: 1,
                path: "src/main.rs".into(),
                line: Some(42),
                author: "Bob <b@c.com>".into(),
                time_unix: 1700000100,
                body: "Nice work!".into(),
                reply_to: None,
            }],
            verdicts: vec![ReviewVerdict {
                author: "Bob <b@c.com>".into(),
                time_unix: 1700000200,
                status: ReviewStatus::Approved,
                body: "LGTM".into(),
            }],
            fuse_strategy: "auto".into(),
            merge_seal: None,
        };

        let json = serde_json::to_string_pretty(&review).unwrap();
        let deserialized: Review = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, 1);
        assert_eq!(deserialized.title, "Test review");
        assert_eq!(deserialized.comments.len(), 1);
        assert_eq!(deserialized.comments[0].line, Some(42));
        assert_eq!(deserialized.verdicts.len(), 1);
        assert_eq!(deserialized.verdicts[0].status, ReviewStatus::Approved);
    }

    #[test]
    fn save_and_load_review() {
        let (_dir, repo) = setup_repo_with_branches();

        let review = Review {
            id: 1,
            title: "Test".into(),
            description: "".into(),
            source_timeline: "feature".into(),
            target_timeline: "main".into(),
            source_head_seal: "seal1".into(),
            target_head_seal: "seal2".into(),
            author: "A".into(),
            created_at: 1000,
            updated_at: 1000,
            status: ReviewStatus::Open,
            comments: Vec::new(),
            verdicts: Vec::new(),
            fuse_strategy: "auto".into(),
            merge_seal: None,
        };

        repo.save_review(&review).unwrap();
        let loaded = repo.load_review(1).unwrap().unwrap();
        assert_eq!(loaded.id, 1);
        assert_eq!(loaded.title, "Test");
    }

    #[test]
    fn list_reviews_all() {
        let (_dir, repo) = setup_repo_with_branches();

        for i in 1..=3 {
            let review = Review {
                id: i,
                title: format!("Review {}", i),
                description: "".into(),
                source_timeline: "feature".into(),
                target_timeline: "main".into(),
                source_head_seal: "".into(),
                target_head_seal: "".into(),
                author: "A".into(),
                created_at: 1000 + i as i64,
                updated_at: 1000 + i as i64,
                status: if i == 2 {
                    ReviewStatus::Closed
                } else {
                    ReviewStatus::Open
                },
                comments: Vec::new(),
                verdicts: Vec::new(),
                fuse_strategy: "auto".into(),
                merge_seal: None,
            };
            repo.save_review(&review).unwrap();
        }

        // Update next_id counter
        repo.store.set_meta("review.next_id", "4").unwrap();

        let all = list_reviews(&repo, &ReviewFilter::default()).unwrap();
        assert_eq!(all.len(), 3);

        let open_only = list_reviews(
            &repo,
            &ReviewFilter {
                status: Some(ReviewStatus::Open),
            },
        )
        .unwrap();
        assert_eq!(open_only.len(), 2);

        let closed_only = list_reviews(
            &repo,
            &ReviewFilter {
                status: Some(ReviewStatus::Closed),
            },
        )
        .unwrap();
        assert_eq!(closed_only.len(), 1);
    }

    #[test]
    fn next_review_id_increments() {
        let (_dir, repo) = setup_repo_with_branches();
        let id1 = repo.next_review_id().unwrap();
        let id2 = repo.next_review_id().unwrap();
        let id3 = repo.next_review_id().unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn create_review_validates_timelines() {
        let (_dir, repo) = setup_repo_with_branches();

        // Same source and target
        let err = create_review(&repo, "Title", "", "main", "main", "auto");
        assert!(err.is_err());

        // Nonexistent source
        let err = create_review(&repo, "Title", "", "nonexistent", "main", "auto");
        assert!(err.is_err());
    }

    #[test]
    fn create_review_success() {
        let (_dir, repo) = setup_repo_with_branches();

        let review = create_review(
            &repo,
            "Add login",
            "Implements login",
            "feature",
            "main",
            "auto",
        )
        .unwrap();

        assert_eq!(review.id, 1);
        assert_eq!(review.title, "Add login");
        assert_eq!(review.status, ReviewStatus::Open);
        assert_eq!(review.source_timeline, "feature");
        assert_eq!(review.target_timeline, "main");
        assert!(!review.source_head_seal.is_empty());
        assert!(!review.target_head_seal.is_empty());
    }

    #[test]
    fn add_comment_to_review() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();

        let updated =
            add_comment(&repo, review.id, "src/main.rs", Some(10), "Fix this", None).unwrap();
        assert_eq!(updated.comments.len(), 1);
        assert_eq!(updated.comments[0].path, "src/main.rs");
        assert_eq!(updated.comments[0].line, Some(10));
        assert_eq!(updated.comments[0].body, "Fix this");

        // Add reply
        let updated2 =
            add_comment(&repo, review.id, "src/main.rs", Some(10), "Done", Some(1)).unwrap();
        assert_eq!(updated2.comments.len(), 2);
        assert_eq!(updated2.comments[1].reply_to, Some(1));
    }

    #[test]
    fn approval_flow() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();
        assert_eq!(review.status, ReviewStatus::Open);

        // Request changes
        let r2 = submit_verdict(
            &repo,
            review.id,
            ReviewStatus::ChangesRequested,
            "Needs work",
        )
        .unwrap();
        assert_eq!(r2.status, ReviewStatus::ChangesRequested);

        // Approve
        let r3 = submit_verdict(&repo, review.id, ReviewStatus::Approved, "LGTM").unwrap();
        assert_eq!(r3.status, ReviewStatus::Approved);
        assert_eq!(r3.verdicts.len(), 2);
    }

    #[test]
    fn merge_requires_approval() {
        let (_dir, mut repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();

        // Merging unapproved review should fail
        let err = merge_review(&mut repo, review.id);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("approved"));
    }

    #[test]
    fn close_and_reopen() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();

        let closed = close_review(&repo, review.id).unwrap();
        assert_eq!(closed.status, ReviewStatus::Closed);

        let reopened = reopen_review(&repo, review.id).unwrap();
        assert_eq!(reopened.status, ReviewStatus::Open);
    }

    #[test]
    fn reopen_non_closed_fails() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();

        // Can't reopen an already-open review
        let err = reopen_review(&repo, review.id);
        assert!(err.is_err());
    }

    #[test]
    fn cannot_comment_on_closed_review() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();
        close_review(&repo, review.id).unwrap();

        let err = add_comment(&repo, review.id, "file.rs", None, "comment", None);
        assert!(err.is_err());
    }

    #[test]
    fn cannot_verdict_on_closed_review() {
        let (_dir, repo) = setup_repo_with_branches();
        let review = create_review(&repo, "Test", "", "feature", "main", "auto").unwrap();
        close_review(&repo, review.id).unwrap();

        let err = submit_verdict(&repo, review.id, ReviewStatus::Approved, "LGTM");
        assert!(err.is_err());
    }
}
