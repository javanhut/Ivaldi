# Ivaldi Timeline Prompt Installation

This guide shows how to add Ivaldi timeline information to your shell prompt, similar to how Git shows branch information.

## What You Get

The prompt will show your current Ivaldi timeline like this:
```bash
user@host:~/project (timeline: main) $ 
user@host:~/project (timeline: feature-branch) $ 
```

## Installation

### For Bash

Add this to your `~/.bashrc` or `~/.bash_profile`:

```bash
# Source the Ivaldi prompt script
source /path/to/ivaldi/scripts/ivaldi-prompt.sh

# Add timeline info to your PS1 prompt
# This adds it to the end of your existing prompt
PS1="${PS1%\$ }\$(ivaldi_timeline_prompt)\$ "
```

#### Example: Complete Bash Setup

If you want a full prompt setup:

```bash
# Source the Ivaldi prompt script
source /path/to/ivaldi/scripts/ivaldi-prompt.sh

# Define colors (optional)
RED='\[\033[0;31m\]'
GREEN='\[\033[0;32m\]'
YELLOW='\[\033[1;33m\]'
BLUE='\[\033[0;34m\]'
PURPLE='\[\033[0;35m\]'
CYAN='\[\033[0;36m\]'
NC='\[\033[0m\]' # No Color

# Set prompt with Ivaldi timeline
PS1="${GREEN}\u@\h${NC}:${BLUE}\w${PURPLE}\$(ivaldi_timeline_prompt)${NC}\$ "
```

### For Zsh

Add this to your `~/.zshrc`:

```zsh
# Source the Ivaldi prompt script
source /path/to/ivaldi/scripts/ivaldi-prompt.sh

# Enable command substitution in prompts
setopt PROMPT_SUBST

# Add timeline info to your prompt
# This adds it to the end of your existing prompt
PROMPT="${PROMPT%% }%{\$(ivaldi_timeline_prompt_zsh)%} "
```

#### Example: Complete Zsh Setup

If you want a full prompt setup:

```zsh
# Source the Ivaldi prompt script
source /path/to/ivaldi/scripts/ivaldi-prompt.sh

# Enable command substitution in prompts
setopt PROMPT_SUBST

# Set prompt with Ivaldi timeline
PROMPT='%F{green}%n@%m%f:%F{blue}%~%f%F{magenta}$(ivaldi_timeline_prompt_zsh)%f$ '
```

### For Oh My Zsh Users

If you use Oh My Zsh, add this to your `~/.zshrc` after the Oh My Zsh source line:

```zsh
# Source the Ivaldi prompt script
source /path/to/ivaldi/scripts/ivaldi-prompt.sh

# Add to your existing theme
PROMPT="${PROMPT%% }%{\$(ivaldi_timeline_prompt_zsh)%} "

# Or if your theme uses RPROMPT (right side prompt)
RPROMPT="${RPROMPT}%{\$(ivaldi_timeline_prompt_zsh)%}"
```

## Customization

### Changing the Format

You can customize the format by editing the `ivaldi-prompt.sh` script or creating your own wrapper:

```bash
# Custom format function
my_ivaldi_prompt() {
    local timeline_info=$(ivaldi_timeline)
    if [[ -n "$timeline_info" ]]; then
        # Change format here - examples:
        echo " [$timeline_info]"           # [timeline: main]
        echo " 🌿$timeline_info"           # 🌿(timeline: main)  
        echo " on ${timeline_info#*: }"    # on main
    fi
}

# Use your custom function in PS1/PROMPT
PS1="${PS1%\$ }\$(my_ivaldi_prompt)\$ "
```

### Adding Colors

For colored output, you can modify the script or use your shell's color codes:

```bash
# Bash example with colors
ivaldi_colored_prompt() {
    local timeline_info=$(ivaldi_timeline)
    if [[ -n "$timeline_info" ]]; then
        echo " \[\033[0;35m\]$timeline_info\[\033[0m\]"  # Purple timeline
    fi
}
```

## Testing

After installation, reload your shell:

```bash
# For bash
source ~/.bashrc

# For zsh  
source ~/.zshrc
```

Then navigate to an Ivaldi repository and you should see the timeline in your prompt!

## Troubleshooting

### Timeline Shows as "unknown"
- Make sure you're in an Ivaldi repository (has `.ivaldi` directory)
- Check that `.ivaldi/timelines/config.json` exists

### Script Not Found
- Make sure the path in your shell config points to the correct location
- Verify the script is executable: `chmod +x ivaldi-prompt.sh`

### No Timeline Showing
- Check if you're in an Ivaldi repository
- Try running the script directly: `./ivaldi-prompt.sh`
- Make sure your shell supports command substitution in prompts

### Performance Issues
- The script is lightweight, but if you notice slowdowns, you can cache the result or check less frequently
- Consider using the `jq` command for faster JSON parsing (install with your package manager)

## Dependencies

- **Required**: None (uses standard shell tools)
- **Optional**: `jq` for faster JSON parsing (recommended)

Install jq:
```bash
# Ubuntu/Debian
sudo apt install jq

# macOS
brew install jq

# CentOS/RHEL
sudo yum install jq
```