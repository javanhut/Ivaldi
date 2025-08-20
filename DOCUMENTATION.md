# Ivaldi Documentation Index

All documentation has been organized in the **[docs/](docs/)** directory for better structure and navigation.

## Quick Access

### **Getting Started**
- **[Complete Documentation Index](docs/README.md)** - Full documentation overview
- **[Quick Usage Guide](docs/IVALDI_USAGE.md)** - Essential commands and workflows
- **[Shell Integration Guide](docs/SHELL_INTEGRATION.md)** - Timeline info in your prompt
- **[Revolutionary Demo](docs/REVOLUTIONARY_DEMO_COMPLETE.md)** - See all features in action

### **Understanding Ivaldi**
- **[Why Revolutionary](docs/REVOLUTIONARY_DEMO.md)** - Core concepts and philosophy  
- **[Implementation Status](docs/IMPLEMENTATION_STATUS.md)** - Complete feature list and status
- **[Development Workflow](docs/DEMO_WORKFLOW.md)** - Examples and best practices

## Navigation

```
docs/
├── README.md                        # Complete documentation index
├── IVALDI_USAGE.md                 # Quick start and essential commands
├── SHELL_INTEGRATION.md            # Timeline info in shell prompts
├── REFERENCE.md                    # Complete command reference
├── CONFIG_COMMAND.md               # GitHub credential setup
├── SYNC_COMMAND.md                 # Git-independent sync documentation
├── SQUASH_COMMAND.md               # Commit squashing and history cleanup
├── IVALDIIGNORE.md                 # File ignore patterns and management
├── REVOLUTIONARY_DEMO_COMPLETE.md  # Full feature demonstration
├── REVOLUTIONARY_DEMO.md           # Revolutionary concepts explained
├── IMPLEMENTATION_STATUS.md        # Feature status and development progress
└── DEMO_WORKFLOW.md               # Development workflows and examples
```

## Quick Command Reference

```bash
# Essential workflow (Git-Independent)
ivaldi config                  # Setup GitHub credentials
ivaldi forge                   # Create repository
ivaldi gather all              # Stage files
ivaldi seal "Add feature"       # Commit with memorable name
ivaldi sync origin             # Sync to GitHub (no git commands)

# Timeline management  
ivaldi timeline create auth    # Create timeline
ivaldi timeline switch auth    # Switch timeline
ivaldi fuse auth --squash      # Merge with squash

# Advanced features
ivaldi squash --all "Clean"    # Squash commits
ivaldi squash --force-push     # Rewrite GitHub history

# File management
echo "build/\n*.log" > .ivaldiignore  # Ignore patterns
ivaldi refresh                 # Reload ignore patterns

# Natural language navigation
ivaldi jump to "yesterday"
ivaldi jump to bright-river-42
ivaldi jump back 3
```

**[→ Go to Complete Documentation](docs/README.md)**