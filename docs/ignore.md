# Ignore Module (`ignore.rs`)

Pattern matching for `.ivaldiignore` files.

## Overview

Controls which files are excluded from version control. Supports glob patterns, directory matching, recursive globs, and automatic security exclusions.

## Pattern Syntax

| Pattern | Meaning |
|---------|---------|
| `*.log` | Glob: matches any `.log` file (basename) |
| `build/` | Directory: matches `build/` and everything under it |
| `**/*.tmp` | Recursive glob: matches `.tmp` files at any depth |
| `node_modules` | Literal: exact name match |
| `file[0-9].txt` | Character class: `file0.txt` through `file9.txt` |
| `test?.txt` | Single char wildcard: `test1.txt`, `testA.txt` |
| `# comment` | Comment line (ignored) |

## Auto-Excluded (Security)

Always excluded regardless of `.ivaldiignore`:
- `.env`, `.env.*` — environment variable files
- `.venv`, `.venv/` — Python virtual environments

## Built-In Defaults

Always excluded (VCS/tool directories):
- `.git/`, `.svn/`, `.hg/`, `.fossil/`, `.claude/`

## Special Rules

- `.ivaldiignore` itself is **never** ignored
- Directory patterns enable early pruning during `scan` (skip traversal entirely)

## Usage

```rust
use ivaldi::ignore::{PatternCache, load_pattern_cache};

// From patterns
let cache = PatternCache::new(&["*.log", "build/", "**/*.tmp"]);
assert!(cache.is_ignored("error.log"));
assert!(cache.is_ignored("build/output.js"));
assert!(!cache.is_ignored("README.md"));

// Directory pruning
assert!(cache.is_dir_ignored("build"));

// From .ivaldiignore file
let cache = load_pattern_cache(work_dir);
```
