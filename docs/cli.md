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
| `timeline create/switch/list/remove` | `tl` | Manage timelines |
| `timeline butterfly create/up/down/rm` | `tl bf` | Butterfly timelines |
| `fuse <src> to <tgt>` | | Merge timelines |
| `travel` | | Interactive history (TUI pending) |
| `shift` | | Squash commits |
| `config` | | View/modify settings |
| `exclude <patterns>` | | Add to .ivaldiignore |
| `portal add/list/remove` | | Manage remotes |
| `auth login/status/logout` | | Authentication |
| `download <repo>` | | Clone (transport pending) |
| `upload` | | Push (transport pending) |
| `scout` | | Discover remotes (transport pending) |
| `harvest` | | Fetch branches (transport pending) |

## Global Flags

- `-v, --verbose` — Increase verbosity (-v info, -vv debug)
- `-q, --quiet` — Suppress non-essential output
- `-V, --version` — Print version

## Working Examples

```bash
# Initialize
ivaldi forge

# Configure
ivaldi config --set user.name "Alice"
ivaldi config --set user.email "alice@example.com"

# Daily workflow
ivaldi gather .
ivaldi seal "Add feature"
ivaldi status

# Timelines
ivaldi tl create feature
ivaldi tl sw feature
ivaldi tl ls
ivaldi tl rm feature

# Portals
ivaldi portal add owner/repo
ivaldi portal list

# Ignore patterns
ivaldi exclude "*.log" "build/" "node_modules/"
```

## Architecture

- `cli/mod.rs` — Clap parser definitions (Commands, Args structs)
- `cli/commands.rs` — Command implementations wiring to core modules
- Each command finds the repo context, instantiates needed modules, and executes
