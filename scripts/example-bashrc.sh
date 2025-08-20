#!/bin/bash
# Example ~/.bashrc addition for Ivaldi timeline prompt

# Source the Ivaldi prompt script (update path as needed)
IVALDI_PROMPT_SCRIPT="/usr/local/share/ivaldi/scripts/ivaldi-prompt.sh"
if [[ -f "$IVALDI_PROMPT_SCRIPT" ]]; then
    source "$IVALDI_PROMPT_SCRIPT"
fi

# Simple colored prompt with Ivaldi timeline
if command -v ivaldi_timeline_prompt >/dev/null 2>&1; then
    # Colors
    RED='\[\033[0;31m\]'
    GREEN='\[\033[0;32m\]'
    YELLOW='\[\033[1;33m\]'
    BLUE='\[\033[0;34m\]'
    PURPLE='\[\033[0;35m\]'
    CYAN='\[\033[0;36m\]'
    NC='\[\033[0m\]' # No Color
    
    # Prompt: user@host:path (timeline: name) $ 
    PS1="${GREEN}\u@\h${NC}:${BLUE}\w${PURPLE}\$(ivaldi_timeline_prompt)${NC}\$ "
else
    # Fallback prompt if Ivaldi script not available
    PS1='\u@\h:\w\$ '
fi