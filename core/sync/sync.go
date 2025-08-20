package sync

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"ivaldi/core/fuse"
	"ivaldi/core/network"
	"ivaldi/core/objects"
)

// SyncManager handles synchronization with remote portals
type SyncManager struct {
	network   *network.NetworkManager
	fuse      *fuse.FuseManager
	timeline  TimelineManager
	storage   Storage
}

// Storage interface for loading/storing seals
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
	StoreSeal(seal *objects.Seal) error
	LoadTree(hash objects.Hash) (*objects.Tree, error)
	LoadBlob(hash objects.Hash) (*objects.Blob, error)
}

// TimelineManager interface for timeline operations
type TimelineManager interface {
	Current() string
	GetHead(timeline string) (objects.Hash, error)
	UpdateHead(timeline string, hash objects.Hash) error
	Create(name, description string) error
	Switch(name string) error
}

// SyncOptions configures synchronization behavior
type SyncOptions struct {
	PortalName     string
	RemoteTimeline string
	LocalTimeline  string
	Strategy       fuse.FuseStrategy
	Force          bool
	DryRun         bool
}

// SyncResult contains the outcome of a sync operation
type SyncResult struct {
	FetchedSeals   int
	MergeResult    *fuse.FuseResult
	ConflictCount  int
	Success        bool
	Message        string
}

// NewSyncManager creates a new sync manager
func NewSyncManager(storage Storage, timeline TimelineManager, fuseManager *fuse.FuseManager, root string) *SyncManager {
	return &SyncManager{
		network:  network.NewNetworkManager(root),
		fuse:     fuseManager,
		timeline: timeline,
		storage:  storage,
	}
}

// Sync performs a complete synchronization with a remote portal
func (sm *SyncManager) Sync(portalURL string, opts SyncOptions) (*SyncResult, error) {
	// Step 1: Save local changes if any
	localChanges, err := sm.saveLocalChanges()
	if err != nil {
		return nil, fmt.Errorf("failed to save local changes: %v", err)
	}
	
	// Step 2: Fetch remote changes
	fetchResult, err := sm.network.FetchFromPortal(portalURL, opts.RemoteTimeline)
	if err != nil {
		return nil, fmt.Errorf("failed to fetch from portal: %v", err)
	}

	// If no remote changes, we're up to date
	if len(fetchResult.Seals) == 0 {
		return &SyncResult{
			FetchedSeals:  0,
			ConflictCount: 0,
			Success:       true,
			Message:       "Already up to date",
		}, nil
	}

	// Step 3: Store fetched seals
	for _, seal := range fetchResult.Seals {
		if err := sm.storage.StoreSeal(seal); err != nil {
			return nil, fmt.Errorf("failed to store fetched seal: %v", err)
		}
	}

	// Step 4: Create temporary timeline for remote changes
	remoteTimelineName := fmt.Sprintf("temp-remote-%d", time.Now().Unix())
	if err := sm.timeline.Create(remoteTimelineName, "Temporary timeline for remote changes"); err != nil {
		return nil, fmt.Errorf("failed to create temporary timeline: %v", err)
	}

	// Step 5: Update temporary timeline with remote head
	if len(fetchResult.Refs) > 0 {
		remoteHead := fetchResult.Refs[0].Hash // Use first ref as head
		if err := sm.timeline.UpdateHead(remoteTimelineName, remoteHead); err != nil {
			return nil, fmt.Errorf("failed to update remote timeline head: %v", err)
		}
	}

	// Step 6: Determine target timeline
	targetTimeline := opts.LocalTimeline
	if targetTimeline == "" {
		targetTimeline = sm.timeline.Current()
	}

	// Step 7: Check for divergent branches
	localHead, err := sm.timeline.GetHead(targetTimeline)
	if err != nil {
		return nil, fmt.Errorf("failed to get local head: %v", err)
	}

	remoteHead := fetchResult.Refs[0].Hash
	if localHead == remoteHead {
		// Restore local changes if we had any
		if err := sm.restoreLocalChanges(localChanges); err != nil {
			return nil, fmt.Errorf("failed to restore local changes: %v", err)
		}
		
		return &SyncResult{
			FetchedSeals:  len(fetchResult.Seals),
			ConflictCount: 0,
			Success:       true,
			Message:       "Already synchronized",
		}, nil
	}

	// Step 8: Determine sync strategy
	strategy := opts.Strategy
	if strategy == 0 { // Default to automatic
		strategy = fuse.FuseStrategyAutomatic
	}

	// Step 9: Perform the fuse operation
	fuseOpts := fuse.FuseOptions{
		SourceTimeline: remoteTimelineName,
		TargetTimeline: targetTimeline,
		FuseMessage:    fmt.Sprintf("Sync with %s", opts.PortalName),
		Strategy:       strategy,
		DeleteSource:   true, // Clean up temporary timeline
		DryRun:         opts.DryRun,
	}

	fuseResult, err := sm.fuse.Fuse(fuseOpts)
	if err != nil {
		return nil, fmt.Errorf("failed to fuse remote changes: %v", err)
	}

	// Step 10: Update working directory to reflect the new state
	if !opts.DryRun && fuseResult.Success {
		if err := sm.UpdateWorkingDirectory(targetTimeline); err != nil {
			return nil, fmt.Errorf("failed to update working directory: %v", err)
		}
		
		// Step 11: Restore local changes on top of the updated working directory
		if err := sm.restoreLocalChanges(localChanges); err != nil {
			return nil, fmt.Errorf("failed to restore local changes: %v", err)
		}
	}

	return &SyncResult{
		FetchedSeals:  len(fetchResult.Seals),
		MergeResult:   fuseResult,
		ConflictCount: fuseResult.ConflictCount,
		Success:       true,
		Message:       "Sync completed successfully",
	}, nil
}

// Push uploads local changes to a remote portal
func (sm *SyncManager) Push(portalURL, timeline string) error {
	// Get seals to upload (this would be more sophisticated in practice)
	localHead, err := sm.timeline.GetHead(timeline)
	if err != nil {
		return fmt.Errorf("failed to get local head: %v", err)
	}

	// For now, just upload the head seal
	seal, err := sm.storage.LoadSeal(localHead)
	if err != nil {
		return fmt.Errorf("failed to load seal for upload: %v", err)
	}

	return sm.network.UploadToPortal(portalURL, timeline, []*objects.Seal{seal})
}

// DetectDivergence checks if local and remote timelines have diverged
func (sm *SyncManager) DetectDivergence(localHead, remoteHead objects.Hash) (bool, error) {
	// Simple divergence detection - in practice this would check ancestry
	return localHead != remoteHead, nil
}

// CreateSyncStrategy determines the best sync strategy based on the situation
func (sm *SyncManager) CreateSyncStrategy(localHead, remoteHead objects.Hash, force bool) (fuse.FuseStrategy, error) {
	// Check if this can be a fast-forward
	if localHead == (objects.Hash{}) {
		// No local changes, can fast-forward
		return fuse.FuseStrategyFastForward, nil
	}

	if force {
		// Force merge with manual resolution
		return fuse.FuseStrategyManual, nil
	}

	// Default to automatic merge
	return fuse.FuseStrategyAutomatic, nil
}

// LocalChanges stores information about local modifications
type LocalChanges struct {
	ModifiedFiles map[string][]byte // path -> content
	DeletedFiles  []string
	AddedFiles    map[string][]byte // path -> content
}

// saveLocalChanges preserves local uncommitted changes before sync
func (sm *SyncManager) saveLocalChanges() (*LocalChanges, error) {
	changes := &LocalChanges{
		ModifiedFiles: make(map[string][]byte),
		AddedFiles:    make(map[string][]byte),
		DeletedFiles:  []string{},
	}
	
	// Scan for local changes
	workDir := sm.network.GetRoot()
	
	// Walk through the working directory to find changes
	err := filepath.Walk(workDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		
		// Skip directories and special paths
		if info.IsDir() {
			// Skip .git and .ivaldi directories
			if info.Name() == ".git" || info.Name() == ".ivaldi" {
				return filepath.SkipDir
			}
			return nil
		}
		
		relPath, err := filepath.Rel(workDir, path)
		if err != nil {
			return err
		}
		
		// Read current file content
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		
		// For now, save all files as potentially modified
		// In a real implementation, we'd compare with the last committed state
		changes.ModifiedFiles[relPath] = content
		
		return nil
	})
	
	return changes, err
}

// restoreLocalChanges reapplies local changes after sync
func (sm *SyncManager) restoreLocalChanges(changes *LocalChanges) error {
	if changes == nil {
		return nil
	}
	
	workDir := sm.network.GetRoot()
	
	// Restore modified files
	for path, content := range changes.ModifiedFiles {
		fullPath := filepath.Join(workDir, path)
		
		// Check if file exists after sync
		currentContent, err := os.ReadFile(fullPath)
		if err != nil {
			// File doesn't exist after sync, treat as added
			changes.AddedFiles[path] = content
			continue
		}
		
		// Only restore if content is different
		if !bytes.Equal(currentContent, content) {
			// For now, just overwrite with local version
			// In a real implementation, we'd do a three-way merge
			if err := os.WriteFile(fullPath, content, 0644); err != nil {
				return fmt.Errorf("failed to restore %s: %v", path, err)
			}
			fmt.Printf("Restored local changes to %s\n", path)
		}
	}
	
	// Restore added files
	for path, content := range changes.AddedFiles {
		fullPath := filepath.Join(workDir, path)
		
		// Ensure directory exists
		dir := filepath.Dir(fullPath)
		if err := os.MkdirAll(dir, 0755); err != nil {
			return fmt.Errorf("failed to create directory for %s: %v", path, err)
		}
		
		if err := os.WriteFile(fullPath, content, 0644); err != nil {
			return fmt.Errorf("failed to restore added file %s: %v", path, err)
		}
		fmt.Printf("Restored added file %s\n", path)
	}
	
	// Handle deleted files (for now, we'll skip this as it's complex)
	// In a real implementation, we'd check if the file was added by sync
	// and only delete it if it wasn't
	
	return nil
}

// ValidateSync checks if a sync operation is safe to perform
func (sm *SyncManager) ValidateSync(timeline string) error {
	// Check for uncommitted changes would go here
	// For now, assume it's always safe
	return nil
}

// UpdateWorkingDirectory updates the working directory files to match the new timeline state
func (sm *SyncManager) UpdateWorkingDirectory(timeline string) error {
	// Get the current head of the timeline
	head, err := sm.timeline.GetHead(timeline)
	if err != nil {
		return fmt.Errorf("failed to get timeline head: %v", err)
	}

	// Load the seal at the head
	seal, err := sm.storage.LoadSeal(head)
	if err != nil {
		return fmt.Errorf("failed to load seal: %v", err)
	}

	// If seal has no position (tree reference), nothing to extract
	if seal.Position.IsZero() {
		fmt.Println("No tree associated with seal, skipping file extraction")
		return nil
	}

	// Load the tree from the seal's position
	tree, err := sm.storage.LoadTree(seal.Position)
	if err != nil {
		return fmt.Errorf("failed to load tree: %v", err)
	}

	// Extract files from tree to working directory
	return sm.extractTreeToWorkingDirectory(tree, sm.network.GetRoot())
}

// extractTreeToWorkingDirectory recursively extracts files from a tree to the working directory
func (sm *SyncManager) extractTreeToWorkingDirectory(tree *objects.Tree, targetPath string) error {
	for _, entry := range tree.Entries {
		fullPath := filepath.Join(targetPath, entry.Name)
		
		switch entry.Type {
		case objects.ObjectTypeTree:
			// Create directory if it doesn't exist
			if err := os.MkdirAll(fullPath, 0755); err != nil {
				return fmt.Errorf("failed to create directory %s: %v", fullPath, err)
			}
			
			// Recursively extract subtree
			subtree, err := sm.storage.LoadTree(entry.Hash)
			if err != nil {
				return fmt.Errorf("failed to load subtree %s: %v", entry.Name, err)
			}
			
			if err := sm.extractTreeToWorkingDirectory(subtree, fullPath); err != nil {
				return err
			}
			
		case objects.ObjectTypeBlob:
			// Load blob content
			blob, err := sm.storage.LoadBlob(entry.Hash)
			if err != nil {
				return fmt.Errorf("failed to load blob %s: %v", entry.Name, err)
			}
			
			// Ensure parent directory exists
			dir := filepath.Dir(fullPath)
			if err := os.MkdirAll(dir, 0755); err != nil {
				return fmt.Errorf("failed to create directory for %s: %v", fullPath, err)
			}
			
			// Write file with proper permissions
			fileMode := os.FileMode(entry.Mode)
			if fileMode == 0 {
				fileMode = 0644 // Default file permissions
			}
			
			if err := os.WriteFile(fullPath, blob.Data, fileMode); err != nil {
				return fmt.Errorf("failed to write file %s: %v", fullPath, err)
			}
		}
	}
	
	return nil
}