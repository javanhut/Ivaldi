# CLI Module (`cli/`)

Command-line interface for Ivaldi VCS, built with `clap`.

## Command Reference

| Command | Alias | Description |
|---------|-------|-------------|
| `forge` | | Initialize repository |
| `gather [files] [-p]` | | Stage files for next seal (`-p`/`--patch` picks hunks interactively) |
| `seal "msg"` | | Create sealed commit |
| `reseal [msg]` | | Redo the most recent seal — folds in staged changes and/or a new message |
| `status [--json]` | | Show repository status |
| `whereami` | `wai` | Show current position |
| `log [--format short\|medium\|full\|json]` | | View commit history |
| `whodidit <file> [--summary]` | `blame` | Line-by-line seal attribution |
| `diff` | | Compare changes |
| `discard [files]` | | Remove files from the gathered set (none = everything) |
| `reverse --all` | | Throw away all uncommitted changes, restore from last seal (destructive!) |
| `rewind <seal> [--discard]` | | Move the timeline head back to an earlier seal (`--discard` also rewrites files) |
| `undo <seal>` | | New seal that removes an earlier seal's changes |
| `pluck <seal>` | `cherry-pick` | New seal that applies another seal's changes |
| `timeline create/switch/list/rename/remove` | `tl` | Manage timelines |
| `timeline butterfly create/up/down/rm` | `tl bf` | Butterfly timelines |
| `fuse <src> to <tgt>` | | Merge timelines (auto strategy uses MMR-based merge base) |
| `travel [--all] [--search Q]` | | Interactive history browser (DAG walk by default; `--all` shows every MMR leaf) |
| `weld --last N` / `weld START to END` | `w` | Combine seal range into one (linear history) |
| `config` | | View/modify settings |
| `exclude <patterns>` | | Add to .ivaldiignore |
| `portal add/list/remove` | | Manage remotes (HTTPS / SSH / `ivaldi://`) |
| `auth login/status/logout [--gitlab]` | | OAuth (GitHub or GitLab device flow) |
| `download <url>` | | Clone via HTTPS / SSH / `ivaldi://` (auto-detected from URL) |
| `upload` | | Push via HTTPS / SSH / `ivaldi://` (auto-detected from portal) |
| `scout` | | Discover remote branches (HTTPS / SSH) |
| `harvest <name>` | | Fetch specific branches (HTTPS / SSH) |
| `sync [branch]` | | Pull remote changes, delta only (HTTPS) |
| `serve [--bind addr:port]` | | Run an `ivaldi://` peer server |
| `peer trust/list/forget/whoami/known` | | Manage peer pubkey allowlists + TOFU known servers |
| `review create/list/show/diff/comment/approve/request-changes/merge/close/reopen` | `rv` | Local code review system |
| `completions <shell>` | | Print a bash/zsh/fish/powershell/elvish completion script |
| `man [--out dir]` | | Generate man pages (used by `make install-extras`) |

`timeline list`, `portal list`, and `status` accept `--json` for scripting;
`log --format json` does the same for history.

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

# Fixing up the most recent seal
ivaldi gather forgotten.txt
ivaldi reseal                              # fold staged changes in, keep the message
ivaldi reseal "better message"             # and/or replace the message

# Stage only some hunks of a file
ivaldi gather -p src/main.rs               # y/n per hunk; a=rest, d=skip rest, q=quit

# Going back
ivaldi undo swift-eagle                    # new seal that removes swift-eagle's changes
ivaldi pluck gentle-otter                  # new seal that applies gentle-otter's changes
ivaldi rewind calm-river                   # head moves back; your files stay as-is
ivaldi rewind calm-river --discard         # head moves back AND files are rewritten

# Scripting
ivaldi status --json | jq '.files[].path'
ivaldi log --format json | jq '.[0].seal_name'

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

# Travel — browse history with arrow keys / PgUp / PgDn
ivaldi travel                              # walks full DAG of current timeline
ivaldi travel --all                        # every leaf in the MMR (incl. orphans)
ivaldi travel --search "auth"              # filter by message/author/seal name

# Portals — transport is auto-detected from the URL
ivaldi portal add owner/repo                                 # GitHub HTTPS shorthand
ivaldi portal add https://github.com/owner/repo.git          # explicit HTTPS
ivaldi portal add git@github.com:owner/repo.git              # SSH (uses your SSH agent)
ivaldi portal add ssh://git@gitea.example.com:2222/team/proj.git
ivaldi portal add ivaldi://10.0.0.5:9418                     # peer-to-peer
ivaldi portal list

# Download — same URL forms work; transport picked automatically
ivaldi download torvalds/linux                               # HTTPS, anonymous
ivaldi download https://github.com/rust-lang/book/tree/main  # /tree/<branch> selects branch
ivaldi download git@example.com:team/repo.git                # SSH
ivaldi download ivaldi://alice.local:9418/main               # P2P

# GitLab OAuth (HTTPS only — works for SaaS gitlab.com and self-hosted)
ivaldi auth login --gitlab
ivaldi auth login --gitlab --gitlab-host https://gitlab.example.com

# Peer-to-peer (no third party in the loop)
ivaldi peer whoami                            # this machine's pubkey
ivaldi peer trust <pubkey> alice              # whitelist a peer for inbound
ivaldi peer list                              # show authorized peers
ivaldi peer known list                        # TOFU known servers we connect to
ivaldi serve --bind 0.0.0.0:9418              # accept ivaldi:// connections

# TOFU flags on download (peer-only)
ivaldi download --accept-new-peer ivaldi://1.2.3.4:9418/main   # auto-trust on first connect
ivaldi download --strict-peer ivaldi://1.2.3.4:9418/main       # refuse unknowns

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

`configure` is an alias for `config`. `ivaldi config --help` lists every
known key with a description and example; `--set` validates values per
key (email shape, true/false toggles, repo specs) and rejects dotless
keys. See [config.md](config.md) for the full key reference.

The interactive form's first field is the **scope** — toggle between
repo-local and global with ←/→ or Enter; the form reloads from (and saves
to) whichever config file is selected. Below that it covers `user.name`,
`user.email`, `color.ui`, `core.autoshelf`, and (local scope only)
`portal.default`. Email and repo-spec values are validated as you type.

`ivaldi config` **no longer requires being inside an Ivaldi repo** — outside
a repo it automatically operates on the global config (the scope selector
is locked to global).

## Architecture

- `cli/mod.rs` — Clap parser definitions (Commands, Args structs)
- `cli/commands.rs` — Command implementations wiring to core modules
- Each command finds the repo context, instantiates needed modules, and executes
