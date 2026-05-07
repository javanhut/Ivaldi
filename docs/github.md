# GitHub Module (`github.rs`)

GitHub API client for Ivaldi VCS.

## Overview

Synchronous HTTP client using `ureq` for all GitHub API interactions. Handles
authentication automatically via the `auth` module's credential resolution.
Auth is **optional** — read-only operations on public repositories work
anonymously (see [Public repo access](#public-repo-access) below).

## Operations

| Method | Purpose |
|--------|---------|
| `get_repo` | Repository metadata |
| `list_branches` | All branches (paginated) |
| `get_tree` | Recursive tree listing |
| `list_commits` | Commit history (paginated, depth-limited) |
| `download_file` | Raw file content via raw.githubusercontent.com |
| `create_blob` | Upload file content (base64 encoded) |
| `create_tree` | Create Git tree object |
| `create_commit` | Create Git commit object (preserves author + committer) |
| `update_ref` | Update branch pointer (with force option) |
| `create_ref` | Create new branch |
| `request_device_code` | Start OAuth device flow |
| `poll_for_token` | Poll for OAuth token completion |

## Authentication

```bash
# Option 1: OAuth (recommended)
ivaldi auth login

# Option 2: Environment variable
export GITHUB_TOKEN=ghp_...

# Option 3: GitHub CLI (automatic fallback)
gh auth login
```

## Commit-identity fidelity

`create_commit` accepts optional `author: &CommitIdentity` and
`committer: &CommitIdentity` parameters; `sync::upload` always supplies
both. The upload path:

1. Walks back from the local timeline head along `prev_idx`, collecting
   every leaf whose BLAKE3 isn't already in `HashMapping`
   (`collect_unpushed_leaves`).
2. Replays each leaf as its own GitHub commit so multi-commit local
   history isn't squashed into one on the server.
3. For each leaf, fills `author` from `leaf.author + leaf.time_unix +
   meta["git.author_tz"]` and `committer` from the `git.committer*`
   meta keys (set during `download` import) — falling back to the
   author when no meta is present (matches git's default behavior).

Net effect: `download` → `upload` round-trips a real GitHub repo without
silently rewriting commit identity. See
[`docs/git_export.md`](git_export.md) for the analogous (canonical bytes)
translation used by SSH push.

## Public repo access

The Git smart-HTTP transport (`src/git_remote.rs`) works anonymously for public
repositories. `download`, `scout`, and `harvest` all tolerate a missing
token — only write operations (`upload`, `sync`) require authentication.

### Stale-token fallback

If a stored token (from `~/.config/ivaldi/auth.json`, `GITHUB_TOKEN`, `.netrc`,
or `gh` CLI) is expired or revoked, GitHub returns `401 Bad credentials` even
for public repos. Ivaldi detects this and automatically retries the request
anonymously, printing a one-line notice:

```
$ ivaldi download rust-lang/book
stored token rejected — falling back to anonymous access
Downloading rust-lang/book...
```

The retry fires only for `401` and for `403` responses that are **not**
rate-limit errors (detected via `X-RateLimit-Remaining: 0`). If the anonymous
retry also fails, the original error is surfaced.

### Rate limiting

Anonymous GitHub requests are capped at 60/hour. When the Git transport sees a
`403` with `X-RateLimit-Remaining: 0`, it returns
`GitRemoteError::RateLimited { reset_at }` with a clear message:

```
Error: GitHub rate limit reached (60/hr unauthenticated).
Run 'ivaldi auth login' to raise the limit to 5000/hr.
```

## OAuth Device Flow

```
1. Client requests device code → GitHub returns user_code + verification_uri
2. User visits verification_uri, enters user_code
3. Client polls for access_token every N seconds
4. Token stored in ~/.config/ivaldi/auth.json
```
