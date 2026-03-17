# Shelf Module (`shelf.rs`)

Auto-shelving system for Ivaldi VCS.

## Overview

When switching timelines, uncommitted changes are automatically saved to a shelf and restored when returning. This is one of Ivaldi's key differentiators from Git — no manual stashing required.

## How It Works

1. **Before switch**: Staged files are saved to `.ivaldi/shelves/<timeline>.shelf`
2. **During switch**: Workspace is materialized to the target timeline's tree
3. **On return**: Shelf is loaded and staged files are restored

## Storage

Shelves are stored as simple text files:

```
timeline feature-auth
created_at 1700000000
staged <blake3_hash> src/auth.rs
staged <blake3_hash> src/login.rs
```

Location: `.ivaldi/shelves/<timeline-name>.shelf`

## Usage

```rust
use ivaldi::shelf::{ShelfManager, Shelf};

let mgr = ShelfManager::new(&ivaldi_dir);

// Save shelf
let shelf = Shelf {
    timeline: "feature".into(),
    staged_files: staged_map,
    created_at: timestamp,
};
mgr.save_shelf(&shelf)?;

// Load shelf
if let Some(shelf) = mgr.load_shelf("feature")? {
    // Restore staged files
}

// Check/list/remove
mgr.has_shelf("feature");
mgr.list_shelves()?;
mgr.remove_shelf("feature")?;
```

## Properties

- **Per-timeline**: Each timeline has at most one shelf
- **Overwrite on re-save**: New shelf replaces old one
- **Idempotent remove**: Removing nonexistent shelf is a no-op
- **Sorted listing**: `list_shelves()` returns timeline names in sorted order

## Design Decisions

- **Text format**: Simple, human-readable, easy to debug
- **One shelf per timeline**: Latest state wins, no shelf history needed
- **Auto-created**: User never interacts with shelves directly
- **BTreeMap for staged files**: Deterministic save order
