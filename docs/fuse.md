# Fuse Module (`fuse.rs`)

Three-way merge engine for Ivaldi VCS.

## Overview

The fuse engine merges file sets from two divergent timelines using a common ancestor as the base. Unlike Git's line-based merge, Ivaldi operates at the file level using BLAKE3 content hashes, eliminating false conflicts from identical changes.

**Key guarantee**: No conflict markers are ever written to workspace files. The workspace always stays clean.

## Strategies

| Strategy | Behavior |
|----------|----------|
| `Auto` | Intelligent three-way merge (default). Auto-resolves non-conflicting changes. Only flags truly conflicting files. |
| `Ours` | Keep all target timeline (left) versions. No conflicts possible. |
| `Theirs` | Accept all source timeline (right) versions. No conflicts possible. |
| `Union` | Combine both versions: clean changes auto-resolve; a genuine conflict concatenates ours then theirs into one blob. No conflicts surfaced. |
| `Base` | Revert to common ancestor. Discards all changes. |

## Usage

```rust
use ivaldi::fuse::{FuseEngine, Strategy};
use std::collections::BTreeMap;

let result = FuseEngine::fuse(&base_files, &ours_files, &theirs_files, Strategy::Auto);

if result.success {
    // result.merged_files contains the merged file set
} else {
    // result.conflicts contains unresolved conflicts
    for conflict in &result.conflicts {
        println!("CONFLICT: {}", conflict.path);
    }
}
```

## Three-Way Merge Logic (Auto Strategy)

For each file path across all three versions:

| Base | Ours | Theirs | Result |
|------|------|--------|--------|
| - | - | - | Delete |
| - | A | - | Take A (added left) |
| - | - | A | Take A (added right) |
| - | A | A | Take A (same addition) |
| - | A | B | **CONFLICT** (different additions) |
| X | - | - | Delete (both deleted) |
| X | X | - | Delete (unchanged left, deleted right) |
| X | A | - | **CONFLICT** (modified left, deleted right) |
| X | - | X | Delete (deleted left, unchanged right) |
| X | - | A | **CONFLICT** (deleted left, modified right) |
| X | A | A | Take A (both same change) |
| X | A | X | Take A (only left changed) |
| X | X | A | Take A (only right changed) |
| X | A | B | **CONFLICT** (both changed differently) |

## Fast-Forward Detection

```rust
if FuseEngine::is_fast_forward(&ours, &theirs, &base) {
    // No merge commit needed — just advance the pointer
}
```

A fast-forward occurs when the target timeline hasn't changed since the divergence point (`base == ours`).

## Design Decisions

- **Hash-based comparison**: Identical changes are auto-merged regardless of file content. No false conflicts from whitespace or formatting.
- **No conflict markers**: Conflicts are tracked as data structures, never written to files.
- **Strategy as parameter**: Same engine handles all strategies, simplifying the API.
- **BTreeMap for file sets**: Deterministic ordering, efficient lookup.
- **Union concatenation**: On a genuine conflict, union combines both versions by
  concatenating ours then theirs (deterministic, no separator) into a single blob,
  so no side is silently dropped. Intended for append-only files.
