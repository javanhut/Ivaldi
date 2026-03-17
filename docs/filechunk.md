# File Chunking Module (`filechunk.rs`)

Chunked Merkle trees for efficient file storage.

## Overview

Large files are split into 64KB chunks and organized as a binary Merkle tree. This enables:
- **Deduplication**: Identical chunks across files or versions are stored once
- **Efficient updates**: Only modified chunks need re-uploading
- **Parallel processing**: Chunks can be hashed/transferred concurrently
- **Integrity verification**: Tree structure provides cryptographic proof of content

## Constants

- `DEFAULT_CHUNK_SIZE`: 64 KiB (65,536 bytes)

## Canonical Encoding

Each node in the Merkle tree has a canonical byte representation:

**Leaf node** (contains raw chunk data):
```
0x00 | uvarint(chunk_length) | chunk_bytes
```

**Internal node** (references child nodes):
```
0x01 | uvarint(child_count) | child_hash[32] * count | uvarint(total_size)
```

Node hash = `BLAKE3(canonical_bytes)`

## Usage

### Building a Merkle tree

```rust
use ivaldi::cas::MemoryCas;
use ivaldi::filechunk::{ChunkBuilder, ChunkLoader};

let cas = MemoryCas::new();
let builder = ChunkBuilder::new(&cas);

// Build tree from content
let root = builder.build(b"file content here")?;
println!("Root hash: {}", root.hash);
println!("Total size: {} bytes", root.size);
println!("Node kind: {:?}", root.kind);  // Leaf for small files, Internal for large
```

### Reading content back

```rust
let loader = ChunkLoader::new(&cas);
let content = loader.read_all(&root)?;
assert_eq!(content, b"file content here");
```

### Custom chunk size (for testing)

```rust
let builder = ChunkBuilder::with_chunk_size(&cas, 1024); // 1KB chunks
```

## Types

### `NodeKind`

```rust
pub enum NodeKind {
    Leaf,      // Contains raw chunk data
    Internal,  // Contains references to child nodes
}
```

### `NodeRef`

Reference to a node in the chunked Merkle tree:
```rust
pub struct NodeRef {
    pub hash: B3Hash,    // BLAKE3 hash of canonical encoding
    pub kind: NodeKind,  // Leaf or Internal
    pub size: u64,       // Total bytes covered by this subtree
}
```

## Variable-Length Integer Encoding

The module provides LEB128 encoding helpers used throughout Ivaldi:

```rust
use ivaldi::filechunk::{write_uvarint, read_uvarint, write_varint, read_varint};

// Unsigned
let mut buf = Vec::new();
write_uvarint(&mut buf, 300);
let (value, bytes_read) = read_uvarint(&buf);
assert_eq!(value, 300);

// Signed (zigzag encoding)
let mut buf = Vec::new();
write_varint(&mut buf, -42);
let (value, bytes_read) = read_varint(&buf);
assert_eq!(value, -42);
```

## Tree Structure Example

For a 256KB file with 64KB chunks:

```
         Internal (root)
        /              \
    Internal          Internal
    /     \           /     \
Leaf(64K) Leaf(64K) Leaf(64K) Leaf(64K)
```

## Design Decisions

- **64KB chunks**: Balances deduplication granularity with tree overhead
- **Binary tree**: Pairs of children at each level, odd children promoted
- **LEB128 encoding**: Space-efficient for variable-length integers, compatible with Go's `binary.PutUvarint`
- **Deterministic**: Same content always produces the same tree hash
