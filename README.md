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
| `git merge --squash feature` | `ivaldi fuse feature --squash` |
| `git push origin feature` | `ivaldi upload` |

### Key Benefits

- **Memorable Names**: Every commit gets a human-friendly name like `bright-river-42`
- **Never Lose Work**: Automatic preservation prevents data loss
- **Natural Language**: `ivaldi jump to "yesterday"` or `ivaldi jump to "Sarah's last commit"`
- **Seamless GitHub**: Direct API integration without git dependency
- **Intuitive Commands**: Workshop metaphors that make sense

## Quick Start

### Installation

```bash
# Clone and build
git clone https://github.com/javanhut/Ivaldi.git
cd Ivaldi
make build

# Install system-wide (optional)
make install
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

# Make changes
echo "auth code" > auth.go
ivaldi gather .
ivaldi seal "Add authentication"
# → Sealed as: bright-shield-42

# Switch back to main and merge
ivaldi timeline switch main
ivaldi fuse auth --squash
# → Fused auth timeline into main
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
ivaldi timeline create <name>  # Create new timeline/branch
ivaldi timeline switch <name>  # Switch timeline
ivaldi timeline list           # List all timelines
ivaldi fuse <timeline>         # Merge timeline into current
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
ivaldi portal add origin <url>  # Add GitHub remote
ivaldi portal list             # List remotes
ivaldi upload                  # Upload current timeline
ivaldi sync origin             # Sync with remote
```

## Features

###  **Human-Friendly**
- Memorable commit names instead of SHA hashes
- Natural language navigation and references
- Workshop metaphor commands (forge, gather, seal, fuse)

###  **Data Protection**
- Automatic work preservation during timeline switches
- Mathematical impossibility of data loss
- Smart conflict resolution

###  **GitHub Integration**
- Direct REST API integration (no git dependency)
- Automatic branch creation for new timelines
- Batch uploads for performance
- Support for .ivaldiignore files

###  **Intelligent**
- Auto-detects file changes
- Smart timeline inheritance from main
- Efficient content-based storage

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
# Download existing repository
ivaldi download https://github.com/team/project.git
cd project

# Create your feature
ivaldi timeline create my-feature
# Make changes...
ivaldi gather . && ivaldi seal "My contribution"
ivaldi upload  # Creates branch on GitHub

# Then create pull request on GitHub
```

## FAQ

**Q: Do I need git installed?**
A: No! Ivaldi works completely independently and integrates directly with GitHub's API.

**Q: Can I use existing Git repositories?**
A: Yes! Use `ivaldi download <git-url>` to import any Git repository with full history preservation.

**Q: What happens to my work when I switch timelines?**
A: Ivaldi automatically preserves all uncommitted work - you can never lose changes.

**Q: How do memorable names work?**
A: Every commit gets a unique name like `bright-river-42`. You can reference commits by these names instead of SHA hashes.

**Q: Is this compatible with my team's Git workflow?**
A: Yes! Ivaldi creates standard Git repositories on GitHub that your team can interact with normally.

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
