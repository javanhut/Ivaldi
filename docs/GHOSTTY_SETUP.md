# Ghostty Terminal Setup for Ivaldi Timeline

This guide shows you how to configure the Ghostty terminal to display Ivaldi timeline information in your prompt, similar to how Git branch information is displayed.

## Quick Setup

### For Bash Users

1. **Add to your `~/.bashrc`:**
```bash
# Ivaldi timeline prompt for Ghostty
IVALDI_PROMPT_SCRIPT="$HOME/.local/share/ivaldi/scripts/ivaldi-prompt.sh"
if [[ -f "$IVALDI_PROMPT_SCRIPT" ]]; then
    source "$IVALDI_PROMPT_SCRIPT"
    
    # Colors for Ghostty terminal
    RED='\[\033[0;31m\]'
    GREEN='\[\033[0;32m\]'
    YELLOW='\[\033[1;33m\]'
    BLUE='\[\033[0;34m\]'
    PURPLE='\[\033[0;35m\]'
    CYAN='\[\033[0;36m\]'
    NC='\[\033[0m\]' # No Color
    
    # Prompt with timeline info
    PS1="${GREEN}\u@\h${NC}:${BLUE}\w${PURPLE}\$(ivaldi_timeline_prompt)${NC}\$ "
fi
```

2. **Reload your shell:**
```bash
source ~/.bashrc
```

### For Zsh Users

1. **Install the Oh My Zsh plugin:**
```bash
# Create plugin directory
mkdir -p $ZSH_CUSTOM/plugins/ivaldi

# Copy the plugin file (adjust path as needed)
cp scripts/oh-my-zsh-plugin/ivaldi.plugin.zsh $ZSH_CUSTOM/plugins/ivaldi/
```

2. **Add to your `~/.zshrc`:**
```bash
plugins=(git ivaldi)
```

3. **Optional: Use the custom theme:**
```bash
# Copy the theme
cp scripts/oh-my-zsh-plugin/robbyrussell-ivaldi.zsh-theme $ZSH_CUSTOM/themes/

# Set in ~/.zshrc
ZSH_THEME="robbyrussell-ivaldi"
```

4. **Reload your shell:**
```bash
source ~/.zshrc
```

## Ghostty-Specific Configuration

Ghostty supports excellent color and Unicode rendering. You can enhance your Ivaldi timeline display:

### Enhanced Color Scheme

Add to your shell configuration:

```bash
# Ghostty-optimized Ivaldi colors
ZSH_THEME_IVALDI_PROMPT_PREFIX="%{$fg_bold[magenta]%}timeline:(%{$fg[cyan]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$fg[magenta]%})%{$reset_color%} "
```

For Bash:
```bash
PURPLE_BOLD='\[\033[1;35m\]'
CYAN_BRIGHT='\[\033[1;36m\]'
PS1="${GREEN}\u@\h${NC}:${BLUE}\w${PURPLE_BOLD}\$(ivaldi_timeline_prompt)${NC}\$ "
```

### Unicode Symbols

Ghostty handles Unicode well, so you can use symbols:

```bash
# With Unicode timeline symbol
ZSH_THEME_IVALDI_PROMPT_PREFIX="⏱ %{$fg[cyan]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$reset_color%} "
```

## Example Output

Once configured, your Ghostty terminal will show:

```bash
# In a regular directory
user@host:~/projects $ 

# In an Ivaldi repository
user@host:~/myproject timeline:(main) $ 

# On a feature timeline
user@host:~/myproject timeline:(feature-auth) $ 

# With Git and Ivaldi (using Oh My Zsh)
➜  myproject git:(main) timeline:(feature-auth) 
```

## Troubleshooting

### Timeline Shows as "unknown"
- Verify you're in an Ivaldi repository: `ls -la .ivaldi/`
- Check timeline config exists: `cat .ivaldi/timelines/config.json`
- Run `ivaldi status` to verify repository state

### Colors Not Working in Ghostty
- Ensure Ghostty color settings are enabled in config
- Check your `$TERM` variable: `echo $TERM`
- Try forcing color output: `export FORCE_COLOR=1`

### Prompt Not Updating
- Source your shell config: `source ~/.bashrc` or `source ~/.zshrc`
- Restart Ghostty terminal
- Check script paths are correct

## Advanced Configuration

### Conditional Timeline Display
Only show timeline when not on "main":

```bash
function ivaldi_timeline_conditional() {
    local timeline_info=$(ivaldi_timeline)
    if [[ -n "$timeline_info" && ! "$timeline_info" =~ "main" ]]; then
        echo " $timeline_info"
    fi
}

# Use in PS1
PS1="${GREEN}\u@\h${NC}:${BLUE}\w${PURPLE}\$(ivaldi_timeline_conditional)${NC}\$ "
```

### Right-Side Timeline (Zsh only)
```bash
# Add to ~/.zshrc after Oh My Zsh source
RPROMPT='$(ivaldi_prompt_info)'
```

### Performance Optimization
For better performance in large repositories:

```bash
# Install jq for faster JSON parsing
# macOS: brew install jq
# Linux: sudo apt install jq

# The Ivaldi scripts automatically use jq when available
```

## Integration with Ghostty Features

### Ghostty Keybindings
You can bind timeline switching to Ghostty keybindings in your Ghostty config:

```
keybind = ctrl+shift+t>1=new_tab,exec=ivaldi timeline switch main
keybind = ctrl+shift+t>2=new_tab,exec=ivaldi timeline switch feature
```

### Ghostty Tabs
Timeline information will appear in each tab, making it easy to track which timeline you're working on across multiple tabs.

## Files Modified/Created

This setup references:
- `scripts/ivaldi-prompt.sh` - Core prompt functions
- `scripts/oh-my-zsh-plugin/ivaldi.plugin.zsh` - Zsh plugin
- `scripts/oh-my-zsh-plugin/robbyrussell-ivaldi.zsh-theme` - Custom theme
- `.ivaldi/timelines/config.json` - Timeline configuration