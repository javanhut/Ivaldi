#!/bin/bash

# Ivaldi Timeline Prompt
# Displays current Ivaldi timeline in shell prompt similar to Git branch display

ivaldi_timeline() {
    # Check if we're in an Ivaldi repository
    local repo_root=""
    local dir="$PWD"
    
    # Walk up the directory tree looking for .ivaldi directory
    while [[ "$dir" != "/" ]]; do
        if [[ -d "$dir/.ivaldi" ]]; then
            repo_root="$dir"
            break
        fi
        dir="$(dirname "$dir")"
    done
    
    # If not in an Ivaldi repo, return nothing
    if [[ -z "$repo_root" ]]; then
        return
    fi
    
    # Path to timeline config
    local timeline_config="$repo_root/.ivaldi/timelines/config.json"
    
    # Check if timeline config exists
    if [[ ! -f "$timeline_config" ]]; then
        echo "(timeline: unknown)"
        return
    fi
    
    # Extract current timeline using jq if available, otherwise use grep/sed
    local current_timeline=""
    if command -v jq >/dev/null 2>&1; then
        current_timeline=$(jq -r '.current // "main"' "$timeline_config" 2>/dev/null)
    else
        # Fallback: use grep and sed to extract timeline
        current_timeline=$(grep -o '"current"[[:space:]]*:[[:space:]]*"[^"]*"' "$timeline_config" 2>/dev/null | sed 's/.*"current"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
    fi
    
    # Default to "main" if we couldn't extract the timeline
    if [[ -z "$current_timeline" || "$current_timeline" == "null" ]]; then
        current_timeline="main"
    fi
    
    # Format and return the timeline
    echo "(timeline: $current_timeline)"
}

# For bash prompt integration
ivaldi_timeline_prompt() {
    local timeline_info=$(ivaldi_timeline)
    if [[ -n "$timeline_info" ]]; then
        echo " $timeline_info"
    fi
}

# For zsh prompt integration  
ivaldi_timeline_prompt_zsh() {
    local timeline_info=$(ivaldi_timeline)
    if [[ -n "$timeline_info" ]]; then
        echo " $timeline_info"
    fi
}

# Direct call - just show the timeline
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    ivaldi_timeline
fi