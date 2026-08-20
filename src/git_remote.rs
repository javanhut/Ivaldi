//! Pure Rust remote fetch support for Git-compatible Smart HTTP servers.
//!
//! This module implements the minimal subset needed by `ivaldi download`:
//! ref advertisement, upload-pack fetch, packfile parsing, and object import.

use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read, Write};

use base64::Engine;
use flate2::Compression;
use flate2::bufread::ZlibDecoder;
use flate2::write::ZlibEncoder;
use indicatif::MultiProgress;
use rayon::prelude::*;

use crate::hash::B3Hash;
use crate::progress;
use crate::remote::RemoteBranch;

const GITHUB_BASE: &str = "https://github.com";

/// Capabilities advertised on the first `want` line. `shallow` is what earns
/// the right to send `deepen` and to receive the server's `shallow` boundary
/// lines back; the rest are the framing/encoding we already handle.
pub(crate) const UPLOAD_PACK_CAPS: &str =
    "multi_ack_detailed side-band-64k ofs-delta shallow no-progress include-tag agent=ivaldi/0.1.0";

/// Object ids to `want` on top of the branch tip so a fetch also carries the
/// repository's tags.
///
/// Advertised `refs/tags/*` only; the peeled `^{}` companions resolve to
/// objects the tag itself already reaches. Duplicates and the branch tip are
/// dropped so the request never asks for the same object twice.
pub(crate) fn tag_wants(refs: &[AdvertisedRef], head_sha: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    seen.insert(head_sha.to_string());
    refs.iter()
        .filter(|r| r.name.starts_with("refs/tags/") && !r.name.ends_with("^{}"))
        .filter(|r| seen.insert(r.id.clone()))
        .map(|r| r.id.clone())
        .collect()
}

/// Hard ceiling on a single decoded git object (mirrors
/// `pack::MAX_DELTA_OUTPUT`). Any host or MITM'd anonymous clone can hand us
/// a pack; a forged size varint must never drive a giant allocation.
const MAX_GIT_OBJECT_SIZE: usize = 1 << 30; // 1 GiB
/// Maximum nesting depth for tree walks / imports. Real repos are a few
/// dozen levels deep; an attacker-shaped pack can nest trees (or reference a
/// tree from itself) to blow the stack of a recursive walker.
const MAX_TREE_DEPTH: usize = 512;

/// Build an HTTP Basic auth header for a GitHub token.
///
/// GitHub's smart-HTTP git endpoints (`github.com/.../info/refs`,
/// `git-upload-pack`) accept tokens via Basic auth with `x-access-token` as the
/// username. Bearer tokens work for `api.github.com` but are not consistently
/// accepted on the git endpoints, so we use Basic here.
fn basic_auth_header(token: &str) -> String {
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{}", token));
    format!("Basic {}", encoded)
}

#[derive(Debug, Clone)]
pub struct AdvertisedRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub branch: String,
    pub head_sha: String,
    pub refs: Vec<AdvertisedRef>,
    pub objects: HashMap<String, GitObject>,
    /// Commits the server declared as shallow boundaries: their parents were
    /// deliberately left out of the pack, so importing them as roots is
    /// correct rather than corruption. Empty for a full fetch, where a
    /// missing parent still means a broken pack and import must refuse.
    pub shallow: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct GitObject {
    pub kind: GitObjectKind,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitObjectKind {
    Commit,
    Tree,
    Blob,
    Tag,
}

#[derive(Debug, Clone)]
pub struct ParsedCommit {
    pub tree: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub author_tz: String,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub committer_tz: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub name: String,
    pub sha: String,
}

pub struct SmartHttpClient {
    token: Option<String>,
    agent: ureq::Agent,
    depth: usize,
    include_tags: bool,
}

impl SmartHttpClient {
    pub fn new(token: Option<&str>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            token: token.map(str::to_string),
            agent,
            depth: 0,
            include_tags: false,
        }
    }

    /// Truncate the fetch to `depth` commits of history per branch tip
    /// (0 = full history, the default). Sent to the server as `deepen`, so
    /// the pack itself is smaller — this is a bandwidth fix, not just a
    /// display filter.
    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Also fetch the objects behind `refs/tags/*`. Off by default: tags
    /// commonly point outside the branch being cloned, so including them can
    /// pull in a great deal of extra history.
    pub fn with_tags(mut self, include_tags: bool) -> Self {
        self.include_tags = include_tags;
        self
    }

    pub fn fetch_repo(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<FetchResult, GitRemoteError> {
        self.fetch_repo_url(&format!("{}/{}/{}.git", GITHUB_BASE, owner, repo), branch)
    }

    /// Clone from an explicit Git smart-HTTP base URL rather than a GitHub
    /// owner/repo — e.g. `https://aur.archlinux.org/yay.git` or a self-hosted
    /// Gitea/cgit instance. The URL is used verbatim (trailing slash trimmed),
    /// matching `git clone` semantics.
    pub fn fetch_repo_url(
        &self,
        base: &str,
        branch: Option<&str>,
    ) -> Result<FetchResult, GitRemoteError> {
        let base = base.trim_end_matches('/');
        let discovery = self.discover_refs(base, "git-upload-pack")?;
        let (branch_name, head_sha) = select_branch_from_discovery(&discovery, branch)?;
        let extra_wants = if self.include_tags {
            tag_wants(&discovery.refs, &head_sha)
        } else {
            Vec::new()
        };
        let response = self.fetch_pack(base, &head_sha, &extra_wants)?;
        let objects = parse_packfile(&response.pack)?;

        Ok(FetchResult {
            branch: branch_name,
            head_sha,
            refs: discovery.refs,
            objects,
            shallow: response.shallow,
        })
    }

    pub fn list_branch_refs(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<RemoteBranch>, GitRemoteError> {
        self.list_branch_refs_url(&format!("{}/{}/{}.git", GITHUB_BASE, owner, repo))
    }

    /// List branch refs from an explicit smart-HTTP base URL (generic host).
    pub fn list_branch_refs_url(&self, base: &str) -> Result<Vec<RemoteBranch>, GitRemoteError> {
        let base = base.trim_end_matches('/');
        let discovery = self.discover_refs(base, "git-upload-pack")?;
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

    pub fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<String>, GitRemoteError> {
        Ok(self
            .list_branch_refs(owner, repo)?
            .into_iter()
            .map(|b| b.name)
            .collect())
    }

    /// Fetch the smart-HTTP ref advertisement for `service`
    /// (`git-upload-pack` for fetch, `git-receive-pack` for push).
    fn discover_refs(&self, base: &str, service: &str) -> Result<Discovery, GitRemoteError> {
        let pb = progress::spinner("Discovering remote refs");
        let url = format!("{}/info/refs?service={}", base, service);
        let accept = format!("application/x-{}-advertisement", service);
        let do_call =
            |token: Option<&str>| -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
                let mut r = self
                    .agent
                    .get(&url)
                    .header("Accept", accept.as_str())
                    .header("User-Agent", "ivaldi-vcs/0.1.0");
                if let Some(t) = token {
                    r = r.header("Authorization", basic_auth_header(t));
                }
                r.call()
            };

        let resp = match do_call(self.token.as_deref()) {
            Ok(resp) => {
                if !resp.status().is_success() {
                    if self.token.is_some() && is_auth_failure(&resp) {
                        crate::logging::warn(
                            "GitHub rejected the stored authentication token — retrying anonymously",
                        );
                        match do_call(None) {
                            Ok(resp2) => {
                                if !resp2.status().is_success() {
                                    pb.finish_and_clear();
                                    return Err(token_rejected_or(resp2));
                                }
                                resp2
                            }
                            Err(e2) => {
                                pb.finish_and_clear();
                                return Err(map_transport_error(e2));
                            }
                        }
                    } else {
                        pb.finish_and_clear();
                        return Err(map_response_error(resp));
                    }
                } else {
                    resp
                }
            }
            Err(err) => {
                pb.finish_and_clear();
                return Err(map_transport_error(err));
            }
        };
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        pb.finish_with_message("Remote refs discovered");
        parse_discovery(&bytes)
    }

    fn fetch_pack(
        &self,
        base: &str,
        want_sha: &str,
        extra_wants: &[String],
    ) -> Result<UploadPackResponse, GitRemoteError> {
        let url = format!("{}/git-upload-pack", base);
        let mut body = Vec::new();
        // Only the first want carries the capability list; the rest are bare.
        body.extend(pkt_line(&format!(
            "want {} {}\n",
            want_sha, UPLOAD_PACK_CAPS
        )));
        for want in extra_wants {
            body.extend(pkt_line(&format!("want {}\n", want)));
        }
        // Per the pack protocol the depth request goes after the wants and
        // before the flush that ends the request.
        if self.depth > 0 {
            body.extend(pkt_line(&format!("deepen {}\n", self.depth)));
        }
        body.extend_from_slice(b"0000");
        body.extend(pkt_line("done\n"));

        let do_call = |token: Option<&str>,
                       body: &[u8]|
         -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
            let mut r = self
                .agent
                .post(&url)
                .header("Content-Type", "application/x-git-upload-pack-request")
                .header("Accept", "application/x-git-upload-pack-result")
                .header("User-Agent", "ivaldi-vcs/0.1.0");
            if let Some(t) = token {
                r = r.header("Authorization", basic_auth_header(t));
            }
            r.send(body)
        };

        let resp = match do_call(self.token.as_deref(), &body) {
            Ok(resp) => {
                if !resp.status().is_success() {
                    if self.token.is_some() && is_auth_failure(&resp) {
                        crate::logging::warn(
                            "GitHub rejected the stored authentication token — retrying anonymously",
                        );
                        match do_call(None, &body) {
                            Ok(resp2) => {
                                if !resp2.status().is_success() {
                                    return Err(token_rejected_or(resp2));
                                }
                                resp2
                            }
                            Err(e2) => return Err(map_transport_error(e2)),
                        }
                    } else {
                        return Err(map_response_error(resp));
                    }
                } else {
                    resp
                }
            }
            Err(err) => return Err(map_transport_error(err)),
        };
        let total = resp
            .headers()
            .get("Content-Length")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<u64>().ok());
        let pb = total
            .map(|len| progress::byte_bar(len, "Downloading pack"))
            .unwrap_or_else(|| progress::spinner("Downloading pack"));
        let mut bytes = Vec::new();
        let mut reader = resp.into_body().into_reader();
        let mut chunk = [0u8; 8192];
        loop {
            let n = reader
                .read(&mut chunk)
                .map_err(|e| GitRemoteError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            pb.inc(n as u64);
        }
        if total.is_some() {
            pb.finish_with_message(format!("{} bytes downloaded", bytes.len()));
        } else {
            pb.finish_with_message(format!("Pack downloaded ({} bytes)", bytes.len()));
        }
        extract_pack_from_upload_pack(&bytes)
    }

    /// Push `branch`'s history to a Git-compatible server over smart-HTTP
    /// `git-receive-pack`.
    ///
    /// Mirrors the SSH push path ([`crate::ssh_transport::SshClient::push_repo`])
    /// but over HTTPS: translate the Ivaldi leaf chain into git objects, pack
    /// them into a single packfile, and POST it in one request. This replaces
    /// the old per-object GitHub REST upload, which fired one HTTP request per
    /// blob/tree/commit and tripped GitHub's secondary rate limit.
    pub fn push_repo(
        &self,
        repo: &mut crate::repo::Repo,
        owner: &str,
        repo_name: &str,
        branch: &str,
        force: bool,
        include_tags: bool,
    ) -> Result<PushReport, GitRemoteError> {
        self.push_repo_url(
            repo,
            &format!("{}/{}/{}.git", GITHUB_BASE, owner, repo_name),
            branch,
            force,
            include_tags,
        )
    }

    /// Push to an explicit Git smart-HTTP base URL (a non-GitHub host such as
    /// a self-hosted Gitea or cgit instance). Same protocol as [`push_repo`].
    pub fn push_repo_url(
        &self,
        repo: &mut crate::repo::Repo,
        base: &str,
        branch: &str,
        force: bool,
        include_tags: bool,
    ) -> Result<PushReport, GitRemoteError> {
        use crate::remote::HashMapping;

        let base = base.trim_end_matches('/');

        // ---- Resolve local head.
        let head_idx = repo
            .get_timeline_head(branch)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
            .ok_or_else(|| {
                GitRemoteError::Protocol(format!("local timeline '{}' has no head", branch))
            })?;

        // ---- Discover the remote's receive-pack advertisement.
        let discovery = self.discover_refs(base, "git-receive-pack")?;
        let target_ref = format!("refs/heads/{}", branch);
        let old_sha1 = discovery
            .refs
            .iter()
            .find(|r| r.name == target_ref)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| ZERO_SHA1.to_string());

        // ---- Refuse a non-fast-forward push unless forced. Some hosts
        // (git's default without receive.denyNonFastForwards) happily
        // rewrite the branch, silently destroying remote seals.
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        if old_sha1 != ZERO_SHA1 {
            check_push_fast_forward(repo, &mapping, branch, &old_sha1, head_idx, force)?;
        }

        // ---- Translate Ivaldi history (and any tags) to git objects.
        let prepare_pb = progress::spinner("Preparing incremental upload");
        let plan = plan_push(
            repo,
            branch,
            head_idx,
            &discovery.refs,
            &mapping,
            include_tags,
            force,
        )?;
        prepare_pb.finish_with_message(format!(
            "Upload prepared ({} changed objects)",
            plan.objects.len()
        ));
        if plan.updates.is_empty() {
            return Err(nothing_to_push(&plan.skipped_tags));
        }

        // ---- Build the request body: command pkt-lines + flush + packfile.
        let preamble = receive_pack_commands(&plan.updates);
        let preamble_len = preamble.len();
        let mut object_refs: Vec<&crate::git_export::GitObject> = plan.objects.values().collect();
        object_refs.sort_by_key(|o| o.sha1);
        let pack_pb = progress::spinner("Compressing upload pack");
        let body = crate::git_pack_writer::write_pack_after(preamble, &object_refs)
            .map_err(|e| GitRemoteError::Protocol(e.to_string()))?;
        pack_pb.finish_with_message(format!(
            "Pack ready ({} bytes)",
            body.len() - preamble_len
        ));
        // None when only tags moved — there is then no new branch tip to map.
        let branch_new_sha1 = plan
            .updates
            .iter()
            .find(|(name, _, _)| *name == target_ref)
            .map(|(_, _, new)| new.clone());

        // ---- Send it in one request and parse the report-status reply.
        let pb = progress::spinner("Uploading pack");
        let response = self.post_receive_pack(base, &body)?;
        pb.finish_with_message(format!("Pack uploaded ({} objects)", plan.objects.len()));
        let mut report = parse_report_status(&response)?;
        report.skipped_tags = plan.skipped_tags;

        // Record the new mapping locally on full success so the next push can
        // short-circuit. Non-fatal if the save fails.
        if report.unpack_ok
            && report.refs.iter().all(|r| r.error.is_none())
            && let Some(new_sha1_hex) = branch_new_sha1
            && let Ok(Some(leaf)) = repo.get_leaf(head_idx)
        {
            let mut mapping = HashMapping::new(&repo.ivaldi_dir);
            mapping.insert(&new_sha1_hex, leaf.hash());
            mapping.mark_verified(&new_sha1_hex);
            let _ = mapping.save();
        }

        Ok(report)
    }

    /// POST a `git-receive-pack` request body and return the raw response
    /// bytes (the `report-status` pkt-line block).
    fn post_receive_pack(&self, base: &str, body: &[u8]) -> Result<Vec<u8>, GitRemoteError> {
        let url = format!("{}/git-receive-pack", base);
        let mut r = self
            .agent
            .post(&url)
            .header("Content-Type", "application/x-git-receive-pack-request")
            .header("Accept", "application/x-git-receive-pack-result")
            .header("User-Agent", "ivaldi-vcs/0.1.0");
        if let Some(t) = self.token.as_deref() {
            r = r.header("Authorization", basic_auth_header(t));
        }
        let resp = r.send(body).map_err(map_transport_error)?;
        if !resp.status().is_success() {
            return Err(map_response_error(resp));
        }
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        Ok(bytes)
    }
}

/// Everything a push has to send, resolved against the server's advertisement.
pub(crate) struct PushPlan {
    pub objects: std::collections::BTreeMap<[u8; 20], crate::git_export::GitObject>,
    /// `(ref name, old sha1 hex, new sha1 hex)`, branch first. No-ops are
    /// already filtered out, so an empty list means there is nothing to do.
    pub updates: Vec<(String, String, String)>,
    /// Tags deliberately left out, with the reason, for the caller to report.
    pub skipped_tags: Vec<String>,
}

/// The 40 zeros git uses for "this ref does not exist yet".
pub(crate) const ZERO_SHA1: &str = "0000000000000000000000000000000000000000";

/// Expand the server's advertised commit tips through the locally known DAG.
///
/// Git advertises ref tips, not every reachable commit/object. Once an
/// advertised SHA maps to a local leaf, all of that leaf's ancestors and its
/// current filesystem snapshot are known to exist on the receiver. Returning
/// both facts lets export stop at an internal fork point and omit unchanged
/// tree/blob bodies without confusing identity with remote presence.
fn advertised_remote_state(
    repo: &crate::repo::Repo,
    advertised: &[AdvertisedRef],
    mapping: &crate::remote::HashMapping,
) -> Result<(BTreeSet<[u8; 20]>, Vec<B3Hash>), GitRemoteError> {
    let mut server_has = BTreeSet::new();
    for r in advertised.iter().filter(|r| r.id != ZERO_SHA1) {
        if let Some(sha) = sha1_hex_to_array(&r.id) {
            server_has.insert(sha);
        }
    }

    // One store scan avoids an O(refs × leaves) resolve for repositories with
    // many branches/tags.
    let mut leaf_by_hash = std::collections::BTreeMap::new();
    for idx in 0..repo.commit_count() {
        if let Some(leaf) = repo
            .get_leaf(idx)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
        {
            leaf_by_hash.insert(leaf.hash(), (idx, leaf.tree_root));
        }
    }

    let mut remote_tree_roots = BTreeSet::new();
    let mut stack = Vec::new();
    for r in advertised.iter().filter(|r| r.id != ZERO_SHA1) {
        let Some(b3) = mapping.get_blake3(&r.id) else {
            continue;
        };
        let Some(&(idx, tree_root)) = leaf_by_hash.get(&b3) else {
            continue;
        };
        remote_tree_roots.insert(tree_root);
        stack.push(idx);
    }

    let mut visited = BTreeSet::new();
    while let Some(idx) = stack.pop() {
        if !visited.insert(idx) {
            continue;
        }
        let Some(leaf) = repo
            .get_leaf(idx)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
        else {
            continue;
        };
        if let Some(sha) = mapping.get_sha1(leaf.hash()).and_then(sha1_hex_to_array) {
            server_has.insert(sha);
        }
        stack.extend(leaf.all_parents());
    }

    Ok((server_has, remote_tree_roots.into_iter().collect()))
}

fn sha1_hex_to_array(value: &str) -> Option<[u8; 20]> {
    let raw = hex::decode(value).ok()?;
    if raw.len() != 20 {
        return None;
    }
    let mut sha = [0u8; 20];
    sha.copy_from_slice(&raw);
    Some(sha)
}

/// Work out which refs a push should update and which objects that needs.
///
/// Shared by the HTTPS and SSH push paths so the two can't drift on which
/// commits are considered already-present, when a tag is safe to move, or how
/// "nothing to push" is decided.
///
/// Tags are only included when asked for. A tag the remote already holds at a
/// different object is a tag *move* — skipped unless forced, because
/// overwriting a published release tag is exactly the kind of loss `--force`
/// should have to be explicit about.
pub(crate) fn plan_push(
    repo: &crate::repo::Repo,
    branch: &str,
    head_idx: u64,
    advertised: &[AdvertisedRef],
    mapping: &crate::remote::HashMapping,
    include_tags: bool,
    force: bool,
) -> Result<PushPlan, GitRemoteError> {
    let advertised_sha = |name: &str| -> String {
        advertised
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| ZERO_SHA1.to_string())
    };

    let (server_has, remote_tree_roots) = advertised_remote_state(repo, advertised, mapping)?;

    let tags = if include_tags {
        repo.list_tags()
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
    } else {
        Vec::new()
    };

    // Tag targets join the branch head as export roots: a tag on a seal that
    // is not an ancestor of the branch still needs its commits on the server.
    let mut roots = vec![head_idx];
    for tag in &tags {
        if !roots.contains(&tag.target_index) {
            roots.push(tag.target_index);
        }
    }

    let export = crate::git_export::export_roots_incremental(
        repo,
        &roots,
        mapping,
        &server_has,
        &remote_tree_roots,
    )
    .map_err(|e| GitRemoteError::Protocol(format!("git export: {}", e)))?;
    let mut objects = export.objects;

    let mut updates = Vec::new();
    let branch_ref = format!("refs/heads/{}", branch);
    let branch_old = advertised_sha(&branch_ref);
    let branch_new = hex::encode(export.tip_sha1);
    if branch_new != branch_old {
        updates.push((branch_ref, branch_old, branch_new));
    }

    let mut skipped_tags = Vec::new();
    let tag_updates = crate::git_export::export_tags(&tags, &export.leaf_to_git, &mut objects)
        .map_err(|e| GitRemoteError::Protocol(format!("git export: {}", e)))?;
    for update in tag_updates {
        let old = advertised_sha(&update.name);
        let new = hex::encode(update.new_sha1);
        if old == new {
            continue; // remote already has this tag at this object
        }
        if old != ZERO_SHA1 && !force {
            skipped_tags.push(format!(
                "{} (remote has a different object; --force to move it)",
                update.name
            ));
            continue;
        }
        updates.push((update.name, old, new));
    }

    Ok(PushPlan {
        objects,
        updates,
        skipped_tags,
    })
}

/// The "nothing to send" error, which must not read as "all done" when tags
/// were in fact refused — that is the difference between an idle push and a
/// tag the user asked for that never left.
pub(crate) fn nothing_to_push(skipped_tags: &[String]) -> GitRemoteError {
    if skipped_tags.is_empty() {
        return GitRemoteError::Protocol(
            "nothing to push: the remote already has this timeline and its tags".into(),
        );
    }
    GitRemoteError::Protocol(format!(
        "nothing pushed: the timeline is already up to date and these tags were left alone: {}",
        skipped_tags.join(", ")
    ))
}

/// Encode a receive-pack request preamble: one command pkt-line per update
/// (capabilities ride on the first, per the protocol) then a flush.
pub(crate) fn receive_pack_commands(updates: &[(String, String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (name, old, new)) in updates.iter().enumerate() {
        let mut line = format!("{} {} {}", old, new, name);
        if i == 0 {
            // `report-status` makes the server tell us the outcome; we do not
            // request side-band-64k, so the response stays plain pkt-lines.
            line.push('\0');
            line.push_str("report-status agent=ivaldi/0.1.0");
        }
        line.push('\n');
        out.extend(pkt_line(&line));
    }
    out.extend_from_slice(b"0000");
    out
}

/// Refuse a non-fast-forward smart-HTTP push unless `force` is set.
///
/// `old_sha1` is the remote's advertised tip for the branch (known non-zero
/// by the caller). The push is a fast-forward only if that tip maps — via
/// the SHA1↔BLAKE3 map — to a local leaf that is an ancestor of the local
/// head being pushed. Anything else (unknown tip, unmapped tip,
/// mapped-but-diverged tip) would overwrite remote seals on hosts that
/// permit non-fast-forward updates. Runs entirely locally, BEFORE any bytes
/// are POSTed.
fn check_push_fast_forward(
    repo: &crate::repo::Repo,
    mapping: &crate::remote::HashMapping,
    branch: &str,
    old_sha1: &str,
    head_idx: u64,
    force: bool,
) -> Result<(), GitRemoteError> {
    if force {
        return Ok(());
    }
    // ponytail: O(n) leaf scan; fine until repos get huge (mirrors the
    // existing find_leaf_idx_by_hash pattern in sync::import).
    let remote_tip_local_idx = mapping.get_blake3(old_sha1).and_then(|b3| {
        (0..repo.commit_count())
            .find(|&idx| matches!(repo.get_leaf(idx), Ok(Some(leaf)) if leaf.hash() == b3))
    });
    let is_ff = match remote_tip_local_idx {
        Some(idx) => repo.is_ancestor(idx, head_idx).unwrap_or(false),
        None => false,
    };
    if !is_ff {
        return Err(GitRemoteError::Protocol(format!(
            "remote timeline '{}' has seals you do not have (remote tip {}) — \
             run 'ivaldi harvest' to sync first, or pass --force to overwrite remote history",
            branch, old_sha1
        )));
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Discovery {
    pub(crate) refs: Vec<AdvertisedRef>,
    pub(crate) default_branch: Option<String>,
}

fn header_value<'a>(resp: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

/// True for status codes that suggest the token was the problem, not the repo.
fn is_auth_failure(resp: &ureq::http::Response<ureq::Body>) -> bool {
    let status = resp.status().as_u16();
    if status == 401 {
        return true;
    }
    if status == 403 {
        // 403 with rate-limit headers is NOT an auth failure; retrying
        // anonymously would hit the same (or stricter) limit.
        return header_value(resp, "X-RateLimit-Remaining") != Some("0");
    }
    false
}

/// Mapper used after an anonymous retry that followed a stored-token rejection.
/// A 401 here means the repo also requires auth, so the actionable problem is
/// the rejected stored token — surface that explicitly instead of the generic
/// "authentication required" the second 401 would otherwise produce.
fn token_rejected_or(resp: ureq::http::Response<ureq::Body>) -> GitRemoteError {
    if resp.status().as_u16() == 401 {
        return GitRemoteError::TokenRejected;
    }
    map_response_error(resp)
}

fn map_response_error(resp: ureq::http::Response<ureq::Body>) -> GitRemoteError {
    let status = resp.status().as_u16();
    // Detect rate-limiting before consuming the body.
    if status == 403 && header_value(&resp, "X-RateLimit-Remaining") == Some("0") {
        let reset_at = header_value(&resp, "X-RateLimit-Reset")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        return GitRemoteError::RateLimited { reset_at };
    }
    let mut body = String::new();
    let _ = resp.into_body().into_reader().read_to_string(&mut body);
    match status {
        401 => GitRemoteError::AuthRequired,
        404 => GitRemoteError::RepoUnavailable,
        _ => GitRemoteError::Http {
            status,
            message: body.trim().to_string(),
        },
    }
}

fn map_transport_error(err: ureq::Error) -> GitRemoteError {
    GitRemoteError::Io(err.to_string())
}

pub(crate) fn pkt_line(payload: &str) -> Vec<u8> {
    let len = payload.len() + 4;
    let mut out = format!("{:04x}", len).into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out
}

/// Pick the (branch_name, head_sha) we want to fetch from a parsed
/// advertisement. Shared by every transport (HTTPS, SSH, future ivaldi://).
///
/// `requested_branch` may be `None` (use the default branch / HEAD), a short
/// name like `main`, or a full ref like `refs/heads/main`. Returns
/// `BranchNotFound` if an explicit branch was asked for but isn't advertised.
pub(crate) fn select_branch_from_discovery(
    discovery: &Discovery,
    requested_branch: Option<&str>,
) -> Result<(String, String), GitRemoteError> {
    let explicit_branch = requested_branch.is_some();
    let requested_owned = requested_branch.map(str::to_string);
    let wanted_ref = requested_owned
        .as_deref()
        .map(|name| {
            if name.starts_with("refs/") {
                name.to_string()
            } else {
                format!("refs/heads/{}", name)
            }
        })
        .or_else(|| {
            discovery
                .default_branch
                .as_ref()
                .map(|name| format!("refs/heads/{}", name))
        });

    let head_ref = discovery.refs.iter().find(|r| r.name == "HEAD");
    let selected = wanted_ref
        .as_ref()
        .and_then(|name| discovery.refs.iter().find(|r| r.name == *name))
        .cloned()
        .or_else(|| {
            if explicit_branch {
                None
            } else {
                head_ref.cloned()
            }
        })
        .ok_or_else(|| {
            requested_owned
                .clone()
                .map(GitRemoteError::BranchNotFound)
                .unwrap_or_else(|| {
                    GitRemoteError::Protocol("remote did not advertise a usable default ref".into())
                })
        })?;

    let branch_name = if selected.name == "HEAD" {
        discovery
            .refs
            .iter()
            .find(|r| r.name.starts_with("refs/heads/") && r.id == selected.id)
            .and_then(|r| r.name.strip_prefix("refs/heads/"))
            .unwrap_or("HEAD")
            .to_string()
    } else {
        selected
            .name
            .strip_prefix("refs/heads/")
            .unwrap_or(&selected.name)
            .to_string()
    };
    Ok((branch_name, selected.id.clone()))
}

pub(crate) fn parse_discovery(data: &[u8]) -> Result<Discovery, GitRemoteError> {
    let lines = parse_pkt_lines(data)?;
    if lines.is_empty() {
        return Err(GitRemoteError::Protocol("empty ref advertisement".into()));
    }

    let mut refs = Vec::new();
    let mut default_branch = None;

    for line in lines.into_iter().flatten() {
        if line.starts_with(b"# service=") {
            continue;
        }

        let (main, capabilities) = if let Some(null_idx) = line.iter().position(|&b| b == 0) {
            (&line[..null_idx], Some(&line[null_idx + 1..]))
        } else {
            (&line[..], None)
        };

        let text = std::str::from_utf8(main)
            .map_err(|_| GitRemoteError::Protocol("invalid UTF-8 in ref advertisement".into()))?;
        let text = text.trim_end_matches('\n');
        let mut parts = text.splitn(2, ' ');
        let id = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if id.len() == 40 && !name.is_empty() {
            refs.push(AdvertisedRef {
                id: id.to_string(),
                name: name.to_string(),
            });
        }

        if let Some(caps) = capabilities {
            let caps = std::str::from_utf8(caps).map_err(|_| {
                GitRemoteError::Protocol("invalid UTF-8 in capability advertisement".into())
            })?;
            for cap in caps.split(' ') {
                if let Some(symref) = cap.strip_prefix("symref=HEAD:refs/heads/") {
                    default_branch = Some(symref.to_string());
                }
            }
        }
    }

    if default_branch.is_none()
        && let Some(head) = refs.iter().find(|r| r.name == "HEAD")
        && let Some(target) = refs
            .iter()
            .find(|r| r.name.starts_with("refs/heads/") && r.id == head.id)
    {
        default_branch = target.name.strip_prefix("refs/heads/").map(str::to_string);
    }

    Ok(Discovery {
        refs,
        default_branch,
    })
}

fn parse_pkt_lines(data: &[u8]) -> Result<Vec<Option<Vec<u8>>>, GitRemoteError> {
    let mut idx = 0usize;
    let mut out = Vec::new();
    while idx + 4 <= data.len() {
        let len_hex = std::str::from_utf8(&data[idx..idx + 4])
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        idx += 4;
        let len = usize::from_str_radix(len_hex, 16)
            .map_err(|_| GitRemoteError::Protocol("invalid pkt-line length".into()))?;
        if len == 0 {
            out.push(None);
            continue;
        }
        if len < 4 || idx + (len - 4) > data.len() {
            return Err(GitRemoteError::Protocol("truncated pkt-line".into()));
        }
        out.push(Some(data[idx..idx + (len - 4)].to_vec()));
        idx += len - 4;
    }
    if idx != data.len() {
        return Err(GitRemoteError::Protocol(
            "trailing bytes after pkt-lines".into(),
        ));
    }
    Ok(out)
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
    /// Tags the push deliberately left alone, with the reason. Carried on the
    /// report rather than logged, so the caller always surfaces them: a tag
    /// the user asked to push and that silently didn't go is the one outcome
    /// they must not have to run with `-v` to discover.
    pub skipped_tags: Vec<String>,
}

/// Parse a `report-status` block from `git-receive-pack` (shared by the SSH
/// and smart-HTTP push paths). Format per gitprotocol-pack(5):
///
/// ```text
///   "unpack ok\n" | "unpack <error>\n"
///   ( "ok <ref>\n" | "ng <ref> <reason>\n" )*
///   flush-pkt
/// ```
pub(crate) fn parse_report_status(data: &[u8]) -> Result<PushReport, GitRemoteError> {
    // GitHub relays the report-status multiplexed over the git side-band even
    // though we never advertise `side-band-64k`, so the raw bytes look like
    // `\x01001dunpack index-pack failed`. Demux to channel 1 before parsing the
    // inner pkt-lines; a clean push that comes back as plain pkt-lines passes
    // through untouched. Without this, a real failure parses as garbage and a
    // rejected push can be misreported as success.
    let payload = demux_report_status(data)?;
    let lines = parse_pkt_text_lines(&payload)?;
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
        skipped_tags: Vec::new(),
        unpack_ok,
        unpack_error,
        refs,
    })
}

/// Demultiplex a `git-receive-pack` response that GitHub muxes over the git
/// side-band (channel 1 = report-status, 2 = progress, 3 = fatal error). Return
/// the channel-1 byte stream — the actual report-status pkt-lines. If the
/// response isn't muxed (a plain pkt-line report, common on a clean push) or
/// isn't clean pkt framing at all, return it unchanged so the text parser can
/// try. A channel-3 frame is a fatal server message; surface it as an error.
fn demux_report_status(data: &[u8]) -> Result<Vec<u8>, GitRemoteError> {
    let frames = match parse_pkt_lines(data) {
        Ok(f) => f,
        Err(_) => return Ok(data.to_vec()),
    };
    let muxed = frames
        .iter()
        .flatten()
        .find(|f| !f.is_empty())
        .map(|f| matches!(f[0], 1..=3))
        .unwrap_or(false);
    if !muxed {
        return Ok(data.to_vec());
    }
    let mut report = Vec::new();
    for frame in frames.iter().flatten() {
        let Some((&channel, rest)) = frame.split_first() else {
            continue;
        };
        match channel {
            1 => report.extend_from_slice(rest),
            2 => {} // progress (stderr) — not part of report-status
            3 => {
                return Err(GitRemoteError::Protocol(
                    String::from_utf8_lossy(rest).trim().to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(report)
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

/// A demultiplexed `git-upload-pack` response.
pub(crate) struct UploadPackResponse {
    pub pack: Vec<u8>,
    /// Shallow boundaries declared by the server (empty unless we deepened).
    pub shallow: BTreeSet<String>,
}

pub(crate) fn extract_pack_from_upload_pack(
    data: &[u8],
) -> Result<UploadPackResponse, GitRemoteError> {
    let mut pack = Vec::new();
    let mut shallow = BTreeSet::new();
    for line in parse_pkt_lines(data)? {
        let Some(line) = line else { continue };
        if line == b"NAK\n" || line.starts_with(b"ACK ") {
            continue;
        }
        // Shallow-update block, sent ahead of the pack when we asked to
        // `deepen`. `unshallow` only appears when deepening an existing
        // shallow repo, which `download` never does (it clones into an
        // empty directory), so the boundary set is purely additive here.
        if let Some(sha) = line.strip_prefix(b"shallow ".as_slice()) {
            shallow.insert(String::from_utf8_lossy(sha).trim().to_string());
            continue;
        }
        if line.starts_with(b"PACK") {
            pack.extend_from_slice(&line);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match line[0] {
            1 => pack.extend_from_slice(&line[1..]),
            2 => {}
            3 => {
                let msg = String::from_utf8_lossy(&line[1..]).trim().to_string();
                return Err(GitRemoteError::Protocol(format!("remote error: {}", msg)));
            }
            _ => {}
        }
    }

    if pack.len() < 12 || &pack[..4] != b"PACK" {
        return Err(GitRemoteError::Protocol(
            "upload-pack response did not contain a packfile".into(),
        ));
    }
    Ok(UploadPackResponse { pack, shallow })
}

enum PackedKind {
    Base(GitObjectKind),
    OfsDelta { base_offset: usize },
    RefDelta { base_sha: String },
}

struct PackedEntry {
    offset: usize,
    kind: PackedKind,
    data: Vec<u8>,
}

/// One resolved delta entry: `(entry index, object data, sha1, object kind)`.
type ResolvedDelta = (usize, Vec<u8>, String, GitObjectKind);

/// Parse a git packfile into a `sha1 → object` map.
///
/// Network-facing: every count, size, and varint in the pack is validated
/// against the actual byte length before it drives an allocation, and each
/// object's zlib stream is capped at its declared size. Public so the fuzz
/// target `git_parse_packfile` can exercise it.
pub fn parse_packfile(data: &[u8]) -> Result<HashMap<String, GitObject>, GitRemoteError> {
    if data.len() < 12 || &data[..4] != b"PACK" {
        return Err(GitRemoteError::Protocol("invalid packfile header".into()));
    }
    let version = u32::from_be_bytes(
        data[4..8]
            .try_into()
            .map_err(|_| GitRemoteError::Protocol("truncated packfile header".into()))?,
    );
    if !(2..=3).contains(&version) {
        return Err(GitRemoteError::Unsupported(format!(
            "unsupported pack version {}",
            version
        )));
    }
    let count = u32::from_be_bytes(
        data[8..12]
            .try_into()
            .map_err(|_| GitRemoteError::Protocol("truncated packfile header".into()))?,
    ) as usize;
    // The smallest possible entry is a 1-byte header plus an ~8-byte zlib
    // stream; a claimed count beyond that is corrupt. Checked BEFORE the
    // `with_capacity` below so a forged 4-byte count can't allocate gigabytes.
    if count > data.len() / 9 {
        return Err(GitRemoteError::Protocol(format!(
            "corrupt packfile: claims {} entries but is only {} bytes",
            count,
            data.len()
        )));
    }

    let mut idx = 12usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = idx;
        let (kind, header_len, size) = parse_object_header(&data[idx..])?;
        idx += header_len;

        let kind = match kind {
            1 => PackedKind::Base(GitObjectKind::Commit),
            2 => PackedKind::Base(GitObjectKind::Tree),
            3 => PackedKind::Base(GitObjectKind::Blob),
            4 => PackedKind::Base(GitObjectKind::Tag),
            6 => {
                let (base_offset, used) = parse_ofs_delta_base(&data[idx..], offset)?;
                idx += used;
                PackedKind::OfsDelta { base_offset }
            }
            7 => {
                if idx + 20 > data.len() {
                    return Err(GitRemoteError::Protocol("truncated ref-delta base".into()));
                }
                let base_sha = hex::encode(&data[idx..idx + 20]);
                idx += 20;
                PackedKind::RefDelta { base_sha }
            }
            other => {
                return Err(GitRemoteError::Unsupported(format!(
                    "unsupported object type {}",
                    other
                )));
            }
        };

        let (inflated, consumed) = inflate_from(&data[idx..], size)?;
        idx += consumed;
        entries.push(PackedEntry {
            offset,
            kind,
            data: inflated,
        });
    }

    resolve_pack_entries(entries)
}

/// Resolve all pack entries (bases + delta chains) into a `sha → GitObject`
/// map using parallel waves. Each wave contains entries whose parent has
/// already been resolved, so SHA-1 + delta apply run on multiple cores.
fn resolve_pack_entries(
    entries: Vec<PackedEntry>,
) -> Result<HashMap<String, GitObject>, GitRemoteError> {
    let n = entries.len();
    let entry_idx_by_offset: HashMap<usize, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.offset, i))
        .collect();

    // Slot per entry, populated as its wave completes.
    let mut slot: Vec<Option<(Vec<u8>, String, GitObjectKind)>> = (0..n).map(|_| None).collect();
    let mut sha_to_idx: HashMap<String, usize> = HashMap::with_capacity(n);

    // Wave 0: all base objects, hashed in parallel.
    let base_indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e.kind, PackedKind::Base(_)).then_some(i))
        .collect();

    let base_outputs: Vec<(usize, String, GitObjectKind)> = base_indices
        .par_iter()
        .map(|&i| {
            let e = &entries[i];
            let kind = match e.kind {
                PackedKind::Base(k) => k,
                _ => unreachable!(),
            };
            let sha = git_object_id(kind, &e.data);
            (i, sha, kind)
        })
        .collect();

    for (i, sha, kind) in base_outputs {
        sha_to_idx.entry(sha.clone()).or_insert(i);
        slot[i] = Some((entries[i].data.clone(), sha, kind));
    }

    let mut remaining: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| (!matches!(e.kind, PackedKind::Base(_))).then_some(i))
        .collect();

    while !remaining.is_empty() {
        // Partition into entries whose parent is already resolved (ready)
        // vs. entries that need a later wave (deferred).
        let (ready, deferred): (Vec<usize>, Vec<usize>) =
            remaining
                .iter()
                .copied()
                .partition(|&i| match &entries[i].kind {
                    PackedKind::OfsDelta { base_offset } => entry_idx_by_offset
                        .get(base_offset)
                        .is_some_and(|&pi| slot[pi].is_some()),
                    PackedKind::RefDelta { base_sha } => sha_to_idx
                        .get(base_sha)
                        .is_some_and(|&pi| slot[pi].is_some()),
                    PackedKind::Base(_) => true,
                });

        if ready.is_empty() {
            return Err(GitRemoteError::Protocol(
                "unresolvable delta chain in packfile".into(),
            ));
        }

        let outputs: Result<Vec<ResolvedDelta>, GitRemoteError> = ready
            .par_iter()
            .map(|&i| {
                let e = &entries[i];
                let parent_idx = match &e.kind {
                    PackedKind::OfsDelta { base_offset } => entry_idx_by_offset[base_offset],
                    PackedKind::RefDelta { base_sha } => sha_to_idx[base_sha],
                    PackedKind::Base(_) => unreachable!(),
                };
                let (parent_data, _, parent_kind) = slot[parent_idx].as_ref().unwrap();
                let data = apply_delta(parent_data, &e.data)?;
                let sha = git_object_id(*parent_kind, &data);
                Ok((i, data, sha, *parent_kind))
            })
            .collect();
        let outputs = outputs?;

        for (i, data, sha, kind) in outputs {
            sha_to_idx.entry(sha.clone()).or_insert(i);
            slot[i] = Some((data, sha, kind));
        }

        remaining = deferred;
    }

    let mut out = HashMap::with_capacity(n);
    for opt in slot.into_iter() {
        let (data, sha, kind) = opt.expect("every entry resolved");
        out.entry(sha).or_insert(GitObject { kind, data });
    }
    Ok(out)
}

fn parse_object_header(data: &[u8]) -> Result<(u8, usize, usize), GitRemoteError> {
    let first = *data
        .first()
        .ok_or_else(|| GitRemoteError::Protocol("truncated object header".into()))?;
    let kind = (first >> 4) & 0x07;
    let mut size = (first & 0x0f) as u64;
    let mut shift = 4u32;
    let mut used = 1usize;
    let mut byte = first;

    while byte & 0x80 != 0 {
        byte = *data
            .get(used)
            .ok_or_else(|| GitRemoteError::Protocol("truncated object header".into()))?;
        // A shift ≥ 64 is a malformed (or hostile) varint; without this cap
        // the shift overflows and panics under overflow-checks.
        if shift >= 64 {
            return Err(GitRemoteError::Protocol(
                "object header size varint too long".into(),
            ));
        }
        size |= ((byte & 0x7f) as u64) << shift;
        shift += 7;
        used += 1;
    }

    if size > MAX_GIT_OBJECT_SIZE as u64 {
        return Err(GitRemoteError::Protocol(format!(
            "object declares size {} which exceeds the {} byte limit",
            size, MAX_GIT_OBJECT_SIZE
        )));
    }
    Ok((kind, used, size as usize))
}

fn parse_ofs_delta_base(
    data: &[u8],
    current_offset: usize,
) -> Result<(usize, usize), GitRemoteError> {
    let mut used = 0usize;
    let mut byte = *data
        .get(used)
        .ok_or_else(|| GitRemoteError::Protocol("truncated ofs-delta base".into()))?;
    used += 1;
    let mut value = (byte & 0x7f) as u64;
    while byte & 0x80 != 0 {
        // 10 continuation bytes already exceed any offset a real pack can
        // hold; further bytes only overflow the accumulator.
        if used >= 10 {
            return Err(GitRemoteError::Protocol(
                "ofs-delta base varint too long".into(),
            ));
        }
        byte = *data
            .get(used)
            .ok_or_else(|| GitRemoteError::Protocol("truncated ofs-delta base".into()))?;
        used += 1;
        value = value
            .checked_add(1)
            .and_then(|v| v.checked_shl(7))
            .map(|v| v | ((byte & 0x7f) as u64))
            .ok_or_else(|| GitRemoteError::Protocol("ofs-delta base overflow".into()))?;
    }
    let value = usize::try_from(value)
        .map_err(|_| GitRemoteError::Protocol("ofs-delta base overflow".into()))?;
    current_offset
        .checked_sub(value)
        .map(|base| (base, used))
        .ok_or_else(|| GitRemoteError::Protocol("invalid ofs-delta base offset".into()))
}

/// Inflate one zlib stream, capped at `expected` bytes (the size the pack
/// entry header declared). A stream producing more than declared is a zlib
/// bomb; less is truncation. Both are hard errors.
fn inflate_from(data: &[u8], expected: usize) -> Result<(Vec<u8>, usize), GitRemoteError> {
    let reader = Cursor::new(data);
    let decoder = ZlibDecoder::new(reader);
    // +1 so an over-long stream is detectable rather than silently clipped.
    let mut limited = decoder.take(expected as u64 + 1);
    let mut out = Vec::new();
    limited
        .read_to_end(&mut out)
        .map_err(|e| GitRemoteError::Protocol(format!("zlib decode failed: {}", e)))?;
    if out.len() != expected {
        return Err(GitRemoteError::Protocol(format!(
            "object inflated to {}+ bytes but its header declared {}",
            out.len(),
            expected
        )));
    }
    let consumed = limited.into_inner().into_inner().position() as usize;
    Ok((out, consumed))
}

pub fn git_object_id(kind: GitObjectKind, data: &[u8]) -> String {
    let kind_name = match kind {
        GitObjectKind::Commit => "commit",
        GitObjectKind::Tree => "tree",
        GitObjectKind::Blob => "blob",
        GitObjectKind::Tag => "tag",
    };
    let mut canonical = format!("{} {}\0", kind_name, data.len()).into_bytes();
    canonical.extend_from_slice(data);
    hex::encode(sha1_digest(&canonical))
}

/// Apply a git delta stream to `base`. Public so the fuzz target
/// `git_apply_delta` can exercise it against arbitrary bytes.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, GitRemoteError> {
    let mut cursor = 0usize;
    let base_size = read_varint(delta, &mut cursor)?;
    let result_size = read_varint(delta, &mut cursor)?;
    if base_size != base.len() {
        return Err(GitRemoteError::Protocol("delta base size mismatch".into()));
    }
    // Checked BEFORE `with_capacity`: a forged result-size varint must not
    // drive the allocation (mirrors pack::MAX_DELTA_OUTPUT).
    if result_size > MAX_GIT_OBJECT_SIZE {
        return Err(GitRemoteError::Protocol(format!(
            "delta declares result size {} which exceeds the {} byte limit",
            result_size, MAX_GIT_OBJECT_SIZE
        )));
    }

    let mut out = Vec::with_capacity(result_size);
    while cursor < delta.len() {
        let opcode = delta[cursor];
        cursor += 1;
        if opcode & 0x80 != 0 {
            let mut offset = 0usize;
            let mut size = 0usize;
            for shift in [0, 8, 16, 24] {
                if opcode & (1 << (shift / 8)) != 0 {
                    offset |=
                        (delta.get(cursor).copied().ok_or_else(|| {
                            GitRemoteError::Protocol("truncated delta copy".into())
                        })? as usize)
                            << shift;
                    cursor += 1;
                }
            }
            for (bit, shift) in [(0x10, 0usize), (0x20, 8usize), (0x40, 16usize)] {
                if opcode & bit != 0 {
                    size |=
                        (delta.get(cursor).copied().ok_or_else(|| {
                            GitRemoteError::Protocol("truncated delta size".into())
                        })? as usize)
                            << shift;
                    cursor += 1;
                }
            }
            if size == 0 {
                size = 0x10000;
            }
            let end = offset
                .checked_add(size)
                .ok_or_else(|| GitRemoteError::Protocol("delta copy range overflow".into()))?;
            if end > base.len() {
                return Err(GitRemoteError::Protocol(
                    "delta copy exceeds base size".into(),
                ));
            }
            out.extend_from_slice(&base[offset..end]);
        } else if opcode != 0 {
            let end = cursor + opcode as usize;
            if end > delta.len() {
                return Err(GitRemoteError::Protocol("truncated delta insert".into()));
            }
            out.extend_from_slice(&delta[cursor..end]);
            cursor = end;
        } else {
            return Err(GitRemoteError::Protocol("invalid delta opcode".into()));
        }
    }

    if out.len() != result_size {
        return Err(GitRemoteError::Protocol(
            "delta result size mismatch".into(),
        ));
    }
    Ok(out)
}

fn read_varint(data: &[u8], cursor: &mut usize) -> Result<usize, GitRemoteError> {
    let mut shift = 0u32;
    let mut value = 0u64;
    loop {
        let byte = *data
            .get(*cursor)
            .ok_or_else(|| GitRemoteError::Protocol("truncated varint".into()))?;
        *cursor += 1;
        // Cap the shift so a run of continuation bytes can't overflow it
        // (panic under overflow-checks) or wrap the accumulator.
        if shift >= 64 {
            return Err(GitRemoteError::Protocol("varint too long".into()));
        }
        value |= u64::from(byte & 0x7f)
            .checked_shl(shift)
            .filter(|v| v >> shift == u64::from(byte & 0x7f))
            .ok_or_else(|| GitRemoteError::Protocol("varint overflow".into()))?;
        if byte & 0x80 == 0 {
            return usize::try_from(value)
                .map_err(|_| GitRemoteError::Protocol("varint overflow".into()));
        }
        shift += 7;
    }
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// An annotated git tag object.
#[derive(Debug, Clone)]
pub struct ParsedTag {
    /// SHA-1 of the object this tag names (usually a commit, occasionally
    /// another tag).
    pub object: String,
    pub tagger: String,
    pub timestamp: i64,
    /// The tagger's UTC offset, e.g. `-0500`. Part of the object's SHA-1, so
    /// it has to survive a fetch/push round trip.
    pub tz: String,
    pub message: String,
}

/// Parse an annotated tag object (`object`/`type`/`tag`/`tagger` headers,
/// blank line, message). Lightweight tags have no such object — their ref
/// points straight at a commit.
pub fn parse_tag_object(data: &[u8]) -> Result<ParsedTag, GitRemoteError> {
    let text = std::str::from_utf8(data)
        .map_err(|_| GitRemoteError::Protocol("tag object is not valid UTF-8".into()))?;
    let (headers, message) = text.split_once("\n\n").unwrap_or((text, ""));

    let mut object = None;
    let mut tagger = String::new();
    let mut timestamp = 0i64;
    let mut tz = String::from("+0000");
    for line in headers.lines() {
        if let Some(value) = line.strip_prefix("object ") {
            object = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("tagger ") {
            // `Name <email> <unix> <tz>` — same shape as a commit's author.
            let (identity, time, offset) = split_identity_and_time(value);
            tagger = identity;
            timestamp = time;
            tz = offset;
        }
    }

    Ok(ParsedTag {
        object: object
            .ok_or_else(|| GitRemoteError::Protocol("tag object has no target".into()))?,
        tagger,
        timestamp,
        tz,
        message: message.to_string(),
    })
}

/// Split a git identity line into `"Name <email>"`, its unix timestamp, and
/// its UTC offset. A malformed or absent timestamp reads as 0 and a missing
/// offset as `+0000`, rather than failing the parse — the identity is the
/// part callers display.
fn split_identity_and_time(value: &str) -> (String, i64, String) {
    match value.rfind('>') {
        Some(end) => {
            let identity = value[..=end].trim().to_string();
            let mut rest = value[end + 1..].split_whitespace();
            let timestamp = rest.next().and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
            let tz = rest.next().unwrap_or("+0000").to_string();
            (identity, timestamp, tz)
        }
        None => (value.trim().to_string(), 0, "+0000".to_string()),
    }
}

pub fn parse_commit(data: &[u8]) -> Result<ParsedCommit, GitRemoteError> {
    let text = std::str::from_utf8(data)
        .map_err(|_| GitRemoteError::Protocol("commit object is not valid UTF-8".into()))?;
    let (headers, message) = text.split_once("\n\n").unwrap_or((text, ""));

    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = (String::new(), String::new(), 0i64, String::new());
    let mut committer = (String::new(), String::new(), 0i64, String::new());

    for line in headers.lines() {
        if let Some(value) = line.strip_prefix("tree ") {
            tree = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("parent ") {
            parents.push(value.to_string());
        } else if let Some(value) = line.strip_prefix("author ") {
            author = parse_identity_line(value);
        } else if let Some(value) = line.strip_prefix("committer ") {
            committer = parse_identity_line(value);
        }
    }

    // If a commit object only has an author line (rare but possible in
    // hand-crafted fixtures), fall back to mirroring author into committer.
    if committer.0.is_empty() && committer.1.is_empty() && committer.2 == 0 {
        committer = author.clone();
    }

    Ok(ParsedCommit {
        tree: tree.ok_or_else(|| GitRemoteError::Protocol("commit missing tree".into()))?,
        parents,
        author_name: author.0,
        author_email: author.1,
        author_time: author.2,
        author_tz: author.3,
        committer_name: committer.0,
        committer_email: committer.1,
        committer_time: committer.2,
        committer_tz: committer.3,
        message: message.to_string(),
    })
}

/// Parse the value portion of a Git `author`/`committer` header line
/// (`Name <email> <unix-seconds> <±HHMM>`).
fn parse_identity_line(value: &str) -> (String, String, i64, String) {
    let mut name = String::new();
    let mut email = String::new();
    let mut time = 0i64;
    let mut tz = String::new();
    if let Some((prefix, tz_part)) = value.rsplit_once(' ') {
        tz = tz_part.to_string();
        if let Some((identity, timestamp)) = prefix.rsplit_once(' ') {
            time = timestamp.parse().unwrap_or(0);
            if let Some((n, e)) = identity.rsplit_once(" <") {
                name = n.to_string();
                email = e.trim_end_matches('>').to_string();
            }
        }
    }
    (name, email, time, tz)
}

pub fn parse_tree(data: &[u8]) -> Result<Vec<TreeEntry>, GitRemoteError> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < data.len() {
        let mode_end = data[idx..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| GitRemoteError::Protocol("invalid tree entry mode".into()))?;
        let mode = std::str::from_utf8(&data[idx..idx + mode_end])
            .map_err(|_| GitRemoteError::Protocol("invalid tree entry mode".into()))?
            .to_string();
        idx += mode_end + 1;

        let name_end = data[idx..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| GitRemoteError::Protocol("invalid tree entry name".into()))?;
        let name = String::from_utf8(data[idx..idx + name_end].to_vec())
            .map_err(|_| GitRemoteError::Protocol("invalid tree entry name".into()))?;
        idx += name_end + 1;

        if idx + 20 > data.len() {
            return Err(GitRemoteError::Protocol("truncated tree entry sha".into()));
        }
        let sha = hex::encode(&data[idx..idx + 20]);
        idx += 20;

        out.push(TreeEntry { mode, name, sha });
    }
    Ok(out)
}

pub fn encode_pack_for_tests(objects: &[(GitObjectKind, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PACK");
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&(objects.len() as u32).to_be_bytes());

    for (kind, data) in objects {
        out.extend(encode_object_header(*kind, data.len()));
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        out.extend_from_slice(&encoder.finish().unwrap());
    }

    out.extend_from_slice(&[0u8; 20]);
    out
}

fn encode_object_header(kind: GitObjectKind, size: usize) -> Vec<u8> {
    let kind_code = match kind {
        GitObjectKind::Commit => 1,
        GitObjectKind::Tree => 2,
        GitObjectKind::Blob => 3,
        GitObjectKind::Tag => 4,
    };
    let mut size = size;
    let mut first = ((kind_code as u8) << 4) | ((size & 0x0f) as u8);
    size >>= 4;
    let mut out = Vec::new();
    if size > 0 {
        first |= 0x80;
    }
    out.push(first);
    while size > 0 {
        let mut byte = (size & 0x7f) as u8;
        size >>= 7;
        if size > 0 {
            byte |= 0x80;
        }
        out.push(byte);
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum GitRemoteError {
    #[error("authentication required — run 'ivaldi auth login' or set GITHUB_TOKEN")]
    AuthRequired,
    #[error(
        "GitHub rejected the stored authentication token. Run 'ivaldi auth logout && ivaldi auth login' to re-authenticate."
    )]
    TokenRejected,
    #[error("repository not found or requires authentication")]
    RepoUnavailable,
    #[error("branch not found: {0}")]
    BranchNotFound(String),
    #[error(
        "GitHub rate limit reached (60/hr unauthenticated). Run 'ivaldi auth login' to raise the limit to 5000/hr."
    )]
    RateLimited { reset_at: u64 },
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Io(String),
}

pub fn import_fetch_result(
    repo: &mut crate::repo::Repo,
    fetch: &FetchResult,
) -> Result<crate::sync::ImportResult, GitRemoteError> {
    use crate::cas::FileCas;
    use crate::fsmerkle::FsStore;
    use crate::leaf::{Leaf, NO_PARENT};
    use crate::remote::HashMapping;

    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    let store = FsStore::new(&cas);
    let mut mapping = HashMapping::new(&repo.ivaldi_dir);

    let mut commit_order = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_commit_order(
        &fetch.head_sha,
        &fetch.objects,
        &fetch.shallow,
        &mut seen,
        &mut commit_order,
    )?;
    // Tagged commits usually sit outside the branch being cloned, so they need
    // their own walk roots — without this their objects arrive in the pack but
    // never become seals, and every tag then resolves to nothing.
    // `seen` is shared, so the combined order stays topological and each commit
    // is emitted once.
    let tags = advertised_tags(fetch)?;
    for tag in &tags {
        collect_commit_order(
            &tag.target_commit,
            &fetch.objects,
            &fetch.shallow,
            &mut seen,
            &mut commit_order,
        )?;
    }

    let mut tree_cache: HashMap<String, B3Hash> = HashMap::new();
    let mut leaf_idx_by_sha: HashMap<String, u64> = HashMap::new();
    let mut leaf_idx_by_hash: HashMap<B3Hash, u64> = HashMap::new();
    let mut commits_imported = 0usize;
    let mut commits_skipped = 0usize;
    // Accumulator for submodule (gitlink) entries we couldn't materialize.
    // Written to `.ivaldi/submodules.skipped` after import.
    let mut submodules_skipped: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for idx in 0..repo.commit_count() {
        if let Ok(Some(leaf)) = repo.get_leaf(idx) {
            let leaf_hash = leaf.hash();
            leaf_idx_by_hash.insert(leaf_hash, idx);
            // A process can die after the append-only leaf transaction commits
            // but before the separate Git↔Ivaldi mapping file is published.
            // The source SHA is authenticated leaf metadata, so reconstruct
            // the mapping from history instead of appending a duplicate leaf.
            if let Some(git_sha) = leaf.meta.get("git.sha1") {
                mapping.insert(git_sha, leaf_hash);
                leaf_idx_by_sha.insert(git_sha.clone(), idx);
            }
        }
    }

    // Walk every reachable blob once, then write the new ones to CAS in
    // parallel up front. After this, the per-commit tree import is pure
    // mapping lookups + tree assembly — no blob I/O on the hot path.
    let tag_roots: Vec<&str> = tags.iter().map(|t| t.target_commit.as_str()).collect();
    let all_blobs = collect_reachable_blobs(
        &fetch.head_sha,
        &tag_roots,
        &fetch.objects,
        &fetch.shallow,
    )?;
    let pending_count = all_blobs
        .iter()
        .filter(|sha| mapping.get_blake3(sha).is_none())
        .count();

    let mp = MultiProgress::new();
    let pb_blobs = mp.add(progress::file_bar(pending_count as u64, "Importing blobs"));
    let pb_commits = mp.add(progress::file_bar(
        commit_order.len() as u64,
        "Importing commits",
    ));

    let prefetched = prefetch_blobs(&all_blobs, &fetch.objects, &store, &mapping, &pb_blobs)?;
    let blobs_downloaded = prefetched.len();
    for (git_sha, hash) in prefetched {
        mapping.insert(&git_sha, hash);
    }
    // Persist the blob mappings now: a crash during the commit loop must not
    // lose the record of CAS work already done (retry would re-import
    // everything and, worse, resolve parents against an empty map). The CAS
    // is flushed FIRST — the mapping must never durably claim a blob whose
    // directory entry could still vanish on power loss.
    crate::failpoint::fail_point("import.after_blob_prefetch");
    cas.flush().map_err(|e| GitRemoteError::Io(e.to_string()))?;
    mapping
        .save()
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;

    // Leaves built but not yet landed. Drained by `land_commit_batch` every
    // COMMIT_BATCH commits and once more after the loop.
    let mut batch: Vec<(String, Leaf)> = Vec::with_capacity(COMMIT_BATCH);

    for sha in commit_order.iter() {
        // Skip only when the mapped leaf actually exists locally. A mapping
        // entry whose leaf is missing (stale/foreign map) falls through to a
        // fresh import instead of silently severing its children's ancestry.
        if let Some(existing_hash) = mapping.get_blake3(sha)
            && let Some(idx) = leaf_idx_by_hash.get(&existing_hash).copied()
        {
            leaf_idx_by_sha.insert(sha.clone(), idx);
            commits_skipped += 1;
            pb_commits.inc(1);
            continue;
        }

        let object = fetch
            .objects
            .get(sha)
            .ok_or_else(|| GitRemoteError::Protocol(format!("missing commit object {}", sha)))?;
        let commit = parse_commit(&object.data)?;
        let tree_hash = import_tree(
            &commit.tree,
            "",
            &fetch.objects,
            &store,
            &mut mapping,
            &mut tree_cache,
            &mut submodules_skipped,
        )?;
        let author = if commit.author_name.is_empty() || commit.author_email.is_empty() {
            "unknown <unknown>".to_string()
        } else {
            format!("{} <{}>", commit.author_name, commit.author_email)
        };

        // Every parent must resolve to a real local leaf. `commit_order` is
        // topological and the pack is self-contained, so an unresolvable
        // parent means corrupted state — refusing beats silently importing
        // this commit as a fake root (severed ancestry).
        let resolve_parent = |p: &String| -> Result<u64, GitRemoteError> {
            leaf_idx_by_sha.get(p).copied().ok_or_else(|| {
                GitRemoteError::Protocol(format!(
                    "commit {} lists parent {} which resolves to no local seal — \
                     refusing to sever ancestry; run 'ivaldi verify --full' and retry the harvest",
                    sha, p
                ))
            })
        };
        // A shallow boundary lands as a root: the server was asked to omit
        // its ancestry, so there is nothing to sever. Everywhere else an
        // unresolvable parent still means a broken pack.
        let is_boundary = fetch.shallow.contains(sha);
        let (prev_idx, merge_idxs) = if is_boundary {
            (NO_PARENT, Vec::new())
        } else {
            let prev_idx = match commit.parents.first() {
                Some(p) => resolve_parent(p)?,
                None => NO_PARENT,
            };
            let merge_idxs = commit
                .parents
                .iter()
                .skip(1)
                .map(resolve_parent)
                .collect::<Result<Vec<_>, _>>()?;
            (prev_idx, merge_idxs)
        };

        let mut leaf = Leaf::new(
            tree_hash,
            &fetch.branch,
            &author,
            commit.author_time,
            &commit.message,
        );
        leaf.prev_idx = prev_idx;
        leaf.merge_idxs = merge_idxs;
        // Recovery anchor for the unavoidable database↔mapping-file crash
        // window. On retry this makes an already-landed leaf discoverable by
        // its remote identity without guessing from content or position.
        leaf.meta.insert("git.sha1".into(), sha.clone());
        // Mark truncated ancestry in authenticated leaf metadata so a root
        // that only looks like a root is never mistaken for the real start
        // of history — by a reader, by `whereami`, or by a later deepen.
        if is_boundary {
            leaf.meta.insert("git.shallow".into(), "1".into());
        }

        // Preserve git fidelity that doesn't fit Leaf's typed fields. Stored
        // under reserved `git.*` meta keys; the canonical leaf encoding
        // already serializes `meta` so no version bump is needed.
        if !commit.author_tz.is_empty() {
            leaf.meta
                .insert("git.author_tz".into(), commit.author_tz.clone());
        }
        if !commit.committer_name.is_empty() || !commit.committer_email.is_empty() {
            let committer = format!("{} <{}>", commit.committer_name, commit.committer_email,);
            leaf.meta.insert("git.committer".into(), committer);
            leaf.meta.insert(
                "git.committer_time".into(),
                commit.committer_time.to_string(),
            );
            if !commit.committer_tz.is_empty() {
                leaf.meta
                    .insert("git.committer_tz".into(), commit.committer_tz.clone());
            }
        }
        // Queue rather than commit. `commit_batch_raw` assigns consecutive
        // indices from the current MMR size, so this leaf's index is known
        // before it lands and children later in the topological order can
        // resolve it as a parent immediately.
        leaf_idx_by_sha.insert(sha.clone(), repo.commit_count() + batch.len() as u64);
        batch.push((sha.clone(), leaf));
        commits_imported += 1;
        if batch.len() >= COMMIT_BATCH {
            land_commit_batch(
                repo,
                &cas,
                &mut mapping,
                &mut leaf_idx_by_hash,
                &fetch.branch,
                &mut batch,
            )?;
        }
        pb_commits.inc(1);
    }
    land_commit_batch(
        repo,
        &cas,
        &mut mapping,
        &mut leaf_idx_by_hash,
        &fetch.branch,
        &mut batch,
    )?;

    pb_blobs.finish_with_message(format!("{} blobs imported", blobs_downloaded));
    let commit_msg = if commits_skipped > 0 {
        format!(
            "{} new commits imported ({} already present)",
            commits_imported, commits_skipped
        )
    } else {
        format!("{} commits imported", commits_imported)
    };
    pb_commits.finish_with_message(commit_msg);

    if !submodules_skipped.is_empty() {
        crate::logging::warn(&format!(
            "skipped {} git submodule entr{} (Ivaldi does not yet clone submodules); see .ivaldi/submodules.skipped",
            submodules_skipped.len(),
            if submodules_skipped.len() == 1 {
                "y"
            } else {
                "ies"
            },
        ));
        let payload: String = submodules_skipped
            .iter()
            .map(|p| format!("{}\n", p))
            .collect();
        let _ = std::fs::write(repo.ivaldi_dir.join("submodules.skipped"), payload);
    }

    // Make sure the timeline head + ref file exist for the harvested branch,
    // even if every commit was already present in the repo (e.g. harvesting
    // a branch whose tip is an ancestor of an already-imported timeline).
    // `commit_raw` only updates the head when it actually writes a commit,
    // so without this the branch silently fails to materialize as a local
    // timeline.
    let head_idx = leaf_idx_by_sha.get(&fetch.head_sha).copied().or_else(|| {
        mapping
            .get_blake3(&fetch.head_sha)
            .and_then(|b3| leaf_idx_by_hash.get(&b3).copied())
    });
    // Flush any remaining CAS writes BEFORE the head and mapping become
    // durable records referencing them (usually a no-op: the commit loop
    // flushed per commit).
    cas.flush().map_err(|e| GitRemoteError::Io(e.to_string()))?;
    if let Some(idx) = head_idx {
        repo.set_timeline_head(&fetch.branch, idx)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    }

    crate::failpoint::fail_point("import.before_mapping_save");
    mapping
        .save()
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    crate::failpoint::fail_point("import.after_mapping_save");

    // Tags land last: every one names a seal, so they can only be resolved
    // once the leaves exist. Nothing is gated on `--include-tags` here — a tag
    // is recorded exactly when its target was actually fetched, which also
    // picks up tags that happen to sit on the history a plain clone pulled.
    let tags = tags_for_landed_commits(tags, repo, &mapping, &leaf_idx_by_sha, &leaf_idx_by_hash)?;
    let tags_imported = tags.len();
    repo.set_tags(&tags)
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;

    Ok(crate::sync::ImportResult {
        commits_imported,
        commits_skipped,
        blobs_downloaded,
        timeline: fetch.branch.clone(),
        tags_imported,
    })
}

/// One advertised `refs/tags/<name>` resolved against the fetched objects.
struct AdvertisedTag {
    name: String,
    /// The commit the tag ultimately names, after peeling any tag objects.
    target_commit: String,
    /// Present for annotated tags — the outermost tag object's own message.
    annotation: Option<ParsedTag>,
}

/// Resolve every advertised tag whose objects this fetch actually carries.
///
/// Lightweight tags point straight at a commit; annotated ones point at a tag
/// object carrying the message and tagger, and may even chain tag→tag. Tags
/// that were not fetched, or that name something other than a commit, are
/// skipped — that is the normal outcome of cloning one branch without
/// `--include-tags`, not a broken pack.
fn advertised_tags(fetch: &FetchResult) -> Result<Vec<AdvertisedTag>, GitRemoteError> {
    // Chained annotated tags are legal but vanishingly rare; a cap keeps a
    // hostile pack from looping us forever.
    const MAX_TAG_CHAIN: usize = 16;

    let mut out = Vec::new();
    for advertised in &fetch.refs {
        let Some(name) = advertised.name.strip_prefix("refs/tags/") else {
            continue;
        };
        if name.ends_with("^{}") {
            continue; // peeled companion of the ref above
        }
        // A name Ivaldi can't safely represent as a ref path is skipped, not
        // an error: one odd remote tag must not fail an otherwise good clone.
        if crate::refname::validate_timeline_name(name).is_err() {
            crate::logging::warn(&format!("skipping remote tag {name:?}: unusable name"));
            continue;
        }

        let mut sha = advertised.id.clone();
        let mut annotation: Option<ParsedTag> = None;
        for _ in 0..MAX_TAG_CHAIN {
            match fetch.objects.get(&sha) {
                Some(object) if object.kind == GitObjectKind::Tag => {
                    let parsed = parse_tag_object(&object.data)?;
                    sha = parsed.object.clone();
                    // The outermost annotation is the tag's own message.
                    annotation.get_or_insert(parsed);
                }
                _ => break,
            }
        }

        // Only commits become seals. A tag on a tree or blob has nothing to
        // point at in Ivaldi's model.
        let is_local_commit = fetch
            .objects
            .get(&sha)
            .is_some_and(|o| o.kind == GitObjectKind::Commit);
        if !is_local_commit {
            continue;
        }
        out.push(AdvertisedTag {
            name: name.to_string(),
            target_commit: sha,
            annotation,
        });
    }
    Ok(out)
}

/// Bind resolved tags to the seals their target commits landed at.
fn tags_for_landed_commits(
    tags: Vec<AdvertisedTag>,
    repo: &crate::repo::Repo,
    mapping: &crate::remote::HashMapping,
    leaf_idx_by_sha: &HashMap<String, u64>,
    leaf_idx_by_hash: &HashMap<B3Hash, u64>,
) -> Result<Vec<crate::tags::Tag>, GitRemoteError> {
    let mut out = Vec::new();
    for tag in tags {
        let Some(index) = leaf_idx_by_sha
            .get(&tag.target_commit)
            .copied()
            .or_else(|| {
                mapping
                    .get_blake3(&tag.target_commit)
                    .and_then(|b3| leaf_idx_by_hash.get(&b3).copied())
            })
        else {
            continue; // target commit was not imported
        };
        // Re-tagging the same seal is a no-op; a tag that moved gets rewritten.
        if repo
            .get_tag(&tag.name)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?
            .is_some_and(|existing| existing.target_index == index)
        {
            continue;
        }
        out.push(match tag.annotation {
            Some(a) => crate::tags::Tag::annotated(
                &tag.name,
                index,
                &a.message,
                &a.tagger,
                a.timestamp,
                &a.tz,
            ),
            None => crate::tags::Tag::lightweight(&tag.name, index),
        });
    }
    Ok(out)
}

/// How many imported commits land per store transaction. Each batch costs one
/// CAS device barrier plus one transaction, so this is the whole difference
/// between importing history at disk speed and at fsync speed. Small enough
/// that an interrupted import re-does at most this many commits on retry.
const COMMIT_BATCH: usize = 512;

/// Land a batch of freshly built leaves in ONE store transaction.
///
/// Ordering is the same contract `commit_raw` had, just amortized: the
/// batch's tree nodes are made durable before the transaction that references
/// them, so a leaf can never outlive its tree. A crash between the
/// transaction and the mapping save is the pre-existing recoverable window —
/// each leaf carries its `git.sha1`, so a retry rediscovers it from history
/// instead of appending a duplicate.
fn land_commit_batch(
    repo: &mut crate::repo::Repo,
    cas: &crate::cas::FileCas,
    mapping: &mut crate::remote::HashMapping,
    leaf_idx_by_hash: &mut HashMap<B3Hash, u64>,
    branch: &str,
    batch: &mut Vec<(String, crate::leaf::Leaf)>,
) -> Result<(), GitRemoteError> {
    if batch.is_empty() {
        return Ok(());
    }
    cas.flush().map_err(|e| GitRemoteError::Io(e.to_string()))?;
    let (shas, leaves): (Vec<String>, Vec<crate::leaf::Leaf>) =
        std::mem::take(batch).into_iter().unzip();
    let results = repo
        .commit_batch_raw(leaves, branch)
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    crate::failpoint::fail_point("import.mid_commit_loop");
    for (sha, result) in shas.iter().zip(results.iter()) {
        mapping.insert(sha, result.hash);
        leaf_idx_by_hash.insert(result.hash, result.index);
    }
    mapping
        .save()
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    Ok(())
}

/// Topologically order all commits reachable from `sha` (parents before
/// children). Iterative with an explicit stack: history depth equals commit
/// count, so recursion would blow the stack on any large (or
/// attacker-shaped) history.
fn collect_commit_order(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    shallow: &BTreeSet<String>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) -> Result<(), GitRemoteError> {
    // (sha, expanded): expanded=false → visit parents first; true → emit.
    let mut stack: Vec<(String, bool)> = vec![(sha.to_string(), false)];
    while let Some((sha, expanded)) = stack.pop() {
        if expanded {
            out.push(sha);
            continue;
        }
        if !seen.insert(sha.clone()) {
            continue;
        }
        let object = objects
            .get(&sha)
            .ok_or_else(|| GitRemoteError::Protocol(format!("missing commit object {}", sha)))?;
        let commit = parse_commit(&object.data)?;
        let boundary = shallow.contains(&sha);
        stack.push((sha, true));
        // A shallow boundary's parents are outside the pack by design; walking
        // into them would fail the same way a truncated pack does.
        if boundary {
            continue;
        }
        for parent in commit.parents.iter().rev() {
            if !seen.contains(parent) {
                stack.push((parent.clone(), false));
            }
        }
    }
    Ok(())
}

fn import_tree(
    sha: &str,
    path_prefix: &str,
    objects: &HashMap<String, GitObject>,
    store: &crate::fsmerkle::FsStore<'_>,
    mapping: &mut crate::remote::HashMapping,
    tree_cache: &mut HashMap<String, B3Hash>,
    submodules_skipped: &mut std::collections::BTreeSet<String>,
) -> Result<B3Hash, GitRemoteError> {
    use crate::fsmerkle::{Entry, MODE_DIR, MODE_EXEC, MODE_FILE, MODE_SYMLINK, NodeKind};

    // Depth guard: `tree_cache` only dedupes completed subtrees, so a
    // self-referencing tree in a hostile pack would otherwise recurse
    // forever. Path depth == number of separators + 1.
    if path_prefix.split('/').count() >= MAX_TREE_DEPTH {
        return Err(GitRemoteError::Protocol(format!(
            "tree nesting exceeds {} levels — refusing (malformed or hostile pack)",
            MAX_TREE_DEPTH
        )));
    }

    if let Some(hash) = tree_cache.get(sha).copied() {
        return Ok(hash);
    }

    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing tree object {}", sha)))?;
    let entries = parse_tree(&object.data)?;
    // Track which output names we've added so duplicates from rename collisions
    // (`.gitignore` → `.ivaldiignore` when both exist) resolve deterministically:
    // a real `.ivaldiignore` always wins over a renamed `.gitignore`.
    let mut ivaldi_entries: Vec<Entry> = Vec::new();
    let mut seen_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut rename_present: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for entry in entries {
        // Translate dotfile policy at import time:
        // - `.gitignore` is renamed to `.ivaldiignore` (same blob, new name).
        // - `.ivaldiignore` passes through unchanged.
        // - Every other dotfile is dropped — ivaldi auto-ignores dotfiles
        //   in the workspace, and importing them would just create phantom
        //   "deleted" entries in status/auto-shelf.
        let mapped_name = match entry.name.as_str() {
            ".ivaldiignore" => Some(entry.name.clone()),
            ".gitignore" => Some(".ivaldiignore".to_string()),
            n if n.starts_with('.') && entry.mode != "40000" && entry.mode != "040000" => None,
            _ => Some(entry.name.clone()),
        };

        let Some(out_name) = mapped_name else {
            continue;
        };

        let child_path = if path_prefix.is_empty() {
            out_name.clone()
        } else {
            format!("{}/{}", path_prefix, out_name)
        };

        match entry.mode.as_str() {
            "40000" | "040000" => {
                let hash = import_tree(
                    &entry.sha,
                    &child_path,
                    objects,
                    store,
                    mapping,
                    tree_cache,
                    submodules_skipped,
                )?;
                ivaldi_entries.push(Entry {
                    name: out_name,
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash,
                });
            }
            "100644" | "100755" | "120000" => {
                let hash = mapping.get_blake3(&entry.sha).ok_or_else(|| {
                    GitRemoteError::Protocol(format!(
                        "blob {} missing from mapping after prefetch",
                        entry.sha
                    ))
                })?;
                let was_renamed = entry.name == ".gitignore";
                if was_renamed {
                    rename_present.insert(out_name.clone());
                    if seen_names.contains(&out_name) {
                        // A real `.ivaldiignore` already won this slot; drop
                        // the renamed `.gitignore`.
                        continue;
                    }
                } else if out_name == ".ivaldiignore" && rename_present.contains(&out_name) {
                    // We earlier kept a renamed `.gitignore`; replace it now
                    // that the real `.ivaldiignore` is here.
                    ivaldi_entries.retain(|e| e.name != out_name);
                }
                seen_names.insert(out_name.clone());
                // Preserve the original git file mode so the round-trip stays
                // byte-for-byte: executable bit (100755) and symlinks (120000)
                // must not collapse to a plain file (100644).
                let mode = match entry.mode.as_str() {
                    "100755" => MODE_EXEC,
                    "120000" => MODE_SYMLINK,
                    _ => MODE_FILE,
                };
                ivaldi_entries.push(Entry {
                    name: out_name,
                    mode,
                    kind: NodeKind::Blob,
                    hash,
                });
            }
            "160000" => {
                // Submodule (gitlink). We don't yet clone or track the
                // submodule's repository; record the path so the user can see
                // what we skipped. Logged once via crate::logging::warn so a
                // huge repo doesn't spam the console.
                submodules_skipped.insert(child_path.clone());
            }
            other => {
                return Err(GitRemoteError::Unsupported(format!(
                    "unsupported tree entry mode {}",
                    other
                )));
            }
        }
    }

    let hash = store
        .put_tree(ivaldi_entries)
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    // Tree identity is deterministic just like blob identity. Persisting it
    // lets a later upload reference a remote-known subtree without loading
    // and re-hashing every blob below it.
    mapping.insert(sha, hash);
    tree_cache.insert(sha.to_string(), hash);
    Ok(hash)
}

fn collect_reachable_blobs(
    head_sha: &str,
    extra_roots: &[&str],
    objects: &HashMap<String, GitObject>,
    shallow: &BTreeSet<String>,
) -> Result<Vec<String>, GitRemoteError> {
    let mut seen_commits = std::collections::HashSet::new();
    let mut seen_trees = std::collections::HashSet::new();
    let mut seen_blobs = std::collections::HashSet::new();
    let mut order = Vec::new();
    // Iterate commits with a worklist: recursing per commit would blow the
    // stack on deep histories.
    let mut queue: Vec<String> = vec![head_sha.to_string()];
    queue.extend(extra_roots.iter().map(|s| s.to_string()));
    while let Some(sha) = queue.pop() {
        if !seen_commits.insert(sha.clone()) {
            continue;
        }
        let object = objects
            .get(&sha)
            .ok_or_else(|| GitRemoteError::Protocol(format!("missing commit object {}", sha)))?;
        let commit = parse_commit(&object.data)?;
        walk_tree_for_blobs(
            &commit.tree,
            objects,
            &mut seen_trees,
            &mut seen_blobs,
            &mut order,
            0,
        )?;
        // Same boundary rule as `collect_commit_order`: a shallow commit's
        // parents (and their trees) were never sent.
        if shallow.contains(&sha) {
            continue;
        }
        for parent in &commit.parents {
            if !seen_commits.contains(parent) {
                queue.push(parent.clone());
            }
        }
    }
    Ok(order)
}

fn walk_tree_for_blobs(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    seen_trees: &mut std::collections::HashSet<String>,
    seen_blobs: &mut std::collections::HashSet<String>,
    order: &mut Vec<String>,
    depth: usize,
) -> Result<(), GitRemoteError> {
    if depth >= MAX_TREE_DEPTH {
        return Err(GitRemoteError::Protocol(format!(
            "tree nesting exceeds {} levels — refusing (malformed or hostile pack)",
            MAX_TREE_DEPTH
        )));
    }
    if !seen_trees.insert(sha.to_string()) {
        return Ok(());
    }
    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing tree object {}", sha)))?;
    for entry in parse_tree(&object.data)? {
        match entry.mode.as_str() {
            "40000" | "040000" => {
                walk_tree_for_blobs(
                    &entry.sha,
                    objects,
                    seen_trees,
                    seen_blobs,
                    order,
                    depth + 1,
                )?;
            }
            "100644" | "100755" | "120000" => {
                if seen_blobs.insert(entry.sha.clone()) {
                    order.push(entry.sha);
                }
            }
            "160000" => {}
            other => {
                return Err(GitRemoteError::Unsupported(format!(
                    "unsupported tree entry mode {}",
                    other
                )));
            }
        }
    }
    Ok(())
}

/// Write every reachable blob to the CAS in parallel and return the
/// (git_sha → blake3) pairs to merge into `HashMapping`. Skips blobs already
/// known to the mapping. Drives the blob progress bar.
fn prefetch_blobs(
    blob_shas: &[String],
    objects: &HashMap<String, GitObject>,
    store: &crate::fsmerkle::FsStore<'_>,
    mapping: &crate::remote::HashMapping,
    pb_blobs: &indicatif::ProgressBar,
) -> Result<Vec<(String, B3Hash)>, GitRemoteError> {
    let pending: Vec<&String> = blob_shas
        .iter()
        .filter(|sha| mapping.get_blake3(sha).is_none())
        .collect();

    pending
        .par_iter()
        .map(|sha| {
            let blob = objects
                .get(sha.as_str())
                .ok_or_else(|| GitRemoteError::Protocol(format!("missing blob object {}", sha)))?;
            let (hash, _) = store
                .put_blob(&blob.data)
                .map_err(|e| GitRemoteError::Io(e.to_string()))?;
            pb_blobs.inc(1);
            Ok(((*sha).clone(), hash))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::HashMapping;

    #[test]
    fn pkt_line_roundtrip() {
        let lines = parse_pkt_lines(&[pkt_line("hello\n"), b"0000".to_vec()].concat()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].as_deref(), Some(b"hello\n".as_slice()));
        assert!(lines[1].is_none());
    }

    fn pkt(payload: &str) -> Vec<u8> {
        pkt_line(payload)
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
    fn tag_wants_skips_peeled_refs_branches_and_the_tip() {
        let refs = vec![
            AdvertisedRef {
                id: "aa".into(),
                name: "refs/heads/main".into(),
            },
            AdvertisedRef {
                id: "bb".into(),
                name: "refs/tags/v1.0".into(),
            },
            AdvertisedRef {
                id: "cc".into(),
                name: "refs/tags/v1.0^{}".into(),
            },
            // A tag on the tip we already want, and a duplicate id: both are
            // already covered, so re-asking wastes a want line.
            AdvertisedRef {
                id: "aa".into(),
                name: "refs/tags/tip".into(),
            },
            AdvertisedRef {
                id: "bb".into(),
                name: "refs/tags/alias".into(),
            },
        ];
        assert_eq!(tag_wants(&refs, "aa"), vec!["bb".to_string()]);
    }

    #[test]
    fn annotated_tag_object_yields_target_tagger_and_message() {
        let data = b"object 1111111111111111111111111111111111111111\n\
            type commit\n\
            tag v1.0\n\
            tagger Jane Doe <jane@example.com> 1710000000 -0500\n\
            \nRelease notes\n\nwith a blank line\n";
        let parsed = parse_tag_object(data).unwrap();
        assert_eq!(parsed.object, "1111111111111111111111111111111111111111");
        assert_eq!(parsed.tagger, "Jane Doe <jane@example.com>");
        assert_eq!(parsed.timestamp, 1710000000);
        assert_eq!(parsed.tz, "-0500");
        assert_eq!(parsed.message, "Release notes\n\nwith a blank line\n");
    }

    #[test]
    fn tag_object_without_a_target_is_refused() {
        let err = parse_tag_object(b"type commit\ntag v1.0\n\nhi").unwrap_err();
        assert!(err.to_string().contains("no target"), "{}", err);
    }

    #[test]
    fn upload_pack_response_collects_shallow_boundaries_before_the_pack() {
        // What a server sends for `deepen 1`: the shallow-update block, a
        // flush, NAK, then the pack muxed over side-band channel 1.
        let mut bytes = Vec::new();
        bytes.extend(pkt("shallow 1111111111111111111111111111111111111111\n"));
        bytes.extend(pkt("shallow 2222222222222222222222222222222222222222\n"));
        bytes.extend(b"0000");
        bytes.extend(pkt("NAK\n"));
        let pack = encode_pack_for_tests(&[(GitObjectKind::Blob, b"hi".to_vec())]);
        let mut payload = vec![1u8];
        payload.extend_from_slice(&pack);
        bytes.extend(format!("{:04x}", payload.len() + 4).into_bytes());
        bytes.extend(payload);

        let r = extract_pack_from_upload_pack(&bytes).unwrap();
        assert_eq!(&r.pack[..4], b"PACK");
        assert_eq!(r.shallow.len(), 2);
        assert!(r.shallow.contains("1111111111111111111111111111111111111111"));
    }

    #[test]
    fn full_fetch_reports_no_shallow_boundaries() {
        let pack = encode_pack_for_tests(&[(GitObjectKind::Blob, b"hi".to_vec())]);
        let mut bytes = pkt("NAK\n");
        let mut payload = vec![1u8];
        payload.extend_from_slice(&pack);
        bytes.extend(format!("{:04x}", payload.len() + 4).into_bytes());
        bytes.extend(payload);

        let r = extract_pack_from_upload_pack(&bytes).unwrap();
        assert!(r.shallow.is_empty());
    }

    #[test]
    fn parse_report_status_demuxes_sideband_failure() {
        // Reproduce GitHub's real reply: the report-status muxed over side-band
        // channel 1, i.e. the `\x01001dunpack index-pack failed` bytes the user
        // saw. Before the demux fix this parsed as the opaque "unexpected
        // receive-pack first line" error and could even masquerade as success.
        fn sideband1(inner: &[u8]) -> Vec<u8> {
            let mut payload = vec![1u8];
            payload.extend_from_slice(inner);
            let mut out = format!("{:04x}", payload.len() + 4).into_bytes();
            out.extend_from_slice(&payload);
            out
        }
        let mut inner = Vec::new();
        inner.extend(pkt("unpack index-pack failed\n"));
        inner.extend(pkt("ng refs/heads/carrier-mac-vm unpacker error\n"));
        inner.extend(b"0000"); // inner (report-status) flush

        let mut bytes = sideband1(&inner);
        bytes.extend(b"0000"); // outer (side-band) flush

        let r = parse_report_status(&bytes).unwrap();
        assert!(!r.unpack_ok);
        assert_eq!(r.unpack_error.as_deref(), Some("index-pack failed"));
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].name, "refs/heads/carrier-mac-vm");
        assert_eq!(r.refs[0].error.as_deref(), Some("unpacker error"));
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

    #[test]
    fn parse_ref_advertisement_with_symref() {
        let mut adv = Vec::new();
        adv.extend(pkt_line("# service=git-upload-pack\n"));
        adv.extend(b"0000");
        adv.extend(pkt_line("0123456789012345678901234567890123456789 HEAD\0symref=HEAD:refs/heads/main agent=git/github\n"));
        adv.extend(pkt_line(
            "0123456789012345678901234567890123456789 refs/heads/main\n",
        ));
        adv.extend(b"0000");

        let parsed = parse_discovery(&adv).unwrap();
        assert_eq!(parsed.default_branch.as_deref(), Some("main"));
        assert_eq!(parsed.refs.len(), 2);
    }

    #[test]
    fn delta_apply_insert_and_copy() {
        let base = b"hello world";
        let delta = vec![11, 11, 0x90, 11];
        let result = apply_delta(base, &delta).unwrap();
        assert_eq!(result, base);
    }

    #[test]
    fn parse_packfile_with_base_objects() {
        let blob = b"hello".to_vec();
        let tree = {
            let mut v = Vec::new();
            v.extend_from_slice(b"100644 file.txt\0");
            v.extend_from_slice(&hex::decode(git_object_id(GitObjectKind::Blob, &blob)).unwrap());
            v
        };
        let commit = format!(
            "tree {}\nauthor Test <test@example.com> 1710000000 +0000\ncommitter Test <test@example.com> 1710000000 +0000\n\nmsg\n",
            git_object_id(GitObjectKind::Tree, &tree)
        ).into_bytes();

        let pack = encode_pack_for_tests(&[
            (GitObjectKind::Blob, blob.clone()),
            (GitObjectKind::Tree, tree.clone()),
            (GitObjectKind::Commit, commit.clone()),
        ]);

        let objects = parse_packfile(&pack).unwrap();
        assert!(objects.contains_key(&git_object_id(GitObjectKind::Blob, &blob)));
        assert!(objects.contains_key(&git_object_id(GitObjectKind::Tree, &tree)));
        assert!(objects.contains_key(&git_object_id(GitObjectKind::Commit, &commit)));
    }

    #[test]
    fn parse_commit_metadata() {
        let commit = b"tree abcdef\nparent 123456\nauthor Jane Doe <jane@example.com> 1710000000 +0000\ncommitter Jane Doe <jane@example.com> 1710000000 +0000\n\nhello\n";
        let parsed = parse_commit(commit).unwrap();
        assert_eq!(parsed.tree, "abcdef");
        assert_eq!(parsed.parents, vec!["123456"]);
        assert_eq!(parsed.author_name, "Jane Doe");
        assert_eq!(parsed.author_email, "jane@example.com");
        assert_eq!(parsed.author_time, 1710000000);
        assert_eq!(parsed.message, "hello\n");
    }

    #[test]
    fn parse_commit_captures_distinct_committer_and_timezones() {
        // Distinct author and committer with non-UTC timezones — the parser
        // must keep both identities and both offsets, not collapse them.
        let commit = b"tree abcdef\n\
            author Jane Doe <jane@example.com> 1710000000 -0500\n\
            committer Bob <bob@example.com> 1710001000 +0100\n\
            \nhello\n";
        let parsed = parse_commit(commit).unwrap();
        assert_eq!(parsed.author_name, "Jane Doe");
        assert_eq!(parsed.author_email, "jane@example.com");
        assert_eq!(parsed.author_time, 1710000000);
        assert_eq!(parsed.author_tz, "-0500");
        assert_eq!(parsed.committer_name, "Bob");
        assert_eq!(parsed.committer_email, "bob@example.com");
        assert_eq!(parsed.committer_time, 1710001000);
        assert_eq!(parsed.committer_tz, "+0100");
    }

    #[test]
    fn parse_tree_entries() {
        let mut tree = Vec::new();
        tree.extend_from_slice(b"100644 a.txt\0");
        tree.extend_from_slice(&[1u8; 20]);
        tree.extend_from_slice(b"40000 src\0");
        tree.extend_from_slice(&[2u8; 20]);
        let parsed = parse_tree(&tree).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "a.txt");
        assert_eq!(parsed[1].mode, "40000");
    }

    #[test]
    fn sha1_digest_matches_known_value() {
        assert_eq!(
            hex::encode(sha1_digest(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    // ---- Hostile-input tests: network bytes must error, never allocate
    // ---- unbounded, panic, or blow the stack.

    fn pack_header(count: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(b"PACK");
        p.extend_from_slice(&2u32.to_be_bytes());
        p.extend_from_slice(&count.to_be_bytes());
        p
    }

    #[test]
    fn packfile_with_forged_huge_entry_count_errors_without_allocating() {
        // Claims u32::MAX entries with an empty body; must error out of the
        // count sanity check before Vec::with_capacity.
        let pack = pack_header(u32::MAX);
        let err = parse_packfile(&pack).unwrap_err();
        assert!(err.to_string().contains("claims"), "{}", err);
    }

    #[test]
    fn object_header_with_giant_declared_size_is_rejected() {
        let mut pack = pack_header(1);
        // Object header declaring ~2^60 bytes: type=blob(3), size varint with
        // many continuation bytes.
        pack.push(0x80 | (3 << 4) | 0x0f);
        pack.extend(std::iter::repeat_n(0xffu8, 7));
        pack.push(0x7f);
        let err = parse_packfile(&pack).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exceeds") || msg.contains("varint"),
            "unexpected error: {}",
            msg
        );
    }

    #[test]
    fn object_header_varint_with_endless_continuation_errors() {
        let mut data = vec![0x80 | (3 << 4)];
        data.extend(std::iter::repeat_n(0xffu8, 64));
        let err = parse_object_header(&data).unwrap_err();
        assert!(err.to_string().contains("varint too long"), "{}", err);
    }

    #[test]
    fn zlib_stream_longer_than_declared_size_is_rejected() {
        // Entry declares size 1 but the zlib stream inflates to 5 bytes.
        let mut pack = pack_header(1);
        pack.push((3 << 4) | 1); // blob, size=1
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"hello").unwrap();
        pack.extend(enc.finish().unwrap());
        let err = parse_packfile(&pack).unwrap_err();
        assert!(err.to_string().contains("declared"), "{}", err);
    }

    #[test]
    fn delta_with_forged_result_size_errors_before_allocation() {
        // base_size = 0 (matches empty base), result_size = 1 << 40.
        let mut delta = vec![0x00];
        let mut size = 1u64 << 40;
        while size > 0 {
            let mut b = (size & 0x7f) as u8;
            size >>= 7;
            if size > 0 {
                b |= 0x80;
            }
            delta.push(b);
        }
        let err = apply_delta(&[], &delta).unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{}", err);
    }

    #[test]
    fn delta_varint_with_64_continuation_bytes_errors() {
        let delta = vec![0xffu8; 64];
        assert!(apply_delta(&[], &delta).is_err());
    }

    #[test]
    fn ofs_delta_base_varint_overflow_is_rejected() {
        let data = vec![0xffu8; 16];
        assert!(parse_ofs_delta_base(&data, 100).is_err());
    }

    #[test]
    fn deep_commit_chain_does_not_overflow_the_stack() {
        // 100k-commit linear chain: the recursive walkers would blow the
        // stack; the iterative rewrites must handle it.
        let n = 100_000usize;
        let tree = Vec::new(); // empty tree object
        let tree_sha = git_object_id(GitObjectKind::Tree, &tree);
        let mut objects: HashMap<String, GitObject> = HashMap::new();
        objects.insert(
            tree_sha.clone(),
            GitObject {
                kind: GitObjectKind::Tree,
                data: tree,
            },
        );
        let mut parent: Option<String> = None;
        let mut head = String::new();
        for i in 0..n {
            let mut c = format!("tree {}\n", tree_sha);
            if let Some(p) = &parent {
                c.push_str(&format!("parent {}\n", p));
            }
            c.push_str(&format!(
                "author A <a@x> {} +0000\ncommitter A <a@x> {} +0000\n\nc{}\n",
                i, i, i
            ));
            let data = c.into_bytes();
            let sha = git_object_id(GitObjectKind::Commit, &data);
            objects.insert(
                sha.clone(),
                GitObject {
                    kind: GitObjectKind::Commit,
                    data,
                },
            );
            parent = Some(sha.clone());
            head = sha;
        }

        let mut seen = std::collections::HashSet::new();
        let mut order = Vec::new();
        collect_commit_order(&head, &objects, &BTreeSet::new(), &mut seen, &mut order).unwrap();
        assert_eq!(order.len(), n);
        // Topological: every commit appears after its parent.
        assert_eq!(order.last().unwrap(), &head);

        let blobs = collect_reachable_blobs(&head, &[], &objects, &BTreeSet::new()).unwrap();
        assert!(blobs.is_empty());
    }

    #[test]
    fn overly_deep_tree_nesting_is_rejected() {
        // Chain of trees nested beyond MAX_TREE_DEPTH.
        let mut objects: HashMap<String, GitObject> = HashMap::new();
        let mut child_sha: Option<String> = None;
        let mut top = String::new();
        for _ in 0..(MAX_TREE_DEPTH + 8) {
            let mut tree = Vec::new();
            if let Some(c) = &child_sha {
                tree.extend_from_slice(b"40000 d\0");
                tree.extend_from_slice(&hex::decode(c).unwrap());
            }
            let sha = git_object_id(GitObjectKind::Tree, &tree);
            objects.insert(
                sha.clone(),
                GitObject {
                    kind: GitObjectKind::Tree,
                    data: tree,
                },
            );
            child_sha = Some(sha.clone());
            top = sha;
        }
        let commit = format!(
            "tree {}\nauthor A <a@x> 0 +0000\ncommitter A <a@x> 0 +0000\n\nm\n",
            top
        )
        .into_bytes();
        let head = git_object_id(GitObjectKind::Commit, &commit);
        objects.insert(
            head.clone(),
            GitObject {
                kind: GitObjectKind::Commit,
                data: commit,
            },
        );

        let err = collect_reachable_blobs(&head, &[], &objects, &BTreeSet::new()).unwrap_err();
        assert!(err.to_string().contains("nesting"), "{}", err);
    }

    // ---- Non-fast-forward push guard (pure local check, no server).

    fn repo_with_two_seals() -> (tempfile::TempDir, crate::repo::Repo) {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = crate::fsmerkle::FsStore::new(&cas);
        for body in [&b"one"[..], &b"two"[..]] {
            let (blob, _) = store.put_blob(body).unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree = store
                .put_tree(vec![Entry {
                    name: "f.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob,
                }])
                .unwrap();
            repo.commit(tree, "t <t@x>", "seal").unwrap();
        }
        (dir, repo)
    }

    /// A two-seal repo with `v1.0` on the oldest seal, ready to plan pushes.
    ///
    /// Seal timestamps come from the clock, so anything comparing object ids
    /// across two plans has to reuse ONE repo — two `repo_with_two_seals()`
    /// calls land in different seconds under load and mint different commits.
    fn repo_with_a_tag() -> (tempfile::TempDir, crate::repo::Repo) {
        let (dir, repo) = repo_with_two_seals();
        repo.set_tags(&[crate::tags::Tag::lightweight("v1.0", 0)])
            .unwrap();
        (dir, repo)
    }

    fn plan_for(
        repo: &crate::repo::Repo,
        advertised: Vec<AdvertisedRef>,
        include_tags: bool,
        force: bool,
    ) -> PushPlan {
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        plan_push(repo, "main", 1, &advertised, &mapping, include_tags, force).unwrap()
    }

    fn advertised_ref(name: &str, sha: [u8; 20]) -> AdvertisedRef {
        AdvertisedRef {
            id: hex::encode(sha),
            name: name.into(),
        }
    }

    fn update_named<'a>(plan: &'a PushPlan, name: &str) -> Option<&'a (String, String, String)> {
        plan.updates.iter().find(|(n, _, _)| n == name)
    }

    #[test]
    fn push_plan_omits_tags_unless_asked() {
        let (_dir, repo) = repo_with_a_tag();
        let plan = plan_for(&repo, Vec::new(), false, false);
        assert!(update_named(&plan, "refs/heads/main").is_some());
        assert!(
            plan.updates.iter().all(|(n, _, _)| !n.starts_with("refs/tags/")),
            "tags must not ride along on a plain push: {:?}",
            plan.updates
        );
    }

    #[test]
    fn incremental_push_omits_unchanged_blob_from_remote_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = crate::fsmerkle::FsStore::new(&cas);
        use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};

        let (stable, _) = store.put_blob(b"large unchanged payload").unwrap();
        let (old, _) = store.put_blob(b"old").unwrap();
        let first_tree = store
            .put_tree(vec![
                Entry {
                    name: "stable.bin".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: stable,
                },
                Entry {
                    name: "value.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: old,
                },
            ])
            .unwrap();
        repo.commit(first_tree, "t <t@x>", "first").unwrap();

        // Export once to establish the exact Git identities the advertised
        // remote tip would have. This also exercises persistent tree/blob
        // identity caching.
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let first = crate::git_export::export_chain(&repo, 0, &mapping, &BTreeSet::new()).unwrap();

        let (new, _) = store.put_blob(b"new").unwrap();
        let second_tree = store
            .put_tree(vec![
                Entry {
                    name: "stable.bin".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: stable,
                },
                Entry {
                    name: "value.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: new,
                },
            ])
            .unwrap();
        repo.commit(second_tree, "t <t@x>", "second").unwrap();

        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let plan = plan_push(
            &repo,
            "main",
            1,
            &[advertised_ref("refs/heads/main", first.tip_sha1)],
            &mapping,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.objects.len(), 3, "new blob + tree + commit only");
        assert!(plan.objects.values().any(|o| o.body == b"new"));
        assert!(
            plan.objects
                .values()
                .all(|o| o.body != b"large unchanged payload"),
            "a blob reachable from the advertised snapshot must not be resent"
        );
    }

    #[test]
    fn advertised_tip_expands_to_internal_ancestor_for_new_branch() {
        let (dir, mut repo) = repo_with_two_seals();
        let cas = crate::cas::FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = crate::fsmerkle::FsStore::new(&cas);
        let (blob, _) = store.put_blob(b"three").unwrap();
        let tree = store
            .put_tree(vec![crate::fsmerkle::Entry {
                name: "f.txt".into(),
                mode: crate::fsmerkle::MODE_FILE,
                kind: crate::fsmerkle::NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        repo.commit(tree, "t <t@x>", "third").unwrap();

        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let remote_main =
            crate::git_export::export_chain(&repo, 2, &mapping, &BTreeSet::new()).unwrap();
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let plan = plan_push(
            &repo,
            "fork",
            1,
            &[advertised_ref("refs/heads/main", remote_main.tip_sha1)],
            &mapping,
            false,
            false,
        )
        .unwrap();

        assert!(
            plan.objects.is_empty(),
            "the remote main tip already reaches the fork commit"
        );
        assert!(update_named(&plan, "refs/heads/fork").is_some());
    }

    #[test]
    fn push_plan_creates_a_tag_ref_on_an_empty_remote() {
        let (_dir, repo) = repo_with_a_tag();
        let plan = plan_for(&repo, Vec::new(), true, false);
        let (_, old, new) = update_named(&plan, "refs/tags/v1.0").expect("tag update");
        assert_eq!(old, ZERO_SHA1, "a new remote tag starts from zeros");
        assert_ne!(new, ZERO_SHA1);
        assert!(plan.skipped_tags.is_empty());
    }

    #[test]
    fn push_plan_refuses_to_move_an_existing_remote_tag_without_force() {
        // The remote already publishes v1.0 at some other object.
        let advertised = vec![AdvertisedRef {
            id: "1".repeat(40),
            name: "refs/tags/v1.0".into(),
        }];
        let (_dir, repo) = repo_with_a_tag();
        let plan = plan_for(&repo, advertised.clone(), true, false);
        assert!(
            update_named(&plan, "refs/tags/v1.0").is_none(),
            "moving a published tag needs --force"
        );
        assert_eq!(plan.skipped_tags.len(), 1);
        assert!(plan.skipped_tags[0].contains("v1.0"));

        // ...and --force is exactly what lets it through.
        let forced = plan_for(&repo, advertised, true, true);
        assert!(update_named(&forced, "refs/tags/v1.0").is_some());
        assert!(forced.skipped_tags.is_empty());
    }

    #[test]
    fn push_plan_skips_a_tag_the_remote_already_has_at_the_same_object() {
        // Plan once against an empty remote to learn the tag's object id,
        // then replay that id as the remote's advertisement — same repo, so
        // the second plan mints the identical object.
        let (_dir, repo) = repo_with_a_tag();
        let first = plan_for(&repo, Vec::new(), true, false);
        let (_, _, new) = update_named(&first, "refs/tags/v1.0").unwrap().clone();

        let second = plan_for(
            &repo,
            vec![AdvertisedRef {
                id: new,
                name: "refs/tags/v1.0".into(),
            }],
            true,
            false,
        );
        assert!(update_named(&second, "refs/tags/v1.0").is_none());
        assert!(
            second.skipped_tags.is_empty(),
            "already-present is not a refusal worth reporting"
        );
    }

    #[test]
    fn receive_pack_commands_put_capabilities_on_the_first_line_only() {
        let updates = vec![
            ("refs/heads/main".to_string(), "a".repeat(40), "b".repeat(40)),
            ("refs/tags/v1".to_string(), ZERO_SHA1.to_string(), "c".repeat(40)),
        ];
        let body = receive_pack_commands(&updates);
        let text = String::from_utf8_lossy(&body);
        assert_eq!(
            text.matches("report-status").count(),
            1,
            "capabilities ride on the first command only"
        );
        assert!(text.contains("refs/heads/main"));
        assert!(text.contains("refs/tags/v1"));
        assert!(text.ends_with("0000"), "commands end with a flush-pkt");
    }

    #[test]
    fn push_guard_rejects_unknown_remote_tip_without_force() {
        let (_dir, repo) = repo_with_two_seals();
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let unknown_tip = "ab".repeat(20);
        let err =
            check_push_fast_forward(&repo, &mapping, "main", &unknown_tip, 1, false).unwrap_err();
        assert!(err.to_string().contains("seals you do not have"), "{}", err);
    }

    #[test]
    fn push_guard_allows_force() {
        let (_dir, repo) = repo_with_two_seals();
        let mapping = HashMapping::new(&repo.ivaldi_dir);
        let unknown_tip = "ab".repeat(20);
        assert!(check_push_fast_forward(&repo, &mapping, "main", &unknown_tip, 1, true).is_ok());
    }

    #[test]
    fn push_guard_allows_fast_forward_from_mapped_ancestor() {
        let (_dir, repo) = repo_with_two_seals();
        let ancestor = repo.get_leaf(0).unwrap().unwrap();
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let remote_tip = "cd".repeat(20);
        mapping.insert(&remote_tip, ancestor.hash());
        assert!(check_push_fast_forward(&repo, &mapping, "main", &remote_tip, 1, false).is_ok());
    }

    #[test]
    fn push_guard_rejects_diverged_remote_tip() {
        let (_dir, repo) = repo_with_two_seals();
        // Remote tip maps to leaf 1, but we're pushing from head at leaf 0 —
        // the remote has a seal our chain does not include.
        let other = repo.get_leaf(1).unwrap().unwrap();
        let mut mapping = HashMapping::new(&repo.ivaldi_dir);
        let remote_tip = "ef".repeat(20);
        mapping.insert(&remote_tip, other.hash());
        assert!(check_push_fast_forward(&repo, &mapping, "main", &remote_tip, 0, false).is_err());
    }
}
