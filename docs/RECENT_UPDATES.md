# Recent Updates to Ivaldi

> **Latest enhancements and new features added to Ivaldi**

## ✨ New Features

### 🔄 Timeline Rename Functionality
**Easy timeline renaming with remote overwrite support**

- **Command**: `ivaldi rename <old> --to <new>` or `ivaldi timeline rename <old> --to <new>`
- **Examples**:
  ```bash
  ivaldi rename master --to main
  ivaldi rename old-feature --to new-feature
  ivaldi rename --to better-name  # renames current timeline
  ```
- **Features**:
  - Preserves all timeline history and metadata
  - Updates current timeline reference if renaming active timeline
  - Renames associated state files automatically
  - When uploading, creates new remote branch with new name
  - Perfect for GitHub's master→main migration

### 🐚 Shell Integration & Prompt Support
**Timeline information in your shell prompt like Git branches**

- **Quick Setup**: Run `./scripts/quick-setup.sh` for Oh My Zsh
- **Manual Setup**: See `./docs/SHELL_INTEGRATION.md` for all shells
- **Visual**: `➜  myproject git:(main) ivaldi:(feature-timeline)`
- **Features**:
  - Oh My Zsh plugin with convenient aliases
  - Custom theme showing both Git and Ivaldi info
  - Works with bash, zsh, and other shells
  - Fast JSON parsing with `jq` support

### 📥 Distinguished Download Commands
**Clear distinction between mirror and download operations**

- **Mirror**: `ivaldi mirror <url>` - Clones with full Git history
- **Download**: `ivaldi download <url>` - Downloads current files only
- **Use Cases**:
  - Mirror for preserving development history
  - Download for clean slate without historical baggage

## 🔧 Enhanced Features

### 📋 Oh My Zsh Plugin Aliases
**Convenient shortcuts for common operations**

| Alias | Command | Purpose |
|-------|---------|---------|
| `iva` | `ivaldi` | Main command |
| `igather` | `ivaldi gather` | Stage files |
| `iseal` | `ivaldi seal` | Commit changes |
| `irename` | `ivaldi rename` | Rename timeline |
| `iswitch` | `ivaldi timeline switch` | Switch timeline |
| `istatus` | `ivaldi status` | Check status |
| `imirror` | `ivaldi mirror` | Mirror with history |
| `idownload` | `ivaldi download` | Download files only |

### 🎨 Enhanced Help and Documentation
**Improved command help and comprehensive guides**

- Updated main help text with clear command descriptions
- Enhanced timeline command help with rename options
- New shell integration documentation
- Updated README with latest features
- Complete command reference documentation

### ⚙️ Installation Improvements
**Better installation experience with shell integration**

- `make install` now includes shell prompt scripts
- `make dev-install` for user-local installation
- Quick setup script for Oh My Zsh users
- Comprehensive installation documentation

## 🔄 Updated Commands

### Timeline Commands
```bash
# All timeline management in one place
ivaldi timeline create <name>        # Create timeline
ivaldi timeline switch <name>        # Switch timeline  
ivaldi timeline list                 # List timelines
ivaldi timeline delete <name>        # Delete timeline
ivaldi timeline rename <old> --to <new>  # Rename timeline

# Convenient shortcuts
ivaldi rename <old> --to <new>       # Top-level rename
ivaldi rename --to <new>             # Rename current timeline
```

### Repository Commands
```bash
# Clear distinction between operations
ivaldi mirror <url> [dest]          # Clone with Git history
ivaldi download <url> [dest]        # Download files only
ivaldi forge <name>                  # Create new repository
```

## 🚀 Installation & Setup

### Quick Installation
```bash
# Clone and build
git clone https://github.com/javanhut/Ivaldi.git
cd Ivaldi
make build

# Install with shell integration
make install  # system-wide
# OR
make dev-install  # user-local

# Setup shell prompt (Oh My Zsh)
./scripts/quick-setup.sh
```

### Shell Integration
```bash
# Quick setup for Oh My Zsh
./scripts/quick-setup.sh

# Manual setup for other shells
source scripts/ivaldi-prompt.sh
```

## 📚 Documentation Updates

### New Documentation Files
- **[SHELL_INTEGRATION.md](SHELL_INTEGRATION.md)** - Complete shell integration guide
- **[RECENT_UPDATES.md](RECENT_UPDATES.md)** - This file with latest changes

### Updated Files
- **[README.md](../README.md)** - Updated with new features and examples
- **[REFERENCE.md](REFERENCE.md)** - Added rename commands and mirror/download distinction
- **[DOCUMENTATION.md](../DOCUMENTATION.md)** - Updated index with new guides

### Installation Scripts
- **[quick-setup.sh](../scripts/quick-setup.sh)** - One-click Oh My Zsh setup
- **[oh-my-zsh-plugin/](../scripts/oh-my-zsh-plugin/)** - Complete plugin with theme
- **[INSTALL_PROMPT.md](../scripts/INSTALL_PROMPT.md)** - General shell setup guide

## 🔍 Migration Guide

### From Previous Ivaldi Versions
```bash
# Update your installation
git pull
make clean && make build
make install  # or make dev-install

# Setup shell integration  
./scripts/quick-setup.sh

# Start using new features
ivaldi rename master --to main
```

### GitHub Master→Main Migration
```bash
# Rename your timeline
ivaldi rename master --to main

# Upload to create new remote branch
ivaldi upload

# The old 'master' branch remains on GitHub for safety
# You can delete it manually when ready
```

## 🎯 What's Next

These updates make Ivaldi more user-friendly and integrate better with modern development workflows:

- ✅ **Easy timeline renaming** for better branch management
- ✅ **Visual feedback** in shell prompts
- ✅ **Convenient aliases** for faster workflows  
- ✅ **Clear command distinctions** between mirror and download
- ✅ **Better documentation** and setup experience

## 🤝 Feedback

Have ideas for improvements? Found issues? We'd love to hear from you:

1. **GitHub Issues**: Report bugs or request features
2. **Documentation**: Help improve guides and examples
3. **Shell Integration**: Test with different terminal setups
4. **Workflow Feedback**: Share your Ivaldi usage patterns

---

**Ready to try the latest features?** Update your Ivaldi installation and run `./scripts/quick-setup.sh` to get started!