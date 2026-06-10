//! Chunked Merkle trees for efficient file storage.
//!
//! Files are split into 64KB chunks and organized as a binary Merkle tree.
//!
//! Canonical Encoding:
//! - Leaf:     `0x00 | uvarint(len(chunk)) | chunk`
//! - Internal: `0x01 | uvarint(child_count) | child_hash[32] * count | uvarint(total_size)`
//! - Hash:     `BLAKE3(canonical_bytes)`

use crate::cas::{Cas, CasError};
use crate::hash::B3Hash;

/// Default chunk size: 64 KiB.
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Leaf marker byte.
const LEAF_MARKER: u8 = 0x00;
/// Internal node marker byte.
const NODE_MARKER: u8 = 0x01;

/// The kind of a Merkle tree node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Leaf,
    Internal,
}

/// A reference to a node in the chunked Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRef {
    pub hash: B3Hash,
    pub kind: NodeKind,
    pub size: u64,
}

/// Builds chunked Merkle trees from file content.
pub struct ChunkBuilder<'a> {
    cas: &'a dyn Cas,
    chunk_size: usize,
}

impl<'a> ChunkBuilder<'a> {
    pub fn new(cas: &'a dyn Cas) -> Self {
        Self {
            cas,
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    pub fn with_chunk_size(cas: &'a dyn Cas, chunk_size: usize) -> Self {
        Self { cas, chunk_size }
    }

    /// Build a Merkle tree from content bytes.
    pub fn build(&self, content: &[u8]) -> Result<NodeRef, CasError> {
        if content.is_empty() {
            return self.build_leaf(&[]);
        }

        let chunks: Vec<&[u8]> = content.chunks(self.chunk_size).collect();
        self.build_tree(&chunks)
    }

    fn build_tree(&self, chunks: &[&[u8]]) -> Result<NodeRef, CasError> {
        if chunks.is_empty() {
            return self.build_leaf(&[]);
        }

        if chunks.len() == 1 {
            return self.build_leaf(chunks[0]);
        }

        // Build leaf nodes
        let mut nodes: Vec<NodeRef> = chunks
            .iter()
            .map(|c| self.build_leaf(c))
            .collect::<Result<Vec<_>, _>>()?;

        // Build binary tree bottom-up
        while nodes.len() > 1 {
            let mut next_level = Vec::with_capacity(nodes.len().div_ceil(2));
            let mut i = 0;
            while i < nodes.len() {
                if i + 1 < nodes.len() {
                    let internal =
                        self.build_internal(&[nodes[i].clone(), nodes[i + 1].clone()])?;
                    next_level.push(internal);
                    i += 2;
                } else {
                    next_level.push(nodes[i].clone());
                    i += 1;
                }
            }
            nodes = next_level;
        }

        Ok(nodes.into_iter().next().unwrap())
    }

    fn build_leaf(&self, chunk: &[u8]) -> Result<NodeRef, CasError> {
        let canonical = encode_leaf(chunk);
        let hash = B3Hash::digest(&canonical);
        self.cas.put(hash, &canonical)?;
        Ok(NodeRef {
            hash,
            kind: NodeKind::Leaf,
            size: chunk.len() as u64,
        })
    }

    fn build_internal(&self, children: &[NodeRef]) -> Result<NodeRef, CasError> {
        let (canonical, total_size) = encode_internal(children);
        let hash = B3Hash::digest(&canonical);
        self.cas.put(hash, &canonical)?;
        Ok(NodeRef {
            hash,
            kind: NodeKind::Internal,
            size: total_size,
        })
    }
}

/// Reads content from chunked Merkle trees.
pub struct ChunkLoader<'a> {
    cas: &'a dyn Cas,
}

impl<'a> ChunkLoader<'a> {
    pub fn new(cas: &'a dyn Cas) -> Self {
        Self { cas }
    }

    /// Read the entire content of a file from its Merkle tree root.
    pub fn read_all(&self, root: &NodeRef) -> Result<Vec<u8>, CasError> {
        if root.size == 0 {
            return Ok(Vec::new());
        }
        let mut result = Vec::with_capacity(root.size as usize);
        self.read_node(root, &mut result)?;
        Ok(result)
    }

    fn read_node(&self, node: &NodeRef, out: &mut Vec<u8>) -> Result<(), CasError> {
        let data = self.cas.get(node.hash)?;
        match node.kind {
            NodeKind::Leaf => self.read_leaf(&data, out),
            NodeKind::Internal => self.read_internal(&data, out),
        }
    }

    fn read_leaf(&self, data: &[u8], out: &mut Vec<u8>) -> Result<(), CasError> {
        if data.is_empty() || data[0] != LEAF_MARKER {
            return Err(CasError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid leaf node encoding",
            )));
        }
        let (chunk_len, bytes_read) = read_uvarint(&data[1..]);
        let start = 1 + bytes_read;
        let end = start + chunk_len as usize;
        if end > data.len() {
            return Err(CasError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated leaf chunk data",
            )));
        }
        out.extend_from_slice(&data[start..end]);
        Ok(())
    }

    fn read_internal(&self, data: &[u8], out: &mut Vec<u8>) -> Result<(), CasError> {
        if data.is_empty() || data[0] != NODE_MARKER {
            return Err(CasError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid internal node encoding",
            )));
        }
        let (child_count, bytes_read) = read_uvarint(&data[1..]);
        let mut offset = 1 + bytes_read;

        for _ in 0..child_count {
            if offset + 32 > data.len() {
                return Err(CasError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "truncated child hash",
                )));
            }
            let child_hash = B3Hash::from_slice(&data[offset..offset + 32]).unwrap();
            offset += 32;

            // Determine child kind by reading its data
            let child_data = self.cas.get(child_hash)?;
            let kind = match child_data.first() {
                Some(&LEAF_MARKER) => NodeKind::Leaf,
                Some(&NODE_MARKER) => NodeKind::Internal,
                _ => {
                    return Err(CasError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid child node encoding",
                    )));
                }
            };

            let child_ref = NodeRef {
                hash: child_hash,
                kind,
                size: 0, // not needed for reading
            };
            self.read_node(&child_ref, out)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

fn encode_leaf(chunk: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 10 + chunk.len());
    buf.push(LEAF_MARKER);
    write_uvarint(&mut buf, chunk.len() as u64);
    buf.extend_from_slice(chunk);
    buf
}

fn encode_internal(children: &[NodeRef]) -> (Vec<u8>, u64) {
    let mut buf = Vec::new();
    let mut total_size: u64 = 0;

    buf.push(NODE_MARKER);
    write_uvarint(&mut buf, children.len() as u64);

    for child in children {
        buf.extend_from_slice(child.hash.as_bytes());
        total_size += child.size;
    }

    write_uvarint(&mut buf, total_size);
    (buf, total_size)
}

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

    #[test]
    fn empty_content() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::new(&cas);
        let root = builder.build(b"").unwrap();
        assert_eq!(root.size, 0);
        assert_eq!(root.kind, NodeKind::Leaf);

        let loader = ChunkLoader::new(&cas);
        let data = loader.read_all(&root).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn small_content_single_leaf() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::new(&cas);
        let content = b"hello ivaldi chunking";
        let root = builder.build(content).unwrap();
        assert_eq!(root.size, content.len() as u64);
        assert_eq!(root.kind, NodeKind::Leaf);

        let loader = ChunkLoader::new(&cas);
        let data = loader.read_all(&root).unwrap();
        assert_eq!(data, content);
    }

    #[test]
    fn exact_chunk_size() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::with_chunk_size(&cas, 16);
        let content = vec![0xAB; 16];
        let root = builder.build(&content).unwrap();
        assert_eq!(root.size, 16);
        assert_eq!(root.kind, NodeKind::Leaf);

        let loader = ChunkLoader::new(&cas);
        assert_eq!(loader.read_all(&root).unwrap(), content);
    }

    #[test]
    fn two_chunks() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::with_chunk_size(&cas, 8);
        let content = b"hello ivaldi world!!"; // 20 bytes → 3 chunks (8+8+4)
        let root = builder.build(content).unwrap();
        assert_eq!(root.size, 20);
        assert_eq!(root.kind, NodeKind::Internal);

        let loader = ChunkLoader::new(&cas);
        let data = loader.read_all(&root).unwrap();
        assert_eq!(data, content);
    }

    #[test]
    fn large_content_multiple_levels() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::with_chunk_size(&cas, 4);
        // 32 bytes → 8 chunks → 4 nodes → 2 nodes → 1 root
        let content: Vec<u8> = (0..32).collect();
        let root = builder.build(&content).unwrap();
        assert_eq!(root.size, 32);

        let loader = ChunkLoader::new(&cas);
        let data = loader.read_all(&root).unwrap();
        assert_eq!(data, content);
    }

    #[test]
    fn odd_number_of_chunks() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::with_chunk_size(&cas, 10);
        let content = vec![0xFF; 35]; // 4 chunks: 10+10+10+5
        let root = builder.build(&content).unwrap();
        assert_eq!(root.size, 35);

        let loader = ChunkLoader::new(&cas);
        assert_eq!(loader.read_all(&root).unwrap(), content);
    }

    #[test]
    fn deterministic_hash() {
        let cas1 = MemoryCas::new();
        let cas2 = MemoryCas::new();
        let content = b"deterministic test";

        let root1 = ChunkBuilder::new(&cas1).build(content).unwrap();
        let root2 = ChunkBuilder::new(&cas2).build(content).unwrap();
        assert_eq!(root1.hash, root2.hash);
    }

    #[test]
    fn default_chunk_size_is_64k() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::new(&cas);
        // Content smaller than 64K should be a single leaf
        let content = vec![0u8; 60_000];
        let root = builder.build(&content).unwrap();
        assert_eq!(root.kind, NodeKind::Leaf);

        // Content larger than 64K should produce internal nodes
        let content = vec![0u8; 128_000];
        let root = builder.build(&content).unwrap();
        assert_eq!(root.kind, NodeKind::Internal);
    }

    #[test]
    fn deduplication() {
        let cas = MemoryCas::new();
        let builder = ChunkBuilder::with_chunk_size(&cas, 8);
        // Repeated data: all chunks identical
        let content = vec![0xAA; 24]; // 3 chunks of 8 identical bytes
        let root = builder.build(&content).unwrap();

        let loader = ChunkLoader::new(&cas);
        assert_eq!(loader.read_all(&root).unwrap(), content);

        // CAS should have fewer objects than if each chunk were unique
        // (leaf stored once, but internal nodes reference same hash)
        assert!(cas.len() <= 3); // 1 unique leaf + internal nodes
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
