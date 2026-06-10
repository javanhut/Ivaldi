# OAuth Device Flow Module (`oauth_device.rs`)

Provider-agnostic OAuth 2.0 device flow (RFC 8628), shared by the GitHub
and GitLab clients.

## Overview

`github.rs` and `gitlab.rs` used to carry near-identical copies of the
device-code request and token-poll loop. Both are now thin wrappers over
this module; each builds a `DeviceFlowConfig` from its own endpoints and
maps `DeviceFlowError` into its provider error type, so the public
signatures of the provider clients are unchanged.

```rust
pub struct DeviceFlowConfig { device_code_url, token_url, client_id, scopes }
pub enum DeviceFlowError { Http(String), Expired, Denied, Other(String) }

pub fn request_device_code(cfg) -> Result<DeviceCodeResponse, DeviceFlowError>
pub fn poll_for_token(cfg, device_code, interval_secs) -> Result<auth::Token, DeviceFlowError>
```

## Behavior

- Scopes are URL-encoded in the form body; the device-code response's
  HTTP status is checked.
- The poll loop sleeps `interval` seconds; `slow_down` increases the
  interval by 5 (RFC 8628), `authorization_pending` continues,
  `expired_token` → `Expired`, `access_denied` → `Denied`.
- A success response with an empty/missing token fails loudly instead of
  looping forever.
- `DeviceCodeResponse` is the superset of both providers' shapes
  (GitLab's `verification_uri_complete` is optional; `browser_url()`
  picks the best URL to open).
- The pure `decide_poll_outcome(TokenPollResponse) -> PollOutcome` seam
  carries all the branching and is unit-tested without any network.

Tokens are stored by [auth.md](auth.md)'s `TokenStorage` exactly as
before.
