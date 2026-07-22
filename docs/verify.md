# Verify Module (`verify.rs`)

Repository integrity checking, exposed as `ivaldi verify`.

## Overview

`verify` answers "is this repository intact?" without modifying anything. It
never panics on a broken repository — problems are reported as failed checks so
a caller (or `ivaldi doctor`) always gets a diagnosis.

## Checks

| Check | Fast | `--full` | What it validates |
|-------|:----:|:--------:|-------------------|
| `format` | ✓ | ✓ | `.ivaldi/FORMAT` is readable and not newer than this binary supports |
| `structure` | ✓ | ✓ | MMR leaf-index sequence, size/root checkpoints, every leaf parses, rebuilt root matches |
| `cas-objects` | | ✓ | every stored object re-hashes to its own address |

The fast (default) check reuses `Repo::open`, which already performs the
structural validation. `--full` adds the content pass — re-hashing every CAS
object — which is the one integrity property `FileCas::get` does not check on
read. It is `O(total repository size)`.

## Usage

```bash
ivaldi verify              # fast structural check
ivaldi verify --full       # + re-hash every object
ivaldi verify --full --json
```

`--json` emits the full report. The process exits non-zero if any check fails,
so `verify` works directly in CI or a cron health check.

## Design

- **Read-only and panic-free.** A hostile or corrupt repository yields failed
  checks, not a crash. `verify()` returns a `Report`, never an `Err`.
- **Reuse over reimplementation.** Structural validation is `Repo::open`, not a
  parallel copy that could drift from the real open path.
- **Extensible.** Each check is one entry appended to the report; deeper
  `--full` checks (refs, seal mappings, shelves, journals) slot in the same way.

Related: [rescue.md](rescue.md) (recovery when verify fails), the `doctor`
command (guidance layered over verify).
