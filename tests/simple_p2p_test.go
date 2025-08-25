package tests

import (
	"os"
	"testing"

	"ivaldi/forge"
)

// TestP2PInitialization tests basic P2P initialization
func TestP2PInitialization(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-init-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer repo.Close()

	// Test that P2P methods don't panic
	status := repo.GetP2PStatus()
	if status == nil {
		t.Error("P2P status should not be nil")
	}

	config := repo.GetP2PConfig()
	if config == nil {
		t.Error("P2P config should not be nil")
	}

	isRunning := repo.IsP2PRunning()
	if isRunning {
		t.Error("P2P should not be running initially")
	}

	t.Logf("P2P status: running=%v", status.Running)
	t.Logf("P2P config: port=%d, auto-sync=%v", config.Port, config.AutoSyncEnabled)

	t.Log("P2P initialization test passed!")
}

// TestP2PConfigOperations tests P2P configuration operations
func TestP2PConfigOperations(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-config-ops-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer repo.Close()

	// Test config access (should not fail even if P2P manager is not fully initialized)
	config := repo.GetP2PConfig()
	if config == nil {
		t.Fatal("P2P config should not be nil")
	}

	originalPort := config.Port
	originalAutoSync := config.AutoSyncEnabled

	t.Logf("Original config: port=%d, auto-sync=%v", originalPort, originalAutoSync)

	// Test getting status (should not panic)
	status := repo.GetP2PStatus()
	if status == nil {
		t.Error("P2P status should not be nil")
	}

	t.Logf("P2P status: running=%v, node_id=%s", status.Running, status.NodeID)

	t.Log("P2P config operations test passed!")
}
