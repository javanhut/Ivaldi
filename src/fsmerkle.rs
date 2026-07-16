//! Filesystem Merkle DAG for Ivaldi VCS.
//!
//! Represents filesystem trees as immutable Merkle structures where:
//! - `BlobNode` represents file content
//! - `TreeNode` represents directories with sorted entries
//! - All content is identified by BLAKE3-256 hashes
//! - Structural sharing enables efficient storage and comparison
//!
//! Canonical Encodings:
//! - Blob: `"blob <size>\x00" || content` → `BLAKE3(canonical)`
//! - Tree: `uvarint(count) || entries...` → `BLAKE3(canonical)`

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use crate::cas::{Cas, CasError};
use crate::filechunk::write_uvarint;
use crate::hash::B3Hash;
use crate::reader::ByteReader;

/// The kind of a filesystem node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeKind {
    Blob = 1,
    Tree = 2,
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeKind::Blob => write!(f, "blob"),
            NodeKind::Tree => write!(f, "tree"),
        }
    }
}

/// File mode constants.
pub const MODE_FILE: u32 = 0o100644;
pub const MODE_EXEC: u32 = 0o100755;
pub const MODE_SYMLINK: u32 = 0o120000;
pub const MODE_DIR: u32 = 0o040000;

/// A single entry in a directory tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub mode: u32,
    pub kind: NodeKind,
    pub hash: B3Hash,
}

/// A blob node representing file content.
#[derive(Debug, Clone)]
pub struct BlobNode {
    pub size: usize,
}

impl BlobNode {
    /// Create canonical bytes: `"blob <size>\x00"`
    pub fn header_bytes(size: usize) -> Vec<u8> {
        format!("blob {}\x00", size).into_bytes()
    }

    /// Create full canonical bytes: header + content.
    pub fn canonical_bytes(content: &[u8]) -> Vec<u8> {
        let header = Self::header_bytes(content.len());
        let mut buf = Vec::with_capacity(header.len() + content.len());
        buf.extend_from_slice(&header);
        buf.extend_from_slice(content);
        buf
    }

    /// Compute the BLAKE3 hash of blob canonical bytes.
    pub fn hash_content(content: &[u8]) -> B3Hash {
        let canonical = Self::canonical_bytes(content);
        B3Hash::digest(&canonical)
    }
}

/// A tree node representing a directory with sorted entries.
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub entries: Vec<Entry>,
}

impl TreeNode {
    /// Create a new tree node, sorting entries by name.
    pub fn new(mut entries: Vec<Entry>) -> Self {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Self { entries }
    }

    /// Validate the tree node.
    pub fn validate(&self) -> Result<(), FsMerkleError> {
        let mut seen = HashMap::new();
        for (i, entry) in self.entries.iter().enumerate() {
            validate_name(&entry.name)?;
            validate_mode(entry.mode, entry.kind)?;

            if seen.insert(&entry.name, i).is_some() {
                return Err(FsMerkleError::DuplicateName(entry.name.clone()));
            }

            if i > 0 && entry.name <= self.entries[i - 1].name {
                return Err(FsMerkleError::UnsortedEntries {
                    prev: self.entries[i - 1].name.clone(),
                    current: entry.name.clone(),
                });
            }
        }
        Ok(())
    }

    /// Encode to canonical bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, FsMerkleError> {
        self.validate()?;
        let mut buf = Vec::new();

        write_uvarint(&mut buf, self.entries.len() as u64);

        for entry in &self.entries {
            write_uvarint(&mut buf, entry.mode as u64);
            write_uvarint(&mut buf, entry.name.len() as u64);
            buf.extend_from_slice(entry.name.as_bytes());
            buf.push(entry.kind as u8);
            buf.extend_from_slice(entry.hash.as_bytes());
        }

        Ok(buf)
    }

    /// Compute the BLAKE3 hash of canonical bytes.
    pub fn hash(&self) -> Result<B3Hash, FsMerkleError> {
        let canonical = self.canonical_bytes()?;
        Ok(B3Hash::digest(&canonical))
    }

    /// Find an entry by name.
    pub fn find_entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Parse canonical tree bytes back into a TreeNode.
///
/// Bounds-checked throughout: a truncated buffer or a bogus entry count returns
/// a typed error instead of panicking or pre-allocating from the count.
pub fn parse_tree(data: &[u8]) -> Result<TreeNode, FsMerkleError> {
    let mut r = ByteReader::new(data);

    let count = r.uvarint()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let mode = r.uvarint()? as u32;
        let name = r.string("name")?;
        let kind = match r.u8()? {
            1 => NodeKind::Blob,
            2 => NodeKind::Tree,
            k => return Err(FsMerkleError::InvalidData(format!("unknown kind: {}", k))),
        };
        let hash = B3Hash::from_bytes(r.array::<32>()?);
        entries.push(Entry {
            name,
            mode,
            kind,
            hash,
        });
    }

    r.finish()?; // reject trailing data after the tree

    // Decode-side validation, not just encode-side: tree-node bytes arrive
    // verbatim from remote peers (the CAS only checks the hash), and every
    // materialize path joins entry names into filesystem paths. Rejecting
    // "../", "/", ".", duplicates, and bad modes here protects every
    // consumer of decoded trees at once — no bypass via a hostile peer.
    let node = TreeNode { entries };
    node.validate()?;
    Ok(node)
}

/// Parse canonical blob bytes back into content.
pub fn parse_blob(data: &[u8]) -> Result<(BlobNode, Vec<u8>), FsMerkleError> {
    let null_idx = data
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| FsMerkleError::InvalidData("no null terminator in blob header".into()))?;

    let header = std::str::from_utf8(&data[..null_idx])
        .map_err(|_| FsMerkleError::InvalidData("invalid blob header".into()))?;

    let size: usize = header
        .strip_prefix("blob ")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| FsMerkleError::InvalidData(format!("invalid blob header: {:?}", header)))?;

    let content = &data[null_idx + 1..];
    if content.len() != size {
        return Err(FsMerkleError::InvalidData(format!(
            "blob size mismatch: header says {}, got {}",
            size,
            content.len()
        )));
    }

    Ok((BlobNode { size }, content.to_vec()))
}

// ---------------------------------------------------------------------------
// Store: combines Builder + Loader
// ---------------------------------------------------------------------------

/// Filesystem Merkle store backed by a CAS.
pub struct FsStore<'a> {
    cas: &'a dyn Cas,
}

impl<'a> FsStore<'a> {
    pub fn new(cas: &'a dyn Cas) -> Self {
        Self { cas }
    }

    /// Store a blob, returning its hash and size.
    pub fn put_blob(&self, content: &[u8]) -> Result<(B3Hash, usize), FsMerkleError> {
        let canonical = BlobNode::canonical_bytes(content);
        let hash = B3Hash::digest(&canonical);
        self.cas.put(hash, &canonical)?;
        Ok((hash, content.len()))
    }

    /// Store a tree from entries, returning its hash.
    pub fn put_tree(&self, entries: Vec<Entry>) -> Result<B3Hash, FsMerkleError> {
        let tree = TreeNode::new(entries);
        let canonical = tree.canonical_bytes()?;
        let hash = B3Hash::digest(&canonical);
        self.cas.put(hash, &canonical)?;
        Ok(hash)
    }

    /// Load a tree by hash.
    pub fn load_tree(&self, hash: B3Hash) -> Result<TreeNode, FsMerkleError> {
        let data = self.cas.get(hash)?;
        parse_tree(&data)
    }

    /// Load a blob by hash, returning node and content.
    pub fn load_blob(&self, hash: B3Hash) -> Result<(BlobNode, Vec<u8>), FsMerkleError> {
        let data = self.cas.get(hash)?;
        parse_blob(&data)
    }

    /// Build a filesystem tree from a map of paths to content.
    pub fn build_tree_from_map(
        &self,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<B3Hash, FsMerkleError> {
        if files.is_empty() {
            return self.put_tree(Vec::new());
        }
        self.build_recursive(files, "")
    }

    /// Build a filesystem tree from a map of paths to blob hashes (blobs already in CAS).
    ///
    /// Unlike `build_tree_from_map`, this does NOT store blobs — it assumes they
    /// are already present in the CAS. Used by auto-fuse to construct merged trees
    /// without re-reading file content.
    pub fn build_tree_from_hash_map(
        &self,
        files: &BTreeMap<String, B3Hash>,
    ) -> Result<B3Hash, FsMerkleError> {
        if files.is_empty() {
            return self.put_tree(Vec::new());
        }
        self.build_hash_recursive(files, "")
    }

    fn build_hash_recursive(
        &self,
        files: &BTreeMap<String, B3Hash>,
        prefix: &str,
    ) -> Result<B3Hash, FsMerkleError> {
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut subdirs: BTreeMap<String, BTreeMap<String, B3Hash>> = BTreeMap::new();

        for (path, hash) in files {
            let rel_path = if prefix.is_empty() {
                path.clone()
            } else if let Some(stripped) = path.strip_prefix(&format!("{}/", prefix)) {
                stripped.to_string()
            } else {
                continue;
            };

            if let Some(slash_pos) = rel_path.find('/') {
                let dir_name = &rel_path[..slash_pos];
                subdirs
                    .entry(dir_name.to_string())
                    .or_default()
                    .insert(path.clone(), *hash);
            } else {
                entries.insert(
                    rel_path.clone(),
                    Entry {
                        name: rel_path,
                        mode: MODE_FILE,
                        kind: NodeKind::Blob,
                        hash: *hash,
                    },
                );
            }
        }

        for (dir_name, sub_files) in &subdirs {
            let sub_prefix = if prefix.is_empty() {
                dir_name.clone()
            } else {
                format!("{}/{}", prefix, dir_name)
            };
            let sub_hash = self.build_hash_recursive(sub_files, &sub_prefix)?;
            entries.insert(
                dir_name.clone(),
                Entry {
                    name: dir_name.clone(),
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: sub_hash,
                },
            );
        }

        let entry_vec: Vec<Entry> = entries.into_values().collect();
        self.put_tree(entry_vec)
    }

    fn build_recursive(
        &self,
        files: &BTreeMap<String, Vec<u8>>,
        prefix: &str,
    ) -> Result<B3Hash, FsMerkleError> {
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut subdirs: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();

        for (path, content) in files {
            let rel_path = if prefix.is_empty() {
                path.clone()
            } else if let Some(stripped) = path.strip_prefix(&format!("{}/", prefix)) {
                stripped.to_string()
            } else {
                continue;
            };

            if let Some(slash_pos) = rel_path.find('/') {
                let dir_name = &rel_path[..slash_pos];
                subdirs
                    .entry(dir_name.to_string())
                    .or_default()
                    .insert(path.clone(), content.clone());
            } else {
                let (hash, _size) = self.put_blob(content)?;
                entries.insert(
                    rel_path.clone(),
                    Entry {
                        name: rel_path,
                        mode: MODE_FILE,
                        kind: NodeKind::Blob,
                        hash,
                    },
                );
            }
        }

        for (dir_name, sub_files) in &subdirs {
            let sub_prefix = if prefix.is_empty() {
                dir_name.clone()
            } else {
                format!("{}/{}", prefix, dir_name)
            };
            let sub_hash = self.build_recursive(sub_files, &sub_prefix)?;
            entries.insert(
                dir_name.clone(),
                Entry {
                    name: dir_name.clone(),
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: sub_hash,
                },
            );
        }

        let entry_vec: Vec<Entry> = entries.into_values().collect();
        self.put_tree(entry_vec)
    }
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// The kind of change between two trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    TypeChange,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeKind::Added => write!(f, "added"),
            ChangeKind::Deleted => write!(f, "deleted"),
            ChangeKind::Modified => write!(f, "modified"),
            ChangeKind::TypeChange => write!(f, "typechange"),
        }
    }
}

/// A single change between two filesystem trees.
#[derive(Debug, Clone)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    pub old_hash: B3Hash,
    pub new_hash: B3Hash,
    pub old_mode: u32,
    pub new_mode: u32,
}

/// Compute differences between two filesystem trees.
pub fn diff_trees(a: B3Hash, b: B3Hash, store: &FsStore<'_>) -> Result<Vec<Change>, FsMerkleError> {
    let mut changes = Vec::new();
    diff_recursive(a, b, "", store, &mut changes)?;
    Ok(changes)
}

fn diff_recursive(
    a_hash: B3Hash,
    b_hash: B3Hash,
    prefix: &str,
    store: &FsStore<'_>,
    changes: &mut Vec<Change>,
) -> Result<(), FsMerkleError> {
    // Structural sharing: identical hashes means identical content
    if a_hash == b_hash {
        return Ok(());
    }

    let a_tree = if a_hash == B3Hash::ZERO {
        TreeNode { entries: vec![] }
    } else {
        store.load_tree(a_hash)?
    };

    let b_tree = if b_hash == B3Hash::ZERO {
        TreeNode { entries: vec![] }
    } else {
        store.load_tree(b_hash)?
    };

    let a_map: HashMap<&str, &Entry> = a_tree
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();
    let b_map: HashMap<&str, &Entry> = b_tree
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Collect all unique names
    let mut all_names: Vec<&str> = a_map.keys().chain(b_map.keys()).copied().collect();
    all_names.sort();
    all_names.dedup();

    for name in all_names {
        let child_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", prefix, name)
        };

        match (a_map.get(name), b_map.get(name)) {
            (None, Some(b_entry)) => {
                changes.push(Change {
                    path: child_path,
                    kind: ChangeKind::Added,
                    old_hash: B3Hash::ZERO,
                    new_hash: b_entry.hash,
                    old_mode: 0,
                    new_mode: b_entry.mode,
                });
            }
            (Some(a_entry), None) => {
                changes.push(Change {
                    path: child_path,
                    kind: ChangeKind::Deleted,
                    old_hash: a_entry.hash,
                    new_hash: B3Hash::ZERO,
                    old_mode: a_entry.mode,
                    new_mode: 0,
                });
            }
            (Some(a_entry), Some(b_entry)) => {
                if a_entry.kind != b_entry.kind {
                    changes.push(Change {
                        path: child_path,
                        kind: ChangeKind::TypeChange,
                        old_hash: a_entry.hash,
                        new_hash: b_entry.hash,
                        old_mode: a_entry.mode,
                        new_mode: b_entry.mode,
                    });
                } else if a_entry.hash != b_entry.hash {
                    if a_entry.kind == NodeKind::Tree {
                        diff_recursive(a_entry.hash, b_entry.hash, &child_path, store, changes)?;
                    } else {
                        changes.push(Change {
                            path: child_path,
                            kind: ChangeKind::Modified,
                            old_hash: a_entry.hash,
                            new_hash: b_entry.hash,
                            old_mode: a_entry.mode,
                            new_mode: b_entry.mode,
                        });
                    }
                }
            }
            (None, None) => unreachable!(),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

pub(crate) fn validate_name(name: &str) -> Result<(), FsMerkleError> {
    if name.is_empty() {
        return Err(FsMerkleError::InvalidName("empty filename".into()));
    }
    if name == "." || name == ".." {
        return Err(FsMerkleError::InvalidName(format!(
            "invalid filename: {:?}",
            name
        )));
    }
    if name.contains('/') {
        return Err(FsMerkleError::InvalidName(format!(
            "filename cannot contain path separator: {:?}",
            name
        )));
    }
    Ok(())
}

pub(crate) fn validate_mode(mode: u32, kind: NodeKind) -> Result<(), FsMerkleError> {
    match kind {
        NodeKind::Blob if mode != MODE_FILE && mode != MODE_EXEC && mode != MODE_SYMLINK => {
            Err(FsMerkleError::InvalidMode {
                mode,
                kind,
                expected: MODE_FILE,
            })
        }
        NodeKind::Tree if mode != MODE_DIR => Err(FsMerkleError::InvalidMode {
            mode,
            kind,
            expected: MODE_DIR,
        }),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FsMerkleError {
    #[error("invalid name: {0}")]
    InvalidName(String),

    #[error("duplicate name: {0}")]
    DuplicateName(String),

    #[error("unsorted entries: {prev:?} should come before {current:?}")]
    UnsortedEntries { prev: String, current: String },

    #[error("invalid mode {mode:#o} for {kind}, expected {expected:#o}")]
    InvalidMode {
        mode: u32,
        kind: NodeKind,
        expected: u32,
    },

    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("CAS error: {0}")]
    Cas(#[from] CasError),

    #[error("malformed tree: {0}")]
    Read(#[from] crate::reader::ReadError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::MemoryCas;

    #[test]
    fn blob_hash_deterministic() {
        let h1 = BlobNode::hash_content(b"hello");
        let h2 = BlobNode::hash_content(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn blob_different_content_different_hash() {
        let h1 = BlobNode::hash_content(b"hello");
        let h2 = BlobNode::hash_content(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn blob_canonical_format() {
        let content = b"hello";
        let canonical = BlobNode::canonical_bytes(content);
        assert!(canonical.starts_with(b"blob 5\x00"));
        assert!(canonical.ends_with(b"hello"));
    }

    #[test]
    fn blob_put_load_roundtrip() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let content = b"file content here";

        let (hash, size) = store.put_blob(content).unwrap();
        assert_eq!(size, content.len());

        let (node, loaded_content) = store.load_blob(hash).unwrap();
        assert_eq!(node.size, content.len());
        assert_eq!(loaded_content, content);
    }

    #[test]
    fn tree_empty() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let hash = store.put_tree(vec![]).unwrap();

        let tree = store.load_tree(hash).unwrap();
        assert!(tree.entries.is_empty());
    }

    #[test]
    fn tree_single_entry() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let blob_hash = BlobNode::hash_content(b"content");

        let hash = store
            .put_tree(vec![Entry {
                name: "file.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob_hash,
            }])
            .unwrap();

        let tree = store.load_tree(hash).unwrap();
        assert_eq!(tree.entries.len(), 1);
        assert_eq!(tree.entries[0].name, "file.txt");
        assert_eq!(tree.entries[0].hash, blob_hash);
    }

    #[test]
    fn tree_entries_sorted() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let h = B3Hash::ZERO;

        // Insert in reverse order
        let hash = store
            .put_tree(vec![
                Entry {
                    name: "z.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: h,
                },
                Entry {
                    name: "a.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: h,
                },
                Entry {
                    name: "m.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: h,
                },
            ])
            .unwrap();

        let tree = store.load_tree(hash).unwrap();
        assert_eq!(tree.entries[0].name, "a.txt");
        assert_eq!(tree.entries[1].name, "m.txt");
        assert_eq!(tree.entries[2].name, "z.txt");
    }

    #[test]
    fn tree_canonical_roundtrip() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let blob_hash = BlobNode::hash_content(b"data");

        let entries = vec![
            Entry {
                name: "README.md".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob_hash,
            },
            Entry {
                name: "src".into(),
                mode: MODE_DIR,
                kind: NodeKind::Tree,
                hash: B3Hash::ZERO,
            },
        ];

        let hash = store.put_tree(entries.clone()).unwrap();
        let loaded = store.load_tree(hash).unwrap();

        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].name, "README.md");
        assert_eq!(loaded.entries[1].name, "src");
    }

    #[test]
    fn validate_rejects_empty_name() {
        let tree = TreeNode {
            entries: vec![Entry {
                name: "".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: B3Hash::ZERO,
            }],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn validate_rejects_dot() {
        let tree = TreeNode {
            entries: vec![Entry {
                name: ".".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: B3Hash::ZERO,
            }],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn validate_rejects_dotdot() {
        let tree = TreeNode {
            entries: vec![Entry {
                name: "..".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: B3Hash::ZERO,
            }],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn validate_rejects_slash_in_name() {
        let tree = TreeNode {
            entries: vec![Entry {
                name: "a/b".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: B3Hash::ZERO,
            }],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn validate_rejects_wrong_mode_for_blob() {
        let tree = TreeNode {
            entries: vec![Entry {
                name: "file".into(),
                mode: MODE_DIR,
                kind: NodeKind::Blob,
                hash: B3Hash::ZERO,
            }],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn validate_rejects_duplicates() {
        let tree = TreeNode {
            entries: vec![
                Entry {
                    name: "dup".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: B3Hash::ZERO,
                },
                Entry {
                    name: "dup".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: B3Hash::ZERO,
                },
            ],
        };
        assert!(tree.validate().is_err());
    }

    #[test]
    fn build_tree_from_map() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files = BTreeMap::new();
        files.insert("README.md".into(), b"# Project".to_vec());
        files.insert("src/main.rs".into(), b"fn main() {}".to_vec());
        files.insert("src/lib.rs".into(), b"pub mod x;".to_vec());

        let root = store.build_tree_from_map(&files).unwrap();
        assert_ne!(root, B3Hash::ZERO);

        // Load root tree
        let tree = store.load_tree(root).unwrap();
        assert_eq!(tree.entries.len(), 2); // README.md + src/
        assert_eq!(tree.entries[0].name, "README.md");
        assert_eq!(tree.entries[0].kind, NodeKind::Blob);
        assert_eq!(tree.entries[1].name, "src");
        assert_eq!(tree.entries[1].kind, NodeKind::Tree);

        // Load src/ subtree
        let src_tree = store.load_tree(tree.entries[1].hash).unwrap();
        assert_eq!(src_tree.entries.len(), 2);
        assert_eq!(src_tree.entries[0].name, "lib.rs");
        assert_eq!(src_tree.entries[1].name, "main.rs");
    }

    #[test]
    fn build_tree_from_map_empty() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);
        let files = BTreeMap::new();
        let root = store.build_tree_from_map(&files).unwrap();

        let tree = store.load_tree(root).unwrap();
        assert!(tree.entries.is_empty());
    }

    #[test]
    fn structural_sharing() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        // Same content in different files should produce same blob hash
        let (h1, _) = store.put_blob(b"shared content").unwrap();
        let (h2, _) = store.put_blob(b"shared content").unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn diff_identical_trees() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files = BTreeMap::new();
        files.insert("a.txt".into(), b"hello".to_vec());
        let root = store.build_tree_from_map(&files).unwrap();

        let changes = diff_trees(root, root, &store).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn diff_added_file() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files_a = BTreeMap::new();
        files_a.insert("a.txt".into(), b"hello".to_vec());
        let root_a = store.build_tree_from_map(&files_a).unwrap();

        let mut files_b = BTreeMap::new();
        files_b.insert("a.txt".into(), b"hello".to_vec());
        files_b.insert("b.txt".into(), b"world".to_vec());
        let root_b = store.build_tree_from_map(&files_b).unwrap();

        let changes = diff_trees(root_a, root_b, &store).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "b.txt");
        assert_eq!(changes[0].kind, ChangeKind::Added);
    }

    #[test]
    fn diff_deleted_file() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files_a = BTreeMap::new();
        files_a.insert("a.txt".into(), b"hello".to_vec());
        files_a.insert("b.txt".into(), b"world".to_vec());
        let root_a = store.build_tree_from_map(&files_a).unwrap();

        let mut files_b = BTreeMap::new();
        files_b.insert("a.txt".into(), b"hello".to_vec());
        let root_b = store.build_tree_from_map(&files_b).unwrap();

        let changes = diff_trees(root_a, root_b, &store).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "b.txt");
        assert_eq!(changes[0].kind, ChangeKind::Deleted);
    }

    #[test]
    fn diff_modified_file() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files_a = BTreeMap::new();
        files_a.insert("a.txt".into(), b"original".to_vec());
        let root_a = store.build_tree_from_map(&files_a).unwrap();

        let mut files_b = BTreeMap::new();
        files_b.insert("a.txt".into(), b"modified".to_vec());
        let root_b = store.build_tree_from_map(&files_b).unwrap();

        let changes = diff_trees(root_a, root_b, &store).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "a.txt");
        assert_eq!(changes[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn diff_nested_changes() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files_a = BTreeMap::new();
        files_a.insert("src/main.rs".into(), b"fn main() {}".to_vec());
        files_a.insert("src/lib.rs".into(), b"pub mod x;".to_vec());
        let root_a = store.build_tree_from_map(&files_a).unwrap();

        let mut files_b = BTreeMap::new();
        files_b.insert("src/main.rs".into(), b"fn main() { run() }".to_vec());
        files_b.insert("src/lib.rs".into(), b"pub mod x;".to_vec()); // unchanged
        let root_b = store.build_tree_from_map(&files_b).unwrap();

        let changes = diff_trees(root_a, root_b, &store).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/main.rs");
        assert_eq!(changes[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn diff_from_empty() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let empty = store.build_tree_from_map(&BTreeMap::new()).unwrap();

        let mut files = BTreeMap::new();
        files.insert("new.txt".into(), b"content".to_vec());
        let root = store.build_tree_from_map(&files).unwrap();

        let changes = diff_trees(empty, root, &store).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Added);
    }

    #[test]
    fn build_tree_from_hash_map_flat() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        // Pre-store blobs
        let (h1, _) = store.put_blob(b"file one").unwrap();
        let (h2, _) = store.put_blob(b"file two").unwrap();

        let mut hash_files = BTreeMap::new();
        hash_files.insert("a.txt".into(), h1);
        hash_files.insert("b.txt".into(), h2);

        let root = store.build_tree_from_hash_map(&hash_files).unwrap();
        let tree = store.load_tree(root).unwrap();
        assert_eq!(tree.entries.len(), 2);
        assert_eq!(tree.entries[0].name, "a.txt");
        assert_eq!(tree.entries[0].hash, h1);
        assert_eq!(tree.entries[1].name, "b.txt");
        assert_eq!(tree.entries[1].hash, h2);
    }

    #[test]
    fn build_tree_from_hash_map_nested() {
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let (h1, _) = store.put_blob(b"readme").unwrap();
        let (h2, _) = store.put_blob(b"main code").unwrap();
        let (h3, _) = store.put_blob(b"lib code").unwrap();

        let mut hash_files = BTreeMap::new();
        hash_files.insert("README.md".into(), h1);
        hash_files.insert("src/main.rs".into(), h2);
        hash_files.insert("src/lib.rs".into(), h3);

        let root = store.build_tree_from_hash_map(&hash_files).unwrap();
        let tree = store.load_tree(root).unwrap();
        assert_eq!(tree.entries.len(), 2); // README.md + src/
        assert_eq!(tree.entries[0].name, "README.md");
        assert_eq!(tree.entries[1].name, "src");
        assert_eq!(tree.entries[1].kind, NodeKind::Tree);

        // Check src/ subtree
        let src_tree = store.load_tree(tree.entries[1].hash).unwrap();
        assert_eq!(src_tree.entries.len(), 2);
        assert_eq!(src_tree.entries[0].name, "lib.rs");
        assert_eq!(src_tree.entries[1].name, "main.rs");
    }

    #[test]
    fn build_tree_from_hash_map_matches_content_map() {
        // Verify that build_tree_from_hash_map produces the same tree as
        // build_tree_from_map when given matching inputs
        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut content_files = BTreeMap::new();
        content_files.insert("a.txt".into(), b"hello".to_vec());
        content_files.insert("dir/b.txt".into(), b"world".to_vec());

        let root_content = store.build_tree_from_map(&content_files).unwrap();

        // Now build the same tree using hash map
        let (ha, _) = store.put_blob(b"hello").unwrap();
        let (hb, _) = store.put_blob(b"world").unwrap();

        let mut hash_files = BTreeMap::new();
        hash_files.insert("a.txt".into(), ha);
        hash_files.insert("dir/b.txt".into(), hb);

        let root_hash = store.build_tree_from_hash_map(&hash_files).unwrap();

        // Both should produce the same root hash
        assert_eq!(root_content, root_hash);
    }
}
