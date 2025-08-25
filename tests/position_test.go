package tests

import (
	"io/ioutil"
	"os"
	"testing"
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/position"
)

func TestPositionManager(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hash1 := objects.NewHash([]byte("test1"))
	hash2 := objects.NewHash([]byte("test2"))

	if err := manager.SetPosition(hash1, "main"); err != nil {
		t.Fatalf("Failed to set position: %v", err)
	}

	current := manager.Current()
	if current.Hash != hash1 {
		t.Errorf("Expected current hash to be %x, got %x", hash1, current.Hash)
	}

	if current.Timeline != "main" {
		t.Errorf("Expected timeline 'main', got '%s'", current.Timeline)
	}

	if err := manager.SetPosition(hash2, "main"); err != nil {
		t.Fatalf("Failed to set second position: %v", err)
	}

	current = manager.Current()
	if current.Hash != hash2 {
		t.Errorf("Expected current hash to be %x, got %x", hash2, current.Hash)
	}
}

func TestMemorableNames(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hash := objects.NewHash([]byte("test"))
	name := "bright-river-42"

	if err := manager.SetMemorableName(hash, name); err != nil {
		t.Fatalf("Failed to set memorable name: %v", err)
	}

	retrievedName, exists := manager.GetMemorableName(hash)
	if !exists {
		t.Error("Expected memorable name to exist")
	}

	if retrievedName != name {
		t.Errorf("Expected name '%s', got '%s'", name, retrievedName)
	}
}

func TestReferenceParsingIteration(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hashes := make([]objects.Hash, 5)
	for i := 0; i < 5; i++ {
		hashes[i] = objects.NewHash([]byte(string(rune('a' + i))))
		if err := manager.SetPosition(hashes[i], "main"); err != nil {
			t.Fatalf("Failed to set position %d: %v", i, err)
		}
	}

	hash, err := manager.ParseReference("#0")
	if err != nil {
		t.Fatalf("Failed to parse iteration reference: %v", err)
	}

	if hash != hashes[0] {
		t.Errorf("Expected hash %x for #0, got %x", hashes[0], hash)
	}

	hash, err = manager.ParseReference("#-1")
	if err != nil {
		history := manager.GetHistory()
		t.Logf("History length: %d", len(history))
		for i, pos := range history {
			t.Logf("History[%d]: %x", i, pos.Hash)
		}
		t.Fatalf("Failed to parse relative iteration reference: %v", err)
	}

	if hash != hashes[3] {
		t.Errorf("Expected hash %x for #-1, got %x", hashes[3], hash)
	}
}

func TestReferenceParsingMemorableName(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hash := objects.NewHash([]byte("test"))
	name := "bright-river-42"

	if err := manager.SetMemorableName(hash, name); err != nil {
		t.Fatalf("Failed to set memorable name: %v", err)
	}

	retrievedHash, err := manager.ParseReference(name)
	if err != nil {
		t.Fatalf("Failed to parse memorable name reference: %v", err)
	}

	if retrievedHash != hash {
		t.Errorf("Expected hash %x for name '%s', got %x", hash, name, retrievedHash)
	}
}

func TestReferenceParsingNaturalLanguage(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hash1 := objects.NewHash([]byte("today"))
	hash2 := objects.NewHash([]byte("yesterday"))

	if err := manager.SetPosition(hash1, "main"); err != nil {
		t.Fatalf("Failed to set today position: %v", err)
	}

	if err := manager.SetPosition(hash2, "main"); err != nil {
		t.Fatalf("Failed to set yesterday position: %v", err)
	}

	time.Sleep(time.Millisecond)

	_, parseErr := manager.ParseReference("yesterday")
	if parseErr != nil {
		t.Fatalf("Failed to parse natural language reference: %v", parseErr)
	}
}

func TestAliases(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager := position.NewManager(tempDir)

	hash := objects.NewHash([]byte("test"))
	alias := "stable"

	if err := manager.AddAlias(alias, hash); err != nil {
		t.Fatalf("Failed to add alias: %v", err)
	}

	retrievedHash, err := manager.ParseReference(alias)
	if err != nil {
		t.Fatalf("Failed to parse alias reference: %v", err)
	}

	if retrievedHash != hash {
		t.Errorf("Expected hash %x for alias '%s', got %x", hash, alias, retrievedHash)
	}
}

func TestPositionPersistence(t *testing.T) {
	tempDir, err := ioutil.TempDir("", "ivaldi-test-position-")
	if err != nil {
		t.Fatalf("Failed to create temp directory: %v", err)
	}
	defer os.RemoveAll(tempDir)

	manager1 := position.NewManager(tempDir)

	hash := objects.NewHash([]byte("test"))
	name := "bright-river-42"

	if err := manager1.SetPosition(hash, "main"); err != nil {
		t.Fatalf("Failed to set position: %v", err)
	}

	if err := manager1.SetMemorableName(hash, name); err != nil {
		t.Fatalf("Failed to set memorable name: %v", err)
	}

	manager2 := position.NewManager(tempDir)
	if err := manager2.Load(); err != nil {
		t.Fatalf("Failed to load position state: %v", err)
	}

	current := manager2.Current()
	if current.Hash != hash {
		t.Errorf("Expected loaded hash to be %x, got %x", hash, current.Hash)
	}

	retrievedName, exists := manager2.GetMemorableName(hash)
	if !exists || retrievedName != name {
		t.Errorf("Expected loaded name to be '%s', got '%s' (exists: %v)", name, retrievedName, exists)
	}
}
