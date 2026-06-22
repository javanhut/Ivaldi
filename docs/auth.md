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
4. **Platform CLI** (`gh` / `glab`)

For GitHub, the platform-CLI source asks `gh auth token` directly rather than
parsing `~/.config/gh/hosts.yml`. By default `gh` stores its token in the OS
keyring (macOS Keychain / libsecret) and only writes `hosts.yml` with
`--insecure-storage`, so the old file-parse silently missed most installs —
which pushed Ivaldi into minting its own competing OAuth token. `hosts.yml`
remains a fallback for older `gh` versions.

## `ivaldi auth login` (GitHub)

The login command is **reuse-aware** so it does not pile up tokens:

1. `--with-token` — read a Personal Access Token from stdin and store it,
   skipping the browser device flow. **This is the recommended choice for
   multi-device use** (see below).
2. Otherwise, if a usable credential already resolves (a valid `gh` / env /
   `.netrc` token, or a prior ivaldi token that still validates against
   `GET /user`), Ivaldi reuses it and does **not** mint a new token. A stored
   ivaldi token that GitHub now rejects is dropped first, so login falls back
   to `gh` instead of adding another token.
3. Only when nothing usable exists does Ivaldi run the OAuth device flow.
4. `--force` skips steps 2–3 and always mints a fresh ivaldi token.

## Multi-device behavior & the 10-token cap

GitHub limits an OAuth App to **10 tokens per user / application / scope**;
minting an 11th **silently revokes the oldest**
([docs](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps)).
Because Ivaldi's device flow uses the GitHub CLI's public OAuth App with a
fixed scope, every `ivaldi auth login` (and every `gh auth login` / refresh)
spends a slot — so after enough logins across machines, an *older* device gets
logged out. That is the "cross-device auth ping-pong", and refresh tokens do
**not** fix it (OAuth Apps can't issue them, and refreshing rotates one
device's own chain without freeing a slot).

Mitigations, in order of robustness:

- **Personal Access Token** (`ivaldi auth login --with-token`): a PAT is
  independent of the OAuth-App cap. Paste the **same** fine-grained or classic
  PAT (with `repo` scope) on every device and none of them ever evict another.
- **Reuse `gh`**: on machines where `gh` is logged in, Ivaldi now reuses that
  one token instead of minting its own, so it stops contributing to the churn.
- **Your own GitHub App** (advanced): register a GitHub App with the device
  flow enabled and point Ivaldi at it via `IVALDI_GITHUB_CLIENT_ID`. GitHub
  Apps issue expiring access tokens + refresh tokens (no client secret needed
  for the device flow), giving each device an independent, self-refreshing
  credential — though the 10-token cap still applies beyond 10 devices.

## Token Storage

Location: `~/.config/ivaldi/auth.json` (permissions: 0600)

The file is written **atomically with 0600 from creation** (temp file created
mode 0600, then renamed over the target), so the token is never momentarily
world-readable as a plain write-then-chmod would leave it.

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
