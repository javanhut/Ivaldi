//! Timeline (branch) management for Ivaldi VCS.
//!
//! Timelines are lightweight pointers to leaf indices in the MMR.
//! The HistoryManager orchestrates commits, timeline heads, and LCA computation.

use std::collections::{HashMap, HashSet};

use crate::hash::B3Hash;
use crate::leaf::{Leaf, NO_PARENT};
use crate::mmr::Mmr;

/// Error type for timeline operations.
#[derive(Debug, thiserror::Error)]
pub enum TimelineError {
    #[error("timeline not found: {0}")]
    NotFound(String),
    #[error("timeline already exists: {0}")]
    AlreadyExists(String),
    #[error("cannot remove current timeline")]
    CannotRemoveCurrent,
    #[error("leaf index {0} out of range")]
    LeafOutOfRange(u64),
    #[error("no common ancestor found")]
    NoCommonAncestor,
    #[error("corrupt history: {0}")]
    CorruptHistory(String),
}

/// In-memory timeline store. `None` is an existing timeline with no commits.
pub struct TimelineStore {
    heads: HashMap<String, Option<u64>>,
}

impl TimelineStore {
    pub fn new() -> Self {
        Self {
            heads: HashMap::new(),
        }
    }

    pub fn get_head(&self, name: &str) -> Option<u64> {
        self.heads.get(name).copied().flatten()
    }

    pub fn set_head(&mut self, name: &str, idx: u64) {
        self.heads.insert(name.to_string(), Some(idx));
    }

    pub fn create(&mut self, name: &str, head: Option<u64>) {
        self.heads.insert(name.to_string(), head);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.heads.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.heads.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn exists(&self, name: &str) -> bool {
        self.heads.contains_key(name)
    }
}

impl Default for TimelineStore {
    fn default() -> Self {
        Self::new()
    }
}

/// High-level history manager combining MMR and timeline operations.
pub struct HistoryManager {
    pub mmr: Mmr,
    pub timelines: TimelineStore,
    current: String,
}

impl HistoryManager {
    /// Create a new history manager with "main" as the default timeline.
    pub fn new() -> Self {
        let mut timelines = TimelineStore::new();
        timelines.create("main", None);
        Self {
            mmr: Mmr::new(),
            timelines,
            current: "main".to_string(),
        }
    }

    /// Get the current active timeline name.
    pub fn current_timeline(&self) -> &str {
        &self.current
    }

    /// Set the current active timeline.
    pub fn set_current_timeline(&mut self, name: &str) -> Result<(), TimelineError> {
        if !self.timelines.exists(name) {
            return Err(TimelineError::NotFound(name.to_string()));
        }
        self.current = name.to_string();
        Ok(())
    }

    /// Create a new commit on a timeline.
    ///
    /// Automatically fills `prev_idx` from the current timeline head
    /// and sets `timeline_id`.
    pub fn commit(
        &mut self,
        timeline: &str,
        mut leaf: Leaf,
    ) -> Result<(u64, B3Hash), TimelineError> {
        if !self.timelines.exists(timeline) {
            return Err(TimelineError::NotFound(timeline.to_string()));
        }
        // Fill prev_idx from current timeline head
        leaf.prev_idx = self.timelines.get_head(timeline).unwrap_or(NO_PARENT);
        leaf.timeline_id = timeline.to_string();

        let (idx, root) = self.mmr.append_leaf(leaf);
        self.timelines.set_head(timeline, idx);

        Ok((idx, root))
    }

    /// Create a new timeline pointing at the same commit as `source` (or current).
    pub fn create_timeline(
        &mut self,
        name: &str,
        source: Option<&str>,
    ) -> Result<(), TimelineError> {
        if self.timelines.exists(name) {
            return Err(TimelineError::AlreadyExists(name.to_string()));
        }

        let source_name = source.unwrap_or(&self.current);
        if !self.timelines.exists(source_name) {
            return Err(TimelineError::NotFound(source_name.to_string()));
        }
        self.timelines
            .create(name, self.timelines.get_head(source_name));

        Ok(())
    }

    /// Remove a timeline. Cannot remove the current timeline.
    pub fn remove_timeline(&mut self, name: &str) -> Result<(), TimelineError> {
        if name == self.current {
            return Err(TimelineError::CannotRemoveCurrent);
        }
        if !self.timelines.remove(name) {
            return Err(TimelineError::NotFound(name.to_string()));
        }
        Ok(())
    }

    /// Switch to a different timeline.
    pub fn switch_timeline(&mut self, name: &str) -> Result<(), TimelineError> {
        if !self.timelines.exists(name) {
            return Err(TimelineError::NotFound(name.to_string()));
        }
        self.current = name.to_string();
        Ok(())
    }

    /// List all timeline names (sorted).
    pub fn list_timelines(&self) -> Vec<String> {
        self.timelines.list()
    }

    /// Get the head leaf index for a timeline.
    pub fn get_timeline_head(&self, name: &str) -> Option<u64> {
        self.timelines.get_head(name)
    }

    /// Compute the Lowest Common Ancestor of two leaf indices.
    ///
    /// Uses ancestor-set tracing (works across timelines).
    pub fn lca(&self, a_idx: u64, b_idx: u64) -> Result<u64, TimelineError> {
        if a_idx >= self.mmr.size() {
            return Err(TimelineError::LeafOutOfRange(a_idx));
        }
        if b_idx >= self.mmr.size() {
            return Err(TimelineError::LeafOutOfRange(b_idx));
        }
        if a_idx == b_idx {
            return Ok(a_idx);
        }

        // Build ancestor set for A, validating every link as we traverse it.
        let mut ancestors_a = HashSet::new();
        let mut current = a_idx;
        loop {
            if !ancestors_a.insert(current) {
                return Err(TimelineError::CorruptHistory(format!(
                    "cycle at leaf {current}"
                )));
            }
            let leaf = self.checked_leaf(current)?;
            if leaf.has_parent() {
                current = leaf.prev_idx;
            } else {
                break;
            }
        }

        // Walk back from B looking for common ancestor
        current = b_idx;
        let mut ancestors_b = HashSet::new();
        loop {
            if ancestors_a.contains(&current) {
                return Ok(current);
            }
            if !ancestors_b.insert(current) {
                return Err(TimelineError::CorruptHistory(format!(
                    "cycle at leaf {current}"
                )));
            }
            let leaf = self.checked_leaf(current)?;
            if leaf.has_parent() {
                current = leaf.prev_idx;
            } else {
                break;
            }
        }

        Err(TimelineError::NoCommonAncestor)
    }

    fn checked_leaf(&self, idx: u64) -> Result<&Leaf, TimelineError> {
        self.mmr.get_leaf(idx).ok_or_else(|| {
            TimelineError::CorruptHistory(format!("leaf {idx} references a missing parent"))
        })
    }
}

impl Default for HistoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_leaf(msg: &str) -> Leaf {
        Leaf::new(
            B3Hash::digest(msg.as_bytes()),
            "", // will be set by commit
            "Author <a@b.com>",
            1700000000,
            msg,
        )
    }

    #[test]
    fn new_manager_defaults() {
        let mgr = HistoryManager::new();
        assert_eq!(mgr.current_timeline(), "main");
        assert_eq!(mgr.mmr.size(), 0);
        assert_eq!(mgr.list_timelines(), vec!["main"]);
    }

    #[test]
    fn commit_creates_head() {
        let mut mgr = HistoryManager::new();
        let (idx, _) = mgr.commit("main", make_leaf("first")).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(mgr.get_timeline_head("main"), Some(0));

        // Verify leaf has correct timeline and prev_idx
        let leaf = mgr.mmr.get_leaf(0).unwrap();
        assert_eq!(leaf.timeline_id, "main");
        assert_eq!(leaf.prev_idx, NO_PARENT);
    }

    #[test]
    fn multiple_commits_chain() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.commit("main", make_leaf("second")).unwrap();
        mgr.commit("main", make_leaf("third")).unwrap();

        assert_eq!(mgr.get_timeline_head("main"), Some(2));

        // Check chain
        let leaf2 = mgr.mmr.get_leaf(2).unwrap();
        assert_eq!(leaf2.prev_idx, 1);
        let leaf1 = mgr.mmr.get_leaf(1).unwrap();
        assert_eq!(leaf1.prev_idx, 0);
        let leaf0 = mgr.mmr.get_leaf(0).unwrap();
        assert_eq!(leaf0.prev_idx, NO_PARENT);
    }

    #[test]
    fn create_timeline_from_current() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();

        mgr.create_timeline("feature", None).unwrap();
        assert_eq!(mgr.get_timeline_head("feature"), Some(0));
        assert!(mgr.timelines.exists("feature"));
    }

    #[test]
    fn create_timeline_from_source() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.commit("main", make_leaf("second")).unwrap();

        mgr.create_timeline("feature", Some("main")).unwrap();
        assert_eq!(mgr.get_timeline_head("feature"), Some(1));
    }

    #[test]
    fn create_and_switch_empty_timeline() {
        let mut mgr = HistoryManager::new();
        mgr.create_timeline("empty", Some("main")).unwrap();

        assert!(mgr.timelines.exists("empty"));
        assert_eq!(mgr.get_timeline_head("empty"), None);
        mgr.switch_timeline("empty").unwrap();
        mgr.commit("empty", make_leaf("first")).unwrap();
        assert_eq!(mgr.get_timeline_head("empty"), Some(0));
    }

    #[test]
    fn create_timeline_rejects_missing_source() {
        let mut mgr = HistoryManager::new();
        let result = mgr.create_timeline("feature", Some("missing"));
        assert!(matches!(result, Err(TimelineError::NotFound(_))));
        assert!(!mgr.timelines.exists("feature"));
    }

    #[test]
    fn commit_rejects_missing_timeline() {
        let mut mgr = HistoryManager::new();
        let result = mgr.commit("missing", make_leaf("first"));
        assert!(matches!(result, Err(TimelineError::NotFound(_))));
    }

    #[test]
    fn create_duplicate_timeline_fails() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.create_timeline("feature", None).unwrap();

        let result = mgr.create_timeline("feature", None);
        assert!(matches!(result, Err(TimelineError::AlreadyExists(_))));
    }

    #[test]
    fn remove_timeline() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.create_timeline("feature", None).unwrap();

        mgr.remove_timeline("feature").unwrap();
        assert!(!mgr.timelines.exists("feature"));
    }

    #[test]
    fn cannot_remove_current_timeline() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();

        let result = mgr.remove_timeline("main");
        assert!(matches!(result, Err(TimelineError::CannotRemoveCurrent)));
    }

    #[test]
    fn switch_timeline() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.create_timeline("feature", None).unwrap();

        mgr.switch_timeline("feature").unwrap();
        assert_eq!(mgr.current_timeline(), "feature");
    }

    #[test]
    fn switch_nonexistent_fails() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();

        let result = mgr.switch_timeline("nope");
        assert!(matches!(result, Err(TimelineError::NotFound(_))));
    }

    #[test]
    fn list_timelines_sorted() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        mgr.create_timeline("zeta", None).unwrap();
        mgr.create_timeline("alpha", None).unwrap();

        let list = mgr.list_timelines();
        assert_eq!(list, vec!["alpha", "main", "zeta"]);
    }

    #[test]
    fn divergent_timelines() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("base")).unwrap();

        mgr.create_timeline("feature", None).unwrap();

        // Commit on main
        mgr.commit("main", make_leaf("main work")).unwrap();

        // Commit on feature
        mgr.commit("feature", make_leaf("feature work")).unwrap();

        assert_eq!(mgr.get_timeline_head("main"), Some(1));
        assert_eq!(mgr.get_timeline_head("feature"), Some(2));

        // Feature's parent should be the base commit (0)
        let feature_leaf = mgr.mmr.get_leaf(2).unwrap();
        assert_eq!(feature_leaf.prev_idx, 0);
    }

    #[test]
    fn lca_same_index() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();
        assert_eq!(mgr.lca(0, 0).unwrap(), 0);
    }

    #[test]
    fn lca_same_timeline() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("A")).unwrap();
        mgr.commit("main", make_leaf("B")).unwrap();
        mgr.commit("main", make_leaf("C")).unwrap();

        // LCA of C (idx=2) and A (idx=0) should be A
        assert_eq!(mgr.lca(2, 0).unwrap(), 0);
        // LCA of C and B should be B
        assert_eq!(mgr.lca(2, 1).unwrap(), 1);
    }

    #[test]
    fn lca_divergent_timelines() {
        let mut mgr = HistoryManager::new();
        // Base commit
        mgr.commit("main", make_leaf("base")).unwrap(); // idx=0

        // Create feature from main
        mgr.create_timeline("feature", None).unwrap();

        // Diverge
        mgr.commit("main", make_leaf("main-1")).unwrap(); // idx=1
        mgr.commit("feature", make_leaf("feature-1")).unwrap(); // idx=2

        // LCA of main head (1) and feature head (2) should be base (0)
        assert_eq!(mgr.lca(1, 2).unwrap(), 0);
    }

    #[test]
    fn lca_deep_chains() {
        let mut mgr = HistoryManager::new();
        // Build a chain of 20 commits on main
        for i in 0..20 {
            mgr.commit("main", make_leaf(&format!("commit {}", i)))
                .unwrap();
        }

        // Fork at commit 10
        mgr.timelines.set_head("feature", 10);
        for i in 0..5 {
            mgr.commit("feature", make_leaf(&format!("feature {}", i)))
                .unwrap();
        }

        let main_head = mgr.get_timeline_head("main").unwrap();
        let feature_head = mgr.get_timeline_head("feature").unwrap();

        assert_eq!(mgr.lca(main_head, feature_head).unwrap(), 10);
    }

    #[test]
    fn lca_out_of_range() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("first")).unwrap();

        let result = mgr.lca(0, 99);
        assert!(matches!(result, Err(TimelineError::LeafOutOfRange(99))));
        assert!(matches!(
            mgr.lca(99, 99),
            Err(TimelineError::LeafOutOfRange(99))
        ));
    }

    #[test]
    fn lca_rejects_corrupt_parent_link() {
        let mut mgr = HistoryManager::new();
        mgr.commit("main", make_leaf("base")).unwrap();
        let mut corrupt = make_leaf("corrupt");
        corrupt.prev_idx = 99;
        mgr.mmr.append_leaf(corrupt);

        let result = mgr.lca(1, 0);
        assert!(matches!(result, Err(TimelineError::CorruptHistory(_))));
    }
}
