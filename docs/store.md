# Store Module (`store.rs`)

ACID persistent storage for Ivaldi VCS, backed by `redb`.

## Overview

All data that must survive across sessions is stored in `store.db` via redb — a pure-Rust, crash-safe, embedded key-value database with no unsafe code.

Location: `.ivaldi/store.db`

## Tables

| Table | Key → Value | Purpose |
|-------|-------------|---------|
| `leaves` | `u64 → [u8]` | MMR leaf canonical bytes by index |
| `timeline_heads` | `str → u64` | Timeline name → head leaf index |
| `seal_to_hash` | `str → [u8]` | Seal name → BLAKE3 hash |
| `hash_to_seal` | `[u8] → str` | BLAKE3 hash → seal name |
| `butterflies` | `str → [u8]` | Butterfly name → metadata |
| `bf_children` | `str → str` | Parent → comma-separated children |
| `meta` | `str → str` | Generic metadata (e.g., mmr.size) |

## Properties

- **ACID**: All writes are transactional — commit or rollback, no partial state
- **Crash-safe**: Survives power loss, process crashes
- **Never-fail**: Critical for workspace rematerialization
- **No unsafe code**: redb is pure safe Rust
