package p2p

import (
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"ivaldi/core/config"
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
	var sealHashes []objects.Hash

	// Get the objects directory path from the underlying storage
	objectsDir := sa.Storage.GetObjectsDir()

	// Walk through all object files in the storage directory
	err := filepath.Walk(objectsDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}

		// Skip directories
		if info.IsDir() {
			return nil
		}

		// Parse the file path to reconstruct the hash
		// Objects are stored as: objectsDir/xx/xxxxx...
		// where xx is the first two chars of the hex hash
		relativePath, err := filepath.Rel(objectsDir, path)
		if err != nil {
			return nil // Skip malformed paths
		}

		// Reconstruct hash string from directory structure
		// Objects are stored as: objectsDir/xx/yyyyyyyy...
		// where xx is first 2 chars of hex hash, yyyyyy is remaining chars
		dir, file := filepath.Split(relativePath)
		dir = strings.TrimSuffix(dir, "/") // Remove trailing slash

		if len(dir) == 2 && len(file) > 0 {
			// The filename is double-hex-encoded, so we need to decode twice
			doubleHexStr := dir + file

			// First decode: hex -> original hex string
			hexStrBytes, err := hex.DecodeString(doubleHexStr)
			if err != nil {
				return nil // Skip invalid hex
			}

			// The decoded bytes should form a hex string representing the actual hash
			actualHashStr := string(hexStrBytes)

			// Second decode: hex string -> hash bytes
			hashBytes, err := hex.DecodeString(actualHashStr)
			if err != nil || len(hashBytes) != 32 {
				return nil // Skip invalid hashes
			}

			var hash objects.Hash
			copy(hash[:], hashBytes)

			// Try to load as a seal to verify it's actually a seal object
			seal, err := sa.Storage.LoadSeal(hash)
			if err == nil && seal != nil {
				// Check if this is actually a seal by verifying it has seal-specific fields
				// A valid seal must have a name, iteration >= 0, and a proper timestamp
				if seal.Name != "" || seal.Iteration > 0 || !seal.Timestamp.IsZero() {
					sealHashes = append(sealHashes, hash)
				}
			}
			// If it fails to load as seal or doesn't look like a seal, skip silently
		}

		return nil
	})

	if err != nil {
		return nil, err
	}

	return sealHashes, nil
}

// TimelineAdapter adapts timeline.Manager to the P2P TimelineManager interface
type TimelineAdapter struct {
	*timeline.Manager
	rootDir string // Optional root directory for configuration access
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

	// Get timeline info directly from Manager.Get() to access UpdatedAt field
	timeline, exists := ta.Manager.Get(name)
	if !exists {
		return nil, fmt.Errorf("timeline '%s' not found", name)
	}

	// Get author from configuration if available
	var author objects.Identity
	if ta.rootDir != "" {
		// Try to load configuration to get user information
		configManager := config.NewConfigManager(ta.rootDir)
		if creds, err := configManager.LoadCredentials(); err == nil && creds != nil {
			if creds.UserName != "" || creds.UserEmail != "" {
				author = objects.Identity{
					Name:  creds.UserName,
					Email: creds.UserEmail,
				}
			} else {
				author = objects.Identity{Name: "Unknown", Email: ""}
			}
		} else {
			// Fallback: configuration not available or failed to load
			author = objects.Identity{Name: "Unknown", Email: ""}
		}
	} else {
		// No root directory available for config access
		author = objects.Identity{Name: "Unknown", Email: ""}
	}

	return &TimelineMetadata{
		Name:        timeline.Name,
		Description: timeline.Description,
		Head:        head,
		LastUpdate:  timeline.UpdatedAt, // Use actual timeline update time
		Author:      author,
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

// NewTimelineAdapterWithConfig creates a new timeline adapter with configuration access
func NewTimelineAdapterWithConfig(manager *timeline.Manager, rootDir string) TimelineManager {
	return &TimelineAdapter{Manager: manager, rootDir: rootDir}
}
