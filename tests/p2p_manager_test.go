package tests

import (
	"os"
	"testing"
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/p2p"
	"ivaldi/forge"
)

// TestDirectP2PManagerCreation tests creating P2P manager directly
func TestDirectP2PManagerCreation(t *testing.T) {
	// Create temporary directory
	tempDir, err := os.MkdirTemp("", "ivaldi-direct-p2p-*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	t.Logf("Testing P2P manager creation in: %s", tempDir)

	// Create a minimal repository for adapters
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repo: %v", err)
	}
	defer repo.Close()

	// Create real adapters using the repository
	storageAdapter := p2p.NewStorageAdapter(repo.GetStorage())
	timelineAdapter := p2p.NewTimelineAdapter(repo.GetTimelineManager())
	t.Log("Real adapters created successfully")

	// Try to create P2P manager with real adapters
	p2pMgr, err := p2p.NewP2PManager(tempDir, storageAdapter, timelineAdapter)
	if err != nil {
		t.Fatalf("Failed to create P2P manager with real adapters: %v", err)
	}

	t.Log("P2P manager created successfully with real adapters!")

	// Test basic functionality
	status := p2pMgr.GetStatus()
	t.Logf("P2P Status: %+v", status)

	config := p2pMgr.GetConfig()
	t.Logf("P2P Config port: %d", config.Port)

	t.Log("Direct P2P manager creation test completed successfully!")
}

// FakeStorage implements the P2P Storage interface for testing
type FakeStorage struct{}

func (fs *FakeStorage) LoadSeal(hash objects.Hash) (*objects.Seal, error) {
	return nil, nil
}

func (fs *FakeStorage) StoreSeal(seal *objects.Seal) error {
	return nil
}

func (fs *FakeStorage) LoadTree(hash objects.Hash) (*objects.Tree, error) {
	return nil, nil
}

func (fs *FakeStorage) LoadBlob(hash objects.Hash) (*objects.Blob, error) {
	return nil, nil
}

func (fs *FakeStorage) StoreTree(tree *objects.Tree) error {
	return nil
}

func (fs *FakeStorage) StoreBlob(blob *objects.Blob) error {
	return nil
}

func (fs *FakeStorage) HasObject(hash objects.Hash) bool {
	return false
}

func (fs *FakeStorage) ListSeals() ([]objects.Hash, error) {
	return []objects.Hash{}, nil
}

// FakeTimelineManager implements the P2P TimelineManager interface for testing
type FakeTimelineManager struct{}

func (ftm *FakeTimelineManager) Current() string {
	return "main"
}

func (ftm *FakeTimelineManager) GetHead(timeline string) (objects.Hash, error) {
	return objects.Hash{}, nil
}

func (ftm *FakeTimelineManager) UpdateHead(timeline string, hash objects.Hash) error {
	return nil
}

func (ftm *FakeTimelineManager) Create(name, description string) error {
	return nil
}

func (ftm *FakeTimelineManager) Switch(name string) error {
	return nil
}

func (ftm *FakeTimelineManager) ListTimelines() []string {
	return []string{"main"}
}

func (ftm *FakeTimelineManager) GetTimelineMetadata(name string) (*p2p.TimelineMetadata, error) {
	return &p2p.TimelineMetadata{
		Name:        name,
		Description: "Test timeline",
		Head:        objects.Hash{},
		LastUpdate:  time.Now(),
		Author:      objects.Identity{Name: "Test"},
	}, nil
}