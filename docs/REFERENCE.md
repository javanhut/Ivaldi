# Ivaldi VCS Command Reference

> **Complete reference guide for all Ivaldi commands and features.**

## Table of Contents

1. [Repository Management](#repository-management)
2. [File Operations](#file-operations)
3. [Timeline Management](#timeline-management)
4. [Portal Operations](#portal-operations)
5. [Information Commands](#information-commands)
6. [Advanced Operations](#advanced-operations)
7. [Utility Commands](#utility-commands)
8. [Natural Language Reference](#natural-language-reference)

---

## Repository Management

### `ivaldi forge [path]`
**Create a new Ivaldi repository**

```bash
ivaldi forge                    # Create repository in current directory
ivaldi forge my-project        # Create repository in new directory
```

**Revolutionary Features:**
- Initializes all Ivaldi revolutionary features
- Sets up natural language reference system
- Configures automatic work preservation
- Creates memorable name generator

### `ivaldi download <url> [destination]`
**Download repository from URL**

```bash
ivaldi download https://github.com/user/repo.git
ivaldi download https://github.com/user/repo.git my-project
```

**Features:**
- Auto-detects destination from URL
- Sets up all Ivaldi features automatically
- Imports Git history with memorable names
- Configures origin portal

### `ivaldi status`
**Show workspace status**

```bash
ivaldi status
```

**Shows:**
- Current timeline and position
- Modified files
- Files on anvil (staged)
- Current memorable name and iteration

---

## File Operations

### `ivaldi gather <patterns...>`
**Stage files for sealing (like git add)**

```bash
ivaldi gather file.txt                # Gather specific file
ivaldi gather src/ tests/             # Gather directories
ivaldi gather all                     # Gather all changes
ivaldi gather *.go *.md              # Gather by pattern
```

**Features:**
- Respects .ivaldiignore patterns
- Smart pattern matching
- Visual feedback for gathered files

### `ivaldi exclude <patterns...>`
**Exclude files from tracking**

```bash
ivaldi exclude build/                 # Exclude directory
ivaldi exclude *.log *.tmp           # Exclude by pattern
ivaldi exclude secrets.txt config.env # Exclude specific files
```

**Features:**
- Updates .ivaldiignore automatically
- Removes files from staging immediately
- Supports all pattern types
- Provides clear feedback

### `ivaldi remove <files...> [flags]`
**Remove files from repository**

```bash
ivaldi remove old-file.txt                    # Remove file
ivaldi remove --from-remote secrets.txt       # Remove only from remote
ivaldi remove --exclude logs/ temp/           # Remove and exclude
ivaldi remove *.cache --from-remote --exclude # Combined options

# Complete workflow example:
ivaldi remove --from-remote CLAUDE.md         # Remove file from remote
ivaldi seal "docs: removed unused file"       # Commit the removal
ivaldi upload                                 # Upload changes
```

**Flags:**
- `--from-remote` - Remove only from remote repository (keep local)
- `--exclude` - Also add to .ivaldiignore to prevent re-tracking

**Features:**
- Flexible removal options
- Automatic staging of removals
- Optional exclusion from future tracking

### `ivaldi seal [message]`
**Create commit with memorable name**

```bash
ivaldi seal                           # AI-generated message
ivaldi seal "Add new feature"         # Custom message
```

**Features:**
- Generates memorable name (e.g., bright-river-42)
- AI-powered semantic commit messages
- Assigns iteration numbers
- Preserves complete workspace state

---

## Timeline Management

### `ivaldi timeline <subcommand>`
**Manage development timelines (branches)**

#### `ivaldi timeline create <name>`
```bash
ivaldi timeline create feature
ivaldi timeline create auth-system
```

#### `ivaldi timeline switch <name>`
```bash
ivaldi timeline switch main
ivaldi timeline switch feature
```
**Revolutionary Auto-Shelving:**
- **Automatically preserves ALL uncommitted work** when switching
- **Automatically restores work** when returning to a timeline
- **Preserves both modified files AND gathered files** on anvil
- **Zero work loss guarantee** - mathematically impossible to lose work
- **No manual shelving required** - completely automatic
- **Smart cleanup** - removes auto-shelves when work is restored

**Process:**
1. Detects uncommitted changes and staged files
2. Creates auto-shelf with descriptive name
3. Switches to target timeline
4. Restores any previous auto-shelf for target timeline
5. Provides clear feedback about what was preserved/restored

#### `ivaldi timeline list`
```bash
ivaldi timeline list
```

### `ivaldi fuse <timeline> [flags]`
**Merge timelines with multiple strategies**

```bash
ivaldi fuse feature                          # Auto-detect strategy
ivaldi fuse feature --strategy=squash        # Squash all commits
ivaldi fuse feature --strategy=fast-forward  # Fast-forward if possible
ivaldi fuse feature --delete-source          # Delete timeline after merge
ivaldi fuse feature --dry-run                # Preview changes only
```

**Strategies:**
- `auto` - Automatically choose best strategy
- `fast-forward` - Fast-forward if possible
- `squash` - Combine all commits into one
- `manual` - Manual merge resolution

---

## Portal Operations

### `ivaldi upload [branch] [portal]`
**Upload current branch to portal (shorthand)**

```bash
ivaldi upload                         # Upload current branch to origin
ivaldi upload main                    # Upload main branch to origin
ivaldi upload main upstream           # Upload main branch to upstream portal
```

**Features:**
- Defaults to current branch if not specified
- Defaults to origin portal if not specified
- Automatic upstream tracking
- Most commonly used command for uploading

### `ivaldi sync --with <branch> [portal]`
**Sync with remote branch automatically**

```bash
ivaldi sync --with main               # Sync with origin/main
ivaldi sync --with main upstream      # Sync with upstream/main
ivaldi sync --with develop            # Sync with origin/develop
```

**Process:**
1. Pulls latest changes from remote branch
2. Auto-seals any uncommitted local changes
3. Fuses remote changes into current timeline
4. Preserves your work with automatic conflict resolution

**Features:**
- Automatic pull + fuse in one command
- Preserves uncommitted work automatically
- Smart conflict resolution
- Keeps you up-to-date with remote changes

### `ivaldi portal <subcommand>`
**Manage remote connections**

#### `ivaldi portal add <name> <url>`
```bash
ivaldi portal add origin https://github.com/user/repo.git
ivaldi portal add upstream https://github.com/upstream/repo.git
```

#### `ivaldi portal list`
```bash
ivaldi portal list
```

#### `ivaldi portal new <branch> [flags]`
**Create new branch with optional migration**

```bash
ivaldi portal new main --migrate master     # Migrate from master to main
ivaldi portal new feature                   # Create new branch
```

#### `ivaldi portal upload <branch> [portal]`
**Upload branch with automatic upstream**

```bash
ivaldi portal upload main                   # Upload to origin
ivaldi portal upload main upstream          # Upload to specific portal
```

**Features:**
- Automatic upstream tracking
- Exports Ivaldi state to Git
- Clean visual feedback

#### `ivaldi portal rename <old> --with <new> [portal]`
**Rename branch on remote**

```bash
ivaldi portal rename master --with main
ivaldi portal rename old-feature --with new-feature upstream
```

#### `ivaldi portal push/pull [portal]`
**Traditional push/pull operations**

```bash
ivaldi portal push origin
ivaldi portal pull origin
```

---

## Information Commands

### `ivaldi whereami`
**Show current location**

```bash
ivaldi whereami
```

**Shows:**
- Current branch name
- Current timeline
- Current position with memorable name
- Upstream tracking status

### `ivaldi what-changed [reference]`
**Show file changes and diffs**

```bash
ivaldi what-changed                    # Changes since last seal
ivaldi what-changed bright-river-42    # Changes since specific seal
ivaldi what-changed #5                 # Changes since iteration 5
ivaldi what-changed "yesterday"        # Changes since yesterday
```

**Features:**
- Colored diff output
- Staged vs modified separation
- Context-aware summaries
- Natural language reference support

### `ivaldi log [flags]`
**View history with memorable names**

```bash
ivaldi log
ivaldi log --limit 10
```

### `ivaldi search <term>`
**Search commits**

```bash
ivaldi search "authentication"
ivaldi search "bug fix"
```

---

## Advanced Operations

### `ivaldi jump <reference>`
**Navigate using natural language**

```bash
# Memorable names
ivaldi jump to bright-river-42
ivaldi jump to swift-mountain-156

# Iteration numbers  
ivaldi jump to #5
ivaldi jump to main#15

# Temporal references
ivaldi jump to "yesterday"
ivaldi jump to "2 hours ago"
ivaldi jump to "last Friday"

# Author references
ivaldi jump to "Sarah's last commit"
ivaldi jump to "my morning changes"

# Content references
ivaldi jump to "where auth was added"
ivaldi jump to "the commit about tests"
```

### `ivaldi reshape <count> [flags]`
**Modify history with accountability**

```bash
ivaldi reshape 3 --justification "Combine related commits"
```

**Features:**
- Mandatory justification for changes
- Complete audit trail
- Protected commit support

### `ivaldi pluck <reference>`
**Cherry-pick commits (coming soon)**

### `ivaldi hunt <test-command>`
**Binary search for bugs (coming soon)**

---

## Utility Commands

### `ivaldi refresh`
**Refresh ignore patterns**

```bash
ivaldi refresh
```

**Features:**
- Reloads .ivaldiignore patterns
- Removes ignored files from staging
- Updates workspace state

### `ivaldi clean`
**Remove build artifacts**

```bash
ivaldi clean
```

### `ivaldi version`
**Show version information**

```bash
ivaldi version
```

### `ivaldi config`
**Manage configuration**

```bash
ivaldi config --list
ivaldi config user.name "Your Name"
ivaldi config user.email "you@example.com"
```

---

## Natural Language Reference

### Memorable Names
Every commit gets a unique, memorable name:
- `bright-river-42`
- `swift-mountain-156` 
- `calm-forest-23`
- `golden-stream-89`

### Iteration Numbers
Sequential numbers within timelines:
- `#1`, `#2`, `#3` - Global iterations
- `main#5` - Iteration 5 on main timeline
- `feature#2` - Iteration 2 on feature timeline

### Temporal References
Human-friendly time references:
- `"yesterday"`
- `"2 hours ago"`
- `"last Friday"`
- `"yesterday at 3pm"`
- `"this morning"`

### Author References
Reference commits by author:
- `"Sarah's last commit"`
- `"my morning changes"`
- `"John's work"`
- `"recent changes by Alice"`

### Content References
Find commits by content:
- `"where auth was added"`
- `"the commit about tests"`
- `"bug fix for login"`
- `"when we added the database"`

---

## File Patterns

### .ivaldiignore Patterns

```bash
# Directories
build/
logs/
temp/

# Files by extension
*.log
*.tmp
*.exe

# Specific files
secrets.txt
config.env

# Nested patterns
**/node_modules/
**/*.cache
```

### Command Patterns

```bash
# Gather patterns
ivaldi gather *.go *.md          # By extension
ivaldi gather src/ tests/        # Directories
ivaldi gather all                # All changes

# Exclude patterns  
ivaldi exclude *.log *.tmp       # Temporary files
ivaldi exclude build/ dist/      # Build directories
ivaldi exclude secrets.*         # Sensitive files

# Remove patterns
ivaldi remove old-*              # Files starting with 'old-'
ivaldi remove **/*.cache         # All cache files
```

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Repository not found |
| 4 | Permission denied |
| 5 | Network error |

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `IVALDI_EDITOR` | Default editor for messages | `vim` |
| `IVALDI_PAGER` | Pager for long output | `less` |
| `IVALDI_COLOR` | Enable/disable colors | `auto` |

---

## Configuration Files

### `.ivaldiignore`
File exclusion patterns (like .gitignore)

### `.ivaldi/config`
Repository-specific configuration

### `~/.ivaldi/global_config`
Global user configuration

---

## Keyboard Shortcuts (Interactive Mode)

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate history |
| `Tab` | Auto-complete |
| `Ctrl+C` | Cancel operation |
| `Enter` | Confirm selection |

---

## Integration

### Git Compatibility
Ivaldi maintains full Git compatibility:
- All Git repositories can be imported
- Ivaldi repositories work with Git tools
- Portal operations use Git under the hood
- Memorable names become Git commit messages

### IDE Integration
- Visual Studio Code extension (coming soon)
- Vim/Neovim plugins (coming soon)
- Terminal integration with rich output

---

## Troubleshooting

### Common Error Messages

**"not in repository"**
```bash
# Solution: Initialize or navigate to repository
ivaldi forge
# or
cd /path/to/repository
```

**"failed to refresh ignore patterns"**
```bash
# Solution: Check .ivaldiignore syntax
ivaldi refresh
```

**"portal not found"**
```bash
# Solution: Add portal first
ivaldi portal add origin <url>
```

### Debug Mode

```bash
# Enable verbose output
export IVALDI_DEBUG=1
ivaldi <command>
```

### Getting Help

```bash
# Command-specific help
ivaldi <command> --help

# Examples and usage
ivaldi help <command>

# Check status anytime
ivaldi status
ivaldi whereami
```

---

**This reference covers all current Ivaldi features. For tutorials and examples, see [TUTORIAL.md](TUTORIAL.md).**