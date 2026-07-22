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
    /// Parse from any supported repo spec (`owner/repo`, full URL, SSH,
    /// `ivaldi://`, etc).
    ///
    /// Returns `None` on parse failure. For structured error info or branch
    /// hints, use [`parse_repo_spec`] directly.
    ///
    /// SSH and `ivaldi://` inputs round-trip the original URL into
    /// `base_url` so callers (notably `transport()`) can reconstruct the
    /// transport target. P2P URLs synthesize `owner=peer`, `repo=<host>:<port>`
    /// since the Ivaldi P2P transport doesn't have an owner/repo concept.
    pub fn parse(s: &str) -> Option<Self> {
        // Ivaldi P2P URLs don't go through `parse_repo_spec` (which
        // requires owner/repo segments). Handle them up front.
        if let Some(peer) = crate::p2p::PeerUrl::parse(s) {
            return Some(Self {
                owner: "peer".to_string(),
                repo: format!("{}:{}", peer.host, peer.port),
                platform: Platform::GitHub, // unused for P2P transport
                base_url: Some(s.to_string()),
            });
        }
        let spec = parse_repo_spec(s).ok()?;
        let base_url = if crate::ssh_transport::SshTarget::parse(s).is_some() {
            Some(s.to_string())
        } else {
            None
        };
        Some(Self {
            owner: spec.owner,
            repo: spec.repo,
            platform: spec.platform,
            base_url,
        })
    }

    /// Derive the transport for this portal. Inspects `base_url` first
    /// (which round-trips the original spec for SSH/P2P portals) and falls
    /// back to the platform-default HTTPS endpoint.
    pub fn transport(&self) -> Transport {
        if let Some(ref url) = self.base_url {
            if let Some(target) = crate::ssh_transport::SshTarget::parse(url) {
                return Transport::Ssh(target);
            }
            if let Some(peer) = crate::p2p::PeerUrl::parse(url) {
                return Transport::Peer(peer);
            }
            // A plain http(s) base URL that isn't github/gitlab is a generic
            // Git smart-HTTP host (AUR, Gitea, cgit, self-hosted, ...).
            if is_generic_https(url) {
                return Transport::GenericHttps(url.clone());
            }
        }
        Transport::Https
    }

    /// Format as "owner/repo".
    pub fn to_string_repr(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// Case-insensitive match against an "owner/repo" string.
    ///
    /// Owner and repo names are case-insensitive on GitHub/GitLab (the host
    /// redirects `Owner/Repo` to its canonical casing), so portal lookups
    /// must be too — otherwise `Owner/Repo` and `owner/repo` would be treated
    /// as distinct portals, producing failed lookups and silent duplicates.
    pub fn matches_repr(&self, owner_repo: &str) -> bool {
        self.to_string_repr().eq_ignore_ascii_case(owner_repo)
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

/// Extract the host from an `http(s)://` URL, e.g. `aur.archlinux.org`.
/// Returns `None` for non-HTTP URLs (SSH, `ivaldi://`, bare shorthand).
pub fn http_host(url: &str) -> Option<String> {
    let after = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    after
        .split('/')
        .next()
        .filter(|h| !h.is_empty())
        .map(|h| h.to_string())
}

/// True for an `http(s)` URL whose host is neither github.com nor gitlab.com —
/// i.e. a generic Git smart-HTTP host handled by the URL-based transport.
fn is_generic_https(url: &str) -> bool {
    match http_host(url) {
        Some(h) => !h.eq_ignore_ascii_case("github.com") && !h.eq_ignore_ascii_case("gitlab.com"),
        None => false,
    }
}

/// Transport used to talk to a portal's remote. Derived from the portal's
/// stored URL via [`Portal::transport`]; nothing on disk changes — SSH
/// and `ivaldi://` portals just round-trip their original URL through
/// `base_url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// Default — talk to GitHub/GitLab over their HTTPS smart-HTTP +
    /// REST APIs (existing `SmartHttpClient` + `GitHubClient` paths).
    Https,
    /// Generic Git smart-HTTP host (AUR, Gitea, cgit, self-hosted). Carries
    /// the full base URL, used verbatim for `git-upload-pack`/`receive-pack`.
    GenericHttps(String),
    /// Talk to a Git server over SSH using the resolved target.
    Ssh(crate::ssh_transport::SshTarget),
    /// Ivaldi-native peer-to-peer over `ivaldi://`.
    Peer(crate::p2p::PeerUrl),
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

    // A repo spec is `owner/repo` or a URL — never a local filesystem path.
    // Reject path-anchored inputs up front so a shell-expanded absolute path
    // like `/Users/me/owner/repo` fails loudly here instead of being silently
    // misread as `owner=Users, repo=me` (the cause of `Downloading Users/...`).
    if is_local_path(raw) {
        return Err(RepoSpecError::Invalid);
    }

    // Host (if one can be determined) and the "owner/repo/..." remainder.
    let (host, remainder) = extract_host_and_path(raw)?;
    let is_ssh = raw.starts_with("ssh://") || raw.starts_with("git@");
    let platform = match host.as_deref() {
        Some("github.com") | None => Platform::GitHub,
        Some("gitlab.com") => Platform::GitLab,
        Some(other) => {
            if is_ssh {
                // Self-hosted Git over SSH (Gitea, Forgejo, GitLab CE on
                // a custom host, ...) — we don't need to know the platform
                // for the SSH transport. Pick a benign default; the
                // platform field is only consulted for HTTPS-/REST-style
                // flows, which this URL won't take.
                Platform::GitHub
            } else {
                return Err(RepoSpecError::UnsupportedHost(other.to_string()));
            }
        }
    };

    // Strip trailing slashes then a trailing `.git`.
    let cleaned = remainder.trim_matches('/');
    let cleaned = cleaned.strip_suffix(".git").unwrap_or(cleaned);

    let segments: Vec<&str> = cleaned.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err(RepoSpecError::MissingSegment);
    }

    // A `/tree/<branch>` suffix (from a web URL) is the only reason a spec
    // legitimately carries more than two path segments.
    let is_tree = segments.len() >= 4 && segments[2] == "tree";

    // A bare `owner/repo` shorthand (no scheme or recognized host) must be
    // exactly two segments. Anything deeper is almost always a filesystem path
    // a shell expanded by mistake (e.g. `Users/me/owner/repo`); silently taking
    // the first two segments would target a completely unrelated repository.
    if host.is_none() && !is_tree && segments.len() != 2 {
        return Err(RepoSpecError::Invalid);
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

    let branch_hint = if is_tree {
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

/// Reports whether `s` is written as a local filesystem path rather than a
/// repository spec. Repo specs are `owner/repo` or URLs and never begin with a
/// path anchor, so anything that does is a mistake (commonly a shell that
/// expanded a relative argument to an absolute path before passing it on).
fn is_local_path(s: &str) -> bool {
    s == "."
        || s == ".."
        || s == "~"
        || s.starts_with('/')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with("~/")
}

fn extract_host_and_path(raw: &str) -> Result<(Option<String>, String), RepoSpecError> {
    // ssh://git@host/owner/repo
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let after_user = rest.splitn(2, '@').last().unwrap_or(rest);
        let (host, path) = after_user.split_once('/').ok_or(RepoSpecError::Invalid)?;
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
        if portals.iter().any(|p| p.matches_repr(&key)) {
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
        portals.retain(|p| !p.matches_repr(owner_repo));
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
            .find(|p| p.matches_repr(owner_repo)))
    }

    /// Make `owner_repo` the default portal by moving it to the front of the
    /// list (the default is the first entry). Returns false if no such portal
    /// is configured; the list is left unchanged in that case.
    pub fn set_default(&self, owner_repo: &str) -> Result<bool, PortalError> {
        let mut portals = self.list()?;
        let Some(pos) = portals.iter().position(|p| p.matches_repr(owner_repo)) else {
            return Ok(false);
        };
        if pos > 0 {
            let portal = portals.remove(pos);
            portals.insert(0, portal);
            self.save(&portals)?;
        }
        Ok(true)
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
    fn portal_transport_https_for_github_shorthand() {
        let p = Portal::parse("owner/repo").unwrap();
        assert!(matches!(p.transport(), Transport::Https));
    }

    #[test]
    fn portal_transport_https_for_https_url() {
        let p = Portal::parse("https://github.com/owner/repo.git").unwrap();
        assert!(matches!(p.transport(), Transport::Https));
    }

    #[test]
    fn portal_transport_generic_https_for_non_github_base_url() {
        let p = Portal::parse("aur.archlinux.org/yay")
            .unwrap()
            .with_base_url("https://aur.archlinux.org/yay.git");
        match p.transport() {
            Transport::GenericHttps(url) => {
                assert_eq!(url, "https://aur.archlinux.org/yay.git")
            }
            other => panic!("expected GenericHttps, got {:?}", other),
        }
    }

    #[test]
    fn portal_transport_https_not_generic_for_github_base_url() {
        // A github/gitlab base URL must keep the platform-specific Https path.
        let p = Portal::parse("owner/repo")
            .unwrap()
            .with_base_url("https://github.com/owner/repo.git");
        assert!(matches!(p.transport(), Transport::Https));
    }

    #[test]
    fn portal_transport_ssh_for_scp_form() {
        let p = Portal::parse("git@github.com:owner/repo.git").unwrap();
        match p.transport() {
            Transport::Ssh(target) => {
                assert_eq!(target.host, "github.com");
                assert_eq!(target.user, "git");
                assert_eq!(target.repo_path, "owner/repo.git");
            }
            other => panic!("expected SSH, got {:?}", other),
        }
    }

    #[test]
    fn portal_transport_ssh_for_ssh_url() {
        let p = Portal::parse("ssh://git@example.com:2222/team/proj.git").unwrap();
        match p.transport() {
            Transport::Ssh(target) => {
                assert_eq!(target.host, "example.com");
                assert_eq!(target.port, Some(2222));
                assert_eq!(target.repo_path, "team/proj.git");
            }
            other => panic!("expected SSH, got {:?}", other),
        }
    }

    #[test]
    fn portal_transport_ssh_round_trips_via_save_load() {
        let (_dir, mgr) = setup();
        let p = Portal::parse("git@example.com:owner/repo.git").unwrap();
        mgr.add(&p).unwrap();
        let loaded = mgr.list().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(matches!(loaded[0].transport(), Transport::Ssh(_)));
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
        // A leading slash makes this a filesystem path, not a bare shorthand.
        assert!(matches!(
            parse_repo_spec("/empty").unwrap_err(),
            RepoSpecError::Invalid
        ));
        assert!(matches!(
            parse_repo_spec("empty/").unwrap_err(),
            RepoSpecError::MissingSegment
        ));
    }

    #[test]
    fn spec_rejects_absolute_path() {
        // The original bug: a shell expanded `javanhut/JiraCli` to an absolute
        // path, and the parser silently read it as `owner=Users, repo=<user>`.
        for input in [
            "/Users/jhutchinson/Development/javanhut/JiraCli",
            "/Users/jhutchinson/javanhut/JiraCli",
            "/javanhut/JiraCli",
            "/tmp/foo/bar",
        ] {
            assert_eq!(
                parse_repo_spec(input).unwrap_err(),
                RepoSpecError::Invalid,
                "expected {input:?} to be rejected as a path"
            );
        }
    }

    #[test]
    fn spec_rejects_relative_and_home_paths() {
        for input in [
            "./owner/repo",
            "../owner/repo",
            "~/owner/repo",
            ".",
            "..",
            "~",
        ] {
            assert_eq!(
                parse_repo_spec(input).unwrap_err(),
                RepoSpecError::Invalid,
                "expected {input:?} to be rejected as a path"
            );
        }
    }

    #[test]
    fn spec_rejects_overlong_shorthand() {
        // A bare shorthand with more than two segments (no leading slash) is a
        // relative path mistake, not `owner/repo`.
        assert_eq!(
            parse_repo_spec("Development/javanhut/JiraCli").unwrap_err(),
            RepoSpecError::Invalid
        );
    }

    #[test]
    fn spec_still_accepts_valid_forms_after_hardening() {
        // Guard against the hardening being too aggressive.
        assert_eq!(
            parse_repo_spec("javanhut/JiraCli").unwrap().owner,
            "javanhut"
        );
        assert_eq!(parse_repo_spec("javanhut/JiraCli").unwrap().repo, "JiraCli");
        assert_eq!(
            parse_repo_spec("javanhut/JiraCli/").unwrap().repo,
            "JiraCli"
        );
        assert_eq!(
            parse_repo_spec("https://github.com/torvalds/linux/tree/master")
                .unwrap()
                .branch_hint
                .as_deref(),
            Some("master")
        );
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
    fn portal_parse_url_with_underscores() {
        let p = Portal::parse("https://github.com/owner/repo_name.git").unwrap();
        assert_eq!(p.owner, "owner");
        assert_eq!(p.repo, "repo_name");
        assert_eq!(p.platform, Platform::GitHub);

        // Underscores in both segments, and without the .git suffix.
        let p = Portal::parse("https://github.com/my_org/my_repo_name").unwrap();
        assert_eq!(p.owner, "my_org");
        assert_eq!(p.repo, "my_repo_name");
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
    fn add_duplicate_is_case_insensitive() {
        let (_dir, mgr) = setup();

        assert!(mgr.add(&Portal::parse("Owner/Repo").unwrap()).unwrap());
        // Same repo, different casing — must be rejected as a duplicate.
        assert!(!mgr.add(&Portal::parse("owner/repo").unwrap()).unwrap());
        assert!(!mgr.add(&Portal::parse("OWNER/REPO").unwrap()).unwrap());
        let list = mgr.list().unwrap();
        assert_eq!(list.len(), 1);
        // Original casing is preserved on disk.
        assert_eq!(list[0].to_string_repr(), "Owner/Repo");
    }

    #[test]
    fn remove_is_case_insensitive() {
        let (_dir, mgr) = setup();
        mgr.add(&Portal::parse("Owner/Repo").unwrap()).unwrap();

        assert!(mgr.remove("owner/repo").unwrap());
        assert!(mgr.list().unwrap().is_empty());
    }

    #[test]
    fn get_is_case_insensitive() {
        let (_dir, mgr) = setup();
        mgr.add(&Portal::parse("Owner/Repo").unwrap()).unwrap();

        assert!(mgr.get("owner/repo").unwrap().is_some());
        assert!(mgr.get("OWNER/REPO").unwrap().is_some());
        assert!(mgr.get("Owner/Repo").unwrap().is_some());
        assert!(mgr.get("other/repo").unwrap().is_none());
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
    fn set_default_reorders_and_persists() {
        let (_dir, mgr) = setup();
        mgr.add(&Portal::parse("first/repo").unwrap()).unwrap();
        mgr.add(&Portal::parse("second/repo").unwrap()).unwrap();
        mgr.add(
            &Portal::parse("third/repo")
                .unwrap()
                .with_base_url("git@example.com:third/repo.git"),
        )
        .unwrap();

        // Unknown portal is a miss; the order is unchanged.
        assert!(!mgr.set_default("nope/repo").unwrap());
        assert_eq!(
            mgr.get_default().unwrap().unwrap().to_string_repr(),
            "first/repo"
        );

        // Case-insensitive, and the new order persists to disk (list()
        // re-reads the file on every call).
        assert!(mgr.set_default("SECOND/REPO").unwrap());
        let list = mgr.list().unwrap();
        let order: Vec<String> = list.iter().map(|p| p.to_string_repr()).collect();
        assert_eq!(order, vec!["second/repo", "first/repo", "third/repo"]);
        // Reordering must not drop per-portal extras like an SSH base URL.
        assert_eq!(
            list[2].base_url.as_deref(),
            Some("git@example.com:third/repo.git")
        );

        // Already the default: a no-op success.
        assert!(mgr.set_default("second/repo").unwrap());
        assert_eq!(
            mgr.get_default().unwrap().unwrap().to_string_repr(),
            "second/repo"
        );
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
