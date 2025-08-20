#!/bin/bash

# Ivaldi Oh My Zsh Plugin Installer

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m' # No Color

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if Oh My Zsh is installed
if [[ ! -d "$HOME/.oh-my-zsh" ]]; then
    log_error "Oh My Zsh not found. Please install it first:"
    echo "  sh -c \"\$(curl -fsSL https://raw.github.com/ohmyzsh/ohmyzsh/master/tools/install.sh)\""
    exit 1
fi

# Check if ZSH_CUSTOM is set, default to standard location
if [[ -z "$ZSH_CUSTOM" ]]; then
    ZSH_CUSTOM="$HOME/.oh-my-zsh/custom"
fi

log_info "Installing Ivaldi plugin for Oh My Zsh..."

# Create plugin directory
PLUGIN_DIR="$ZSH_CUSTOM/plugins/ivaldi"
mkdir -p "$PLUGIN_DIR"

# Get the script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Copy plugin file
if [[ -f "$SCRIPT_DIR/ivaldi.plugin.zsh" ]]; then
    cp "$SCRIPT_DIR/ivaldi.plugin.zsh" "$PLUGIN_DIR/"
    log_success "Plugin file copied to $PLUGIN_DIR"
else
    log_error "Plugin file not found at $SCRIPT_DIR/ivaldi.plugin.zsh"
    exit 1
fi

# Copy theme file
THEME_DIR="$ZSH_CUSTOM/themes"
mkdir -p "$THEME_DIR"

if [[ -f "$SCRIPT_DIR/robbyrussell-ivaldi.zsh-theme" ]]; then
    cp "$SCRIPT_DIR/robbyrussell-ivaldi.zsh-theme" "$THEME_DIR/"
    log_success "Theme file copied to $THEME_DIR"
else
    log_warning "Theme file not found, skipping theme installation"
fi

# Check if plugin is already in .zshrc
ZSHRC="$HOME/.zshrc"
if grep -q "plugins=.*ivaldi" "$ZSHRC"; then
    log_success "Ivaldi plugin already added to ~/.zshrc"
else
    log_info "Adding Ivaldi plugin to ~/.zshrc..."
    
    # Backup .zshrc
    cp "$ZSHRC" "$ZSHRC.backup.$(date +%Y%m%d_%H%M%S)"
    log_info "Backed up ~/.zshrc"
    
    # Add ivaldi to plugins
    if grep -q "^plugins=(" "$ZSHRC"; then
        # Replace existing plugins line
        sed -i.tmp 's/^plugins=(\([^)]*\))/plugins=(\1 ivaldi)/' "$ZSHRC"
        rm -f "$ZSHRC.tmp"
        log_success "Added 'ivaldi' to existing plugins list"
    else
        # Add plugins line
        echo "" >> "$ZSHRC"
        echo "# Added by Ivaldi installer" >> "$ZSHRC"
        echo "plugins=(git ivaldi)" >> "$ZSHRC"
        log_success "Added plugins line with git and ivaldi"
    fi
fi

echo ""
log_success "Ivaldi Oh My Zsh plugin installed successfully!"
echo ""
echo -e "${PURPLE}Next steps:${NC}"
echo "1. Restart your terminal or run: source ~/.zshrc"
echo "2. Navigate to an Ivaldi repository to see timeline info"
echo ""
echo -e "${PURPLE}Optional - Use the Ivaldi-enhanced theme:${NC}"
echo "Change ZSH_THEME in ~/.zshrc to:"
echo "  ZSH_THEME=\"robbyrussell-ivaldi\""
echo ""
echo -e "${PURPLE}Available aliases:${NC}"
echo "  iva, igather, iseal, istatus, itimeline, iswitch, etc."
echo ""
echo -e "${PURPLE}For more info:${NC}"
echo "  cat $SCRIPT_DIR/README.md"