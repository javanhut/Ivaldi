# Repo Module (`repo.rs`)

Persistent repository context for Ivaldi VCS.

## Overview

`Repo` is the main entry point for operations that need persistent state. It wires the `Store` (redb) to the in-memory MMR, providing commits, timeline management, and seal name lookups that survive across sessions.

## Usage

```rust
use ivaldi::repo::Repo;

// Open existing repo
let mut repo = Repo::open(work_dir)?;

// Create a commit
let result = repo.commit(tree_hash, "Alice <a@b.com>", "Add feature")?;
println!("Seal: {} ({})", result.seal_name, result.hash.short8());

// Walk history (full DAG: prev_idx + merge_idxs, newest first by index).
// Surfaces every commit reachable from the timeline head — including
// merge-second-parent ancestors that the older first-parent walk hid.
let history = repo.walk_history("main")?;

// First-parent only (legacy "linear chain" view). Useful for callers
// that explicitly want git-log-style first-parent traversal.
let linear = repo.walk_history_first_parent("main")?;

// Every leaf in the MMR — including ones orphaned from any timeline
// head (e.g., commits that were welded out of the chain). The MMR is
// append-only, so destructive history rewrites still leave the
// originals here, recoverable.
let all_leaves = repo.walk_all_leaves()?;

// Timeline management
repo.create_timeline("feature", None)?;
repo.switch_timeline("feature")?;
repo.remove_timeline("feature")?;
let timelines = repo.list_timelines()?;

// Resolve seal by name prefix or hash prefix
let (idx, leaf) = repo.resolve_seal("swift-eagle")?.unwrap();
```

## Persistence Guarantees

- **Commits**: Leaf bytes stored in redb, survive process restart
- **Timeline heads**: Stored in redb, updated atomically with each commit
- **Seal names**: Bidirectional mapping in redb, searchable by prefix
- **MMR**: Rebuilt from stored leaves on repo open — deterministic reconstruction

## Architecture

```
Repo::open()
  ├── Store::open(.ivaldi/store.db)  → redb database
  ├── FileCas::new(.ivaldi/objects)  → content-addressable storage
  └── Mmr::new() + replay stored leaves → in-memory MMR
```

## History walk modes

| Method | Walks | Use case |
|---|---|---|
| `walk_history` (default) | Full DAG: `prev_idx` + `merge_idxs` | `ivaldi log`, `ivaldi travel` |
| `walk_history_first_parent` | `prev_idx` only (linear chain) | Anyone wanting `git log --first-parent` semantics |
| `walk_history_dag` | Same as `walk_history` (explicit name) | Internal, for clarity at call sites |
| `walk_all_leaves` | Every leaf in the MMR (no chain following) | `ivaldi travel --all`; orphan recovery; reflog-style browsing |

All three return `Vec<HistoryEntry>` sorted newest-first by MMR index.

## Tested Scenarios

- Commit chains persist across reopen
- Divergent timelines (main + feature) persist independently
- Seal names survive reopen
- History walk returns correct order after reopen
- DAG walk includes merge-second-parent commits hidden from
  first-parent walk (regression test
  `walk_history_includes_merge_parents`)
