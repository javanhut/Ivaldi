# Config Module (`config.rs`)

Configuration system for Ivaldi VCS.

## Overview

Two-level configuration with repository overriding user settings:
- **User config**: `~/.ivaldi/config` (applies to all repos)
- **Repo config**: `.ivaldi/config` (per-repository overrides)

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
```

## Usage

```rust
use ivaldi::config::{Config, load_config};

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

// Load merged config (user + repo)
let cfg = load_config(&ivaldi_dir);
```

## Default Values

| Key | Default |
|-----|---------|
| `color.ui` | `true` |
| `core.autoshelf` | `true` |

## Required Settings

Before creating seals:
- `user.name` — your name
- `user.email` — your email
