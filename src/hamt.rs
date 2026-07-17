//! CAS-backed Hash Array Mapped Trie (HAMT) directory index.
//!
//! A directory is represented by the BLAKE3 hash of its root node. Every node
//! is stored in the CAS under the hash of its canonical encoding, so unchanged
//! subtrees keep their hashes across updates: inserting, overwriting, or
//! removing one entry rewrites only the ~log32(n) nodes on the path to it,
//! while `fsmerkle` re-encodes the whole directory. Old roots stay readable
//! forever — persistence falls out of content addressing.
//!
//! The structure is canonical: the same entry set always produces the same
//! root hash, regardless of the insert/remove order that built it. The
//! invariant that guarantees this: a subtree holding exactly one entry is
//! always a single leaf node (never a branch wrapping a leaf). Branches with
//! one *branch* child are allowed — shared hash-prefix chains force them.
//!
//! Wired into repository storage on format-2 repositories: `fsmerkle` routes
//! directories above `HAMT_DIR_THRESHOLD` entries through here (see docs/hamt.md).

use crate::cas::{put_and_hash, Cas, CasError};
use crate::filechunk::write_uvarint;
use crate::fsmerkle::{validate_mode, validate_name, Entry, FsMerkleError, NodeKind};
use crate::hash::B3Hash;
use crate::reader::ByteReader;
use std::collections::HashSet;

/// Encoding version of HAMT nodes. Bump on any breaking change.
pub const HAMT_VERSION: u8 = 1;

/// Magic byte prefixing every HAMT node — domain separation from fsmerkle
/// blobs/trees sharing the CAS (rescue discriminates objects by trial-parse).
const MAGIC: u8 = b'H';
const TAG_LEAF: u8 = 1;
const TAG_BRANCH: u8 = 2;

/// 5 hash bits per level: 32-way fan-out, matching the u32 bitmap.
const BITS_PER_LEVEL: usize = 5;
/// ceil(256 / 5). Two distinct names can only need a split past the last
/// level if their full BLAKE3 digests are equal — which already breaks the
/// entire CAS, so that is a hard error, not a supported state.
const MAX_LEVELS: u32 = 52;

/// Errors from HAMT operations.
#[derive(Debug, thiserror::Error)]
pub enum HamtError {
    #[error("invalid HAMT node: {0}")]
    InvalidData(String),

    #[error("node hash mismatch: expected {expected}, got {actual}")]
    NodeHashMismatch { expected: B3Hash, actual: B3Hash },

    #[error("full BLAKE3 digest collision between {0:?} and {1:?}")]
    HashCollision(String, String),

    #[error("HAMT deeper than {0} levels")]
    DepthExceeded(u32),

    #[error(transparent)]
    Entry(#[from] FsMerkleError),

    #[error("CAS error: {0}")]
    Cas(#[from] CasError),

    #[error("malformed HAMT node: {0}")]
    Read(#[from] crate::reader::ReadError),
}

/// A HAMT node: a leaf holding one directory entry, or a branch whose bitmap
/// says which of the 32 slots are occupied by the child node hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HamtNode {
    Leaf(Entry),
    Branch { bitmap: u32, children: Vec<B3Hash> },
}

impl HamtNode {
    /// Encode to canonical bytes, validating first. Exactly one byte string
    /// exists per logical node: uvarints are written minimally, children
    /// appear in ascending bit order, and there are no optional fields.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, HamtError> {
        let mut buf = Vec::new();
        buf.push(MAGIC);
        buf.push(HAMT_VERSION);
        match self {
            HamtNode::Leaf(entry) => {
                validate_name(&entry.name)?;
                validate_mode(entry.mode, entry.kind)?;
                buf.push(TAG_LEAF);
                write_uvarint(&mut buf, entry.mode as u64);
                write_uvarint(&mut buf, entry.name.len() as u64);
                buf.extend_from_slice(entry.name.as_bytes());
                buf.push(entry.kind as u8);
                buf.extend_from_slice(entry.hash.as_bytes());
            }
            HamtNode::Branch { bitmap, children } => {
                if bitmap.count_ones() as usize != children.len() {
                    return Err(HamtError::InvalidData(format!(
                        "bitmap popcount {} != child count {}",
                        bitmap.count_ones(),
                        children.len()
                    )));
                }
                buf.push(TAG_BRANCH);
                write_uvarint(&mut buf, *bitmap as u64);
                for child in children {
                    buf.extend_from_slice(child.as_bytes());
                }
            }
        }
        Ok(buf)
    }
}

/// Parse canonical HAMT node bytes. Bounds-checked and fully validated —
/// node bytes arrive verbatim from untrusted peers, the CAS only checks the
/// hash. Rejects anything that is not the exact canonical encoding: after a
/// structural parse the node is re-encoded and must reproduce the input
/// byte-for-byte, so non-minimal varints and any other representation slack
/// cannot smuggle in a second byte string for the same logical node.
pub fn parse_node(data: &[u8]) -> Result<HamtNode, HamtError> {
    let mut r = ByteReader::new(data);

    if r.u8()? != MAGIC {
        return Err(HamtError::InvalidData("bad magic".into()));
    }
    let version = r.u8()?;
    if version != HAMT_VERSION {
        return Err(HamtError::InvalidData(format!(
            "unsupported version: {}",
            version
        )));
    }

    let node = match r.u8()? {
        TAG_LEAF => {
            let mode = r.uvarint()?;
            if mode > u32::MAX as u64 {
                return Err(HamtError::InvalidData(format!("mode overflow: {}", mode)));
            }
            let mode = mode as u32;
            let name = r.string("name")?;
            let kind = match r.u8()? {
                1 => NodeKind::Blob,
                2 => NodeKind::Tree,
                k => return Err(HamtError::InvalidData(format!("unknown kind: {}", k))),
            };
            let hash = B3Hash::from_bytes(r.array::<32>()?);
            validate_name(&name)?;
            validate_mode(mode, kind)?;
            HamtNode::Leaf(Entry {
                name,
                mode,
                kind,
                hash,
            })
        }
        TAG_BRANCH => {
            let bitmap = r.uvarint()?;
            if bitmap > u32::MAX as u64 {
                return Err(HamtError::InvalidData(format!(
                    "bitmap overflow: {}",
                    bitmap
                )));
            }
            let bitmap = bitmap as u32;
            let mut children = Vec::new();
            for _ in 0..bitmap.count_ones() {
                children.push(B3Hash::from_bytes(r.array::<32>()?));
            }
            HamtNode::Branch { bitmap, children }
        }
        t => return Err(HamtError::InvalidData(format!("unknown tag: {}", t))),
    };

    r.finish()?; // reject trailing bytes

    if node.canonical_bytes()? != data {
        return Err(HamtError::InvalidData("non-canonical encoding".into()));
    }
    Ok(node)
}

/// Quick check whether raw CAS bytes are a HAMT node. Used to dispatch
/// between the two directory encodings: the `'H' <version>` prefix can never
/// begin a valid fsmerkle tree (it would put mode 1 on the first entry,
/// which `validate_mode` rejects), so the check is unambiguous.
pub fn is_hamt_node(data: &[u8]) -> bool {
    data.len() >= 3
        && data[0] == MAGIC
        && data[1] == HAMT_VERSION
        && (data[2] == TAG_LEAF || data[2] == TAG_BRANCH)
}

/// The 5-bit slot index for `hash` at trie level `level`. The hash is a
/// big-endian bit string; level L reads bits [5L, 5L+5) MSB-first through a
/// 16-bit window so the field crosses byte boundaries correctly. Bits past
/// 255 read as zero (only level 51 pads).
fn index_at_level(hash: &B3Hash, level: u32) -> usize {
    debug_assert!(level < MAX_LEVELS);
    let start = level as usize * BITS_PER_LEVEL;
    let b = hash.as_bytes();
    let hi = b[start / 8] as u16;
    let lo = *b.get(start / 8 + 1).unwrap_or(&0) as u16;
    (((hi << 8 | lo) >> (11 - start % 8)) & 0x1F) as usize
}

fn digest_name(name: &str) -> B3Hash {
    B3Hash::digest(name.as_bytes())
}

/// One entry's difference between two roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HamtChange {
    pub name: String,
    pub old: Option<Entry>,
    pub new: Option<Entry>,
}

/// Result of removing below a branch. `Leaf` bubbles a lone surviving leaf
/// upward without wrapping it in single-child branches, which is what keeps
/// the structure canonical after removals.
enum Removed {
    NotFound,
    Empty,
    Leaf(B3Hash),
    Node(B3Hash),
}

/// HAMT store backed by a CAS. Mirrors `FsStore`.
pub struct HamtStore<'a> {
    cas: &'a dyn Cas,
}

impl<'a> HamtStore<'a> {
    pub fn new(cas: &'a dyn Cas) -> Self {
        Self { cas }
    }

    /// Build a directory from entries, returning its root hash. Canonical by
    /// construction; writes exactly one CAS object per node.
    pub fn put_root(&self, entries: Vec<Entry>) -> Result<B3Hash, HamtError> {
        let mut seen = HashSet::new();
        for e in &entries {
            if !seen.insert(e.name.clone()) {
                return Err(FsMerkleError::DuplicateName(e.name.clone()).into());
            }
        }
        if entries.is_empty() {
            return self.store(&HamtNode::Branch {
                bitmap: 0,
                children: Vec::new(),
            });
        }
        let items: Vec<(B3Hash, Entry)> = entries
            .into_iter()
            .map(|e| (digest_name(&e.name), e))
            .collect();
        self.build_at(items, 0)
    }

    /// Look up an entry by name.
    pub fn get(&self, root: B3Hash, name: &str) -> Result<Option<Entry>, HamtError> {
        let digest = digest_name(name);
        let mut hash = root;
        let mut level = 0;
        loop {
            match self.load(hash, level)? {
                HamtNode::Leaf(e) => return Ok((e.name == name).then_some(e)),
                HamtNode::Branch { bitmap, children } => {
                    let bit = 1u32 << index_at_level(&digest, level);
                    if bitmap & bit == 0 {
                        return Ok(None);
                    }
                    hash = children[(bitmap & (bit - 1)).count_ones() as usize];
                    level += 1;
                }
            }
        }
    }

    /// Add or overwrite an entry, returning the new root hash. Writes only
    /// the nodes on the path to the entry; the old root stays valid.
    pub fn insert(&self, root: B3Hash, entry: Entry) -> Result<B3Hash, HamtError> {
        let digest = digest_name(&entry.name);
        self.insert_at(root, entry, &digest, 0)
    }

    /// Remove an entry by name. Returns the unchanged root if absent.
    pub fn remove(&self, root: B3Hash, name: &str) -> Result<B3Hash, HamtError> {
        let digest = digest_name(name);
        match self.remove_at(root, name, &digest, 0)? {
            Removed::NotFound => Ok(root),
            Removed::Empty => self.store(&HamtNode::Branch {
                bitmap: 0,
                children: Vec::new(),
            }),
            Removed::Leaf(h) | Removed::Node(h) => Ok(h),
        }
    }

    /// All entries, sorted by name. Verifies each leaf sits on the slot path
    /// its name's digest dictates, so a relocated leaf in a hostile tree is
    /// rejected rather than silently served under the wrong lookup path.
    // ponytail: collect + sort per call, same cost fsmerkle export pays; no
    // lazy sorted iterator until a profiler blames this.
    pub fn entries(&self, root: B3Hash) -> Result<Vec<Entry>, HamtError> {
        let mut out = Vec::new();
        let mut slots = Vec::new();
        self.walk(root, 0, &mut slots, &mut |e| out.push(e))?;
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Every HAMT node hash under `root` (inclusive) plus all directory
    /// entries. Object-graph walkers (sync transfer sets, reachability) use
    /// this to enumerate interior nodes, which no directory entry references
    /// directly — missing them would strand the receiver or GC them away.
    pub fn nodes_and_entries(
        &self,
        root: B3Hash,
    ) -> Result<(Vec<B3Hash>, Vec<Entry>), HamtError> {
        let mut nodes = Vec::new();
        let mut entries = Vec::new();
        self.collect_nodes(root, 0, &mut nodes, &mut entries)?;
        Ok((nodes, entries))
    }

    fn collect_nodes(
        &self,
        hash: B3Hash,
        level: u32,
        nodes: &mut Vec<B3Hash>,
        entries: &mut Vec<Entry>,
    ) -> Result<(), HamtError> {
        nodes.push(hash);
        match self.load(hash, level)? {
            HamtNode::Leaf(e) => entries.push(e),
            HamtNode::Branch { children, .. } => {
                for child in children {
                    self.collect_nodes(child, level + 1, nodes, entries)?;
                }
            }
        }
        Ok(())
    }

    /// Differences between two roots, sorted by name. Skips shared subtrees
    /// by hash equality, so cost is proportional to the change, not the tree.
    pub fn diff(&self, a: B3Hash, b: B3Hash) -> Result<Vec<HamtChange>, HamtError> {
        let mut out = Vec::new();
        self.diff_at(Some(a), Some(b), 0, &mut out)?;
        out.sort_by(|x, y| x.name.cmp(&y.name));
        Ok(out)
    }

    // -- internal ----------------------------------------------------------

    fn store(&self, node: &HamtNode) -> Result<B3Hash, HamtError> {
        Ok(put_and_hash(self.cas, &node.canonical_bytes()?)?)
    }

    /// Load and fully validate a node. Re-hashes the bytes (Cas::get does not
    /// verify) and applies the depth-context checks parsing alone cannot.
    fn load(&self, hash: B3Hash, level: u32) -> Result<HamtNode, HamtError> {
        if level >= MAX_LEVELS {
            return Err(HamtError::DepthExceeded(MAX_LEVELS));
        }
        let data = self.cas.get(hash)?;
        let actual = B3Hash::digest(&data);
        if actual != hash {
            return Err(HamtError::NodeHashMismatch {
                expected: hash,
                actual,
            });
        }
        let node = parse_node(&data)?;
        if matches!(node, HamtNode::Branch { bitmap: 0, .. }) && level != 0 {
            return Err(HamtError::InvalidData("empty branch below root".into()));
        }
        Ok(node)
    }

    fn build_at(&self, items: Vec<(B3Hash, Entry)>, level: u32) -> Result<B3Hash, HamtError> {
        if items.len() == 1 {
            let (_, entry) = items.into_iter().next().expect("len checked");
            return self.store(&HamtNode::Leaf(entry));
        }
        if level >= MAX_LEVELS {
            let mut names = items.into_iter().map(|(_, e)| e.name);
            return Err(HamtError::HashCollision(
                names.next().unwrap_or_default(),
                names.next().unwrap_or_default(),
            ));
        }
        let mut buckets: [Vec<(B3Hash, Entry)>; 32] = Default::default();
        for (digest, entry) in items {
            buckets[index_at_level(&digest, level)].push((digest, entry));
        }
        let mut bitmap = 0u32;
        let mut children = Vec::new();
        for (idx, bucket) in buckets.into_iter().enumerate() {
            if !bucket.is_empty() {
                bitmap |= 1 << idx;
                children.push(self.build_at(bucket, level + 1)?);
            }
        }
        self.store(&HamtNode::Branch { bitmap, children })
    }

    fn insert_at(
        &self,
        hash: B3Hash,
        entry: Entry,
        digest: &B3Hash,
        level: u32,
    ) -> Result<B3Hash, HamtError> {
        match self.load(hash, level)? {
            HamtNode::Leaf(existing) => {
                if existing.name == entry.name {
                    return self.store(&HamtNode::Leaf(entry)); // overwrite
                }
                // Split: descend to the first level where the two digests
                // diverge, put a two-child branch there, and chain
                // single-child branches back up to this level. The existing
                // leaf node is reused as-is — its hash does not change.
                let existing_digest = digest_name(&existing.name);
                let mut split = level;
                while split < MAX_LEVELS
                    && index_at_level(digest, split) == index_at_level(&existing_digest, split)
                {
                    split += 1;
                }
                if split >= MAX_LEVELS {
                    return Err(HamtError::HashCollision(existing.name, entry.name));
                }
                let new_idx = index_at_level(digest, split);
                let old_idx = index_at_level(&existing_digest, split);
                let new_leaf = self.store(&HamtNode::Leaf(entry))?;
                let (bitmap, children) = if new_idx < old_idx {
                    (1 << new_idx | 1 << old_idx, vec![new_leaf, hash])
                } else {
                    (1 << old_idx | 1 << new_idx, vec![hash, new_leaf])
                };
                let mut node_hash = self.store(&HamtNode::Branch { bitmap, children })?;
                for lvl in (level..split).rev() {
                    node_hash = self.store(&HamtNode::Branch {
                        bitmap: 1 << index_at_level(digest, lvl),
                        children: vec![node_hash],
                    })?;
                }
                Ok(node_hash)
            }
            HamtNode::Branch { bitmap: 0, .. } => {
                // Empty root: the single entry becomes a bare leaf root.
                self.store(&HamtNode::Leaf(entry))
            }
            HamtNode::Branch {
                bitmap,
                mut children,
            } => {
                let bit = 1u32 << index_at_level(digest, level);
                let pos = (bitmap & (bit - 1)).count_ones() as usize;
                if bitmap & bit == 0 {
                    let leaf = self.store(&HamtNode::Leaf(entry))?;
                    children.insert(pos, leaf);
                    self.store(&HamtNode::Branch {
                        bitmap: bitmap | bit,
                        children,
                    })
                } else {
                    children[pos] = self.insert_at(children[pos], entry, digest, level + 1)?;
                    self.store(&HamtNode::Branch { bitmap, children })
                }
            }
        }
    }

    fn remove_at(
        &self,
        hash: B3Hash,
        name: &str,
        digest: &B3Hash,
        level: u32,
    ) -> Result<Removed, HamtError> {
        match self.load(hash, level)? {
            HamtNode::Leaf(e) => Ok(if e.name == name {
                Removed::Empty
            } else {
                Removed::NotFound
            }),
            HamtNode::Branch {
                bitmap,
                mut children,
            } => {
                let bit = 1u32 << index_at_level(digest, level);
                if bitmap & bit == 0 {
                    return Ok(Removed::NotFound);
                }
                let pos = (bitmap & (bit - 1)).count_ones() as usize;
                match self.remove_at(children[pos], name, digest, level + 1)? {
                    Removed::NotFound => Ok(Removed::NotFound),
                    Removed::Empty => {
                        children.remove(pos);
                        let bitmap = bitmap & !bit;
                        self.collapse(bitmap, children, level)
                    }
                    Removed::Leaf(h) => {
                        children[pos] = h;
                        self.collapse(bitmap, children, level)
                    }
                    Removed::Node(h) => {
                        children[pos] = h;
                        Ok(Removed::Node(
                            self.store(&HamtNode::Branch { bitmap, children })?,
                        ))
                    }
                }
            }
        }
    }

    /// Rebuild a branch after a removal beneath it, restoring the canonical
    /// invariant: a lone surviving leaf bubbles up as `Removed::Leaf` instead
    /// of being wrapped in a single-child branch.
    fn collapse(
        &self,
        bitmap: u32,
        children: Vec<B3Hash>,
        level: u32,
    ) -> Result<Removed, HamtError> {
        match children.len() {
            0 => Ok(Removed::Empty),
            1 => {
                // Peek the survivor: a leaf bubbles up, a branch stays put
                // (prefix chains legitimately have single-branch children).
                match self.load(children[0], level + 1)? {
                    HamtNode::Leaf(_) => Ok(Removed::Leaf(children[0])),
                    HamtNode::Branch { .. } => Ok(Removed::Node(
                        self.store(&HamtNode::Branch { bitmap, children })?,
                    )),
                }
            }
            _ => Ok(Removed::Node(
                self.store(&HamtNode::Branch { bitmap, children })?,
            )),
        }
    }

    /// Depth-first walk over every entry. `slots` records the (level, slot)
    /// path taken; each leaf's name digest must reproduce it exactly.
    fn walk(
        &self,
        hash: B3Hash,
        level: u32,
        slots: &mut Vec<(u32, usize)>,
        f: &mut impl FnMut(Entry),
    ) -> Result<(), HamtError> {
        match self.load(hash, level)? {
            HamtNode::Leaf(e) => {
                let digest = digest_name(&e.name);
                for &(lvl, slot) in slots.iter() {
                    if index_at_level(&digest, lvl) != slot {
                        return Err(HamtError::InvalidData(format!(
                            "leaf {:?} under wrong slot path",
                            e.name
                        )));
                    }
                }
                f(e);
                Ok(())
            }
            HamtNode::Branch { bitmap, children } => {
                let mut pos = 0;
                for idx in 0..32 {
                    if bitmap & (1 << idx) != 0 {
                        slots.push((level, idx));
                        self.walk(children[pos], level + 1, slots, f)?;
                        slots.pop();
                        pos += 1;
                    }
                }
                Ok(())
            }
        }
    }

    /// Emit every entry under `hash` as purely-added or purely-removed.
    fn emit_all(
        &self,
        hash: B3Hash,
        level: u32,
        as_new: bool,
        out: &mut Vec<HamtChange>,
    ) -> Result<(), HamtError> {
        let mut slots = Vec::new();
        self.walk(hash, level, &mut slots, &mut |e| {
            out.push(HamtChange {
                name: e.name.clone(),
                old: (!as_new).then(|| e.clone()),
                new: as_new.then_some(e),
            })
        })
    }

    fn diff_at(
        &self,
        a: Option<B3Hash>,
        b: Option<B3Hash>,
        level: u32,
        out: &mut Vec<HamtChange>,
    ) -> Result<(), HamtError> {
        if a == b {
            return Ok(()); // identical subtrees (or both absent)
        }
        let a_node = a.map(|h| self.load(h, level)).transpose()?;
        let b_node = b.map(|h| self.load(h, level)).transpose()?;
        match (a_node, b_node) {
            (None, None) => Ok(()),
            (Some(_), None) => self.emit_all(a.expect("checked"), level, false, out),
            (None, Some(_)) => self.emit_all(b.expect("checked"), level, true, out),
            (Some(HamtNode::Leaf(ea)), Some(HamtNode::Leaf(eb))) => {
                if ea.name == eb.name {
                    if ea != eb {
                        out.push(HamtChange {
                            name: ea.name.clone(),
                            old: Some(ea),
                            new: Some(eb),
                        });
                    }
                } else {
                    out.push(HamtChange {
                        name: ea.name.clone(),
                        old: Some(ea),
                        new: None,
                    });
                    out.push(HamtChange {
                        name: eb.name.clone(),
                        old: None,
                        new: Some(eb),
                    });
                }
                Ok(())
            }
            // Leaf vs branch: route the leaf into the branch slot its digest
            // dictates at this level and recurse; other slots are one-sided.
            (Some(HamtNode::Leaf(ea)), Some(HamtNode::Branch { bitmap, children })) => {
                self.diff_leaf_vs_branch(a.expect("checked"), &ea, (bitmap, &children), level, false, out)
            }
            (Some(HamtNode::Branch { bitmap, children }), Some(HamtNode::Leaf(eb))) => {
                self.diff_leaf_vs_branch(b.expect("checked"), &eb, (bitmap, &children), level, true, out)
            }
            (
                Some(HamtNode::Branch {
                    bitmap: ba,
                    children: ca,
                }),
                Some(HamtNode::Branch {
                    bitmap: bb,
                    children: cb,
                }),
            ) => {
                let (mut pa, mut pb) = (0usize, 0usize);
                for idx in 0..32 {
                    let bit = 1u32 << idx;
                    let ha = (ba & bit != 0).then(|| {
                        let h = ca[pa];
                        pa += 1;
                        h
                    });
                    let hb = (bb & bit != 0).then(|| {
                        let h = cb[pb];
                        pb += 1;
                        h
                    });
                    if ha.is_some() || hb.is_some() {
                        self.diff_at(ha, hb, level + 1, out)?;
                    }
                }
                Ok(())
            }
        }
    }

    /// Diff a single leaf node against a branch at the same position.
    /// `leaf_is_new` says which root the leaf came from.
    fn diff_leaf_vs_branch(
        &self,
        leaf_hash: B3Hash,
        leaf: &Entry,
        (bitmap, children): (u32, &[B3Hash]),
        level: u32,
        leaf_is_new: bool,
        out: &mut Vec<HamtChange>,
    ) -> Result<(), HamtError> {
        let leaf_idx = index_at_level(&digest_name(&leaf.name), level);
        let mut pos = 0;
        let mut leaf_routed = false;
        for idx in 0..32 {
            let bit = 1u32 << idx;
            if bitmap & bit == 0 {
                continue;
            }
            let child = children[pos];
            pos += 1;
            if idx == leaf_idx {
                leaf_routed = true;
                let (ha, hb) = if leaf_is_new {
                    (Some(child), Some(leaf_hash))
                } else {
                    (Some(leaf_hash), Some(child))
                };
                self.diff_at(ha, hb, level + 1, out)?;
            } else {
                self.emit_all(child, level + 1, !leaf_is_new, out)?;
            }
        }
        if !leaf_routed {
            out.push(HamtChange {
                name: leaf.name.clone(),
                old: (!leaf_is_new).then(|| leaf.clone()),
                new: leaf_is_new.then(|| leaf.clone()),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::MemoryCas;
    use crate::fsmerkle::{MODE_DIR, MODE_FILE};

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            mode: MODE_FILE,
            kind: NodeKind::Blob,
            hash: B3Hash::digest(name.as_bytes()),
        }
    }

    #[test]
    fn index_crosses_byte_boundaries() {
        // Levels 1 and 3 start mid-byte (bit offsets 5 and 15): the 5-bit
        // field spans two bytes and must pull the low bits from the next one.
        let mut bytes = [0u8; 32];
        bytes[0] = 0b0000_0111; // bits 5..8 = 111
        bytes[1] = 0b1100_0000; // bits 8..10 = 11 → level 1 = 0b11111
        bytes[1] |= 0b0000_0001; // bit 15 = 1
        bytes[2] = 0b1111_0000; // bits 16..20 = 1111 → level 3 = 0b11111
        let h = B3Hash::from_bytes(bytes);
        assert_eq!(index_at_level(&h, 0), 0);
        assert_eq!(index_at_level(&h, 1), 0b11111);
        assert_eq!(index_at_level(&h, 3), 0b11111);
    }

    #[test]
    fn index_level_51_zero_pads() {
        // Level 51 reads bits [255, 260): only bit 255 is real, the rest pad
        // with zero, so an all-ones hash yields 0b10000.
        let h = B3Hash::from_bytes([0xFF; 32]);
        for level in 0..51 {
            assert_eq!(index_at_level(&h, level), 0x1F, "level {}", level);
        }
        assert_eq!(index_at_level(&h, 51), 0b10000);
    }

    #[test]
    fn golden_leaf_encoding() {
        // Locks the on-disk format: any change to these bytes is a breaking
        // format change and must bump HAMT_VERSION.
        let node = HamtNode::Leaf(Entry {
            name: "a".into(),
            mode: MODE_FILE,
            kind: NodeKind::Blob,
            hash: B3Hash::ZERO,
        });
        let mut expected = vec![b'H', 0x01, 0x01, 0xA4, 0x83, 0x02, 0x01, b'a', 0x01];
        expected.extend_from_slice(&[0u8; 32]);
        assert_eq!(node.canonical_bytes().unwrap(), expected);
        assert_eq!(parse_node(&expected).unwrap(), node);
    }

    #[test]
    fn golden_branch_encoding() {
        let c1 = B3Hash::from_bytes([0x11; 32]);
        let c2 = B3Hash::from_bytes([0x22; 32]);
        let node = HamtNode::Branch {
            bitmap: 0b101,
            children: vec![c1, c2],
        };
        let mut expected = vec![b'H', 0x01, 0x02, 0x05];
        expected.extend_from_slice(&[0x11; 32]);
        expected.extend_from_slice(&[0x22; 32]);
        assert_eq!(node.canonical_bytes().unwrap(), expected);
        assert_eq!(parse_node(&expected).unwrap(), node);
    }

    #[test]
    fn empty_root_roundtrip() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let root = store.put_root(Vec::new()).unwrap();
        assert_eq!(store.entries(root).unwrap(), Vec::new());
        assert_eq!(store.get(root, "x").unwrap(), None);
    }

    #[test]
    fn insert_get_overwrite_remove() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let empty = store.put_root(Vec::new()).unwrap();

        let root = store.insert(empty, entry("hello")).unwrap();
        assert_eq!(store.get(root, "hello").unwrap(), Some(entry("hello")));
        assert_eq!(store.get(root, "nope").unwrap(), None);

        let mut e2 = entry("hello");
        e2.hash = B3Hash::digest(b"other content");
        let root2 = store.insert(root, e2.clone()).unwrap();
        assert_eq!(store.get(root2, "hello").unwrap(), Some(e2));
        assert_eq!(store.get(root, "hello").unwrap(), Some(entry("hello"))); // old root intact

        let root3 = store.remove(root2, "hello").unwrap();
        assert_eq!(store.get(root3, "hello").unwrap(), None);
        assert_eq!(root3, empty); // canonical empty root
    }

    #[test]
    fn remove_absent_is_noop() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let root = store.put_root(vec![entry("a"), entry("b")]).unwrap();
        assert_eq!(store.remove(root, "missing").unwrap(), root);
    }

    #[test]
    fn incremental_equals_bulk() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let names: Vec<String> = (0..200).map(|i| format!("file_{:03}", i)).collect();

        let bulk = store
            .put_root(names.iter().map(|n| entry(n)).collect())
            .unwrap();
        let mut incremental = store.put_root(Vec::new()).unwrap();
        for n in &names {
            incremental = store.insert(incremental, entry(n)).unwrap();
        }
        assert_eq!(bulk, incremental);
        assert_eq!(store.entries(bulk).unwrap().len(), 200);
    }

    #[test]
    fn remove_restores_canonical_form() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let names: Vec<String> = (0..50).map(|i| format!("f{}", i)).collect();
        let full = store
            .put_root(names.iter().map(|n| entry(n)).collect())
            .unwrap();

        // Removing each entry must land exactly on the direct build of the
        // remaining set — collapse has to erase every trace of the removal.
        for victim in &names {
            let removed = store.remove(full, victim).unwrap();
            let direct = store
                .put_root(
                    names
                        .iter()
                        .filter(|n| n != &victim)
                        .map(|n| entry(n))
                        .collect(),
                )
                .unwrap();
            assert_eq!(removed, direct, "removing {:?}", victim);
        }
    }

    #[test]
    fn entries_sorted_by_name() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let root = store
            .put_root(vec![entry("zebra"), entry("apple"), entry("mango")])
            .unwrap();
        let names: Vec<String> = store
            .entries(root)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["apple", "mango", "zebra"]);
    }

    #[test]
    fn put_root_rejects_duplicates() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let err = store.put_root(vec![entry("a"), entry("a")]).unwrap_err();
        assert!(matches!(
            err,
            HamtError::Entry(FsMerkleError::DuplicateName(_))
        ));
    }

    #[test]
    fn diff_reports_changes() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let a = store
            .put_root(vec![entry("keep"), entry("drop"), entry("edit")])
            .unwrap();
        let mut edited = entry("edit");
        edited.hash = B3Hash::digest(b"new content");
        let b = store
            .put_root(vec![entry("keep"), entry("add"), edited.clone()])
            .unwrap();

        let changes = store.diff(a, b).unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(
            changes[0],
            HamtChange {
                name: "add".into(),
                old: None,
                new: Some(entry("add"))
            }
        );
        assert_eq!(
            changes[1],
            HamtChange {
                name: "drop".into(),
                old: Some(entry("drop")),
                new: None
            }
        );
        assert_eq!(
            changes[2],
            HamtChange {
                name: "edit".into(),
                old: Some(entry("edit")),
                new: Some(edited)
            }
        );

        assert_eq!(store.diff(a, a).unwrap(), Vec::new());
    }

    #[test]
    fn dir_entries_supported() {
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let dir = Entry {
            name: "src".into(),
            mode: MODE_DIR,
            kind: NodeKind::Tree,
            hash: B3Hash::digest(b"subtree"),
        };
        let root = store.put_root(vec![dir.clone(), entry("README")]).unwrap();
        assert_eq!(store.get(root, "src").unwrap(), Some(dir));
    }

    #[test]
    fn parse_rejects_bad_input() {
        // Structural rejects; the exhaustive corruption matrix lives in
        // tests/hamt_props.rs.
        assert!(parse_node(b"").is_err());
        assert!(parse_node(b"X\x01\x01").is_err()); // bad magic
        assert!(parse_node(b"H\x02\x01").is_err()); // bad version
        assert!(parse_node(b"H\x01\x03").is_err()); // bad tag
        let good = HamtNode::Leaf(entry("a")).canonical_bytes().unwrap();
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(parse_node(&trailing).is_err());
        assert!(parse_node(&good[..good.len() - 1]).is_err()); // truncated
    }
}
