//! Network commands: portal, auth, download, upload, scout, harvest, sync, serve, peer.

use super::*;

/// "portal 'x' not found" error naming the configured portals, so a typo is
/// fixable from the message alone.
fn portal_not_found(mgr: &PortalManager, repr: &str) -> String {
    let available: Vec<String> = match mgr.list() {
        Ok(portals) => portals.iter().map(|p| p.to_string_repr()).collect(),
        Err(_) => Vec::new(),
    };
    if available.is_empty() {
        format!(
            "portal '{repr}' not found — no portals configured. Run 'ivaldi portal add owner/repo'."
        )
    } else {
        format!(
            "portal '{repr}' not found. Configured portals: {}",
            available.join(", ")
        )
    }
}

/// Resolve which portal a remote command targets: the one named by
/// `--portal`, else the configured default (the first portal).
fn resolve_portal(mgr: &PortalManager, name: Option<&str>) -> Result<Portal, String> {
    match name {
        Some(repr) => mgr
            .get(repr)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| portal_not_found(mgr, repr)),
        None => mgr
            .get_default()
            .map_err(|e| e.to_string())?
            .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.".into()),
    }
}

pub(super) fn cmd_portal(args: PortalArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mgr = PortalManager::new(&ctx.ivaldi_dir);
    match args.command {
        PortalCommands::Add(add_args) => {
            // `ivaldi://` URLs synthesize a portal directly (P2P has no
            // owner/repo). Other inputs go through the parse_repo_spec
            // pipeline and preserve SSH origin in base_url for transport
            // dispatch.
            let mut portal = if let Some(p) = Portal::parse(&add_args.repo) {
                if p.base_url.is_some() {
                    p
                } else {
                    let spec = parse_repo_arg(&add_args.repo)?;
                    Portal {
                        owner: spec.owner,
                        repo: spec.repo,
                        platform: spec.platform,
                        base_url: None,
                    }
                }
            } else {
                let spec = parse_repo_arg(&add_args.repo)?;
                Portal {
                    owner: spec.owner,
                    repo: spec.repo,
                    platform: spec.platform,
                    base_url: None,
                }
            };
            if add_args.gitlab {
                portal.platform = Platform::GitLab;
            }
            if let Some(url) = add_args.url {
                portal.base_url = Some(url);
            }
            let added = mgr.add(&portal).map_err(|e| e.to_string())?;
            if !quiet {
                if added {
                    println!("Added portal: {}", portal.to_string_repr());
                } else {
                    println!("Portal already configured: {}", portal.to_string_repr());
                }
            }
        }
        PortalCommands::List(list_args) => {
            let portals = mgr.list().map_err(|e| e.to_string())?;
            if list_args.json {
                let out: Vec<json::PortalJson> = portals
                    .iter()
                    .enumerate()
                    .map(|(i, p)| json::PortalJson {
                        repo: p.to_string_repr(),
                        platform: match p.platform {
                            Platform::GitHub => "github",
                            Platform::GitLab => "gitlab",
                        }
                        .to_string(),
                        url: p.base_url.clone(),
                        default: i == 0,
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
                );
            } else if portals.is_empty() {
                println!("No portals configured.");
            } else {
                println!("Configured portals:");
                for (i, p) in portals.iter().enumerate() {
                    let plat = match p.platform {
                        Platform::GitHub => "github",
                        Platform::GitLab => "gitlab",
                    };
                    print!("  {} ({})", p.to_string_repr(), plat);
                    if let Some(url) = &p.base_url {
                        print!(" [{}]", url);
                    }
                    if i == 0 {
                        print!(" (default)");
                    }
                    println!();
                }
            }
        }
        PortalCommands::Remove(remove_args) => {
            let removed = mgr.remove(&remove_args.repo).map_err(|e| e.to_string())?;
            if !quiet {
                if removed {
                    println!("Removed portal: {}", remove_args.repo);
                } else {
                    println!("Portal not found: {}", remove_args.repo);
                }
            }
        }
        PortalCommands::SetDefault(sd_args) => {
            if !mgr.set_default(&sd_args.repo).map_err(|e| e.to_string())? {
                return Err(portal_not_found(&mgr, &sd_args.repo));
            }
            if !quiet {
                // Show the canonical stored casing, not the user's spelling.
                let repr = mgr
                    .get(&sd_args.repo)
                    .map_err(|e| e.to_string())?
                    .map(|p| p.to_string_repr())
                    .unwrap_or_else(|| sd_args.repo.clone());
                println!("Default portal set to: {repr}");
            }
        }
    }
    Ok(())
}

/// Try to open `url` in the user's default browser. Best-effort: returns
/// false when no opener exists, the environment is headless, or the user
/// opted out via IVALDI_NO_BROWSER. The caller still prints the URL so the
/// user can fall back to copy/paste.
pub(super) fn open_in_browser(url: &str) -> bool {
    if std::env::var_os("IVALDI_NO_BROWSER").is_some() {
        return false;
    }
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .is_ok()
}

pub(super) fn cmd_auth(args: AuthArgs) -> Result<(), String> {
    match args.command {
        AuthCommands::Login(login_args) => {
            use crate::auth::TokenStore;
            use crate::github::GitHubClient;
            use crate::gitlab;

            if login_args.gitlab {
                let host = gitlab::resolve_host(login_args.gitlab_host.as_deref());
                println!("Initiating GitLab authentication against {}...", host);
                let device_code = gitlab::request_device_code(&host).map_err(|e| e.to_string())?;
                println!(
                    "\nFirst, copy your one-time code: {}",
                    device_code.user_code
                );
                let url = device_code.browser_url();
                if open_in_browser(url) {
                    println!("Opened {} in your browser.", url);
                    println!("(If nothing opened, visit the URL above manually.)");
                } else {
                    println!("Then visit: {}", url);
                }
                println!("\nWaiting for authentication...");
                let token =
                    gitlab::poll_for_token(&host, &device_code.device_code, device_code.interval)
                        .map_err(|e| e.to_string())?;
                let store = TokenStore::new().map_err(|e| e.to_string())?;
                store
                    .save_token(Platform::GitLab, token)
                    .map_err(|e| e.to_string())?;
                println!("\nAuthentication successful!");
                return Ok(());
            }

            // PAT path: store a Personal Access Token read from stdin instead of
            // running the device flow. PATs are independent per device and are
            // immune to GitHub's 10-token-per-OAuth-app eviction, so this is the
            // most reliable option when ivaldi is used on many machines.
            if login_args.with_token {
                use std::io::BufRead;
                eprintln!(
                    "Paste a GitHub Personal Access Token (needs 'repo' scope) and press Enter:"
                );
                let mut input = String::new();
                std::io::stdin()
                    .lock()
                    .read_line(&mut input)
                    .map_err(|e| e.to_string())?;
                let pat = input.trim().to_string();
                if pat.is_empty() {
                    return Err("no token provided on stdin".into());
                }
                if GitHubClient::with_token(pat.clone()).verify_token() == Some(false) {
                    return Err(
                        "GitHub rejected that token. Check it is valid and has 'repo' scope."
                            .into(),
                    );
                }
                let token = crate::auth::Token {
                    access_token: pat,
                    token_type: "pat".to_string(),
                    scope: String::new(),
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                };
                let store = TokenStore::new().map_err(|e| e.to_string())?;
                store
                    .save_token(Platform::GitHub, token)
                    .map_err(|e| e.to_string())?;
                println!("Stored your GitHub Personal Access Token for this device.");
                return Ok(());
            }

            // Reuse-aware: don't mint another OAuth token if a usable one
            // already resolves. Every extra token counts against GitHub's
            // 10-per-OAuth-app/scope limit and can silently evict an older
            // device, which is the root cause of the cross-device logouts.
            // `--force` skips this entirely.
            if !login_args.force {
                // If our own stored token is no longer accepted by GitHub, drop
                // it so we can fall back to gh / env / .netrc instead of piling
                // on yet another token. Only delete on a definitive rejection,
                // never on a transient network error (verify_token -> None).
                if let Some(existing) = crate::auth::resolve_auth(Platform::GitHub)
                    && existing.name == "ivaldi"
                    && GitHubClient::with_token(existing.token.clone()).verify_token()
                        == Some(false)
                    && let Ok(store) = TokenStore::new()
                {
                    let _ = store.delete_token(Platform::GitHub);
                }

                // A surviving credential (a valid ivaldi token, or a gh / env /
                // .netrc one) means there is nothing to do.
                if let Some(existing) = crate::auth::resolve_auth(Platform::GitHub) {
                    let trustworthy = existing.name != "ivaldi"
                        || GitHubClient::with_token(existing.token.clone()).verify_token()
                            != Some(false);
                    if trustworthy {
                        println!("Already authenticated with GitHub via {}.", existing.name);
                        println!("  {}", existing.description);
                        println!(
                            "Ivaldi will use this credential automatically — no new login needed."
                        );
                        println!(
                            "(Run 'ivaldi auth login --force' to mint a separate ivaldi token, or"
                        );
                        println!(
                            " 'ivaldi auth login --with-token' to paste a Personal Access Token.)"
                        );
                        return Ok(());
                    }
                }
            }

            println!("Initiating GitHub authentication...");
            let device_code = GitHubClient::request_device_code().map_err(|e| e.to_string())?;

            println!(
                "\nFirst, copy your one-time code: {}",
                device_code.user_code
            );
            if open_in_browser(&device_code.verification_uri) {
                println!("Opened {} in your browser.", device_code.verification_uri);
                println!("(If nothing opened, visit the URL above manually.)");
            } else {
                println!("Then visit: {}", device_code.verification_uri);
            }
            println!("\nWaiting for authentication...");

            let token =
                GitHubClient::poll_for_token(&device_code.device_code, device_code.interval)
                    .map_err(|e| e.to_string())?;

            let store = TokenStore::new().map_err(|e| e.to_string())?;
            store
                .save_token(Platform::GitHub, token)
                .map_err(|e| e.to_string())?;
            println!("\nAuthentication successful!");
        }
        AuthCommands::Status => {
            use crate::auth;
            for (platform, name) in &[(Platform::GitHub, "GitHub"), (Platform::GitLab, "GitLab")] {
                if let Some(method) = auth::resolve_auth(*platform) {
                    println!("{}: {}", name, method.description);
                } else {
                    println!("{}: Not authenticated", name);
                }
            }
        }
        AuthCommands::Logout(logout_args) => {
            use crate::auth::TokenStore;
            let platform = if logout_args.gitlab {
                Platform::GitLab
            } else {
                Platform::GitHub
            };
            match TokenStore::new() {
                Ok(store) => {
                    store.delete_token(platform).map_err(|e| e.to_string())?;
                    println!("Logged out successfully");
                }
                Err(e) => println!("Warning: {}", e),
            }
        }
    }
    Ok(())
}

pub(super) fn cmd_download(args: DownloadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::ssh_transport::SshTarget;
    use crate::sync;

    // `ivaldi://host[:port][/timeline]` — peer-to-peer transport, no
    // GitHub / GitLab in the loop.
    if let Some(url) = crate::p2p::PeerUrl::parse(&args.repo) {
        use crate::identity;
        use crate::known_peers::TofuPolicy;
        let id_path =
            identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
        let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
        let default_dir = url.timeline.clone().unwrap_or_else(|| url.host.clone());
        let target_dir =
            std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&default_dir));
        let policy = if args.accept_new_peer {
            TofuPolicy::AcceptAll
        } else if args.strict_peer {
            TofuPolicy::StrictKnown
        } else {
            TofuPolicy::Prompt
        };
        let summary = crate::p2p::fetch_into_with_policy(&url, &target_dir, &id, policy)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Cloned ivaldi://{}:{} → {}",
                url.host,
                url.port,
                target_dir.display()
            );
            println!(
                "  timeline: {}, {} leaves, {} objects",
                summary.timeline, summary.leaves_imported, summary.blobs_imported
            );
        }
        return Ok(());
    }

    // Try SSH next — `git@host:owner/repo.git` and `ssh://...` go straight
    // to the SSH transport. Anything that doesn't parse as SSH falls
    // through to the existing HTTPS / GitHub flow.
    if let Some(target) = SshTarget::parse(&args.repo) {
        let default_dir = target
            .repo_path
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or("repo")
            .to_string();
        let target_dir =
            std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&default_dir));
        let result = sync::download_ssh(&target, &target_dir, None).map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Cloned {}@{}:{} → {}",
                target.user,
                target.host,
                target.repo_path,
                target_dir.display()
            );
            println!("  {} files downloaded", result.files_downloaded);
        }
        return Ok(());
    }

    // Generic Git smart-HTTP host (AUR, Gitea, cgit, self-hosted, ...).
    // GitHub/GitLab return None and keep their existing REST-aware path below.
    if let Some((base, owner, repo)) = parse_generic_git_url(&args.repo) {
        let target_dir = std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&repo));
        let result = sync::download_url(&base, &owner, &repo, &target_dir, None, None)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!("Cloned {} → {}", base, target_dir.display());
            println!("  {} files downloaded", result.files_downloaded);
        }
        return Ok(());
    }

    let spec = parse_repo_arg(&args.repo)?;
    let client = GitHubClient::new();

    let target_dir = std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&spec.repo));
    let branch = spec.branch_hint.as_deref();

    let result = sync::download(&client, &spec.owner, &spec.repo, &target_dir, branch)
        .map_err(|e| e.to_string())?;

    if !quiet {
        println!(
            "Cloned {}/{} → {}",
            spec.owner,
            spec.repo,
            target_dir.display()
        );
        println!("  {} files downloaded", result.files_downloaded);
    }
    Ok(())
}

/// Parse a generic Git smart-HTTP URL into `(base_url, owner, repo)`.
///
/// Returns `None` for non-URLs (bare `owner/repo` shorthand) and for
/// github.com/gitlab.com, which keep their platform-specific download path.
/// The base URL is returned verbatim (trailing slash trimmed); owner/repo are
/// display/portal labels derived from the path (host stands in for owner when
/// the path is a single segment, e.g. AUR's `/yay.git`).
pub(super) fn parse_generic_git_url(raw: &str) -> Option<(String, String, String)> {
    let raw = raw.trim();
    let after = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))?;
    let (host, path) = after.split_once('/')?;
    if host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("gitlab.com") {
        return None;
    }
    let path_clean = path.trim_end_matches('/');
    let path_clean = path_clean.strip_suffix(".git").unwrap_or(path_clean);
    let segs: Vec<&str> = path_clean.split('/').filter(|s| !s.is_empty()).collect();
    let (owner, repo) = match segs.as_slice() {
        [] => return None,
        [one] => (host.to_string(), (*one).to_string()),
        many => (
            many[many.len() - 2].to_string(),
            many[many.len() - 1].to_string(),
        ),
    };
    Some((raw.trim_end_matches('/').to_string(), owner, repo))
}

pub(super) fn cmd_upload(args: UploadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::portal::Transport;

    let mut repo = open_repo()?;

    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = resolve_portal(&portal_mgr, args.portal.as_deref())?;

    // SSH push — bypass the GitHub auth check entirely.
    if let Transport::Ssh(target) = portal.transport() {
        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));
        if args.force {
            print!(
                "WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: "
            );
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|e| e.to_string())?;
            if input.trim() != "force push" {
                println!("Aborted.");
                return Ok(());
            }
        }
        let report = crate::ssh_transport::SshClient::new(target)
            .push_repo(&mut repo, &timeline, args.force)
            .map_err(|e| e.to_string())?;

        if !report.unpack_ok {
            return Err(format!(
                "remote rejected pack: {}",
                report.unpack_error.unwrap_or_else(|| "unknown".into())
            ));
        }
        let mut had_failure = false;
        for r in &report.refs {
            match &r.error {
                Some(reason) => {
                    println!("  {} REJECTED: {}", r.name, reason);
                    had_failure = true;
                }
                None => {
                    if !quiet {
                        println!("  {} updated", r.name);
                    }
                }
            }
        }
        if had_failure {
            return Err("one or more refs were rejected".into());
        }
        if !quiet {
            println!("Pushed timeline '{}' over SSH.", timeline);
        }
        return Ok(());
    }

    // Peer-to-peer push — branch out before the GitHub auth check.
    if let Transport::Peer(peer_url) = portal.transport() {
        use crate::identity;
        use crate::known_peers::TofuPolicy;

        let id_path =
            identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
        let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));
        let summary = crate::p2p::push_to(&peer_url, &mut repo, &id, &timeline, TofuPolicy::Prompt)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Pushed timeline '{}' to ivaldi://{}:{}",
                timeline, peer_url.host, peer_url.port
            );
            println!(
                "  landed as: {} ({} leaves, {} objects)",
                summary.landed_as, summary.leaves_sent, summary.objects_sent
            );
        }
        return Ok(());
    }

    // Generic Git smart-HTTP host (AUR, Gitea, cgit, self-hosted). Push
    // straight to the portal's URL; auth resolves per-host (env / .netrc).
    if let Transport::GenericHttps(base_url) = portal.transport() {
        let host = crate::portal::http_host(&base_url).unwrap_or_default();
        let token = crate::auth::generic_git_token(&host);

        if args.force {
            print!(
                "WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: "
            );
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|e| e.to_string())?;
            if input.trim() != "force push" {
                println!("Aborted.");
                return Ok(());
            }
        }

        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

        let report = crate::git_remote::SmartHttpClient::new(token.as_deref())
            .push_repo_url(&mut repo, &base_url, &timeline, args.force)
            .map_err(|e| e.to_string())?;

        if !report.unpack_ok {
            return Err(format!(
                "remote rejected pack: {}",
                report.unpack_error.unwrap_or_else(|| "unknown".into())
            ));
        }
        let mut had_failure = false;
        for r in &report.refs {
            match &r.error {
                Some(reason) => {
                    println!("  {} REJECTED: {}", r.name, reason);
                    had_failure = true;
                }
                None => {
                    if !quiet {
                        println!("  {} updated", r.name);
                    }
                }
            }
        }
        if had_failure {
            return Err("one or more refs were rejected".into());
        }
        if !quiet {
            println!("Uploaded timeline '{}' to {}.", timeline, base_url);
        }
        return Ok(());
    }

    let client = GitHubClient::new();
    if !client.is_authenticated() {
        return Err("not authenticated. Run 'ivaldi auth login' or set GITHUB_TOKEN.".into());
    }

    if args.force {
        print!("WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: ");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|e| e.to_string())?;
        if input.trim() != "force push" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Push the timeline as a single packfile over smart-HTTP
    // `git-receive-pack` (one request), instead of the old per-object REST
    // upload that fired hundreds of requests and tripped GitHub's rate limit.
    let timeline = args
        .branch
        .clone()
        .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

    let report = crate::git_remote::SmartHttpClient::new(client.token())
        .push_repo(
            &mut repo,
            &portal.owner,
            &portal.repo,
            &timeline,
            args.force,
        )
        .map_err(|e| e.to_string())?;

    if !report.unpack_ok {
        return Err(format!(
            "remote rejected pack: {}",
            report.unpack_error.unwrap_or_else(|| "unknown".into())
        ));
    }
    let mut had_failure = false;
    for r in &report.refs {
        match &r.error {
            Some(reason) => {
                println!("  {} REJECTED: {}", r.name, reason);
                had_failure = true;
            }
            None => {
                if !quiet {
                    println!("  {} updated", r.name);
                }
            }
        }
    }
    if had_failure {
        return Err("one or more refs were rejected".into());
    }
    if !quiet {
        println!(
            "Uploaded timeline '{}' to {}/{}.",
            timeline, portal.owner, portal.repo
        );
    }
    Ok(())
}

pub(super) fn cmd_scout(_args: ScoutArgs) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    let branches = sync::scout_with_status(&client, &repo, &portal).map_err(|e| e.to_string())?;

    println!("Remote timelines available:");
    for branch in &branches {
        let status = match branch.state {
            sync::RemoteTimelineState::NotDownloaded => "not downloaded",
            sync::RemoteTimelineState::UpToDate => "local, up to date",
            sync::RemoteTimelineState::OutOfSync => "local, out of sync",
            sync::RemoteTimelineState::LocalOnly => "local, no remote mapping",
        };
        println!("  {} [{}]", branch.name, status);
    }
    println!("\nUse 'ivaldi harvest <name>' to download");
    Ok(())
}

pub(super) fn cmd_harvest(args: HarvestArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let mut repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    if args.timelines.is_empty() {
        // List available and prompt
        let branches = sync::scout(&client, &portal).map_err(|e| e.to_string())?;
        println!("Available remote timelines:");
        for b in &branches {
            println!("  {}", b);
        }
        println!("\nSpecify timelines to harvest: ivaldi harvest <name> [<name>...]");
        return Ok(());
    }

    let harvested =
        sync::harvest(&client, &mut repo, &portal, &args.timelines).map_err(|e| e.to_string())?;

    if !quiet {
        for name in &harvested {
            println!("Harvested timeline: {}", name);
        }
    }
    Ok(())
}

pub(super) fn cmd_sync(args: SyncArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let mut repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = resolve_portal(&portal_mgr, args.portal.as_deref())?;

    let timeline = args
        .timeline
        .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

    if !quiet {
        println!(
            "Syncing timeline '{}' with {}/{}...\n",
            color::timeline(&timeline),
            portal.owner,
            portal.repo
        );
        if args.force {
            println!(
                "{}",
                color::yellow("--force: uncommitted changes will be overwritten")
            );
        }
    }

    // Consent-first: the sync fetches and reports what's incoming, then asks
    // before integrating anything. `--yes` (or a piped stdin with --yes)
    // skips the prompt; non-interactive without --yes declines safely.
    let mut consent = |incoming: usize, local: usize| -> bool {
        if args.yes {
            return true;
        }
        let what = if local == 0 {
            format!(
                "{} incoming seal(s) would fast-forward '{}'",
                incoming, timeline
            )
        } else {
            format!(
                "{} incoming seal(s) would fuse with {} local seal(s) on '{}'",
                incoming, local, timeline
            )
        };
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "{}. Refusing to integrate without confirmation in a non-interactive \
                 session — rerun with 'ivaldi sync --yes'.",
                what
            );
            return false;
        }
        eprint!("{}. Pull them in? [y/N] ", what);
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    };

    let result = match sync::sync_timeline(
        &client,
        &mut repo,
        &portal.owner,
        &portal.repo,
        &timeline,
        &mut consent,
        args.force,
    ) {
        Ok(result) => result,
        // Declining is a clean outcome, not a failure: nothing was mutated.
        Err(sync::SyncError::Declined) => {
            println!(
                "Sync declined — timeline '{}' is unchanged. Run 'ivaldi sync' again when ready.",
                color::bold(&timeline)
            );
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    };

    if result.no_changes {
        println!(
            "{} Timeline '{}' is already up to date",
            color::green("✓"),
            color::bold(&timeline)
        );
        return Ok(());
    }

    for f in &result.added {
        println!("{} {}", color::green("++"), f);
    }
    for f in &result.modified {
        println!("{} {}", color::green("++"), f);
    }
    for f in &result.deleted {
        println!("{} {}", color::red("--"), f);
    }

    let total = result.added.len() + result.modified.len() + result.deleted.len();
    println!(
        "\n{} Synced {} file(s) from remote",
        color::green("✓"),
        total
    );
    if !result.added.is_empty() {
        println!("  Added: {}", color::green(&result.added.len().to_string()));
    }
    if !result.modified.is_empty() {
        println!(
            "  Modified: {}",
            color::blue(&result.modified.len().to_string())
        );
    }
    if !result.deleted.is_empty() {
        println!(
            "  Deleted: {}",
            color::red(&result.deleted.len().to_string())
        );
    }

    Ok(())
}

pub(super) fn cmd_serve(args: ServeArgs, _quiet: bool) -> Result<(), String> {
    use crate::identity;
    use crate::p2p;

    let ctx = find_repo()?;

    let id_path =
        identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
    let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
    let peer_store_path = ctx.ivaldi_dir.join("authorized_peers");
    p2p::serve(&args.bind, ctx.work_dir.clone(), &id, peer_store_path)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn cmd_peer(args: PeerArgs, _quiet: bool) -> Result<(), String> {
    use crate::identity;
    use crate::peers::{PeerStore, decode_pubkey};

    match args.command {
        PeerCommands::Whoami => {
            let id_path =
                identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
            let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
            println!("{}", id.pubkey_hex());
            Ok(())
        }
        PeerCommands::Trust(t) => {
            let ctx = find_repo()?;
            let pubkey = decode_pubkey(&t.pubkey)?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            store
                .trust(pubkey, t.name.as_deref())
                .map_err(|e| e.to_string())?;
            println!(
                "Trusted peer {}{}",
                t.pubkey,
                t.name
                    .as_ref()
                    .map(|n| format!(" ({})", n))
                    .unwrap_or_default()
            );
            Ok(())
        }
        PeerCommands::List => {
            let ctx = find_repo()?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            let entries = store.list().map_err(|e| e.to_string())?;
            if entries.is_empty() {
                println!("(no trusted peers)");
            } else {
                for e in entries {
                    let key = e.pubkey_hex();
                    match e.name {
                        Some(n) => println!("{}  {}", key, n),
                        None => println!("{}", key),
                    }
                }
            }
            Ok(())
        }
        PeerCommands::Forget(f) => {
            let ctx = find_repo()?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            match store.forget(&f.prefix).map_err(|e| e.to_string())? {
                Some(removed) => {
                    println!("Forgot {}", removed.pubkey_hex());
                    Ok(())
                }
                None => Err(format!("no trusted peer matches '{}'", f.prefix)),
            }
        }
        PeerCommands::Known(known) => {
            use crate::known_peers::{KnownPeers, fingerprint};
            let store = KnownPeers::default_for_user()
                .ok_or("could not resolve $HOME for ~/.ivaldi/known_peers")?;
            match known.command {
                PeerKnownCommands::List => {
                    let entries = store.list().map_err(|e| e.to_string())?;
                    if entries.is_empty() {
                        println!("(no known peers)");
                    } else {
                        for (key, pk) in entries {
                            println!("{}  {}", key, fingerprint(&pk));
                        }
                    }
                    Ok(())
                }
                PeerKnownCommands::Forget(f) => {
                    let (host, port) = match f.host.rsplit_once(':') {
                        Some((h, p)) => match p.parse::<u16>() {
                            Ok(n) => (h.to_string(), n),
                            Err(_) => (f.host.clone(), crate::p2p::DEFAULT_PORT),
                        },
                        None => (f.host.clone(), crate::p2p::DEFAULT_PORT),
                    };
                    let removed = store.forget(&host, port).map_err(|e| e.to_string())?;
                    if removed {
                        println!("Forgot {}:{}", host, port);
                        Ok(())
                    } else {
                        Err(format!("no known peer at {}:{}", host, port))
                    }
                }
            }
        }
    }
}
