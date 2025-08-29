#!/bin/bash

# Ghostty Terminal Setup Script for Ivaldi Timeline
# This script configures Ghostty terminal to display Ivaldi timeline information

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IVALDI_SCRIPTS_DIR="$HOME/.local/share/ivaldi/scripts"

echo "Setting up Ivaldi timeline display for Ghostty terminal..."

# Create scripts directory
mkdir -p "$IVALDI_SCRIPTS_DIR"

# Copy the prompt script
echo "Installing Ivaldi prompt script..."
cp "$SCRIPT_DIR/ivaldi-prompt.sh" "$IVALDI_SCRIPTS_DIR/"
chmod +x "$IVALDI_SCRIPTS_DIR/ivaldi-prompt.sh"

setup_bash() {
    echo "Configuring Bash for Ghostty..."
    
    local bashrc="$HOME/.bashrc"
    local config_block="
# Ivaldi timeline prompt for Ghostty (added by ghostty-setup.sh)
IVALDI_PROMPT_SCRIPT=\"\$HOME/.local/share/ivaldi/scripts/ivaldi-prompt.sh\"
if [[ -f \"\$IVALDI_PROMPT_SCRIPT\" ]]; then
    source \"\$IVALDI_PROMPT_SCRIPT\"
    
    # Ghostty-optimized colors
    RED='\\[\\033[0;31m\\]'
    GREEN='\\[\\033[0;32m\\]'
    YELLOW='\\[\\033[1;33m\\]'
    BLUE='\\[\\033[0;34m\\]'
    PURPLE='\\[\\033[0;35m\\]'
    CYAN='\\[\\033[0;36m\\]'
    NC='\\[\\033[0m\\]' # No Color
    
    # Enhanced prompt with timeline info
    PS1=\"\${GREEN}\\u@\\h\${NC}:\${BLUE}\\w\${PURPLE}\\$(ivaldi_timeline_prompt)\${NC}\\$ \"
fi
# End Ivaldi configuration"

    # Check if already configured
    if grep -q "Ivaldi timeline prompt for Ghostty" "$bashrc" 2>/dev/null; then
        echo "WARNING: Bash already configured. Skipping..."
        return
    fi
    
    # Backup existing bashrc
    if [[ -f "$bashrc" ]]; then
        cp "$bashrc" "$bashrc.ivaldi-backup-$(date +%Y%m%d-%H%M%S)"
        echo "Backed up existing ~/.bashrc"
    fi
    
    # Add configuration
    echo "$config_block" >> "$bashrc"
    echo "Added Ivaldi configuration to ~/.bashrc"
}

setup_zsh() {
    echo "Configuring Zsh for Ghostty..."
    
    local zshrc="$HOME/.zshrc"
    
    # Check if Oh My Zsh is installed
    if [[ -d "$HOME/.oh-my-zsh" ]] || [[ -n "$ZSH" ]]; then
        setup_oh_my_zsh
    else
        setup_plain_zsh
    fi
}

setup_oh_my_zsh() {
    echo "Oh My Zsh detected - installing plugin..."
    
    local custom_dir="${ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}"
    local plugin_dir="$custom_dir/plugins/ivaldi"
    local zshrc="$HOME/.zshrc"
    
    # Create plugin directory
    mkdir -p "$plugin_dir"
    
    # Copy plugin file
    cp "$SCRIPT_DIR/oh-my-zsh-plugin/ivaldi.plugin.zsh" "$plugin_dir/"
    echo "Installed Ivaldi plugin to $plugin_dir"
    
    # Update .zshrc plugins
    if grep -q "plugins=.*ivaldi" "$zshrc" 2>/dev/null; then
        echo "WARNING: Plugin already added to ~/.zshrc"
    else
        # Backup existing zshrc
        if [[ -f "$zshrc" ]]; then
            cp "$zshrc" "$zshrc.ivaldi-backup-$(date +%Y%m%d-%H%M%S)"
            echo "Backed up existing ~/.zshrc"
        fi
        
        # Add ivaldi to plugins
        if grep -q "^plugins=" "$zshrc" 2>/dev/null; then
            # Replace existing plugins line
            sed -i.tmp 's/^plugins=(\(.*\))/plugins=(\1 ivaldi)/' "$zshrc"
            echo "Added 'ivaldi' to existing plugins list"
        else
            # Add plugins line
            echo "" >> "$zshrc"
            echo "# Ivaldi plugin (added by ghostty-setup.sh)" >> "$zshrc"
            echo "plugins=(git ivaldi)" >> "$zshrc"
            echo "Added plugins configuration to ~/.zshrc"
        fi
    fi
    
    # Optional: Install custom theme
    local theme_dir="$custom_dir/themes"
    mkdir -p "$theme_dir"
    cp "$SCRIPT_DIR/oh-my-zsh-plugin/robbyrussell-ivaldi.zsh-theme" "$theme_dir/"
    echo "Installed custom Ghostty-optimized theme (optional)"
    echo "   To use it, set: ZSH_THEME=\"robbyrussell-ivaldi\" in ~/.zshrc"
}

setup_plain_zsh() {
    echo "Configuring plain Zsh..."
    
    local zshrc="$HOME/.zshrc"
    local config_block="
# Ivaldi timeline prompt for Ghostty (added by ghostty-setup.sh)
IVALDI_PROMPT_SCRIPT=\"\$HOME/.local/share/ivaldi/scripts/ivaldi-prompt.sh\"
if [[ -f \"\$IVALDI_PROMPT_SCRIPT\" ]]; then
    source \"\$IVALDI_PROMPT_SCRIPT\"
    
    # Ghostty-optimized prompt with colors
    autoload -U colors && colors
    
    ivaldi_zsh_prompt() {
        local timeline_info=\$(ivaldi_timeline_prompt_zsh)
        echo \"\$timeline_info\"
    }
    
    # Enhanced prompt with timeline info
    PROMPT='%F{green}%n@%m%f:%F{blue}%~%f%F{magenta}\$(ivaldi_zsh_prompt)%f%# '
fi
# End Ivaldi configuration"

    # Check if already configured
    if grep -q "Ivaldi timeline prompt for Ghostty" "$zshrc" 2>/dev/null; then
        echo "WARNING: Zsh already configured. Skipping..."
        return
    fi
    
    # Backup existing zshrc
    if [[ -f "$zshrc" ]]; then
        cp "$zshrc" "$zshrc.ivaldi-backup-$(date +%Y%m%d-%H%M%S)"
        echo "Backed up existing ~/.zshrc"
    fi
    
    # Add configuration
    echo "$config_block" >> "$zshrc"
    echo "Added Ivaldi configuration to ~/.zshrc"
}

# Detect shell and configure accordingly
detect_shell() {
    if [[ -n "$ZSH_VERSION" ]] || [[ "$SHELL" == */zsh ]]; then
        echo "zsh"
    elif [[ -n "$BASH_VERSION" ]] || [[ "$SHELL" == */bash ]]; then
        echo "bash"
    else
        echo "unknown"
    fi
}

# Script execution starts here
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    SHELL_TYPE=$(detect_shell)

    case "$SHELL_TYPE" in
        "bash")
            setup_bash
            ;;
        "zsh")
            setup_zsh
            ;;
        *)
        echo "WARNING: Unknown shell. Please manually configure using the documentation."
        echo "See: docs/GHOSTTY_SETUP.md"
            exit 1
            ;;
    esac

    echo "Ghostty terminal setup complete!"
    echo ""
    echo "Please restart your terminal or run:"
    case "$SHELL_TYPE" in
        "bash")
            echo "   source ~/.bashrc"
            ;;
        "zsh")
            echo "   source ~/.zshrc"
            ;;
    esac
    echo ""
    echo "Navigate to an Ivaldi repository to see timeline information!"
fi