//! Butterfly timelines for Ivaldi VCS.
//!
//! Butterflies are experimental sandbox timelines that branch from a parent.
//! They enable safe experimentation without polluting the parent timeline's history.
//!
//! Features:
//! - Branch from a parent timeline at a specific divergence point
//! - Sync changes bidirectionally (up to parent, down from parent)
//! - Nested butterflies (butterfly → butterfly → butterfly)
//! - Cascade delete of nested butterflies
//! - Orphan detection when parent is deleted
//!
//! Storage: in-memory metadata (file-based `.ivaldi/butterflies/` deferred to persistence layer)

use std::collections::BTreeMap;

use crate::hash::B3Hash;

/// A butterfly timeline record.
#[derive(Debug, Clone)]
pub struct Butterfly {
    /// Name of this butterfly timeline.
    pub name: String,
    /// Name of the parent timeline.
    pub parent_name: String,
    /// Hash at which the butterfly diverged from the parent.
    pub divergence_hash: B3Hash,
    /// Unix timestamp when the butterfly was created.
    pub created_at: i64,
    /// Whether the parent has been deleted (orphaned).
    pub is_orphaned: bool,
    /// Original parent name (preserved after orphaning).
    pub original_parent: Option<String>,
}

/// Metadata about a timeline's butterfly status.
#[derive(Debug, Clone)]
pub struct ButterflyMetadata {
    /// Timeline name.
    pub timeline: String,
    /// Whether this timeline is a butterfly.
    pub is_butterfly: bool,
    /// Butterfly info (if is_butterfly).
    pub butterfly: Option<Butterfly>,
    /// Names of child butterflies.
    pub children: Vec<String>,
}

/// Manages butterfly timeline metadata.
pub struct ButterflyManager {
    /// Butterfly records: name → Butterfly.
    butterflies: BTreeMap<String, Butterfly>,
    /// Parent → children mapping.
    children: BTreeMap<String, Vec<String>>,
}

impl ButterflyManager {
    pub fn new() -> Self {
        Self {
            butterflies: BTreeMap::new(),
            children: BTreeMap::new(),
        }
    }

    /// Create a new butterfly from a parent timeline.
    pub fn create(
        &mut self,
        name: &str,
        parent_name: &str,
        divergence_hash: B3Hash,
        created_at: i64,
    ) -> Result<(), ButterflyError> {
        if self.butterflies.contains_key(name) {
            return Err(ButterflyError::AlreadyExists(name.to_string()));
        }

        let bf = Butterfly {
            name: name.to_string(),
            parent_name: parent_name.to_string(),
            divergence_hash,
            created_at,
            is_orphaned: false,
            original_parent: None,
        };

        self.butterflies.insert(name.to_string(), bf);
        self.children
            .entry(parent_name.to_string())
            .or_default()
            .push(name.to_string());

        Ok(())
    }

    /// Check if a timeline is a butterfly.
    pub fn is_butterfly(&self, name: &str) -> bool {
        self.butterflies.contains_key(name)
    }

    /// Get butterfly info.
    pub fn get(&self, name: &str) -> Option<&Butterfly> {
        self.butterflies.get(name)
    }

    /// Get the parent name of a butterfly.
    pub fn get_parent(&self, name: &str) -> Option<&str> {
        self.butterflies.get(name).map(|bf| bf.parent_name.as_str())
    }

    /// Get children of a timeline (butterflies that branch from it).
    pub fn get_children(&self, name: &str) -> Vec<String> {
        self.children
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the divergence hash for a butterfly.
    pub fn get_divergence(&self, name: &str) -> Option<B3Hash> {
        self.butterflies.get(name).map(|bf| bf.divergence_hash)
    }

    /// Update the divergence point after a sync operation.
    pub fn update_divergence(
        &mut self,
        name: &str,
        new_divergence: B3Hash,
    ) -> Result<(), ButterflyError> {
        let bf = self
            .butterflies
            .get_mut(name)
            .ok_or_else(|| ButterflyError::NotFound(name.to_string()))?;
        bf.divergence_hash = new_divergence;
        Ok(())
    }

    /// Delete a butterfly. If cascade is true, recursively delete children.
    /// If cascade is false, children become orphaned.
    pub fn delete(&mut self, name: &str, cascade: bool) -> Result<Vec<String>, ButterflyError> {
        let bf = self
            .butterflies
            .get(name)
            .ok_or_else(|| ButterflyError::NotFound(name.to_string()))?
            .clone();

        let children = self.get_children(name);
        let mut deleted = Vec::new();

        if cascade {
            // Recursively delete children first
            for child in &children {
                let child_deleted = self.delete(child, true)?;
                deleted.extend(child_deleted);
            }
        } else {
            // Orphan the children
            for child in &children {
                if let Some(child_bf) = self.butterflies.get_mut(child) {
                    child_bf.is_orphaned = true;
                    child_bf.original_parent = Some(bf.parent_name.clone());
                }
            }
        }

        // Remove from parent's children list
        if !bf.is_orphaned {
            if let Some(siblings) = self.children.get_mut(&bf.parent_name) {
                siblings.retain(|c| c != name);
                if siblings.is_empty() {
                    self.children.remove(&bf.parent_name);
                }
            }
        }

        // Remove children mapping for this butterfly
        self.children.remove(name);

        // Remove the butterfly itself
        self.butterflies.remove(name);
        deleted.push(name.to_string());

        Ok(deleted)
    }

    /// Get full metadata for a timeline.
    pub fn get_metadata(&self, timeline: &str) -> ButterflyMetadata {
        let butterfly = self.butterflies.get(timeline).cloned();
        let children = self.get_children(timeline);

        ButterflyMetadata {
            timeline: timeline.to_string(),
            is_butterfly: butterfly.is_some(),
            butterfly,
            children,
        }
    }

    /// List all butterflies.
    pub fn list_all(&self) -> Vec<&Butterfly> {
        self.butterflies.values().collect()
    }

    /// List orphaned butterflies.
    pub fn list_orphaned(&self) -> Vec<&Butterfly> {
        self.butterflies
            .values()
            .filter(|bf| bf.is_orphaned)
            .collect()
    }

    /// Get the full butterfly tree starting from a root timeline.
    pub fn get_tree(&self, root: &str) -> Vec<(String, usize)> {
        let mut result = Vec::new();
        self.collect_tree(root, 0, &mut result);
        result
    }

    fn collect_tree(&self, name: &str, depth: usize, result: &mut Vec<(String, usize)>) {
        result.push((name.to_string(), depth));
        for child in self.get_children(name) {
            self.collect_tree(&child, depth + 1, result);
        }
    }
}

impl Default for ButterflyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ButterflyError {
    #[error("butterfly already exists: {0}")]
    AlreadyExists(String),
    #[error("butterfly not found: {0}")]
    NotFound(String),
    #[error("cannot sync orphaned butterfly: {0}")]
    Orphaned(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_butterfly() {
        let mut mgr = ButterflyManager::new();
        let hash = B3Hash::digest(b"diverge point");

        mgr.create("experiment", "main", hash, 1700000000).unwrap();

        assert!(mgr.is_butterfly("experiment"));
        assert!(!mgr.is_butterfly("main"));

        let bf = mgr.get("experiment").unwrap();
        assert_eq!(bf.name, "experiment");
        assert_eq!(bf.parent_name, "main");
        assert_eq!(bf.divergence_hash, hash);
        assert!(!bf.is_orphaned);
    }

    #[test]
    fn create_duplicate_fails() {
        let mut mgr = ButterflyManager::new();
        let hash = B3Hash::digest(b"point");

        mgr.create("exp", "main", hash, 0).unwrap();
        let result = mgr.create("exp", "main", hash, 0);
        assert!(matches!(result, Err(ButterflyError::AlreadyExists(_))));
    }

    #[test]
    fn get_parent() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.get_parent("exp"), Some("main"));
        assert_eq!(mgr.get_parent("main"), None);
    }

    #[test]
    fn children_tracking() {
        let mut mgr = ButterflyManager::new();
        mgr.create("feat-a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-b", "main", B3Hash::ZERO, 0).unwrap();

        let children = mgr.get_children("main");
        assert_eq!(children.len(), 2);
        assert!(children.contains(&"feat-a".to_string()));
        assert!(children.contains(&"feat-b".to_string()));
    }

    #[test]
    fn nested_butterflies() {
        let mut mgr = ButterflyManager::new();
        mgr.create("develop", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feature", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub-feature", "feature", B3Hash::ZERO, 0)
            .unwrap();

        assert_eq!(mgr.get_children("main"), vec!["develop"]);
        assert_eq!(mgr.get_children("develop"), vec!["feature"]);
        assert_eq!(mgr.get_children("feature"), vec!["sub-feature"]);
    }

    #[test]
    fn delete_without_cascade_orphans_children() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child-a", "parent-bf", B3Hash::ZERO, 0)
            .unwrap();
        mgr.create("child-b", "parent-bf", B3Hash::ZERO, 0)
            .unwrap();

        let deleted = mgr.delete("parent-bf", false).unwrap();
        assert_eq!(deleted, vec!["parent-bf"]);

        // Children still exist but are orphaned
        assert!(mgr.is_butterfly("child-a"));
        assert!(mgr.get("child-a").unwrap().is_orphaned);
        assert_eq!(
            mgr.get("child-a").unwrap().original_parent,
            Some("main".to_string())
        );

        assert!(mgr.get("child-b").unwrap().is_orphaned);
    }

    #[test]
    fn delete_with_cascade() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child-a", "parent-bf", B3Hash::ZERO, 0)
            .unwrap();
        mgr.create("grandchild", "child-a", B3Hash::ZERO, 0)
            .unwrap();

        let deleted = mgr.delete("parent-bf", true).unwrap();
        assert_eq!(deleted.len(), 3);
        assert!(deleted.contains(&"parent-bf".to_string()));
        assert!(deleted.contains(&"child-a".to_string()));
        assert!(deleted.contains(&"grandchild".to_string()));

        assert!(!mgr.is_butterfly("parent-bf"));
        assert!(!mgr.is_butterfly("child-a"));
        assert!(!mgr.is_butterfly("grandchild"));
    }

    #[test]
    fn delete_nonexistent_fails() {
        let mut mgr = ButterflyManager::new();
        let result = mgr.delete("nope", false);
        assert!(matches!(result, Err(ButterflyError::NotFound(_))));
    }

    #[test]
    fn update_divergence() {
        let mut mgr = ButterflyManager::new();
        let hash1 = B3Hash::digest(b"point1");
        let hash2 = B3Hash::digest(b"point2");

        mgr.create("exp", "main", hash1, 0).unwrap();
        assert_eq!(mgr.get_divergence("exp"), Some(hash1));

        mgr.update_divergence("exp", hash2).unwrap();
        assert_eq!(mgr.get_divergence("exp"), Some(hash2));
    }

    #[test]
    fn get_metadata_butterfly() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "exp", B3Hash::ZERO, 0).unwrap();

        let meta = mgr.get_metadata("exp");
        assert!(meta.is_butterfly);
        assert_eq!(meta.children, vec!["sub"]);
        assert_eq!(meta.butterfly.unwrap().parent_name, "main");
    }

    #[test]
    fn get_metadata_non_butterfly() {
        let mgr = ButterflyManager::new();
        let meta = mgr.get_metadata("main");
        assert!(!meta.is_butterfly);
        assert!(meta.butterfly.is_none());
        assert!(meta.children.is_empty());
    }

    #[test]
    fn list_all() {
        let mut mgr = ButterflyManager::new();
        mgr.create("a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("b", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("c", "a", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.list_all().len(), 3);
    }

    #[test]
    fn list_orphaned() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child", "parent-bf", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.list_orphaned().len(), 0);

        mgr.delete("parent-bf", false).unwrap();
        assert_eq!(mgr.list_orphaned().len(), 1);
        assert_eq!(mgr.list_orphaned()[0].name, "child");
    }

    #[test]
    fn get_tree() {
        let mut mgr = ButterflyManager::new();
        mgr.create("develop", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-a", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-b", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "feat-a", B3Hash::ZERO, 0).unwrap();

        let tree = mgr.get_tree("main");
        // main(0) → develop(1) → feat-a(2) → sub(3), feat-b(2)
        assert_eq!(tree[0], ("main".to_string(), 0));
        assert_eq!(tree[1], ("develop".to_string(), 1));
        // Children order depends on BTreeMap
        assert!(tree.len() == 5);
    }

    #[test]
    fn delete_removes_from_parent_children() {
        let mut mgr = ButterflyManager::new();
        mgr.create("a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("b", "main", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.get_children("main").len(), 2);

        mgr.delete("a", false).unwrap();
        let children = mgr.get_children("main");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], "b");
    }

    #[test]
    fn delete_cleans_up_children_mapping() {
        let mut mgr = ButterflyManager::new();
        mgr.create("bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child", "bf", B3Hash::ZERO, 0).unwrap();

        mgr.delete("bf", true).unwrap();
        assert!(mgr.get_children("bf").is_empty());
    }
}
