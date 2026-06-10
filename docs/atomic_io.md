# Atomic I/O Module (`atomic_io.rs`)

Atomic file replacement for repository metadata files.

## Overview

A plain `fs::write` can leave a truncated file behind if the process
crashes mid-write. Every metadata file under `.ivaldi/` goes through
`atomic_write` instead: bytes are written to a unique temp file in the
same directory, fsynced, then renamed over the destination, and the
parent directory is fsynced best-effort. Readers observe either the old
contents or the new contents — never a partial file.

```rust
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()>
```

## Call sites

| File | Writer |
|------|--------|
| `.ivaldi/stage/files` | `StagingArea::save` (workspace.rs) |
| `.ivaldi/HEAD` | `forge::write_head` |
| `.ivaldi/shelves/<timeline>.shelf` | `ShelfManager::save_shelf` |
| `.ivaldi/MERGE_STATE` | `Repo::save_merge_state` |
| `.ivaldi/reviews/<id>.json` | `Repo::save_review` |
| `.ivaldi/dotfile-allowlist` | `DotfileAllowlist::save` |
| `.ivaldi/config` | `Config::save` |
| `.ivaldi/SWITCH_IN_PROGRESS` | `switch_journal::write` |

Working-tree writes (materialize/apply_changes) intentionally do NOT use
this — those are user files where plain writes are correct.

## Notes

- Temp names are `{name}.tmp.{pid}.{counter}`, mirroring `FileCas::put`,
  so concurrent processes can't collide; failures clean up the temp file.
- The rename requires the parent directory to already exist.
- macOS note: `sync_all` issues `F_FULLFSYNC`; these are sub-KB files
  written once per command, so the cost is negligible.
