# Carry Module (`carry.rs`): fusing with uncommitted work

## Overview

A fuse merges *seals*. Work that is not sealed yet has no side in that merge,
and the fused tree written over the working directory would erase it. Ivaldi
neither erases it nor refuses and hands the chore back ("commit or stash your
changes"): it carries the work through.

```
$ ivaldi fuse main
Carrying 6 uncommitted change(s) through the fuse...
[OK] Merge completed successfully!
  Merge seal: simple-ocean-watches-gentle (8e3f60d6)
[OK] Re-applied your uncommitted changes: 4 clean, 2 need a look
       crates/comp/huginn-comp/src/backend/input.rs  (conflict markers written)
       crates/comp/huginn-comp/src/render.rs  (conflict markers written)
  Not what you wanted? 'ivaldi oops' puts everything back as it was before the fuse.
```

## How it works

1. **Set aside.** The uncommitted state is captured as a
   [snapshot](snapshot.md) — recorded for `ivaldi oops` and also parked in
   `.ivaldi/fuse-carry.snap` — and the working directory is put back to the
   timeline's tip. The fuse gets a clean tree, which matters when it conflicts:
   markers are written into, and `--continue` reads resolutions from, files
   that contain nothing but the two sealed sides.
2. **Fuse**, exactly as on a clean tree.
3. **Re-apply.** Each set-aside change was an edit *of the old tip*, so it is
   merged three ways: old tip as base, the user's version as one side, the
   fused file as the other.

The result is left uncommitted, as it was.

| The fuse… | The user… | Outcome |
|-----------|-----------|---------|
| did not touch the file | changed / added / deleted it | Change drops straight back in |
| changed it | changed other lines | Both sets of edits, merged line by line |
| changed it | changed the same lines | Conflict markers: `your uncommitted changes` vs `fused from <source>` |
| made the identical change | — | Nothing left to carry |
| deleted it | changed it | User's version kept (now a new file), reported |
| changed it | deleted it | Fused version kept, reported |
| changed a binary | changed it too | Fused version kept, reported; the user's is in the snapshot (`ivaldi oops`) |

Gathered entries are un-gathered, and said so. They name blobs made against
the old tip; sealed after the fuse they would silently write those blobs over
whatever the fuse brought in for the same paths. Their content is carried like
any other edit.

## When the fuse itself conflicts

The set-aside work stays parked while the merge is open:

- `ivaldi fuse --continue` seals the merge, then re-applies it on top.
- `ivaldi fuse --abort` (or a bare `ivaldi oops`) puts it back untouched, and
  rewrites the conflict-marked files back to the tip.

## Crash safety

The carry file is written, and the CAS flushed, before the first working-tree
byte is overwritten. A carry file with no merge in progress means a fuse died
still owing the work back; the next `fuse` settles it first:

- head unmoved (`fuse.after_carry_save`) → the merge never happened; the work
  is restored as it was, then the fuse proceeds normally;
- head moved (`fuse.before_reapply`) → the merge seal landed; the work is
  re-applied on top of it.

Either way the working directory is snapshotted before recovery rewrites it,
so edits made since the crash are not lost. `ivaldi oops` is an equally valid
way out of both windows. Covered in `tests/crash_matrix_ops.rs`.
