//! Shared OAuth 2.0 Device Authorization Grant (RFC 8628) implementation.
//!
//! Both the GitHub (`src/github.rs`) and GitLab (`src/gitlab.rs`) device
//! flows are thin wrappers around this module: they build a
//! [`DeviceFlowConfig`] from their provider-specific endpoints/client ids and
//! map [`DeviceFlowError`] into their own error enums, keeping their public
//! signatures unchanged.

use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::auth::Token;

/// Provider-specific endpoints and credentials for one device flow.
pub struct DeviceFlowConfig {
    /// Full URL of the device-code endpoint (e.g. `.../login/device/code`).
    pub device_code_url: String,
    /// Full URL of the token endpoint polled for completion.
    pub token_url: String,
    /// OAuth application client id.
    pub client_id: String,
    /// Scope string in the provider's native format (url-encoded on send).
    pub scopes: String,
}

/// Errors from the device authorization flow.
#[derive(Debug, thiserror::Error)]
pub enum DeviceFlowError {
    #[error("HTTP: {0}")]
    Http(String),
    #[error("authorization expired before completion — run `ivaldi auth login` again")]
    Expired,
    #[error("user denied the authorization request")]
    Denied,
    #[error("{0}")]
    Other(String),
}

fn http_err(e: impl std::fmt::Display) -> DeviceFlowError {
    DeviceFlowError::Http(e.to_string())
}

/// Response from the device-code endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Some providers (e.g. GitLab) return `verification_uri_complete`
    /// (preferred URL to open in the browser — already includes the
    /// user_code).
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

/// One response from the token endpoint while polling.
#[derive(Debug, Deserialize)]
pub struct TokenPollResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Kick off the device flow. Returns the user code + URL the caller prints.
pub fn request_device_code(cfg: &DeviceFlowConfig) -> Result<DeviceCodeResponse, DeviceFlowError> {
    let body = format!(
        "client_id={}&scope={}",
        cfg.client_id,
        urlencode(&cfg.scopes)
    );
    let resp = ureq::post(&cfg.device_code_url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body.as_bytes())
        .map_err(http_err)?;
    if !resp.status().is_success() {
        return Err(DeviceFlowError::Http(format!(
            "device-code request returned HTTP {}",
            resp.status().as_u16()
        )));
    }
    resp.into_body().read_json().map_err(http_err)
}

/// Poll the token endpoint until the user completes (or denies)
/// authorization. Per RFC 8628, `slow_down` increases the poll interval by
/// 5 seconds.
pub fn poll_for_token(
    cfg: &DeviceFlowConfig,
    device_code: &str,
    interval_secs: u64,
) -> Result<Token, DeviceFlowError> {
    let mut interval = interval_secs.max(1);
    loop {
        thread::sleep(Duration::from_secs(interval));
        let body = format!(
            "client_id={}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
            cfg.client_id, device_code
        );
        let resp = ureq::post(&cfg.token_url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .map_err(http_err)?;
        let r: TokenPollResponse = resp.into_body().read_json().map_err(http_err)?;
        match decide_poll_outcome(r) {
            PollOutcome::Token(token) => return Ok(token),
            PollOutcome::Continue => continue,
            PollOutcome::SlowDown => {
                interval = interval.saturating_add(5);
                continue;
            }
            PollOutcome::Fail(e) => return Err(e),
        }
    }
}

/// What the poll loop should do after one token-endpoint response.
pub(crate) enum PollOutcome {
    Token(Token),
    Continue,
    SlowDown,
    Fail(DeviceFlowError),
}

/// Pure decision logic for one poll response (unit-testable seam).
pub(crate) fn decide_poll_outcome(resp: TokenPollResponse) -> PollOutcome {
    if let Some(access_token) = resp.access_token.filter(|t| !t.is_empty()) {
        return PollOutcome::Token(Token {
            access_token,
            token_type: resp.token_type.unwrap_or_default(),
            scope: resp.scope.unwrap_or_default(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        });
    }
    match resp.error.as_deref() {
        Some("authorization_pending") => PollOutcome::Continue,
        Some("slow_down") => PollOutcome::SlowDown,
        Some("expired_token") => PollOutcome::Fail(DeviceFlowError::Expired),
        Some("access_denied") => PollOutcome::Fail(DeviceFlowError::Denied),
        Some(other) => PollOutcome::Fail(DeviceFlowError::Other(format!(
            "{}: {}",
            other,
            resp.error_description.unwrap_or_default()
        ))),
        // A "success" response with an empty/missing token and no error code
        // is malformed — fail loudly rather than polling forever.
        None => PollOutcome::Fail(DeviceFlowError::Other(
            "token endpoint returned neither an access token nor an error".into(),
        )),
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

    fn poll_resp(json: &str) -> TokenPollResponse {
        serde_json::from_str(json).unwrap()
    }

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
    fn urlencode_preserves_unreserved() {
        assert_eq!(
            urlencode("read_user write_repository"),
            "read_user%20write_repository"
        );
        assert_eq!(urlencode("api"), "api");
    }

    #[test]
    fn poll_outcome_pending_continues() {
        let r = poll_resp(r#"{"error":"authorization_pending"}"#);
        assert!(matches!(decide_poll_outcome(r), PollOutcome::Continue));
    }

    #[test]
    fn poll_outcome_slow_down() {
        let r = poll_resp(r#"{"error":"slow_down"}"#);
        assert!(matches!(decide_poll_outcome(r), PollOutcome::SlowDown));
    }

    #[test]
    fn poll_outcome_expired() {
        let r = poll_resp(r#"{"error":"expired_token"}"#);
        assert!(matches!(
            decide_poll_outcome(r),
            PollOutcome::Fail(DeviceFlowError::Expired)
        ));
    }

    #[test]
    fn poll_outcome_denied() {
        let r = poll_resp(r#"{"error":"access_denied"}"#);
        assert!(matches!(
            decide_poll_outcome(r),
            PollOutcome::Fail(DeviceFlowError::Denied)
        ));
    }

    #[test]
    fn poll_outcome_unknown_error_includes_description() {
        let r = poll_resp(r#"{"error":"server_error","error_description":"boom"}"#);
        match decide_poll_outcome(r) {
            PollOutcome::Fail(DeviceFlowError::Other(msg)) => {
                assert_eq!(msg, "server_error: boom");
            }
            _ => panic!("expected Other failure"),
        }
    }

    #[test]
    fn poll_outcome_success_builds_token() {
        let r = poll_resp(r#"{"access_token":"tok123","token_type":"bearer","scope":"repo"}"#);
        match decide_poll_outcome(r) {
            PollOutcome::Token(t) => {
                assert_eq!(t.access_token, "tok123");
                assert_eq!(t.token_type, "bearer");
                assert_eq!(t.scope, "repo");
                assert!(t.created_at > 0);
            }
            _ => panic!("expected token"),
        }
    }

    #[test]
    fn poll_outcome_empty_token_is_failure() {
        let r = poll_resp(r#"{"access_token":""}"#);
        assert!(matches!(
            decide_poll_outcome(r),
            PollOutcome::Fail(DeviceFlowError::Other(_))
        ));
    }
}
