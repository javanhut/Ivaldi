//! Leaf (commit/seal record) for Ivaldi VCS.
//!
//! Each commit is represented as a Leaf containing the filesystem tree root,
//! timeline lineage, author info, and metadata. Leaves are appended to the MMR.
//!
//! Canonical encoding format (version 1):
//! ```text
//! uvarint(1)                    // version
//! 32 bytes TreeRoot             // filesystem tree hash
//! uvarint(len(TimelineID))      // timeline ID length
//! bytes(TimelineID)             // timeline ID string
//! uvarint(PrevIdx)              // previous index (NO_PARENT if none)
//! uvarint(len(MergeIdxs))       // number of merge parents
//! repeat: uvarint(index)        // merge parent indices
//! uvarint(len(Author))          // author string length
//! bytes(Author)                 // author string
//! varint(TimeUnix)              // timestamp (signed)
//! uvarint(len(Message))         // message length
//! bytes(Message)                // message string
//! uvarint(len(Meta))            // metadata map size
//! repeat (sorted by key):       // key-value pairs
//!   uvarint(len(key)) + bytes(key)
//!   uvarint(len(value)) + bytes(value)
//! ```

use std::collections::BTreeMap;

use crate::filechunk::{read_uvarint, read_varint, write_uvarint, write_varint};
use crate::hash::B3Hash;

/// Sentinel value indicating no parent commit.
pub const NO_PARENT: u64 = u64::MAX;

/// A commit record in the history.
#[derive(Debug, Clone)]
pub struct Leaf {
    /// BLAKE3 root hash of the filesystem tree (from fsmerkle).
    pub tree_root: B3Hash,
    /// Timeline/branch name this commit belongs to.
    pub timeline_id: String,
    /// Index of the previous commit on this timeline; `NO_PARENT` if first.
    pub prev_idx: u64,
    /// Additional parent indices for merge commits.
    pub merge_idxs: Vec<u64>,
    /// Commit author (e.g., "Jane Doe <jane@example.com>").
    pub author: String,
    /// Unix timestamp of the commit.
    pub time_unix: i64,
    /// Commit message.
    pub message: String,
    /// Additional metadata (e.g., "autoshelved" → "1").
    pub meta: BTreeMap<String, String>,
}

impl Leaf {
    /// Create a new leaf with required fields.
    pub fn new(
        tree_root: B3Hash,
        timeline_id: impl Into<String>,
        author: impl Into<String>,
        time_unix: i64,
        message: impl Into<String>,
    ) -> Self {
        Self {
            tree_root,
            timeline_id: timeline_id.into(),
            prev_idx: NO_PARENT,
            merge_idxs: Vec::new(),
            author: author.into(),
            time_unix,
            message: message.into(),
            meta: BTreeMap::new(),
        }
    }

    /// Returns true if this leaf has a parent on its timeline.
    pub fn has_parent(&self) -> bool {
        self.prev_idx != NO_PARENT
    }

    /// Returns true if this leaf is a merge commit.
    pub fn is_merge(&self) -> bool {
        !self.merge_idxs.is_empty()
    }

    /// Returns all parent indices (previous + merge parents).
    pub fn all_parents(&self) -> Vec<u64> {
        let mut parents = Vec::with_capacity(1 + self.merge_idxs.len());
        if self.has_parent() {
            parents.push(self.prev_idx);
        }
        parents.extend_from_slice(&self.merge_idxs);
        parents
    }

    /// Returns true if this leaf is marked as autoshelved.
    pub fn is_autoshelved(&self) -> bool {
        self.meta.get("autoshelved").is_some_and(|v| v == "1")
    }

    /// Mark this leaf as autoshelved.
    pub fn set_autoshelved(&mut self, autoshelved: bool) {
        if autoshelved {
            self.meta.insert("autoshelved".into(), "1".into());
        } else {
            self.meta.remove("autoshelved");
        }
    }

    /// Encode to canonical bytes (version 1).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);

        // Version
        write_uvarint(&mut buf, 1);

        // TreeRoot (32 bytes)
        buf.extend_from_slice(self.tree_root.as_bytes());

        // TimelineID
        write_uvarint(&mut buf, self.timeline_id.len() as u64);
        buf.extend_from_slice(self.timeline_id.as_bytes());

        // PrevIdx
        write_uvarint(&mut buf, self.prev_idx);

        // MergeIdxs
        write_uvarint(&mut buf, self.merge_idxs.len() as u64);
        for &idx in &self.merge_idxs {
            write_uvarint(&mut buf, idx);
        }

        // Author
        write_uvarint(&mut buf, self.author.len() as u64);
        buf.extend_from_slice(self.author.as_bytes());

        // TimeUnix (signed varint)
        write_varint(&mut buf, self.time_unix);

        // Message
        write_uvarint(&mut buf, self.message.len() as u64);
        buf.extend_from_slice(self.message.as_bytes());

        // Meta (BTreeMap is already sorted by key)
        write_uvarint(&mut buf, self.meta.len() as u64);
        for (key, value) in &self.meta {
            write_uvarint(&mut buf, key.len() as u64);
            buf.extend_from_slice(key.as_bytes());
            write_uvarint(&mut buf, value.len() as u64);
            buf.extend_from_slice(value.as_bytes());
        }

        buf
    }

    /// Compute the BLAKE3 hash of the canonical representation.
    pub fn hash(&self) -> B3Hash {
        B3Hash::digest(&self.canonical_bytes())
    }
}

/// Parse canonical bytes back into a Leaf.
pub fn parse_leaf(data: &[u8]) -> Result<Leaf, LeafError> {
    let mut offset = 0;

    // Version
    let (version, n) = read_uvarint(&data[offset..]);
    offset += n;
    if version != 1 {
        return Err(LeafError::UnsupportedVersion(version));
    }

    // TreeRoot
    if offset + 32 > data.len() {
        return Err(LeafError::InvalidData("truncated tree root".into()));
    }
    let tree_root = B3Hash::from_slice(&data[offset..offset + 32]).unwrap();
    offset += 32;

    // TimelineID
    let (tl_len, n) = read_uvarint(&data[offset..]);
    offset += n;
    let tl_end = offset + tl_len as usize;
    if tl_end > data.len() {
        return Err(LeafError::InvalidData("truncated timeline ID".into()));
    }
    let timeline_id = String::from_utf8(data[offset..tl_end].to_vec())
        .map_err(|_| LeafError::InvalidData("invalid UTF-8 in timeline ID".into()))?;
    offset = tl_end;

    // PrevIdx
    let (prev_idx, n) = read_uvarint(&data[offset..]);
    offset += n;

    // MergeIdxs
    let (merge_count, n) = read_uvarint(&data[offset..]);
    offset += n;
    let mut merge_idxs = Vec::with_capacity(merge_count as usize);
    for _ in 0..merge_count {
        let (idx, n) = read_uvarint(&data[offset..]);
        offset += n;
        merge_idxs.push(idx);
    }

    // Author
    let (author_len, n) = read_uvarint(&data[offset..]);
    offset += n;
    let author_end = offset + author_len as usize;
    if author_end > data.len() {
        return Err(LeafError::InvalidData("truncated author".into()));
    }
    let author = String::from_utf8(data[offset..author_end].to_vec())
        .map_err(|_| LeafError::InvalidData("invalid UTF-8 in author".into()))?;
    offset = author_end;

    // TimeUnix (signed varint)
    let (time_unix, n) = read_varint(&data[offset..]);
    offset += n;

    // Message
    let (msg_len, n) = read_uvarint(&data[offset..]);
    offset += n;
    let msg_end = offset + msg_len as usize;
    if msg_end > data.len() {
        return Err(LeafError::InvalidData("truncated message".into()));
    }
    let message = String::from_utf8(data[offset..msg_end].to_vec())
        .map_err(|_| LeafError::InvalidData("invalid UTF-8 in message".into()))?;
    offset = msg_end;

    // Meta
    let (meta_count, n) = read_uvarint(&data[offset..]);
    offset += n;
    let mut meta = BTreeMap::new();
    for _ in 0..meta_count {
        let (key_len, n) = read_uvarint(&data[offset..]);
        offset += n;
        let key_end = offset + key_len as usize;
        let key = String::from_utf8(data[offset..key_end].to_vec())
            .map_err(|_| LeafError::InvalidData("invalid UTF-8 in meta key".into()))?;
        offset = key_end;

        let (val_len, n) = read_uvarint(&data[offset..]);
        offset += n;
        let val_end = offset + val_len as usize;
        let value = String::from_utf8(data[offset..val_end].to_vec())
            .map_err(|_| LeafError::InvalidData("invalid UTF-8 in meta value".into()))?;
        offset = val_end;

        meta.insert(key, value);
    }

    if offset != data.len() {
        return Err(LeafError::InvalidData("extra data after leaf".into()));
    }

    Ok(Leaf {
        tree_root,
        timeline_id,
        prev_idx,
        merge_idxs,
        author,
        time_unix,
        message,
        meta,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum LeafError {
    #[error("unsupported leaf version: {0}")]
    UnsupportedVersion(u64),
    #[error("invalid data: {0}")]
    InvalidData(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_leaf() -> Leaf {
        Leaf::new(
            B3Hash::digest(b"tree root"),
            "main",
            "Alice <alice@example.com>",
            1700000000,
            "Initial commit",
        )
    }

    #[test]
    fn canonical_roundtrip() {
        let leaf = sample_leaf();
        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();

        assert_eq!(parsed.tree_root, leaf.tree_root);
        assert_eq!(parsed.timeline_id, "main");
        assert_eq!(parsed.prev_idx, NO_PARENT);
        assert!(parsed.merge_idxs.is_empty());
        assert_eq!(parsed.author, "Alice <alice@example.com>");
        assert_eq!(parsed.time_unix, 1700000000);
        assert_eq!(parsed.message, "Initial commit");
        assert!(parsed.meta.is_empty());
    }

    #[test]
    fn hash_deterministic() {
        let leaf = sample_leaf();
        assert_eq!(leaf.hash(), leaf.hash());
    }

    #[test]
    fn hash_changes_with_content() {
        let leaf1 = sample_leaf();
        let mut leaf2 = sample_leaf();
        leaf2.message = "Different message".into();
        assert_ne!(leaf1.hash(), leaf2.hash());
    }

    #[test]
    fn with_parent() {
        let mut leaf = sample_leaf();
        assert!(!leaf.has_parent());
        assert_eq!(leaf.all_parents(), Vec::<u64>::new());

        leaf.prev_idx = 42;
        assert!(leaf.has_parent());
        assert_eq!(leaf.all_parents(), vec![42]);

        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.prev_idx, 42);
    }

    #[test]
    fn with_merge_parents() {
        let mut leaf = sample_leaf();
        leaf.prev_idx = 5;
        leaf.merge_idxs = vec![10, 20];
        assert!(leaf.is_merge());
        assert_eq!(leaf.all_parents(), vec![5, 10, 20]);

        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.merge_idxs, vec![10, 20]);
    }

    #[test]
    fn with_metadata() {
        let mut leaf = sample_leaf();
        leaf.meta.insert("key1".into(), "value1".into());
        leaf.meta.insert("key2".into(), "value2".into());

        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.meta.len(), 2);
        assert_eq!(parsed.meta["key1"], "value1");
        assert_eq!(parsed.meta["key2"], "value2");
    }

    #[test]
    fn autoshelved_flag() {
        let mut leaf = sample_leaf();
        assert!(!leaf.is_autoshelved());

        leaf.set_autoshelved(true);
        assert!(leaf.is_autoshelved());
        assert_eq!(leaf.meta["autoshelved"], "1");

        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert!(parsed.is_autoshelved());

        let mut leaf2 = parsed;
        leaf2.set_autoshelved(false);
        assert!(!leaf2.is_autoshelved());
        assert!(!leaf2.meta.contains_key("autoshelved"));
    }

    #[test]
    fn meta_sorted_determinism() {
        let mut leaf1 = sample_leaf();
        leaf1.meta.insert("z".into(), "last".into());
        leaf1.meta.insert("a".into(), "first".into());

        let mut leaf2 = sample_leaf();
        leaf2.meta.insert("a".into(), "first".into());
        leaf2.meta.insert("z".into(), "last".into());

        // Same hash regardless of insertion order (BTreeMap sorts)
        assert_eq!(leaf1.hash(), leaf2.hash());
    }

    #[test]
    fn negative_timestamp() {
        let mut leaf = sample_leaf();
        leaf.time_unix = -1000;

        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.time_unix, -1000);
    }

    #[test]
    fn empty_strings() {
        let leaf = Leaf::new(B3Hash::ZERO, "", "", 0, "");
        let bytes = leaf.canonical_bytes();
        let parsed = parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.timeline_id, "");
        assert_eq!(parsed.author, "");
        assert_eq!(parsed.message, "");
    }

    #[test]
    fn parse_bad_version() {
        let mut buf = Vec::new();
        write_uvarint(&mut buf, 99); // bad version
        assert!(matches!(
            parse_leaf(&buf).unwrap_err(),
            LeafError::UnsupportedVersion(99)
        ));
    }
}
