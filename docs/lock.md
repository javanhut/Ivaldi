# Lock Module (`lock.rs`)

Process-level repository lock for mutating commands.

## Overview

redb serializes individual store transactions, but multi-step operations
(seal, timeline switch, fuse, …) also touch plain files under `.ivaldi/`
(HEAD, staging, shelves) with no coordination. `RepoLock` gives mutating
commands an exclusive advisory `flock(2)` on `.ivaldi/repo.lock` so two
concurrent ivaldi processes can't interleave.

```rust
let _lock = RepoLock::acquire(&ivaldi_dir)?;  // released on drop / process death
```

## Behavior

- **Non-blocking**: a second mutating command fails immediately with
  "another ivaldi process is operating on this repository" instead of
  hanging.
- **Crash-safe**: the kernel releases the flock when the holding process
  exits, even on a crash — a stale lock file is never a problem (this is
  why an `O_CREAT|O_EXCL` sentinel was rejected).
- **Read-only commands take no lock** (`status`, `log`, `diff`, …). They
  still serialize against writers via redb's own file lock; that
  contention surfaces as a friendly "store is in use" message from
  `Store::open`.
- The lock file contains the holder's PID for diagnostics only — it is
  never read for correctness.

## Which commands lock

See `command_mutates()` in `cli/commands.rs`: gather, seal, reseal,
discard, reverse, rewind, undo, pluck, fuse, travel, weld, harvest, sync, upload,
exclude, and the mutating timeline/review subcommands. `download` is
excluded because it may target a fresh clone outside any repository.
