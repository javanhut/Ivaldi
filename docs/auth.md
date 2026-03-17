# Auth Module (`auth.rs`)

Authentication management for Ivaldi VCS.

## Overview

Handles OAuth token storage and multi-source credential resolution for GitHub and GitLab.

## Token Priority

1. **Ivaldi OAuth token** (`~/.config/ivaldi/auth.json`) — highest priority
2. **Environment variable** (`GITHUB_TOKEN` / `GITLAB_TOKEN`)
3. **`.netrc` file**
4. **Platform CLI** (`gh auth login` / `glab auth login`)

## Token Storage

Location: `~/.config/ivaldi/auth.json` (permissions: 0600)

```json
{
  "github": {
    "access_token": "ghp_...",
    "token_type": "bearer",
    "scope": "repo,read:user,user:email",
    "created_at": 1700000000
  }
}
```

## Usage

```rust
use ivaldi::auth::{TokenStore, Token, resolve_auth, is_authenticated};
use ivaldi::portal::Platform;

// Check auth status
if is_authenticated(Platform::GitHub) {
    let method = resolve_auth(Platform::GitHub).unwrap();
    println!("{}", method.description);
}

// Token management
let store = TokenStore::new()?;
store.save_token(Platform::GitHub, token)?;
let loaded = store.load_token(Platform::GitHub)?;
store.delete_token(Platform::GitHub)?;
```

## Multi-Platform

Both GitHub and GitLab tokens stored in the same file. Saving one doesn't affect the other. Deleting the last token deletes the file.
