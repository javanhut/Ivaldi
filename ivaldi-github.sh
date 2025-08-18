#!/bin/bash

# Ivaldi GitHub Management Script
# This script helps manage the Ivaldi codebase using ivaldi itself

IVALDI_BIN="./build/ivaldi"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

function print_status() {
    echo -e "${BLUE}=== Ivaldi Repository Status ===${NC}"
    $IVALDI_BIN status
}

function init_repo() {
    echo -e "${GREEN}Initializing Ivaldi repository...${NC}"
    
    # Build ivaldi if not exists
    if [ ! -f "$IVALDI_BIN" ]; then
        echo "Building ivaldi..."
        make build
    fi
    
    # Initialize repository if not already initialized
    if [ ! -d ".ivaldi" ]; then
        $IVALDI_BIN forge
        echo -e "${GREEN}Repository initialized!${NC}"
    else
        echo -e "${YELLOW}Repository already initialized${NC}"
    fi
}

function add_files() {
    echo -e "${GREEN}Gathering files to anvil...${NC}"
    
    # Add core directories
    $IVALDI_BIN gather cmd/
    $IVALDI_BIN gather core/
    $IVALDI_BIN gather forge/
    $IVALDI_BIN gather storage/
    $IVALDI_BIN gather sync/
    $IVALDI_BIN gather tests/
    $IVALDI_BIN gather ui/
    
    # Add root files
    $IVALDI_BIN gather go.mod
    $IVALDI_BIN gather go.sum
    $IVALDI_BIN gather Makefile
    
    echo -e "${GREEN}Files gathered!${NC}"
}

function create_seal() {
    local message="$1"
    
    if [ -z "$message" ]; then
        echo -e "${YELLOW}Please provide a seal message${NC}"
        echo "Usage: $0 seal \"Your message here\""
        return 1
    fi
    
    echo -e "${GREEN}Creating seal...${NC}"
    $IVALDI_BIN seal -m "$message"
}

function show_log() {
    echo -e "${BLUE}=== Seal History ===${NC}"
    $IVALDI_BIN log
}

function setup_github_remote() {
    local repo_url="$1"
    
    if [ -z "$repo_url" ]; then
        echo -e "${YELLOW}Please provide a GitHub repository URL${NC}"
        echo "Usage: $0 remote <github-url>"
        return 1
    fi
    
    echo -e "${GREEN}Setting up GitHub portal...${NC}"
    # Portal command not yet implemented, using git for now
    if [ ! -d ".git" ]; then
        git init
        git remote add origin "$repo_url"
        echo -e "${GREEN}GitHub remote configured!${NC}"
    else
        echo -e "${YELLOW}Git repository already exists${NC}"
    fi
}

function push_to_github() {
    echo -e "${GREEN}Pushing to GitHub...${NC}"
    
    # Since ivaldi sync is not yet implemented, use git
    if [ -d ".git" ]; then
        # Add all files
        git add .
        
        # Get latest seal message if available
        local message="Ivaldi update: $(date +%Y-%m-%d)"
        
        git commit -m "$message" 2>/dev/null || echo "No changes to commit"
        git push origin main
        
        echo -e "${GREEN}Pushed to GitHub!${NC}"
    else
        echo -e "${YELLOW}Git not initialized. Run: $0 remote <github-url>${NC}"
    fi
}

# Main script logic
case "$1" in
    init)
        init_repo
        ;;
    status)
        print_status
        ;;
    add)
        add_files
        ;;
    seal)
        create_seal "$2"
        ;;
    log)
        show_log
        ;;
    remote)
        setup_github_remote "$2"
        ;;
    push)
        push_to_github
        ;;
    help|*)
        echo "Ivaldi GitHub Management Script"
        echo ""
        echo "Usage: $0 <command> [args]"
        echo ""
        echo "Commands:"
        echo "  init          - Initialize ivaldi repository"
        echo "  status        - Show repository status"
        echo "  add           - Gather all project files to anvil"
        echo "  seal <msg>    - Create a seal with message"
        echo "  log           - Show seal history"
        echo "  remote <url>  - Setup GitHub remote repository"
        echo "  push          - Push changes to GitHub"
        echo "  help          - Show this help message"
        echo ""
        echo "Example workflow:"
        echo "  $0 init"
        echo "  $0 add"
        echo "  $0 seal \"Initial commit\""
        echo "  $0 remote https://github.com/yourusername/ivaldi.git"
        echo "  $0 push"
        ;;
esac