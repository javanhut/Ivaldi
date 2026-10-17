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

## Commands

```bash
ivaldi tl bf create experiment        # butterfly of the current timeline; switches to it
# ... make changes, seal ...
ivaldi tl bf up                       # merge the butterfly's sealed work into its parent
ivaldi tl bf down                     # merge the parent's sealed work into the butterfly
ivaldi tl bf rm experiment            # remove it; its own butterflies become orphans
ivaldi tl bf rm experiment --cascade  # remove it and everything nested under it
ivaldi tl bf rm experiment --force    # remove it even with shelved work, discarding that
```

- **`up` and `down` are fuses.** Each merges per file and per line, exactly
  as `ivaldi fuse` does, so neither side loses a seal: work the parent sealed
  after the butterfly was made survives `up`, and the butterfly's own seals
  survive `down`. Running either when the other side already has everything
  seals nothing.
- **`up` sends sealed work only.** Uncommitted changes stay on the butterfly,
  and the working directory is not touched.
- **`down` updates the working directory** to the merged result and carries
  uncommitted work through it, like `ivaldi fuse`. `ivaldi oops` undoes it.
- **What collides is settled by the layer rules** (below) instead of by a
  prompt, and the command lists every file it settled that way. The fuse
  engine treats edits within a few lines of a collision as part of it, so
  that whole stretch is settled together.
- **`rm` only removes butterflies** (`ivaldi timeline remove` is for other
  timelines) and refuses, before removing anything, if the checked-out
  timeline would go with it.
- **`rm` never strands shelved work.** Switching away from a timeline with
  uncommitted changes shelves them (`.ivaldi/shelves/<timeline>.shelf`), and
  nothing could reach that shelf once the timeline is gone. So `rm` checks
  every timeline it would remove, `--cascade` included, and refuses before
  removing any if one has shelved work. The error names each of them: switch
  to it and seal to keep the work, or rerun with `--force` to remove the
  shelves with the timelines. Plain `rm` checks only the butterfly itself;
  its children stay, shelves and all.
- **Records follow the timelines.** `ivaldi timeline remove` of a parent
  orphans its butterflies, and `ivaldi timeline rename` of a butterfly or a
  parent keeps every link intact. A renamed timeline takes its shelved work
  along.

## Library usage

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
- `parent_name` still names the deleted parent; `original_parent` records the timeline that parent was itself made from (`None` when the deleted parent was not a butterfly)
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

"Theirs" is the incoming side: the butterfly on `up`, the parent on `down`.
Changes that do not collide merge as in any fuse; these rules settle only
what does.

| Scenario | Resolution |
|----------|-----------|
| Both added same file | Layer: keep theirs on top |
| Both modified same file | Layer: apply changes sequentially |
| Deleted vs modified | Keep modified version |
| Added in one | Keep added file |

## Design Decisions

- **Stored metadata**: one JSON record per butterfly in the repository store (`store.db`). `ButterflyManager` is the in-memory model: `butterfly::load` builds it from the records, and each change is written back in one store transaction.
- **Parent-child tracking**: Bidirectional — parent knows children, children know parent.
- **Cascade delete is recursive**: Deletes entire subtree depth-first, after
  checking the whole subtree for the checked-out timeline and for shelved
  work, so a refusal removes nothing.
- **Orphan preservation**: Orphaned butterflies remain usable, just can't sync with deleted parent.
- **Tree visualization**: `get_tree()` returns (name, depth) pairs for display.
