# Pick Module (`pick.rs`)

Shared three-way "apply a delta as a new seal" engine behind `undo` and
`pluck`.

## Overview

Both commands are the same operation with different inputs: run the fuse
engine with ours = the current head tree and a (base, theirs) pair that
encodes the delta, then seal the merged tree as a plain (non-merge) seal.

| Command | base              | theirs            |
|---------|-------------------|-------------------|
| undo    | the seal's tree   | its parent's tree |
| pluck   | its parent's tree | the seal's tree   |

```rust
pub enum ApplyOutcome {
    Applied(CommitResult),   // new seal created
    Conflicts(Vec<String>),  // refused; conflicting paths listed
    NoChanges,               // merged tree == head tree; nothing sealed
}

pub fn three_way_seal(repo, cas, base, theirs, author, message)
    -> Result<ApplyOutcome, String>
```

## Behavior

- The fuse engine works at file-hash granularity (no line-level merging),
  so any file touched by both the delta and other history conflicts. On
  conflict the operation **refuses and commits nothing** — the working
  tree is untouched.
- `NoChanges` covers plucking a seal whose changes are already in the
  head.
- `tree_files(store, Option<root>)` builds the `path → blob hash` map for
  a tree; `None` yields an empty map (the parent of a timeline's first
  seal), which is how undoing a first seal deletes its files.

User-facing documentation: [undo.md](undo.md).
