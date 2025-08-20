# Ivaldi plugin for Oh My Zsh
# Provides Ivaldi timeline information for prompts and useful aliases

# Ivaldi timeline detection
function ivaldi_timeline_info() {
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
        echo "unknown"
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
    
    echo "$current_timeline"
}

# Timeline prompt info (similar to git_prompt_info)
function ivaldi_prompt_info() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" ]]; then
        echo "$ZSH_THEME_IVALDI_PROMPT_PREFIX$timeline$ZSH_THEME_IVALDI_PROMPT_SUFFIX"
    fi
}

# Timeline status (for more detailed info)
function ivaldi_prompt_status() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -z "$timeline" ]]; then
        return
    fi
    
    # You could add more status indicators here, like:
    # - Modified files count
    # - Files on anvil (staged)
    # - etc.
    
    echo "$ZSH_THEME_IVALDI_PROMPT_PREFIX$timeline$ZSH_THEME_IVALDI_PROMPT_SUFFIX"
}

# Default theme settings (can be overridden in themes)
ZSH_THEME_IVALDI_PROMPT_PREFIX=${ZSH_THEME_IVALDI_PROMPT_PREFIX-"(timeline: "}
ZSH_THEME_IVALDI_PROMPT_SUFFIX=${ZSH_THEME_IVALDI_PROMPT_SUFFIX-")"}

# Useful Ivaldi aliases
alias iva='ivaldi'
alias igather='ivaldi gather'
alias iseal='ivaldi seal'
alias istatus='ivaldi status'
alias itimeline='ivaldi timeline'
alias iswitch='ivaldi timeline switch'
alias icreate='ivaldi timeline create'
alias ilist='ivaldi timeline list'
alias imirror='ivaldi mirror'
alias idownload='ivaldi download'
alias iforge='ivaldi forge'
alias ijump='ivaldi jump'
alias ihistory='ivaldi history'

# Advanced aliases with common patterns
alias igatherall='ivaldi gather .'
alias isealmsg='ivaldi seal'
alias istatusshort='ivaldi status --short'

# Git-style shortcuts for Ivaldi
alias iadd='ivaldi gather'
alias icommit='ivaldi seal'
alias ibranch='ivaldi timeline'
alias icheckout='ivaldi timeline switch'

# Check if the user has Ivaldi installed
if ! command -v ivaldi >/dev/null 2>&1; then
    echo "Ivaldi not found in PATH. Install it from: https://github.com/javanhut/Ivaldi"
fi