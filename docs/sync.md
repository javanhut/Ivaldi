# Sync Module (`sync/`)

Download, upload, scout, and harvest operations for Ivaldi VCS. The
transport-agnostic orchestration layer; the actual wire work lives in
`git_remote` (HTTPS Smart), `ssh_transport` (SSH), or `p2p` (Ivaldi
peer-to-peer).

## Module layout

| File | Contents |
|------|----------|
| `sync/mod.rs` | `SyncError`, `RemoteFetcher`, download/scout/harvest, shared helpers, re-exports |
| `sync/upload.rs` | `upload` and its per-step helpers |
| `sync/import.rs` | `import_full_history(_into)` and its phase helpers |
| `sync/timeline_sync.rs` | `sync_timeline` (fast-forward and diverged-merge paths) |

**Diverged sync and collisions.** When both sides have new seals, sync fuses
them with the same engine as `ivaldi fuse`, so edits to the same file in
different places merge by themselves. What `sync_timeline` does about true
collisions is the caller's choice, passed as `Collisions`:

| `Collisions::` | Used by | Effect |
|---|---|---|
| `Ask(resolver)` | CLI on a terminal, or with `--prefer` | Settled in memory, then fused in the same call. Cancelling integrates nothing |
| `Refuse` | CLI with no terminal and no `--prefer` | `SyncError::Collisions(files)`; nothing integrated |
| `Park` | TUI (its sync thread cannot ask) | `SyncError::Parked { timeline, files }`; nothing integrated, but the fetched seals stay on scratch timeline `timeline` for the Fuse tab to fuse |
| `Markers` | CLI `--markers` | Conflict markers written, merge left open for `fuse --continue` |

Except for `Markers`, a sync that does not integrate leaves no trace: no
scratch timeline (bar `Park`'s), no journal, no open merge, no `oops` entry.

All public paths are re-exported from `mod.rs`, so callers still use
`crate::sync::upload(...)` etc. `SyncError` carries typed `#[from]`
variants (`Repo`, `Forge`, `GitRemote`, `Cas`, `FsMerkle`, `Remote`,
`Store`, `Io`, `GitHub`); `Other(String)` is reserved for genuinely
ad-hoc messages.

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
2. Fetches the packfile via Git smart-HTTP (`.../info/refs` + `git-upload-pack`),
   spooling it to a file in the target directory and indexing it as it arrives
3. Loads commits, trees, and tags from the spool
4. Streams blobs from the spool into the CAS with BLAKE3 hashing, one delta
   tree at a time, then imports trees level by level in parallel
5. Creates SHA1↔BLAKE3 mappings
6. Writes files to working directory
7. Creates the initial Ivaldi timeline

Memory stays bounded regardless of repository size: the pack is never held
in RAM, and blobs are rebuilt and written a few per thread
(`src/git_unpack.rs`). The spool sits beside the repository rather than in
the system temp directory, which is often RAM-backed; it is unlinked as soon
as it is created, so it cannot outlive the process. `harvest` spools under
`.ivaldi/`.

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

Only what is missing is transferred. Harvest offers the server the git id of
every local seal that has one (`have` lines — timeline heads first, then
newest to oldest, capped at `MAX_HAVES`), and the server replies with a pack
that leaves out everything reachable from them:

- A branch whose tip is already sealed locally costs no pack request at all.
- Boundaries of a `--depth` clone are sent as `shallow` lines, so the server
  does not assume the history behind them is present.
- A new annotated tag on a commit already held is requested explicitly; the
  server only volunteers tags for commits it is sending.
- The import resolves objects the pack omits (parents, unchanged trees and
  blobs) from the local store, and only trusts a hash-map entry whose object
  is really in the CAS. If something the haves promised turns out to be gone
  (pruned by `gc`, or uploaded by a build that recorded no mapping), the
  import stops before sealing anything and harvest retries once with a full,
  un-negotiated fetch.

## Force Push Safety
```bash
ivaldi upload --force
# Prompts: "Type 'force push' to confirm:"
```
