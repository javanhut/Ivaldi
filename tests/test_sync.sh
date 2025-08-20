#!/bin/bash

# Test script to verify sync functionality
set -e

TEST_DIR="/tmp/test-sync-$(date +%s)"
REMOTE_REPO="https://github.com/yourusername/test-repo.git"  # Replace with actual test repo

echo "=== Testing Ivaldi Sync Functionality ==="
echo "Test directory: $TEST_DIR"

# Create test directory
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Initialize a new Ivaldi repository
echo "1. Initializing Ivaldi repository..."
../../build/ivaldi forge

# Add a remote portal (you'll need to replace with an actual test repo)
echo "2. Adding remote portal..."
../../build/ivaldi portal add origin "$REMOTE_REPO"

# List portals to verify
echo "3. Listing portals..."
../../build/ivaldi portal list

# Scout for changes (fetch without merging)
echo "4. Scouting for remote changes..."
../../build/ivaldi scout origin

# Perform sync (pull changes)
echo "5. Syncing with remote..."
../../build/ivaldi sync --pull origin

# Verify files were downloaded
echo "6. Checking downloaded files..."
ls -la

# Check status
echo "7. Checking status..."
../../build/ivaldi inspect

echo "=== Sync Test Complete ==="
echo "If files were downloaded and appear in the listing above, sync is working!"

# Cleanup
cd ..
rm -rf "$TEST_DIR"