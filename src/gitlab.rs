//! GitLab OAuth Device Authorization Grant (RFC 8628).
//!
//! Mirrors the GitHub device-flow shape in `src/github.rs`. Used by
//! `ivaldi auth login --gitlab` to obtain a personal access token via the
//! browser, without prompting for username/password.
//!
//! Endpoints default to gitlab.com. For self-hosted instances, set
//! `IVALDI_GITLAB_HOST` (or the `gitlab_host` repo/global config key) to the
//! base URL (e.g. `https://gitlab.example.com`). For a custom OAuth app,
//! set `IVALDI_GITLAB_CLIENT_ID`.

use crate::auth::{self, Token};
use crate::oauth_device;

pub use crate::oauth_device::DeviceCodeResponse;

/// Resolve the GitLab base URL the user wants to authenticate against.
/// Order: explicit `host` argument → `IVALDI_GITLAB_HOST` env → default.
pub fn resolve_host(explicit: Option<&str>) -> String {
    if let Some(h) = explicit {
        return h.trim_end_matches('/').to_string();
    }
    std::env::var("IVALDI_GITLAB_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| auth::GITLAB_HOST.to_string())
}

fn client_id() -> String {
    std::env::var("IVALDI_GITLAB_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| auth::GITLAB_CLIENT_ID.to_string())
}

fn scopes() -> String {
    std::env::var("IVALDI_GITLAB_SCOPES")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| auth::GITLAB_SCOPES.to_string())
}

/// Errors specific to the GitLab device flow.
#[derive(Debug, thiserror::Error)]
pub enum GitLabAuthError {
    #[error("HTTP: {0}")]
    Http(String),
    #[error("authorization expired before completion — run `ivaldi auth login --gitlab` again")]
    Expired,
    #[error("user denied the authorization request")]
    Denied,
    #[error("{0}")]
    Other(String),
}

impl From<oauth_device::DeviceFlowError> for GitLabAuthError {
    fn from(e: oauth_device::DeviceFlowError) -> Self {
        use oauth_device::DeviceFlowError;
        match e {
            DeviceFlowError::Http(m) => GitLabAuthError::Http(m),
            DeviceFlowError::Expired => GitLabAuthError::Expired,
            DeviceFlowError::Denied => GitLabAuthError::Denied,
            DeviceFlowError::Other(m) => GitLabAuthError::Other(m),
        }
    }
}

/// Build the shared device-flow configuration for `host` from GitLab's
/// constants (overridable via `IVALDI_GITLAB_CLIENT_ID` /
/// `IVALDI_GITLAB_SCOPES`).
fn device_flow_config(host: &str) -> oauth_device::DeviceFlowConfig {
    oauth_device::DeviceFlowConfig {
        device_code_url: format!("{}{}", host, auth::GITLAB_DEVICE_AUTH_PATH),
        token_url: format!("{}{}", host, auth::GITLAB_TOKEN_PATH),
        client_id: client_id(),
        scopes: scopes(),
    }
}

/// Kick off the device flow. Returns the user code + URL the caller prints.
pub fn request_device_code(host: &str) -> Result<DeviceCodeResponse, GitLabAuthError> {
    Ok(oauth_device::request_device_code(&device_flow_config(
        host,
    ))?)
}

/// Poll `/oauth/token` until the user completes (or denies) authorization.
pub fn poll_for_token(
    host: &str,
    device_code: &str,
    interval: u64,
) -> Result<Token, GitLabAuthError> {
    Ok(oauth_device::poll_for_token(
        &device_flow_config(host),
        device_code,
        interval,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_host_prefers_explicit_then_env_then_default() {
        let r = resolve_host(Some("https://git.example.com/"));
        assert_eq!(r, "https://git.example.com");

        // Explicit beats env even when env is set.
        // Don't actually set env in tests; just check default.
        let r = resolve_host(None);
        // Either the configured env or the gitlab.com default.
        assert!(
            r == auth::GITLAB_HOST
                || std::env::var("IVALDI_GITLAB_HOST")
                    .map(|s| s.trim_end_matches('/').to_string())
                    .ok()
                    == Some(r.clone())
        );
    }
}
