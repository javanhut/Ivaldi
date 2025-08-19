package tests

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"ivaldi/forge"
	"ivaldi/core/objects"
)

func TestSealCreation(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-seal-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}

	// Test 1: Attempt to seal with nothing gathered (should fail)
	_, err = repo.Seal("empty seal")
	if err == nil {
		t.Error("Expected error when sealing with nothing gathered")
	}
	if err.Error() != "nothing gathered on the anvil to seal" {
		t.Errorf("Expected specific error message, got: %v", err)
	}

	// Test 2: Create test files and gather them
	testFiles := map[string]string{
		"file1.txt": "Content of file 1",
		"file2.txt": "Content of file 2", 
		"dir/file3.txt": "Content of file 3 in directory",
	}

	for path, content := range testFiles {
		fullPath := filepath.Join(tempDir, path)
		if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
			t.Fatalf("Failed to create directory: %v", err)
		}
		if err := os.WriteFile(fullPath, []byte(content), 0644); err != nil {
			t.Fatalf("Failed to write test file %s: %v", path, err)
		}
	}

	// Gather files
	if err := repo.Gather([]string{"."}); err != nil {
		t.Fatalf("Failed to gather files: %v", err)
	}

	// Test 3: Create first seal (iteration=1, no parents)
	firstSeal, err := repo.Seal("First commit")
	if err != nil {
		t.Fatalf("Failed to create first seal: %v", err)
	}

	// Verify first seal properties
	if firstSeal.Iteration != 1 {
		t.Errorf("Expected first seal iteration=1, got %d", firstSeal.Iteration)
	}

	if len(firstSeal.Parents) != 0 {
		t.Errorf("Expected first seal to have no parents, got %d", len(firstSeal.Parents))
	}

	if firstSeal.Message != "First commit" {
		t.Errorf("Expected message 'First commit', got '%s'", firstSeal.Message)
	}

	if firstSeal.Name == "" {
		t.Error("Expected memorable name to be set")
	}

	if firstSeal.Author.Name == "" {
		t.Error("Expected author to be set")
	}

	t.Logf("First seal created: %s (iteration %d)", firstSeal.Name, firstSeal.Iteration)

	// Test 4: Verify content-addressed objects were stored
	ws := repo.GetWorkspace()
	store := ws.Store

	// Check that the tree and seal were stored in content-addressed format
	stats, err := store.Stats()
	if err != nil {
		t.Errorf("Failed to get store stats: %v", err)
	} else {
		t.Logf("Store stats: %d objects (%d blobs, %d trees, %d seals)", 
			stats.ObjectCount, stats.BlobCount, stats.TreeCount, stats.SealCount)

		if stats.TreeCount < 1 {
			t.Error("Expected at least 1 tree to be stored")
		}
		
		if stats.SealCount < 1 {
			t.Error("Expected at least 1 seal to be stored")
		}

		if stats.BlobCount < 3 {
			t.Errorf("Expected at least 3 blobs (for files), got %d", stats.BlobCount)
		}
	}

	// Test 5: Modify a file and create second seal
	modifiedContent := "Modified content of file 1"
	modifiedPath := filepath.Join(tempDir, "file1.txt")
	if err := os.WriteFile(modifiedPath, []byte(modifiedContent), 0644); err != nil {
		t.Fatalf("Failed to modify file: %v", err)
	}

	// Add a new file
	newFilePath := filepath.Join(tempDir, "file4.txt")
	if err := os.WriteFile(newFilePath, []byte("New file content"), 0644); err != nil {
		t.Fatalf("Failed to create new file: %v", err)
	}

	// Gather changes
	if err := repo.Gather([]string{"."}); err != nil {
		t.Fatalf("Failed to gather changes: %v", err)
	}

	// Create second seal (should have parent)
	secondSeal, err := repo.Seal("Second commit with changes")
	if err != nil {
		t.Fatalf("Failed to create second seal: %v", err)
	}

	// Verify second seal properties
	if secondSeal.Iteration != 2 {
		t.Errorf("Expected second seal iteration=2, got %d", secondSeal.Iteration)
	}

	if secondSeal.Message != "Second commit with changes" {
		t.Errorf("Expected message 'Second commit with changes', got '%s'", secondSeal.Message)
	}

	if secondSeal.Name == "" {
		t.Error("Expected memorable name to be set")
	}

	if secondSeal.Name == firstSeal.Name {
		t.Error("Expected different memorable names for different seals")
	}

	t.Logf("Second seal created: %s (iteration %d)", secondSeal.Name, secondSeal.Iteration)

	// Test 6: Verify workspace is clean after sealing
	if len(ws.AnvilFiles) != 0 {
		t.Errorf("Expected anvil to be empty after sealing, got %d files", len(ws.AnvilFiles))
	}

	// Verify files are marked as unmodified
	for path := range testFiles {
		if status, exists := ws.GetFileStatus(path); exists {
			if status != 0 { // StatusUnmodified = 0
				t.Errorf("Expected file %s to be unmodified after sealing, got status %d", path, status)
			}
		}
	}

	// Test 7: Verify seal mapping was stored
	mappingPath := filepath.Join(tempDir, ".ivaldi", "seal_mapping.json")
	if _, err := os.Stat(mappingPath); os.IsNotExist(err) {
		t.Error("Expected seal mapping file to be created")
	}

	t.Logf("Seal creation tests passed successfully")
}

func TestSealChaining(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-chain-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}

	// Create multiple seals to test chaining
	var seals []*objects.Seal
	
	for i := 1; i <= 3; i++ {
		// Create and write a file
		fileName := filepath.Join(tempDir, fmt.Sprintf("file%d.txt", i))
		content := fmt.Sprintf("Content of file %d", i)
		if err := os.WriteFile(fileName, []byte(content), 0644); err != nil {
			t.Fatalf("Failed to write file %d: %v", i, err)
		}

		// Gather and seal
		if err := repo.Gather([]string{"."}); err != nil {
			t.Fatalf("Failed to gather files for seal %d: %v", i, err)
		}

		message := fmt.Sprintf("Commit %d", i)
		seal, err := repo.Seal(message)
		if err != nil {
			t.Fatalf("Failed to create seal %d: %v", i, err)
		}

		seals = append(seals, seal)

		// Verify iteration number
		if seal.Iteration != i {
			t.Errorf("Expected seal %d to have iteration %d, got %d", i, i, seal.Iteration)
		}

		t.Logf("Created seal %d: %s (iteration %d)", i, seal.Name, seal.Iteration)
	}

	// Verify seals have different names and proper iteration sequence
	namesSeen := make(map[string]bool)
	for i, seal := range seals {
		if namesSeen[seal.Name] {
			t.Errorf("Duplicate memorable name found: %s", seal.Name)
		}
		namesSeen[seal.Name] = true

		expectedIteration := i + 1
		if seal.Iteration != expectedIteration {
			t.Errorf("Seal %d has wrong iteration: expected %d, got %d", 
				i, expectedIteration, seal.Iteration)
		}
	}

	t.Logf("Seal chaining tests passed successfully")
}

func TestSealWithEmptyDirectory(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-empty-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}

	// Create an empty directory structure
	emptyDir := filepath.Join(tempDir, "empty", "nested")
	if err := os.MkdirAll(emptyDir, 0755); err != nil {
		t.Fatalf("Failed to create empty directory: %v", err)
	}

	// Create a single file
	testFile := filepath.Join(tempDir, "single.txt")
	if err := os.WriteFile(testFile, []byte("lone file"), 0644); err != nil {
		t.Fatalf("Failed to write test file: %v", err)
	}

	// Gather and seal
	if err := repo.Gather([]string{"."}); err != nil {
		t.Fatalf("Failed to gather files: %v", err)
	}

	seal, err := repo.Seal("Single file commit")
	if err != nil {
		t.Fatalf("Failed to create seal: %v", err)
	}

	// Verify seal was created successfully
	if seal == nil {
		t.Fatal("Expected seal to be created")
	}

	if seal.Message != "Single file commit" {
		t.Errorf("Expected message 'Single file commit', got '%s'", seal.Message)
	}

	t.Logf("Empty directory test passed: %s", seal.Name)
}