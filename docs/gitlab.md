# GitLab OAuth (`gitlab.rs`)

GitLab OAuth Device Authorization Grant (RFC 8628), mirroring the
GitHub device flow in [`github.md`](github.md). Powers
`ivaldi auth login --gitlab`.

Works against gitlab.com out of the box; supports self-hosted GitLab
instances via a `--gitlab-host` flag or `IVALDI_GITLAB_HOST` env var.

## Endpoints

```text
{host}/oauth/authorize_device     — initial device-code request
{host}/oauth/token                — token polling
```

Where `{host}` defaults to `https://gitlab.com` (override via
`--gitlab-host` / `IVALDI_GITLAB_HOST`). The default OAuth client id
embedded in the binary is glab CLI's public application; override with
`IVALDI_GITLAB_CLIENT_ID` to use your own.

## Flow

```text
client                                     gitlab
  │ POST /oauth/authorize_device           │
  │   client_id=…&scope=…                  │
  │ ──────────────────────────────────────►│
  │                                        │
  │ ◄──────────────────────────────────────│
  │   { device_code, user_code,            │
  │     verification_uri,                  │
  │     verification_uri_complete?,        │
  │     expires_in, interval }             │
  │                                        │
  │ < ivaldi prints user_code,             │
  │   opens verification_uri_complete      │
  │   (or verification_uri) in browser >   │
  │                                        │
  │   loop every `interval` seconds:       │
  │ POST /oauth/token                      │
  │   grant_type=urn:ietf:params:oauth:    │
  │     grant-type:device_code             │
  │ ──────────────────────────────────────►│
  │ ◄──────────────────────────────────────│
  │   { access_token } | { error }         │
```

Errors handled in the polling loop:

- `authorization_pending` — keep polling
- `slow_down` — back off the interval by 5s
- `expired_token` — surface as `GitLabAuthError::Expired`
- `access_denied` — surface as `GitLabAuthError::Denied`
- any other → `GitLabAuthError::Other(error: description)`

## CLI

```bash
# gitlab.com
ivaldi auth login --gitlab

# self-hosted
ivaldi auth login --gitlab --gitlab-host https://gitlab.example.com

# Or via env (useful in CI)
IVALDI_GITLAB_HOST=https://gitlab.example.com \
IVALDI_GITLAB_CLIENT_ID=… \
  ivaldi auth login --gitlab

# After login the token lands in ~/.config/ivaldi/auth.json
ivaldi auth status
# GitHub: Not authenticated
# GitLab: stored OAuth token (gitlab.com)

ivaldi auth logout --gitlab
```

`GITLAB_TOKEN` env-var fallback keeps working for users who already use
[glab](https://gitlab.com/gitlab-org/cli) — see
[`docs/auth.md`](auth.md) for the resolution order.

## Where it fits

- **HTTPS upload** to GitLab — use this. Posts to GitLab's REST API to
  create commits the way the GitHub path does.
- **SSH upload** to GitLab — does **not** need OAuth. SSH auth is
  handled by your SSH agent / keys; `ssh git@gitlab.com` either works
  or it doesn't.
- **`ivaldi://` (peer-to-peer)** — independent of GitHub/GitLab entirely.

## Files / scopes

- `src/gitlab.rs` — `request_device_code(host)`, `poll_for_token(host, …)`,
  `resolve_host(explicit) -> String`.
- `src/auth.rs` — constants:
  - `GITLAB_HOST = "https://gitlab.com"`
  - `GITLAB_CLIENT_ID` (glab's public OAuth app)
  - `GITLAB_DEVICE_AUTH_PATH = "/oauth/authorize_device"`
  - `GITLAB_TOKEN_PATH = "/oauth/token"`
  - `GITLAB_SCOPES = "read_user read_api write_repository"`
- `src/cli/commands.rs::cmd_auth` — branches on `--gitlab` and dispatches.

Tests (`src/gitlab.rs`):

- `device_code_deserializes_with_optional_verification_uri_complete`
- `browser_url_falls_back_to_verification_uri`
- `resolve_host_prefers_explicit_then_env_then_default`
- `urlencode_preserves_unreserved`
