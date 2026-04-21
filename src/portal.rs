//! Portal (remote connection) management for Ivaldi VCS.
//!
//! Portals represent connections to remote repositories (GitHub, GitLab, etc.).
//! Format: `owner/repo` (not full URLs).
//!
//! Storage: `.ivaldi/portals` file, one portal per line.

use std::fs;
use std::path::{Path, PathBuf};

/// A remote repository connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portal {
    /// Repository owner (e.g., "javanhut").
    pub owner: String,
    /// Repository name (e.g., "IvaldiVCS").
    pub repo: String,
    /// Platform type.
    pub platform: Platform,
    /// Custom base URL (for self-hosted instances).
    pub base_url: Option<String>,
}

impl Portal {
    /// Parse from any supported repo spec (`owner/repo`, full URL, SSH, etc).
    ///
    /// Returns `None` on parse failure. For structured error info or branch
    /// hints, use [`parse_repo_spec`] directly.
    pub fn parse(s: &str) -> Option<Self> {
        let spec = parse_repo_spec(s).ok()?;
        Some(Self {
            owner: spec.owner,
            repo: spec.repo,
            platform: spec.platform,
            base_url: None,
        })
    }

    /// Format as "owner/repo".
    pub fn to_string_repr(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// Set the platform.
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
}

/// Remote hosting platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    GitHub,
    GitLab,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::GitHub => write!(f, "github"),
            Platform::GitLab => write!(f, "gitlab"),
        }
    }
}

/// Structured result of parsing a repo identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    pub owner: String,
    pub repo: String,
    pub platform: Platform,
    /// Optional branch extracted from a `/tree/<branch>` URL suffix.
    pub branch_hint: Option<String>,
}

/// Parser errors for repo identifiers.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RepoSpecError {
    #[error("empty repository spec")]
    Empty,
    #[error("missing owner or repository name")]
    MissingSegment,
    #[error("unsupported host: {0}")]
    UnsupportedHost(String),
    #[error("invalid repository spec")]
    Invalid,
}

/// Parse a repository identifier in any supported format.
///
/// Accepts (produces `owner=torvalds, repo=linux`):
/// - `torvalds/linux`, `torvalds/linux.git`, `torvalds/linux/`
/// - `https://github.com/torvalds/linux[.git][/]`, `http://…`
/// - `github.com/torvalds/linux`, `github:torvalds/linux`
/// - `git@github.com:torvalds/linux[.git]`, `ssh://git@github.com/torvalds/linux[.git]`
/// - `https://github.com/torvalds/linux/tree/master` → `branch_hint = Some("master")`
///
/// `gitlab.com` / `gitlab:` variants set `platform = Platform::GitLab`.
pub fn parse_repo_spec(input: &str) -> Result<RepoSpec, RepoSpecError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(RepoSpecError::Empty);
    }

    // Host (if one can be determined) and the "owner/repo/..." remainder.
    let (host, remainder) = extract_host_and_path(raw)?;
    let platform = match host.as_deref() {
        Some("github.com") | None => Platform::GitHub,
        Some("gitlab.com") => Platform::GitLab,
        Some(other) => return Err(RepoSpecError::UnsupportedHost(other.to_string())),
    };

    // Strip trailing slashes then a trailing `.git`.
    let cleaned = remainder.trim_matches('/');
    let cleaned = cleaned.strip_suffix(".git").unwrap_or(cleaned);

    let segments: Vec<&str> = cleaned.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err(RepoSpecError::MissingSegment);
    }

    let owner = segments[0].to_string();
    let mut repo = segments[1].to_string();
    // `owner/repo.git/extra` shouldn't happen often, but be defensive.
    if let Some(stripped) = repo.strip_suffix(".git") {
        repo = stripped.to_string();
    }
    if owner.is_empty() || repo.is_empty() {
        return Err(RepoSpecError::MissingSegment);
    }

    let branch_hint = if segments.len() >= 4 && segments[2] == "tree" {
        Some(segments[3..].join("/"))
    } else {
        None
    };

    Ok(RepoSpec {
        owner,
        repo,
        platform,
        branch_hint,
    })
}

fn extract_host_and_path(raw: &str) -> Result<(Option<String>, String), RepoSpecError> {
    // ssh://git@host/owner/repo
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let after_user = rest.splitn(2, '@').last().unwrap_or(rest);
        let (host, path) = after_user
            .split_once('/')
            .ok_or(RepoSpecError::Invalid)?;
        return Ok((Some(host.to_string()), path.to_string()));
    }
    // git@host:owner/repo
    if let Some(rest) = raw.strip_prefix("git@") {
        let (host, path) = rest.split_once(':').ok_or(RepoSpecError::Invalid)?;
        return Ok((Some(host.to_string()), path.to_string()));
    }
    // https://host/owner/repo  |  http://host/owner/repo
    for scheme in ["https://", "http://"] {
        if let Some(rest) = raw.strip_prefix(scheme) {
            let (host, path) = rest.split_once('/').ok_or(RepoSpecError::Invalid)?;
            return Ok((Some(host.to_string()), path.to_string()));
        }
    }
    // github:owner/repo  |  gitlab:owner/repo
    for (prefix, host) in [("github:", "github.com"), ("gitlab:", "gitlab.com")] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return Ok((Some(host.to_string()), rest.to_string()));
        }
    }
    // bare host prefix: github.com/owner/repo | gitlab.com/owner/repo
    for host in ["github.com", "gitlab.com"] {
        let with_slash = format!("{}/", host);
        if let Some(rest) = raw.strip_prefix(&with_slash) {
            return Ok((Some(host.to_string()), rest.to_string()));
        }
    }
    // Plain shorthand: owner/repo (host unknown, default to GitHub).
    Ok((None, raw.to_string()))
}

/// Manages portal configurations for a repository.
pub struct PortalManager {
    portals_path: PathBuf,
}

impl PortalManager {
    pub fn new(ivaldi_dir: &Path) -> Self {
        Self {
            portals_path: ivaldi_dir.join("portals"),
        }
    }

    /// Add a portal. Returns false if it already exists.
    pub fn add(&self, portal: &Portal) -> Result<bool, PortalError> {
        let mut portals = self.list()?;
        let key = portal.to_string_repr();
        if portals.iter().any(|p| p.to_string_repr() == key) {
            return Ok(false);
        }
        portals.push(portal.clone());
        self.save(&portals)?;
        Ok(true)
    }

    /// Remove a portal by "owner/repo". Returns false if not found.
    pub fn remove(&self, owner_repo: &str) -> Result<bool, PortalError> {
        let mut portals = self.list()?;
        let before = portals.len();
        portals.retain(|p| p.to_string_repr() != owner_repo);
        if portals.len() == before {
            return Ok(false);
        }
        self.save(&portals)?;
        Ok(true)
    }

    /// List all configured portals.
    pub fn list(&self) -> Result<Vec<Portal>, PortalError> {
        let content = match fs::read_to_string(&self.portals_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortalError::Io(e)),
        };

        let mut portals = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Format: "platform owner/repo [base_url]"
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                let platform = match parts[0] {
                    "gitlab" => Platform::GitLab,
                    _ => Platform::GitHub,
                };
                if let Some(mut portal) = Portal::parse(parts[1]) {
                    portal.platform = platform;
                    if parts.len() >= 3 {
                        portal.base_url = Some(parts[2].to_string());
                    }
                    portals.push(portal);
                }
            } else if let Some(portal) = Portal::parse(line) {
                portals.push(portal);
            }
        }

        Ok(portals)
    }

    /// Get the default (first) portal.
    pub fn get_default(&self) -> Result<Option<Portal>, PortalError> {
        Ok(self.list()?.into_iter().next())
    }

    /// Get a specific portal by "owner/repo".
    pub fn get(&self, owner_repo: &str) -> Result<Option<Portal>, PortalError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|p| p.to_string_repr() == owner_repo))
    }

    fn save(&self, portals: &[Portal]) -> Result<(), PortalError> {
        let mut lines = Vec::new();
        for portal in portals {
            let platform_str = match portal.platform {
                Platform::GitHub => "github",
                Platform::GitLab => "gitlab",
            };
            if let Some(ref url) = portal.base_url {
                lines.push(format!(
                    "{} {} {}",
                    platform_str,
                    portal.to_string_repr(),
                    url
                ));
            } else {
                lines.push(format!("{} {}", platform_str, portal.to_string_repr()));
            }
        }
        fs::write(&self.portals_path, lines.join("\n") + "\n").map_err(PortalError::Io)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid portal format: {0}")]
    InvalidFormat(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PortalManager) {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();
        (dir, PortalManager::new(&ivaldi_dir))
    }

    #[test]
    fn portal_parse() {
        let p = Portal::parse("javanhut/IvaldiVCS").unwrap();
        assert_eq!(p.owner, "javanhut");
        assert_eq!(p.repo, "IvaldiVCS");
        assert_eq!(p.platform, Platform::GitHub);
    }

    #[test]
    fn portal_parse_invalid() {
        assert!(Portal::parse("noslash").is_none());
        assert!(Portal::parse("/empty").is_none());
        assert!(Portal::parse("empty/").is_none());
        assert!(Portal::parse("").is_none());
    }

    #[test]
    fn spec_shorthand() {
        let s = parse_repo_spec("torvalds/linux").unwrap();
        assert_eq!(s.owner, "torvalds");
        assert_eq!(s.repo, "linux");
        assert_eq!(s.platform, Platform::GitHub);
        assert!(s.branch_hint.is_none());
    }

    #[test]
    fn spec_strips_dotgit_and_slash() {
        assert_eq!(parse_repo_spec("torvalds/linux.git").unwrap().repo, "linux");
        assert_eq!(parse_repo_spec("torvalds/linux/").unwrap().repo, "linux");
        assert_eq!(
            parse_repo_spec("torvalds/linux.git/").unwrap().repo,
            "linux"
        );
    }

    #[test]
    fn spec_https() {
        let s = parse_repo_spec("https://github.com/torvalds/linux").unwrap();
        assert_eq!(s.owner, "torvalds");
        assert_eq!(s.repo, "linux");
        assert_eq!(s.platform, Platform::GitHub);

        let s = parse_repo_spec("https://github.com/torvalds/linux.git").unwrap();
        assert_eq!(s.repo, "linux");

        let s = parse_repo_spec("http://github.com/torvalds/linux/").unwrap();
        assert_eq!(s.repo, "linux");
    }

    #[test]
    fn spec_bare_host() {
        let s = parse_repo_spec("github.com/torvalds/linux").unwrap();
        assert_eq!(s.owner, "torvalds");
        assert_eq!(s.repo, "linux");
        assert_eq!(s.platform, Platform::GitHub);
    }

    #[test]
    fn spec_shorthand_prefix() {
        let s = parse_repo_spec("github:torvalds/linux").unwrap();
        assert_eq!(s.platform, Platform::GitHub);
        assert_eq!(s.repo, "linux");

        let s = parse_repo_spec("gitlab:foo/bar").unwrap();
        assert_eq!(s.platform, Platform::GitLab);
    }

    #[test]
    fn spec_ssh() {
        let s = parse_repo_spec("git@github.com:torvalds/linux.git").unwrap();
        assert_eq!(s.owner, "torvalds");
        assert_eq!(s.repo, "linux");

        let s = parse_repo_spec("git@github.com:torvalds/linux").unwrap();
        assert_eq!(s.repo, "linux");

        let s = parse_repo_spec("ssh://git@github.com/torvalds/linux.git").unwrap();
        assert_eq!(s.owner, "torvalds");
        assert_eq!(s.repo, "linux");
    }

    #[test]
    fn spec_gitlab_host() {
        let s = parse_repo_spec("https://gitlab.com/foo/bar").unwrap();
        assert_eq!(s.platform, Platform::GitLab);
        assert_eq!(s.owner, "foo");
    }

    #[test]
    fn spec_tree_branch_hint() {
        let s = parse_repo_spec("https://github.com/torvalds/linux/tree/master").unwrap();
        assert_eq!(s.branch_hint.as_deref(), Some("master"));

        let s = parse_repo_spec("https://github.com/owner/repo/tree/feature/nested").unwrap();
        assert_eq!(s.branch_hint.as_deref(), Some("feature/nested"));
    }

    #[test]
    fn spec_unsupported_host() {
        let err = parse_repo_spec("https://bitbucket.org/foo/bar").unwrap_err();
        assert!(matches!(err, RepoSpecError::UnsupportedHost(_)));
    }

    #[test]
    fn spec_empty_and_missing() {
        assert_eq!(parse_repo_spec("").unwrap_err(), RepoSpecError::Empty);
        assert_eq!(parse_repo_spec("   ").unwrap_err(), RepoSpecError::Empty);
        assert!(matches!(
            parse_repo_spec("noslash").unwrap_err(),
            RepoSpecError::MissingSegment
        ));
        assert!(matches!(
            parse_repo_spec("/empty").unwrap_err(),
            RepoSpecError::MissingSegment
        ));
        assert!(matches!(
            parse_repo_spec("empty/").unwrap_err(),
            RepoSpecError::MissingSegment
        ));
    }

    #[test]
    fn portal_parse_accepts_url() {
        let p = Portal::parse("https://github.com/torvalds/linux.git").unwrap();
        assert_eq!(p.owner, "torvalds");
        assert_eq!(p.repo, "linux");
        assert_eq!(p.platform, Platform::GitHub);

        let p = Portal::parse("git@gitlab.com:foo/bar.git").unwrap();
        assert_eq!(p.owner, "foo");
        assert_eq!(p.platform, Platform::GitLab);
    }

    #[test]
    fn add_and_list() {
        let (_dir, mgr) = setup();
        let portal = Portal::parse("owner/repo").unwrap();

        assert!(mgr.add(&portal).unwrap());
        let list = mgr.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].to_string_repr(), "owner/repo");
    }

    #[test]
    fn add_duplicate() {
        let (_dir, mgr) = setup();
        let portal = Portal::parse("owner/repo").unwrap();

        assert!(mgr.add(&portal).unwrap());
        assert!(!mgr.add(&portal).unwrap()); // duplicate
        assert_eq!(mgr.list().unwrap().len(), 1);
    }

    #[test]
    fn remove() {
        let (_dir, mgr) = setup();
        mgr.add(&Portal::parse("a/b").unwrap()).unwrap();
        mgr.add(&Portal::parse("c/d").unwrap()).unwrap();

        assert!(mgr.remove("a/b").unwrap());
        assert_eq!(mgr.list().unwrap().len(), 1);
        assert_eq!(mgr.list().unwrap()[0].to_string_repr(), "c/d");
    }

    #[test]
    fn remove_nonexistent() {
        let (_dir, mgr) = setup();
        assert!(!mgr.remove("nope/nope").unwrap());
    }

    #[test]
    fn get_default() {
        let (_dir, mgr) = setup();
        assert!(mgr.get_default().unwrap().is_none());

        mgr.add(&Portal::parse("first/repo").unwrap()).unwrap();
        mgr.add(&Portal::parse("second/repo").unwrap()).unwrap();

        let default = mgr.get_default().unwrap().unwrap();
        assert_eq!(default.to_string_repr(), "first/repo");
    }

    #[test]
    fn get_specific() {
        let (_dir, mgr) = setup();
        mgr.add(&Portal::parse("a/b").unwrap()).unwrap();
        mgr.add(&Portal::parse("c/d").unwrap()).unwrap();

        assert!(mgr.get("c/d").unwrap().is_some());
        assert!(mgr.get("e/f").unwrap().is_none());
    }

    #[test]
    fn gitlab_portal() {
        let (_dir, mgr) = setup();
        let portal = Portal::parse("owner/repo")
            .unwrap()
            .with_platform(Platform::GitLab);

        mgr.add(&portal).unwrap();
        let list = mgr.list().unwrap();
        assert_eq!(list[0].platform, Platform::GitLab);
    }

    #[test]
    fn portal_with_base_url() {
        let (_dir, mgr) = setup();
        let portal = Portal::parse("owner/repo")
            .unwrap()
            .with_platform(Platform::GitLab)
            .with_base_url("https://gitlab.internal.com");

        mgr.add(&portal).unwrap();
        let list = mgr.list().unwrap();
        assert_eq!(
            list[0].base_url,
            Some("https://gitlab.internal.com".to_string())
        );
    }

    #[test]
    fn empty_list() {
        let (_dir, mgr) = setup();
        assert!(mgr.list().unwrap().is_empty());
    }
}
