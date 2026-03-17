# Butterfly Module (`butterfly.rs`)

Experimental sandbox timelines for Ivaldi VCS.

## Overview

Butterflies are lightweight experimental timelines that branch from a parent. They enable safe experimentation — try code and implementations before committing to them on the parent timeline.

## Key Features

- **Branch from any timeline**: Creates a sandbox at the current divergence point
- **Bidirectional sync**: Push changes up to parent, pull parent changes down
- **Nested butterflies**: Create butterflies from butterflies (arbitrary depth)
- **Cascade delete**: Recursively delete entire butterfly trees
- **Orphan detection**: When a parent is deleted, children are marked orphaned

## Usage

```rust
use ivaldi::butterfly::ButterflyManager;
use ivaldi::hash::B3Hash;

let mut mgr = ButterflyManager::new();

// Create a butterfly from main
mgr.create("experiment", "main", divergence_hash, timestamp)?;
assert!(mgr.is_butterfly("experiment"));
assert!(!mgr.is_butterfly("main"));

// Get info
let bf = mgr.get("experiment").unwrap();
assert_eq!(bf.parent_name, "main");

// Nested butterflies
mgr.create("sub-experiment", "experiment", hash, ts)?;

// Get children
let children = mgr.get_children("experiment"); // ["sub-experiment"]

// Get tree structure
let tree = mgr.get_tree("main");
// [("main", 0), ("experiment", 1), ("sub-experiment", 2)]

// Update divergence after sync
mgr.update_divergence("experiment", new_hash)?;
```

## Deletion

```rust
// Without cascade: children become orphaned
mgr.delete("experiment", false)?;
assert!(mgr.get("sub-experiment").unwrap().is_orphaned);

// With cascade: recursively delete all descendants
mgr.delete("experiment", true)?;
assert!(!mgr.is_butterfly("sub-experiment")); // gone
```

## Orphaned Butterflies

When a parent butterfly is deleted without cascade:
- Child butterflies remain functional
- Marked as `is_orphaned = true`
- `original_parent` preserves the deleted parent's name
- Cannot sync up/down while orphaned

```rust
let orphans = mgr.list_orphaned();
```

## Metadata

```rust
let meta = mgr.get_metadata("experiment");
// ButterflyMetadata {
//   timeline: "experiment",
//   is_butterfly: true,
//   butterfly: Some(Butterfly { parent_name: "main", ... }),
//   children: ["sub-experiment"],
// }

// Non-butterfly timelines return empty metadata
let meta = mgr.get_metadata("main");
assert!(!meta.is_butterfly);
```

## Conflict Resolution Rules (for sync)

| Scenario | Resolution |
|----------|-----------|
| Both added same file | Layer: keep theirs on top |
| Both modified same file | Layer: apply changes sequentially |
| Deleted vs modified | Keep modified version |
| Added in one | Keep added file |

## Design Decisions

- **In-memory metadata**: BTreeMap-based storage. Persistent storage (`.ivaldi/butterflies/`) deferred to persistence layer.
- **Parent-child tracking**: Bidirectional — parent knows children, children know parent.
- **Cascade delete is recursive**: Deletes entire subtree depth-first.
- **Orphan preservation**: Orphaned butterflies remain usable, just can't sync with deleted parent.
- **Tree visualization**: `get_tree()` returns (name, depth) pairs for display.
