#!/bin/bash

# Quick setup script for Ivaldi timeline prompt with Oh My Zsh
# This script will set up the timeline display similar to Git branch info

set -e

echo "🔧 Setting up Ivaldi timeline prompt for Oh My Zsh..."

# Check if Oh My Zsh is installed
if [[ ! -d "$HOME/.oh-my-zsh" ]]; then
    echo "❌ Oh My Zsh not found. Please install it first."
    exit 1
fi

# Set ZSH_CUSTOM if not set
if [[ -z "$ZSH_CUSTOM" ]]; then
    ZSH_CUSTOM="$HOME/.oh-my-zsh/custom"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "📁 Creating plugin directory..."
PLUGIN_DIR="$ZSH_CUSTOM/plugins/ivaldi"
mkdir -p "$PLUGIN_DIR"

echo "📋 Copying plugin files..."
cp "$SCRIPT_DIR/oh-my-zsh-plugin/ivaldi.plugin.zsh" "$PLUGIN_DIR/"

echo "🎨 Copying theme..."
THEME_DIR="$ZSH_CUSTOM/themes"
mkdir -p "$THEME_DIR"
cp "$SCRIPT_DIR/oh-my-zsh-plugin/robbyrussell-ivaldi.zsh-theme" "$THEME_DIR/"

echo "⚙️  Updating ~/.zshrc..."
ZSHRC="$HOME/.zshrc"

# Backup .zshrc
cp "$ZSHRC" "$ZSHRC.backup.$(date +%Y%m%d_%H%M%S)"

# Add ivaldi to plugins if not already there
if ! grep -q "plugins=.*ivaldi" "$ZSHRC"; then
    if grep -q "^plugins=(" "$ZSHRC"; then
        # Add ivaldi to existing plugins
        sed -i.tmp 's/^plugins=(\([^)]*\))/plugins=(\1 ivaldi)/' "$ZSHRC"
        rm -f "$ZSHRC.tmp"
    else
        # Add new plugins line
        echo "" >> "$ZSHRC"
        echo "# Added by Ivaldi setup" >> "$ZSHRC"
        echo "plugins=(git ivaldi)" >> "$ZSHRC"
    fi
fi

# Switch to the Ivaldi theme
if grep -q "^ZSH_THEME=" "$ZSHRC"; then
    sed -i.tmp 's/^ZSH_THEME=.*/ZSH_THEME="robbyrussell-ivaldi"/' "$ZSHRC"
    rm -f "$ZSHRC.tmp"
else
    echo 'ZSH_THEME="robbyrussell-ivaldi"' >> "$ZSHRC"
fi

echo ""
echo "✅ Setup complete!"
echo ""
echo "Your prompt will now show:"
echo "  ➜  project git:(main) ivaldi:(timeline-name)"
echo ""
echo "🔄 Restart your terminal or run: source ~/.zshrc"
echo ""
echo "🎯 Available aliases:"
echo "  iva → ivaldi"
echo "  igather → ivaldi gather"
echo "  iseal → ivaldi seal"
echo "  iswitch → ivaldi timeline switch"
echo "  And many more!"
echo ""
echo "📚 For more info: $SCRIPT_DIR/oh-my-zsh-plugin/README.md"