# Workspace Module (`workspace.rs`)

Workspace scanning, staging, status, and materialization.

## Overview

Manages the working directory state for Ivaldi VCS:
- **Scanning**: Walk the directory tree, respecting ignore patterns
- **Staging (gather)**: Read files, store in CAS, add to staging area
- **Status**: Compare working directory against last seal tree
- **Materialization**: Apply a tree hash to the working directory

## File States

| State | Meaning |
|-------|---------|
| `Untracked` | New file not in any seal |
| `Unmodified` | Matches the last seal |
| `Modified` | Changed since last seal, not staged |
| `Staged` | Gathered for the next seal |
| `Deleted` | Was in last seal, now missing |

## Staging Area

The staging area tracks files gathered for the next seal:

```rust
use ivaldi::workspace::StagingArea;

let mut staging = StagingArea::new();
staging.stage("file.txt", hash);
assert!(staging.is_staged("file.txt"));

staging.unstage("file.txt");
staging.clear();

// Persists to .ivaldi/stage/files
staging.save(&ivaldi_dir)?;
let loaded = StagingArea::load(&ivaldi_dir);
```

## Workspace Operations

```rust
use ivaldi::workspace::Workspace;

let mut ws = Workspace::new(&cas, work_dir, ivaldi_dir);

// Scan (respects ignore patterns)
let files = ws.scan(&ignore_cache)?;

// Gather specific files
ws.gather(&["src/main.rs", "README.md"])?;

// Gather everything
ws.gather_all(&ignore_cache)?;

// Progress-reporting variants (the CLI uses these to drive its
// spinner/progress bar; the plain forms delegate with a no-op callback)
ws.gather_all_with_progress(&ignore_cache, &mut |path| eprintln!("{path}"))?;

// Build tree from staged files
let tree_hash = ws.build_staged_tree()?;

// Check status
let status = ws.status(Some(last_tree_hash), &ignore)?;
for file in &status {
    println!("{}: {:?}", file.path, file.state);
}

// Materialize a tree to disk
ws.materialize(tree_hash)?;
```

## Materialization

When switching timelines, `materialize` applies the target tree state:
1. Loads all file paths and hashes from the target tree
2. Only writes files that differ from current disk state
3. Removes files not in the target tree
4. Creates missing directories

## Design Decisions

- **Scan skips `.ivaldi/`**: Internal directory is never included
- **Staging persists to disk**: Survives process restarts
- **Minimal writes on materialize**: Compares content before writing
- **BlobNode encoding for gather**: Files stored with canonical `"blob <size>\0"` prefix
- **BTreeMap for staging**: Deterministic ordering
