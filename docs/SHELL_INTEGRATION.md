# Ivaldi Shell Integration Guide

> **Get timeline information in your shell prompt just like Git branch info**

This guide covers how to integrate Ivaldi timeline information into your shell prompt, providing visual feedback about your current timeline similar to how Git shows branch information.

## What You Get

With shell integration, your prompt will show Ivaldi timeline information:

```bash
# Before integration
➜  myproject git:(main)

# After integration  
➜  myproject git:(main) ivaldi:(feature-timeline)

# In Ivaldi-only repos
➜  myproject ivaldi:(main)

# Outside any repository
➜  ~
```

## Quick Setup (Oh My Zsh)

**For Oh My Zsh users, this is the fastest way:**

```bash
# Navigate to your Ivaldi directory
cd /path/to/Ivaldi

# Run the quick setup script
./scripts/quick-setup.sh

# Restart your terminal or reload your config
source ~/.zshrc
```

This will:
- ✅ Install the Ivaldi Oh My Zsh plugin
- ✅ Switch to the enhanced robbyrussell theme (shows both Git and Ivaldi)
- ✅ Add convenient aliases (`irename`, `igather`, `iseal`, etc.)
- ✅ Backup your current `.zshrc`

## Manual Installation

### Oh My Zsh Plugin

1. **Install the plugin:**
   ```bash
   # Create plugin directory
   mkdir -p ~/.oh-my-zsh/custom/plugins/ivaldi
   
   # Copy plugin file
   cp scripts/oh-my-zsh-plugin/ivaldi.plugin.zsh ~/.oh-my-zsh/custom/plugins/ivaldi/
   
   # Copy enhanced theme (optional)
   cp scripts/oh-my-zsh-plugin/robbyrussell-ivaldi.zsh-theme ~/.oh-my-zsh/custom/themes/
   ```

2. **Update your ~/.zshrc:**
   ```bash
   # Add ivaldi to plugins
   plugins=(git ivaldi)
   
   # Optional: use the enhanced theme
   ZSH_THEME="robbyrussell-ivaldi"
   ```

3. **Reload shell:**
   ```bash
   source ~/.zshrc
   ```

### General Bash/Zsh (Non-Oh My Zsh)

1. **Source the prompt script:**
   ```bash
   # Add to ~/.bashrc or ~/.zshrc
   source /path/to/Ivaldi/scripts/ivaldi-prompt.sh
   ```

2. **For Bash - Update PS1:**
   ```bash
   # Add timeline info to your prompt
   PS1="${PS1%\$ }\$(ivaldi_timeline_prompt)\$ "
   
   # Or with colors
   PS1="\[\033[0;32m\]\u@\h\[\033[0m\]:\[\033[0;34m\]\w\[\033[0;35m\]\$(ivaldi_timeline_prompt)\[\033[0m\]\$ "
   ```

3. **For Zsh - Update PROMPT:**
   ```bash
   # Enable command substitution
   setopt PROMPT_SUBST
   
   # Add timeline info to your prompt  
   PROMPT="${PROMPT%% }%{\$(ivaldi_timeline_prompt_zsh)%} "
   
   # Or with colors
   PROMPT='%F{green}%n@%m%f:%F{blue}%~%f%F{magenta}$(ivaldi_timeline_prompt_zsh)%f$ '
   ```

## Available Aliases

The Oh My Zsh plugin provides many convenient aliases:

### Core Commands
- `iva` → `ivaldi`
- `igather` → `ivaldi gather`
- `iseal` → `ivaldi seal` 
- `istatus` → `ivaldi status`

### Timeline Management
- `itimeline` → `ivaldi timeline`
- `iswitch` → `ivaldi timeline switch`
- `icreate` → `ivaldi timeline create`
- `ilist` → `ivaldi timeline list`
- `irename` → `ivaldi rename`

### Repository Operations
- `iforge` → `ivaldi forge`
- `imirror` → `ivaldi mirror`
- `idownload` → `ivaldi download`
- `ijump` → `ivaldi jump`
- `ihistory` → `ivaldi history`

### Git-Style Shortcuts
- `iadd` → `ivaldi gather`
- `icommit` → `ivaldi seal`
- `ibranch` → `ivaldi timeline`
- `icheckout` → `ivaldi timeline switch`

### Advanced Patterns
- `igatherall` → `ivaldi gather .`
- `isealmsg` → `ivaldi seal`

## Customization

### Changing Timeline Display Format

You can customize how timeline information appears:

```bash
# Default format: (timeline: main)
ZSH_THEME_IVALDI_PROMPT_PREFIX="(timeline: "
ZSH_THEME_IVALDI_PROMPT_SUFFIX=")"

# Custom formats
ZSH_THEME_IVALDI_PROMPT_PREFIX="[timeline: "
ZSH_THEME_IVALDI_PROMPT_SUFFIX="] "

# Minimal format
ZSH_THEME_IVALDI_PROMPT_PREFIX=" on "
ZSH_THEME_IVALDI_PROMPT_SUFFIX=""

# With emoji
ZSH_THEME_IVALDI_PROMPT_PREFIX=" 🌿"
ZSH_THEME_IVALDI_PROMPT_SUFFIX=""
```

### Adding Colors

```bash
# Colorful timeline info
ZSH_THEME_IVALDI_PROMPT_PREFIX="%{$fg_bold[magenta]%}ivaldi:(%{$fg[yellow]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$fg[magenta]%})%{$reset_color%} "

# Different colors for different scenarios
ZSH_THEME_IVALDI_PROMPT_PREFIX="%{$fg_bold[green]%}timeline:(%{$fg[cyan]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$fg[green]%})%{$reset_color%} "
```

### Right-Side Prompt (RPROMPT)

```bash
# Show timeline info on the right side instead
RPROMPT='$(ivaldi_prompt_info)'

# Combine with other right-side info
RPROMPT='%D{%H:%M:%S} $(ivaldi_prompt_info)'
```

### Conditional Display

Only show timeline when it's not "main":

```bash
function ivaldi_prompt_info_conditional() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" && "$timeline" != "main" ]]; then
        echo "$ZSH_THEME_IVALDI_PROMPT_PREFIX$timeline$ZSH_THEME_IVALDI_PROMPT_SUFFIX"
    fi
}

# Use in your PROMPT
PROMPT='${ret_status} %{$fg[cyan]%}%c%{$reset_color%} $(git_prompt_info)$(ivaldi_prompt_info_conditional)'
```

## Performance Optimization

### Install jq for Faster JSON Parsing

The timeline detection uses JSON parsing. For better performance:

```bash
# macOS
brew install jq

# Ubuntu/Debian
sudo apt install jq

# CentOS/RHEL
sudo yum install jq
```

Without `jq`, the script uses grep/sed which is slightly slower but still very fast.

## Troubleshooting

### Timeline Shows as "unknown"
- Make sure you're in an Ivaldi repository (has `.ivaldi` directory)
- Check that `.ivaldi/timelines/config.json` exists
- Try running `ivaldi status` to verify the repository

### Plugin Not Loading
- Verify the plugin is in the correct directory: `$ZSH_CUSTOM/plugins/ivaldi/ivaldi.plugin.zsh`
- Make sure `ivaldi` is in your plugins list in ~/.zshrc
- Restart your terminal or run `source ~/.zshrc`

### No Timeline Showing
- Check if you're in an Ivaldi repository
- Try running the script directly: `./scripts/ivaldi-prompt.sh`
- Make sure your shell supports command substitution in prompts

### Slow Prompt Performance
- Install `jq` for faster JSON parsing (see above)
- The script is lightweight, but very large repositories might see slight delays
- Consider using conditional display to reduce checks

## Integration with Existing Themes

### Popular Oh My Zsh Themes

**Agnoster Theme:**
```bash
# Add to the end of your agnoster theme's build_prompt() function
prompt_ivaldi() {
  local timeline=$(ivaldi_timeline_info)
  if [[ -n "$timeline" ]]; then
    prompt_segment magenta white "ivaldi:$timeline"
  fi
}

# Add to build_prompt function
build_prompt() {
  RETVAL=$?
  prompt_status
  prompt_virtualenv
  prompt_context
  prompt_dir
  prompt_git
  prompt_ivaldi  # Add this line
  prompt_end
}
```

**Powerlevel10k Theme:**
```bash
# Add to ~/.p10k.zsh
POWERLEVEL9K_CUSTOM_IVALDI="ivaldi_timeline_info"
POWERLEVEL9K_CUSTOM_IVALDI_FOREGROUND="yellow"
POWERLEVEL9K_CUSTOM_IVALDI_BACKGROUND="magenta"

# Add to your prompt elements
POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(
  dir
  vcs
  custom_ivaldi  # Add this
  newline
  prompt_char
)
```

### Creating Custom Functions

```bash
# Simple timeline display
my_ivaldi_info() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" ]]; then
        echo " [$timeline]"
    fi
}

# Timeline with status indicators
my_ivaldi_detailed() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" ]]; then
        # You could add status checks here
        # local status=$(ivaldi status --porcelain 2>/dev/null)
        echo " ivaldi:$timeline"
    fi
}

# Use in your prompt
PS1='${debian_chroot:+($debian_chroot)}\u@\h:\w$(my_ivaldi_info)\$ '
```

## Updating

When you update Ivaldi, also update your shell integration:

```bash
# Update the prompt scripts
make install  # or make dev-install

# Update Oh My Zsh plugin
./scripts/quick-setup.sh

# Reload your shell
source ~/.zshrc
```

## File Locations

After installation, files are located at:

**System install:**
- Plugin: `/usr/local/share/ivaldi/scripts/oh-my-zsh-plugin/`
- Scripts: `/usr/local/share/ivaldi/scripts/`

**User install:**
- Plugin: `~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/`  
- Scripts: `~/.local/share/ivaldi/scripts/`

**Oh My Zsh custom:**
- Plugin: `~/.oh-my-zsh/custom/plugins/ivaldi/`
- Theme: `~/.oh-my-zsh/custom/themes/robbyrussell-ivaldi.zsh-theme`

---

**Ready to see your timeline in every prompt?** Run `./scripts/quick-setup.sh` and start experiencing Ivaldi's seamless shell integration!