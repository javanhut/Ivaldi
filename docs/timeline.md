# Timeline Module (`timeline.rs`)

Timeline (branch) management for Ivaldi VCS.

## Overview

Timelines are Ivaldi's equivalent to Git branches, with enhanced features. A timeline is a lightweight pointer to the latest leaf (commit) index in the MMR.

The `HistoryManager` orchestrates:
- Committing to timelines
- Creating/switching/removing timelines
- LCA (Lowest Common Ancestor) computation for merges

CLI timeline switches are crash-recoverable: the switch sequence is
journaled to `.ivaldi/SWITCH_IN_PROGRESS` so an interrupted switch can be
completed or rolled back instead of leaving the working tree half
transitioned — see [switch_journal.md](switch_journal.md).

## Usage

```rust
use ivaldi::timeline::HistoryManager;
use ivaldi::leaf::Leaf;
use ivaldi::hash::B3Hash;

let mut mgr = HistoryManager::new();
// Default timeline is "main"

// Commit
let leaf = Leaf::new(tree_root, "main", "Author", timestamp, "message");
let (idx, root) = mgr.commit("main", leaf)?;

// Create timeline from current
mgr.create_timeline("feature", None)?;

// Create timeline from specific source
mgr.create_timeline("hotfix", Some("main"))?;

// Switch
mgr.switch_timeline("feature")?;
assert_eq!(mgr.current_timeline(), "feature");

// List (sorted)
let timelines = mgr.list_timelines();

// Remove
mgr.remove_timeline("feature")?;
```

## Rename (CLI)

`ivaldi tl rename` accepts three forms:

| Form | Behavior |
|---|---|
| `ivaldi tl rename NEW` | Rename the current timeline to NEW. |
| `ivaldi tl rename OLD NEW` | Rename OLD to NEW. |
| `ivaldi tl rename OLD to NEW` | Same as above with `to` as a connector word, for ergonomics: `ivaldi tl rename master to main`. |

Backed by `Repo::rename_timeline(old, new)`, which:

1. Refuses if NEW already exists, or if OLD has shelved work and NEW already
   holds a different shelf (one stranded by an older Ivaldi), before
   changing anything.
2. Creates the `.ivaldi/refs/heads/NEW` marker.
3. Moves OLD's shelf, if it has one, to `.ivaldi/shelves/NEW.shelf`, so
   uncommitted work shelved on OLD comes back on switching to NEW.
4. Moves the head from OLD to NEW in one store transaction.
5. Updates HEAD if the renamed timeline was the current one.
6. Removes the OLD marker, and moves OLD's butterfly records and links to NEW.

Every crash window leaves at worst a headless leftover marker, and rerunning
the rename finishes it. The shelf moves while OLD still exists for that
reason: a rename interrupted after it is completed by the retry.

Connectors other than `to` are rejected with a clear error
(`expected 'tl rename OLD to NEW' (got '<word>' between names)`).

## Remove (CLI)

```bash
ivaldi tl rm feature           # refused while 'feature' has shelved work
ivaldi tl rm feature --force   # remove it and discard its shelved work
```

Switching away from a timeline with uncommitted changes shelves them in
`.ivaldi/shelves/<timeline>.shelf` (see [shelf.md](shelf.md)). Only switching
back to that timeline restores them, so removing the timeline would leave the
shelf where nothing can reach it. `Repo::remove_timeline` therefore refuses
while the timeline has a shelf, with an error that names it and says how to
get the work back:

```
error: 'feature' has shelved work: uncommitted changes set aside when you switched away from it. Removing it would lose them, so nothing was removed.
To keep them, switch to it and seal:
  ivaldi timeline switch feature
To discard them, rerun with --force.
```

`--force` (`Repo::remove_timeline_discarding_shelf`) removes the shelf first
and then the timeline, so an interrupted removal leaves the timeline, which a
retry removes. The current timeline is refused either way, before anything
is discarded. `ivaldi tl bf rm` applies the same rule to every butterfly it
would remove (see [butterfly.md](butterfly.md)).

## Commit Behavior

When committing to a timeline, `HistoryManager` automatically:
1. Sets `leaf.prev_idx` to the current timeline head (or `NO_PARENT` if first)
2. Sets `leaf.timeline_id` to the timeline name
3. Appends the leaf to the MMR
4. Updates the timeline head pointer

```rust
mgr.commit("main", leaf1)?;  // idx=0, prev=NO_PARENT
mgr.commit("main", leaf2)?;  // idx=1, prev=0
mgr.commit("main", leaf3)?;  // idx=2, prev=1
```

## LCA (Lowest Common Ancestor)

Finds the common ancestor of two commits, essential for merge operations:

```rust
// Same timeline
mgr.commit("main", base)?;    // idx=0
mgr.commit("main", work)?;    // idx=1
assert_eq!(mgr.lca(1, 0)?, 0);

// Divergent timelines
mgr.commit("main", base)?;           // idx=0
mgr.create_timeline("feature", None)?;
mgr.commit("main", main_work)?;      // idx=1
mgr.commit("feature", feat_work)?;   // idx=2
assert_eq!(mgr.lca(1, 2)?, 0);       // base is LCA
```

## Error Types

```rust
pub enum TimelineError {
    NotFound(String),         // Timeline doesn't exist
    AlreadyExists(String),    // Name already taken
    CannotRemoveCurrent,      // Can't remove active timeline
    LeafOutOfRange(u64),      // Invalid leaf index
    NoCommonAncestor,         // No shared history
}
```

## Design Decisions

- **In-memory store**: Current implementation uses `HashMap` for timeline heads. Will be replaced with file-based refs in Stage 3 (`.ivaldi/refs/heads/`)
- **Ancestor-set LCA**: Traces back both chains and finds intersection. Works across timelines. Will be optimized with binary lifting for deep chains.
- **Sorted listing**: `list_timelines()` always returns names in sorted order
- **Default "main"**: New managers start with "main" as the current timeline
- **Cannot remove current**: Safety check prevents orphaning the active workspace
- **Cannot strand shelved work**: Removal refuses while the timeline has a shelf unless `--force` discards it; renaming moves the shelf
