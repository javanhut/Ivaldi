# Log Module (`log.rs`)

History log retrieval and formatting for Ivaldi VCS.

## Overview

Walks commit history from timeline heads backwards, producing display-ready entries with seal names, authors, timestamps, and parent info.

## Usage

```rust
use ivaldi::log::{walk_timeline, get_log, LogOptions, relative_time};

// Walk a single timeline (newest first)
let entries = walk_timeline(&mgr, "main");

// With options
let opts = LogOptions { limit: 10, all_timelines: false };
let entries = get_log(&mgr, &opts);

// All timelines (deduplicated, sorted by time)
let opts = LogOptions { limit: 0, all_timelines: true };
let entries = get_log(&mgr, &opts);
```

## LogEntry Fields

| Field | Description |
|-------|-------------|
| `index` | Leaf index in the MMR |
| `hash` | Full BLAKE3 hash |
| `short_hash` | First 8 hex chars |
| `seal_name` | Deterministic memorable name |
| `author` | Commit author string |
| `message` | Commit message |
| `time_unix` | Unix timestamp |
| `timeline` | Timeline name |
| `is_merge` | Whether this is a merge commit |
| `parents` | Parent leaf indices |

## Formatting

```rust
use ivaldi::log::{format_entry_oneline, format_entry_full, relative_time};

// Oneline: "a1b2c3d4 swift-eagle-flies-high-a1b2c3d4 Add feature"
let line = format_entry_oneline(&entry);

// Full multi-line display
let full = format_entry_full(&entry);

// Relative time: "3 hours ago", "just now", "2 days ago"
let rel = relative_time(timestamp, now);
```

## Design Decisions

- **Newest first**: Matches user expectation for `ivaldi log`
- **Dedup on all_timelines**: Same commit on multiple timelines appears once
- **Seal names generated on-the-fly**: No lookup needed, deterministic from hash
