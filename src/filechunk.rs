//! Chunked storage for large files (repository format >= 3).
//!
//! A file larger than [`CHUNKED_FILE_THRESHOLD`] is split into fixed
//! [`CHUNK_SIZE`] chunks. Each chunk is stored as a leaf object, and the
//! leaves are indexed by a tree of interior nodes with up to [`FANOUT`]
//! children each. The root takes the place of the classic `blob` object: a
//! directory entry of kind `Blob` names either encoding, and
//! `FsStore::load_blob` reads both. Editing part of a large file therefore
//! stores only the changed chunks plus the interior nodes on their paths, and
//! files are hashed, stored and materialized without holding them in memory.
//!
//! Canonical encoding (every node starts `'C' <version> <tag>`):
//! - Leaf:     `'C' 1 0 | chunk bytes`  (1..=CHUNK_SIZE bytes)
//! - Interior: `'C' 1 1 | height u8 | uvarint(total_size) | uvarint(count) | child_hash[32] * count`
//! - Hash:     `BLAKE3(canonical_bytes)`
//!
//! Shape: the leaves are the file's consecutive chunks, all full except the
//! last. Each level groups the one below it into runs of `FANOUT`, left to
//! right, and the root is the first level with a single node. So every leaf
//! sits at the same depth, every node except the rightmost on its level is
//! full, and the root has at least two children. Readers enforce all of it,
//! which makes content → root hash a bijection: the same bytes produce the
//! same root in every repository, and no second encoding of a file exists.
//!
//! The chunk size, fanout and threshold are part of the on-disk format.
//! Never change them without a repository format bump.

use crate::cas::{Cas, CasError};
use crate::hash::B3Hash;
use crate::reader::{ByteReader, ReadError};

/// Size of every chunk except a file's last: 1 MiB. Large enough that the
/// per-object fsync in `FileCas::put` stays cheap per GiB stored, small
/// enough that an in-place edit or append rewrites little.
pub const CHUNK_SIZE: usize = 1 << 20;

/// Files strictly larger than this are chunked (when the repository format
/// allows it — `Cas::chunked_files`). Smaller files stay classic blobs.
pub const CHUNKED_FILE_THRESHOLD: u64 = 4 << 20;

/// Maximum children per interior node.
pub const FANOUT: usize = 64;

/// Encoding version carried in every node.
pub const CHUNK_VERSION: u8 = 1;

const MAGIC: u8 = b'C';
const TAG_LEAF: u8 = 0;
const TAG_INTERIOR: u8 = 1;
const HEADER_LEN: usize = 3;

/// Deepest interior node accepted. 64^8 MiB exceeds any u64 file size, so
/// anything taller is hostile; the bound also stops a crafted reference cycle.
const MAX_HEIGHT: u8 = 8;

/// Shape parameters. Production always uses [`Params::DEFAULT`]; tests use
/// tiny values to exercise multi-level trees without gigabytes of input.
#[derive(Debug, Clone, Copy)]
struct Params {
    chunk_size: usize,
    fanout: usize,
    threshold: u64,
}

impl Params {
    const DEFAULT: Params = Params {
        chunk_size: CHUNK_SIZE,
        fanout: FANOUT,
        threshold: CHUNKED_FILE_THRESHOLD,
    };
}

fn invalid(msg: impl Into<String>) -> CasError {
    CasError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.into(),
    ))
}

fn invalid_read(e: ReadError) -> CasError {
    CasError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Whether `data` is any chunk node (leaf or interior).
pub fn is_chunk_node(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN
        && data[0] == MAGIC
        && data[1] == CHUNK_VERSION
        && data[2] <= TAG_INTERIOR
}

/// Whether `data` is an interior chunk node — the only kind a directory
/// entry may name. Classic blobs start with `blob `, so a three-byte prefix
/// is enough to tell the two encodings apart.
pub fn is_chunked_root(data: &[u8]) -> bool {
    is_chunk_node(data) && data[2] == TAG_INTERIOR
}

/// A decoded chunk node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkNode<'a> {
    Leaf(&'a [u8]),
    Interior {
        height: u8,
        total_size: u64,
        children: Vec<B3Hash>,
    },
}

/// Decode one node. Checks the encoding is canonical (exactly one byte
/// string per logical node) but not its place in a tree — that needs the
/// walk in [`read_to`].
pub fn parse_node(data: &[u8]) -> Result<ChunkNode<'_>, CasError> {
    parse_node_with(data, Params::DEFAULT)
}

fn parse_node_with(data: &[u8], params: Params) -> Result<ChunkNode<'_>, CasError> {
    if !is_chunk_node(data) {
        return Err(invalid("not a chunk node"));
    }
    let body = &data[HEADER_LEN..];
    if data[2] == TAG_LEAF {
        if body.is_empty() || body.len() > params.chunk_size {
            return Err(invalid(format!("chunk leaf of {} bytes", body.len())));
        }
        return Ok(ChunkNode::Leaf(body));
    }

    let mut r = ByteReader::new(body);
    let height = r.u8().map_err(invalid_read)?;
    let total_size = r.uvarint().map_err(invalid_read)?;
    let count = r.uvarint().map_err(invalid_read)?;
    if height == 0 || height > MAX_HEIGHT {
        return Err(invalid(format!("chunk interior height {height}")));
    }
    if count == 0 || count > params.fanout as u64 {
        return Err(invalid(format!("chunk interior with {count} children")));
    }
    if total_size < count {
        return Err(invalid("chunk interior smaller than its child count"));
    }
    let mut children = Vec::with_capacity(count as usize);
    for _ in 0..count {
        children.push(B3Hash::from_bytes(r.array::<32>().map_err(invalid_read)?));
    }
    r.finish().map_err(invalid_read)?;

    // Re-encode and compare: non-minimal varints must not smuggle in a
    // second encoding of the same node.
    let pairs: Vec<(B3Hash, u64)> = children.iter().map(|h| (*h, 0)).collect();
    if encode_interior(height, total_size, &pairs) != data {
        return Err(invalid("non-canonical chunk interior encoding"));
    }
    Ok(ChunkNode::Interior {
        height,
        total_size,
        children,
    })
}

fn encode_interior(height: u8, total_size: u64, children: &[(B3Hash, u64)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN + 1 + 20 + children.len() * 32);
    buf.extend_from_slice(&[MAGIC, CHUNK_VERSION, TAG_INTERIOR, height]);
    write_uvarint(&mut buf, total_size);
    write_uvarint(&mut buf, children.len() as u64);
    for (hash, _) in children {
        buf.extend_from_slice(hash.as_bytes());
    }
    buf
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Level {
    pending: Vec<(B3Hash, u64)>,
    emitted: u64,
}

/// Streams a file's bytes into the chunked encoding. With a sink, every node
/// is stored as soon as it is complete, children before parents, so a present
/// root implies a complete tree. Without one, it only computes the root hash
/// (used to compare a working file against a stored one).
///
/// Memory use is one chunk plus 40 bytes per pending node, independent of
/// the file size.
pub struct ChunkWriter<'a> {
    sink: Option<&'a dyn Cas>,
    params: Params,
    /// The leaf being filled, header included, so a full leaf is stored
    /// without copying.
    leaf: Vec<u8>,
    levels: Vec<Level>,
    total: u64,
}

impl<'a> ChunkWriter<'a> {
    pub fn new(sink: Option<&'a dyn Cas>) -> Self {
        Self::with_params(sink, Params::DEFAULT)
    }

    fn with_params(sink: Option<&'a dyn Cas>, params: Params) -> Self {
        let mut leaf = Vec::with_capacity(HEADER_LEN + params.chunk_size);
        leaf.extend_from_slice(&[MAGIC, CHUNK_VERSION, TAG_LEAF]);
        Self {
            sink,
            params,
            leaf,
            levels: vec![Level::default()],
            total: 0,
        }
    }

    pub fn write(&mut self, mut data: &[u8]) -> Result<(), CasError> {
        while !data.is_empty() {
            let room = HEADER_LEN + self.params.chunk_size - self.leaf.len();
            let take = room.min(data.len());
            self.leaf.extend_from_slice(&data[..take]);
            self.total += take as u64;
            data = &data[take..];
            if self.leaf.len() == HEADER_LEN + self.params.chunk_size {
                self.flush_leaf()?;
            }
        }
        Ok(())
    }

    fn store(&self, bytes: &[u8]) -> Result<B3Hash, CasError> {
        let hash = B3Hash::digest(bytes);
        if let Some(cas) = self.sink {
            cas.put(hash, bytes)?;
        }
        Ok(hash)
    }

    fn flush_leaf(&mut self) -> Result<(), CasError> {
        let size = (self.leaf.len() - HEADER_LEN) as u64;
        let hash = self.store(&self.leaf)?;
        self.leaf.truncate(HEADER_LEN);
        self.push(0, (hash, size))
    }

    fn push(&mut self, level: usize, node: (B3Hash, u64)) -> Result<(), CasError> {
        self.levels[level].pending.push(node);
        if self.levels[level].pending.len() == self.params.fanout {
            self.emit(level)?;
        }
        Ok(())
    }

    /// Replace a level's pending run with one interior node on the level above.
    fn emit(&mut self, level: usize) -> Result<(), CasError> {
        let children = std::mem::take(&mut self.levels[level].pending);
        let total: u64 = children.iter().map(|(_, size)| size).sum();
        let height = u8::try_from(level + 1)
            .ok()
            .filter(|h| *h <= MAX_HEIGHT)
            .ok_or_else(|| invalid("file too large for chunk tree"))?;
        let hash = self.store(&encode_interior(height, total, &children))?;
        self.levels[level].emitted += 1;
        if self.levels.len() == level + 1 {
            self.levels.push(Level::default());
        }
        self.push(level + 1, (hash, total))
    }

    /// Finish the tree and return `(root hash, total size)`. The content must
    /// span at least two chunks — smaller files belong in a classic blob.
    pub fn finish(mut self) -> Result<(B3Hash, u64), CasError> {
        if self.total <= self.params.chunk_size as u64 {
            return Err(invalid("content too small for chunked encoding"));
        }
        if self.leaf.len() > HEADER_LEN {
            self.flush_leaf()?;
        }
        // Bottom-up: a level whose only node never moved up is the root;
        // otherwise its trailing partial run becomes one more node above.
        let mut level = 0;
        loop {
            let lv = &self.levels[level];
            if lv.emitted == 0 && lv.pending.len() == 1 {
                let (hash, size) = lv.pending[0];
                return Ok((hash, size));
            }
            if !lv.pending.is_empty() {
                self.emit(level)?;
            }
            level += 1;
        }
    }
}

/// Root hash of `content` in the chunked encoding, without storing anything.
pub fn chunked_hash(content: &[u8]) -> Result<B3Hash, CasError> {
    let mut writer = ChunkWriter::new(None);
    writer.write(content)?;
    Ok(writer.finish()?.0)
}

/// Store `content` in the chunked encoding and return its root hash.
pub fn put_chunked(cas: &dyn Cas, content: &[u8]) -> Result<B3Hash, CasError> {
    let mut writer = ChunkWriter::new(Some(cas));
    writer.write(content)?;
    Ok(writer.finish()?.0)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Fetch a node and check it hashes to the name it was fetched by — `get`
/// does not verify, and nodes arrive verbatim from untrusted peers.
fn fetch(cas: &dyn Cas, hash: B3Hash) -> Result<Vec<u8>, CasError> {
    let data = cas.get(hash)?;
    let actual = B3Hash::digest(&data);
    if actual != hash {
        return Err(CasError::HashMismatch {
            expected: hash,
            actual,
        });
    }
    Ok(data)
}

/// Decode `data` as a tree root: an interior node with at least two
/// children, holding more than the threshold.
fn parse_root(data: &[u8], params: Params) -> Result<(u8, u64, Vec<B3Hash>), CasError> {
    match parse_node_with(data, params)? {
        ChunkNode::Interior {
            height,
            total_size,
            children,
        } => {
            if children.len() < 2 {
                return Err(invalid("chunk root with a single child"));
            }
            if total_size <= params.threshold {
                return Err(invalid(format!(
                    "chunked file of {total_size} bytes is below the chunking threshold"
                )));
            }
            Ok((height, total_size, children))
        }
        ChunkNode::Leaf(_) => Err(invalid("chunk leaf where a file root was expected")),
    }
}

/// Total size recorded in a root's bytes, without walking the tree.
pub fn root_size(data: &[u8]) -> Result<u64, CasError> {
    Ok(parse_root(data, Params::DEFAULT)?.1)
}

struct Walker<'a, 'f> {
    cas: &'a dyn Cas,
    params: Params,
    leaf: &'f mut dyn FnMut(&[u8]) -> Result<(), CasError>,
}

impl Walker<'_, '_> {
    /// Visit the children of an interior node at `height`, returning the
    /// number of content bytes below them.
    fn children(
        &mut self,
        height: u8,
        children: &[B3Hash],
        rightmost: bool,
    ) -> Result<u64, CasError> {
        let mut sum = 0u64;
        for (i, child) in children.iter().enumerate() {
            let child_rightmost = rightmost && i + 1 == children.len();
            sum = sum
                .checked_add(self.node(*child, height - 1, child_rightmost)?)
                .ok_or_else(|| invalid("chunk tree size overflows"))?;
        }
        Ok(sum)
    }

    fn node(&mut self, hash: B3Hash, height: u8, rightmost: bool) -> Result<u64, CasError> {
        let data = fetch(self.cas, hash)?;
        match parse_node_with(&data, self.params)? {
            ChunkNode::Leaf(bytes) => {
                if height != 0 {
                    return Err(invalid(format!("chunk leaf {hash} at height {height}")));
                }
                if !rightmost && bytes.len() != self.params.chunk_size {
                    return Err(invalid(format!(
                        "short chunk {hash} before the end of a file"
                    )));
                }
                (self.leaf)(bytes)?;
                Ok(bytes.len() as u64)
            }
            ChunkNode::Interior {
                height: h,
                total_size,
                children,
            } => {
                if h != height {
                    return Err(invalid(format!(
                        "chunk node {hash} has height {h}, expected {height}"
                    )));
                }
                if !rightmost && children.len() != self.params.fanout {
                    return Err(invalid(format!(
                        "partial chunk node {hash} before the end of a file"
                    )));
                }
                let sum = self.children(h, &children, rightmost)?;
                if sum != total_size {
                    return Err(invalid(format!(
                        "chunk node {hash} records {total_size} bytes but holds {sum}"
                    )));
                }
                Ok(sum)
            }
        }
    }
}

fn read_to_with(
    cas: &dyn Cas,
    root: B3Hash,
    params: Params,
    leaf: &mut dyn FnMut(&[u8]) -> Result<(), CasError>,
) -> Result<u64, CasError> {
    let data = fetch(cas, root)?;
    let (height, total_size, children) = parse_root(&data, params)?;
    let mut walker = Walker { cas, params, leaf };
    let sum = walker.children(height, &children, true)?;
    if sum != total_size {
        return Err(invalid(format!(
            "chunked file {root} records {total_size} bytes but holds {sum}"
        )));
    }
    Ok(sum)
}

/// Stream a chunked file's content to `leaf`, in order, validating every node
/// on the way (hash, encoding, height, shape, sizes). Returns the total size.
/// A tree that passes is exactly the tree [`ChunkWriter`] builds for the
/// bytes it yields.
pub fn read_to(
    cas: &dyn Cas,
    root: B3Hash,
    leaf: &mut dyn FnMut(&[u8]) -> Result<(), CasError>,
) -> Result<u64, CasError> {
    read_to_with(cas, root, Params::DEFAULT, leaf)
}

/// Read a chunked file's whole content.
pub fn read_all(cas: &dyn Cas, root: B3Hash) -> Result<Vec<u8>, CasError> {
    let size = root_size(&fetch(cas, root)?)?;
    // Capacity is capped: the recorded size is only trusted once the walk
    // has matched it against real leaves.
    let mut out = Vec::with_capacity(size.min(64 << 20) as usize);
    read_to(cas, root, &mut |bytes| {
        out.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(out)
}

/// Every object hash in a chunked file's tree, root first. Reads only the
/// interior nodes (about 1/64 of the tree); leaves are named, not fetched,
/// so a caller shipping or checking objects decides what to do with them.
pub fn node_hashes(cas: &dyn Cas, root: B3Hash) -> Result<Vec<B3Hash>, CasError> {
    node_hashes_with(cas, root, Params::DEFAULT)
}

fn node_hashes_with(cas: &dyn Cas, root: B3Hash, params: Params) -> Result<Vec<B3Hash>, CasError> {
    let mut out = vec![root];
    let mut stack = vec![(root, None::<u8>)];
    while let Some((hash, expected)) = stack.pop() {
        let data = fetch(cas, hash)?;
        let (height, children) = match expected {
            None => {
                let (height, _, children) = parse_root(&data, params)?;
                (height, children)
            }
            Some(expected) => match parse_node_with(&data, params)? {
                ChunkNode::Interior {
                    height, children, ..
                } if height == expected => (height, children),
                _ => {
                    return Err(invalid(format!(
                        "chunk node {hash} is not at height {expected}"
                    )));
                }
            },
        };
        out.extend_from_slice(&children);
        if height > 1 {
            stack.extend(children.into_iter().map(|c| (c, Some(height - 1))));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Varint helpers (shared by the other canonical encodings)
// ---------------------------------------------------------------------------

/// Write a u64 as a variable-length integer (LEB128 unsigned).
pub fn write_uvarint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Read a variable-length unsigned integer. Returns (value, bytes_consumed).
pub fn read_uvarint(data: &[u8]) -> (u64, usize) {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return (value, i + 1);
        }
        shift += 7;
        if shift >= 64 {
            break;
        }
    }
    (value, data.len())
}

/// Write an i64 as a variable-length signed integer (LEB128 zigzag).
pub fn write_varint(buf: &mut Vec<u8>, value: i64) {
    // Zigzag encoding: (value << 1) ^ (value >> 63)
    let encoded = ((value << 1) ^ (value >> 63)) as u64;
    write_uvarint(buf, encoded);
}

/// Read a variable-length signed integer. Returns (value, bytes_consumed).
pub fn read_varint(data: &[u8]) -> (i64, usize) {
    let (encoded, n) = read_uvarint(data);
    // Zigzag decode
    let value = ((encoded >> 1) as i64) ^ (-((encoded & 1) as i64));
    (value, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::MemoryCas;

    /// Chunk 4 bytes, fanout 3, threshold 8: 28 bytes is 7 leaves → 3 level-1
    /// nodes (3, 3, 1) → 1 root. Multi-level trees in a few bytes.
    const SMALL: Params = Params {
        chunk_size: 4,
        fanout: 3,
        threshold: 8,
    };

    fn build(cas: &MemoryCas, content: &[u8]) -> B3Hash {
        let mut w = ChunkWriter::with_params(Some(cas), SMALL);
        w.write(content).unwrap();
        w.finish().unwrap().0
    }

    fn read(cas: &MemoryCas, root: B3Hash) -> Result<Vec<u8>, CasError> {
        let mut out = Vec::new();
        read_to_with(cas, root, SMALL, &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })?;
        Ok(out)
    }

    fn put(cas: &MemoryCas, bytes: Vec<u8>) -> B3Hash {
        let hash = B3Hash::digest(&bytes);
        cas.put(hash, &bytes).unwrap();
        hash
    }

    fn leaf(cas: &MemoryCas, chunk: &[u8]) -> (B3Hash, u64) {
        let mut bytes = vec![MAGIC, CHUNK_VERSION, TAG_LEAF];
        bytes.extend_from_slice(chunk);
        (put(cas, bytes), chunk.len() as u64)
    }

    fn interior(cas: &MemoryCas, height: u8, children: &[(B3Hash, u64)]) -> (B3Hash, u64) {
        let total = children.iter().map(|c| c.1).sum();
        (put(cas, encode_interior(height, total, children)), total)
    }

    #[test]
    fn roundtrips_every_size_across_level_boundaries() {
        // 9..=120 bytes covers 3..=30 leaves: heights 1 through 4, full and
        // partial runs, and the single-child trailing node.
        for len in 9..=120usize {
            let cas = MemoryCas::new();
            let content: Vec<u8> = (0..len).map(|i| (i * 7 + len) as u8).collect();
            let root = build(&cas, &content);
            assert_eq!(read(&cas, root).unwrap(), content, "len {len}");
        }
    }

    #[test]
    fn streaming_in_any_pieces_gives_the_same_root() {
        let content: Vec<u8> = (0..100u8).collect();
        let whole = build(&MemoryCas::new(), &content);
        for piece in [1, 3, 4, 5, 17, 99] {
            let cas = MemoryCas::new();
            let mut w = ChunkWriter::with_params(Some(&cas), SMALL);
            for part in content.chunks(piece) {
                w.write(part).unwrap();
            }
            assert_eq!(w.finish().unwrap().0, whole, "piece size {piece}");
        }
    }

    #[test]
    fn hash_only_writer_matches_stored_root_and_stores_nothing() {
        let content = vec![9u8; 50];
        let stored = build(&MemoryCas::new(), &content);
        let mut w = ChunkWriter::with_params(None, SMALL);
        w.write(&content).unwrap();
        assert_eq!(w.finish().unwrap(), (stored, 50));
    }

    #[test]
    fn writer_matches_hand_built_canonical_shape() {
        // 7 leaves, fanout 3 → level 1: [l0 l1 l2] [l3 l4 l5] [l6] → root.
        let cas = MemoryCas::new();
        let content: Vec<u8> = (0..27u8).collect();
        let leaves: Vec<_> = content.chunks(4).map(|c| leaf(&cas, c)).collect();
        let a = interior(&cas, 1, &leaves[0..3]);
        let b = interior(&cas, 1, &leaves[3..6]);
        let c = interior(&cas, 1, &leaves[6..7]);
        let (root, _) = interior(&cas, 2, &[a, b, c]);
        assert_eq!(build(&MemoryCas::new(), &content), root);
        assert_eq!(read(&cas, root).unwrap(), content);
    }

    #[test]
    fn exact_fanout_power_has_no_extra_level() {
        // 9 leaves = 3 full level-1 nodes → root at height 2.
        let cas = MemoryCas::new();
        let root = build(&cas, &[1u8; 36]);
        match parse_node_with(&cas.get(root).unwrap(), SMALL).unwrap() {
            ChunkNode::Interior {
                height, children, ..
            } => {
                assert_eq!(height, 2);
                assert_eq!(children.len(), 3);
            }
            ChunkNode::Leaf(_) => panic!("root is a leaf"),
        }
    }

    #[test]
    fn content_within_one_chunk_is_refused() {
        let mut w = ChunkWriter::with_params(None, SMALL);
        w.write(b"abcd").unwrap();
        assert!(w.finish().is_err());
    }

    #[test]
    fn identical_chunks_are_stored_once() {
        let cas = MemoryCas::new();
        let root = build(&cas, &[0xAA; 36]); // 9 identical leaves
        assert_eq!(read(&cas, root).unwrap(), vec![0xAA; 36]);
        // 1 leaf + 1 level-1 node (all three identical) + root.
        assert_eq!(cas.len(), 3);
    }

    #[test]
    fn editing_one_chunk_rewrites_only_its_path() {
        let cas = MemoryCas::new();
        let mut content: Vec<u8> = (0..108u8).collect(); // 27 leaves, height 3
        build(&cas, &content);
        let before = cas.len();
        content[50] ^= 0xFF;
        build(&cas, &content);
        // One new leaf plus one new node per level (3).
        assert_eq!(cas.len() - before, 4);
    }

    #[test]
    fn node_hashes_names_every_object_reading_only_interiors() {
        let cas = MemoryCas::new();
        let content: Vec<u8> = (0..100u8).collect();
        let root = {
            let mut w = ChunkWriter::with_params(Some(&cas), SMALL);
            w.write(&content).unwrap();
            w.finish().unwrap().0
        };
        let mut all: Vec<B3Hash> = Vec::new();
        let mut stack = vec![root];
        while let Some(h) = stack.pop() {
            all.push(h);
            if let ChunkNode::Interior { children, .. } =
                parse_node_with(&cas.get(h).unwrap(), SMALL).unwrap()
            {
                stack.extend(children);
            }
        }
        let mut named = node_hashes_with(&cas, root, SMALL).unwrap();
        named.sort();
        named.dedup();
        all.sort();
        all.dedup();
        assert_eq!(named, all);
    }

    #[test]
    fn production_parameters_roundtrip_and_dedup() {
        let cas = MemoryCas::new();
        let mut content = vec![0u8; CHUNK_SIZE * 5 + 123];
        for (i, b) in content.iter_mut().enumerate() {
            *b = (i / 4096 + i / CHUNK_SIZE * 7) as u8;
        }
        let root = put_chunked(&cas, &content).unwrap();
        assert_eq!(chunked_hash(&content).unwrap(), root);
        assert_eq!(read_all(&cas, root).unwrap(), content);
        assert_eq!(
            root_size(&cas.get(root).unwrap()).unwrap(),
            content.len() as u64
        );
        assert_eq!(node_hashes(&cas, root).unwrap().len(), 7); // root + 6 leaves
    }

    // ---- Rejection of non-canonical and hostile trees ----

    #[test]
    fn rejects_short_chunk_before_the_end() {
        let cas = MemoryCas::new();
        let a = leaf(&cas, b"abc"); // short, but not last
        let b = leaf(&cas, b"defg");
        let c = leaf(&cas, b"hijk");
        let (root, _) = interior(&cas, 1, &[a, b, c]);
        assert!(read(&cas, root).is_err());
    }

    #[test]
    fn rejects_partial_node_before_the_end() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..4).map(|i| leaf(&cas, &[i; 4])).collect();
        let a = interior(&cas, 1, &l[0..2]); // partial, not rightmost
        let b = interior(&cas, 1, &l[2..4]);
        let (root, _) = interior(&cas, 2, &[a, b]);
        assert!(read(&cas, root).is_err());
    }

    #[test]
    fn rejects_single_child_root() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..3).map(|i| leaf(&cas, &[i; 4])).collect();
        let a = interior(&cas, 1, &l);
        let (root, _) = interior(&cas, 2, &[a]);
        assert!(read(&cas, root).is_err());
    }

    #[test]
    fn rejects_wrong_height_and_leaf_roots() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..3).map(|i| leaf(&cas, &[i; 4])).collect();
        let (bad, _) = interior(&cas, 2, &l); // leaves under a height-2 node
        assert!(read(&cas, bad).is_err());
        assert!(read(&cas, l[0].0).is_err());
    }

    #[test]
    fn rejects_size_mismatch() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..3).map(|i| leaf(&cas, &[i; 4])).collect();
        let root = put(&cas, encode_interior(1, 13, &l)); // holds 12
        assert!(read(&cas, root).is_err());
    }

    #[test]
    fn rejects_content_at_or_below_threshold() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..2).map(|i| leaf(&cas, &[i; 4])).collect();
        let (root, _) = interior(&cas, 1, &l); // 8 bytes == threshold
        assert!(read(&cas, root).is_err());
    }

    #[test]
    fn rejects_tampered_child() {
        /// Serves forged bytes under one hash, as a hostile peer could.
        struct Lying<'a>(&'a MemoryCas, B3Hash);
        impl Cas for Lying<'_> {
            fn put(&self, h: B3Hash, d: &[u8]) -> Result<(), CasError> {
                self.0.put(h, d)
            }
            fn get(&self, h: B3Hash) -> Result<Vec<u8>, CasError> {
                if h == self.1 {
                    Ok(vec![MAGIC, CHUNK_VERSION, TAG_LEAF, 1, 2, 3, 4])
                } else {
                    self.0.get(h)
                }
            }
            fn has(&self, h: B3Hash) -> Result<bool, CasError> {
                self.0.has(h)
            }
        }

        let cas = MemoryCas::new();
        let root = build(&cas, &(0..20u8).collect::<Vec<_>>());
        let victim = match parse_node_with(&cas.get(root).unwrap(), SMALL).unwrap() {
            ChunkNode::Interior { children, .. } => children[0],
            ChunkNode::Leaf(_) => unreachable!(),
        };
        assert!(matches!(
            read_to_with(&Lying(&cas, victim), root, SMALL, &mut |_| Ok(())),
            Err(CasError::HashMismatch { .. })
        ));
    }

    #[test]
    fn rejects_non_canonical_varints_and_trailing_bytes() {
        let cas = MemoryCas::new();
        let l: Vec<_> = (0..3).map(|i| leaf(&cas, &[i; 4])).collect();
        let good = encode_interior(1, 12, &l);
        assert!(parse_node_with(&good, SMALL).is_ok());

        // total_size 12 as a two-byte varint.
        let mut padded = good[..4].to_vec();
        padded.extend_from_slice(&[0x8C, 0x00]);
        padded.extend_from_slice(&good[5..]);
        assert!(parse_node_with(&padded, SMALL).is_err());

        let mut trailing = good.clone();
        trailing.push(0);
        assert!(parse_node_with(&trailing, SMALL).is_err());
    }

    #[test]
    fn classic_blobs_and_trees_are_not_chunk_nodes() {
        assert!(!is_chunk_node(b"blob 3\0abc"));
        assert!(!is_chunk_node(b""));
        assert!(!is_chunk_node(b"C"));
        assert!(!is_chunk_node(&[MAGIC, 2, TAG_LEAF, 0]));
        assert!(is_chunk_node(&[MAGIC, CHUNK_VERSION, TAG_LEAF, 0]));
        assert!(!is_chunked_root(&[MAGIC, CHUNK_VERSION, TAG_LEAF, 0]));
    }

    #[test]
    fn arbitrary_bytes_never_panic_the_parser() {
        let mut state = 0x9E3779B97F4A7C15u64;
        for _ in 0..2000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state % 80) as usize;
            let mut bytes: Vec<u8> = (0..len).map(|i| (state >> (i % 56)) as u8).collect();
            if bytes.len() >= 3 {
                bytes[0] = MAGIC;
                bytes[1] = CHUNK_VERSION;
                bytes[2] = (state & 1) as u8;
            }
            let _ = parse_node(&bytes);
        }
    }

    // ---- Uvarint encoding tests ----

    #[test]
    fn uvarint_roundtrip() {
        let test_values = [0u64, 1, 127, 128, 255, 256, 16383, 16384, u64::MAX];
        for &val in &test_values {
            let mut buf = Vec::new();
            write_uvarint(&mut buf, val);
            let (decoded, _) = read_uvarint(&buf);
            assert_eq!(decoded, val, "failed for value {}", val);
        }
    }

    #[test]
    fn varint_roundtrip() {
        let test_values = [0i64, 1, -1, 63, -64, 127, -128, i64::MAX, i64::MIN];
        for &val in &test_values {
            let mut buf = Vec::new();
            write_varint(&mut buf, val);
            let (decoded, _) = read_varint(&buf);
            assert_eq!(decoded, val, "failed for value {}", val);
        }
    }
}
