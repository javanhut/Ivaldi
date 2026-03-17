//! Shift (commit squash) engine for Ivaldi VCS.
//!
//! Combines multiple sequential commits into a single commit,
//! preserving the final tree state. Used for cleaning up WIP
//! commits before uploading.
//!
//! Modes:
//! - `--last N`: squash last N commits
//! - Range: squash from start to end commit
//! - Interactive: arrow-key selection (handled in CLI/TUI layer)

use crate::hash::B3Hash;
use crate::leaf::{Leaf, NO_PARENT};
use crate::timeline::{HistoryManager, TimelineError};

/// Information about a commit in a range to be squashed.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub index: u64,
    pub hash: B3Hash,
    pub message: String,
    pub author: String,
    pub time_unix: i64,
    pub tree_root: B3Hash,
}

/// Result of a shift (squash) operation.
#[derive(Debug)]
pub struct ShiftResult {
    /// Index of the new squashed commit.
    pub new_index: u64,
    /// Hash of the new squashed commit.
    pub new_hash: B3Hash,
    /// Number of commits that were squashed.
    pub squashed_count: usize,
    /// The combined commit message.
    pub message: String,
}

/// Errors from shift operations.
#[derive(Debug, thiserror::Error)]
pub enum ShiftError {
    #[error("need at least 2 commits to squash")]
    TooFewCommits,
    #[error("timeline has no commits")]
    NoCommits,
    #[error("not enough commits: have {have}, need {need}")]
    NotEnoughCommits { have: usize, need: usize },
    #[error("end commit is not a descendant of start commit")]
    NotDescendant,
    #[error("commit index {0} out of range")]
    IndexOutOfRange(u64),
    #[error("timeline error: {0}")]
    Timeline(#[from] TimelineError),
}

/// Get the last N commits on a timeline (newest first).
pub fn get_last_n(
    mgr: &HistoryManager,
    timeline: &str,
    n: usize,
) -> Result<Vec<CommitInfo>, ShiftError> {
    let head = mgr
        .get_timeline_head(timeline)
        .ok_or(ShiftError::NoCommits)?;

    let mut commits = Vec::new();
    let mut current = Some(head);

    while let Some(idx) = current {
        if commits.len() >= n {
            break;
        }
        let leaf = mgr
            .mmr
            .get_leaf(idx)
            .ok_or(ShiftError::IndexOutOfRange(idx))?;

        commits.push(CommitInfo {
            index: idx,
            hash: leaf.hash(),
            message: leaf.message.clone(),
            author: leaf.author.clone(),
            time_unix: leaf.time_unix,
            tree_root: leaf.tree_root,
        });

        current = if leaf.has_parent() {
            Some(leaf.prev_idx)
        } else {
            None
        };
    }

    if commits.len() < n {
        return Err(ShiftError::NotEnoughCommits {
            have: commits.len(),
            need: n,
        });
    }

    Ok(commits)
}

/// Get commits in a range from start_idx to end_idx (inclusive).
/// Returns in chronological order (oldest first).
pub fn get_range(
    mgr: &HistoryManager,
    start_idx: u64,
    end_idx: u64,
) -> Result<Vec<CommitInfo>, ShiftError> {
    // Walk backwards from end to start
    let mut commits = Vec::new();
    let mut current = Some(end_idx);

    loop {
        let idx = current.ok_or(ShiftError::NotDescendant)?;
        let leaf = mgr
            .mmr
            .get_leaf(idx)
            .ok_or(ShiftError::IndexOutOfRange(idx))?;

        commits.push(CommitInfo {
            index: idx,
            hash: leaf.hash(),
            message: leaf.message.clone(),
            author: leaf.author.clone(),
            time_unix: leaf.time_unix,
            tree_root: leaf.tree_root,
        });

        if idx == start_idx {
            break;
        }

        current = if leaf.has_parent() {
            Some(leaf.prev_idx)
        } else {
            None
        };
    }

    // Reverse to chronological order
    commits.reverse();
    Ok(commits)
}

/// Generate a combined commit message from a list of commits.
pub fn combined_message(commits: &[CommitInfo]) -> String {
    if commits.is_empty() {
        return "Empty squash".to_string();
    }
    if commits.len() == 1 {
        return commits[0].message.clone();
    }

    let messages: Vec<&str> = commits
        .iter()
        .map(|c| c.message.lines().next().unwrap_or(""))
        .collect();

    format!(
        "Squashed {} commits:\n\n{}",
        commits.len(),
        messages.join("\n")
    )
}

/// Perform the squash: create a new commit with the final tree state
/// and the parent of the oldest commit in the range.
///
/// Returns the new commit index and updates the timeline head.
pub fn squash(
    mgr: &mut HistoryManager,
    timeline: &str,
    commits: &[CommitInfo],
    message: &str,
    author: &str,
) -> Result<ShiftResult, ShiftError> {
    if commits.len() < 2 {
        return Err(ShiftError::TooFewCommits);
    }

    let oldest = &commits[0];
    let newest = &commits[commits.len() - 1];

    // The squashed commit uses the newest tree (final state)
    // and the oldest commit's parent as its parent
    let parent_idx = {
        let oldest_leaf = mgr
            .mmr
            .get_leaf(oldest.index)
            .ok_or(ShiftError::IndexOutOfRange(oldest.index))?;
        if oldest_leaf.has_parent() {
            oldest_leaf.prev_idx
        } else {
            NO_PARENT
        }
    };

    // Create the squashed leaf
    let mut squashed_leaf = Leaf::new(
        newest.tree_root,
        timeline,
        author,
        newest.time_unix,
        message,
    );
    squashed_leaf.prev_idx = parent_idx;
    squashed_leaf.timeline_id = timeline.to_string();

    // Append to MMR (bypassing the normal commit flow since we set prev_idx manually)
    let (new_idx, _root) = mgr.mmr.append_leaf(squashed_leaf.clone());
    mgr.timelines.set_head(timeline, new_idx);

    let new_hash = squashed_leaf.hash();

    Ok(ShiftResult {
        new_index: new_idx,
        new_hash,
        squashed_count: commits.len(),
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_mgr(n: usize) -> HistoryManager {
        let mut mgr = HistoryManager::new();
        for i in 0..n {
            let leaf = Leaf::new(
                B3Hash::digest(format!("tree {}", i).as_bytes()),
                "",
                "Author <a@b.com>",
                1700000000 + i as i64 * 60,
                format!("Commit {}", i),
            );
            mgr.commit("main", leaf).unwrap();
        }
        mgr
    }

    #[test]
    fn get_last_n_basic() {
        let mgr = setup_mgr(5);
        let commits = get_last_n(&mgr, "main", 3).unwrap();

        assert_eq!(commits.len(), 3);
        // Newest first
        assert_eq!(commits[0].message, "Commit 4");
        assert_eq!(commits[1].message, "Commit 3");
        assert_eq!(commits[2].message, "Commit 2");
    }

    #[test]
    fn get_last_n_all() {
        let mgr = setup_mgr(3);
        let commits = get_last_n(&mgr, "main", 3).unwrap();
        assert_eq!(commits.len(), 3);
    }

    #[test]
    fn get_last_n_too_many() {
        let mgr = setup_mgr(2);
        let result = get_last_n(&mgr, "main", 5);
        assert!(matches!(
            result,
            Err(ShiftError::NotEnoughCommits { have: 2, need: 5 })
        ));
    }

    #[test]
    fn get_last_n_empty_timeline() {
        let mgr = HistoryManager::new();
        let result = get_last_n(&mgr, "main", 2);
        assert!(matches!(result, Err(ShiftError::NoCommits)));
    }

    #[test]
    fn get_range_basic() {
        let mgr = setup_mgr(5);
        // Range from commit 1 to commit 3 (inclusive)
        let commits = get_range(&mgr, 1, 3).unwrap();

        assert_eq!(commits.len(), 3);
        // Chronological order (oldest first)
        assert_eq!(commits[0].message, "Commit 1");
        assert_eq!(commits[1].message, "Commit 2");
        assert_eq!(commits[2].message, "Commit 3");
    }

    #[test]
    fn get_range_single_commit() {
        let mgr = setup_mgr(3);
        let commits = get_range(&mgr, 1, 1).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].message, "Commit 1");
    }

    #[test]
    fn get_range_not_descendant() {
        let mut mgr = setup_mgr(3);
        // Create a separate timeline
        mgr.create_timeline("feature", None).unwrap();
        let leaf = Leaf::new(
            B3Hash::digest(b"feature"),
            "",
            "Author",
            1700000000,
            "Feature",
        );
        mgr.commit("feature", leaf).unwrap();

        // Commit 3 (feature) is not a descendant of commit 0 via linear parent chain
        // Actually this depends on the structure — commit 3 has parent 2
        // Let's test a clear non-descendant case
        let result = get_range(&mgr, 1, 0); // 0 is NOT a descendant of 1
        assert!(result.is_err());
    }

    #[test]
    fn combined_message_empty() {
        assert_eq!(combined_message(&[]), "Empty squash");
    }

    #[test]
    fn combined_message_single() {
        let commits = vec![CommitInfo {
            index: 0,
            hash: B3Hash::ZERO,
            message: "Only commit".into(),
            author: "A".into(),
            time_unix: 0,
            tree_root: B3Hash::ZERO,
        }];
        assert_eq!(combined_message(&commits), "Only commit");
    }

    #[test]
    fn combined_message_multiple() {
        let commits = vec![
            CommitInfo {
                index: 0,
                hash: B3Hash::ZERO,
                message: "WIP: start".into(),
                author: "A".into(),
                time_unix: 0,
                tree_root: B3Hash::ZERO,
            },
            CommitInfo {
                index: 1,
                hash: B3Hash::ZERO,
                message: "WIP: progress".into(),
                author: "A".into(),
                time_unix: 0,
                tree_root: B3Hash::ZERO,
            },
            CommitInfo {
                index: 2,
                hash: B3Hash::ZERO,
                message: "WIP: done".into(),
                author: "A".into(),
                time_unix: 0,
                tree_root: B3Hash::ZERO,
            },
        ];
        let msg = combined_message(&commits);
        assert!(msg.starts_with("Squashed 3 commits:"));
        assert!(msg.contains("WIP: start"));
        assert!(msg.contains("WIP: progress"));
        assert!(msg.contains("WIP: done"));
    }

    #[test]
    fn squash_basic() {
        let mut mgr = setup_mgr(5);

        // Get last 3 commits (newest first), reverse for squash
        let mut commits = get_last_n(&mgr, "main", 3).unwrap();
        commits.reverse(); // Oldest first for squash

        let result = squash(
            &mut mgr,
            "main",
            &commits,
            "Squashed 3 commits",
            "Author <a@b.com>",
        )
        .unwrap();

        assert_eq!(result.squashed_count, 3);
        assert_eq!(result.message, "Squashed 3 commits");

        // Timeline head should point to the new squashed commit
        assert_eq!(mgr.get_timeline_head("main"), Some(result.new_index));

        // The squashed commit should have the tree of the newest commit
        let squashed = mgr.mmr.get_leaf(result.new_index).unwrap();
        assert_eq!(squashed.tree_root, commits[2].tree_root);

        // The parent should be the parent of the oldest commit in range
        // Commit 2 (oldest in range) has parent 1
        assert_eq!(squashed.prev_idx, 1);
    }

    #[test]
    fn squash_too_few() {
        let mut mgr = setup_mgr(3);
        let commits = get_last_n(&mgr, "main", 1).unwrap();

        let result = squash(&mut mgr, "main", &commits, "msg", "author");
        assert!(matches!(result, Err(ShiftError::TooFewCommits)));
    }

    #[test]
    fn squash_all_commits() {
        let mut mgr = setup_mgr(3);
        let mut commits = get_last_n(&mgr, "main", 3).unwrap();
        commits.reverse();

        let result = squash(
            &mut mgr,
            "main",
            &commits,
            "All squashed",
            "Author",
        )
        .unwrap();

        assert_eq!(result.squashed_count, 3);

        // Squashed commit's parent should be NO_PARENT (root commit squashed)
        let squashed = mgr.mmr.get_leaf(result.new_index).unwrap();
        assert_eq!(squashed.prev_idx, NO_PARENT);
    }

    #[test]
    fn squash_preserves_final_tree() {
        let mut mgr = setup_mgr(4);

        let mut commits = get_last_n(&mgr, "main", 2).unwrap();
        commits.reverse();

        let final_tree = commits[1].tree_root;

        let result = squash(&mut mgr, "main", &commits, "msg", "author").unwrap();

        let squashed = mgr.mmr.get_leaf(result.new_index).unwrap();
        assert_eq!(squashed.tree_root, final_tree);
    }
}
