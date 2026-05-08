# Sync Module (`sync.rs`)

Download, upload, scout, and harvest operations for Ivaldi VCS. The
transport-agnostic orchestration layer; the actual wire work lives in
`git_remote` (HTTPS Smart), `ssh_transport` (SSH), or `p2p` (Ivaldi
peer-to-peer).

## Overview

Bridges Ivaldi's BLAKE3-based internal storage with the various wire
formats: GitHub/GitLab SHA-1 objects (HTTPS + SSH) and Ivaldi-native
objects (`ivaldi://`). Internal hashing is always BLAKE3; SHA-1 only
appears at HTTPS / SSH boundaries.

Authentication is **optional** for read operations on public repos
(`download`, `scout`, `harvest` over HTTPS). SSH uses your system SSH
agent. P2P uses each user's ed25519 identity. See [auth.md](auth.md),
[ssh.md](ssh.md), and [p2p.md](p2p.md) for the per-transport details.

## Transport dispatch — `RemoteFetcher`

`scout`, `scout_with_status`, and `harvest` all take a `&Portal` and
internally build a `RemoteFetcher` based on `portal.transport()`:

```rust
pub enum RemoteFetcher {
    Https { token: Option<String> },
    Ssh   { target: SshTarget },
}

impl RemoteFetcher {
    fn list_branches(&self, owner, repo)        -> Result<Vec<String>, ...>
    fn list_branch_refs(&self, owner, repo)     -> Result<Vec<RemoteBranch>, ...>
    fn fetch_repo(&self, owner, repo, branch)   -> Result<FetchResult, ...>
}
```

For `download` and `upload`, dispatch happens at the CLI layer
(`cmd_download` / `cmd_upload` in `src/cli/commands.rs`):

| Portal transport | `download` calls | `upload` calls |
|---|---|---|
| `Https` | `sync::download` | `sync::upload` (REST API) |
| `Ssh(target)` | `sync::download_ssh` | `SshClient::push_repo` (git pack) |
| `Peer(url)` | `p2p::fetch_into` | `p2p::push_to` |

## Commands

### Download (Clone)
```bash
ivaldi download <repo> [directory]
```

`<repo>` accepts any format supported by [`parse_repo_spec`](portal.md):

```bash
ivaldi download owner/repo
ivaldi download https://github.com/owner/repo.git
ivaldi download git@github.com:owner/repo.git
ivaldi download github:owner/repo
ivaldi download https://github.com/owner/repo/tree/feature-branch  # auto-selects branch
```

Flow:
1. Gets repo info and default branch (or the URL-encoded branch hint)
2. Fetches the packfile via Git smart-HTTP (`.../info/refs` + `git-upload-pack`)
3. Parses commits, trees, and blobs
4. Stores in CAS with BLAKE3 hashing
5. Creates SHA1↔BLAKE3 mappings
6. Writes files to working directory
7. Creates the initial Ivaldi timeline

Public repos require no authentication; a stale token triggers automatic
anonymous retry.

### Upload (Push)
```bash
ivaldi upload [branch] [--force]
```
1. Reads head commit tree
2. Creates blobs on GitHub (base64 upload)
3. Creates Git tree from blob SHAs
4. Creates Git commit pointing to tree
5. Updates branch reference (or creates new branch)

**Requires authentication** (`ivaldi auth login` or `GITHUB_TOKEN`).

### Scout
```bash
ivaldi scout
```
Lists remote branches — metadata only, no data downloaded. Works on public
repos without auth.

### Harvest
```bash
ivaldi harvest branch-a branch-b
```
Downloads specific branches into CAS and creates local timelines. Works on
public repos without auth.

## Force Push Safety
```bash
ivaldi upload --force
# Prompts: "Type 'force push' to confirm:"
```
