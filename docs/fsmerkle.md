# Filesystem Merkle DAG Module (`fsmerkle.rs`)

Immutable Merkle DAG for representing filesystem trees.

## Overview

This module represents the working tree as an immutable Merkle DAG where:
- **BlobNode** represents file content
- **TreeNode** represents directories with sorted entries
- All content is identified by BLAKE3-256 hashes
- **Structural sharing** means unchanged subtrees share the same hash across timelines

## Canonical Encodings

### Blob

```
"blob <size>\x00" || raw_file_bytes
```

Hash = `BLAKE3(canonical_bytes)`

Example for a 5-byte file containing "hello":
```
blob 5\x00hello
```

### Tree

```
uvarint(entry_count)
for each entry (sorted by name):
  uvarint(mode)
  uvarint(name_length)
  name_bytes (UTF-8, no NUL terminator)
  kind_byte (1=blob, 2=tree)
  32_byte_hash
```

Hash = `BLAKE3(canonical_bytes)`

## Types

### `NodeKind`

```rust
pub enum NodeKind {
    Blob = 1,  // Regular file
    Tree = 2,  // Directory
}
```

### `Entry`

A single entry in a directory:
```rust
pub struct Entry {
    pub name: String,    // UTF-8 filename (POSIX rules)
    pub mode: u32,       // 0o100644 (file) or 0o040000 (directory)
    pub kind: NodeKind,  // Blob or Tree
    pub hash: B3Hash,    // BLAKE3 hash of the child node
}
```

### Mode Constants

- `MODE_FILE`: `0o100644` — regular file
- `MODE_DIR`: `0o040000` — directory

## Validation Rules

Entry names must:
- Be non-empty
- Not be `"."` or `".."`
- Not contain `"/"`
- Be unique within a directory
- Be sorted lexicographically

Modes must match their kind:
- `Blob` entries must use `MODE_FILE` (0o100644)
- `Tree` entries must use `MODE_DIR` (0o040000)

## Usage

### Storing and loading blobs

```rust
use ivaldi::cas::MemoryCas;
use ivaldi::fsmerkle::FsStore;

let cas = MemoryCas::new();
let store = FsStore::new(&cas);

let (hash, size) = store.put_blob(b"file content")?;
let (node, content) = store.load_blob(hash)?;
assert_eq!(content, b"file content");
```

### Building a directory tree

```rust
use ivaldi::fsmerkle::{FsStore, Entry, NodeKind, MODE_FILE, MODE_DIR};
use ivaldi::hash::B3Hash;

let cas = MemoryCas::new();
let store = FsStore::new(&cas);

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
        hash: src_tree_hash,
    },
];

let root_hash = store.put_tree(entries)?;
```

### Building from a file map (convenience)

```rust
use std::collections::BTreeMap;

let mut files = BTreeMap::new();
files.insert("README.md".into(), b"# Project".to_vec());
files.insert("src/main.rs".into(), b"fn main() {}".to_vec());
files.insert("src/lib.rs".into(), b"pub mod x;".to_vec());

let root_hash = store.build_tree_from_map(&files)?;

// Automatically creates:
//   root/
//   ├── README.md (blob)
//   └── src/ (tree)
//       ├── lib.rs (blob)
//       └── main.rs (blob)
```

### Diffing two trees

```rust
use ivaldi::fsmerkle::{diff_trees, ChangeKind};

let changes = diff_trees(old_root, new_root, &store)?;

for change in &changes {
    println!("{}: {}", change.kind, change.path);
    // Output: "modified: src/main.rs"
}
```

## Change Types

```rust
pub enum ChangeKind {
    Added,      // File/directory was added
    Deleted,    // File/directory was removed
    Modified,   // File content changed
    TypeChange, // Node type changed (file <-> directory)
}
```

## Structural Sharing

When two trees share subtrees, `diff_trees` short-circuits on identical hashes:

```
Tree A:                    Tree B:
root (hash_a)              root (hash_b)
├── README.md (hash_r)     ├── README.md (hash_r)  ← SAME hash, skipped
└── src/ (hash_s1)         └── src/ (hash_s2)      ← different, recurse
    ├── lib.rs (hash_l)        ├── lib.rs (hash_l)  ← SAME, skipped
    └── main.rs (hash_m1)     └── main.rs (hash_m2) ← different = Modified
```

Only the changed path (`src/main.rs`) is reported. Unchanged subtrees are never loaded from the CAS.

## Error Types

```rust
pub enum FsMerkleError {
    InvalidName(String),
    DuplicateName(String),
    UnsortedEntries { prev: String, current: String },
    InvalidMode { mode: u32, kind: NodeKind, expected: u32 },
    InvalidData(String),
    Cas(CasError),
}
```

## Design Decisions

- **Sorted entries**: Ensures canonical encoding is deterministic regardless of insertion order
- **POSIX filename validation**: Prevents path traversal and invalid filesystem entries
- **BTreeMap for build_tree_from_map**: Maintains sorted order naturally during tree construction
- **Structural sharing on diff**: O(changed_nodes) instead of O(all_nodes) — critical for large repos
- **Immutable trees**: New versions create new nodes, unchanged parts share storage
