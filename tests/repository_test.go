package tests

import (
	"fmt"
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"ivaldi/forge"
)

func TestRepositoryInitialization(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	defer repo.Close()

	if repo.Root() != tempDir {
		t.Errorf("Expected root %s, got %s", tempDir, repo.Root())
	}

	if _, err := os.Stat(filepath.Join(tempDir, ".ivaldi")); os.IsNotExist(err) {
		t.Error("Expected .ivaldi directory to be created")
	}
}

func TestRepositoryOpen(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo1, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	repo1.Close()

	repo2, err := forge.Open(tempDir)
	if err != nil {
		t.Fatalf("Failed to open repository: %v", err)
	}
	defer repo2.Close()

	if repo2.Root() != tempDir {
		t.Errorf("Expected root %s, got %s", tempDir, repo2.Root())
	}
}

func TestBasicWorkflow(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	defer repo.Close()

	testFile := filepath.Join(tempDir, "test.txt")
	if err := ioutil.WriteFile(testFile, []byte("Hello, Ivaldi!"), 0644); err != nil {
		t.Fatalf("Failed to create test file: %v", err)
	}

	if err := repo.Gather([]string{"test.txt"}); err != nil {
		t.Fatalf("Failed to gather file: %v", err)
	}

	seal, err := repo.Seal("Initial test seal")
	if err != nil {
		t.Fatalf("Failed to create seal: %v", err)
	}

	if seal.Message != "Initial test seal" {
		t.Errorf("Expected message 'Initial test seal', got '%s'", seal.Message)
	}

	if seal.Name == "" {
		t.Error("Expected seal to have a memorable name")
	}

	if seal.Iteration != 1 {
		t.Errorf("Expected first seal to have iteration 1, got %d", seal.Iteration)
	}
}

func TestTimelineManagement(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	defer repo.Close()

	if repo.CurrentTimeline() != "main" {
		t.Errorf("Expected initial timeline to be 'main', got '%s'", repo.CurrentTimeline())
	}

	if err := repo.CreateTimeline("feature", "Feature development timeline"); err != nil {
		t.Fatalf("Failed to create timeline: %v", err)
	}

	timelines := repo.ListTimelines()
	if len(timelines) != 2 {
		t.Errorf("Expected 2 timelines, got %d", len(timelines))
	}

	var foundFeature bool
	for _, timeline := range timelines {
		if timeline.Name == "feature" {
			foundFeature = true
			if timeline.Description != "Feature development timeline" {
				t.Errorf("Expected feature timeline description, got '%s'", timeline.Description)
			}
		}
	}

	if !foundFeature {
		t.Error("Expected to find 'feature' timeline")
	}

	if err := repo.SwitchTimeline("feature"); err != nil {
		t.Fatalf("Failed to switch timeline: %v", err)
	}

	if repo.CurrentTimeline() != "feature" {
		t.Errorf("Expected current timeline to be 'feature', got '%s'", repo.CurrentTimeline())
	}
}

func TestWorkspaceStatus(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	defer repo.Close()

	testFile := filepath.Join(tempDir, "test.txt")
	if err := ioutil.WriteFile(testFile, []byte("Hello, Ivaldi!"), 0644); err != nil {
		t.Fatalf("Failed to create test file: %v", err)
	}

	status := repo.Status()
	
	if len(status.Untracked) != 1 || status.Untracked[0] != "test.txt" {
		t.Errorf("Expected 1 untracked file 'test.txt', got %v", status.Untracked)
	}

	if err := repo.Gather([]string{"test.txt"}); err != nil {
		t.Fatalf("Failed to gather file: %v", err)
	}

	status = repo.Status()
	
	if len(status.Gathered) != 1 || status.Gathered[0] != "test.txt" {
		t.Errorf("Expected 1 gathered file 'test.txt', got %v", status.Gathered)
	}

	if len(status.Untracked) != 0 {
		t.Errorf("Expected 0 untracked files after gathering, got %v", status.Untracked)
	}
}

func TestSealHistory(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}
	defer repo.Close()

	for i := 1; i <= 3; i++ {
		testFile := filepath.Join(tempDir, fmt.Sprintf("test%d.txt", i))
		if err := ioutil.WriteFile(testFile, []byte(fmt.Sprintf("Content %d", i)), 0644); err != nil {
			t.Fatalf("Failed to create test file %d: %v", i, err)
		}

		if err := repo.Gather([]string{fmt.Sprintf("test%d.txt", i)}); err != nil {
			t.Fatalf("Failed to gather file %d: %v", i, err)
		}

		if _, err := repo.Seal(fmt.Sprintf("Seal %d", i)); err != nil {
			t.Fatalf("Failed to create seal %d: %v", i, err)
		}
	}

	history, err := repo.History(10)
	if err != nil {
		t.Fatalf("Failed to get history: %v", err)
	}

	if len(history) != 3 {
		t.Errorf("Expected 3 seals in history, got %d", len(history))
	}

	for i, seal := range history {
		expectedMessage := fmt.Sprintf("Seal %d", 3-i)
		if seal.Message != expectedMessage {
			t.Errorf("Expected seal %d to have message '%s', got '%s'", i, expectedMessage, seal.Message)
		}
	}
}