# Sync Module (`sync.rs`)

Download, upload, scout, and harvest operations for Ivaldi VCS.

## Overview

Bridges Ivaldi's BLAKE3-based internal storage with GitHub's SHA1-based Git
objects. SHA1 is used ONLY for API communication — never in the internal
pipeline.

Authentication is **optional** for read operations (`download`, `scout`,
`harvest`) — public repositories work without a token. See
[github.md](github.md#public-repo-access) for stale-token fallback and
rate-limit handling details.

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
