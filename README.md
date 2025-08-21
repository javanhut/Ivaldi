# Ivaldi - New  Version Control System

> **Version control that makes sense to humans, not just computers.**

Ivaldi is a modern version control system designed around how developers actually think and work. Instead of cryptic hashes and complex commands, Ivaldi gives you memorable names, intuitive workflows, and automatic safeguards against data loss.

## Why Ivaldi?

**Git forces you to think like a computer. Ivaldi lets you think like a human.**

| Git (Complex) | Ivaldi (Simple) |
|---------------|-----------------|
| `git checkout a7b8c9d` | `ivaldi jump to bright-river-42` |
| `git add . && git commit -m "..."` | `ivaldi gather . && ivaldi seal "..."` |
| `git branch feature && git checkout feature` | `ivaldi timeline create feature` |
| `git branch -m master main` | `ivaldi rename master --to main` |
| `git merge --squash feature` | `ivaldi fuse feature --squash` |
| `git push origin feature` | `ivaldi upload` |

### Key Benefits

- **Memorable Names**: Every commit gets a human-friendly name like `bright-river-42`
- **Never Lose Work**: Automatic preservation prevents data loss
- **Natural Language**: `ivaldi jump to "yesterday"` or `ivaldi jump to "Sarah's last commit"`
- **Shell Integration**: Timeline info in your prompt just like Git branches
- **Easy Renaming**: Rename timelines with automatic remote handling
- **Seamless GitHub**: Direct API integration without git dependency
- **Intuitive Commands**: Workshop metaphors that make sense

## Quick Start

### Installation

```bash
# Clone and build
git clone https://github.com/javanhut/Ivaldi.git
cd Ivaldi
make build

# Install system-wide (includes shell prompt integration)
make install

# OR install to user directory (no sudo required)
make dev-install
```

### Shell Integration

Get timeline info in your prompt like Git branches:

**For Oh My Zsh users:**
```bash
# Quick setup (recommended)
./scripts/quick-setup.sh

# Your prompt will show: ➜ myproject git:(main) ivaldi:(feature-timeline)
```

**For other shells:**
```bash
# General installation
./scripts/oh-my-zsh-plugin/install.sh

# See full documentation
./scripts/INSTALL_PROMPT.md
```

### First Repository

```bash
# Create a new project
ivaldi forge my-project
cd my-project

# Configure GitHub (one-time setup)
ivaldi config
# Enter your GitHub username and personal access token

# Add some files
echo "Hello World" > main.go

# Stage and commit
ivaldi gather .
ivaldi seal "Initial implementation"
# → Sealed as: golden-stream-1

# Add GitHub remote and upload
ivaldi portal add origin https://github.com/your-username/my-project.git
ivaldi upload
# → Upload complete: origin/main
```

### Working with Timelines (Branches)

```bash
# Create and switch to feature timeline
ivaldi timeline create auth
ivaldi timeline switch auth

# Rename timeline if needed
ivaldi rename auth --to user-authentication

# Make changes
echo "auth code" > auth.go
ivaldi gather .
ivaldi seal "Add authentication"
# → Sealed as: bright-shield-42

# Switch back to main and merge
ivaldi timeline switch main
ivaldi fuse user-authentication --squash
# → Fused user-authentication timeline into main
```

## Essential Commands

### Basic Workflow
```bash
ivaldi forge <name>           # Create new repository
ivaldi gather <files>         # Stage files (like git add)
ivaldi seal "<message>"       # Commit changes (like git commit)
ivaldi status                 # Show workspace status
ivaldi upload                 # Push to remote (like git push)
```

### Timeline Management
```bash
ivaldi timeline create <name>    # Create new timeline/branch
ivaldi timeline switch <name>    # Switch timeline
ivaldi timeline list             # List all timelines
ivaldi rename <old> --to <new>   # Rename timeline
ivaldi fuse <timeline>           # Merge timeline into current
```

### Navigation
```bash
ivaldi jump to <reference>     # Go to any commit
ivaldi whereami               # Show current position
ivaldi log                    # View history
ivaldi what-changed           # Show changes since last commit
```

### Natural Language Examples
```bash
# Time-based navigation
ivaldi jump to "yesterday"
ivaldi jump to "2 hours ago" 
ivaldi jump to "last Friday"

# Author-based references
ivaldi jump to "Sarah's last commit"
ivaldi jump to "my morning changes"

# Content-based references  
ivaldi jump to "where auth was added"
ivaldi jump to bright-river-42
ivaldi jump to #15
```

### Remote Operations
```bash
ivaldi mirror <url>            # Clone repo with full Git history  
ivaldi download <url>          # Download current files only
ivaldi portal add origin <url> # Add GitHub remote
ivaldi portal list             # List remotes
ivaldi upload                  # Upload current timeline
ivaldi upload --all            # Upload all local timelines
ivaldi sync origin             # Sync current timeline with remote
ivaldi sync origin --all       # Sync all remote timelines
ivaldi scout origin            # Discover all remote timelines
```

## Features

###  **Human-Friendly**
- Memorable commit names instead of SHA hashes
- Natural language navigation and references
- Workshop metaphor commands (forge, gather, seal, fuse)
- Timeline info in shell prompt like Git branches
- Easy timeline renaming with `--to` syntax

###  **Data Protection**
- Automatic work preservation during timeline switches
- Mathematical impossibility of data loss
- Smart conflict resolution
- Local changes preserved during sync operations

###  **GitHub Integration**
- Direct REST API integration (no git dependency)
- Automatic branch creation for new timelines
- Timeline rename creates new remote branches
- Distinguish between `mirror` (with Git history) and `download` (files only)
- Batch uploads for performance
- Support for .ivaldiignore files
- Per-timeline upload state tracking
- Timeline name validation for Git compatibility

###  **Advanced Sync Capabilities**
- **Concurrent Downloads**: 8x faster sync with parallel file downloads
- **Timeline Discovery**: Automatically find all remote branches/timelines
- **Bulk Synchronization**: Sync all remote timelines at once
- **Selective Sync**: Choose specific timelines to sync
- **Smart Change Detection**: Only sync modified files
- **Progress Tracking**: Real-time download progress bars
- **Multiple Timeline Upload**: Upload different timelines independently
- **Error Resilience**: Continue syncing even if individual timelines fail

###  **Intelligent**
- Auto-detects file changes
- Smart timeline inheritance from main
- Efficient content-based storage
- Optimized file scanning (skips binaries and large files)

## Configuration

### GitHub Setup
```bash
ivaldi config
# Follow prompts to enter:
# - GitHub username
# - Personal access token (with repo permissions)
```

### Ignore Files
Create `.ivaldiignore` to exclude files:
```
build/
*.log
*.tmp
node_modules/
.env
```

## Examples

### Starting a New Project
```bash
# Initialize
ivaldi forge web-app
cd web-app
ivaldi portal add origin https://github.com/user/web-app.git

# First commit
echo "# My Web App" > README.md
ivaldi gather . && ivaldi seal "Initial commit"
ivaldi upload  # Creates remote repository
```

### Feature Development
```bash
# Create feature timeline
ivaldi timeline create user-auth
# Work is automatically preserved when switching

# Develop feature
echo "auth logic" > auth.js
ivaldi gather . && ivaldi seal "Add user authentication"

# Switch back and merge
ivaldi timeline switch main
ivaldi fuse user-auth --squash  # Clean merge
ivaldi upload  # Push to GitHub
```

### Collaborating
```bash
# Mirror repository with full Git history
ivaldi mirror https://github.com/team/project.git
cd project

# OR just download current files without history
ivaldi download https://github.com/team/project.git
cd project

# Create your feature
ivaldi timeline create my-feature
# Make changes...
ivaldi gather . && ivaldi seal "My contribution"

# Rename timeline if needed
ivaldi rename my-feature --to better-feature-name

ivaldi upload  # Creates branch on GitHub

# Then create pull request on GitHub
```

### Advanced Sync Operations
```bash
# Discover all remote timelines
ivaldi scout origin
# → Found 5 timelines: main, develop, feature-auth, bugfix-123, release-2.0

# Sync all remote timelines at once
ivaldi sync origin --all
# → Syncing 5 timelines...
# → ✓ main: 42 files updated
# → ✓ develop: 15 files updated
# → ✓ feature-auth: 8 files updated
# → ✓ bugfix-123: 3 files updated
# → ✓ release-2.0: Already up to date

# Sync specific timelines only
ivaldi sync origin --timelines main,develop,feature-auth
# → Syncing 3 selected timelines...

# Upload multiple timelines
ivaldi timeline list
# → main, feature-x, bugfix-y
ivaldi upload --all
# → Uploading all 3 timelines to origin...
# → Each timeline maintains its own upload state

# Handle timeline conflicts
ivaldi sync origin --timeline feature-auth
# → Fetching feature-auth from origin...
# → Local changes preserved
# → Merging remote changes...
# → ✓ Sync complete: 8 files updated, 2 local changes preserved
```

### Shell Prompt Integration
```bash
# With Oh My Zsh plugin installed:
➜  myproject git:(main) ivaldi:(feature-timeline) 

# Available aliases:
irename master --to main    # Rename timeline
igather .                   # Stage files  
iseal "message"            # Commit changes
iswitch main               # Switch timeline
```

## FAQ

**Q: Do I need git installed?**
A: No! Ivaldi works completely independently and integrates directly with GitHub's API.

**Q: Can I use existing Git repositories?**
A: Yes! Use `ivaldi mirror <git-url>` to import any Git repository with full history preservation, or `ivaldi download <git-url>` for just the current files.

**Q: What happens to my work when I switch timelines?**
A: Ivaldi automatically preserves all uncommitted work - you can never lose changes.

**Q: How do memorable names work?**
A: Every commit gets a unique name like `bright-river-42`. You can reference commits by these names instead of SHA hashes.

**Q: How do I rename timelines like master to main?**
A: Use `ivaldi rename master --to main`. When you upload, it creates the new branch name on GitHub.

**Q: Can I get timeline info in my shell prompt?**
A: Yes! Run `./scripts/quick-setup.sh` for Oh My Zsh integration, or see `./scripts/INSTALL_PROMPT.md` for other shells.

**Q: Is this compatible with my team's Git workflow?**
A: Yes! Ivaldi creates standard Git repositories on GitHub that your team can interact with normally.

**Q: How fast is syncing compared to git?**
A: Ivaldi uses concurrent downloads with 8 parallel workers, making large repository syncs up to 8x faster than sequential operations.

**Q: Can I sync multiple branches at once?**
A: Yes! Use `ivaldi sync origin --all` to sync all remote timelines, or `ivaldi sync origin --timelines main,develop` for specific ones.

**Q: What happens if I upload multiple timelines?**
A: Each timeline maintains its own upload state, so you can upload different timelines independently without conflicts. Use `ivaldi upload --all` to upload all local timelines.

## Contributing

1. Fork the repository
2. Create timeline: `ivaldi timeline create feature-name`
3. Make changes and seal: `ivaldi seal "Add feature"`
4. Upload: `ivaldi upload`
5. Create pull request on GitHub

## License

MIT License - see [LICENSE](LICENSE) for details.

---

**Ready to try version control that actually makes sense?** Start with `ivaldi forge my-project` and experience the difference.
