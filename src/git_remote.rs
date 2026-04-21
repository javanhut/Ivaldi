//! Pure Rust remote fetch support for Git-compatible Smart HTTP servers.
//!
//! This module implements the minimal subset needed by `ivaldi download`:
//! ref advertisement, upload-pack fetch, packfile parsing, and object import.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use flate2::Compression;
use flate2::bufread::ZlibDecoder;
use flate2::write::ZlibEncoder;
use indicatif::MultiProgress;

use crate::hash::B3Hash;
use crate::progress;
use crate::remote::RemoteBranch;

const GITHUB_BASE: &str = "https://github.com";

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
}

impl SmartHttpClient {
    pub fn new(token: Option<&str>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(30))
            .timeout_read(std::time::Duration::from_secs(120))
            .build();
        Self {
            token: token.map(str::to_string),
            agent,
        }
    }

    pub fn fetch_repo(
        &self,
        owner: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<FetchResult, GitRemoteError> {
        let base = format!("{}/{}/{}.git", GITHUB_BASE, owner, repo);
        let discovery = self.discover_refs(&base)?;
        let explicit_branch = branch.is_some();
        let requested_branch = branch.map(str::to_string);
        let wanted_ref = requested_branch
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
                requested_branch
                    .clone()
                    .map(GitRemoteError::BranchNotFound)
                    .unwrap_or_else(|| {
                        GitRemoteError::Protocol(
                            "remote did not advertise a usable default ref".into(),
                        )
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
        let head_sha = selected.id.clone();

        let pack = self.fetch_pack(&base, &head_sha)?;
        let objects = parse_packfile(&pack)?;

        Ok(FetchResult {
            branch: branch_name,
            head_sha,
            refs: discovery.refs,
            objects,
        })
    }

    pub fn list_branch_refs(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<RemoteBranch>, GitRemoteError> {
        let base = format!("{}/{}/{}.git", GITHUB_BASE, owner, repo);
        let discovery = self.discover_refs(&base)?;
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

    fn discover_refs(&self, base: &str) -> Result<Discovery, GitRemoteError> {
        let pb = progress::spinner("Discovering remote refs");
        let url = format!("{}/info/refs?service=git-upload-pack", base);
        let build = |token: Option<&str>| {
            let mut r = self
                .agent
                .get(&url)
                .set("Accept", "application/x-git-upload-pack-advertisement")
                .set("User-Agent", "ivaldi-vcs/0.1.0");
            if let Some(t) = token {
                r = r.set("Authorization", &format!("Bearer {}", t));
            }
            r
        };

        let resp = match build(self.token.as_deref()).call() {
            Ok(resp) => resp,
            Err(err) => {
                // If we attached a stored token and the server rejected it,
                // the repo may still be public — retry anonymously once.
                if self.token.is_some() && is_auth_failure(&err) {
                    match build(None).call() {
                        Ok(resp) => {
                            crate::logging::warn(
                                "stored token rejected — falling back to anonymous access",
                            );
                            resp
                        }
                        Err(e2) => {
                            pb.finish_and_clear();
                            return Err(map_http_error(e2));
                        }
                    }
                } else {
                    pb.finish_and_clear();
                    return Err(map_http_error(err));
                }
            }
        };
        let mut bytes = Vec::new();
        resp.into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        pb.finish_with_message("Remote refs discovered");
        parse_discovery(&bytes)
    }

    fn fetch_pack(&self, base: &str, want_sha: &str) -> Result<Vec<u8>, GitRemoteError> {
        let url = format!("{}/git-upload-pack", base);
        let caps =
            "multi_ack_detailed side-band-64k ofs-delta no-progress include-tag agent=ivaldi/0.1.0";
        let mut body = Vec::new();
        body.extend(pkt_line(&format!("want {} {}\n", want_sha, caps)));
        body.extend_from_slice(b"0000");
        body.extend(pkt_line("done\n"));

        let build = |token: Option<&str>| {
            let mut r = self
                .agent
                .post(&url)
                .set("Content-Type", "application/x-git-upload-pack-request")
                .set("Accept", "application/x-git-upload-pack-result")
                .set("User-Agent", "ivaldi-vcs/0.1.0");
            if let Some(t) = token {
                r = r.set("Authorization", &format!("Bearer {}", t));
            }
            r
        };

        let resp = match build(self.token.as_deref()).send_bytes(&body) {
            Ok(resp) => resp,
            Err(err) => {
                if self.token.is_some() && is_auth_failure(&err) {
                    match build(None).send_bytes(&body) {
                        Ok(resp) => {
                            crate::logging::warn(
                                "stored token rejected — falling back to anonymous access",
                            );
                            resp
                        }
                        Err(e2) => return Err(map_http_error(e2)),
                    }
                } else {
                    return Err(map_http_error(err));
                }
            }
        };
        let total = resp
            .header("Content-Length")
            .and_then(|h| h.parse::<u64>().ok());
        let pb = total
            .map(|len| progress::byte_bar(len, "Downloading pack"))
            .unwrap_or_else(|| progress::spinner("Downloading pack"));
        let mut bytes = Vec::new();
        let mut reader = resp.into_reader();
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
}

#[derive(Debug)]
struct Discovery {
    refs: Vec<AdvertisedRef>,
    default_branch: Option<String>,
}

/// True for status codes that suggest the token was the problem, not the repo.
fn is_auth_failure(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::Status(401, _) => true,
        ureq::Error::Status(403, resp) => {
            // 403 with rate-limit headers is NOT an auth failure; retrying
            // anonymously would hit the same (or stricter) limit.
            resp.header("X-RateLimit-Remaining") != Some("0")
        }
        _ => false,
    }
}

fn map_http_error(err: ureq::Error) -> GitRemoteError {
    match err {
        ureq::Error::Status(status, resp) => {
            // Detect rate-limiting before consuming the body.
            if status == 403 && resp.header("X-RateLimit-Remaining") == Some("0") {
                let reset_at = resp
                    .header("X-RateLimit-Reset")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                return GitRemoteError::RateLimited { reset_at };
            }
            let mut body = String::new();
            let mut reader = resp.into_reader();
            let _ = reader.read_to_string(&mut body);
            match status {
                401 => GitRemoteError::AuthRequired,
                404 => GitRemoteError::RepoUnavailable,
                _ => GitRemoteError::Http {
                    status,
                    message: body.trim().to_string(),
                },
            }
        }
        ureq::Error::Transport(e) => GitRemoteError::Io(e.to_string()),
    }
}

fn pkt_line(payload: &str) -> Vec<u8> {
    let len = payload.len() + 4;
    let mut out = format!("{:04x}", len).into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out
}

fn parse_discovery(data: &[u8]) -> Result<Discovery, GitRemoteError> {
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

    if default_branch.is_none() {
        if let Some(head) = refs.iter().find(|r| r.name == "HEAD") {
            if let Some(target) = refs
                .iter()
                .find(|r| r.name.starts_with("refs/heads/") && r.id == head.id)
            {
                default_branch = target.name.strip_prefix("refs/heads/").map(str::to_string);
            }
        }
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

fn extract_pack_from_upload_pack(data: &[u8]) -> Result<Vec<u8>, GitRemoteError> {
    let mut pack = Vec::new();
    for line in parse_pkt_lines(data)? {
        let Some(line) = line else { continue };
        if line == b"NAK\n" || line.starts_with(b"ACK ") {
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
    Ok(pack)
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

fn parse_packfile(data: &[u8]) -> Result<HashMap<String, GitObject>, GitRemoteError> {
    if data.len() < 12 || &data[..4] != b"PACK" {
        return Err(GitRemoteError::Protocol("invalid packfile header".into()));
    }
    let version = u32::from_be_bytes(data[4..8].try_into().unwrap());
    if !(2..=3).contains(&version) {
        return Err(GitRemoteError::Unsupported(format!(
            "unsupported pack version {}",
            version
        )));
    }
    let count = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;

    let mut idx = 12usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = idx;
        let (kind, header_len, _size) = parse_object_header(&data[idx..])?;
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

        let (inflated, consumed) = inflate_from(&data[idx..])?;
        idx += consumed;
        entries.push(PackedEntry {
            offset,
            kind,
            data: inflated,
        });
    }

    let mut by_offset = HashMap::new();
    for entry in entries {
        by_offset.insert(entry.offset, entry);
    }

    let offsets: Vec<usize> = by_offset.keys().copied().collect();
    let mut resolved = HashMap::new();
    for offset in offsets {
        resolve_object(offset, &by_offset, &mut resolved)?;
    }
    Ok(resolved)
}

fn resolve_object(
    offset: usize,
    entries: &HashMap<usize, PackedEntry>,
    resolved: &mut HashMap<String, GitObject>,
) -> Result<(String, GitObject), GitRemoteError> {
    let entry = entries.get(&offset).ok_or_else(|| {
        GitRemoteError::Protocol(format!("missing pack entry at offset {}", offset))
    })?;

    match &entry.kind {
        PackedKind::Base(kind) => {
            let sha = git_object_id(*kind, &entry.data);
            if let Some(existing) = resolved.get(&sha) {
                return Ok((sha, existing.clone()));
            }
            let object = GitObject {
                kind: *kind,
                data: entry.data.clone(),
            };
            resolved.insert(sha.clone(), object.clone());
            Ok((sha, object))
        }
        PackedKind::OfsDelta { base_offset } => {
            let (base_sha, base_obj) = resolve_object(*base_offset, entries, resolved)?;
            let _ = base_sha;
            let data = apply_delta(&base_obj.data, &entry.data)?;
            let sha = git_object_id(base_obj.kind, &data);
            if let Some(existing) = resolved.get(&sha) {
                return Ok((sha, existing.clone()));
            }
            let object = GitObject {
                kind: base_obj.kind,
                data,
            };
            resolved.insert(sha.clone(), object.clone());
            Ok((sha, object))
        }
        PackedKind::RefDelta { base_sha } => {
            let base_obj = resolved.get(base_sha).cloned().ok_or_else(|| {
                GitRemoteError::Protocol(format!("missing ref-delta base {}", base_sha))
            })?;
            let data = apply_delta(&base_obj.data, &entry.data)?;
            let sha = git_object_id(base_obj.kind, &data);
            if let Some(existing) = resolved.get(&sha) {
                return Ok((sha, existing.clone()));
            }
            let object = GitObject {
                kind: base_obj.kind,
                data,
            };
            resolved.insert(sha.clone(), object.clone());
            Ok((sha, object))
        }
    }
}

fn parse_object_header(data: &[u8]) -> Result<(u8, usize, usize), GitRemoteError> {
    let first = *data
        .first()
        .ok_or_else(|| GitRemoteError::Protocol("truncated object header".into()))?;
    let kind = (first >> 4) & 0x07;
    let mut size = (first & 0x0f) as usize;
    let mut shift = 4usize;
    let mut used = 1usize;
    let mut byte = first;

    while byte & 0x80 != 0 {
        byte = *data
            .get(used)
            .ok_or_else(|| GitRemoteError::Protocol("truncated object header".into()))?;
        size |= ((byte & 0x7f) as usize) << shift;
        shift += 7;
        used += 1;
    }

    Ok((kind, used, size))
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
    let mut value = (byte & 0x7f) as usize;
    while byte & 0x80 != 0 {
        byte = *data
            .get(used)
            .ok_or_else(|| GitRemoteError::Protocol("truncated ofs-delta base".into()))?;
        used += 1;
        value = ((value + 1) << 7) | ((byte & 0x7f) as usize);
    }
    current_offset
        .checked_sub(value)
        .map(|base| (base, used))
        .ok_or_else(|| GitRemoteError::Protocol("invalid ofs-delta base offset".into()))
}

fn inflate_from(data: &[u8]) -> Result<(Vec<u8>, usize), GitRemoteError> {
    let reader = Cursor::new(data);
    let mut decoder = ZlibDecoder::new(reader);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| GitRemoteError::Protocol(format!("zlib decode failed: {}", e)))?;
    let consumed = decoder.into_inner().position() as usize;
    Ok((out, consumed))
}

fn git_object_id(kind: GitObjectKind, data: &[u8]) -> String {
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

fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, GitRemoteError> {
    let mut cursor = 0usize;
    let base_size = read_varint(delta, &mut cursor)?;
    let result_size = read_varint(delta, &mut cursor)?;
    if base_size != base.len() {
        return Err(GitRemoteError::Protocol("delta base size mismatch".into()));
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
            let end = offset + size;
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
    let mut shift = 0usize;
    let mut value = 0usize;
    loop {
        let byte = *data
            .get(*cursor)
            .ok_or_else(|| GitRemoteError::Protocol("truncated varint".into()))?;
        *cursor += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xefcdab89;
    let mut h2: u32 = 0x98badcfe;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xc3d2e1f0;

    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, bytes) in chunk.chunks_exact(4).enumerate().take(16) {
            w[i] = u32::from_be_bytes(bytes.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

pub fn parse_commit(data: &[u8]) -> Result<ParsedCommit, GitRemoteError> {
    let text = std::str::from_utf8(data)
        .map_err(|_| GitRemoteError::Protocol("commit object is not valid UTF-8".into()))?;
    let (headers, message) = text.split_once("\n\n").unwrap_or((text, ""));

    let mut tree = None;
    let mut parents = Vec::new();
    let mut author_name = String::new();
    let mut author_email = String::new();
    let mut author_time = 0i64;

    for line in headers.lines() {
        if let Some(value) = line.strip_prefix("tree ") {
            tree = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("parent ") {
            parents.push(value.to_string());
        } else if let Some(value) = line.strip_prefix("author ") {
            if let Some((prefix, _tz)) = value.rsplit_once(' ') {
                if let Some((identity, timestamp)) = prefix.rsplit_once(' ') {
                    author_time = timestamp.parse().unwrap_or(0);
                    if let Some((name, email)) = identity.rsplit_once(" <") {
                        author_name = name.to_string();
                        author_email = email.trim_end_matches('>').to_string();
                    }
                }
            }
        }
    }

    Ok(ParsedCommit {
        tree: tree.ok_or_else(|| GitRemoteError::Protocol("commit missing tree".into()))?,
        parents,
        author_name,
        author_email,
        author_time,
        message: message.to_string(),
    })
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
    #[error("authentication required")]
    AuthRequired,
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
        &mut seen,
        &mut commit_order,
    )?;

    let mut tree_cache: HashMap<String, B3Hash> = HashMap::new();
    let mut leaf_idx_by_sha: HashMap<String, u64> = HashMap::new();
    let mut leaf_idx_by_hash: HashMap<B3Hash, u64> = HashMap::new();
    let mut commits_imported = 0usize;
    let mut commits_skipped = 0usize;
    let mut blobs_downloaded = 0usize;
    let mut seen_blob_shas = std::collections::HashSet::new();

    for idx in 0..repo.commit_count() {
        if let Ok(Some(leaf)) = repo.get_leaf(idx) {
            leaf_idx_by_hash.insert(leaf.hash(), idx);
        }
    }

    let total_blobs = count_reachable_blobs(&fetch.head_sha, &fetch.objects)?;
    let mp = MultiProgress::new();
    let pb_blobs = mp.add(progress::file_bar(total_blobs as u64, "Importing blobs"));
    let pb_commits = mp.add(progress::file_bar(
        commit_order.len() as u64,
        "Importing commits",
    ));

    for sha in &commit_order {
        if let Some(existing_hash) = mapping.get_blake3(sha) {
            if let Some(idx) = leaf_idx_by_hash.get(&existing_hash).copied() {
                leaf_idx_by_sha.insert(sha.clone(), idx);
            }
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
            &fetch.objects,
            &store,
            &mut mapping,
            &mut tree_cache,
            &mut blobs_downloaded,
            &mut seen_blob_shas,
            &pb_blobs,
        )?;
        let author = if commit.author_name.is_empty() || commit.author_email.is_empty() {
            "unknown <unknown>".to_string()
        } else {
            format!("{} <{}>", commit.author_name, commit.author_email)
        };

        let prev_idx = commit
            .parents
            .first()
            .and_then(|p| leaf_idx_by_sha.get(p).copied())
            .unwrap_or(NO_PARENT);
        let merge_idxs = commit
            .parents
            .iter()
            .skip(1)
            .filter_map(|p| leaf_idx_by_sha.get(p).copied())
            .collect();

        let mut leaf = Leaf::new(
            tree_hash,
            &fetch.branch,
            &author,
            commit.author_time,
            &commit.message,
        );
        leaf.prev_idx = prev_idx;
        leaf.merge_idxs = merge_idxs;
        let result = repo
            .commit_raw(leaf, &fetch.branch)
            .map_err(|e| GitRemoteError::Io(e.to_string()))?;
        mapping.insert(sha, result.hash);
        leaf_idx_by_sha.insert(sha.clone(), result.index);
        leaf_idx_by_hash.insert(result.hash, result.index);
        commits_imported += 1;
        pb_commits.inc(1);
    }

    pb_blobs.finish_with_message(format!("{} blobs imported", blobs_downloaded));
    pb_commits.finish_with_message(format!(
        "{} commits imported, {} skipped",
        commits_imported, commits_skipped
    ));
    mapping
        .save()
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    Ok(crate::sync::ImportResult {
        commits_imported,
        commits_skipped,
        blobs_downloaded,
        timeline: fetch.branch.clone(),
    })
}

fn collect_commit_order(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) -> Result<(), GitRemoteError> {
    if !seen.insert(sha.to_string()) {
        return Ok(());
    }
    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing commit object {}", sha)))?;
    let commit = parse_commit(&object.data)?;
    for parent in &commit.parents {
        collect_commit_order(parent, objects, seen, out)?;
    }
    out.push(sha.to_string());
    Ok(())
}

fn import_tree(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    store: &crate::fsmerkle::FsStore<'_>,
    mapping: &mut crate::remote::HashMapping,
    tree_cache: &mut HashMap<String, B3Hash>,
    blob_count: &mut usize,
    seen_blob_shas: &mut std::collections::HashSet<String>,
    pb_blobs: &indicatif::ProgressBar,
) -> Result<B3Hash, GitRemoteError> {
    use crate::fsmerkle::{Entry, MODE_DIR, MODE_FILE, NodeKind};

    if let Some(hash) = tree_cache.get(sha).copied() {
        return Ok(hash);
    }

    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing tree object {}", sha)))?;
    let entries = parse_tree(&object.data)?;
    let mut ivaldi_entries = Vec::new();

    for entry in entries {
        match entry.mode.as_str() {
            "40000" | "040000" => {
                let hash = import_tree(
                    &entry.sha,
                    objects,
                    store,
                    mapping,
                    tree_cache,
                    blob_count,
                    seen_blob_shas,
                    pb_blobs,
                )?;
                ivaldi_entries.push(Entry {
                    name: entry.name,
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash,
                });
            }
            "100644" | "100755" | "120000" => {
                let hash = if let Some(existing) = mapping.get_blake3(&entry.sha) {
                    if seen_blob_shas.insert(entry.sha.clone()) {
                        pb_blobs.inc(1);
                    }
                    existing
                } else {
                    let blob = objects.get(&entry.sha).ok_or_else(|| {
                        GitRemoteError::Protocol(format!("missing blob object {}", entry.sha))
                    })?;
                    let (hash, _) = store
                        .put_blob(&blob.data)
                        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
                    mapping.insert(&entry.sha, hash);
                    if seen_blob_shas.insert(entry.sha.clone()) {
                        *blob_count += 1;
                        pb_blobs.inc(1);
                    }
                    hash
                };
                ivaldi_entries.push(Entry {
                    name: entry.name,
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash,
                });
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

    let hash = store
        .put_tree(ivaldi_entries)
        .map_err(|e| GitRemoteError::Io(e.to_string()))?;
    tree_cache.insert(sha.to_string(), hash);
    Ok(hash)
}

fn count_reachable_blobs(
    head_sha: &str,
    objects: &HashMap<String, GitObject>,
) -> Result<usize, GitRemoteError> {
    let mut seen_commits = std::collections::HashSet::new();
    let mut seen_trees = std::collections::HashSet::new();
    let mut seen_blobs = std::collections::HashSet::new();
    count_reachable_blobs_from_commit(
        head_sha,
        objects,
        &mut seen_commits,
        &mut seen_trees,
        &mut seen_blobs,
    )?;
    Ok(seen_blobs.len())
}

fn count_reachable_blobs_from_commit(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    seen_commits: &mut std::collections::HashSet<String>,
    seen_trees: &mut std::collections::HashSet<String>,
    seen_blobs: &mut std::collections::HashSet<String>,
) -> Result<(), GitRemoteError> {
    if !seen_commits.insert(sha.to_string()) {
        return Ok(());
    }
    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing commit object {}", sha)))?;
    let commit = parse_commit(&object.data)?;
    count_reachable_blobs_from_tree(&commit.tree, objects, seen_trees, seen_blobs)?;
    for parent in &commit.parents {
        count_reachable_blobs_from_commit(parent, objects, seen_commits, seen_trees, seen_blobs)?;
    }
    Ok(())
}

fn count_reachable_blobs_from_tree(
    sha: &str,
    objects: &HashMap<String, GitObject>,
    seen_trees: &mut std::collections::HashSet<String>,
    seen_blobs: &mut std::collections::HashSet<String>,
) -> Result<(), GitRemoteError> {
    if !seen_trees.insert(sha.to_string()) {
        return Ok(());
    }
    let object = objects
        .get(sha)
        .ok_or_else(|| GitRemoteError::Protocol(format!("missing tree object {}", sha)))?;
    for entry in parse_tree(&object.data)? {
        match entry.mode.as_str() {
            "40000" | "040000" => {
                count_reachable_blobs_from_tree(&entry.sha, objects, seen_trees, seen_blobs)?;
            }
            "100644" | "100755" | "120000" => {
                seen_blobs.insert(entry.sha);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkt_line_roundtrip() {
        let lines = parse_pkt_lines(&[pkt_line("hello\n"), b"0000".to_vec()].concat()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].as_deref(), Some(b"hello\n".as_slice()));
        assert!(lines[1].is_none());
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
}
