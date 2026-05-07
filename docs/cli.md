# CLI Module (`cli/`)

Command-line interface for Ivaldi VCS, built with `clap`.

## Command Reference

| Command | Alias | Description |
|---------|-------|-------------|
| `forge` | | Initialize repository |
| `gather [files]` | | Stage files for next seal |
| `seal "msg"` | | Create sealed commit |
| `status` | | Show repository status |
| `whereami` | `wai` | Show current position |
| `log` | | View commit history |
| `diff` | | Compare changes |
| `reset [files]` | | Unstage files |
| `timeline create/switch/list/rename/remove` | `tl` | Manage timelines |
| `timeline butterfly create/up/down/rm` | `tl bf` | Butterfly timelines |
| `fuse <src> to <tgt>` | | Merge timelines (auto strategy uses MMR-based merge base) |
| `travel` | | Interactive history (TUI pending) |
| `weld --last N` / `weld START to END` | `w` | Combine seal range into one (linear history) |
| `config` | | View/modify settings |
| `exclude <patterns>` | | Add to .ivaldiignore |
| `portal add/list/remove` | | Manage remotes |
| `auth login/status/logout` | | Authentication |
| `download <repo>` | | Clone (transport pending) |
| `upload` | | Push (transport pending) |
| `scout` | | Discover remotes (transport pending) |
| `harvest` | | Fetch branches (transport pending) |
| `review create/list/show/diff/comment/approve/request-changes/merge/close/reopen` | `rv` | Local code review system |

## Global Flags

- `-v, --verbose` — Increase verbosity (-v info, -vv debug)
- `-q, --quiet` — Suppress non-essential output
- `-V, --version` — Print version

## Working Examples

```bash
# Initialize
ivaldi forge

# Configure (repo-local by default; --global writes ~/.ivaldi/config;
# outside a repo, falls back to global automatically)
ivaldi config --set user.name "Alice"
ivaldi config --global --set user.email "alice@example.com"
ivaldi config                       # launches interactive ratatui form

# Daily workflow
ivaldi gather .
ivaldi seal "Add feature"
ivaldi status

# Timelines
ivaldi tl create feature
ivaldi tl sw feature
ivaldi tl ls
ivaldi tl rm feature

# Rename a timeline — three accepted forms
ivaldi tl rename main-v2                  # rename the current timeline
ivaldi tl rename feature feat             # rename feature → feat
ivaldi tl rename master to main           # `to` connector (ergonomic form)

# Weld — collapse a range of seals into one (linear history is preserved)
ivaldi weld --last 5 -m "consolidate"     # combine the last 5 seals
ivaldi weld bold-tower -m "msg"           # combine bold-tower..HEAD
ivaldi weld bold-tower to clear-galaxy    # explicit range, auto-summary message
ivaldi weld                               # interactive picker (TUI)

# Portals — any URL/SSH/shorthand form works
ivaldi portal add owner/repo
ivaldi portal add https://github.com/owner/repo.git
ivaldi portal add git@github.com:owner/repo.git
ivaldi portal list

# Download public repos anonymously (no auth required)
ivaldi download torvalds/linux
ivaldi download https://github.com/rust-lang/book/tree/main   # /tree/<branch> picks the branch

# Ignore patterns
ivaldi exclude "*.log" "build/" "node_modules/"
```

## Config flags

| Flag | Behavior |
|------|----------|
| `--list` | Show all values with `(global)` / `(local)` / `(default)` provenance |
| `--get <key>` | Print a single value |
| `--set <key> <value>` | Write to repo-local `.ivaldi/config` by default |
| `--global` | Target `~/.ivaldi/config` instead of repo-local |
| (no flag) | Launch the interactive ratatui form |

`ivaldi config` **no longer requires being inside an Ivaldi repo** — outside
a repo it automatically operates on the global config.

## Architecture

- `cli/mod.rs` — Clap parser definitions (Commands, Args structs)
- `cli/commands.rs` — Command implementations wiring to core modules
- Each command finds the repo context, instantiates needed modules, and executes
