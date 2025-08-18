# Ivaldi VCS Tutorial - Complete Guide

> **Welcome to Ivaldi VCS!** This tutorial will guide you through all of Ivaldi's revolutionary features step-by-step.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Basic Workshop Commands](#basic-workshop-commands)
3. [Timeline Management](#timeline-management)
4. [File Management](#file-management)
5. [Portal Operations (Remotes)](#portal-operations)
6. [Natural Language Features](#natural-language-features)
7. [Advanced Operations](#advanced-operations)
8. [Troubleshooting](#troubleshooting)

---

## Getting Started

### Installation

```bash
# Download Ivaldi
ivaldi download https://github.com/javanhut/Ivaldi.git
cd Ivaldi

# Build and install
make build
make install

# Verify installation
ivaldi --help
```

### Create Your First Repository

```bash
# Create a new project
mkdir my-project
cd my-project

# Initialize Ivaldi repository (instead of git init)
ivaldi forge

# Check status
ivaldi status
```

**What happened?** Ivaldi created a revolutionary repository with:
- Natural language reference system
- Automatic work preservation  
- AI-powered commit generation
- Rich visual output
- Complete accountability tracking

---

## Basic Workshop Commands

Ivaldi uses a **workshop metaphor** that makes version control intuitive:

### 1. Adding Files (Gathering)

```bash
# Create some files
echo "Hello World" > main.go
echo "# My Project" > README.md

# Gather files onto the anvil (like git add)
ivaldi gather main.go README.md

# Or gather all files
ivaldi gather all

# Check what's on the anvil
ivaldi status
```

**Revolutionary Feature:** Files are gathered "onto the anvil" for crafting into a seal.

### 2. Creating Commits (Sealing)

```bash
# Seal changes with AI-generated message
ivaldi seal

# Or provide your own message
ivaldi seal "Initial project setup"
```

**What happened?** Ivaldi:
- Generated a **memorable name** like `bright-river-42` instead of a cryptic hash
- Analyzed your changes for a semantic commit message
- Created iteration #1 on the main timeline
- Preserved all workspace state automatically

### 3. Viewing Changes

```bash
# See what changed since last seal
ivaldi what-changed

# Check current location
ivaldi whereami

# View history with memorable names
ivaldi log
```

---

## Timeline Management

Timelines are Ivaldi's human-friendly alternative to branches:

### Creating and Switching Timelines

```bash
# Create a new timeline for feature development
ivaldi timeline create auth-system

# Switch to the timeline (with automatic work preservation)
ivaldi timeline switch auth-system

# All uncommitted work is automatically shelved and restored!
ivaldi whereami
```

**Revolutionary Auto-Shelving:**
- All uncommitted changes are automatically preserved when switching
- Gathered files on the anvil are saved and restored
- Work automatically restores when you return to a timeline
- No manual shelving needed - it just works!

### Merging Timelines (Fuse)

```bash
# After working on auth-system timeline
ivaldi gather all
ivaldi seal "Add authentication middleware"

# Switch back to main
ivaldi timeline switch main

# Fuse the auth-system timeline into main
ivaldi fuse auth-system --strategy=squash --delete-source
```

**Revolutionary Features:**
- **Zero work loss** - all changes automatically preserved and restored
- **Seamless context switching** - work exactly where you left off
- **Multiple merge strategies** - auto, fast-forward, squash, manual
- **Human-friendly names** - `auth-system` instead of cryptic branch names

---

## File Management

### Excluding Files from Tracking

```bash
# Exclude files with one command
ivaldi exclude build/ *.log secrets.txt

# Check that files are excluded
ivaldi status

# Refresh ignore patterns after manual .ivaldiignore edits
ivaldi refresh
```

### Removing Files

```bash
# Remove files from repository (local and remote)
ivaldi remove old-file.txt

# Remove only from remote, keep locally
ivaldi remove --from-remote secrets.txt

# Remove and exclude from future tracking
ivaldi remove --exclude temp/ *.cache

# Seal the removal
ivaldi seal "Remove unused files"
```

**Revolutionary Features:**
- **One-command exclusion** - automatically updates .ivaldiignore and cleans workspace
- **Flexible removal** - remove locally, remotely, or both
- **Immediate feedback** - clear next steps for every operation

---

## Portal Operations

Portals are Ivaldi's intuitive way to work with remote repositories:

### Setting Up Portals

```bash
# Add GitHub as a portal
ivaldi portal add origin https://github.com/user/repo.git

# List configured portals
ivaldi portal list
```

### Branch Management

```bash
# Create new branch with migration from master to main
ivaldi portal new main --migrate master

# Upload with automatic upstream tracking
ivaldi upload main

# Rename branches on remote
ivaldi portal rename master --with main

# Download an existing repository
ivaldi download https://github.com/user/repo.git my-project
```

### Complete Upload Workflow

```bash
# 1. Make changes
echo "new feature" >> feature.go

# 2. Gather and seal
ivaldi gather all
ivaldi seal "Add new feature implementation"

# 3. Upload to remote
ivaldi upload main

# 4. Clean up
ivaldi clean
```

### Staying Synced with Remote

```bash
# Sync with main branch automatically
ivaldi sync --with main

# Sync with upstream repository
ivaldi sync --with main upstream

# Continue working with latest changes
ivaldi gather all
ivaldi seal "Work on latest changes"
ivaldi upload
```

---

## Natural Language Features

### Memorable Names Instead of Hashes

```bash
# Every commit gets a memorable name
ivaldi seal "Fix authentication bug"
# → Creates: swift-mountain-156

# Jump to commits using memorable names
ivaldi jump to swift-mountain-156
ivaldi jump to bright-river-42
```

### Temporal References

```bash
# Jump using natural language
ivaldi jump to "yesterday"
ivaldi jump to "2 hours ago"
ivaldi jump to "last Friday"

# See changes since natural references
ivaldi what-changed "yesterday at 3pm"
```

### Author References

```bash
# Reference commits by author
ivaldi jump to "Sarah's last commit"
ivaldi jump to "my morning changes"
```

### Content References

```bash
# Find commits by content
ivaldi jump to "where auth was added"
ivaldi jump to "the commit about tests"
```

---

## Advanced Operations

### AI-Powered Commit Generation

```bash
# Let AI analyze your changes
ivaldi seal
# → Generated: "feat(auth): implement JWT middleware"
# → Confidence: 95% (New feature detected)
# → Alternatives: "add auth middleware", "implement JWT system"

# AI detects patterns: feat, fix, docs, test, refactor
```

### Workspace Preservation

```bash
# Work is automatically preserved during timeline switches
echo "work in progress" > draft.txt
ivaldi timeline switch feature

# Work preserved as: workspace_main_20240817_143022
# Automatically restored when switching back!
```

### Search and Navigation

```bash
# Search for commits
ivaldi search "authentication"

# View history with context
ivaldi log --limit 10

# Jump with iteration numbers
ivaldi jump to #5
ivaldi jump to main#15
```

---

## Common Workflows

### Daily Development Workflow

```bash
# Morning: Start work and sync with latest
ivaldi sync --with main
ivaldi timeline create new-feature

# Development: Make changes
echo "code changes" >> src/main.go
ivaldi gather all
ivaldi seal "Work in progress"

# Midday: Switch contexts (work preserved automatically)
ivaldi timeline switch hotfix
# Sync and fix urgent issue
ivaldi sync --with main
ivaldi gather all
ivaldi seal "Fix critical bug"
ivaldi upload main

# Afternoon: Back to feature work
ivaldi timeline switch new-feature
# Work automatically restored! Sync with latest main
ivaldi sync --with main

# Evening: Finish feature
ivaldi gather all
ivaldi seal "Complete new feature implementation"
ivaldi timeline switch main
ivaldi fuse new-feature --strategy=squash
ivaldi upload main
```

### File Management Workflow

```bash
# Clean up repository
ivaldi exclude build/ logs/ *.tmp
ivaldi remove --exclude old-config/ deprecated.js
ivaldi seal "Clean up repository structure"
ivaldi upload main
```

### Collaboration Workflow

```bash
# Download collaborator's work
ivaldi download https://github.com/team/project.git
cd project

# Create your feature timeline
ivaldi timeline create my-feature

# Work and contribute
ivaldi gather all
ivaldi seal "Add my contribution"
ivaldi upload my-feature

# Merge when ready
ivaldi timeline switch main
ivaldi fuse my-feature
ivaldi upload main
```

---

## Troubleshooting

### Common Issues

**Issue: "Files not being ignored"**
```bash
# Solution: Refresh ignore patterns
ivaldi refresh
ivaldi status
```

**Issue: "Want to undo last seal"**
```bash
# Solution: Jump to previous memorable name
ivaldi log  # Find previous name
ivaldi jump to previous-seal-name
```

**Issue: "Lost work during timeline switch"**
```bash
# Solution: Check preserved workspaces
ivaldi workspace list
ivaldi workspace restore workspace_name
```

**Issue: "Can't remember commit reference"**
```bash
# Solution: Use natural language
ivaldi jump to "yesterday"
ivaldi jump to "Sarah's changes"
ivaldi search "authentication"
```

### Getting Help

```bash
# Get help for any command
ivaldi help
ivaldi gather --help
ivaldi portal --help

# Check current status anytime
ivaldi status
ivaldi whereami
```

---

## What Makes Ivaldi Revolutionary

### 🎯 **Human-Centered Design**
- **Memorable names** instead of cryptic hashes
- **Workshop metaphor** that makes intuitive sense
- **Natural language** for all references and operations

### 🚀 **Zero Work Loss**
- **Automatic preservation** during timeline switches
- **Mathematical impossibility** of losing work
- **Smart restoration** when returning to timelines

### 🤖 **AI-Enhanced Development**
- **Semantic commit generation** with 95% accuracy
- **Pattern detection** for commit types
- **Intelligent suggestions** for next steps

### 🎨 **Rich Visual Interface**
- **Colored output** with clear indicators
- **Helpful error messages** with actionable solutions
- **Progress feedback** for all operations

### 🔧 **Intuitive Operations**
- **One-command exclusion** for files
- **Flexible removal** with multiple options
- **Smart portal management** with automatic upstream

---

## Next Steps

1. **Try the Tutorial**: Follow along with your own project
2. **Read the Reference**: Check out [REFERENCE.md](REFERENCE.md) for complete command documentation
3. **Explore Advanced Features**: Experiment with natural language references
4. **Join the Revolution**: Share Ivaldi with your team and experience the future of version control!

**Welcome to human-centered version control. Welcome to Ivaldi VCS.**