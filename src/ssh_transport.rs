//! SSH transport for Git's pack protocol.
//!
//! Implements `fetch_repo` / `list_branch_refs` against any SSH-reachable
//! Git host (github.com, gitlab.com, self-hosted, gitea, etc.) by spawning
//! the system `ssh` binary as a subprocess and speaking pack-protocol over
//! its stdin/stdout. We deliberately don't link in an in-process SSH stack:
//! `ssh` on the user's PATH already knows about their keys, agent,
//! known_hosts, and config — anything we'd reimplement we'd implement worse.
//!
//! Push (`git-receive-pack`) is intentionally out of scope for this slice.
//! See `src/sync.rs` for upload, which still goes through GitHub's REST API.
//!
//! Wire shape (fetch):
//!
//! ```text
//!   ssh user@host git-upload-pack 'path/to/repo.git'
//!   <stdin>:  pkt-lines (`want <sha> <caps>` ... `0000` ... `done\n`)
//!   <stdout>: ref advertisement (no `# service=` prefix), then sideband packfile
//! ```

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::git_remote::{
    FetchResult, GitRemoteError, extract_pack_from_upload_pack, parse_discovery, parse_packfile,
    select_branch_from_discovery,
};
use crate::progress;
use crate::remote::RemoteBranch;

/// Parsed SSH URL.
///
/// Accepts both forms:
/// - `git@host:owner/repo[.git]`            (scp-like)
/// - `ssh://[user@]host[:port]/owner/repo[.git]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
    /// The repo path passed verbatim to `git-upload-pack`. Includes the
    /// `.git` suffix if it was present in the URL.
    pub repo_path: String,
}

impl SshTarget {
    /// Parse one of the two accepted SSH URL forms. Returns `None` if the
    /// input doesn't look like an SSH git URL.
    pub fn parse(url: &str) -> Option<Self> {
        // ssh://[user@]host[:port]/path
        if let Some(rest) = url.strip_prefix("ssh://") {
            let (userhost_port, path) = rest.split_once('/')?;
            let (user, hostport) = match userhost_port.split_once('@') {
                Some((u, hp)) => (u.to_string(), hp),
                None => ("git".to_string(), userhost_port),
            };
            let (host, port) = match hostport.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().ok()),
                None => (hostport.to_string(), None),
            };
            return Some(SshTarget {
                user,
                host,
                port,
                repo_path: path.to_string(),
            });
        }

        // user@host:path  (scp-like). We require an explicit `user@` prefix
        // to disambiguate from `host:port/path`-style URLs that don't carry
        // a scheme. Within the scp form the path may be absolute
        // (`git@host:/abs/path`) — that's still scp, not host:port.
        if !url.contains("://")
            && let Some((userhost, path)) = url.split_once(':')
        {
            let (user, host) = match userhost.split_once('@') {
                Some((u, h)) => (u.to_string(), h.to_string()),
                None => return None, // bare `host:path` is ambiguous; require user@.
            };
            if host.is_empty() || path.is_empty() {
                return None;
            }
            return Some(SshTarget {
                user,
                host,
                port: None,
                repo_path: path.to_string(),
            });
        }
        None
    }

    /// Build the `ssh ...` argument vector. Doesn't include the actual remote
    /// command — caller appends `git-upload-pack` etc.
    fn ssh_command_prefix(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ServerAliveInterval=30".into(),
        ];
        if let Some(p) = self.port {
            v.push("-p".into());
            v.push(p.to_string());
        }
        v.push(format!("{}@{}", self.user, self.host));
        v
    }
}

/// Speaks Git's pack protocol over SSH. Spawned per call (one ssh process
/// per fetch / list).
pub struct SshClient {
    target: SshTarget,
}

impl SshClient {
    pub fn new(target: SshTarget) -> Self {
        Self { target }
    }

    /// List branch refs for a repo via `ls-remote`-style ref advertisement.
    /// Spawns `ssh ... git-upload-pack '<repo>'`, reads the advertisement,
    /// then closes the stream (no `want` lines sent).
    pub fn list_branch_refs(&self) -> Result<Vec<RemoteBranch>, GitRemoteError> {
        let session = self.spawn("git-upload-pack")?;
        let (discovery, _stdin, child) = read_advertisement(session)?;
        // Close stdin to terminate the upload-pack negotiation cleanly.
        finish_child(child)?;

        let mut branches: Vec<RemoteBranch> = discovery
            .refs
            .into_iter()
            .filter_map(|r| {
                r.name.strip_prefix("refs/heads/").map(|name| RemoteBranch {
                    name: name.to_string(),
                    sha1: r.id,
                })
            })
            .collect();
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        branches.dedup_by(|a, b| a.name == b.name);
        Ok(branches)
    }

    /// Fetch the requested branch (or default branch if `None`).
    pub fn fetch_repo(&self, branch: Option<&str>) -> Result<FetchResult, GitRemoteError> {
        let session = self.spawn("git-upload-pack")?;
        let (discovery, mut stdin, mut child) = read_advertisement(session)?;
        let (branch_name, head_sha) = select_branch_from_discovery(&discovery, branch)?;

        // Send the upload-pack request body: one `want` + flush + `done`.
        let caps =
            "multi_ack_detailed side-band-64k ofs-delta no-progress include-tag agent=ivaldi/0.1.0";
        let mut body = Vec::new();
        body.extend(crate::git_remote::pkt_line(&format!(
            "want {} {}\n",
            head_sha, caps
        )));
        body.extend_from_slice(b"0000");
        body.extend(crate::git_remote::pkt_line("done\n"));
        stdin
            .write_all(&body)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        // Flushing stdin (don't drop yet — git-upload-pack may want to send
        // back NAK / ACK before the pack itself).
        stdin
            .flush()
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;

        // Drain stdout (NAK + sideband-multiplexed pack).
        let pb = progress::spinner("Downloading pack (ssh)");
        let mut response = Vec::new();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| GitRemoteError::Io("ssh child has no stdout".into()))?;
        let mut chunk = [0u8; 8192];
        loop {
            let n = stdout
                .read(&mut chunk)
                .map_err(|e| GitRemoteError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..n]);
            pb.inc(n as u64);
        }
        pb.finish_with_message(format!("ssh pack downloaded ({} bytes)", response.len()));
        finish_child(child)?;

        let pack = extract_pack_from_upload_pack(&response)?;
        let objects = parse_packfile(&pack)?;
        Ok(FetchResult {
            branch: branch_name,
            head_sha,
            refs: discovery.refs,
            objects,
        })
    }

    /// Push the given branch's history to the remote via `git-receive-pack`.
    ///
    /// Reads the remote's ref advertisement to discover the current tip
    /// (so we can produce the correct `<old> <new> ref` update command),
    /// translates `repo`'s leaf chain into git objects via
    /// [`crate::git_export::export_chain`], packs them via
    /// [`crate::git_pack_writer::write_pack`], and streams everything
    /// through ssh's stdin. Parses the report-status response and surfaces
    /// any per-ref `ng` reasons.
    pub fn push_repo(
        &self,
        repo: &mut crate::repo::Repo,
        branch: &str,
        force: bool,
    ) -> Result<PushReport, GitRemoteError> {
        use crate::git_export;
        use crate::git_pack_writer;
        use crate::git_remote::{parse_discovery, pkt_line};
        use crate::remote::HashMapping;

        // ---- Resolve local head.
        let head_idx = repo
            .get_timeline_head(branch)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
            .ok_or_else(|| {
                GitRemoteError::Protocol(format!("local timeline '{}' has no head", branch))
            })?;

        // ---- Connect; read advertisement. We extract the three streams
        // up front because we need stdout *after* the advertisement (to
        // read the report-status response) — `read_advertisement` would
        // have consumed it.
        let mut child = self.spawn("git-receive-pack")?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| GitRemoteError::Io("ssh child has no stdin".into()))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| GitRemoteError::Io("ssh child has no stdout".into()))?;
        let stderr = child.stderr.take();

        let adv_bytes = match read_until_flush_owned(&mut stdout) {
            Ok(b) => b,
            Err(e) => {
                drop(stdin);
                let _ = child.wait();
                let mut buf = Vec::new();
                if let Some(mut s) = stderr {
                    let _ = s.read_to_end(&mut buf);
                }
                let stderr_text = String::from_utf8_lossy(&buf).trim().to_string();
                if stderr_text.is_empty() {
                    return Err(e);
                }
                return Err(GitRemoteError::Io(format!("ssh: {}", stderr_text)));
            }
        };
        let discovery = crate::git_remote::parse_discovery(&adv_bytes)
            .map_err(|e| GitRemoteError::Protocol(format!("receive-pack advertisement: {}", e)))?;

        let target_ref = format!("refs/heads/{}", branch);
        let old_sha1 = discovery
            .refs
            .iter()
            .find(|r| r.name == target_ref)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| "0".repeat(40));
        let _ = parse_discovery; // silence unused-import in some build configs

        // ---- Translate Ivaldi history to git objects.
        // Build the set of SHA-1s the server already has from its
        // advertisement, so the exporter only skips ancestors actually
        // present on this remote (not ones merely seen on some prior
        // remote via a different portal).
        let server_has: std::collections::BTreeSet<[u8; 20]> = discovery
            .refs
            .iter()
            .filter_map(|r| {
                if r.id == "0".repeat(40) {
                    None
                } else {
                    let mut bytes = [0u8; 20];
                    let raw = hex::decode(&r.id).ok()?;
                    if raw.len() == 20 {
                        bytes.copy_from_slice(&raw);
                        Some(bytes)
                    } else {
                        None
                    }
                }
            })
            .collect();
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let export = git_export::export_chain(repo, head_idx, &mapping, &server_has)
            .map_err(|e| GitRemoteError::Protocol(format!("git export: {}", e)))?;

        // No new commits and the ref is already at the target sha →
        // nothing to push. Refuse early so we don't send an empty pack.
        if export.objects.is_empty() {
            // Force-update is allowed only when the remote ref points
            // somewhere different; we don't surface that as "no work" so
            // the caller can decide whether it was intentional.
            return Err(GitRemoteError::Protocol(
                "nothing to push: every commit on this branch is already on the remote".into(),
            ));
        }

        let new_sha1_hex = hex::encode(export.tip_sha1);
        if new_sha1_hex == old_sha1 {
            return Err(GitRemoteError::Protocol(
                "nothing to push: remote tip already matches local tip".into(),
            ));
        }

        // ---- Build update command line. Capabilities go after the FIRST
        // command line (per receive-pack protocol).
        let mut command_line = format!("{} {} {}", old_sha1, new_sha1_hex, target_ref);
        // `report-status` is needed so the server tells us whether the
        // push succeeded; `agent` is informational. We do NOT request
        // side-band-64k for v1 — keeps the response parser trivial.
        let caps = "report-status agent=ivaldi/0.1.0";
        command_line.push('\0');
        command_line.push_str(caps);
        command_line.push('\n');

        // For non-force pushes: refuse if the remote isn't at our parent
        // chain. Receive-pack itself will reject with "non-fast-forward",
        // but we can also short-circuit. We don't bother — let the server
        // be the source of truth. `force` is plumbed through so the
        // caller's intent is documented; non-FF still surfaces as `ng`.
        let _ = force;

        // ---- Send command + flush + packfile.
        stdin
            .write_all(&pkt_line(&command_line))
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        stdin
            .write_all(b"0000")
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;

        let mut object_refs: Vec<&crate::git_export::GitObject> = export.objects.values().collect();
        // Stable order — receivers don't care, but determinism helps debugging.
        object_refs.sort_by_key(|o| o.sha1);
        let pack = git_pack_writer::write_pack(&object_refs)
            .map_err(|e| GitRemoteError::Protocol(e.to_string()))?;
        stdin
            .write_all(&pack)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        stdin
            .flush()
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        // Close stdin so the server sees EOF on its input.
        drop(stdin);

        // ---- Read report-status response.
        let mut response = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stdout
                .read(&mut chunk)
                .map_err(|e| GitRemoteError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..n]);
        }
        // Restore stderr into the child so finish_child can attribute
        // failures correctly.
        if let Some(s) = stderr {
            child.stderr = Some(s);
        }
        finish_child(child)?;

        let report = parse_report_status(&response)?;
        // After a successful push, record the new mapping locally so the
        // next push can short-circuit. Non-fatal if save fails.
        if report.unpack_ok && report.refs.iter().all(|r| r.error.is_none()) {
            let mut mapping = HashMapping::new(&repo.ivaldi_dir);
            // Newly minted commit → leaf hash. We need to look up the leaf
            // index for `head_idx` (we already have it).
            if let Ok(Some(leaf)) = repo.get_leaf(head_idx) {
                mapping.insert(&new_sha1_hex, leaf.hash());
                let _ = mapping.save();
            }
        }

        Ok(report)
    }

    /// Spawn `ssh <prefix> <remote_cmd> '<repo_path>'`. Returns the child
    /// with piped stdin + stdout.
    fn spawn(&self, remote_cmd: &str) -> Result<Child, GitRemoteError> {
        let mut args = self.target.ssh_command_prefix();
        // Quote the repo path for the remote shell. Single quotes are safe
        // for everything except `'`, which we escape `\''`-style.
        let quoted = quote_repo_path(&self.target.repo_path);
        args.push(format!("{} {}", remote_cmd, quoted));

        Command::new("ssh")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                GitRemoteError::Io(format!(
                    "failed to spawn ssh — is the `ssh` binary on your PATH? ({})",
                    e
                ))
            })
    }
}

/// One ref's outcome from `git-receive-pack`'s report-status block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedRef {
    pub name: String,
    /// `Some(reason)` if the server rejected this ref's update.
    pub error: Option<String>,
}

/// Parsed `report-status` response. The `unpack_ok` flag covers the
/// pack-receipt phase; `refs` carries one entry per pushed ref.
#[derive(Debug, Clone)]
pub struct PushReport {
    pub unpack_ok: bool,
    pub unpack_error: Option<String>,
    pub refs: Vec<PushedRef>,
}

/// Parse a `report-status` block from `git-receive-pack`. Format per
/// gitprotocol-pack(5):
///
/// ```text
///   "unpack ok\n" | "unpack <error>\n"
///   ( "ok <ref>\n" | "ng <ref> <reason>\n" )*
///   flush-pkt
/// ```
fn parse_report_status(data: &[u8]) -> Result<PushReport, GitRemoteError> {
    let lines = parse_pkt_text_lines(data)?;
    let mut iter = lines.into_iter();

    let first = iter
        .next()
        .ok_or_else(|| GitRemoteError::Protocol("empty receive-pack report".into()))?;
    let (unpack_ok, unpack_error) = if first == "unpack ok" {
        (true, None)
    } else if let Some(rest) = first.strip_prefix("unpack ") {
        (false, Some(rest.to_string()))
    } else {
        return Err(GitRemoteError::Protocol(format!(
            "unexpected receive-pack first line: {:?}",
            first
        )));
    };

    let mut refs = Vec::new();
    for line in iter {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("ok ") {
            refs.push(PushedRef {
                name: rest.to_string(),
                error: None,
            });
        } else if let Some(rest) = line.strip_prefix("ng ") {
            // "ng <ref> <reason>"
            let mut parts = rest.splitn(2, ' ');
            let name = parts.next().unwrap_or_default().to_string();
            let reason = parts.next().unwrap_or("rejected").to_string();
            refs.push(PushedRef {
                name,
                error: Some(reason),
            });
        }
        // Anything else we ignore — receive-pack also sends progress
        // sideband when `side-band-64k` was negotiated; we didn't ask
        // for it but be lenient.
    }
    Ok(PushReport {
        unpack_ok,
        unpack_error,
        refs,
    })
}

/// Read pkt-lines and return their UTF-8 payloads (newline-stripped).
/// Flush packets terminate the stream.
fn parse_pkt_text_lines(data: &[u8]) -> Result<Vec<String>, GitRemoteError> {
    let mut idx = 0usize;
    let mut out = Vec::new();
    while idx + 4 <= data.len() {
        let len_hex = std::str::from_utf8(&data[idx..idx + 4])
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        idx += 4;
        let len = usize::from_str_radix(len_hex, 16)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        if len == 0 {
            // flush — stop here even if more data follows
            break;
        }
        if len < 4 || idx + (len - 4) > data.len() {
            return Err(GitRemoteError::Protocol("truncated pkt-line".into()));
        }
        let payload = &data[idx..idx + (len - 4)];
        idx += len - 4;
        let text = std::str::from_utf8(payload)
            .map_err(|_| GitRemoteError::Protocol("non-UTF-8 pkt-line text".into()))?
            .trim_end_matches('\n')
            .to_string();
        out.push(text);
    }
    Ok(out)
}

/// Quote a single argument for `sh -c` execution on the remote side. Wraps
/// in single quotes and escapes embedded single quotes the standard way.
fn quote_repo_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 2);
    out.push('\'');
    for c in p.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Read the initial ref advertisement from a freshly-spawned upload-pack
/// session. Returns the parsed discovery, the still-open stdin (so the
/// caller can send a `want`/`done` body), and the child handle for cleanup.
fn read_advertisement(
    mut child: Child,
) -> Result<(crate::git_remote::Discovery, ChildStdin, Child), GitRemoteError> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| GitRemoteError::Io("ssh child has no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| GitRemoteError::Io("ssh child has no stdout".into()))?;

    match read_until_flush(stdout) {
        Ok(bytes) => {
            let discovery = parse_discovery(&bytes)?;
            Ok((discovery, stdin, child))
        }
        Err(e) => {
            // If ssh died before producing the advertisement (auth
            // failure, unknown host, repo-not-found, etc.), surface its
            // stderr so the user sees the real diagnostic.
            drop(stdin);
            let _ = child.wait();
            let mut stderr_bytes = Vec::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_end(&mut stderr_bytes);
            }
            let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
            if stderr.is_empty() {
                Err(e)
            } else {
                Err(GitRemoteError::Io(format!("ssh: {}", stderr)))
            }
        }
    }
}

/// Like [`read_until_flush`] but takes a mutable reference so the caller
/// keeps the underlying handle for subsequent reads (the receive-pack
/// flow needs stdout twice: once for the advertisement, then again for
/// the report-status response).
fn read_until_flush_owned(stdout: &mut ChildStdout) -> Result<Vec<u8>, GitRemoteError> {
    let mut buf = Vec::new();
    let mut header = [0u8; 4];
    loop {
        if let Err(e) = stdout.read_exact(&mut header) {
            return Err(GitRemoteError::Protocol(format!(
                "ssh stream ended without ref-advertisement flush ({})",
                e
            )));
        }
        buf.extend_from_slice(&header);
        let len_str = std::str::from_utf8(&header)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        let len = usize::from_str_radix(len_str, 16)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        if len == 0 {
            return Ok(buf);
        }
        if len < 4 {
            return Err(GitRemoteError::Protocol(format!(
                "pkt-line length {} too small",
                len
            )));
        }
        let payload_len = len - 4;
        let mut payload = vec![0u8; payload_len];
        stdout
            .read_exact(&mut payload)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        buf.extend_from_slice(&payload);
    }
}

/// Read pkt-lines until (and including) the first `0000` flush. Returns the
/// raw bytes for `parse_discovery` to consume.
fn read_until_flush(mut stdout: ChildStdout) -> Result<Vec<u8>, GitRemoteError> {
    let mut buf = Vec::new();
    let mut header = [0u8; 4];
    loop {
        if let Err(e) = stdout.read_exact(&mut header) {
            // EOF before flush — treat as protocol error rather than IO so
            // the user sees a sensible message.
            return Err(GitRemoteError::Protocol(format!(
                "ssh stream ended without ref-advertisement flush ({})",
                e
            )));
        }
        buf.extend_from_slice(&header);
        let len_str = std::str::from_utf8(&header)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        let len = usize::from_str_radix(len_str, 16)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        if len == 0 {
            return Ok(buf); // flush
        }
        if len < 4 {
            return Err(GitRemoteError::Protocol(format!(
                "pkt-line length {} too small",
                len
            )));
        }
        let payload_len = len - 4;
        let mut payload = vec![0u8; payload_len];
        stdout
            .read_exact(&mut payload)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        buf.extend_from_slice(&payload);
    }
}

/// Wait for the ssh child to exit, surfacing a useful error including stderr
/// when ssh failed (most common: BatchMode auth failure).
fn finish_child(mut child: Child) -> Result<(), GitRemoteError> {
    let status = child
        .wait()
        .map_err(|e| GitRemoteError::Io(format!("waiting on ssh: {}", e)))?;
    if !status.success() {
        let mut stderr_bytes = Vec::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_end(&mut stderr_bytes);
        }
        let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        return Err(GitRemoteError::Io(format!(
            "ssh exited with status {} — {}",
            status,
            if stderr.is_empty() {
                "(no stderr; check your SSH agent / known_hosts)".into()
            } else {
                stderr
            }
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scp_form() {
        let t = SshTarget::parse("git@github.com:owner/repo.git").unwrap();
        assert_eq!(t.user, "git");
        assert_eq!(t.host, "github.com");
        assert_eq!(t.port, None);
        assert_eq!(t.repo_path, "owner/repo.git");
    }

    #[test]
    fn parse_ssh_url_no_port() {
        let t = SshTarget::parse("ssh://git@gitlab.com/owner/repo.git").unwrap();
        assert_eq!(t.user, "git");
        assert_eq!(t.host, "gitlab.com");
        assert_eq!(t.port, None);
        assert_eq!(t.repo_path, "owner/repo.git");
    }

    #[test]
    fn parse_ssh_url_with_port() {
        let t = SshTarget::parse("ssh://git@example.com:2222/team/proj.git").unwrap();
        assert_eq!(t.user, "git");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, Some(2222));
        assert_eq!(t.repo_path, "team/proj.git");
    }

    #[test]
    fn parse_ssh_url_default_user() {
        // `ssh://host/path` defaults user to `git`.
        let t = SshTarget::parse("ssh://example.com/team/proj.git").unwrap();
        assert_eq!(t.user, "git");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.repo_path, "team/proj.git");
    }

    #[test]
    fn rejects_https_url() {
        assert_eq!(SshTarget::parse("https://github.com/owner/repo.git"), None);
    }

    #[test]
    fn rejects_bare_host_path() {
        // `host:path` without `user@` is ambiguous (could be scp form, could
        // be drive letter on Windows). We require `user@host:path`.
        assert_eq!(SshTarget::parse("github.com:owner/repo.git"), None);
    }

    #[test]
    fn rejects_host_port_path_disguised_as_scp() {
        // `host:port/path` (note leading slash on path) is NOT scp form.
        assert_eq!(SshTarget::parse("github.com:22/owner/repo.git"), None);
    }

    #[test]
    fn quotes_repo_path_with_apostrophe() {
        assert_eq!(quote_repo_path("foo's-repo.git"), "'foo'\\''s-repo.git'");
        assert_eq!(quote_repo_path("plain/repo.git"), "'plain/repo.git'");
    }

    #[test]
    fn ssh_command_prefix_includes_port_when_set() {
        let t = SshTarget {
            user: "git".into(),
            host: "h".into(),
            port: Some(2222),
            repo_path: "r.git".into(),
        };
        let pre = t.ssh_command_prefix();
        assert!(pre.iter().any(|a| a == "-p"));
        assert!(pre.iter().any(|a| a == "2222"));
        assert!(pre.iter().any(|a| a == "git@h"));
    }

    #[test]
    fn ssh_command_prefix_omits_port_when_unset() {
        let t = SshTarget {
            user: "git".into(),
            host: "h".into(),
            port: None,
            repo_path: "r.git".into(),
        };
        let pre = t.ssh_command_prefix();
        assert!(!pre.iter().any(|a| a == "-p"));
    }

    fn pkt(payload: &str) -> Vec<u8> {
        crate::git_remote::pkt_line(payload)
    }

    #[test]
    fn parse_report_status_unpack_ok_with_one_ref() {
        let mut bytes = Vec::new();
        bytes.extend(pkt("unpack ok\n"));
        bytes.extend(pkt("ok refs/heads/main\n"));
        bytes.extend(b"0000");
        let r = parse_report_status(&bytes).unwrap();
        assert!(r.unpack_ok);
        assert!(r.unpack_error.is_none());
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].name, "refs/heads/main");
        assert!(r.refs[0].error.is_none());
    }

    #[test]
    fn parse_report_status_unpack_failure() {
        let mut bytes = Vec::new();
        bytes.extend(pkt("unpack invalid pack\n"));
        bytes.extend(b"0000");
        let r = parse_report_status(&bytes).unwrap();
        assert!(!r.unpack_ok);
        assert_eq!(r.unpack_error.as_deref(), Some("invalid pack"));
        assert!(r.refs.is_empty());
    }

    #[test]
    fn parse_report_status_per_ref_ng_with_reason() {
        let mut bytes = Vec::new();
        bytes.extend(pkt("unpack ok\n"));
        bytes.extend(pkt("ng refs/heads/main non-fast-forward\n"));
        bytes.extend(pkt("ok refs/heads/feat\n"));
        bytes.extend(b"0000");
        let r = parse_report_status(&bytes).unwrap();
        assert!(r.unpack_ok);
        assert_eq!(r.refs.len(), 2);
        assert_eq!(r.refs[0].name, "refs/heads/main");
        assert_eq!(r.refs[0].error.as_deref(), Some("non-fast-forward"));
        assert_eq!(r.refs[1].name, "refs/heads/feat");
        assert!(r.refs[1].error.is_none());
    }

    #[test]
    fn parse_report_status_rejects_garbage_first_line() {
        let mut bytes = Vec::new();
        bytes.extend(pkt("garbage\n"));
        bytes.extend(b"0000");
        assert!(parse_report_status(&bytes).is_err());
    }
}
