# Remote Module (`remote.rs`)

Remote API types and SHA1↔BLAKE3 hash mapping for Ivaldi VCS.

## Overview

Provides data types for GitHub/GitLab API interactions and bidirectional SHA1↔BLAKE3 hash mapping used during remote sync.

**IMPORTANT**: SHA1 is used ONLY for remote sync mapping. All internal operations use BLAKE3 exclusively.

## Hash Mapping

```rust
use ivaldi::remote::HashMapping;

let mut mapping = HashMapping::new(&ivaldi_dir);

// Map during download/upload
mapping.insert("da39a3ee5e6b...", blake3_hash);
mapping.save()?;

// Lookup
let blake3 = mapping.get_blake3("da39a3ee5e6b...");
let sha1 = mapping.get_sha1(blake3_hash);
```

Storage: `.ivaldi/hash-map` — one mapping per line: `<sha1> <blake3_hex>`

## API Types

| Type | Purpose |
|------|---------|
| `RemoteRepo` | Repository metadata |
| `RemoteBranch` | Branch name + SHA1 |
| `RemoteCommit` | Commit with parents, author, message |
| `RemoteTreeEntry` | File entry from remote tree |
| `SyncMetadata` | Per-timeline sync state |

## Design Decisions

- **No HTTP client in this module**: Types only. Actual HTTP calls deferred to a transport layer (will use `ureq` or similar).
- **Bidirectional mapping**: Both directions needed — SHA1→BLAKE3 for download, BLAKE3→SHA1 for upload.
- **Persistent mapping**: Survives across sessions so repeated syncs don't re-download.
