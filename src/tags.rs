//! Tag management for Ivaldi VCS.
//!
//! Supports lightweight and annotated tags pointing to specific seals.
//! Storage: `.ivaldi/refs/tags/<name>` and persistent in redb.

use std::collections::BTreeMap;

use crate::hash::B3Hash;

/// A tag pointing to a specific commit.
#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub target_hash: B3Hash,
    pub target_index: u64,
    pub kind: TagKind,
    /// Annotation message (only for annotated tags).
    pub message: Option<String>,
    /// Tagger identity (only for annotated tags).
    pub tagger: Option<String>,
    /// Unix timestamp (only for annotated tags).
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    Lightweight,
    Annotated,
}

/// Manages tags for a repository.
pub struct TagManager {
    tags: BTreeMap<String, Tag>,
}

impl TagManager {
    pub fn new() -> Self {
        Self { tags: BTreeMap::new() }
    }

    /// Create a lightweight tag.
    pub fn create_lightweight(&mut self, name: &str, target_hash: B3Hash, target_index: u64) -> Result<(), TagError> {
        if self.tags.contains_key(name) {
            return Err(TagError::AlreadyExists(name.to_string()));
        }
        self.tags.insert(name.to_string(), Tag {
            name: name.to_string(),
            target_hash,
            target_index,
            kind: TagKind::Lightweight,
            message: None,
            tagger: None,
            timestamp: None,
        });
        Ok(())
    }

    /// Create an annotated tag with message and tagger info.
    pub fn create_annotated(
        &mut self, name: &str, target_hash: B3Hash, target_index: u64,
        message: &str, tagger: &str, timestamp: i64,
    ) -> Result<(), TagError> {
        if self.tags.contains_key(name) {
            return Err(TagError::AlreadyExists(name.to_string()));
        }
        self.tags.insert(name.to_string(), Tag {
            name: name.to_string(),
            target_hash,
            target_index,
            kind: TagKind::Annotated,
            message: Some(message.to_string()),
            tagger: Some(tagger.to_string()),
            timestamp: Some(timestamp),
        });
        Ok(())
    }

    /// Delete a tag.
    pub fn delete(&mut self, name: &str) -> Result<(), TagError> {
        self.tags.remove(name).ok_or(TagError::NotFound(name.to_string()))?;
        Ok(())
    }

    /// Get a tag by name.
    pub fn get(&self, name: &str) -> Option<&Tag> { self.tags.get(name) }

    /// List all tags sorted by name.
    pub fn list(&self) -> Vec<&Tag> { self.tags.values().collect() }

    /// Find tags pointing to a specific commit.
    pub fn tags_for_commit(&self, hash: B3Hash) -> Vec<&Tag> {
        self.tags.values().filter(|t| t.target_hash == hash).collect()
    }
}

impl Default for TagManager {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("tag already exists: {0}")]
    AlreadyExists(String),
    #[error("tag not found: {0}")]
    NotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightweight_tag() {
        let mut mgr = TagManager::new();
        let hash = B3Hash::digest(b"commit");
        mgr.create_lightweight("v1.0", hash, 0).unwrap();
        let tag = mgr.get("v1.0").unwrap();
        assert_eq!(tag.kind, TagKind::Lightweight);
        assert!(tag.message.is_none());
    }

    #[test]
    fn annotated_tag() {
        let mut mgr = TagManager::new();
        let hash = B3Hash::digest(b"commit");
        mgr.create_annotated("v2.0", hash, 1, "Release 2.0", "Alice", 1700000000).unwrap();
        let tag = mgr.get("v2.0").unwrap();
        assert_eq!(tag.kind, TagKind::Annotated);
        assert_eq!(tag.message.as_deref(), Some("Release 2.0"));
    }

    #[test]
    fn duplicate_fails() {
        let mut mgr = TagManager::new();
        mgr.create_lightweight("v1", B3Hash::ZERO, 0).unwrap();
        assert!(mgr.create_lightweight("v1", B3Hash::ZERO, 0).is_err());
    }

    #[test]
    fn delete_tag() {
        let mut mgr = TagManager::new();
        mgr.create_lightweight("v1", B3Hash::ZERO, 0).unwrap();
        mgr.delete("v1").unwrap();
        assert!(mgr.get("v1").is_none());
    }

    #[test]
    fn list_sorted() {
        let mut mgr = TagManager::new();
        mgr.create_lightweight("v3", B3Hash::ZERO, 0).unwrap();
        mgr.create_lightweight("v1", B3Hash::ZERO, 0).unwrap();
        mgr.create_lightweight("v2", B3Hash::ZERO, 0).unwrap();
        let names: Vec<&str> = mgr.list().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["v1", "v2", "v3"]);
    }

    #[test]
    fn tags_for_commit() {
        let mut mgr = TagManager::new();
        let h = B3Hash::digest(b"c1");
        mgr.create_lightweight("a", h, 0).unwrap();
        mgr.create_lightweight("b", h, 0).unwrap();
        mgr.create_lightweight("c", B3Hash::ZERO, 1).unwrap();
        assert_eq!(mgr.tags_for_commit(h).len(), 2);
    }
}
