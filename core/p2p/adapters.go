package p2p

import (
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/timeline"
	"ivaldi/storage/local"
)

// StorageAdapter adapts local.Storage to the P2P Storage interface
type StorageAdapter struct {
	*local.Storage
}

// HasObject checks if an object exists in storage
func (sa *StorageAdapter) HasObject(hash objects.Hash) bool {
	return sa.Storage.Exists(hash)
}

// StoreTree stores a tree object
func (sa *StorageAdapter) StoreTree(tree *objects.Tree) error {
	// Use StoreObject which handles all object types
	_, err := sa.Storage.StoreObject(tree)
	return err
}

// StoreBlob stores a blob object
func (sa *StorageAdapter) StoreBlob(blob *objects.Blob) error {
	// Use StoreObject which handles all object types
	_, err := sa.Storage.StoreObject(blob)
	return err
}

// ListSeals returns all seal hashes in storage
func (sa *StorageAdapter) ListSeals() ([]objects.Hash, error) {
	// For now, return empty list as this would require additional implementation
	return []objects.Hash{}, nil
}

// TimelineAdapter adapts timeline.Manager to the P2P TimelineManager interface
type TimelineAdapter struct {
	*timeline.Manager
}

// ListTimelines returns all timeline names
func (ta *TimelineAdapter) ListTimelines() []string {
	timelines := ta.Manager.List()
	names := make([]string, len(timelines))
	for i, t := range timelines {
		names[i] = t.Name
	}
	return names
}

// GetTimelineMetadata returns metadata for a timeline
func (ta *TimelineAdapter) GetTimelineMetadata(name string) (*TimelineMetadata, error) {
	head, err := ta.Manager.GetHead(name)
	if err != nil {
		return nil, err
	}

	// Get timeline info
	timelines := ta.Manager.List()
	var description string
	for _, t := range timelines {
		if t.Name == name {
			description = t.Description
			break
		}
	}

	return &TimelineMetadata{
		Name:        name,
		Description: description,
		Head:        head,
		LastUpdate:  time.Now(), // Would need to track this properly
		Author:      objects.Identity{Name: "Unknown"}, // Would need to get from current config
	}, nil
}

// NewStorageAdapter creates a new storage adapter
func NewStorageAdapter(storage *local.Storage) Storage {
	return &StorageAdapter{Storage: storage}
}

// NewTimelineAdapter creates a new timeline adapter
func NewTimelineAdapter(manager *timeline.Manager) TimelineManager {
	return &TimelineAdapter{Manager: manager}
}