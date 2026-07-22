# Rescue Module (`rescue.rs`)

Raw file recovery from a damaged repository, exposed as `ivaldi rescue`.

## Overview

`rescue` gets your files back when a repository is too broken to open normally.
It reads content directly and **never** goes through `Repo::open` — it does not
trust HEAD, refs, the MMR, or redb, because those are exactly what may be
damaged. Every object is verified against its own hash before use, so recovered
content is guaranteed intact; anything corrupt, missing, or unsafe is skipped
and reported, never faked.

## How it works

Two independent sources, either of which may be broken:

1. **The commit store** (`store.db`) supplies commit leaves → tree roots, with
   author and message. Best metadata, but redb may be unreadable.
2. **The CAS** (`objects/`) supplies the blob and tree content to materialize.

Steps:

1. Load every object that hashes to its own name (corrupt ones are excluded).
2. Read commit leaves from the store, if it is readable, for their tree roots.
3. Materialize each distinct snapshot into `<out>/<tree-short-hash>/`.
4. **Orphan sweep:** materialize any tree in the CAS not reached from a commit
   into `<out>/orphans/`. This is what recovers files when the store is dead.
5. Write a `MANIFEST.txt` mapping snapshots to their metadata.

The repository is located leniently — by the presence of `.ivaldi/objects`, not
HEAD — so a repository with wiped refs is still found. `rescue` never opens a
missing `store.db` (redb would create a fresh one inside the repo it is trying
to rescue).

## Usage

```bash
ivaldi rescue                        # writes to ./ivaldi-rescue
ivaldi rescue --out /tmp/recovered
ivaldi rescue --json                 # machine-readable report
```

## Safety

- **Hash-verified content only.** A tampered object never reaches disk.
- **Path-traversal safe.** A tree entry name must be a single safe path
  component; `..`, `/`, and absolute-style names are rejected, so a corrupt
  tree cannot escape the output directory.
- **Bounded recursion.** Tree depth is capped, so a pathological or cyclic-
  looking tree cannot overflow the stack.

Related: [verify.md](verify.md) (detects damage), the `doctor` command (points
you here), [fsmerkle.md](fsmerkle.md) (tree/blob format), [leaf.md](leaf.md).
