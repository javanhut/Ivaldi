# Ivaldi Sync Command

The `ivaldi sync` command provides git-independent synchronization with GitHub repositories using REST API integration.

## Overview

Ivaldi's sync command operates completely independently of git, using GitHub's REST API for fast, reliable synchronization. This approach provides:
- **10x faster uploads** using batch operations
- **No git dependencies** or configuration issues
- **Smart ignore file support** with .ivaldiignore
- **Atomic operations** for data consistency
- **Token-based authentication** for security

## Usage

### Basic Sync (Bidirectional)
```bash
ivaldi sync origin
```

### Push-only Sync
```bash
ivaldi sync origin --push
```

### Pull-only Sync
```bash
ivaldi sync origin --pull
```

## How It Works

### 1. Native Protocol
Ivaldi uses its own synchronization protocol:
```bash
→ Fetching remote changes using native protocol...
→ Analyzing local vs remote state...
→ Determining sync strategy...
```

### 2. GitHub API Integration
For GitHub repositories, uses REST API directly:
```bash
→ Using GitHub API for repository: user/repo
→ Authenticating with stored credentials...
→ ✅ Connected to GitHub API
```

### 3. Batch Operations
Uploads multiple files atomically:
```bash
→ Uploading 15 files in parallel...
→ Creating GitHub tree with 15 items...
→ Creating single commit with all changes...
→ ✅ Successfully uploaded 15 files in single commit: a1b2c3d4
```

## Command Options

| Option | Description | Example |
|--------|-------------|---------|
| `--push` | Push-only sync | `ivaldi sync origin --push` |
| `--pull` | Pull-only sync | `ivaldi sync origin --pull` |
| `--force` | Force sync ignoring conflicts | `ivaldi sync origin --force` |
| `--dry-run` | Preview sync without executing | `ivaldi sync origin --dry-run` |

## Examples

### Example 1: First-time Repository Sync
```bash
# Configure credentials
ivaldi config
→ Enter GitHub username: myusername
→ Enter GitHub token: ghp_abc123...

# Add repository portal
ivaldi portal add origin https://github.com/myusername/myproject.git

# Initial sync
ivaldi sync origin
→ ✅ Repository is empty, uploading all files...
→ ✅ Uploading 23 files in parallel...
→ ✅ Successfully uploaded 23 files in single commit: d4e5f6g7
```

### Example 2: Ongoing Development Sync
```bash
# Make changes
echo "new feature" > feature.txt
ivaldi gather all
ivaldi seal "Add new feature"

# Sync to GitHub
ivaldi sync origin
→ ✅ Uploading 1 new file...
→ ✅ Successfully synced changes to GitHub
```

### Example 3: Sync with Ignore Files
```bash
# Create ignore file
cat > .ivaldiignore << EOF
build/
*.log
*.tmp
node_modules/
EOF

# Sync respects ignore patterns
ivaldi sync origin
→ ✅ Processing 50 files...
→ ✅ Skipped 23 ignored files
→ ✅ Uploaded 27 source files
```

## Ignore File Support

### .ivaldiignore Format
Uses glob patterns similar to .gitignore:
```bash
# Comments start with #
build/          # Ignore build directory
*.log           # Ignore all log files
*.tmp           # Ignore temporary files
**/temp/        # Ignore temp directories anywhere
node_modules/   # Ignore Node.js dependencies
```

### Built-in Ignore Patterns
Automatically ignored (cannot be overridden):
- `.ivaldi/` - Ivaldi metadata
- `.git/` - Git metadata
- `build/` - Build artifacts
- `*.tmp`, `*.temp` - Temporary files
- `*.log`, `*.bak` - Log and backup files
- `.DS_Store` - macOS system files

### Pattern Matching
Supports standard glob patterns:
```bash
*.txt           # All .txt files
src/**/*.go     # All .go files in src/ subdirectories
test_*          # Files starting with test_
!important.log  # Exception: don't ignore this file
```

## Performance Features

### Batch Upload
Groups files into single GitHub API call:
```bash
→ Analyzing 50 files for upload...
→ Creating tree with 50 items...
→ Creating single commit...
→ Updating branch reference...
→ ✅ Uploaded 50 files in 2.3 seconds
```

### Parallel Processing
Processes multiple files simultaneously:
```bash
→ Processing files in parallel...
→ [██████████] 50/50 files processed
→ Upload completed in 1.8 seconds
```

### Incremental Sync
Only uploads changed files:
```bash
→ Comparing local vs remote state...
→ Found 3 modified files, 2 new files
→ Uploading 5 changed files...
→ ✅ Sync completed (45 files unchanged)
```

## Conflict Resolution

### Divergent Branches
Uses Ivaldi's native fuse system:
```bash
→ Detected divergent branches
→ Creating temporary timeline for remote changes...
→ Using automatic fuse strategy...
→ ✅ Successfully merged divergent changes
```

### Merge Strategies
- **Fast-forward**: When local is behind remote
- **Automatic**: Smart merge of non-conflicting changes
- **Manual**: Interactive resolution for conflicts

### Force Sync
Override conflicts when needed:
```bash
ivaldi sync origin --force
→ ⚠️  Force sync will override remote changes
→ Continue? (y/N): y
→ ✅ Force sync completed
```

## GitHub Integration Details

### Authentication
Uses Personal Access Tokens:
```bash
→ Authenticating with GitHub...
→ Token: ghp_****...****
→ User: myusername
→ ✅ Authentication successful
```

### API Endpoints Used
- `GET /repos/{owner}/{repo}/git/refs/heads/{branch}` - Get current commit
- `POST /repos/{owner}/{repo}/git/trees` - Create file tree
- `POST /repos/{owner}/{repo}/git/commits` - Create commit
- `PATCH /repos/{owner}/{repo}/git/refs/heads/{branch}` - Update branch

### Rate Limiting
Handles GitHub API rate limits:
```bash
→ API Rate Limit: 4,987/5,000 remaining
→ Batch upload optimizes API usage
```

## Troubleshooting

### Authentication Issues
```bash
→ ❌ GitHub authentication failed
```
**Solution**: Update credentials with `ivaldi config`

### Network Connectivity
```bash
→ ❌ Failed to connect to GitHub API
```
**Solution**: Check internet connection and GitHub status

### Permission Errors
```bash
→ ❌ Insufficient permissions to push to repository
```
**Solution**: Ensure GitHub token has `repo` scope

### Large File Issues
```bash
→ ❌ File too large for GitHub API (>100MB)
```
**Solution**: Use Git LFS or split large files

## Advanced Usage

### Custom Sync Workflows
```bash
# Development workflow
ivaldi gather all
ivaldi seal "$(git log -1 --pretty=%B)"  # Use git message
ivaldi sync origin --push

# Release workflow  
ivaldi squash --all "Release v1.2.3" --force-push
ivaldi sync origin --push
```

### Integration with CI/CD
```bash
#!/bin/bash
# ci-sync.sh
export GITHUB_TOKEN=$CI_GITHUB_TOKEN
echo "$CI_USERNAME" | ivaldi config --username
echo "$GITHUB_TOKEN" | ivaldi config --token
ivaldi sync origin --push
```

### Backup and Restore
```bash
# Backup before major sync
ivaldi backup create pre-sync-$(date +%Y%m%d)
ivaldi sync origin --force

# Restore if needed
ivaldi backup restore pre-sync-20240817
```

## Comparison with Git

| Feature | Git | Ivaldi Sync |
|---------|-----|-------------|
| Speed | Individual pushes | Batch uploads (10x faster) |
| Dependencies | Git binary required | No dependencies |
| Configuration | Complex git config | Simple token auth |
| Conflicts | Manual resolution | Automatic fuse system |
| Large repos | Slow clone/push | Fast incremental sync |
| Error handling | Cryptic messages | Clear, actionable errors |

## Best Practices

### Regular Sync
Sync frequently to avoid large uploads:
```bash
# After each seal
ivaldi seal "Fix bug in validator"
ivaldi sync origin --push
```

### Use Ignore Files
Keep repositories clean:
```bash
# Comprehensive .ivaldiignore
build/
dist/
*.log
*.tmp
.env
node_modules/
__pycache__/
```

### Batch Related Changes
Group related work into single commits:
```bash
# Multiple files for one feature
ivaldi gather src/auth/
ivaldi gather tests/auth/
ivaldi seal "Implement JWT authentication"
ivaldi sync origin --push
```

## Related Commands

- [`ivaldi config`](CONFIG_COMMAND.md) - Configure GitHub credentials
- [`ivaldi portal`](PORTAL_COMMAND.md) - Manage remote portals
- [`ivaldi squash`](SQUASH_COMMAND.md) - Clean up commit history
- [`ivaldi timeline`](TIMELINE_COMMAND.md) - Manage branches