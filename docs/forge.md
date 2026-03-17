# Forge Module (`forge.rs`)

Repository initialization for Ivaldi VCS.

## Overview

The `forge` command creates a new Ivaldi repository by initializing the `.ivaldi/` directory structure.

## Directory Structure

```
.ivaldi/
├── objects/        # Content-addressable storage (2-char sharding)
├── refs/
│   ├── heads/      # Timeline references
│   ├── remotes/    # Remote timeline references
│   └── seals/      # Seal name → hash mappings
├── shelves/        # Auto-shelving per-timeline
├── butterflies/    # Butterfly metadata
├── stage/          # Staging area
├── config          # Repository configuration
└── HEAD            # Current timeline pointer → "ref: refs/heads/main"
```

## Usage

```rust
use ivaldi::forge::{forge, is_ivaldi_repo, find_repo_root, read_head, HeadRef};

// Initialize
let result = forge(work_dir)?;
assert!(!result.already_existed);
assert_eq!(result.default_timeline, "main");

// Check if repo exists
assert!(is_ivaldi_repo(work_dir));

// Find repo root from any subdirectory
let root = find_repo_root(&subdir).unwrap();

// Read/write HEAD
let head = read_head(&ivaldi_dir)?;
match head {
    HeadRef::Timeline(name) => println!("On timeline: {}", name),
    HeadRef::Detached(hash) => println!("Detached at: {}", hash),
}
```

## Idempotent

Running `forge` on an existing repository is safe — it returns `already_existed: true` without modifying anything.

## HEAD Format

- Timeline: `ref: refs/heads/<name>\n`
- Detached: `<hash>\n`
