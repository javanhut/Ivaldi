#!/bin/zsh
# Example ~/.zshrc addition for Ivaldi timeline prompt

# Source the Ivaldi prompt script (update path as needed)
IVALDI_PROMPT_SCRIPT="/usr/local/share/ivaldi/scripts/ivaldi-prompt.sh"
if [[ -f "$IVALDI_PROMPT_SCRIPT" ]]; then
    source "$IVALDI_PROMPT_SCRIPT"
fi

# Enable command substitution in prompts
setopt PROMPT_SUBST

# Simple colored prompt with Ivaldi timeline
if command -v ivaldi_timeline_prompt_zsh >/dev/null 2>&1; then
    # Prompt: user@host:path (timeline: name) $ 
    PROMPT='%F{green}%n@%m%f:%F{blue}%~%f%F{magenta}$(ivaldi_timeline_prompt_zsh)%f$ '
else
    # Fallback prompt if Ivaldi script not available
    PROMPT='%n@%m:%~$ '
fi

# Alternative: Add to right side of prompt
# RPROMPT='%F{magenta}$(ivaldi_timeline_prompt_zsh)%f'