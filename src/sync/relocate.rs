//! Follow a portal's repository when it is renamed or transferred remotely.
//!
//! Hosts keep answering for a moved repository under its old name by
//! redirecting, so transfers survive a rename either way (see
//! `SmartHttpClient::discover_refs`). This module makes the move stick: it
//! asks the host where the repository lives now and rewrites the stored
//! portal, so the user never has to.

use std::path::Path;

use crate::git_remote::SmartHttpClient;
use crate::github::GitHubClient;
use crate::portal::{Platform, Portal, PortalManager, Transport, http_host};

/// A portal that was updated because its repository moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRelocation {
    /// `owner/repo` before the move.
    pub from: String,
    /// `owner/repo` now.
    pub to: String,
}

/// Bring `portal` up to date with where its repository lives now, updating
/// the stored portal (and `portal.default`) when it has moved.
///
/// Best-effort: a host that can't be asked, or an answer that can't be
/// understood, leaves the portal as it is, and the operation that follows
/// still reaches the repository through the host's redirects.
pub fn refresh_portal(
    ivaldi_dir: &Path,
    portal: &mut Portal,
    client: &GitHubClient,
) -> Option<PortalRelocation> {
    let moved = locate(portal, client)?;
    if moved == *portal {
        return None;
    }
    let relocation = PortalRelocation {
        from: portal.to_string_repr(),
        to: moved.to_string_repr(),
    };

    if let Err(e) = PortalManager::new(ivaldi_dir).relocate(portal, &moved) {
        crate::logging::warn(&format!("could not update portal: {}", e));
    }
    let config_path = ivaldi_dir.join("config");
    if let Ok(mut cfg) = crate::config::Config::load(&config_path)
        && cfg
            .get("portal.default")
            .is_some_and(|d| d.eq_ignore_ascii_case(&relocation.from))
    {
        cfg.set("portal.default", &relocation.to);
        if let Err(e) = cfg.save(&config_path) {
            crate::logging::warn(&format!("could not update portal.default: {}", e));
        }
    }

    *portal = moved;
    Some(relocation)
}

/// Where `portal`'s repository lives now, as a portal. `None` when that
/// can't be determined for this transport.
fn locate(portal: &Portal, client: &GitHubClient) -> Option<Portal> {
    match portal.transport() {
        Transport::Https if portal.platform == Platform::GitHub => {
            let (owner, repo) = github_current_name(client, &portal.owner, &portal.repo)?;
            Some(Portal {
                owner,
                repo,
                ..portal.clone()
            })
        }
        // GitHub serves a renamed repository over SSH without saying so;
        // only its API knows the new name.
        Transport::Ssh(target) if target.host.eq_ignore_ascii_case("github.com") => {
            let (owner, repo) = github_current_name(client, &portal.owner, &portal.repo)?;
            let url = portal.base_url.as_deref()?;
            let base_url = rewrite_ssh_repo_path(url, &target.repo_path, &owner, &repo)?;
            Some(Portal {
                owner,
                repo,
                base_url: Some(base_url),
                ..portal.clone()
            })
        }
        Transport::GenericHttps(url) => {
            let host = http_host(&url).unwrap_or_default();
            let token = crate::auth::generic_git_token(&host);
            let new_base = SmartHttpClient::new(token.as_deref())
                .find_relocation(&url)
                .ok()??;
            let (owner, repo) = url_labels(&new_base)?;
            Some(Portal {
                owner,
                repo,
                base_url: Some(new_base),
                ..portal.clone()
            })
        }
        _ => None,
    }
}

/// The current `(owner, repo)` of a GitHub repository: the API's
/// `full_name`, or — if the API can't be asked (rate limit, outage) — where
/// github.com's Git endpoint redirects the old name to.
fn github_current_name(client: &GitHubClient, owner: &str, repo: &str) -> Option<(String, String)> {
    let full_name = match client.get_repo(owner, repo) {
        Ok(info) => info.full_name,
        Err(_) => {
            let base = format!("https://github.com/{}/{}.git", owner, repo);
            let moved = SmartHttpClient::new(client.token())
                .find_relocation(&base)
                .ok()??;
            let path = moved.strip_prefix("https://github.com/")?;
            path.strip_suffix(".git").unwrap_or(path).to_string()
        }
    };
    let (new_owner, new_repo) = full_name.split_once('/')?;
    if new_owner.is_empty() || new_repo.is_empty() || new_repo.contains('/') {
        return None;
    }
    Some((new_owner.to_string(), new_repo.to_string()))
}

/// Swap the repository path at the end of an SSH URL, keeping its form
/// (`git@host:path` or `ssh://host/path`, leading `/`, `.git` suffix).
fn rewrite_ssh_repo_path(url: &str, repo_path: &str, owner: &str, repo: &str) -> Option<String> {
    let prefix = url.strip_suffix(repo_path)?;
    let lead = if repo_path.starts_with('/') { "/" } else { "" };
    let suffix = if repo_path.ends_with(".git") {
        ".git"
    } else {
        ""
    };
    Some(format!("{prefix}{lead}{owner}/{repo}{suffix}"))
}

/// Portal labels for a generic smart-HTTP URL, as `ivaldi download` derives
/// them: the last two path segments, or the host and the only segment.
fn url_labels(url: &str) -> Option<(String, String)> {
    let host = http_host(url)?;
    let path = url.split_once("://")?.1.split_once('/')?.1;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [] => None,
        [one] => Some((host, (*one).to_string())),
        many => Some((
            many[many.len() - 2].to_string(),
            many[many.len() - 1].to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Serve canned responses, one per connection, recording each request.
    fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 8192];
                let n = stream.read(&mut request).unwrap();
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..n]).into_owned());
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (base, seen)
    }

    fn redirect(status: &str, location: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn advertisement() -> String {
        let mut body = crate::git_remote::pkt_line("# service=git-upload-pack\n");
        body.extend_from_slice(b"0000");
        body.extend(crate::git_remote::pkt_line(
            "1111111111111111111111111111111111111111 refs/heads/main\0symref=HEAD:refs/heads/main\n",
        ));
        body.extend_from_slice(b"0000");
        let body = String::from_utf8(body).unwrap();
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn forged() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        std::fs::create_dir_all(&ivaldi_dir).unwrap();
        (dir, ivaldi_dir)
    }

    #[test]
    fn github_rename_updates_portal_from_full_name() {
        let body = r#"{"name":"new-repo","full_name":"new-owner/new-repo","private":true,"default_branch":"main"}"#;
        let (base, seen) = serve(vec![
            redirect("301 Moved Permanently", "/repositories/42"),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            ),
        ]);
        let (_dir, ivaldi_dir) = forged();
        let mgr = PortalManager::new(&ivaldi_dir);
        mgr.add(&Portal::parse("other/mirror").unwrap()).unwrap();
        mgr.add(&Portal::parse("old-owner/old-repo").unwrap())
            .unwrap();
        mgr.set_default("old-owner/old-repo").unwrap();
        let mut cfg = crate::config::Config::new();
        cfg.set("portal.default", "old-owner/old-repo");
        cfg.save(&ivaldi_dir.join("config")).unwrap();

        let client = GitHubClient::with_base_urls(&base, &base).with_test_token("secret");
        let mut portal = mgr.get("old-owner/old-repo").unwrap().unwrap();
        let relocation = refresh_portal(&ivaldi_dir, &mut portal, &client).unwrap();

        assert_eq!(relocation.from, "old-owner/old-repo");
        assert_eq!(relocation.to, "new-owner/new-repo");
        assert_eq!(portal.to_string_repr(), "new-owner/new-repo");
        let stored: Vec<String> = mgr
            .list()
            .unwrap()
            .iter()
            .map(|p| p.to_string_repr())
            .collect();
        assert_eq!(stored, ["new-owner/new-repo", "other/mirror"]);
        let cfg = crate::config::Config::load(&ivaldi_dir.join("config")).unwrap();
        assert_eq!(cfg.get("portal.default"), Some("new-owner/new-repo"));

        // The token followed the same-host redirect, so a private repository
        // still resolves.
        let requests = seen.lock().unwrap();
        assert!(requests[1].starts_with("GET /repositories/42 "));
        assert!(requests[1].contains("Bearer secret"));
    }

    #[test]
    fn unchanged_github_repo_is_left_alone() {
        let body =
            r#"{"name":"repo","full_name":"owner/repo","private":false,"default_branch":"main"}"#;
        let (base, _) = serve(vec![format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )]);
        let (_dir, ivaldi_dir) = forged();
        let mgr = PortalManager::new(&ivaldi_dir);
        mgr.add(&Portal::parse("owner/repo").unwrap()).unwrap();

        let client = GitHubClient::with_base_urls(&base, &base);
        let mut portal = mgr.get_default().unwrap().unwrap();
        assert_eq!(refresh_portal(&ivaldi_dir, &mut portal, &client), None);
        assert_eq!(portal.to_string_repr(), "owner/repo");
    }

    #[test]
    fn generic_host_move_updates_portal_url() {
        let (base, seen) = serve(vec![
            redirect(
                "301 Moved Permanently",
                "/team/renamed.git/info/refs?service=git-upload-pack",
            ),
            advertisement(),
        ]);
        let (_dir, ivaldi_dir) = forged();
        let mgr = PortalManager::new(&ivaldi_dir);
        let url = format!("{base}/team/project.git");
        let old = Portal::parse("team/project").unwrap().with_base_url(&url);
        mgr.add(&old).unwrap();

        let client = GitHubClient::with_base_urls(&base, &base);
        let mut portal = old.clone();
        let relocation = refresh_portal(&ivaldi_dir, &mut portal, &client).unwrap();

        assert_eq!(relocation.to, "team/renamed");
        assert_eq!(
            portal.base_url.as_deref(),
            Some(format!("{base}/team/renamed.git").as_str())
        );
        assert_eq!(mgr.list().unwrap(), vec![portal]);
        assert!(seen.lock().unwrap()[1].starts_with("GET /team/renamed.git/info/refs?"));
    }

    #[test]
    fn temporary_redirect_does_not_move_portal() {
        let (base, _) = serve(vec![
            redirect(
                "307 Temporary Redirect",
                "/mirror/project.git/info/refs?service=git-upload-pack",
            ),
            advertisement(),
        ]);
        let (_dir, ivaldi_dir) = forged();
        let url = format!("{base}/team/project.git");
        let mut portal = Portal::parse("team/project").unwrap().with_base_url(&url);
        let client = GitHubClient::with_base_urls(&base, &base);
        assert_eq!(refresh_portal(&ivaldi_dir, &mut portal, &client), None);
        assert_eq!(portal.base_url.as_deref(), Some(url.as_str()));
    }

    #[test]
    fn ssh_repo_path_rewrite_keeps_url_form() {
        assert_eq!(
            rewrite_ssh_repo_path("git@github.com:old/repo.git", "old/repo.git", "new", "name"),
            Some("git@github.com:new/name.git".into())
        );
        assert_eq!(
            rewrite_ssh_repo_path("ssh://git@github.com/old/repo", "old/repo", "new", "name"),
            Some("ssh://git@github.com/new/name".into())
        );
    }

    #[test]
    fn url_labels_match_download_labels() {
        assert_eq!(
            url_labels("https://git.example.com/team/proj.git"),
            Some(("team".into(), "proj".into()))
        );
        assert_eq!(
            url_labels("https://aur.archlinux.org/yay.git"),
            Some(("aur.archlinux.org".into(), "yay".into()))
        );
    }
}
