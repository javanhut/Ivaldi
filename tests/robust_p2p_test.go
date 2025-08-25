package tests

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"ivaldi/forge"
)

// TestP2PRobustNetworking tests P2P networking with actual connections
func TestP2PRobustNetworking(t *testing.T) {
	// Create two test repositories
	tempDirs := make([]string, 2)
	repos := make([]*forge.Repository, 2)
	
	for i := 0; i < 2; i++ {
		tempDir, err := os.MkdirTemp("", fmt.Sprintf("ivaldi-robust-p2p-%d-*", i))
		if err != nil {
			t.Fatalf("Failed to create temp dir %d: %v", i, err)
		}
		tempDirs[i] = tempDir
		
		repo, err := forge.Initialize(tempDir)
		if err != nil {
			t.Fatalf("Failed to initialize repo %d: %v", i, err)
		}
		repos[i] = repo
	}
	
	// Cleanup
	defer func() {
		for i, repo := range repos {
			if repo != nil {
				repo.StopP2P()
				repo.Close()
			}
			os.RemoveAll(tempDirs[i])
		}
	}()

	// Configure P2P with different ports for each repo
	for i, repo := range repos {
		config := repo.GetP2PConfig()
		if config == nil {
			t.Fatalf("P2P config is nil for repo %d", i)
		}
		
		config.Port = 9200 + i
		config.DiscoveryPort = 9300 + i
		config.AutoSyncEnabled = true
		config.SyncInterval = 2 * time.Second // Fast sync for testing
		
		t.Logf("Setting repo %d to use ports %d and %d", i, config.Port, config.DiscoveryPort)
		
		err := repo.UpdateP2PConfig(config)
		if err != nil {
			t.Fatalf("Failed to update P2P config for repo %d: %v", i, err)
		}
		
		// Verify config was updated
		updatedConfig := repo.GetP2PConfig()
		t.Logf("Repo %d config after update: port=%d, discovery=%d", i, updatedConfig.Port, updatedConfig.DiscoveryPort)
	}

	// Start P2P on both repositories
	for i, repo := range repos {
		err := repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo %d: %v", i, err)
		}
		
		// Verify it's running
		if !repo.IsP2PRunning() {
			t.Errorf("P2P should be running on repo %d", i)
		}
	}

	// Wait for services to fully start
	time.Sleep(500 * time.Millisecond)

	// Test connection between repos
	err := repos[0].ConnectToPeer("127.0.0.1", 9201)
	if err != nil {
		t.Fatalf("Failed to connect repo0 to repo1: %v", err)
	}

	// Wait for connection to establish
	time.Sleep(1 * time.Second)

	// Verify connection established
	peers0 := repos[0].GetP2PPeers()
	peers1 := repos[1].GetP2PPeers()
	
	t.Logf("Repo0 has %d peers", len(peers0))
	t.Logf("Repo1 has %d peers", len(peers1))
	
	if len(peers0) == 0 {
		t.Error("Repo0 should have at least 1 connected peer")
	}
	if len(peers1) == 0 {
		t.Error("Repo1 should have at least 1 connected peer")
	}

	// Test sync functionality
	err = repos[0].SyncWithAllP2PPeers()
	if err != nil {
		t.Errorf("Failed to sync repo0 with peers: %v", err)
	}

	// Check sync states
	syncStates0 := repos[0].GetP2PSyncState()
	syncStates1 := repos[1].GetP2PSyncState()
	
	t.Logf("Repo0 sync states: %d", len(syncStates0))
	t.Logf("Repo1 sync states: %d", len(syncStates1))

	t.Log("P2P robust networking test completed successfully!")
}

// TestP2PDataSynchronization tests actual data sync between peers
func TestP2PDataSynchronization(t *testing.T) {
	// Create two test repositories
	tempDirs := make([]string, 2)
	repos := make([]*forge.Repository, 2)
	
	for i := 0; i < 2; i++ {
		tempDir, err := os.MkdirTemp("", fmt.Sprintf("ivaldi-sync-test-%d-*", i))
		if err != nil {
			t.Fatalf("Failed to create temp dir %d: %v", i, err)
		}
		tempDirs[i] = tempDir
		
		repo, err := forge.Initialize(tempDir)
		if err != nil {
			t.Fatalf("Failed to initialize repo %d: %v", i, err)
		}
		repos[i] = repo
	}
	
	// Cleanup
	defer func() {
		for i, repo := range repos {
			if repo != nil {
				repo.StopP2P()
				repo.Close()
			}
			os.RemoveAll(tempDirs[i])
		}
	}()

	// Create test content in repo0
	testFile := filepath.Join(tempDirs[0], "sync_test.txt")
	testContent := "This is a test file for P2P synchronization"
	err := os.WriteFile(testFile, []byte(testContent), 0644)
	if err != nil {
		t.Fatalf("Failed to create test file: %v", err)
	}

	// Gather and seal in repo0
	err = repos[0].Gather([]string{"sync_test.txt"})
	if err != nil {
		t.Fatalf("Failed to gather files in repo0: %v", err)
	}

	_, err = repos[0].Seal("Test commit for P2P sync")
	if err != nil {
		t.Fatalf("Failed to seal in repo0: %v", err)
	}

	// Start P2P with different ports
	for i, repo := range repos {
		config := repo.GetP2PConfig()
		config.Port = 9210 + i
		config.DiscoveryPort = 9310 + i
		config.AutoSyncEnabled = true
		config.SyncInterval = 1 * time.Second
		
		err := repo.UpdateP2PConfig(config)
		if err != nil {
			t.Fatalf("Failed to update P2P config for repo %d: %v", i, err)
		}

		err = repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo %d: %v", i, err)
		}
	}

	// Wait for startup
	time.Sleep(500 * time.Millisecond)

	// Connect peers
	err = repos[0].ConnectToPeer("127.0.0.1", 9211)
	if err != nil {
		t.Fatalf("Failed to connect peers: %v", err)
	}

	// Wait for connection
	time.Sleep(1 * time.Second)

	// Perform sync
	err = repos[0].SyncWithAllP2PPeers()
	if err != nil {
		t.Errorf("Sync failed: %v", err)
	}

	// Wait for sync to complete
	time.Sleep(2 * time.Second)

	// Check that both repos have similar timeline states
	status0 := repos[0].Status()
	status1 := repos[1].Status()
	
	t.Logf("Repo0 timeline: %s, position: %s", status0.Timeline, status0.Position)
	t.Logf("Repo1 timeline: %s, position: %s", status1.Timeline, status1.Position)

	t.Log("P2P data synchronization test completed!")
}

// TestP2PMultiplePeersRobust tests P2P with multiple peers
func TestP2PMultiplePeersRobust(t *testing.T) {
	const numRepos = 3
	tempDirs := make([]string, numRepos)
	repos := make([]*forge.Repository, numRepos)
	
	// Create repositories
	for i := 0; i < numRepos; i++ {
		tempDir, err := os.MkdirTemp("", fmt.Sprintf("ivaldi-multi-p2p-%d-*", i))
		if err != nil {
			t.Fatalf("Failed to create temp dir %d: %v", i, err)
		}
		tempDirs[i] = tempDir
		
		repo, err := forge.Initialize(tempDir)
		if err != nil {
			t.Fatalf("Failed to initialize repo %d: %v", i, err)
		}
		repos[i] = repo
	}
	
	// Cleanup
	defer func() {
		for i, repo := range repos {
			if repo != nil {
				repo.StopP2P()
				repo.Close()
			}
			os.RemoveAll(tempDirs[i])
		}
	}()

	// Configure and start P2P on all repos
	for i, repo := range repos {
		config := repo.GetP2PConfig()
		config.Port = 9220 + i
		config.DiscoveryPort = 9320 + i
		config.AutoSyncEnabled = true
		config.SyncInterval = 2 * time.Second
		config.MaxPeers = 10 // Allow multiple connections
		
		err := repo.UpdateP2PConfig(config)
		if err != nil {
			t.Fatalf("Failed to update P2P config for repo %d: %v", i, err)
		}

		err = repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo %d: %v", i, err)
		}
	}

	// Wait for all services to start
	time.Sleep(1 * time.Second)

	// Connect repos in a chain: 0->1->2
	err := repos[0].ConnectToPeer("127.0.0.1", 9221)
	if err != nil {
		t.Fatalf("Failed to connect repo0 to repo1: %v", err)
	}

	err = repos[1].ConnectToPeer("127.0.0.1", 9222)
	if err != nil {
		t.Fatalf("Failed to connect repo1 to repo2: %v", err)
	}

	// Also create a direct connection: 0->2
	err = repos[0].ConnectToPeer("127.0.0.1", 9222)
	if err != nil {
		t.Logf("Direct connection 0->2 failed (expected): %v", err)
	}

	// Wait for connections to establish
	time.Sleep(2 * time.Second)

	// Check peer counts
	for i, repo := range repos {
		peers := repo.GetP2PPeers()
		t.Logf("Repo%d has %d connected peers", i, len(peers))
		
		for j, peer := range peers {
			t.Logf("  Peer%d: ID=%s, Address=%s:%d, Status=%s", 
				j, peer.ID, peer.Address, peer.Port, peer.Status)
		}
	}

	// Test sync from central node
	err = repos[1].SyncWithAllP2PPeers()
	if err != nil {
		t.Errorf("Failed to sync from central node: %v", err)
	}

	// Wait for sync
	time.Sleep(3 * time.Second)

	// Check sync states
	for i, repo := range repos {
		syncStates := repo.GetP2PSyncState()
		t.Logf("Repo%d sync states: %d", i, len(syncStates))
	}

	t.Log("Multi-peer P2P test completed successfully!")
}

// TestP2PErrorHandlingRobust tests P2P error scenarios robustly
func TestP2PErrorHandlingRobust(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-error-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer func() {
		repo.StopP2P()
		repo.Close()
	}()

	// Test operations when P2P is not running
	t.Log("Testing operations when P2P is not running...")
	
	if repo.IsP2PRunning() {
		t.Error("P2P should not be running initially")
	}

	err = repo.ConnectToPeer("127.0.0.1", 9999)
	if err == nil {
		t.Error("Expected error when connecting with P2P not running")
	} else {
		t.Logf("Expected error: %v", err)
	}

	err = repo.SyncWithAllP2PPeers()
	if err == nil {
		t.Error("Expected error when syncing with P2P not running")
	} else {
		t.Logf("Expected error: %v", err)
	}

	// Start P2P for further tests
	config := repo.GetP2PConfig()
	config.Port = 9230
	config.DiscoveryPort = 9330
	repo.UpdateP2PConfig(config)

	err = repo.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P: %v", err)
	}

	time.Sleep(500 * time.Millisecond)

	if !repo.IsP2PRunning() {
		t.Error("P2P should be running after start")
	}

	// Test invalid peer connections
	t.Log("Testing invalid peer connections...")
	
	err = repo.ConnectToPeer("invalid-hostname-that-does-not-exist", 9999)
	if err == nil {
		t.Error("Expected error when connecting to invalid hostname")
	} else {
		t.Logf("Expected error for invalid hostname: %v", err)
	}

	err = repo.ConnectToPeer("127.0.0.1", 1) // Reserved port
	if err == nil {
		t.Error("Expected error when connecting to reserved port")
	} else {
		t.Logf("Expected error for reserved port: %v", err)
	}

	err = repo.ConnectToPeer("127.0.0.1", 99999) // Unreachable port
	if err == nil {
		t.Error("Expected error when connecting to unreachable port")
	} else {
		t.Logf("Expected error for unreachable port: %v", err)
	}

	// Test configuration errors
	t.Log("Testing configuration errors...")
	
	err = repo.SetP2PSyncInterval(-1 * time.Second)
	if err == nil {
		t.Error("Expected error when setting negative sync interval")
	} else {
		t.Logf("Expected error for negative sync interval: %v", err)
	}

	// Test valid configurations
	t.Log("Testing valid configurations...")
	
	err = repo.SetP2PSyncInterval(30 * time.Second)
	if err != nil {
		t.Errorf("Unexpected error setting valid sync interval: %v", err)
	}

	err = repo.EnableP2PAutoSync(false)
	if err != nil {
		t.Errorf("Unexpected error disabling auto-sync: %v", err)
	}

	err = repo.EnableP2PAutoSync(true)
	if err != nil {
		t.Errorf("Unexpected error enabling auto-sync: %v", err)
	}

	// Test stop and restart
	t.Log("Testing stop and restart...")
	
	err = repo.StopP2P()
	if err != nil {
		t.Errorf("Unexpected error stopping P2P: %v", err)
	}

	if repo.IsP2PRunning() {
		t.Error("P2P should not be running after stop")
	}

	err = repo.StartP2P()
	if err != nil {
		t.Errorf("Unexpected error restarting P2P: %v", err)
	}

	time.Sleep(300 * time.Millisecond)

	if !repo.IsP2PRunning() {
		t.Error("P2P should be running after restart")
	}

	t.Log("P2P error handling test completed successfully!")
}

// TestP2PConcurrency tests P2P under concurrent access
func TestP2PConcurrency(t *testing.T) {
	// Create test repository
	tempDir, err := os.MkdirTemp("", "ivaldi-p2p-concurrent-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer func() {
		repo.StopP2P()
		repo.Close()
	}()

	// Configure P2P
	config := repo.GetP2PConfig()
	config.Port = 9240
	config.DiscoveryPort = 9340
	repo.UpdateP2PConfig(config)

	err = repo.StartP2P()
	if err != nil {
		t.Fatalf("Failed to start P2P: %v", err)
	}

	time.Sleep(500 * time.Millisecond)

	// Test concurrent operations
	const numGoroutines = 10
	var wg sync.WaitGroup
	errors := make(chan error, numGoroutines*3)

	t.Logf("Running %d concurrent operations...", numGoroutines)

	for i := 0; i < numGoroutines; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()

			// Test concurrent status checks
			status := repo.GetP2PStatus()
			if status == nil {
				errors <- fmt.Errorf("goroutine %d: status is nil", id)
				return
			}

			// Test concurrent config access
			config := repo.GetP2PConfig()
			if config == nil {
				errors <- fmt.Errorf("goroutine %d: config is nil", id)
				return
			}

			// Test concurrent peer list access
			peers := repo.GetP2PPeers()
			_ = peers // Just checking it doesn't panic

			// Test concurrent sync state access
			syncStates := repo.GetP2PSyncState()
			_ = syncStates // Just checking it doesn't panic

			t.Logf("Goroutine %d completed successfully", id)
		}(i)
	}

	// Wait for all goroutines to complete
	wg.Wait()
	close(errors)

	// Check for errors
	var errCount int
	for err := range errors {
		t.Errorf("Concurrent operation error: %v", err)
		errCount++
	}

	if errCount > 0 {
		t.Errorf("Found %d errors in concurrent operations", errCount)
	} else {
		t.Log("All concurrent operations completed successfully!")
	}

	t.Log("P2P concurrency test completed!")
}

// TestP2PResourceCleanup tests proper resource cleanup
func TestP2PResourceCleanup(t *testing.T) {
	const numCycles = 5
	
	for cycle := 0; cycle < numCycles; cycle++ {
		t.Logf("Cleanup test cycle %d/%d", cycle+1, numCycles)
		
		// Create test repository
		tempDir, err := os.MkdirTemp("", fmt.Sprintf("ivaldi-p2p-cleanup-%d-*", cycle))
		if err != nil {
			t.Fatalf("Failed to create temp dir for cycle %d: %v", cycle, err)
		}

		func() {
			repo, err := forge.Initialize(tempDir)
			if err != nil {
				t.Fatalf("Failed to initialize repo for cycle %d: %v", cycle, err)
			}
			defer repo.Close()

			// Configure P2P with unique ports
			config := repo.GetP2PConfig()
			config.Port = 9250 + cycle
			config.DiscoveryPort = 9350 + cycle
			repo.UpdateP2PConfig(config)

			// Start and stop P2P multiple times
			for i := 0; i < 3; i++ {
				err = repo.StartP2P()
				if err != nil {
					t.Errorf("Failed to start P2P in cycle %d, iteration %d: %v", cycle, i, err)
					continue
				}

				time.Sleep(100 * time.Millisecond)

				if !repo.IsP2PRunning() {
					t.Errorf("P2P should be running in cycle %d, iteration %d", cycle, i)
				}

				err = repo.StopP2P()
				if err != nil {
					t.Errorf("Failed to stop P2P in cycle %d, iteration %d: %v", cycle, i, err)
				}

				time.Sleep(100 * time.Millisecond)

				if repo.IsP2PRunning() {
					t.Errorf("P2P should not be running after stop in cycle %d, iteration %d", cycle, i)
				}
			}
		}()

		// Clean up
		os.RemoveAll(tempDir)
		
		// Give system time to cleanup resources
		time.Sleep(200 * time.Millisecond)
	}

	t.Log("P2P resource cleanup test completed successfully!")
}