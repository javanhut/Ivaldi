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

// Walk history (newest first)
let history = repo.walk_history("main")?;

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

## Tested Scenarios

- Commit chains persist across reopen
- Divergent timelines (main + feature) persist independently
- Seal names survive reopen
- History walk returns correct order after reopen
