//! GitHub API client for Ivaldi VCS.
//!
//! Uses `ureq` (v2) for synchronous HTTP. Provides repository, branch, tree,
//! commit, blob operations, and OAuth device flow.

use std::io::Read;
use std::thread;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::auth::{self, Token};
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

fn header_str<'a>(resp: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

fn check_status(resp: ureq::http::Response<ureq::Body>) -> Result<ureq::http::Response<ureq::Body>, GitHubError> {
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(resp);
    }
    if status == 401 {
        return Err(GitHubError::AuthRequired);
    }
    if status == 403 && header_str(&resp, "X-RateLimit-Remaining") == Some("0") {
        return Err(GitHubError::RateLimited);
    }
    Err(GitHubError::Http(format!("HTTP {}", status)))
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

    fn url(&self, path: &str) -> String {
        if path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", GITHUB_API, path)
        }
    }

    fn get(&self, path: &str) -> Result<ureq::http::Response<ureq::Body>, GitHubError> {
        let url = self.url(path);
        let mut r = self
            .agent
            .get(&url)
            .header("Accept", ACCEPT)
            .header("User-Agent", "ivaldi-vcs/0.1.0");
        if let Some(ref t) = self.token {
            r = r.header("Authorization", &format!("Bearer {}", t));
        }
        let resp = r.call().map_err(gh_err)?;
        check_status(resp)
    }

    fn send_json<T: serde::Serialize>(
        &self,
        method: &str,
        path: &str,
        body: T,
    ) -> Result<ureq::http::Response<ureq::Body>, GitHubError> {
        let url = self.url(path);
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
        let resp = r.send_json(body).map_err(gh_err)?;
        check_status(resp)
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
        let resp = self.send_json("POST", &format!("/repos/{}/{}/git/blobs", owner, repo), body)?;
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
        let resp = self.send_json("POST", &format!("/repos/{}/{}/git/trees", owner, repo), body)?;
        let r: ShaResponse = resp.into_body().read_json().map_err(gh_err)?;
        Ok(r.sha)
    }

    pub fn create_commit(
        &self,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
        parents: &[String],
    ) -> Result<String, GitHubError> {
        let body = serde_json::json!({"message": message, "tree": tree_sha, "parents": parents});
        let resp =
            self.send_json("POST", &format!("/repos/{}/{}/git/commits", owner, repo), body)?;
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
        let client_id =
            std::env::var("IVALDI_GITHUB_CLIENT_ID").unwrap_or(auth::GITHUB_CLIENT_ID.into());
        let scopes = std::env::var("IVALDI_GITHUB_SCOPES").unwrap_or(auth::GITHUB_SCOPES.into());
        let body = format!("client_id={}&scope={}", client_id, scopes);
        let resp = ureq::post(auth::GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .map_err(gh_err)?;
        resp.into_body().read_json().map_err(gh_err)
    }

    pub fn poll_for_token(device_code: &str, interval: u64) -> Result<Token, GitHubError> {
        let client_id =
            std::env::var("IVALDI_GITHUB_CLIENT_ID").unwrap_or(auth::GITHUB_CLIENT_ID.into());
        loop {
            thread::sleep(Duration::from_secs(interval));
            let body = format!(
                "client_id={}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
                client_id, device_code
            );
            let resp = ureq::post(auth::GITHUB_ACCESS_TOKEN_URL)
                .header("Accept", "application/json")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .send(body.as_bytes())
                .map_err(gh_err)?;
            let r: TokenPollResponse = resp.into_body().read_json().map_err(gh_err)?;
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
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
                Some(e) => {
                    return Err(GitHubError::Other(format!(
                        "{}: {}",
                        e,
                        r.error_description.unwrap_or_default()
                    )));
                }
                None => continue,
            }
        }
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

#[derive(Debug, Deserialize)]
struct ShaResponse {
    sha: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

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
}
