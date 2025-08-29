# Ivaldi Squash Command

The `ivaldi squash` command consolidates multiple commits into a single, clean commit with optional force push to rewrite GitHub history.

## Overview

Squashing helps maintain clean commit history by combining multiple related commits into one meaningful commit. This is particularly useful for:
- Cleaning up development commits before merging
- Consolidating feature work into logical units
- Removing "work in progress" commits
- Creating clean, professional commit history

## Usage

### Basic Squash
```bash
ivaldi squash --all "Clean commit message"
```

### Squash with Force Push
```bash
ivaldi squash --all "Clean commit message" --force-push
```

### Squash Specific Number of Commits
```bash
ivaldi squash --count 3 "Consolidated last 3 commits"
```

## Command Options

| Option | Description | Example |
|--------|-------------|---------|
| `--all` | Squash all commits in timeline | `ivaldi squash --all "message"` |
| `--count N` | Squash last N commits | `ivaldi squash --count 5 "message"` |
| `--force-push` | Force push to origin after squash | `ivaldi squash --all "msg" --force-push` |
| `--dry-run` | Preview squash without executing | `ivaldi squash --all "msg" --dry-run` |

## Examples

### Example 1: Clean Up Feature Branch
```bash
# You have multiple commits:
# - "wip: initial auth work"
# - "fix: auth bug"
# - "wip: more auth changes"  
# - "feat: complete auth system"

# Squash all commits into one
ivaldi squash --all "feat: implement user authentication system"
→ CHECKMARK Successfully created clean commit: d6de547d
→ Timeline now has 1 commit instead of 4

# Force push to update GitHub
ivaldi squash --all "feat: implement user authentication system" --force-push
→ CHECKMARK Successfully created clean commit: d6de547d
→ CHECKMARK Successfully squashed commits and updated origin!
```

### Example 2: Squash Recent Commits Only
```bash
# Squash only the last 3 commits
ivaldi squash --count 3 "fix: resolve validation issues"
→ CHECKMARK Squashed 3 commits into: bright-river-42
```

### Example 3: Preview Squash Operation
```bash
# See what would happen without executing
ivaldi squash --all "feat: new feature" --dry-run
→ Would squash 5 commits:
→   - loud-wolf-888: New Changes for sync
→   - proud-flame-678: added changes to sync
→   - sharp-shield-826: feat: fixed the gather file checking
→   - fresh-moon-359: made improvements to the cli
→   - golden-lion-500: removed unused files
→ Into single commit: "feat: new feature"
```

## How It Works

### 1. Commit Analysis
Ivaldi analyzes the commits to be squashed:
```bash
→ Analyzing 4 commits for squashing...
→ Found commits from loud-wolf-888 to current HEAD
→ Total changes: 23 files modified, 1,247 lines changed
```

### 2. History Rewriting
Creates a new commit with combined changes:
```bash
→ Creating new commit with combined changes...
→ New commit SHA: d6de547dac1b2f8a9e3c5d7f1a4b6c8e0f2g4h6i
→ Memorable name: silver-forest-985
```

### 3. Force Push (if requested)
Updates GitHub with rewritten history:
```bash
→ Force pushing to origin...
→ Successfully updated refs/heads/main
→ GitHub history updated successfully
```

## GitHub Integration

### Rewriting Remote History
When using `--force-push`, Ivaldi:
1. Creates the squashed commit locally
2. Force pushes to GitHub using REST API
3. Updates the remote branch reference
4. Preserves all file changes while cleaning commit history

### Before Squash (GitHub)
```
a1b2c3d - "wip: auth work"
b2c3d4e - "fix: auth bug"  
c3d4e5f - "more auth changes"
d4e5f6g - "complete auth"
```

### After Squash (GitHub)
```
d6de547 - "feat: implement user authentication system"
```

## Safety Features

### Backup Creation
Before squashing, Ivaldi automatically creates a backup:
```bash
→ Creating backup of current state...
→ Backup created: .ivaldi/backups/pre-squash-20240817-143022
```

### Validation
Ensures operation is safe:
```bash
→ Validating squash operation...
→ CHECKMARK No uncommitted changes detected
→ CHECKMARK All commits are local (safe to squash)
→ CHECKMARK GitHub credentials validated
```

### Confirmation for Force Push
```bash
→ This will rewrite GitHub history. Continue? (y/N): y
→ Proceeding with force push...
```

## Advanced Usage

### Squash with Custom Timeline
```bash
# Switch to feature timeline and squash
ivaldi timeline switch feature-auth
ivaldi squash --all "feat: complete authentication system"
ivaldi timeline switch main
ivaldi fuse feature-auth --delete-source
```

### Conditional Squash
```bash
# Only squash if more than 3 commits
COMMIT_COUNT=$(ivaldi log --count)
if [ $COMMIT_COUNT -gt 3 ]; then
    ivaldi squash --all "Consolidated commits" --force-push
fi
```

## Best Practices

### When to Squash
- Before merging feature branches
- After completing a logical unit of work
- When cleaning up experimental commits
- Before creating releases

### When NOT to Squash
- When commits represent distinct logical changes
- When preserving detailed development history is important
- When multiple developers contributed to the commits
- For commits already shared with team (unless coordinated)

### Good Squash Messages
```bash
# Good: Describes what was accomplished
"feat: implement user authentication with JWT tokens"
"fix: resolve memory leak in file processing"
"refactor: simplify database connection handling"

# Bad: Vague or unhelpful
"fixes"
"updates"
"changes"
```

## Troubleshooting

### Uncommitted Changes
```bash
→ ❌ Cannot squash with uncommitted changes
```
**Solution**: Commit or discard changes first:
```bash
ivaldi gather all && ivaldi seal "Final changes"
# or
ivaldi discard all
```

### No Commits to Squash
```bash
→ ❌ No commits available to squash
```
**Solution**: Ensure you have multiple commits in the timeline.

### Force Push Failed
```bash
→ ❌ Force push failed: insufficient permissions
```
**Solution**: Check GitHub token permissions or configure with `ivaldi config`.

## Integration with Other Commands

### With Timeline Management
```bash
# Create feature, work, squash, merge
ivaldi timeline create feature-xyz
# ... make commits ...
ivaldi squash --all "feat: implement XYZ feature"
ivaldi timeline switch main
ivaldi fuse feature-xyz --delete-source
```

### With Portal Sync
```bash
# Squash and immediately sync to GitHub
ivaldi squash --all "feat: complete feature" --force-push
# GitHub now has clean, single commit
```

## Related Commands

- [`ivaldi sync`](SYNC_COMMAND.md) - Synchronize with GitHub
- [`ivaldi timeline`](TIMELINE_COMMAND.md) - Manage timelines
- [`ivaldi fuse`](FUSE_COMMAND.md) - Merge timelines