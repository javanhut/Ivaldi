# Config Module (`config.rs`)

Configuration system for Ivaldi VCS.

## Overview

Two-level configuration with repository overriding user settings:
- **User (global) config**: `~/.ivaldi/config` (applies to all repos)
- **Repo (local) config**: `.ivaldi/config` (per-repository overrides)

## Format

INI-style with sections:

```ini
[user]
    name = Alice
    email = alice@example.com

[color]
    ui = true

[core]
    autoshelf = true

[portal]
    default = owner/repo
```

## CLI

```bash
# View (merged, with provenance annotations)
ivaldi config --list
# → user.name = Alice (global)
#   user.email = alice@example.com (local)
#   color.ui = true (default)

# Get a value
ivaldi config --get user.name

# Set a value (local by default when inside a repo)
ivaldi config --set user.name "Alice"

# Set globally (writes to ~/.ivaldi/config)
ivaldi config --global --set user.name "Alice"

# Interactive form (ratatui)
ivaldi config
```

### Outside a repo

Running `ivaldi config` outside an Ivaldi repository no longer errors — it
automatically targets the global config at `~/.ivaldi/config`. A one-line
notice prints on `--set` to make the fallback explicit:

```
$ cd /tmp
$ ivaldi config --set user.name "Alice"
not in an Ivaldi repo — using global config at /home/alice/.ivaldi/config
user.name=Alice (global)
```

Pass `--global` explicitly to write globally even when inside a repo.

### Interactive form

Invoking `ivaldi config` without `--list`/`--get`/`--set` opens a ratatui form:

```
┌─ Config ───────────────────────────────────┐
│ Ivaldi Configuration (repo-local)          │
│ /home/alice/project/.ivaldi/config         │
└────────────────────────────────────────────┘
┌────────────────────────────────────────────┐
│  User                                      │
│   ▸ name             [Alice           ]    │
│     email            [alice@example.com]   │
│                                            │
│  Appearance                                │
│     color.ui         (●) true  ( ) false   │
│                                            │
│  Core                                      │
│     autoshelf        (●) true  ( ) false   │
│                                            │
│  Remote                                    │
│     portal.default   [owner/repo       ]   │
└────────────────────────────────────────────┘
 [↑↓] Navigate  [Enter] Edit  [←→] Toggle  [s] Save  [q] Quit
```

Controls:

| Key | Action |
|-----|--------|
| ↑/↓ or j/k | Navigate fields |
| Enter | Edit text field (or toggle bool) |
| ←/→ or h/l | Toggle bool fields |
| Esc | Cancel edit / exit without saving |
| `s` | Save and exit |
| `q` | Quit (prompts if modified) |

Validation:
- `user.email` must match `x@y.z`
- `portal.default` must parse as a valid repo spec (see [portal](portal.md))

The **Remote** section only appears when run inside an Ivaldi repo.

## Library Usage

```rust
use ivaldi::config::{Config, load_config, load_global, global_config_path};

// Create with defaults
let mut cfg = Config::new();
cfg.set("user.name", "Alice");
cfg.set("user.email", "alice@example.com");

// Get values
assert_eq!(cfg.get("user.name"), Some("Alice"));

// Author string
assert_eq!(cfg.author(), Some("Alice <alice@example.com>".to_string()));

// Merge (repo overrides user)
let mut base = Config::new();
base.merge(&repo_config);

// Save/load
cfg.save(&path)?;
let loaded = Config::load(&path)?;

// Load merged config (global + repo)
let cfg = load_config(&ivaldi_dir);

// Load global only (ignores any repo config)
let cfg = load_global();

// Path to ~/.ivaldi/config
let path = global_config_path();
```

## Default Values

| Key | Default | Purpose |
|-----|---------|---------|
| `color.ui` | `true` | Enable colored terminal output |
| `core.autoshelf` | `true` | Auto-shelve uncommitted changes on timeline switch |

## Known Keys

| Key | Type | Notes |
|-----|------|-------|
| `user.name` | string | Required to create seals |
| `user.email` | string | Required to create seals |
| `color.ui` | bool | |
| `core.autoshelf` | bool | |
| `portal.default` | `owner/repo` | Default remote for `upload` / `sync` / `scout` |
