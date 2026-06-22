//! GitHub API client for Ivaldi VCS.
//!
//! Uses `ureq` (v2) for synchronous HTTP. Provides repository, branch, tree,
//! commit, blob operations, and OAuth device flow.

use std::io::Read;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::auth::{self, Token};
use crate::oauth_device;
use crate::portal::Platform;

const GITHUB_API: &str = "https://api.github.com";
const ACCEPT: &str = "application/vnd.github.v3+json";

pub struct GitHubClient {
    token: Option<String>,
    agent: ureq::Agent,
}

fn make_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// Maximum number of automatic retries when GitHub reports a *secondary*
/// (abuse) rate limit. Each retry waits `Retry-After` (when supplied) or an
/// escalating fixed delay, so this bounds the worst-case stall.
const MAX_RATE_LIMIT_RETRIES: u32 = 5;

fn header_str<'a>(resp: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Pull the human-readable `message` out of a GitHub JSON error body, falling
/// back to the raw (trimmed) body when it isn't the expected shape.
fn github_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string())
}

/// GitHub signals a secondary (abuse) rate limit either via a `Retry-After`
/// header or a message in the body — the primary `X-RateLimit-Remaining`
/// counter is NOT exhausted in that case.
fn is_secondary_limit_body(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("secondary rate limit") || lower.contains("abuse")
}

/// Map a non-2xx response (already drained into `body`) to a `GitHubError`,
/// surfacing GitHub's own message instead of a bare status code.
fn classify_error(status: u16, primary_exhausted: bool, body: &str) -> GitHubError {
    if status == 401 {
        return GitHubError::AuthRequired;
    }
    if status == 403 && primary_exhausted {
        return GitHubError::RateLimited;
    }
    let msg = github_message(body);
    if msg.is_empty() {
        GitHubError::Http(format!("HTTP {}", status))
    } else {
        GitHubError::Http(format!("HTTP {} — {}", status, msg))
    }
}

impl Default for GitHubClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubClient {
    pub fn new() -> Self {
        let token = auth::resolve_auth(Platform::GitHub).map(|m| m.token);
        Self {
            token,
            agent: make_agent(),
        }
    }

    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: Some(token.into()),
            agent: make_agent(),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Best-effort check that GitHub accepts the active token, by calling
    /// `GET /user`. Returns `Some(true)` if accepted, `Some(false)` if GitHub
    /// rejected it (401), and `None` on a network/other error — so callers can
    /// avoid treating a transient outage as a bad token.
    pub fn verify_token(&self) -> Option<bool> {
        self.token.as_ref()?;
        match self.get("/user") {
            Ok(_) => Some(true),
            Err(GitHubError::AuthRequired) => Some(false),
            Err(_) => None,
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", GITHUB_API, path)
        }
    }

    /// Send a request, retrying on GitHub's *secondary* (abuse) rate limit.
    ///
    /// A burst of content-creating requests (e.g. uploading hundreds of blobs
    /// in parallel) can trip the secondary limit, after which the next
    /// mutating call returns `403`/`429` while the primary quota is still
    /// available. GitHub asks clients to back off and retry; we honor
    /// `Retry-After` when present and otherwise wait an escalating delay.
    /// Non-rate-limit failures (and the primary-quota case, whose reset can be
    /// up to an hour away) are surfaced immediately.
    fn execute_with_retry<F>(
        &self,
        send: F,
    ) -> Result<ureq::http::Response<ureq::Body>, GitHubError>
    where
        F: Fn() -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    {
        let mut attempt = 0u32;
        loop {
            let resp = send().map_err(gh_err)?;
            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                return Ok(resp);
            }

            // Capture rate-limit signals from the headers before the body,
            // which consumes the response, is read.
            let retry_after = header_str(&resp, "Retry-After").and_then(|s| s.parse::<u64>().ok());
            let primary_exhausted = header_str(&resp, "X-RateLimit-Remaining") == Some("0");
            let mut body = String::new();
            let _ = resp.into_body().into_reader().read_to_string(&mut body);

            let is_secondary = !primary_exhausted
                && (status == 429
                    || (status == 403
                        && (retry_after.is_some() || is_secondary_limit_body(&body))));

            if is_secondary && attempt < MAX_RATE_LIMIT_RETRIES {
                let wait = match retry_after {
                    Some(secs) => secs.min(300),
                    None => (60u64 << attempt).min(300),
                };
                eprintln!(
                    "GitHub secondary rate limit hit; waiting {}s before retry ({}/{})",
                    wait,
                    attempt + 1,
                    MAX_RATE_LIMIT_RETRIES
                );
                std::thread::sleep(std::time::Duration::from_secs(wait));
                attempt += 1;
                continue;
            }

            return Err(classify_error(status, primary_exhausted, &body));
        }
    }

    fn get(&self, path: &str) -> Result<ureq::http::Response<ureq::Body>, GitHubError> {
        let url = self.url(path);
        self.execute_with_retry(|| {
            let mut r = self
                .agent
                .get(&url)
                .header("Accept", ACCEPT)
                .header("User-Agent", "ivaldi-vcs/0.1.0");
            if let Some(ref t) = self.token {
                r = r.header("Authorization", &format!("Bearer {}", t));
            }
            r.call()
        })
    }

    fn send_json<T: serde::Serialize>(
        &self,
        method: &str,
        path: &str,
        body: T,
    ) -> Result<ureq::http::Response<ureq::Body>, GitHubError> {
        let url = self.url(path);
        // Serialize once so the body can be re-sent on each retry attempt.
        let value = serde_json::to_value(&body)
            .map_err(|e| GitHubError::Other(format!("serialize request body: {}", e)))?;
        self.execute_with_retry(|| {
            let mut r = match method {
                "POST" => self.agent.post(&url),
                "PUT" => self.agent.put(&url),
                "PATCH" => self.agent.patch(&url),
                _ => panic!("unsupported method {}", method),
            };
            r = r
                .header("Accept", ACCEPT)
                .header("User-Agent", "ivaldi-vcs/0.1.0");
            if let Some(ref t) = self.token {
                r = r.header("Authorization", &format!("Bearer {}", t));
            }
            r.send_json(&value)
        })
    }

    pub fn get_repo(&self, owner: &str, repo: &str) -> Result<RepoInfo, GitHubError> {
        let resp = self.get(&format!("/repos/{}/{}", owner, repo))?;
        resp.into_body().read_json().map_err(gh_err)
    }

    pub fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<BranchInfo>, GitHubError> {
        let mut all = Vec::new();
        let mut page = 1u32;
        loop {
            let resp = self.get(&format!(
                "/repos/{}/{}/branches?per_page=100&page={}",
                owner, repo, page
            ))?;
            let batch: Vec<BranchInfo> = resp.into_body().read_json().map_err(gh_err)?;
            let n = batch.len();
            all.extend(batch);
            if n < 100 {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    pub fn get_tree(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<TreeResponse, GitHubError> {
        let resp = self.get(&format!(
            "/repos/{}/{}/git/trees/{}?recursive=1",
            owner, repo, sha
        ))?;
        resp.into_body().read_json().map_err(gh_err)
    }

    pub fn list_commits(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        depth: usize,
    ) -> Result<Vec<CommitInfo>, GitHubError> {
        let mut all = Vec::new();
        let mut page = 1u32;
        loop {
            let resp = self.get(&format!(
                "/repos/{}/{}/commits?sha={}&per_page=100&page={}",
                owner, repo, branch, page
            ))?;
            let batch: Vec<CommitInfo> = resp.into_body().read_json().map_err(gh_err)?;
            let n = batch.len();
            all.extend(batch);
            if depth > 0 && all.len() >= depth {
                all.truncate(depth);
                break;
            }
            if n < 100 {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    pub fn download_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: &str,
    ) -> Result<Vec<u8>, GitHubError> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            owner, repo, git_ref, path
        );
        let resp = self.get(&url)?;
        let mut buf = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| GitHubError::Other(e.to_string()))?;
        Ok(buf)
    }

    pub fn create_blob(
        &self,
        owner: &str,
        repo: &str,
        content: &[u8],
    ) -> Result<String, GitHubError> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        let body = serde_json::json!({"content": encoded, "encoding": "base64"});
        let resp = self.send_json(
            "POST",
            &format!("/repos/{}/{}/git/blobs", owner, repo),
            body,
        )?;
        let r: ShaResponse = resp.into_body().read_json().map_err(gh_err)?;
        Ok(r.sha)
    }

    /// Create a file via the Contents API. Used to bootstrap an empty
    /// repository — GitHub's Git Data API returns 409 on empty repos, but the
    /// Contents API accepts a PUT and creates the initial commit/branch.
    pub fn create_file_contents(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        branch: &str,
        content: &[u8],
        message: &str,
    ) -> Result<(), GitHubError> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        let body = serde_json::json!({
            "message": message,
            "content": encoded,
            "branch": branch,
        });
        self.send_json(
            "PUT",
            &format!("/repos/{}/{}/contents/{}", owner, repo, path),
            body,
        )?;
        Ok(())
    }

    pub fn create_tree(
        &self,
        owner: &str,
        repo: &str,
        entries: Vec<TreeEntryCreate>,
        base_tree: Option<&str>,
    ) -> Result<String, GitHubError> {
        let mut body = serde_json::json!({"tree": entries});
        if let Some(b) = base_tree {
            body["base_tree"] = serde_json::Value::String(b.into());
        }
        let resp = self.send_json(
            "POST",
            &format!("/repos/{}/{}/git/trees", owner, repo),
            body,
        )?;
        let r: ShaResponse = resp.into_body().read_json().map_err(gh_err)?;
        Ok(r.sha)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_commit(
        &self,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
        parents: &[String],
        author: Option<&CommitIdentity>,
        committer: Option<&CommitIdentity>,
    ) -> Result<String, GitHubError> {
        let mut body = serde_json::json!({
            "message": message,
            "tree": tree_sha,
            "parents": parents,
        });
        if let Some(a) = author {
            body["author"] = serde_json::to_value(a).expect("identity serialization");
        }
        if let Some(c) = committer {
            body["committer"] = serde_json::to_value(c).expect("identity serialization");
        }
        let resp = self.send_json(
            "POST",
            &format!("/repos/{}/{}/git/commits", owner, repo),
            body,
        )?;
        let r: ShaResponse = resp.into_body().read_json().map_err(gh_err)?;
        Ok(r.sha)
    }

    pub fn update_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
        force: bool,
    ) -> Result<(), GitHubError> {
        let body = serde_json::json!({"sha": sha, "force": force});
        self.send_json(
            "PATCH",
            &format!("/repos/{}/{}/git/refs/heads/{}", owner, repo, branch),
            body,
        )?;
        Ok(())
    }

    pub fn create_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GitHubError> {
        let body = serde_json::json!({"ref": format!("refs/heads/{}", branch), "sha": sha});
        self.send_json("POST", &format!("/repos/{}/{}/git/refs", owner, repo), body)?;
        Ok(())
    }

    pub fn request_device_code() -> Result<DeviceCodeResponse, GitHubError> {
        Ok(oauth_device::request_device_code(&device_flow_config())?)
    }

    pub fn poll_for_token(device_code: &str, interval: u64) -> Result<Token, GitHubError> {
        Ok(oauth_device::poll_for_token(
            &device_flow_config(),
            device_code,
            interval,
        )?)
    }
}

/// Build the shared device-flow configuration from GitHub's constants
/// (overridable via `IVALDI_GITHUB_CLIENT_ID` / `IVALDI_GITHUB_SCOPES`).
fn device_flow_config() -> oauth_device::DeviceFlowConfig {
    oauth_device::DeviceFlowConfig {
        device_code_url: auth::GITHUB_DEVICE_CODE_URL.to_string(),
        token_url: auth::GITHUB_ACCESS_TOKEN_URL.to_string(),
        client_id: std::env::var("IVALDI_GITHUB_CLIENT_ID")
            .unwrap_or(auth::GITHUB_CLIENT_ID.into()),
        scopes: std::env::var("IVALDI_GITHUB_SCOPES").unwrap_or(auth::GITHUB_SCOPES.into()),
    }
}

// --- API types ---

#[derive(Debug, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub private: bool,
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub commit: BranchCommit,
}
#[derive(Debug, Deserialize)]
pub struct BranchCommit {
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub commit: CommitDetail,
    #[serde(default)]
    pub parents: Vec<ParentRef>,
}
#[derive(Debug, Deserialize)]
pub struct CommitDetail {
    pub message: String,
    pub author: AuthorInfo,
    pub tree: TreeRef,
}
#[derive(Debug, Deserialize)]
pub struct AuthorInfo {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub date: Option<String>,
}
#[derive(Debug, Deserialize)]
pub struct TreeRef {
    pub sha: String,
}
#[derive(Debug, Deserialize)]
pub struct ParentRef {
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TreeResponse {
    pub sha: String,
    pub tree: Vec<TreeEntry>,
    #[serde(default)]
    pub truncated: bool,
}
#[derive(Debug, Clone, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub sha: String,
}

#[derive(Debug, Serialize)]
pub struct TreeEntryCreate {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub sha: String,
}

/// Author or committer identity for `create_commit`.
///
/// `date` is RFC 3339 (ISO 8601 with timezone), e.g. `2024-01-01T12:00:00+00:00`.
/// GitHub's Git Data API accepts the same shape for both `author` and `committer`.
#[derive(Debug, Clone, Serialize)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
struct ShaResponse {
    sha: String,
}

pub use crate::oauth_device::DeviceCodeResponse;

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("HTTP: {0}")]
    Http(String),
    #[error("auth required — run 'ivaldi auth login' or set GITHUB_TOKEN")]
    AuthRequired,
    #[error("rate limited")]
    RateLimited,
    #[error("{0}")]
    Other(String),
}

fn gh_err(e: impl std::fmt::Display) -> GitHubError {
    let m = e.to_string();
    if m.contains("401") {
        GitHubError::AuthRequired
    } else if m.contains("403") && m.contains("rate") {
        GitHubError::RateLimited
    } else {
        GitHubError::Http(m)
    }
}

impl From<oauth_device::DeviceFlowError> for GitHubError {
    fn from(e: oauth_device::DeviceFlowError) -> Self {
        use oauth_device::DeviceFlowError;
        match e {
            DeviceFlowError::Http(m) => GitHubError::Http(m),
            DeviceFlowError::Other(m) => GitHubError::Other(m),
            other @ (DeviceFlowError::Expired | DeviceFlowError::Denied) => {
                GitHubError::Other(other.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_with_token() {
        let c = GitHubClient::with_token("test");
        assert!(c.is_authenticated());
    }

    #[test]
    fn tree_entry_create_serializes() {
        let e = TreeEntryCreate {
            path: "a.txt".into(),
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha: "abc".into(),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"type\":\"blob\""));
    }

    #[test]
    fn device_code_deserializes() {
        let j = r#"{"device_code":"dc","user_code":"ABCD","verification_uri":"https://x","expires_in":900,"interval":5}"#;
        let r: DeviceCodeResponse = serde_json::from_str(j).unwrap();
        assert_eq!(r.user_code, "ABCD");
    }

    #[test]
    fn commit_info_deserializes() {
        let j = r#"{"sha":"abc","commit":{"message":"msg","author":{"name":"A","email":"a@b"},"tree":{"sha":"def"}},"parents":[{"sha":"p1"}]}"#;
        let r: CommitInfo = serde_json::from_str(j).unwrap();
        assert_eq!(r.parents.len(), 1);
    }

    #[test]
    fn github_message_extracts_message_field() {
        let body = r#"{"message":"You have exceeded a secondary rate limit. Please wait a few minutes before you try again.","documentation_url":"https://docs.github.com"}"#;
        assert!(github_message(body).starts_with("You have exceeded a secondary rate limit"));
    }

    #[test]
    fn github_message_falls_back_to_raw_body() {
        assert_eq!(github_message("not json"), "not json");
    }

    #[test]
    fn detects_secondary_rate_limit_body() {
        assert!(is_secondary_limit_body(
            r#"{"message":"You have exceeded a secondary rate limit."}"#
        ));
        assert!(is_secondary_limit_body(
            "triggered an abuse detection mechanism"
        ));
        assert!(!is_secondary_limit_body(r#"{"message":"Not Found"}"#));
    }

    #[test]
    fn classify_error_maps_status_and_message() {
        // 401 → auth required regardless of body.
        assert!(matches!(
            classify_error(401, false, ""),
            GitHubError::AuthRequired
        ));
        // 403 with the primary quota exhausted → rate limited.
        assert!(matches!(
            classify_error(403, true, ""),
            GitHubError::RateLimited
        ));
        // Other failures surface GitHub's message.
        match classify_error(403, false, r#"{"message":"Resource not accessible"}"#) {
            GitHubError::Http(m) => {
                assert!(m.contains("403"));
                assert!(m.contains("Resource not accessible"));
            }
            other => panic!("expected Http, got {:?}", other),
        }
    }

    #[test]
    fn commit_identity_serializes_with_required_fields() {
        let id = CommitIdentity {
            name: "Jane Doe".into(),
            email: "jane@example.com".into(),
            date: "2024-01-15T10:30:00+00:00".into(),
        };
        let j = serde_json::to_value(&id).unwrap();
        assert_eq!(j["name"], "Jane Doe");
        assert_eq!(j["email"], "jane@example.com");
        assert_eq!(j["date"], "2024-01-15T10:30:00+00:00");
    }
}
