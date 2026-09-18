# Snapshot Module (`snapshot.rs`) and `ivaldi oops`

Automatic workspace snapshots: the undo for everything that is not a seal yet.

## Overview

Seals are permanent. The work *between* seals is not, and several commands
rewrite the working directory: `fuse`, `sync`, `reverse --all`, `rewind`.
Before any of them touches a file, it records a snapshot of everything that is
not in a seal, so one command puts it all back:

```
$ ivaldi reverse --all
Reversed all changes. Working directory restored to last seal.
Changed your mind? 'ivaldi oops' brings them back.

$ ivaldi oops
Restored: before 'reverse --all' on main — 3 uncommitted change(s), just now
Run 'ivaldi oops' again to redo.
```

There is nothing to remember to do beforehand. That is the difference from a
reflog, which only knows about commits: uncommitted work overwritten by
`git reset --hard` or `git checkout -- .` was never recorded anywhere.

## What a snapshot holds

The same state a [shelf](shelf.md) holds, plus where the timeline stood —
because the commands it guards move the head as well as the files:

| Field | Meaning |
|-------|---------|
| `command` | What it was taken for (`fuse main`, `sync`), shown by `oops` |
| `timeline`, `head` | Current timeline and its head seal at the time |
| `staged`, `staged_deletion` | The gathered set |
| `modified`, `untracked`, `deleted` | Working-tree changes against the tree at `head` |

File content is hashed into the CAS, which is content-addressed and shared
with every seal, so the snapshot itself is a few lines of text. Unchanged files
cost one `stat` each (the same stat cache `status` uses); only changed files
are read.

## Storage

`.ivaldi/snapshots/<id>.snap`, highest id newest, pruned to the most recent
`KEEP` (20):

```
command fuse main
timeline gaming_changes
created_at 1789749439
head 412
modified <blake3_hash> crates/comp/huginn-comp/src/state.rs
untracked <blake3_hash> notes.txt
deleted old.rs
end
```

The trailing `end` is checked on load. A snapshot is restored *over* the
user's files, so one that is truncated or has a line that cannot be read is
refused rather than half-applied.

## `ivaldi oops`

| Form | Effect |
|------|--------|
| `ivaldi oops` | Restore the latest snapshot: timeline head, files, and gathered set |
| `ivaldi oops <id>` | Restore a specific snapshot |
| `ivaldi oops --list` | Show what can be restored |

- **It is its own inverse.** The state being replaced is snapshotted in turn,
  so `oops` again redoes, and nothing `oops` overwrites is lost either.
- **Moving the head back orphans seals, it does not delete them.** History is
  append-only; an undone merge seal stays reachable by name and via
  `travel --all`, and redo puts the head back on it.
- **During a fuse left open by `--markers`**, a bare `oops` is `fuse --abort`: no seal was
  made, so there is nothing to redo. Any other snapshot is refused until the
  fuse is settled.
- Snapshots belong to a timeline. Restoring one from another timeline is
  refused with the `timeline switch` to run first. (Switching itself needs no
  snapshot — auto-shelving already keeps that work.)

## Crash safety

`capture` flushes the CAS before returning, so the content is durable before
the calling command overwrites a byte. `restore` is idempotent and does not
consume its snapshot; `oops` records the redo snapshot and removes the target
only after the restore succeeds. An `oops` killed at any point
(`oops.after_head`, `oops.after_materialize`, `oops.before_redo_save`)
converges when run again — covered in `tests/crash_matrix_ops.rs`.

## API

```rust
use ivaldi::snapshot::{self, SnapshotManager};

// On the way into a command that rewrites the working directory:
snapshot::take(&repo, &repo.cas, "sync")?;

// Or capture first and decide later whether it is worth recording:
let snap = snapshot::capture(&repo, &repo.cas, "reverse --all")?;
if !snap.is_clean() {
    SnapshotManager::new(&repo.ivaldi_dir).save(&snap)?;
}

snapshot::restore(&repo, &repo.cas, &snap)?;
```

Any new command that materializes a tree over the working directory should
call `take` first.
