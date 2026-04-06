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
    /// Parse from "owner/repo" format.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (owner, repo) = s.split_once('/')?;
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        Some(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            platform: Platform::GitHub,
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
