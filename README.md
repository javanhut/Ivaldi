# Ivaldi VCS

A modern version control system built in Rust, designed to replace Git — not copy it.

## Project identity

Ivaldi is a standalone version control system implemented from scratch in
Rust. It is not a Git wrapper, Git frontend, or reimplementation of Git's
internal architecture. Its native system has its own BLAKE3 content-addressed
object store, transactional `redb` database, append-only Merkle Mountain Range
history, Merkle filesystem with HAMT-backed large directories, timeline and
shelving model, and authenticated peer-to-peer protocol.

Git interoperability is an optional compatibility bridge at the boundary. It
lets people migrate existing work and communicate with Git hosting services;
Git does not participate in Ivaldi's native storage, history, timelines,
fusion, recovery, or `ivaldi://` synchronization. Ivaldi does not invoke Git or
embed libgit2 to perform native VCS operations.

## The Ivaldi contract

Ivaldi should be evaluated against the guarantees of its own design:

1. Acknowledged work remains durable and recoverable.
2. History is append-only and integrity-verifiable.
3. Timeline switching preserves dirty work automatically.
4. Interrupted mutations leave the old state, the new state, or an explicitly
   recoverable state—never silently accepted partial state.
5. Native synchronization authenticates peers and verifies received history
   and content before making it authoritative.
6. Corrupt or hostile input fails closed, while recovery preserves evidence
   and every recoverable byte.

Git feature parity is not an Ivaldi design goal. Git compatibility is evaluated
separately as a migration and hosting bridge.

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
- **Skip files without touching the ignore file.** `ivaldi skip <path>`
  temporarily excludes a file (a regenerated lockfile, test or debug output)
  from staging — and therefore from seals and pushes — until
  `ivaldi unskip`. The list is repo-local and never committed.
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

## Install

### Prebuilt releases

Prebuilt, signed binaries are attached to each
[GitHub release](https://github.com/javanhut/ivaldi/releases) for Linux,
macOS, and Windows on both `x86_64` and `arm64`. See
[Verifying releases](#verifying-releases) for how to check them.

### From source

Prerequisites: a Rust toolchain (rust 1.89+, edition 2024) — install via
[rustup.rs](https://rustup.rs).

```bash
git clone https://github.com/javanhut/ivaldi.git
cd ivaldi
```

The repo ships two equivalent task setups — a `Makefile` and a `lazy.toml`
for the [imlazy](https://github.com/javanhut/ImLazy) task runner. Use
whichever you have:

```bash
# With make
make build
sudo make install                  # installs to /usr/local/bin
make install PREFIX=~/.local       # or a custom prefix, no sudo
sudo make install-extras           # optional: man pages + bash/zsh/fish completions

# With imlazy (same targets; `build` is the default)
imlazy                             # = imlazy build
sudo imlazy install
imlazy install prefix=~/.local
sudo imlazy install-extras
```

Verify with:

```bash
ivaldi forge        # initialize your first repository
```

## Verifying releases

Every release archive is signed, and each release ships a single
`SHA256SUMS` file covering all artifacts.

Signing is keyless via [Sigstore](https://www.sigstore.dev/) — there is no
long-lived public key to trust. Instead you verify that a signature was
produced by Ivaldi's own release workflow. Install
[`cosign`](https://docs.sigstore.dev/cosign/system_config/installation/), then:

```bash
# 1. Verify the checksums file was signed by Ivaldi's release workflow.
cosign verify-blob \
  --bundle SHA256SUMS.cosign.bundle \
  --certificate-identity-regexp 'https://github.com/javanhut/ivaldi/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS

# 2. Check your downloaded archive against the trusted checksums.
sha256sum --check --ignore-missing SHA256SUMS
```

The first command fails if the checksums file was not produced by this
repository's release workflow; the second fails if your download does not match.
Both must pass. To verify an individual archive directly, use its own
`.cosign.bundle` with the same `cosign verify-blob` invocation.

## Command overview

The vocabulary is different on purpose — names match the action, not git
tradition. Common aliases are shown in parentheses.

**Daily work**

| Command | Alias | Description |
|---------|-------|-------------|
| `ivaldi forge` | `init` | Initialize a repository |
| `ivaldi gather [files] [-p]` | `add` | Stage files for the next seal (`-p` picks hunks interactively) |
| `ivaldi seal "msg"` | `se` | Create a sealed commit |
| `ivaldi reseal [msg]` | `rs` | Redo the most recent seal, folding in staged changes |
| `ivaldi status [--json]` | `st` | Show repository status |
| `ivaldi whereami` | `wai` | Show current timeline and position |
| `ivaldi log` | `lg` | View commit history |
| `ivaldi whodidit <file>` | `wdi` | Line-by-line seal attribution |
| `ivaldi diff` | `df` | Compare changes |
| `ivaldi discard [files]` | `dc` | Unstage files (none = everything) |
| `ivaldi skip <paths>` / `ivaldi skip --list` | | Temporarily exclude paths from staging (repo-local) |
| `ivaldi unskip <paths>` | | Stop excluding paths from staging |
| `ivaldi exclude <patterns>` | `ex` | Add patterns to `.ivaldiignore` |
| `ivaldi config` | `cf` | View/modify settings (bare `config` opens an interactive form) |
| `ivaldi tui` | `ui` | Open the interactive TUI dashboard |

**Time travel**

| Command | Alias | Description |
|---------|-------|-------------|
| `ivaldi undo <seal>` | `ud` | New seal that removes an earlier seal's changes |
| `ivaldi pluck <seal>` | `cherry-pick` | New seal that applies another seal's changes |
| `ivaldi rewind <seal> [--discard]` | `rw` | Move the timeline head back to an earlier seal |
| `ivaldi reverse --all` | | Throw away all uncommitted changes (destructive!) |
| `ivaldi travel [--all]` | `tv` | Interactive history browser |
| `ivaldi weld --last N` | `w` | Combine a range of seals into one (linear history) |

**Timelines (branches)**

| Command | Alias | Description |
|---------|-------|-------------|
| `ivaldi timeline create/switch/list/rename/remove` | `tl` | Manage timelines; dirty work shelves automatically on switch |
| `ivaldi timeline butterfly create/up/down/rm` | `tl bf` | Experimental sandbox timelines |
| `ivaldi fuse <src> to <tgt>` | `fu` | Merge timelines (no conflict markers — strategy selection) |

**Sharing**

| Command | Alias | Description |
|---------|-------|-------------|
| `ivaldi portal add/list/remove/set-default` | `pt` | Manage remotes (HTTPS / SSH / `ivaldi://`) |
| `ivaldi auth login/status/logout` | `au` | OAuth for GitHub/GitLab (device flow) |
| `ivaldi download <url>` | `dl` | Clone (transport auto-detected from URL) |
| `ivaldi upload [--portal P]` | `up` | Push to the default or named portal |
| `ivaldi scout` | `sc` | Discover remote timelines |
| `ivaldi harvest <name>` | `hv` | Fetch specific remote timelines |
| `ivaldi sync [branch]` | `sy` | Pull remote changes, delta only |
| `ivaldi serve` | `sv` | Serve the repo to authorized peers over `ivaldi://` |
| `ivaldi peer trust/list/forget` | `pr` | Manage peer pubkey allowlists |

**Review and repository care**

| Command | Alias | Description |
|---------|-------|-------------|
| `ivaldi review create/list/show/diff/comment/approve/merge/close` | `rv` | Local code review system |
| `ivaldi verify [--full]` | | Check repository integrity |
| `ivaldi prove <seal>` | | Emit or verify an MMR inclusion receipt (no git equivalent) |
| `ivaldi rescue [--out dir]` | | Recover files from a damaged repository |
| `ivaldi recover [--dry-run]` | | Safely repair a repository in place (never discards data) |
| `ivaldi doctor [--json]` | | Diagnose a repository and print recovery guidance |
| `ivaldi migrate` | | Upgrade an older repository format (verified rollback snapshot) |
| `ivaldi completions <shell>` | `cmp` | Print a shell completion script |

The full reference with every flag is in [`docs/cli.md`](docs/cli.md), and
`ivaldi <command> --help` is always authoritative.

## Migration guide for Git users

| | Git | Ivaldi |
|---|-----|--------|
| Commit names | `a1b2c3d` | `swift-eagle-flies-high` |
| Hashing | SHA-1 (deprecated) | BLAKE3 (10× faster) |
| History model | Mutable refs, rewritable | Append-only Merkle Mountain Range |
| Undoing a commit | `revert` / `reset` (destructive variants) | `undo` / `rewind` — old seals always recoverable |
| Stashing | Manual `git stash` | Automatic on timeline switch |
| Merge conflicts | Markers in files | Clean workspace, strategy selection |
| Skip a file temporarily | `update-index --skip-worktree` | `skip` / `unskip` |
| Clone | All branches | Selective (`scout` + `harvest`) |
| Peer-to-peer | Not built in | `ivaldi serve` + `ivaldi://` transport |

| Ivaldi | Git equivalent |
|--------|---------------|
| Forge | Init |
| Timeline | Branch |
| Seal | Commit |
| Gather | Add / Stage |
| Discard | Unstage |
| Fuse | Merge |
| Portal | Remote |
| Upload / Download | Push / Clone |
| Scout / Harvest | Fetch (metadata / data) |
| Shelf | Stash (automatic) |
| Skip / Unskip | Skip-worktree |
| Pluck | Cherry-pick |
| Reseal | Commit --amend |
| Weld | Squash a range |
| Whodidit | Blame |
| Butterfly | Experimental sandbox branch |

> **Coming from git?** [`docs/rosetta.md`](docs/rosetta.md) is the full
> translation table — every git command you reach for daily, mapped to its
> Ivaldi equivalent.

## Correctness evidence

Ivaldi's safety properties are exercised at several levels:

- unit tests for the CAS, database, MMR, HAMT, filesystem, timelines, fusion,
  recovery, and native protocol;
- property tests for canonical HAMT construction and randomized operations;
- adversarial tests for corrupt objects, packs, trees, refs, and protocol data;
- real multi-process writer races;
- deterministic process-abort tests at mutation boundaries, followed by
  reopen, full verification, safe retry, and idempotent recovery checks;
- end-to-end native fetch and push over localhost, including authentication,
  parent remapping, chunked blobs, interruption cleanup, and repeated transfer;
- repository rescue and recovery after metadata and object corruption; and
- fuzz targets for native parsers and compatibility-boundary formats.

Run the complete suite, including deterministic crash injection, with:

```bash
cargo test --locked --all-targets --all-features
```

CI runs formatting, Clippy with warnings denied, and the all-features suite on
Linux, macOS, and Windows. See the
[test-evidence matrix](docs/test-matrix.md) for the guarantees exercised by
each layer.

## Maturity and versioning

Ivaldi is an implemented and comprehensively tested standalone VCS. Its `0.x`
version does not mean prototype, proof of concept, or Git-dependent frontend.
It means that every CLI, repository-format, and network-protocol contract has
not yet been frozen for long-term 1.0 compatibility.

The [1.0 certification plan](plan.md) defines additional evidence, historical
upgrade commitments, operating limits, release procedures, and independent
review required before that contract is frozen. An unchecked certification
item does not, by itself, mean the corresponding native feature is absent or
untested; its stated evidence and acceptance criteria define what remains.

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
- **Persistent HAMT directories** — format-2 repositories store directories
  with more than 256 entries as canonical CAS-backed HAMTs; native transfer,
  structural diff, verification, and rescue understand their interior nodes

See [`docs/hamt.md`](docs/hamt.md) for the encoding, format gate, validation,
property tests, and performance characteristics.

## License

Ivaldi is licensed under the **GNU Affero General Public License v3.0**
([AGPL-3.0-only](LICENSE)).

You are free to use, study, and modify Ivaldi. The share-alike obligation is
strong: if you distribute a modified version **or run one as a network
service**, you must make your complete source code available to your users
under the same license. Running Ivaldi unmodified, for yourself, carries no
such obligation.
