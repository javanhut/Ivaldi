# Ivaldi Quick Start

A hands-on walkthrough of every Ivaldi function — what to type, what it
does, and where to read more. Coming from git? Keep
[the Rosetta Stone](rosetta.md) open in another tab; it maps every git
command to its Ivaldi equivalent.

## Contents

- [Install](#install)
- [Create a repository](#create-a-repository)
- [Configure your identity](#configure-your-identity)
- [Daily workflow](#daily-workflow)
- [Going back in time](#going-back-in-time)
- [Timelines (branches)](#timelines-branches)
- [Butterfly timelines](#butterfly-timelines)
- [Remote operations](#remote-operations)
- [Authentication](#authentication)
- [Merge strategies](#merge-strategies)
- [Peer-to-peer sharing](#peer-to-peer-sharing)
- [Command reference](#command-reference)
- [Configuration](#configuration)
- [Scripting and CI](#scripting-and-ci)

## Install

```bash
# Prerequisites: Rust toolchain (edition 2024) — https://rustup.rs

# Build and install to /usr/local/bin
make build
sudo make install

# Or install to a custom location (no sudo needed)
make install PREFIX=~/.local

# Optional: man pages + bash/zsh/fish completions
sudo make install-extras

# Run the test suite
make test

# Uninstall
sudo make uninstall
```

## Create a repository

```bash
ivaldi forge        # like `git init` — creates .ivaldi/
```

`forge` lays down the repository structure:

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

More detail: [repo.md](repo.md), [forge.md](forge.md), [store.md](store.md).

## Configure your identity

```bash
# Set once globally — applies to every repo
ivaldi config --global --set user.name "Your Name"
ivaldi config --global --set user.email "you@example.com"
```

More detail: [config.md](config.md), [identity.md](identity.md).

## Daily workflow

```bash
ivaldi gather .                    # Stage all files
ivaldi gather -p src/main.rs       # Stage only some hunks (interactive)
ivaldi seal "Add new feature"      # Commit
ivaldi reseal "Better message"     # Redo the last seal (message and/or staged changes)
ivaldi status                      # Check workspace state
ivaldi log --oneline               # View history
ivaldi diff                        # Compare changes
ivaldi whodidit src/main.rs        # Which seal last touched each line (blame)
ivaldi whereami                    # Show current position (alias: wai)
ivaldi exclude "*.tmp"             # Add patterns to .ivaldiignore
```

Every seal gets a memorable name like `swift-eagle-flies-high-447abe9b` —
you refer to history by name, not by hash.

More detail: [seal.md](seal.md), [log.md](log.md), [pick.md](pick.md),
[ignore.md](ignore.md).

## Going back in time

History is never rewritten — old seals stay recoverable.

```bash
ivaldi undo swift-eagle            # New seal that removes swift-eagle's changes
ivaldi pluck gentle-otter          # New seal that applies gentle-otter's changes
ivaldi rewind calm-river           # Move the head back; your files stay as-is
ivaldi rewind calm-river --discard # Move the head back AND rewrite the files
ivaldi reset file.txt              # Unstage a file
ivaldi reset --hard                # Discard local changes
ivaldi weld --last 3               # Combine the last 3 seals into one
ivaldi travel                      # Interactive history browser
ivaldi travel --all                # Include orphaned seals (like git reflog)
```

More detail: [undo.md](undo.md), [weld.md](weld.md), [mmr.md](mmr.md).

## Timelines (branches)

Switching timelines automatically shelves your uncommitted changes and
restores them when you come back — no manual stashing.

```bash
ivaldi timeline create feature     # Create timeline (alias: tl)
ivaldi timeline switch feature     # Switch (auto-shelves changes)
ivaldi timeline list               # List all timelines
ivaldi timeline rename old new     # Rename
ivaldi timeline remove feature     # Remove
ivaldi fuse feature to main        # Merge
```

More detail: [timeline.md](timeline.md), [shelf.md](shelf.md),
[switch_journal.md](switch_journal.md).

## Butterfly timelines

Experimental sandboxes with bidirectional parent sync — make risky
changes, then push them up to the parent or pull the parent's progress
down, in either order.

```bash
ivaldi tl bf create experiment        # Create from current timeline
# ... make changes, seal ...
ivaldi tl bf up                       # Merge changes to parent
ivaldi tl bf down                     # Pull parent changes
ivaldi tl bf rm experiment            # Remove
ivaldi tl bf rm experiment --cascade  # Remove with all nested butterflies
```

More detail: [butterfly.md](butterfly.md).

## Remote operations

Ivaldi picks the right transport automatically from the URL on each
command — there is no `--protocol` flag. The same `download` / `upload` /
`scout` / `harvest` / `sync` invocations work for all three:

| URL form | Transport | What runs |
|---|---|---|
| `owner/repo` · `https://github.com/owner/repo.git` · `github:owner/repo` | HTTPS | GitHub Smart-HTTP + REST API |
| `git@host:owner/repo.git` · `ssh://git@host:port/owner/repo.git` | SSH | `ssh ... git-upload-pack/git-receive-pack`, identical wire to plain git |
| `ivaldi://host:port[/timeline]` | Peer-to-peer | Ivaldi-native protocol over Noise XX (mutual ed25519 auth, ChaCha20-Poly1305) |

```bash
# Add a remote (transport detected from the URL form)
ivaldi portal add owner/repo                          # GitHub HTTPS shorthand
ivaldi portal add git@host.example.com:team/repo.git  # SSH (any host: github, gitlab, gitea, …)
ivaldi portal add ivaldi://10.0.0.5:9418              # Peer-to-peer (no third-party host)

# Clone
ivaldi download owner/repo                            # HTTPS (no auth for public repos)
ivaldi download git@example.com:team/repo.git         # SSH
ivaldi download ivaldi://10.0.0.5:9418/main           # P2P

# Push / pull
ivaldi upload                      # Push (auto-routes via SSH / GitHub REST / ivaldi://)
ivaldi sync                        # Pull remote changes (delta only)

# Selective sync — download only the branches you need
ivaldi scout                       # List remote branches
ivaldi harvest feature-branch      # Fetch a specific branch
```

**HTTPS** uses GitHub/GitLab's REST APIs for write paths (creates commits
via `/git/commits` so author/committer round-trip exactly). **SSH** spawns
the system `ssh` binary as a subprocess — your existing keys, agent, and
`known_hosts` keep working — and speaks the standard git pack protocol.

Round-tripping a git repo through Ivaldi (download → upload) preserves
commit SHA-1s byte-for-byte, including author, committer, and timezone.

More detail: [remote.md](remote.md), [portal.md](portal.md),
[sync.md](sync.md), [github.md](github.md), [gitlab.md](gitlab.md),
[ssh.md](ssh.md), [git_export.md](git_export.md).

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

More detail: [auth.md](auth.md), [oauth_device.md](oauth_device.md),
[identity.md](identity.md).

## Merge strategies

Merges never leave conflict markers in your files — when the auto
strategy can't resolve a conflict, you pick a strategy instead.

```bash
ivaldi fuse feature to main                    # Auto (default, MMR-based merge base)
ivaldi fuse --strategy=theirs feature to main  # Accept all source changes
ivaldi fuse --strategy=ours feature to main    # Keep all target changes
ivaldi fuse --strategy=union feature to main   # Combine both
ivaldi fuse --strategy=base feature to main    # Revert to ancestor
ivaldi fuse --abort                            # Cancel merge
ivaldi fuse --continue                         # Resume after conflict resolution
```

More detail: [fsmerkle.md](fsmerkle.md), [timeline.md](timeline.md).

## Peer-to-peer sharing

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

# Trust management
ivaldi peer known list           # Servers we trust (TOFU known_peers)
ivaldi peer forget <name>        # Revoke a peer
```

First connection prompts a TOFU fingerprint check (mirrors `~/.ssh/known_hosts`);
use `--accept-new-peer` for non-interactive scripts or `--strict-peer` to
refuse anything not already in `~/.ivaldi/known_peers`.

Deep dive: [p2p.md](p2p.md).

## Command reference

| Command | Alias | Description |
|---------|-------|-------------|
| `forge` | | Initialize repository |
| `gather [files]` | | Stage files for next seal (`-p` picks hunks interactively) |
| `seal "message"` | | Create a commit |
| `status` | | Show workspace state |
| `whereami` | `wai` | Show current position |
| `log` | | View commit history |
| `whodidit <file>` | `blame` | Show which seal last touched each line of a file |
| `diff` | | Compare changes |
| `reseal` | | Redo the most recent seal (new message and/or staged changes) |
| `reset` | | Unstage files or discard local changes |
| `rewind <seal>` | | Move the timeline head back (`--discard` to also rewrite files) |
| `undo <seal>` | | New seal that removes an earlier seal's changes |
| `pluck <seal>` | `cherry-pick` | New seal that applies another seal's changes |
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
| `review` | `rv` | Local code review system (see [review.md](review.md)) |
| `completions <shell>` | | Print a shell completion script (bash/zsh/fish/powershell/elvish) |
| `man [--out dir]` | | Generate man pages (used by `make install-extras`) |

Full flag-level reference: [cli.md](cli.md).

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

More detail: [config.md](config.md), [color.md](color.md).

## Scripting and CI

`status`, `timeline list`, and `portal list` accept `--json`, and
`log --format json` emits machine-readable history — handy for scripts
and CI. `make install-extras` installs man pages and shell completions.
