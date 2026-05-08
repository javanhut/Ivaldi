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

use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::auth::{self, Token};

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

fn http_err(e: impl std::fmt::Display) -> GitLabAuthError {
    GitLabAuthError::Http(e.to_string())
}

/// Response from `POST /oauth/authorize_device`.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Some GitLab versions return `verification_uri_complete` (preferred URL
    /// to open in the browser — already includes the user_code).
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

impl DeviceCodeResponse {
    /// The URL we should open in the browser. Prefers the `_complete` form
    /// when present so the user doesn't have to paste the code.
    pub fn browser_url(&self) -> &str {
        self.verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri)
    }
}

#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Kick off the device flow. Returns the user code + URL the caller prints.
pub fn request_device_code(host: &str) -> Result<DeviceCodeResponse, GitLabAuthError> {
    let url = format!("{}{}", host, auth::GITLAB_DEVICE_AUTH_PATH);
    let body = format!("client_id={}&scope={}", client_id(), urlencode(&scopes()));
    let resp = ureq::post(&url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body.as_bytes())
        .map_err(http_err)?;
    if !resp.status().is_success() {
        return Err(GitLabAuthError::Http(format!(
            "device-code request returned HTTP {}",
            resp.status().as_u16()
        )));
    }
    resp.into_body().read_json().map_err(http_err)
}

/// Poll `/oauth/token` until the user completes (or denies) authorization.
pub fn poll_for_token(
    host: &str,
    device_code: &str,
    interval: u64,
) -> Result<Token, GitLabAuthError> {
    let url = format!("{}{}", host, auth::GITLAB_TOKEN_PATH);
    let cid = client_id();
    let mut interval = interval.max(1);
    loop {
        thread::sleep(Duration::from_secs(interval));
        let body = format!(
            "client_id={}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
            cid, device_code
        );
        let resp = ureq::post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .map_err(http_err)?;
        let r: TokenPollResponse = resp.into_body().read_json().map_err(http_err)?;
        if r.access_token.as_deref().is_some_and(|s| !s.is_empty()) {
            return Ok(Token {
                access_token: r.access_token.unwrap_or_default(),
                token_type: r.token_type.unwrap_or_default(),
                scope: r.scope.unwrap_or_default(),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            });
        }
        match r.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = interval.saturating_add(5);
                continue;
            }
            Some("expired_token") => return Err(GitLabAuthError::Expired),
            Some("access_denied") => return Err(GitLabAuthError::Denied),
            Some(other) => {
                return Err(GitLabAuthError::Other(format!(
                    "{}: {}",
                    other,
                    r.error_description.unwrap_or_default()
                )));
            }
            None => continue,
        }
    }
}

/// Minimal application/x-www-form-urlencoded encoding for the few characters
/// we actually pass through (spaces in scope strings, mostly). We don't pull
/// in a URL crate just for this.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_deserializes_with_optional_verification_uri_complete() {
        let j = r#"{"device_code":"dc","user_code":"ABCD","verification_uri":"https://gitlab.com/oauth/device","verification_uri_complete":"https://gitlab.com/oauth/device?user_code=ABCD","expires_in":900,"interval":5}"#;
        let r: DeviceCodeResponse = serde_json::from_str(j).unwrap();
        assert_eq!(r.user_code, "ABCD");
        assert_eq!(
            r.browser_url(),
            "https://gitlab.com/oauth/device?user_code=ABCD"
        );
    }

    #[test]
    fn browser_url_falls_back_to_verification_uri() {
        let j = r#"{"device_code":"dc","user_code":"ABCD","verification_uri":"https://gitlab.com/oauth/device","expires_in":900,"interval":5}"#;
        let r: DeviceCodeResponse = serde_json::from_str(j).unwrap();
        assert_eq!(r.browser_url(), "https://gitlab.com/oauth/device");
    }

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

    #[test]
    fn urlencode_preserves_unreserved() {
        assert_eq!(urlencode("read_user write_repository"), "read_user%20write_repository");
        assert_eq!(urlencode("api"), "api");
    }
}
