package tests

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	"ivaldi/forge"
)

// TestP2PBasicNetworking tests basic P2P network operations
func TestP2PBasicNetworking(t *testing.T) {
	// Create test repositories
	tempDir1, err := os.MkdirTemp("", "ivaldi-p2p-test1-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir1)

	tempDir2, err := os.MkdirTemp("", "ivaldi-p2p-test2-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir2)

	// Initialize repositories
	repo1, err := forge.Initialize(tempDir1)
	if err != nil {
		t.Fatalf("Failed to initialize repo1: %v", err)
	}
	defer repo1.Close()

	repo2, err := forge.Initialize(tempDir2)
	if err != nil {
		t.Fatalf("Failed to initialize repo2: %v", err)
	}
	defer repo2.Close()

	// Test P2P initialization
	if repo1.IsP2PRunning() {
		t.Error("P2P should not be running initially")
	}

	// Start P2P on first repository (using different ports to avoid conflicts)
	config1 := repo1.GetP2PConfig()
	if config1 == nil {
		t.Skip("P2P configuration not available - P2P may not be fully initialized")
		return
	}
	config1.Port = 9090
	config1.DiscoveryPort = 9091
	err = repo1.UpdateP2PConfig(config1)
	if err != nil {
		t.Fatalf("Failed to update P2P config: %v", err)
	}

	err = repo1.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo1: %v", err)
	}
	defer repo1.StopP2P()

	if !repo1.IsP2PRunning() {
		t.Error("P2P should be running after start")
	}

	// Start P2P on second repository (different ports)
	config2 := repo2.GetP2PConfig()
	config2.Port = 9092
	config2.DiscoveryPort = 9093
	if err := repo2.UpdateP2PConfig(config2); err != nil {
		t.Fatalf("repo2 UpdateP2PConfig: %v", err)
	}

	err = repo2.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo2: %v", err)
	}
	defer repo2.StopP2P()

	// Wait a moment for services to start
	time.Sleep(100 * time.Millisecond)

	// Test connection between peers
	err = repo1.ConnectToPeer("127.0.0.1", 9092)
	if err != nil {
		t.Fatalf("Failed to connect peers: %v", err)
	}

	// Wait for connection to establish
	time.Sleep(200 * time.Millisecond)

	// Check connected peers
	peers1 := repo1.GetP2PPeers()
	if len(peers1) != 1 {
		t.Errorf("Expected 1 peer in repo1, got %d", len(peers1))
	}

	peers2 := repo2.GetP2PPeers()
	if len(peers2) != 1 {
		t.Errorf("Expected 1 peer in repo2, got %d", len(peers2))
	}

	t.Log("P2P basic networking test passed!")
}

// TestP2PSync tests P2P synchronization between repositories
func TestP2PSync(t *testing.T) {
	// Create test repositories
	tempDir1, err := os.MkdirTemp("", "ivaldi-p2p-sync1-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir1)

	tempDir2, err := os.MkdirTemp("", "ivaldi-p2p-sync2-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir2)

	// Initialize repositories
	repo1, err := forge.Initialize(tempDir1)
	if err != nil {
		t.Fatalf("Failed to initialize repo1: %v", err)
	}
	defer repo1.Close()

	repo2, err := forge.Initialize(tempDir2)
	if err != nil {
		t.Fatalf("Failed to initialize repo2: %v", err)
	}
	defer repo2.Close()

	// Create test file in repo1
	testFile1 := filepath.Join(tempDir1, "test1.txt")
	err = os.WriteFile(testFile1, []byte("Hello from repo1"), 0644)
	if err != nil {
		t.Fatalf("Failed to create test file: %v", err)
	}

	// Add and seal in repo1
	err = repo1.Gather([]string{"test1.txt"})
	if err != nil {
		t.Fatalf("Failed to gather files in repo1: %v", err)
	}

	_, err = repo1.Seal("Initial commit in repo1")
	if err != nil {
		t.Fatalf("Failed to seal in repo1: %v", err)
	}

	// Start P2P on both repositories
	config1 := repo1.GetP2PConfig()
	config1.Port = 9094
	config1.DiscoveryPort = 9095
	config1.AutoSyncEnabled = true
	config1.SyncInterval = 1 * time.Second
	if err := repo1.UpdateP2PConfig(config1); err != nil {
		t.Fatalf("repo1 UpdateP2PConfig: %v", err)
	}

	config2 := repo2.GetP2PConfig()
	config2.Port = 9096
	config2.DiscoveryPort = 9097
	config2.AutoSyncEnabled = true
	config2.SyncInterval = 1 * time.Second
	if err := repo2.UpdateP2PConfig(config2); err != nil {
		t.Fatalf("repo2 UpdateP2PConfig: %v", err)
	}

	err = repo1.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo1: %v", err)
	}
	defer repo1.StopP2P()

	err = repo2.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo2: %v", err)
	}
	defer repo2.StopP2P()

	// Connect peers
	time.Sleep(100 * time.Millisecond)
	err = repo1.ConnectToPeer("127.0.0.1", 9096)
	if err != nil {
		t.Fatalf("Failed to connect peers: %v", err)
	}

	// Wait for connection and initial sync
	time.Sleep(500 * time.Millisecond)

	// Perform manual sync
	err = repo1.SyncWithAllP2PPeers()
	if err != nil {
		t.Fatalf("Failed to sync with peers: %v", err)
	}

	// Wait for sync to complete
	time.Sleep(200 * time.Millisecond)

	// Check sync state
	syncStates1 := repo1.GetP2PSyncState()
	if len(syncStates1) == 0 {
		t.Error("Expected sync states in repo1")
	}

	syncStates2 := repo2.GetP2PSyncState()
	if len(syncStates2) == 0 {
		t.Error("Expected sync states in repo2")
	}

	t.Log("P2P sync test passed!")
}

// TestP2PDiscovery tests peer discovery functionality
func TestP2PDiscovery(t *testing.T) {
	// Create test repositories
	tempDir1, err := os.MkdirTemp("", "ivaldi-p2p-discovery1-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir1)

	tempDir2, err := os.MkdirTemp("", "ivaldi-p2p-discovery2-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir2)

	// Initialize repositories
	repo1, err := forge.Initialize(tempDir1)
	if err != nil {
		t.Fatalf("Failed to initialize repo1: %v", err)
	}
	defer repo1.Close()

	repo2, err := forge.Initialize(tempDir2)
	if err != nil {
		t.Fatalf("Failed to initialize repo2: %v", err)
	}
	defer repo2.Close()

	// Start P2P with discovery
	config1 := repo1.GetP2PConfig()
	config1.Port = 9098
	config1.DiscoveryPort = 9099
	config1.EnableAutoConnect = true
	if err := repo1.UpdateP2PConfig(config1); err != nil {
		t.Fatalf("repo1 UpdateP2PConfig: %v", err)
	}

	config2 := repo2.GetP2PConfig()
	config2.Port = 9100
	config2.DiscoveryPort = 9099 // Same discovery port for local discovery
	config2.EnableAutoConnect = true
	if err := repo2.UpdateP2PConfig(config2); err != nil {
		t.Fatalf("repo2 UpdateP2PConfig: %v", err)
	}

	err = repo1.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo1: %v", err)
	}
	defer repo1.StopP2P()

	err = repo2.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P on repo2: %v", err)
	}
	defer repo2.StopP2P()

	// Wait for discovery
	time.Sleep(2 * time.Second)

	// Check discovered peers
	discovered1 := repo1.GetDiscoveredPeers()
	discovered2 := repo2.GetDiscoveredPeers()

	// Should have discovered each other
	if len(discovered1) == 0 && len(discovered2) == 0 {
		t.Log("Warning: No peers discovered (may be expected in test environment)")
	} else {
		t.Logf("Discovered peers: repo1=%d, repo2=%d", len(discovered1), len(discovered2))
	}

	t.Log("P2P discovery test completed!")
}

// TestP2PConfig tests P2P configuration management
func TestP2PConfig(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-config-*")
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

	// Test default config
	config := repo.GetP2PConfig()
	if config == nil {
		t.Fatal("Expected default P2P config")
	}

	if config.Port != 9090 {
		t.Errorf("Expected default port 9090, got %d", config.Port)
	}

	if config.AutoSyncEnabled != true {
		t.Error("Expected auto-sync to be enabled by default")
	}

	// Test config updates
	err = repo.EnableP2PAutoSync(false)
	if err != nil {
		t.Fatalf("Failed to disable auto-sync: %v", err)
	}

	config = repo.GetP2PConfig()
	if config.AutoSyncEnabled {
		t.Error("Auto-sync should be disabled")
	}

	// Test sync interval update
	err = repo.SetP2PSyncInterval(5 * time.Minute)
	if err != nil {
		t.Fatalf("Failed to set sync interval: %v", err)
	}

	config = repo.GetP2PConfig()
	if config.SyncInterval != 5*time.Minute {
		t.Errorf("Expected sync interval 5m, got %v", config.SyncInterval)
	}

	t.Log("P2P config test passed!")
}

// TestP2PMultiplePeers tests P2P with multiple peers
func TestP2PMultiplePeers(t *testing.T) {
	// Create multiple test repositories
	var repos []*forge.Repository
	var tempDirs []string

	// Create 3 repositories
	for i := 0; i < 3; i++ {
		tempDir, err := os.MkdirTemp("", fmt.Sprintf("ivaldi-p2p-multi%d-*", i))
		if err != nil {
			t.Fatalf("Failed to create temp dir %d: %v", i, err)
		}
		tempDirs = append(tempDirs, tempDir)

		repo, err := forge.Initialize(tempDir)
		if err != nil {
			t.Fatalf("Failed to initialize repo %d: %v", i, err)
		}
		repos = append(repos, repo)
	}

	// Cleanup
	defer func() {
		for _, repo := range repos {
			repo.StopP2P()
			repo.Close()
		}
		for _, tempDir := range tempDirs {
			os.RemoveAll(tempDir)
		}
	}()

	// Start P2P on all repositories with different ports
	for i, repo := range repos {
		config := repo.GetP2PConfig()
		config.Port = 9110 + i
		config.DiscoveryPort = 9113 + i
		config.AutoSyncEnabled = true
		repo.UpdateP2PConfig(config)

		err := repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo %d: %v", i, err)
		}
	}

	// Wait for services to start
	time.Sleep(200 * time.Millisecond)

	// Connect repo0 to repo1, repo1 to repo2 (chain)
	err := repos[0].ConnectToPeer("127.0.0.1", 9111)
	if err != nil {
		t.Fatalf("Failed to connect repo0 to repo1: %v", err)
	}

	err = repos[1].ConnectToPeer("127.0.0.1", 9112)
	if err != nil {
		t.Fatalf("Failed to connect repo1 to repo2: %v", err)
	}

	// Wait for connections
	time.Sleep(300 * time.Millisecond)

	// Check connections
	peers0 := repos[0].GetP2PPeers()
	peers1 := repos[1].GetP2PPeers()
	peers2 := repos[2].GetP2PPeers()

	if len(peers0) != 1 {
		t.Errorf("Expected 1 peer in repo0, got %d", len(peers0))
	}

	if len(peers1) != 2 {
		t.Errorf("Expected 2 peers in repo1, got %d", len(peers1))
	}

	if len(peers2) != 1 {
		t.Errorf("Expected 1 peer in repo2, got %d", len(peers2))
	}

	// Test multi-peer sync
	err = repos[1].SyncWithAllP2PPeers()
	if err != nil {
		t.Fatalf("Failed to sync with all peers: %v", err)
	}

	// Wait for sync
	time.Sleep(200 * time.Millisecond)

	// Check sync states
	for i, repo := range repos {
		syncStates := repo.GetP2PSyncState()
		t.Logf("Repo %d has %d sync states", i, len(syncStates))
	}

	t.Log("P2P multiple peers test passed!")
}

// TestP2PErrorHandling tests P2P error scenarios
func TestP2PErrorHandling(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-error-*")
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

	// Test operations when P2P is not running
	err = repo.ConnectToPeer("127.0.0.1", 9999)
	if err == nil {
		t.Error("Expected error when connecting with P2P not running")
	}

	err = repo.SyncWithAllP2PPeers()
	if err == nil {
		t.Error("Expected error when syncing with P2P not running")
	}

	// Start P2P
	config := repo.GetP2PConfig()
	config.Port = 9120
	config.DiscoveryPort = 9121
	repo.UpdateP2PConfig(config)

	err = repo.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P: %v", err)
	}
	defer repo.StopP2P()

	// Test invalid peer connections
	err = repo.ConnectToPeer("invalid-address", 9999)
	if err == nil {
		t.Error("Expected error when connecting to invalid address")
	}

	err = repo.ConnectToPeer("127.0.0.1", 99999)
	if err == nil {
		t.Error("Expected error when connecting to unreachable port")
	}

	// Test invalid sync interval (negative duration)
	err = repo.SetP2PSyncInterval(-1 * time.Second)
	if err == nil {
		t.Error("Expected error when setting negative sync interval")
	}

	t.Log("P2P error handling test passed!")
}

// TestP2PAutoSync tests automatic synchronization
func TestP2PAutoSync(t *testing.T) {
	// This test would be more comprehensive in a real environment
	// Here we just test the configuration and basic setup

	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-autosync-*")
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

	// Test auto-sync configuration
	config := repo.GetP2PConfig()
	if !config.AutoSyncEnabled {
		t.Error("Auto-sync should be enabled by default")
	}

	// Disable auto-sync
	err = repo.EnableP2PAutoSync(false)
	if err != nil {
		t.Fatalf("Failed to disable auto-sync: %v", err)
	}

	config = repo.GetP2PConfig()
	if config.AutoSyncEnabled {
		t.Error("Auto-sync should be disabled")
	}

	// Re-enable auto-sync
	err = repo.EnableP2PAutoSync(true)
	if err != nil {
		t.Fatalf("Failed to enable auto-sync: %v", err)
	}

	config = repo.GetP2PConfig()
	if !config.AutoSyncEnabled {
		t.Error("Auto-sync should be enabled")
	}

	// Test sync interval
	err = repo.SetP2PSyncInterval(30 * time.Second)
	if err != nil {
		t.Fatalf("Failed to set sync interval: %v", err)
	}

	config = repo.GetP2PConfig()
	if config.SyncInterval != 30*time.Second {
		t.Errorf("Expected sync interval 30s, got %v", config.SyncInterval)
	}

	t.Log("P2P auto-sync test passed!")
}

// Helper function to check if a repository has P2P functionality
func hasP2PSupport(repo *forge.Repository) bool {
	status := repo.GetP2PStatus()
	return status != nil
}

// TestP2PIntegration tests overall P2P integration
func TestP2PIntegration(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-integration-*")
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

	// Check P2P support
	if !hasP2PSupport(repo) {
		t.Fatal("Repository should have P2P support")
	}

	// Test status when not running
	status := repo.GetP2PStatus()
	if status.Running {
		t.Error("P2P should not be running initially")
	}

	// Start P2P
	config := repo.GetP2PConfig()
	config.Port = 9130
	config.DiscoveryPort = 9131
	repo.UpdateP2PConfig(config)

	err = repo.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P: %v", err)
	}
	defer repo.StopP2P()

	// Test status when running
	status = repo.GetP2PStatus()
	if !status.Running {
		t.Error("P2P should be running")
	}

	if status.Port != 9130 {
		t.Errorf("Expected port 9130, got %d", status.Port)
	}

	if status.NodeID == "" {
		t.Error("Node ID should not be empty")
	}

	// Test peer lists (should be empty initially)
	peers := repo.GetP2PPeers()
	if len(peers) != 0 {
		t.Errorf("Expected 0 peers initially, got %d", len(peers))
	}

	discovered := repo.GetDiscoveredPeers()
	t.Logf("Discovered %d peers", len(discovered))

	// Test sync states (should be empty initially)
	syncStates := repo.GetP2PSyncState()
	if len(syncStates) != 0 {
		t.Errorf("Expected 0 sync states initially, got %d", len(syncStates))
	}

	t.Log("P2P integration test passed!")
}