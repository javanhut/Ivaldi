#!/bin/bash

# Pre-commit hook for Ivaldi
# This script runs before each commit to ensure code quality

set -e

echo "🔍 Running pre-commit checks..."

# Check if Go is installed
if ! command -v go &> /dev/null; then
    echo "❌ Go is not installed. Please install Go 1.24 or later."
    exit 1
fi

# Check Go version
GO_VERSION=$(go version | awk '{print $3}' | sed 's/go//')
REQUIRED_VERSION="1.24"

if [ "$(printf '%s\n' "$REQUIRED_VERSION" "$GO_VERSION" | sort -V | head -n1)" != "$REQUIRED_VERSION" ]; then
    echo "❌ Go version $GO_VERSION is too old. Required: $REQUIRED_VERSION or later."
    exit 1
fi

echo "✅ Go version check passed: $GO_VERSION"

# Format code
echo "🎨 Formatting code..."
go fmt ./...
echo "✅ Code formatting complete"

# Run linter
echo "🔍 Running linter..."
if command -v golangci-lint &> /dev/null; then
    golangci-lint run
    echo "✅ Linting complete"
else
    echo "⚠️  golangci-lint not found, skipping linting"
    echo "   Install with: go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest"
fi

# Run tests
echo "🧪 Running tests..."
go test -v ./...
echo "✅ Tests passed"

# Check for TODO comments
echo "📝 Checking for TODO comments..."
TODO_COUNT=$(grep -r "TODO" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$TODO_COUNT" -gt 0 ]; then
    echo "⚠️  Found $TODO_COUNT TODO comments:"
    grep -r "TODO" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider addressing these before committing"
fi

# Check for FIXME comments
echo "🔧 Checking for FIXME comments..."
FIXME_COUNT=$(grep -r "FIXME" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$FIXME_COUNT" -gt 0 ]; then
    echo "⚠️  Found $FIXME_COUNT FIXME comments:"
    grep -r "FIXME" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider addressing these before committing"
fi

# Check for panic statements
echo "🚨 Checking for panic statements..."
PANIC_COUNT=$(grep -r "panic(" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$PANIC_COUNT" -gt 0 ]; then
    echo "❌ Found $PANIC_COUNT panic statements:"
    grep -r "panic(" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md
    echo "   Panic statements should be replaced with proper error handling"
    exit 1
fi

# Check for direct fmt.Printf usage
echo "📊 Checking for direct fmt.Printf usage..."
PRINTF_COUNT=$(grep -r "fmt\.Printf" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | wc -l)
if [ "$PRINTF_COUNT" -gt 0 ]; then
    echo "⚠️  Found $PRINTF_COUNT fmt.Printf usages:"
    grep -r "fmt\.Printf" . --exclude-dir=.git --exclude-dir=.ivaldi --exclude=*.md | head -5
    echo "   Consider using the logging package instead"
fi

# Build the project
echo "🏗️  Building project..."
make build
echo "✅ Build successful"

echo "🎉 All pre-commit checks passed!"
echo "�� Ready to commit!"
