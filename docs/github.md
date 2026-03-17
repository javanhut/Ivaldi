# GitHub Module (`github.rs`)

GitHub API client for Ivaldi VCS.

## Overview

Synchronous HTTP client using `ureq` for all GitHub API interactions. Handles authentication automatically via the `auth` module's credential resolution.

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
| `create_commit` | Create Git commit object |
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

## OAuth Device Flow

```
1. Client requests device code → GitHub returns user_code + verification_uri
2. User visits verification_uri, enters user_code
3. Client polls for access_token every N seconds
4. Token stored in ~/.config/ivaldi/auth.json
```
