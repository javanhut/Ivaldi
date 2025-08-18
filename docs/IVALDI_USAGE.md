# Ivaldi Usage Guide

Complete guide for using Ivaldi's git-independent version control system with seamless GitHub integration.

## Quick Start

### 1. Build Ivaldi
```bash
make build
```

### 2. Initialize Repository
```bash
./build/ivaldi forge
```

### 3. Gather Files
```bash
# Gather all files
./build/ivaldi gather .

# Or gather specific directories/files
./build/ivaldi gather cmd/ core/ forge/
./build/ivaldi gather go.mod go.sum Makefile
```

### 4. Create a Seal (Commit)
```bash
./build/ivaldi seal -m "Your commit message"
```

### 5. Configure GitHub Integration
```bash
# One-time setup for GitHub access
./build/ivaldi config
→ Enter GitHub username: your-username
→ Enter GitHub token: ghp_xxxxxxxxxxxx
→ ✅ GitHub credentials configured successfully
```

### 6. Add GitHub Portal
```bash
./build/ivaldi portal add origin https://github.com/yourusername/ivaldi.git
```

### 7. Sync with GitHub
```bash
# Native sync (no git commands used)
./build/ivaldi sync origin
→ ✅ Uploading 15 files in parallel...
→ ✅ Successfully uploaded 15 files in single commit
```

## Common Commands

### Configuration
```bash
# Interactive credential setup
./build/ivaldi config

# Reset stored credentials
./build/ivaldi config --reset
```

### Status & History
```bash
# Show workspace status
./build/ivaldi status

# View commit history
./build/ivaldi log

# Show current position
./build/ivaldi whereami
```

### Portal Management
```bash
# List configured portals
./build/ivaldi portal list

# Add new portal
./build/ivaldi portal add origin https://github.com/user/repo.git

# Sync with portal
./build/ivaldi sync origin
```

### Timeline Management
```bash
# List timelines (branches)
./build/ivaldi timeline list

# Create new timeline
./build/ivaldi timeline create feature-xyz "Working on XYZ feature"

# Switch timeline
./build/ivaldi timeline switch feature-xyz

# Fuse (merge) timelines
./build/ivaldi fuse feature-xyz                    # Merge feature-xyz into current
./build/ivaldi fuse feature-xyz --dry-run          # Preview merge
./build/ivaldi fuse feature-xyz --strategy=squash  # Squash commits
./build/ivaldi fuse feature-xyz --delete-source    # Delete source timeline after merge
```

## Full Workflow Example

```bash
# Make changes to code...

# Check status
./build/ivaldi status

# Gather changed files
./build/ivaldi gather .

# Create seal
./build/ivaldi seal -m "Implemented new feature"

# Push to GitHub
./build/ivaldi sync --push
```

## Installing System-Wide

```bash
# Install to /usr/local/bin
make install

# Now you can use ivaldi from anywhere
ivaldi status
```

## File Ignore Support

Create a `.ivaldiignore` file to exclude files from tracking:

```bash
# Create ignore file
cat > .ivaldiignore << EOF
build/
*.log
*.tmp
*.exe
node_modules/
.DS_Store
EOF

# Sync respects ignore patterns
./build/ivaldi sync origin
→ ✅ Skipped 23 ignored files
→ ✅ Uploaded 12 source files
```

### Built-in Ignore Patterns
Ivaldi automatically ignores:
- `.ivaldi/` directory
- `.git/` directory  
- `build/` directory
- `*.tmp`, `*.temp`, `*.log`, `*.bak` files
- `.DS_Store` files

## Advanced Features

### Commit Squashing
```bash
# Squash multiple commits into one
./build/ivaldi squash --all "Clean commit message"
→ ✅ Successfully created clean commit: d6de547d

# Squash with force push to rewrite history
./build/ivaldi squash --all "Clean implementation" --force-push
→ ✅ Successfully squashed commits and updated origin!
```

### Force Push with History Rewriting
```bash
# Rewrite GitHub history with clean commits
./build/ivaldi squash --count 5 "Consolidated feature work" --force-push
```

### Batch Upload Performance
Ivaldi uses GitHub's Git Data API for fast batch uploads:
- **10x faster** than individual file uploads
- Atomic commits (all files uploaded together)
- Parallel processing for large repositories

## Notes

- **Git-independent**: Ivaldi does not use git commands underneath
- **GitHub integration**: Direct REST API communication for uploads/downloads
- **Token authentication**: Uses GitHub Personal Access Tokens for security
- **Workspace preservation**: State is automatically saved between commands
- **Ignore file support**: Respects .ivaldiignore patterns with glob matching