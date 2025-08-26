#!/bin/bash
# Demonstration script for Ivaldi Jump Improvements
# This script shows the new workspace-preserving jump functionality

echo "=== Ivaldi Jump Improvements Demo ==="
echo ""

# Check if we're in an Ivaldi repository
if [ ! -d ".ivaldi" ]; then
    echo "Initializing test repository..."
    ./ivaldi forge .
    echo ""
fi

echo "1. Creating initial content..."
echo "Initial content" > test-file.txt
echo "Another file" > another-file.txt
./ivaldi gather test-file.txt another-file.txt
./ivaldi seal "Initial version with two files"
echo ""

echo "2. Making changes for second seal..."
echo "Modified content" > test-file.txt
echo "New file for second seal" > second-file.txt
./ivaldi gather test-file.txt second-file.txt
./ivaldi seal "Second version with modifications"
echo ""

echo "3. Making uncommitted changes..."
echo "Uncommitted changes - should be preserved!" > test-file.txt
echo "Brand new uncommitted file" > uncommitted-file.txt
echo ""

echo "Current workspace state:"
ls -la *.txt
echo ""
echo "Content of test-file.txt:"
cat test-file.txt
echo ""

echo "4. Jumping to previous seal WITH workspace preservation..."
echo "Command: ./ivaldi jump --preserve \$(./ivaldi log | grep 'Initial version' | head -1 | awk '{print \$1}')"
PREV_SEAL=$(./ivaldi log | grep "Initial version" | head -1 | awk '{print $1}')
echo "Previous seal: $PREV_SEAL"
./ivaldi jump --preserve "$PREV_SEAL"
echo ""

echo "After jump - workspace state (uncommitted changes should be preserved):"
ls -la *.txt 2>/dev/null || echo "No .txt files found"
echo ""
if [ -f "test-file.txt" ]; then
    echo "Content of test-file.txt (should show uncommitted changes):"
    cat test-file.txt
    echo ""
fi

if [ -f "uncommitted-file.txt" ]; then
    echo "Content of uncommitted-file.txt (should be preserved):"
    cat uncommitted-file.txt
    echo ""
fi

echo "5. Jumping back to most recent position..."
./ivaldi jump back
echo ""

echo "After jumping back - workspace state:"
ls -la *.txt
echo ""

echo "6. Demonstrating force jump (overwrites local changes)..."
echo "Making new uncommitted changes..."
echo "These changes will be overwritten" > test-file.txt
echo ""

echo "Before force jump:"
cat test-file.txt
echo ""

echo "Force jumping to previous seal..."
./ivaldi jump --force "$PREV_SEAL"
echo ""

echo "After force jump (changes should be overwritten):"
if [ -f "test-file.txt" ]; then
    cat test-file.txt
else
    echo "test-file.txt not found (expected if jumping to a seal without this file)"
fi
echo ""

echo "=== Demo Complete ==="
echo ""
echo "Key features demonstrated:"
echo "✅ Workspace preservation during jumps (--preserve flag)"
echo "✅ Jump history and jump back functionality"  
echo "✅ Force jump to override local changes (--force flag)"
echo "✅ Proper handling of uncommitted files"
echo ""
echo "Available commands:"
echo "  ivaldi jump --preserve <reference>  # Preserve uncommitted changes"
echo "  ivaldi jump --force <reference>     # Force overwrite local changes"
echo "  ivaldi jump back                    # Return to previous position"
echo "  ivaldi jump --no-history <ref>      # Don't save to jump history"