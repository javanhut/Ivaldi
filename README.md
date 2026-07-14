# Ivaldi VCS

A modern version control system built in Rust, designed to replace Git — not copy it.

## Motivation

Git won by being distributed, fast, and ubiquitous. But it carries twenty
years of accumulated sharp edges that every developer has cut themselves on:

- **Hashes are hostile.** `a1b2c3d` tells you nothing. You copy-paste hashes
  around because no human can remember them.
- **Losing work is too easy.** A forgotten `git stash`, a `reset --hard` at
  the wrong moment, a force-push over a colleague's commits — git's most
  destructive operations are some of its shortest commands.
- **Conflict markers vandalize your files.** A failed merge leaves
  `<<<<<<<` debris in your working tree and your build broken until you
  clean it up by hand.
- **The mental model leaks.** Index vs. working tree vs. HEAD, detached
  heads, the difference between `reset --soft/--mixed/--hard` — you're
  forced to learn git's internals just to use it safely.
- **A third party sits in the middle.** Sharing code with the person across
  the desk usually means routing through a hosting service.

Ivaldi is a from-scratch answer to those problems — not a porcelain layer
over git, but a different design with its own storage engine, history
structure, and vocabulary.

## What Ivaldi is

Ivaldi is a complete VCS with a workflow that maps to what you're actually
trying to do:

- **Every commit gets a memorable name.** Seals are named like
  `swift-eagle-flies-high-447abe9b` — you refer to history by name, not hash.
- **History is never rewritten.** The commit history is an append-only
  Merkle Mountain Range. "Undo" means creating a new seal that reverses an
  old one; the old seal stays recoverable forever. There is no
  force-push-shaped footgun.
- **Your work is never silently lost.** Switching timelines (branches)
  auto-shelves uncommitted changes and restores them when you switch back —
  no manual stashing, no "please commit or stash" errors.
- **Merges never leave markers in your files.** When a merge can't resolve
  automatically, you choose a strategy (`theirs`, `ours`, `union`, `base`)
  instead of hand-editing conflict debris.
- **BLAKE3 everywhere.** All hashing is BLAKE3 — roughly 10× faster than
  SHA-256 and cryptographically secure, where git is still migrating off
  deprecated SHA-1.
- **Three transports, zero flags.** GitHub/GitLab over HTTPS, any git host
  over SSH, or direct peer-to-peer over `ivaldi://` — the same `upload` /
  `download` / `sync` commands, with the transport picked automatically
  from the URL.
- **Git interoperability without lock-in.** Ivaldi speaks git's wire
  protocols to existing hosts. Round-tripping a git repo through Ivaldi
  (download → upload) preserves commit SHA-1s byte-for-byte — author,
  committer, and timezone included — so your collaborators on plain git
  never know the difference.
- **Peer-to-peer built in.** Two machines can exchange code directly over
  an encrypted, mutually authenticated channel (Noise XX, ed25519) — no
  hosting service required.

## How it differs from Git

| | Git | Ivaldi |
|---|-----|--------|
| Commit names | `a1b2c3d` | `swift-eagle-flies-high` |
| Hashing | SHA-1 (deprecated) | BLAKE3 (10× faster) |
| History model | Mutable refs, rewritable | Append-only Merkle Mountain Range |
| Undoing a commit | `revert` / `reset` (destructive variants) | `undo` / `rewind` — old seals always recoverable |
| Stashing | Manual `git stash` | Automatic on timeline switch |
| Merge conflicts | Markers in files | Clean workspace, strategy selection |
| Clone | All branches | Selective (`scout` + `harvest`) |
| Directories | Canonical Merkle trees | Content-addressed Merkle trees |
| Peer-to-peer | Not built in | `ivaldi serve` + `ivaldi://` transport |

The vocabulary is different on purpose — names match the action, not git
tradition:

| Ivaldi | Git equivalent |
|--------|---------------|
| Forge | Init |
| Timeline | Branch |
| Seal | Commit |
| Gather | Add / Stage |
| Fuse | Merge |
| Portal | Remote |
| Upload / Download | Push / Clone |
| Scout / Harvest | Fetch (metadata / data) |
| Shelf | Stash (automatic) |
| Pluck | Cherry-pick |
| Weld | Squash a range |
| Whodidit | Blame |
| Butterfly | Experimental sandbox branch |

> **Coming from git?** [`docs/rosetta.md`](docs/rosetta.md) is the full
> translation table — every git command you reach for daily, mapped to its
> Ivaldi equivalent.

## Install

Prerequisites: a Rust toolchain (edition 2024) — install via [rustup.rs](https://rustup.rs).

```bash
git clone https://github.com/javanhut/ivaldi.git
cd ivaldi

make build
sudo make install                  # installs to /usr/local/bin

# Or without sudo, to a custom prefix
make install PREFIX=~/.local

# Optional: man pages + bash/zsh/fish completions
sudo make install-extras
```

Verify with:

```bash
ivaldi forge        # initialize your first repository
```

## Learn Ivaldi

- **[Quick Start guide](docs/quick-start.md)** — a hands-on walkthrough of
  every function: creating a repo, the daily gather/seal workflow, going
  back in time, timelines, merging, remotes, authentication, and
  peer-to-peer sharing, with links to the deep-dive doc for each.
- **[Rosetta Stone](docs/rosetta.md)** — git-to-Ivaldi command translation.
- **[CLI reference](docs/cli.md)** — every command with full flags.
- **[`docs/`](docs/)** — design docs for each subsystem: storage
  ([cas.md](docs/cas.md), [store.md](docs/store.md)), history
  ([mmr.md](docs/mmr.md), [seal.md](docs/seal.md)), merging
  ([fsmerkle.md](docs/fsmerkle.md)), networking ([p2p.md](docs/p2p.md),
  [remote.md](docs/remote.md)), and more.

## Under the hood

- **BLAKE3** — all hashing (files, trees, commits, proofs)
- **Merkle Mountain Range** — append-only commit history with inclusion proofs
- **Content-addressed Merkle trees** — unchanged directory subtrees retain
  the same hashes and are reused across seals
- **Content-addressable storage** — deduplication across timelines, 64KB
  file chunking for large files
- **redb** — pure-Rust, ACID, crash-safe embedded database for commits,
  timelines, and seal names

An in-memory HAMT prototype exists as a possible future backend for very large
directories, but it is not part of the current repository storage path. See
[`docs/hamt.md`](docs/hamt.md) for its status and the criteria for integrating
it.

## License

No license has been granted yet. Until a license is added, the source remains
copyrighted by its contributors and is not available for redistribution under
an open-source license.
