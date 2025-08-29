#!/bin/bash

# Pre-commit hook for Ivaldi
# This script runs before each commit to ensure code quality

set -e

echo " Running pre-commit checks..."

# Check if Go is installed
if ! command -v go &> /dev/null; then
    echo "ERROR: Go is not installed. Please install Go 1.24 or later."
    exit 1
fi

# Check Go version
GO_VERSION=$(go version | awk '{print $3}' | sed 's/go//')
REQUIRED_VERSION="1.24"

# POSIX-safe semantic version comparison
compare_versions() {
    local required="$1"
    local current="$2"
    
    # Parse required version (major.minor.patch)
    local req_major="${required%%.*}"
    local req_rest="${required#*.}"
    local req_minor="${req_rest%%.*}"
    local req_patch="${req_rest#*.}"
    
    # Default missing components to 0
    [ "$req_minor" = "$required" ] && req_minor=0
    [ "$req_patch" = "$req_rest" ] && req_patch=0
    
    # Parse current version (major.minor.patch)
    local cur_major="${current%%.*}"
    local cur_rest="${current#*.}"
    local cur_minor="${cur_rest%%.*}"
    local cur_patch="${cur_rest#*.}"
    
    # Default missing components to 0
    [ "$cur_minor" = "$current" ] && cur_minor=0
    [ "$cur_patch" = "$cur_rest" ] && cur_patch=0
    
    # Compare major version
    if [ "$cur_major" -lt "$req_major" ]; then
        return 1  # current < required
    elif [ "$cur_major" -gt "$req_major" ]; then
        return 0  # current > required
    fi
    
    # Major versions equal, compare minor
    if [ "$cur_minor" -lt "$req_minor" ]; then
        return 1  # current < required
    elif [ "$cur_minor" -gt "$req_minor" ]; then
        return 0  # current > required
    fi
    
    # Major and minor equal, compare patch
    if [ "$cur_patch" -lt "$req_patch" ]; then
        return 1  # current < required
    fi
    
    return 0  # current >= required
}

if ! compare_versions "$REQUIRED_VERSION" "$GO_VERSION"; then
    echo "ERROR: Go version $GO_VERSION is too old. Required: $REQUIRED_VERSION or later."
    exit 1
fi

echo " Go version check passed: $GO_VERSION"

# Format code
echo " Formatting code..."
go fmt ./...
echo " Code formatting complete"

# Run linter
echo " Running linter..."
if command -v golangci-lint &> /dev/null; then
    golangci-lint run
    echo " Linting complete"
else
    echo "WARNING:  golangci-lint not found, skipping linting"
    echo "   Install with: go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest"
fi

# Run tests
echo "🧪 Running tests..."
go test -v ./...
echo " Tests passed"

# Check for TODO comments
echo " Checking for TODO comments..."
TODO_COUNT=$(grep -r "TODO" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$TODO_COUNT" -gt 0 ]; then
    echo "WARNING:  Found $TODO_COUNT TODO comments:"
    grep -r "TODO" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider addressing these before committing"
fi

# Check for FIXME comments
echo " Checking for FIXME comments..."
FIXME_COUNT=$(grep -r "FIXME" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$FIXME_COUNT" -gt 0 ]; then
    echo "WARNING:  Found $FIXME_COUNT FIXME comments:"
    grep -r "FIXME" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider addressing these before committing"
fi

# Check for panic statements
echo "🚨 Checking for panic statements..."
PANIC_COUNT=$(grep -r "panic(" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$PANIC_COUNT" -gt 0 ]; then
    echo "ERROR: Found $PANIC_COUNT panic statements:"
    grep -r "panic(" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md
    echo "   Panic statements should be replaced with proper error handling"
    exit 1
fi

# Check for direct fmt.Printf usage
echo " Checking for direct fmt.Printf usage..."
PRINTF_COUNT=$(grep -r "fmt\.Printf" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$PRINTF_COUNT" -gt 0 ]; then
    echo "WARNING:  Found $PRINTF_COUNT fmt.Printf usages:"
    grep -r "fmt\.Printf" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider using the logging package instead"
fi

# Build the project
echo "🏗️  Building project..."
make build
echo " Build successful"

echo " All pre-commit checks passed!"
echo "�� Ready to commit!"
