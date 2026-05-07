# Ivaldi VCS

A modern Version Control System built in Rust, designed to replace Git — not copy it.

Ivaldi uses BLAKE3 hashing, Merkle Mountain Ranges, and human-readable seal names to provide a faster, safer, and more intuitive version control experience.

## Features

- **BLAKE3 Hashing** — 10x faster than SHA-256, cryptographically secure
- **Seal Names** — Every commit gets a memorable name like `swift-eagle-flies-high-447abe9b`
- **Auto-Shelving** — Uncommitted changes are automatically saved when switching timelines
- **Clean Merges** — No conflict markers in your files, ever
- **Butterfly Timelines** — Experimental sandboxes with bidirectional parent sync
- **Selective Sync** — Download only the branches you need
- **Three Transports** — GitHub/GitLab over HTTPS, any git host over SSH, or peer-to-peer over `ivaldi://` — same commands, picked automatically from the portal URL
- **Bidirectional Git Fidelity** — Round-tripping a git repo through Ivaldi (download → upload) preserves commit SHA-1s byte-for-byte, including author, committer, and timezone

## Quick Start

```bash
# Install
make install

# Initialize a repository
ivaldi forge

# Configure identity
ivaldi config --set user.name "Your Name"
ivaldi config --set user.email "you@example.com"

# Daily workflow
ivaldi gather .                    # Stage all files
ivaldi seal "Add new feature"      # Commit
ivaldi status                      # Check workspace state
ivaldi log --oneline               # View history

# Timelines (branches)
ivaldi timeline create feature     # Create timeline
ivaldi timeline switch feature     # Switch (auto-shelves changes)
ivaldi timeline list               # List all timelines
ivaldi fuse feature to main        # Merge

# Remote operations — transport is picked automatically from the URL
ivaldi portal add owner/repo                          # GitHub HTTPS shorthand
ivaldi portal add git@host.example.com:team/repo.git  # SSH (any host: github, gitlab, gitea, …)
ivaldi portal add ivaldi://10.0.0.5:9418              # Peer-to-peer (no third-party host)

ivaldi auth login                  # GitHub OAuth (HTTPS upload/sync only)
ivaldi auth login --gitlab         # GitLab OAuth (HTTPS upload/sync only)
ivaldi upload                      # Push (auto-routes via SSH / GitHub REST / ivaldi://)
ivaldi download owner/repo         # Clone via HTTPS (no auth needed for public repos)
ivaldi download git@example.com:team/repo.git          # Clone via SSH
ivaldi download ivaldi://10.0.0.5:9418/main            # Clone via P2P
ivaldi scout                       # List remote branches
ivaldi harvest feature-branch      # Fetch specific branch
ivaldi sync                        # Pull remote changes (delta only)

# Peer-to-peer (no GitHub / GitLab in the loop)
ivaldi serve                       # Listen on tcp/9418, accept trusted peers
ivaldi peer whoami                 # Print this machine's pubkey
ivaldi peer trust <pubkey> alice   # Authorize a peer for inbound connections
ivaldi peer known list             # Servers we trust (TOFU known_peers)
```

## Command Reference

| Command | Alias | Description |
|---------|-------|-------------|
| `forge` | | Initialize repository |
| `gather [files]` | | Stage files for next seal |
| `seal "message"` | | Create a commit |
| `status` | | Show workspace state |
| `whereami` | `wai` | Show current position |
| `log` | | View commit history |
| `diff` | | Compare changes |
| `reset` | | Unstage files or hard reset |
| `timeline` | `tl` | Manage timelines (create/switch/list/rename/remove) |
| `butterfly` | `tl bf` | Experimental sandbox timelines |
| `fuse` | | Merge timelines (auto strategy uses MMR-based merge base) |
| `travel` | | Interactive history browser (full DAG walk; `--all` shows orphaned seals) |
| `weld` | `w` | Combine a range of seals into one (linear history) |
| `config` | | View/modify settings |
| `exclude` | | Add to .ivaldiignore |
| `portal` | | Manage remote connections |
| `auth` | | GitHub/GitLab authentication |
| `download` | | Clone a repository (HTTPS / SSH / `ivaldi://`) |
| `upload` | | Push to remote (HTTPS / SSH / `ivaldi://`) |
| `scout` | | Discover remote branches (HTTPS / SSH) |
| `harvest` | | Fetch specific branches (HTTPS / SSH) |
| `sync` | | Pull remote changes (delta only) |
| `serve` | | Run an `ivaldi://` peer server for trusted users |
| `peer` | | Manage trusted peers + known servers (`trust` / `list` / `forget` / `whoami` / `known`) |

## Ivaldi vs Git

| Feature | Git | Ivaldi |
|---------|-----|--------|
| Hashing | SHA-1 (deprecated) | BLAKE3 (10x faster) |
| Commit names | `a1b2c3d` | `swift-eagle-flies-high` |
| Stashing | Manual | Automatic on switch |
| Merge conflicts | Markers in files | Clean workspace, strategy selection |
| Clone | All branches | Selective (`scout` + `harvest`) |
| History | Merkle tree | Merkle Mountain Range |
| Directories | Flat tree | HAMT (structural sharing) |

## Terminology

| Ivaldi | Git Equivalent |
|--------|---------------|
| Timeline | Branch |
| Seal | Commit |
| Gather | Add / Stage |
| Forge | Init |
| Fuse | Merge |
| Portal | Remote |
| Upload | Push |
| Download | Clone |
| Scout | Fetch (metadata) |
| Harvest | Fetch (data) |
| Shelf | Stash (automatic) |
| Butterfly | Experimental branch |
| Travel | Interactive log + checkout |
| Weld | Rebase --squash (range collapse, linear history) |

## Architecture

```
.ivaldi/
├── objects/        # Content-addressable storage (BLAKE3, 2-char sharding)
├── refs/
│   ├── heads/      # Timeline references
│   ├── remotes/    # Remote refs
│   ├── seals/      # Seal name mappings
│   └── tags/       # Tag references
├── shelves/        # Auto-shelving per timeline
├── butterflies/    # Butterfly metadata
├── hooks/          # Pre/post operation scripts
├── stage/          # Staging area
├── store.db        # Persistent storage (redb — ACID, crash-safe)
├── config          # Repository configuration
└── HEAD            # Current timeline pointer
```

### Core Data Structures

- **BLAKE3** — All hashing (files, trees, commits, proofs)
- **Merkle Mountain Range** — Append-only commit history with inclusion proofs
- **HAMT** — Immutable directory trees with structural sharing
- **Content-Addressable Storage** — Deduplication across timelines
- **64KB File Chunking** — Efficient large file handling

### Storage

- **redb** — Pure Rust, ACID, crash-safe embedded database for commits, timelines, seal names
- **File CAS** — 2-character sharded object storage for file content
- **Pack files** — Combine small objects for efficient storage

## Build

```bash
# Prerequisites: Rust toolchain (edition 2024)
# https://rustup.rs

# Build
make build

# Run tests (348 tests)
make test

# Install to /usr/local/bin
sudo make install

# Install to custom location
make install PREFIX=~/.local

# Uninstall
sudo make uninstall

# Clean
make clean
```

## Configuration

```bash
# Required — set globally once, works across all repos
ivaldi config --global --set user.name "Your Name"
ivaldi config --global --set user.email "you@example.com"

# Inside a repo, --set writes to the repo-local config by default
ivaldi config --set color.ui true
ivaldi config --set core.autoshelf true

# View all (annotated with global / local / default provenance)
ivaldi config --list

# Interactive form (ratatui — works inside or outside a repo)
ivaldi config
```

`ivaldi config` runs fine outside a repo — it automatically targets the
global config at `~/.ivaldi/config`.

Config files:
- User: `~/.ivaldi/config`
- Repository: `.ivaldi/config` (overrides user)

## Transports

Ivaldi picks the right transport automatically from the URL on each
command — there is no `--protocol` flag. The same `download` / `upload` /
`scout` / `harvest` / `sync` invocations work for all three:

| URL form | Transport | What runs |
|---|---|---|
| `owner/repo` · `https://github.com/owner/repo.git` · `github:owner/repo` | HTTPS | GitHub Smart-HTTP + REST API |
| `git@host:owner/repo.git` · `ssh://git@host:port/owner/repo.git` | SSH | `ssh ... git-upload-pack/git-receive-pack`, identical wire to plain git |
| `ivaldi://host:port[/timeline]` | Peer-to-peer | Ivaldi-native protocol over Noise XX (mutual ed25519 auth, ChaCha20-Poly1305) |

**HTTPS** uses GitHub/GitLab's REST APIs for write paths (creates commits via
`/git/commits` so author/committer round-trip exactly). **SSH** spawns the
system `ssh` binary as a subprocess — your existing keys, agent, and
`known_hosts` keep working — and speaks the standard git pack protocol.
**P2P** lets two people share code with zero third party: see [`docs/p2p.md`](docs/p2p.md).

Round-tripping a git repo through Ivaldi (download → upload) preserves
commit SHA-1s byte-for-byte. Confirmed live against `octocat/Hello-World` —
the merge commit `7fd1a60b…` lands at the exact same hash on the other side.

## Authentication

Authentication is **optional** for read-only operations on public
repositories — `download`, `scout`, and `harvest` work anonymously. Only
HTTPS `upload` / `sync` require a token. SSH uses your local SSH agent /
keys (no `auth login` needed). P2P uses the user's long-lived ed25519
identity at `~/.ivaldi/identity` (auto-generated on first need).

```bash
# GitHub OAuth (HTTPS only — works like gh auth login)
ivaldi auth login

# GitLab OAuth — supports gitlab.com and self-hosted
ivaldi auth login --gitlab
ivaldi auth login --gitlab --gitlab-host https://gitlab.example.com

# Environment variables
export GITHUB_TOKEN=ghp_...
export GITLAB_TOKEN=glpat-...

# Check / logout
ivaldi auth status
ivaldi auth logout
ivaldi auth logout --gitlab
```

For peer-to-peer auth see [`docs/p2p.md`](docs/p2p.md) and
[`docs/identity.md`](docs/identity.md).

## Merge Strategies

```bash
ivaldi fuse feature to main                    # Auto (default)
ivaldi fuse --strategy=theirs feature to main  # Accept all source changes
ivaldi fuse --strategy=ours feature to main    # Keep all target changes
ivaldi fuse --strategy=union feature to main   # Combine both
ivaldi fuse --strategy=base feature to main    # Revert to ancestor
ivaldi fuse --abort                            # Cancel merge
ivaldi fuse --continue                         # Resume after conflict resolution
```

## Peer-to-Peer (`ivaldi://`)

Two users can exchange code directly over TCP — no GitHub, no GitLab,
no third party. Each side has a long-lived ed25519 identity at
`~/.ivaldi/identity` (auto-minted on first use); the serving side
maintains an `authorized_peers` allowlist (per repo). Connections are
encrypted + mutually authenticated with the Noise XX handshake.

```bash
# On Alice's machine (the serving side)
ivaldi peer whoami                                 # prints alice's pubkey
ivaldi peer trust <bob-pubkey> bob                 # whitelist bob
ivaldi serve --bind 0.0.0.0:9418                   # blocks; Ctrl-C to stop

# On Bob's machine
ivaldi download ivaldi://alice.example.com:9418/main bob-clone   # clone alice's main
echo edit >> file.txt && ivaldi gather . && ivaldi seal -m "edit"
ivaldi portal add ivaldi://alice.example.com:9418  # so `upload` knows where to go
ivaldi upload                                      # push back to alice

# On Alice's machine — bob's seals land at peers/bob/main; alice fuses
# manually rather than alice's `main` advancing under her feet
ivaldi timeline list             # peers/bob/main visible
ivaldi fuse peers/bob/main       # integrate
```

First connection prompts a TOFU fingerprint check (mirrors `~/.ssh/known_hosts`);
use `--accept-new-peer` for non-interactive scripts or `--strict-peer` to
refuse anything not already in `~/.ivaldi/known_peers`.

Deep dive in [`docs/p2p.md`](docs/p2p.md).

## Butterfly Timelines

Experimental sandboxes for safe experimentation:

```bash
ivaldi tl bf create experiment     # Create from current timeline
# ... make changes, commit ...
ivaldi tl bf up                    # Merge changes to parent
ivaldi tl bf down                  # Pull parent changes
ivaldi tl bf rm experiment         # Remove
ivaldi tl bf rm experiment --cascade  # Remove with all nested butterflies
```

## License

See LICENSE file.
