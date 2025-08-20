package tests

import (
    "os"
    "path/filepath"
    "testing"

    "ivaldi/pkg/core/timeline"
    "ivaldi/pkg/storage/objectstore"
)

func TestTimelineSwitching(t *testing.T) {
    // Create a temporary directory for testing
    tmpDir, err := os.MkdirTemp("", "ivaldi-test-*")
    if err != nil {
        t.Fatal(err)
    }
    defer os.RemoveAll(tmpDir)

    store := objectstore.New(tmpDir)

    // Initialize the repository
    if err := timeline.Initialize(tmpDir); err != nil {
        t.Fatal(err)
    }

    // Create test files
    file1 := filepath.Join(tmpDir, "file1.txt")
    file2 := filepath.Join(tmpDir, "file2.txt")
    
    if err := os.WriteFile(file1, []byte("initial content"), 0644); err != nil {
        t.Fatal(err)
    }
    if err := os.WriteFile(file2, []byte("second file"), 0644); err != nil {
        t.Fatal(err)
    }

    // Seal initial state
    if _, err := timeline.Seal(tmpDir, "initial commit", store); err != nil {
        t.Fatal(err)
    }

    // Create a new timeline
    if err := timeline.Create(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Switch to the new timeline
    if err := timeline.Switch(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Modify files on the feature timeline
    if err := os.WriteFile(file1, []byte("modified content"), 0644); err != nil {
        t.Fatal(err)
    }
    
    // Add a new file
    file3 := filepath.Join(tmpDir, "file3.txt")
    if err := os.WriteFile(file3, []byte("new file on feature"), 0644); err != nil {
        t.Fatal(err)
    }

    // Switch back to main - should preserve local changes
    if err := timeline.Switch(tmpDir, "main", store); err != nil {
        t.Fatal(err)
    }

    // Check that local changes are preserved
    content1, err := os.ReadFile(file1)
    if err != nil {
        t.Fatal(err)
    }
    if string(content1) != "modified content" {
        t.Errorf("Expected 'modified content', got '%s'", string(content1))
    }

    // Check that new file is preserved
    content3, err := os.ReadFile(file3)
    if err != nil {
        // File might not exist yet, which is fine for unsaved changes
        if !os.IsNotExist(err) {
            t.Fatal(err)
        }
    } else {
        if string(content3) != "new file on feature" {
            t.Errorf("Expected 'new file on feature', got '%s'", string(content3))
        }
    }

    // Seal the changes on main
    if _, err := timeline.Seal(tmpDir, "changes on main", store); err != nil {
        t.Fatal(err)
    }

    // Switch back to feature
    if err := timeline.Switch(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Files should still be there
    if _, err := os.Stat(file1); os.IsNotExist(err) {
        t.Error("file1 should exist after switching back")
    }
    if _, err := os.Stat(file2); os.IsNotExist(err) {
        t.Error("file2 should exist after switching back")
    }
    // file3 was committed on main, so it should be there
    if _, err := os.Stat(file3); os.IsNotExist(err) {
        t.Error("file3 should exist after switching back")
    }
}

func TestTimelineConflictDetection(t *testing.T) {
    tmpDir, err := os.MkdirTemp("", "ivaldi-conflict-*")
    if err != nil {
        t.Fatal(err)
    }
    defer os.RemoveAll(tmpDir)

    store := objectstore.New(tmpDir)

    if err := timeline.Initialize(tmpDir); err != nil {
        t.Fatal(err)
    }

    // Create a test file
    file1 := filepath.Join(tmpDir, "conflict.txt")
    if err := os.WriteFile(file1, []byte("base content"), 0644); err != nil {
        t.Fatal(err)
    }

    // Seal initial state
    if _, err := timeline.Seal(tmpDir, "initial", store); err != nil {
        t.Fatal(err)
    }

    // Create feature timeline
    if err := timeline.Create(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Switch to feature and modify
    if err := timeline.Switch(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }
    
    if err := os.WriteFile(file1, []byte("feature change"), 0644); err != nil {
        t.Fatal(err)
    }
    
    if _, err := timeline.Seal(tmpDir, "feature change", store); err != nil {
        t.Fatal(err)
    }

    // Switch back to main
    if err := timeline.Switch(tmpDir, "main", store); err != nil {
        t.Fatal(err)
    }
    
    // Modify the same file differently
    if err := os.WriteFile(file1, []byte("main change"), 0644); err != nil {
        t.Fatal(err)
    }
    
    if _, err := timeline.Seal(tmpDir, "main change", store); err != nil {
        t.Fatal(err)
    }

    // Now switch to feature with local changes - this should create a conflict
    if err := os.WriteFile(file1, []byte("local unsaved change"), 0644); err != nil {
        t.Fatal(err)
    }
    
    if err := timeline.Switch(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Check that conflict markers are present
    content, err := os.ReadFile(file1)
    if err != nil {
        t.Fatal(err)
    }
    
    contentStr := string(content)
    if !contains(contentStr, "<<<<<<<") || !contains(contentStr, ">>>>>>>") {
        t.Errorf("Expected conflict markers in file, got: %s", contentStr)
    }
}

func TestTimelineRecovery(t *testing.T) {
    tmpDir, err := os.MkdirTemp("", "ivaldi-recovery-*")
    if err != nil {
        t.Fatal(err)
    }
    defer os.RemoveAll(tmpDir)

    store := objectstore.New(tmpDir)

    if err := timeline.Initialize(tmpDir); err != nil {
        t.Fatal(err)
    }

    // Create test files
    file1 := filepath.Join(tmpDir, "recovery.txt")
    if err := os.WriteFile(file1, []byte("initial"), 0644); err != nil {
        t.Fatal(err)
    }

    if _, err := timeline.Seal(tmpDir, "initial", store); err != nil {
        t.Fatal(err)
    }

    // Create and switch to feature timeline
    if err := timeline.Create(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }

    // Simulate an interrupted switch by manually creating a WAL file
    walPath := filepath.Join(tmpDir, ".ivaldi", "journal", "switch.json")
    os.MkdirAll(filepath.Dir(walPath), 0755)
    walContent := `{
        "from_tree": "dummy",
        "to_commit": "dummy",
        "phase": "stashed"
    }`
    if err := os.WriteFile(walPath, []byte(walContent), 0644); err != nil {
        t.Fatal(err)
    }

    // Initialize should recover from the incomplete switch
    if err := timeline.Initialize(tmpDir); err != nil {
        t.Fatal(err)
    }

    // WAL file should be cleared after recovery
    if _, err := os.Stat(walPath); !os.IsNotExist(err) {
        t.Error("WAL file should be cleared after recovery")
    }

    // System should be functional
    if err := timeline.Switch(tmpDir, "feature", store); err != nil {
        t.Fatal(err)
    }
}

func TestTimelineListAndCurrent(t *testing.T) {
    tmpDir, err := os.MkdirTemp("", "ivaldi-list-*")
    if err != nil {
        t.Fatal(err)
    }
    defer os.RemoveAll(tmpDir)

    store := objectstore.New(tmpDir)

    if err := timeline.Initialize(tmpDir); err != nil {
        t.Fatal(err)
    }

    // Create multiple timelines
    for _, name := range []string{"feature1", "feature2", "bugfix"} {
        if err := timeline.Create(tmpDir, name, store); err != nil {
            t.Fatal(err)
        }
    }

    // List all timelines
    names, err := timeline.List(tmpDir)
    if err != nil {
        t.Fatal(err)
    }

    if len(names) != 4 { // main + 3 created
        t.Errorf("Expected 4 timelines, got %d", len(names))
    }

    // Check current timeline
    current, err := timeline.Current(tmpDir)
    if err != nil {
        t.Fatal(err)
    }
    if current != "main" {
        t.Errorf("Expected current timeline to be 'main', got '%s'", current)
    }

    // Switch and verify current
    if err := timeline.Switch(tmpDir, "feature1", store); err != nil {
        t.Fatal(err)
    }

    current, err = timeline.Current(tmpDir)
    if err != nil {
        t.Fatal(err)
    }
    if current != "feature1" {
        t.Errorf("Expected current timeline to be 'feature1', got '%s'", current)
    }
}

func contains(s, substr string) bool {
    return len(s) > 0 && len(substr) > 0 && 
           (s == substr || len(s) > len(substr) && 
            (s[:len(substr)] == substr || 
             s[len(s)-len(substr):] == substr || 
             containsMiddle(s, substr)))
}

func containsMiddle(s, substr string) bool {
    for i := 1; i < len(s)-len(substr); i++ {
        if s[i:i+len(substr)] == substr {
            return true
        }
    }
    return false
}