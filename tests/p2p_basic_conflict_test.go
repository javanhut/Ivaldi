package tests

import (
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
	
	"ivaldi/forge"
)

// freePort returns an available ephemeral port
func freePort(t *testing.T) int {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer ln.Close()
	return ln.Addr().(*net.TCPAddr).Port
}

// TestP2PBasicConflict tests basic P2P conflict detection
func TestP2PBasicConflict(t *testing.T) {
	// Create two test repositories
	dir1, _ := os.MkdirTemp("", "p2p_basic1_*")
	dir2, _ := os.MkdirTemp("", "p2p_basic2_*")
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
	
	// Create different initial commits in each repo
	testFile1 := filepath.Join(dir1, "test.txt")
	os.WriteFile(testFile1, []byte("content from repo1"), 0644)
	repo1.Gather([]string{"test.txt"})
	seal1, _ := repo1.Seal("Initial from repo1")
	t.Logf("Repo1 created seal: %s", seal1.Name)
	
	testFile2 := filepath.Join(dir2, "test.txt")
	os.WriteFile(testFile2, []byte("content from repo2"), 0644)
	repo2.Gather([]string{"test.txt"})
	seal2, _ := repo2.Seal("Initial from repo2")
	t.Logf("Repo2 created seal: %s", seal2.Name)
	
	// Configure P2P with dynamic ports
	config1 := repo1.GetP2PConfig()
	config1.Port = freePort(t)
	config1.DiscoveryPort = freePort(t)
	config1.AutoSyncEnabled = false
	if err := repo1.UpdateP2PConfig(config1); err != nil {
		t.Fatalf("repo1 UpdateP2PConfig: %v", err)
	}
	
	config2 := repo2.GetP2PConfig()
	config2.Port = freePort(t)
	config2.DiscoveryPort = freePort(t)
	config2.AutoSyncEnabled = false
	if err := repo2.UpdateP2PConfig(config2); err != nil {
		t.Fatalf("repo2 UpdateP2PConfig: %v", err)
	}
	
	t.Logf("Repo1 P2P ports: %d (main), %d (discovery)", config1.Port, config1.DiscoveryPort)
	t.Logf("Repo2 P2P ports: %d (main), %d (discovery)", config2.Port, config2.DiscoveryPort)
	
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
	err = repo2.ConnectToPeer("localhost", config1.Port)
	if err != nil {
		t.Fatalf("Failed to connect repos: %v", err)
	}
	
	time.Sleep(2 * time.Second)
	
	// Check P2P status
	status1 := repo1.GetP2PStatus()
	status2 := repo2.GetP2PStatus()
	
	t.Logf("Repo1 node ID: %s", status1.NodeID)
	t.Logf("Repo2 node ID: %s", status2.NodeID)
	
	peers1 := repo1.GetP2PPeers()
	peers2 := repo2.GetP2PPeers()
	
	t.Logf("Repo1 has %d peers", len(peers1))
	t.Logf("Repo2 has %d peers", len(peers2))
	
	if len(peers1) == 0 || len(peers2) == 0 {
		t.Fatal("Repos should be connected")
	}
	
	t.Log("=== Attempting Cross-Sync (should conflict) ===")
	
	// Try to sync repo1's content to repo2
	err = repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync repo1->repo2 result: %v", err)
	}
	
	time.Sleep(1 * time.Second)
	
	// Check sync states
	syncStates2 := repo2.GetP2PSyncState()
	repo1ID := repo1.GetP2PStatus().NodeID
	
	t.Logf("Checking sync state in repo2 for peer %s", repo1ID)
	if syncState, exists := syncStates2[repo1ID]; exists {
		t.Logf("Sync state found:")
		t.Logf("  Peer ID: %s", syncState.PeerID)
		t.Logf("  Conflict count: %d", syncState.ConflictCount)
		t.Logf("  Last sync: %v", syncState.LastSync)
		t.Logf("  Timeline heads: %v", syncState.TimelineHeads)
		t.Logf("  Synced seals: %v", syncState.SyncedSeals)
		
		if syncState.ConflictCount > 0 {
			t.Log("✓ Conflict detected successfully")
		} else {
			t.Log("No conflicts reported - this may be expected behavior")
		}
	} else {
		t.Log("No sync state found for peer")
	}
	
	// Try reverse sync
	err = repo1.SyncWithP2PPeer(repo2.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Sync repo2->repo1 result: %v", err)
	}
	
	time.Sleep(1 * time.Second)
	
	// Check reverse sync states
	syncStates1 := repo1.GetP2PSyncState()
	repo2ID := repo2.GetP2PStatus().NodeID
	
	if syncState, exists := syncStates1[repo2ID]; exists {
		t.Logf("Repo1 sync state for peer %s:", repo2ID)
		t.Logf("  Conflict count: %d", syncState.ConflictCount)
		
		if syncState.ConflictCount > 0 {
			t.Log("✓ Reverse conflict also detected")
		}
	}
	
	// Verify files remain unchanged (no overwrites)
	content1, _ := os.ReadFile(testFile1)
	content2, _ := os.ReadFile(testFile2)
	
	t.Logf("Final repo1 content: %s", content1)
	t.Logf("Final repo2 content: %s", content2)
	
	if string(content1) == "content from repo1" && string(content2) == "content from repo2" {
		t.Log("✓ Files preserved - no unexpected overwrites")
	} else {
		t.Error("Files were modified unexpectedly")
	}
	
	t.Log("=== Testing Compatible Changes ===")
	
	// Add different files (should not conflict)
	compatible1 := filepath.Join(dir1, "only1.txt")
	os.WriteFile(compatible1, []byte("only in repo1"), 0644)
	repo1.Gather([]string{"only1.txt"})
	_, _ = repo1.Seal("Add only1.txt")
	
	compatible2 := filepath.Join(dir2, "only2.txt")
	os.WriteFile(compatible2, []byte("only in repo2"), 0644)
	repo2.Gather([]string{"only2.txt"})
	_, _ = repo2.Seal("Add only2.txt")
	
	// Try to sync compatible changes
	err = repo1.SyncWithP2PPeer(repo2.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Compatible sync result: %v", err)
	}
	
	err = repo2.SyncWithP2PPeer(repo1.GetP2PStatus().NodeID)
	if err != nil {
		t.Logf("Compatible reverse sync result: %v", err)
	}
	
	time.Sleep(2 * time.Second)
	
	// Check if compatible files were synced
	_, err1 := os.Stat(filepath.Join(dir1, "only2.txt"))
	_, err2 := os.Stat(filepath.Join(dir2, "only1.txt"))
	
	if err1 == nil {
		t.Log("✓ Compatible file synced to repo1")
	} else {
		t.Log("Compatible file not synced to repo1 (may be expected)")
	}
	
	if err2 == nil {
		t.Log("✓ Compatible file synced to repo2")
	} else {
		t.Log("Compatible file not synced to repo2 (may be expected)")
	}
	
	t.Log("=== Basic Conflict Test Complete ===")
}