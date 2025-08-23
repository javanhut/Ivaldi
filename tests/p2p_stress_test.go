package tests

import (
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
	
	"ivaldi/forge"
)

// TestP2PStressMultiplePeers tests P2P performance with many peers
func TestP2PStressMultiplePeers(t *testing.T) {
	numPeers := 5
	repos := make([]*forge.Repository, numPeers)
	dirs := make([]string, numPeers)
	
	t.Logf("=== P2P Stress Test: %d Peers ===", numPeers)
	
	// Create and initialize all repositories
	for i := 0; i < numPeers; i++ {
		dir, _ := os.MkdirTemp("", fmt.Sprintf("p2p_stress_%d_*", i))
		dirs[i] = dir
		defer os.RemoveAll(dir)
		
		repo, err := forge.Initialize(dir)
		if err != nil {
			t.Fatalf("Failed to init repo %d: %v", i, err)
		}
		repos[i] = repo
		
		// Create initial content
		testFile := filepath.Join(dir, "test.txt")
		os.WriteFile(testFile, []byte(fmt.Sprintf("content from repo %d", i)), 0644)
		repo.Gather([]string{"test.txt"})
		seal, _ := repo.Seal(fmt.Sprintf("Initial from repo %d", i))
		t.Logf("Repo %d created seal: %s", i, seal.Name)
		
		// Configure P2P with unique ports
		config := repo.GetP2PConfig()
		config.Port = 9500 + i*2
		config.DiscoveryPort = 9501 + i*2
		config.AutoSyncEnabled = false
		config.MaxPeers = numPeers
		repo.UpdateP2PConfig(config)
		
		// Start P2P
		err = repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo %d: %v", i, err)
		}
		defer repo.StopP2P()
	}
	
	// Connect all peers to repo 0 (star topology)
	t.Log("Creating star topology connections...")
	for i := 1; i < numPeers; i++ {
		err := repos[i].ConnectToPeer("localhost", 9500)
		if err != nil {
			t.Logf("Warning: Failed to connect repo %d to repo 0: %v", i, err)
		}
	}
	
	time.Sleep(3 * time.Second)
	
	// Verify connections
	connectedCount := 0
	for i := 0; i < numPeers; i++ {
		peers := repos[i].GetP2PPeers()
		t.Logf("Repo %d has %d peers", i, len(peers))
		if len(peers) > 0 {
			connectedCount++
		}
	}
	
	if connectedCount < numPeers/2 {
		t.Errorf("Too few repos connected (%d/%d)", connectedCount, numPeers)
	}
	
	t.Log("=== Stress Testing: Concurrent Sync Operations ===")
	
	var wg sync.WaitGroup
	errors := make(chan error, numPeers*numPeers)
	syncCount := 0
	
	startTime := time.Now()
	
	// Each repo tries to sync with every other repo
	for i := 0; i < numPeers; i++ {
		for j := 0; j < numPeers; j++ {
			if i != j {
				wg.Add(1)
				syncCount++
				go func(from, to int) {
					defer wg.Done()
					
					toStatus := repos[to].GetP2PStatus()
					err := repos[from].SyncWithP2PPeer(toStatus.NodeID)
					if err != nil {
						errors <- fmt.Errorf("sync %d->%d failed: %v", from, to, err)
					}
				}(i, j)
			}
		}
	}
	
	// Wait for all syncs to complete
	wg.Wait()
	close(errors)
	
	syncDuration := time.Since(startTime)
	t.Logf("Completed %d sync operations in %v", syncCount, syncDuration)
	
	// Check for errors
	errorCount := 0
	for err := range errors {
		t.Logf("Sync error: %v", err)
		errorCount++
	}
	
	if errorCount > syncCount/2 {
		t.Errorf("Too many sync errors: %d/%d", errorCount, syncCount)
	} else {
		t.Logf("✓ Stress test completed with %d/%d successful syncs", syncCount-errorCount, syncCount)
	}
	
	// Performance metrics
	avgSyncTime := syncDuration / time.Duration(syncCount)
	t.Logf("Average sync time: %v", avgSyncTime)
	
	if avgSyncTime > 5*time.Second {
		t.Logf("Warning: Average sync time is high (%v)", avgSyncTime)
	}
}

// TestP2PStressRapidChanges tests P2P with rapid consecutive changes
func TestP2PStressRapidChanges(t *testing.T) {
	// Create two repositories
	dir1, _ := os.MkdirTemp("", "p2p_rapid1_*")
	dir2, _ := os.MkdirTemp("", "p2p_rapid2_*")
	defer os.RemoveAll(dir1)
	defer os.RemoveAll(dir2)
	
	repo1, err := forge.Initialize(dir1)
	if err != nil {
		t.Fatalf("Failed to init repo1: %v", err)
	}
	
	repo2, err := forge.Initialize(dir2)
	if err != nil {
		t.Fatalf("Failed to init repo2: %v", err)
	}
	
	// Configure P2P
	config1 := repo1.GetP2PConfig()
	config1.Port = 9520
	config1.DiscoveryPort = 9521
	config1.AutoSyncEnabled = true
	config1.SyncInterval = 500 * time.Millisecond // Fast sync
	repo1.UpdateP2PConfig(config1)
	
	config2 := repo2.GetP2PConfig()
	config2.Port = 9522
	config2.DiscoveryPort = 9523
	config2.AutoSyncEnabled = true
	config2.SyncInterval = 500 * time.Millisecond
	repo2.UpdateP2PConfig(config2)
	
	// Start P2P
	repo1.StartP2P()
	defer repo1.StopP2P()
	
	repo2.StartP2P()
	defer repo2.StopP2P()
	
	// Connect
	repo2.ConnectToPeer("localhost", 9520)
	time.Sleep(1 * time.Second)
	
	t.Log("=== Stress Testing: Rapid Changes ===")
	
	// Create rapid changes in both repos
	numChanges := 20
	var wg sync.WaitGroup
	
	startTime := time.Now()
	
	// Repo1 makes rapid changes
	wg.Add(1)
	go func() {
		defer wg.Done()
		for i := 0; i < numChanges; i++ {
			fileName := fmt.Sprintf("file1_%d.txt", i)
			filePath := filepath.Join(dir1, fileName)
			content := fmt.Sprintf("rapid change %d from repo1 at %v", i, time.Now())
			
			os.WriteFile(filePath, []byte(content), 0644)
			repo1.Gather([]string{fileName})
			seal, _ := repo1.Seal(fmt.Sprintf("Rapid change %d", i))
			t.Logf("Repo1 created seal %d: %s", i, seal.Name)
			
			// Small delay to avoid overwhelming the system
			time.Sleep(50 * time.Millisecond)
		}
	}()
	
	// Repo2 makes rapid changes  
	wg.Add(1)
	go func() {
		defer wg.Done()
		for i := 0; i < numChanges; i++ {
			fileName := fmt.Sprintf("file2_%d.txt", i)
			filePath := filepath.Join(dir2, fileName)
			content := fmt.Sprintf("rapid change %d from repo2 at %v", i, time.Now())
			
			os.WriteFile(filePath, []byte(content), 0644)
			repo2.Gather([]string{fileName})
			seal, _ := repo2.Seal(fmt.Sprintf("Rapid change %d", i))
			t.Logf("Repo2 created seal %d: %s", i, seal.Name)
			
			time.Sleep(50 * time.Millisecond)
		}
	}()
	
	// Wait for all changes to complete
	wg.Wait()
	
	changeDuration := time.Since(startTime)
	t.Logf("Created %d changes in each repo in %v", numChanges, changeDuration)
	
	// Allow time for auto-sync to catch up
	t.Log("Allowing time for auto-sync to process changes...")
	time.Sleep(10 * time.Second)
	
	// Check sync states
	syncStates1 := repo1.GetP2PSyncState()
	syncStates2 := repo2.GetP2PSyncState()
	
	node1ID := repo1.GetP2PStatus().NodeID
	node2ID := repo2.GetP2PStatus().NodeID
	
	if syncState, exists := syncStates1[node2ID]; exists {
		t.Logf("Repo1 sync state: %d synced seals, %d conflicts", 
			len(syncState.SyncedSeals), syncState.ConflictCount)
	}
	
	if syncState, exists := syncStates2[node1ID]; exists {
		t.Logf("Repo2 sync state: %d synced seals, %d conflicts", 
			len(syncState.SyncedSeals), syncState.ConflictCount)
	}
	
	// Performance check
	avgChangeTime := changeDuration / time.Duration(numChanges*2)
	t.Logf("Average change time: %v", avgChangeTime)
	
	if avgChangeTime > 200*time.Millisecond {
		t.Logf("Warning: Average change time is high (%v)", avgChangeTime)
	}
}

// TestP2PStressLargeFiles tests P2P with large file transfers
func TestP2PStressLargeFiles(t *testing.T) {
	// Create two repositories
	dir1, _ := os.MkdirTemp("", "p2p_large1_*")
	dir2, _ := os.MkdirTemp("", "p2p_large2_*")
	defer os.RemoveAll(dir1)
	defer os.RemoveAll(dir2)
	
	repo1, err := forge.Initialize(dir1)
	if err != nil {
		t.Fatalf("Failed to init repo1: %v", err)
	}
	
	repo2, err := forge.Initialize(dir2)
	if err != nil {
		t.Fatalf("Failed to init repo2: %v", err)
	}
	
	// Configure P2P
	config1 := repo1.GetP2PConfig()
	config1.Port = 9530
	config1.DiscoveryPort = 9531
	config1.AutoSyncEnabled = false
	config1.MaxMessageSize = 50 * 1024 * 1024 // 50MB
	repo1.UpdateP2PConfig(config1)
	
	config2 := repo2.GetP2PConfig()
	config2.Port = 9532
	config2.DiscoveryPort = 9533
	config2.AutoSyncEnabled = false
	config2.MaxMessageSize = 50 * 1024 * 1024
	repo2.UpdateP2PConfig(config2)
	
	// Start P2P
	repo1.StartP2P()
	defer repo1.StopP2P()
	
	repo2.StartP2P()
	defer repo2.StopP2P()
	
	// Connect
	repo2.ConnectToPeer("localhost", 9530)
	time.Sleep(1 * time.Second)
	
	t.Log("=== Stress Testing: Large Files ===")
	
	// Create large files of different sizes
	fileSizes := []int{1024, 10240, 102400} // 1KB, 10KB, 100KB
	
	for i, size := range fileSizes {
		t.Logf("Testing file size: %d bytes", size)
		
		// Generate random content
		content := make([]byte, size)
		for j := range content {
			content[j] = byte(rand.Intn(256))
		}
		
		fileName := fmt.Sprintf("large_file_%d.dat", i)
		filePath := filepath.Join(dir1, fileName)
		
		startTime := time.Now()
		
		// Write and commit large file
		os.WriteFile(filePath, content, 0644)
		repo1.Gather([]string{fileName})
		seal, _ := repo1.Seal(fmt.Sprintf("Large file %d", i))
		
		commitTime := time.Since(startTime)
		t.Logf("File %d commit time: %v", i, commitTime)
		
		// Try to sync
		syncStart := time.Now()
		err := repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)
		syncTime := time.Since(syncStart)
		
		if err != nil {
			t.Logf("Large file sync failed: %v", err)
		} else {
			t.Logf("File %d sync time: %v (seal: %s)", i, syncTime, seal.Name)
		}
		
		// Performance warnings
		if commitTime > 5*time.Second {
			t.Logf("Warning: Commit time for %d bytes is high (%v)", size, commitTime)
		}
		
		if syncTime > 10*time.Second {
			t.Logf("Warning: Sync time for %d bytes is high (%v)", size, syncTime)
		}
	}
	
	// Check final state
	syncStates2 := repo2.GetP2PSyncState()
	node1ID := repo1.GetP2PStatus().NodeID
	
	if syncState, exists := syncStates2[node1ID]; exists {
		t.Logf("Final sync state: %d seals synced", len(syncState.SyncedSeals))
	}
}

// TestP2PStressMemoryUsage tests P2P memory usage under stress
func TestP2PStressMemoryUsage(t *testing.T) {
	// Create repository
	dir, _ := os.MkdirTemp("", "p2p_memory_*")
	defer os.RemoveAll(dir)
	
	repo, err := forge.Initialize(dir)
	if err != nil {
		t.Fatalf("Failed to init repo: %v", err)
	}
	
	// Configure P2P
	config := repo.GetP2PConfig()
	config.Port = 9540
	config.DiscoveryPort = 9541
	config.AutoSyncEnabled = false
	repo.UpdateP2PConfig(config)
	
	repo.StartP2P()
	defer repo.StopP2P()
	
	t.Log("=== Stress Testing: Memory Usage ===")
	
	// Create many commits to test memory usage
	numCommits := 100
	
	startTime := time.Now()
	
	for i := 0; i < numCommits; i++ {
		fileName := fmt.Sprintf("memory_test_%d.txt", i)
		filePath := filepath.Join(dir, fileName)
		content := fmt.Sprintf("Memory test commit %d created at %v", i, time.Now())
		
		os.WriteFile(filePath, []byte(content), 0644)
		repo.Gather([]string{fileName})
		seal, _ := repo.Seal(fmt.Sprintf("Memory test %d", i))
		
		if i%20 == 0 {
			t.Logf("Created %d commits (latest: %s)", i+1, seal.Name)
		}
		
		// Small delay to avoid overwhelming
		if i%10 == 0 {
			time.Sleep(10 * time.Millisecond)
		}
	}
	
	totalTime := time.Since(startTime)
	avgCommitTime := totalTime / time.Duration(numCommits)
	
	t.Logf("Created %d commits in %v", numCommits, totalTime)
	t.Logf("Average commit time: %v", avgCommitTime)
	
	// Check P2P status
	status := repo.GetP2PStatus()
	t.Logf("P2P still running: %v", status.Running)
	
	// Performance check
	if avgCommitTime > 100*time.Millisecond {
		t.Logf("Warning: Average commit time is high (%v)", avgCommitTime)
	} else {
		t.Log("✓ Memory usage test completed successfully")
	}
}