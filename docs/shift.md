# Shift Module (`shift.rs`)

Commit squashing engine for Ivaldi VCS.

## Overview

Combines multiple sequential commits into a single commit, preserving the final tree state. Equivalent to `git rebase -i` squash but with a simpler, guided interface.

## Usage

```rust
use ivaldi::shift::{get_last_n, get_range, squash, combined_message};

// Get last N commits (newest first)
let commits = get_last_n(&mgr, "main", 3)?;

// Get commits in a range (oldest first)
let commits = get_range(&mgr, start_idx, end_idx)?;

// Generate combined message
let msg = combined_message(&commits);

// Perform squash (commits must be oldest-first)
let mut commits = get_last_n(&mgr, "main", 3)?;
commits.reverse(); // oldest first
let result = squash(&mut mgr, "main", &commits, "Clean message", "Author")?;
```

## How Squash Works

1. Identifies the commit range (start → end)
2. Takes the **final tree state** from the newest commit
3. Takes the **parent** from the oldest commit's parent
4. Creates a new commit: `parent → [squashed] (with final tree)`
5. Updates the timeline head to point to the new commit

```
Before:  A → B → C → D → E
                 \_squash_/
After:   A → B → S (tree of E, parent of C)
```

## Combined Message Format

```
Squashed 3 commits:

WIP: start feature
WIP: add tests
WIP: fix edge case
```

## Error Types

| Error | Cause |
|-------|-------|
| `TooFewCommits` | Less than 2 commits provided |
| `NoCommits` | Timeline has no commits |
| `NotEnoughCommits` | Timeline has fewer commits than requested |
| `NotDescendant` | End commit isn't reachable from start |
| `IndexOutOfRange` | Invalid leaf index |

## Design Decisions

- **Preserves final tree**: The squashed commit contains the exact same filesystem state as the newest commit in the range
- **Appends to MMR**: Old commits remain in the MMR (append-only). Only the timeline head pointer changes.
- **Oldest-first for squash**: `squash()` expects chronological order to correctly identify parent relationships
- **No interactive selection**: Core engine only. Arrow-key TUI selection is in the CLI layer.
