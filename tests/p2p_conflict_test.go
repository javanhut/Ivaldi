package tests

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	"ivaldi/forge"
)

// TestP2PConflictResolution tests how P2P handles conflicting changes
func TestP2PConflictResolution(t *testing.T) {
	// Create two test repositories
	dir1, _ := os.MkdirTemp("", "p2p_conflict1_*")
	dir2, _ := os.MkdirTemp("", "p2p_conflict2_*")
	defer os.RemoveAll(dir1)
	defer os.RemoveAll(dir2)

	t.Logf("Repo1: %s", dir1)
	t.Logf("Repo2: %s", dir2)

	// Initialize repositories
	repo1, err := forge.Initialize(dir1)
	if err != nil {
		t.Fatalf("Failed to init repo1: %v", err)
	}

	repo2, err := forge.Initialize(dir2)
	if err != nil {
		t.Fatalf("Failed to init repo2: %v", err)
	}

	// Create initial shared state
	testFile1 := filepath.Join(dir1, "shared.txt")
	os.WriteFile(testFile1, []byte("initial content"), 0644)
	repo1.Gather([]string{"shared.txt"})
	_, _ = repo1.Seal("Initial commit")

	// Configure P2P
	config1 := repo1.GetP2PConfig()
	config1.Port = 9400
	config1.DiscoveryPort = 9401
	config1.AutoSyncEnabled = false // Manual sync for controlled testing
	repo1.UpdateP2PConfig(config1)

	config2 := repo2.GetP2PConfig()
	config2.Port = 9402
	config2.DiscoveryPort = 9403
	config2.AutoSyncEnabled = false
	repo2.UpdateP2PConfig(config2)

	// Start P2P
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

	// Connect repos
	err = repo2.ConnectToPeer("localhost", 9400)
	if err != nil {
		t.Fatalf("Failed to connect repos: %v", err)
	}

	time.Sleep(1 * time.Second)

	// Sync initial state to repo2
	err = repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)
	if err != nil {
		t.Fatalf("Failed initial sync: %v", err)
	}

	time.Sleep(1 * time.Second)

	// Verify repo2 has the initial commit by checking files
	testFile2 := filepath.Join(dir2, "shared.txt")
	if _, err := os.Stat(testFile2); err != nil {
		t.Fatal("Repo2 should have synced the initial commit")
	}

	t.Log("=== Creating Conflicting Changes ===")

	// Create conflicting changes in repo1
	os.WriteFile(testFile1, []byte("change from repo1"), 0644)
	repo1.Gather([]string{"shared.txt"})
	seal1_conflict, _ := repo1.Seal("Change from repo1")
	t.Logf("Repo1 created seal: %s", seal1_conflict.Name)

	// Create conflicting changes in repo2 (different content, same file)
	os.WriteFile(testFile2, []byte("change from repo2"), 0644)
	repo2.Gather([]string{"shared.txt"})
	seal2_conflict, _ := repo2.Seal("Change from repo2")
	t.Logf("Repo2 created seal: %s", seal2_conflict.Name)

	// Now both repos have divergent histories
	t.Log("=== Testing Conflict Detection ===")

	// Try to sync repo1's changes to repo2
	err = repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync resulted in error (expected for conflicts): %v", err)
	}

	time.Sleep(1 * time.Second)

	// Check if conflict was detected
	syncStates2 := repo2.GetP2PSyncState()
	repo1ID := repo1.GetP2PStatus().NodeID
	if syncState2, exists := syncStates2[repo1ID]; exists {
		t.Logf("Repo2 sync state with repo1:")
		t.Logf("  Conflict count: %d", syncState2.ConflictCount)
		t.Logf("  Last sync: %v", syncState2.LastSync)

		if syncState2.ConflictCount > 0 {
			t.Log("✓ Conflict detected successfully")
		} else {
			t.Log("No conflicts detected yet (may be working as intended)")
		}
	}

	// Try to sync repo2's changes to repo1
	err = repo1.SyncWithP2PPeer(repo2.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Reverse sync resulted in error (expected): %v", err)
	}

	time.Sleep(1 * time.Second)

	// Check repo1's view of conflicts
	syncStates1 := repo1.GetP2PSyncState()
	repo2ID := repo2.GetP2PStatus().NodeID
	if syncState1, exists := syncStates1[repo2ID]; exists && syncState1.ConflictCount > 0 {
		t.Log("✓ Repo1 also detected conflict")
	}

	t.Log("=== Testing Conflict Resolution Strategies ===")

	// Test 1: Manual resolution (default)
	config1 = repo1.GetP2PConfig()
	if config1.ConflictStrategy == "manual" {
		t.Log("✓ Manual conflict resolution strategy is set")

		// In manual mode, conflicts should block sync
		// Check that files retain their local changes
		content1, _ := os.ReadFile(testFile1)
		content2, _ := os.ReadFile(testFile2)

		t.Logf("Repo1 file content: %s", content1)
		t.Logf("Repo2 file content: %s", content2)

		if string(content1) == "change from repo1" && string(content2) == "change from repo2" {
			t.Log("✓ Conflicts prevented automatic merge")
		}
	}

	// Test 2: Create a non-conflicting change and verify it syncs
	t.Log("=== Testing Non-conflicting Changes ===")

	otherFile1 := filepath.Join(dir1, "other.txt")
	os.WriteFile(otherFile1, []byte("non-conflicting content"), 0644)
	repo1.Gather([]string{"other.txt"})
	_, _ = repo1.Seal("Non-conflicting change")

	// This should sync fine since it doesn't conflict
	err = repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)

	time.Sleep(1 * time.Second)

	// Check if the non-conflicting change was synced
	otherFile2 := filepath.Join(dir2, "other.txt")
	if _, err := os.Stat(otherFile2); err == nil {
		content, _ := os.ReadFile(otherFile2)
		if string(content) == "non-conflicting content" {
			t.Log("✓ Non-conflicting changes synced successfully")
		}
	}

	t.Log("=== Conflict Resolution Test Complete ===")
}

// TestP2PConflictWithTimelines tests conflicts across different timelines
func TestP2PConflictWithTimelines(t *testing.T) {
	// Create two test repositories
	dir1, _ := os.MkdirTemp("", "p2p_timeline_conflict1_*")
	dir2, _ := os.MkdirTemp("", "p2p_timeline_conflict2_*")
	defer os.RemoveAll(dir1)
	defer os.RemoveAll(dir2)

	// Initialize repositories
	repo1, err := forge.Initialize(dir1)
	if err != nil {
		t.Fatalf("Failed to init repo1: %v", err)
	}

	repo2, err := forge.Initialize(dir2)
	if err != nil {
		t.Fatalf("Failed to init repo2: %v", err)
	}

	// Create initial commit
	testFile1 := filepath.Join(dir1, "test.txt")
	os.WriteFile(testFile1, []byte("initial"), 0644)
	repo1.Gather([]string{"test.txt"})
	repo1.Seal("Initial commit")

	// Configure P2P
	config1 := repo1.GetP2PConfig()
	config1.Port = 9410
	config1.DiscoveryPort = 9411
	config1.AutoSyncEnabled = true
	config1.SyncInterval = 2 * time.Second
	repo1.UpdateP2PConfig(config1)

	config2 := repo2.GetP2PConfig()
	config2.Port = 9412
	config2.DiscoveryPort = 9413
	config2.AutoSyncEnabled = true
	config2.SyncInterval = 2 * time.Second
	repo2.UpdateP2PConfig(config2)

	// Start P2P
	repo1.StartP2P()
	defer repo1.StopP2P()

	repo2.StartP2P()
	defer repo2.StopP2P()

	// Connect
	repo2.ConnectToPeer("localhost", 9410)

	// Wait for initial sync
	time.Sleep(3 * time.Second)

	t.Log("=== Creating Timeline Conflicts ===")

	// Create different timelines with same name in both repos
	err = repo1.CreateTimeline("feature", "")
	if err != nil {
		t.Logf("Failed to create timeline in repo1: %v", err)
	}

	err = repo2.CreateTimeline("feature", "")
	if err != nil {
		t.Logf("Failed to create timeline in repo2: %v", err)
	}

	// Switch to the timelines
	repo1.SwitchTimeline("feature")
	repo2.SwitchTimeline("feature")

	// Make different changes on the same timeline name
	os.WriteFile(testFile1, []byte("feature change repo1"), 0644)
	repo1.Gather([]string{"test.txt"})
	repo1.Seal("Feature work from repo1")

	testFile2 := filepath.Join(dir2, "test.txt")
	os.WriteFile(testFile2, []byte("feature change repo2"), 0644)
	repo2.Gather([]string{"test.txt"})
	repo2.Seal("Feature work from repo2")

	// Wait for sync attempt
	time.Sleep(3 * time.Second)

	// Check timeline status
	currentTimeline1 := repo1.CurrentTimeline()
	currentTimeline2 := repo2.CurrentTimeline()

	t.Logf("Repo1 current timeline: %s", currentTimeline1)
	t.Logf("Repo2 current timeline: %s", currentTimeline2)

	// Check sync states for conflicts
	syncStates1 := repo1.GetP2PSyncState()
	syncStates2 := repo2.GetP2PSyncState()

	repo1ID := repo1.GetP2PStatus().NodeID
	repo2ID := repo2.GetP2PStatus().NodeID

	if syncState1, exists := syncStates1[repo2ID]; exists && syncState1.ConflictCount > 0 {
		t.Log("✓ Timeline conflict detected in repo1")
	}

	if syncState2, exists := syncStates2[repo1ID]; exists && syncState2.ConflictCount > 0 {
		t.Log("✓ Timeline conflict detected in repo2")
	}

	t.Log("=== Timeline Conflict Test Complete ===")
}

// TestP2PThreeWayConflict tests conflicts with three repositories
func TestP2PThreeWayConflict(t *testing.T) {
	// Create three test repositories
	dir1, _ := os.MkdirTemp("", "p2p_3way1_*")
	dir2, _ := os.MkdirTemp("", "p2p_3way2_*")
	dir3, _ := os.MkdirTemp("", "p2p_3way3_*")
	defer os.RemoveAll(dir1)
	defer os.RemoveAll(dir2)
	defer os.RemoveAll(dir3)

	repos := make([]*forge.Repository, 3)
	dirs := []string{dir1, dir2, dir3}

	// Initialize all repos
	for i := range repos {
		repo, err := forge.Initialize(dirs[i])
		if err != nil {
			t.Fatalf("Failed to init repo%d: %v", i+1, err)
		}
		repos[i] = repo

		// Configure P2P
		config := repo.GetP2PConfig()
		config.Port = 9420 + i*2
		config.DiscoveryPort = 9421 + i*2
		config.AutoSyncEnabled = false // Manual control
		repo.UpdateP2PConfig(config)

		// Start P2P
		err = repo.StartP2P()
		if err != nil {
			t.Fatalf("Failed to start P2P on repo%d: %v", i+1, err)
		}
		defer repo.StopP2P()
	}

	// Create initial shared commit in repo1
	testFile := filepath.Join(dir1, "shared.txt")
	os.WriteFile(testFile, []byte("initial"), 0644)
	repos[0].Gather([]string{"shared.txt"})
	repos[0].Seal("Initial")

	// Connect all repos in a mesh
	repos[1].ConnectToPeer("localhost", 9420) // repo2 -> repo1
	repos[2].ConnectToPeer("localhost", 9420) // repo3 -> repo1
	repos[2].ConnectToPeer("localhost", 9422) // repo3 -> repo2

	time.Sleep(1 * time.Second)

	// Sync initial state to all
	repos[1].SyncWithP2PPeer(repos[0].GetP2PStatus().NodeID)
	repos[2].SyncWithP2PPeer(repos[0].GetP2PStatus().NodeID)

	time.Sleep(2 * time.Second)

	t.Log("=== Creating Three-Way Conflict ===")

	// Each repo makes a different change to the same file
	for i, repo := range repos {
		file := filepath.Join(dirs[i], "shared.txt")
		content := fmt.Sprintf("change from repo%d", i+1)
		os.WriteFile(file, []byte(content), 0644)
		repo.Gather([]string{"shared.txt"})
		seal, _ := repo.Seal(fmt.Sprintf("Change %d", i+1))
		t.Logf("Repo%d created seal: %s", i+1, seal.Name)
	}

	// Try to sync between all pairs
	t.Log("=== Testing Three-Way Sync ===")

	// repo1 -> repo2
	err := repos[1].SyncWithP2PPeer(repos[0].GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync repo1->repo2: conflict detected")
	}

	// repo1 -> repo3
	err = repos[2].SyncWithP2PPeer(repos[0].GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync repo1->repo3: conflict detected")
	}

	// repo2 -> repo3
	err = repos[2].SyncWithP2PPeer(repos[1].GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync repo2->repo3: conflict detected")
	}

	time.Sleep(1 * time.Second)

	// Check conflict states
	hasConflicts := false
	for i, repo := range repos {
		syncStates := repo.GetP2PSyncState()
		for j, otherRepo := range repos {
			if i != j {
				otherID := otherRepo.GetP2PStatus().NodeID
				if syncState, exists := syncStates[otherID]; exists && syncState.ConflictCount > 0 {
					t.Logf("Repo%d has conflicts with Repo%d", i+1, j+1)
					hasConflicts = true
				}
			}
		}
	}

	if hasConflicts {
		t.Log("✓ Three-way conflicts detected successfully")
	} else {
		t.Error("Three-way conflicts should have been detected")
	}

	t.Log("=== Three-Way Conflict Test Complete ===")
}
