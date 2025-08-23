package tests

import (
	"os"
	"testing"

	"ivaldi/forge"
)

// TestP2PManagerCreation tests P2P manager creation directly
func TestP2PManagerCreation(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-debug-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	t.Logf("Creating repository in: %s", tempDir)

	// Initialize repository step by step to see where P2P fails
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer repo.Close()

	t.Log("Repository initialized successfully")

	// Check if P2P manager exists
	status := repo.GetP2PStatus()
	t.Logf("P2P Status: %+v", status)

	config := repo.GetP2PConfig()
	t.Logf("P2P Config: %+v", config)

	// Check if UpdateP2PConfig works (this is what fails in the robust test)
	config.Port = 9999
	err = repo.UpdateP2PConfig(config)
	if err != nil {
		t.Errorf("UpdateP2PConfig failed: %v", err)
		
		// The issue is likely that p2pMgr is nil
		// Let's check the repo struct directly by trying to start P2P
		t.Log("Attempting to start P2P to see if manager exists...")
		startErr := repo.StartP2P()
		if startErr != nil {
			t.Logf("StartP2P also failed: %v", startErr)
		} else {
			t.Log("StartP2P succeeded - P2P manager exists!")
			repo.StopP2P()
		}
	} else {
		t.Log("UpdateP2PConfig succeeded - P2P manager exists!")
	}

	t.Log("P2P manager creation debug test completed!")
}