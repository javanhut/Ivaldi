package tests

import (
	"crypto/rand"
	"os"
	"path/filepath"
	"testing"

	"ivaldi/core/objects"
	"ivaldi/core/workspace"
	"ivaldi/storage/local"
)

func TestEnhancedWorkspace(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-workspace-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Create content-addressed store
	store, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create store: %v", err)
	}

	// Create workspace
	ws := workspace.New(tempDir, store)

	// Test 1: Create test files
	testFiles := map[string][]byte{
		"file1.txt":        []byte("This is file 1 content"),
		"file2.txt":        []byte("This is file 2 content"),
		"subdir/file3.txt": []byte("This is file 3 in subdir"),
	}

	for path, content := range testFiles {
		fullPath := filepath.Join(tempDir, path)
		if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
			t.Fatalf("Failed to create directory: %v", err)
		}
		if err := os.WriteFile(fullPath, content, 0644); err != nil {
			t.Fatalf("Failed to write test file %s: %v", path, err)
		}
	}

	// Test 2: Scan workspace
	if err := ws.Scan(); err != nil {
		t.Fatalf("Failed to scan workspace: %v", err)
	}

	// Verify file statuses
	for path := range testFiles {
		status, exists := ws.GetFileStatus(path)
		if !exists {
			t.Errorf("File %s not found in workspace", path)
		}
		if status != workspace.StatusAdded {
			t.Errorf("Expected file %s to have StatusAdded, got %v", path, status)
		}
	}

	// Test 3: Gather all changed files
	if err := ws.Gather([]string{"."}); err != nil {
		t.Fatalf("Failed to gather files: %v", err)
	}

	// Verify files are staged
	stagedFiles := ws.GetStagedFiles()
	if len(stagedFiles) != len(testFiles) {
		t.Errorf("Expected %d staged files, got %d", len(testFiles), len(stagedFiles))
	}

	for path := range testFiles {
		if !ws.IsFileStaged(path) {
			t.Errorf("File %s should be staged", path)
		}
	}

	// Test 4: Verify candidate tree is built
	candidateTree := ws.GetCandidateTree()
	if candidateTree == nil {
		t.Fatal("Candidate tree should not be nil after gathering")
	}

	entries := candidateTree.Entries
	if len(entries) != len(testFiles) {
		t.Errorf("Expected %d tree entries, got %d", len(testFiles), len(entries))
	}

	// Test 5: Verify blobs are stored in content store
	for _, entry := range entries {
		if !store.Exists(entry.Hash) {
			t.Errorf("Blob %s not found in store", entry.Hash.String())
		}

		// Verify blob content
		data, kind, err := store.Get(entry.Hash)
		if err != nil {
			t.Errorf("Failed to get blob %s: %v", entry.Hash.String(), err)
		}
		if kind != local.KindBlob {
			t.Errorf("Expected blob kind, got %v", kind)
		}

		// Check content matches
		expectedContent := testFiles[entry.Name]
		if string(data) != string(expectedContent) {
			t.Errorf("Blob content mismatch for %s", entry.Name)
		}
	}

	// Test 6: Save and load state
	timeline := "main"
	if err := ws.SaveState(timeline); err != nil {
		t.Fatalf("Failed to save workspace state: %v", err)
	}

	// Create new workspace and load state
	ws2 := workspace.New(tempDir, store)
	if err := ws2.LoadState(timeline); err != nil {
		t.Fatalf("Failed to load workspace state: %v", err)
	}

	// Verify state is preserved
	if len(ws2.GetStagedFiles()) != len(testFiles) {
		t.Errorf("Staged files not preserved after load")
	}

	candidateTree2 := ws2.GetCandidateTree()
	if candidateTree2 == nil {
		t.Error("Candidate tree not preserved after load")
	} else if len(candidateTree2.Entries) != len(testFiles) {
		t.Errorf("Tree entries not preserved after load")
	}

	t.Logf("Enhanced workspace test passed - %d files processed", len(testFiles))
}

func TestLargeFileStreaming(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-large-file-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Create content-addressed store
	store, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create store: %v", err)
	}

	// Create workspace
	ws := workspace.New(tempDir, store)

	// Create a large file (6MB to trigger streaming)
	largeFilePath := filepath.Join(tempDir, "large_file.bin")
	largeData := make([]byte, 6*1024*1024)
	if _, err := rand.Read(largeData); err != nil {
		t.Fatalf("Failed to generate large data: %v", err)
	}

	if err := os.WriteFile(largeFilePath, largeData, 0644); err != nil {
		t.Fatalf("Failed to write large file: %v", err)
	}

	// Scan and gather the large file
	if err := ws.Scan(); err != nil {
		t.Fatalf("Failed to scan workspace: %v", err)
	}

	if err := ws.Gather([]string{"large_file.bin"}); err != nil {
		t.Fatalf("Failed to gather large file: %v", err)
	}

	// Verify large file is processed correctly
	candidateTree := ws.GetCandidateTree()
	if candidateTree == nil || len(candidateTree.Entries) != 1 {
		t.Fatal("Large file not processed correctly")
	}

	entry := candidateTree.Entries[0]
	if !store.Exists(entry.Hash) {
		t.Error("Large file blob not stored")
	}

	// Verify content integrity
	retrievedData, kind, err := store.Get(entry.Hash)
	if err != nil {
		t.Fatalf("Failed to retrieve large file blob: %v", err)
	}

	if kind != local.KindBlob {
		t.Errorf("Expected blob kind, got %v", kind)
	}

	if len(retrievedData) != len(largeData) {
		t.Errorf("Large file size mismatch: expected %d, got %d", len(largeData), len(retrievedData))
	}

	// Verify hash integrity
	if err := store.Verify(entry.Hash); err != nil {
		t.Errorf("Large file hash verification failed: %v", err)
	}

	t.Logf("Large file streaming test passed - %d bytes processed", len(largeData))
}

func TestIgnorePatterns(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-ignore-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Create .ivaldiignore file
	ignoreContent := `# Test ignore patterns
*.tmp
*.log
build/
test_data/
`
	ignoreFile := filepath.Join(tempDir, ".ivaldiignore")
	if err := os.WriteFile(ignoreFile, []byte(ignoreContent), 0644); err != nil {
		t.Fatalf("Failed to create .ivaldiignore: %v", err)
	}

	// Create content-addressed store
	store, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create store: %v", err)
	}

	// Create workspace
	ws := workspace.New(tempDir, store)

	// Create test files - some should be ignored
	testFiles := map[string]bool{
		"normal.txt":         false, // should not be ignored
		"temp.tmp":           true,  // should be ignored
		"debug.log":          true,  // should be ignored
		"build/output.bin":   true,  // should be ignored
		"test_data/data.csv": true,  // should be ignored
		"src/main.go":        false, // should not be ignored
	}

	for path, shouldIgnore := range testFiles {
		fullPath := filepath.Join(tempDir, path)
		if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
			t.Fatalf("Failed to create directory for %s: %v", path, err)
		}
		content := []byte("test content for " + path)
		if err := os.WriteFile(fullPath, content, 0644); err != nil {
			t.Fatalf("Failed to write test file %s: %v", path, err)
		}

		// Test ignore check
		if ws.ShouldIgnore(path) != shouldIgnore {
			t.Errorf("File %s ignore status mismatch: expected %v, got %v",
				path, shouldIgnore, ws.ShouldIgnore(path))
		}
	}

	// Scan workspace
	if err := ws.Scan(); err != nil {
		t.Fatalf("Failed to scan workspace: %v", err)
	}

	// Verify only non-ignored files are tracked
	expectedTracked := 0
	for _, shouldIgnore := range testFiles {
		if !shouldIgnore {
			expectedTracked++
		}
	}

	trackedCount := 0
	for path := range testFiles {
		if _, exists := ws.GetFileStatus(path); exists {
			trackedCount++
		}
	}

	if trackedCount != expectedTracked {
		t.Errorf("Expected %d tracked files, got %d", expectedTracked, trackedCount)
	}

	t.Logf("Ignore patterns test passed - %d files properly ignored", len(testFiles)-expectedTracked)
}
