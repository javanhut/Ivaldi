# Auth Module (`auth.rs`)

OAuth token storage and credential resolution for HTTPS-based remote
operations (GitHub + GitLab REST APIs).

## Overview

Tokens for GitHub and GitLab live side-by-side at
`~/.config/ivaldi/auth.json` (mode 0600). Auth is resolved through a
priority chain so existing `gh` / `glab` users don't have to re-login.

**Auth is only needed for HTTPS upload / sync.** Public-repo download /
scout / harvest work anonymously. SSH transport uses the system SSH
agent (no Ivaldi-side auth). Peer-to-peer uses each user's long-lived
ed25519 identity at `~/.ivaldi/identity` — see [`identity.md`](identity.md)
and [`p2p.md`](p2p.md).

For the GitLab device flow specifically, see [`gitlab.md`](gitlab.md).

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
