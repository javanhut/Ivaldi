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
- **GitHub Integration** — Full download/upload/scout/harvest with OAuth

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

# Remote operations
ivaldi portal add owner/repo       # Connect to GitHub
ivaldi auth login                  # Authenticate via OAuth
ivaldi upload                      # Push to GitHub
ivaldi download owner/repo         # Clone a repository
ivaldi scout                       # List remote branches
ivaldi harvest feature-branch      # Fetch specific branch
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
| `timeline` | `tl` | Manage timelines (create/switch/list/remove) |
| `butterfly` | `tl bf` | Experimental sandbox timelines |
| `fuse` | | Merge timelines |
| `travel` | | Interactive history browser |
| `shift` | | Squash commits |
| `config` | | View/modify settings |
| `exclude` | | Add to .ivaldiignore |
| `portal` | | Manage remote connections |
| `auth` | | GitHub/GitLab authentication |
| `download` | | Clone a repository |
| `upload` | | Push to remote |
| `scout` | | Discover remote branches |
| `harvest` | | Fetch specific branches |

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
| Shift | Rebase --squash |

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
# Required
ivaldi config --set user.name "Your Name"
ivaldi config --set user.email "you@example.com"

# Optional
ivaldi config --set color.ui true
ivaldi config --set core.autoshelf true

# View all
ivaldi config --list
```

Config files:
- User: `~/.ivaldi/config`
- Repository: `.ivaldi/config` (overrides user)

## Authentication

```bash
# OAuth (recommended — works like gh auth login)
ivaldi auth login

# Environment variable
export GITHUB_TOKEN=ghp_...

# Check status
ivaldi auth status

# Logout
ivaldi auth logout
```

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
