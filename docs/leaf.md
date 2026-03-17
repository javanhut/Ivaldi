# Leaf Module (`leaf.rs`)

Commit/seal record structure for Ivaldi VCS.

## Overview

Every commit in Ivaldi is represented as a `Leaf`. Leaves are appended to the MMR (Merkle Mountain Range) and contain all information needed to reconstruct the commit state and lineage.

## Structure

```rust
pub struct Leaf {
    pub tree_root: B3Hash,              // BLAKE3 root of filesystem tree
    pub timeline_id: String,            // Timeline (branch) name
    pub prev_idx: u64,                  // Previous leaf index (NO_PARENT if first)
    pub merge_idxs: Vec<u64>,           // Additional parent indices for merges
    pub author: String,                 // "Name <email>"
    pub time_unix: i64,                 // Unix timestamp
    pub message: String,                // Commit message
    pub meta: BTreeMap<String, String>, // Metadata (e.g., "autoshelved" → "1")
}
```

## Usage

```rust
use ivaldi::leaf::{Leaf, NO_PARENT, parse_leaf};
use ivaldi::hash::B3Hash;

// Create a new leaf
let leaf = Leaf::new(
    tree_root_hash,
    "main",
    "Alice <alice@example.com>",
    1700000000,
    "Add authentication feature",
);

// Check lineage
assert!(!leaf.has_parent());  // First commit
assert!(!leaf.is_merge());

// Compute hash (deterministic from canonical encoding)
let hash = leaf.hash();

// Encode/decode roundtrip
let bytes = leaf.canonical_bytes();
let parsed = parse_leaf(&bytes).unwrap();
```

## Canonical Encoding (Version 1)

Deterministic binary format used for hashing and storage:

```text
uvarint(1)                    // version
32 bytes TreeRoot             // filesystem tree hash
uvarint(len) + TimelineID    // timeline name
uvarint(PrevIdx)              // parent index (u64::MAX = no parent)
uvarint(count) + MergeIdxs   // merge parents
uvarint(len) + Author        // author string
varint(TimeUnix)              // signed timestamp
uvarint(len) + Message       // commit message
uvarint(count) + Meta        // key-value pairs (sorted by key)
```

## Metadata

The `meta` field stores arbitrary key-value pairs. Known keys:
- `"autoshelved"` → `"1"`: Marks this commit as an auto-shelf snapshot

```rust
leaf.set_autoshelved(true);
assert!(leaf.is_autoshelved());
```

## Design Decisions

- **BTreeMap for metadata**: Ensures sorted key order for deterministic canonical encoding
- **NO_PARENT = u64::MAX**: Sentinel value, same as Go's `^uint64(0)`
- **Signed varint for timestamp**: Supports dates before Unix epoch
- **Version byte**: Allows future format evolution without breaking compatibility
