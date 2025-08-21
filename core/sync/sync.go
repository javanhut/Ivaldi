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
	"ivaldi/core/workspace"
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

// WorkspaceAdapter provides access to workspace functionality
type WorkspaceAdapter struct {
	*workspace.Workspace
}

// Scan delegates to the underlying workspace
func (wa *WorkspaceAdapter) Scan() error {
	return wa.Workspace.Scan()
}

// Files returns the workspace files
func (wa *WorkspaceAdapter) Files() map[string]*workspace.FileState {
	return wa.Workspace.Files
}

// AnvilFiles returns the anvil files
func (wa *WorkspaceAdapter) AnvilFiles() map[string]*workspace.FileState {
	return wa.Workspace.AnvilFiles
}

// saveLocalChanges preserves local uncommitted changes before sync
func (sm *SyncManager) saveLocalChanges() (*LocalChanges, error) {
	// Use optimized approach - only scan for common file types in key directories
	// This avoids reading large binary files or deep directory trees
	return sm.saveSelectedFiles()
}

// saveSelectedFiles optimized implementation that saves only commonly changed files
func (sm *SyncManager) saveSelectedFiles() (*LocalChanges, error) {
	changes := &LocalChanges{
		ModifiedFiles: make(map[string][]byte),
		AddedFiles:    make(map[string][]byte),
		DeletedFiles:  []string{},
	}
	
	workDir := sm.network.GetRoot()
	
	// Fast scan - only check specific file patterns and common directories
	searchPaths := []string{
		"*.go", "*.js", "*.ts", "*.py", "*.java", "*.c", "*.cpp", "*.h",
		"*.txt", "*.md", "*.json", "*.yaml", "*.yml", "*.toml", "*.ini",
		"src/*.go", "lib/*.go", "cmd/*.go", "pkg/*.go", "internal/*.go",
		"*.sh", "*.bat", "Dockerfile", "Makefile", "*.sql",
	}
	
	for _, pattern := range searchPaths {
		matches, err := filepath.Glob(filepath.Join(workDir, pattern))
		if err != nil {
			continue // Skip invalid patterns
		}
		
		for _, match := range matches {
			// Check if it's a file and not too large
			info, err := os.Stat(match)
			if err != nil || info.IsDir() || info.Size() > 1024*1024 { // Skip files > 1MB
				continue
			}
			
			relPath, err := filepath.Rel(workDir, match)
			if err != nil {
				continue
			}
			
			// Only save text files to avoid binaries
			if sm.isTextFile(filepath.Ext(match)) {
				content, err := os.ReadFile(match)
				if err != nil {
					continue
				}
				changes.ModifiedFiles[relPath] = content
			}
		}
	}
	
	return changes, nil
}

// saveAllFiles is a fallback implementation that saves all files
func (sm *SyncManager) saveAllFiles() (*LocalChanges, error) {
	changes := &LocalChanges{
		ModifiedFiles: make(map[string][]byte),
		AddedFiles:    make(map[string][]byte),
		DeletedFiles:  []string{},
	}
	
	workDir := sm.network.GetRoot()
	
	// Quick scan of only top-level and common directories to avoid deep recursion
	commonDirs := []string{".", "src", "lib", "cmd", "pkg", "internal"}
	
	for _, dir := range commonDirs {
		dirPath := filepath.Join(workDir, dir)
		if _, err := os.Stat(dirPath); os.IsNotExist(err) {
			continue
		}
		
		err := filepath.Walk(dirPath, func(path string, info os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			
			// Skip deep nesting and special directories
			if info.IsDir() {
				if info.Name() == ".git" || info.Name() == ".ivaldi" || 
				   info.Name() == "node_modules" || info.Name() == "target" {
					return filepath.SkipDir
				}
				return nil
			}
			
			relPath, err := filepath.Rel(workDir, path)
			if err != nil {
				return err
			}
			
			// Only save files with common extensions to avoid binaries
			ext := filepath.Ext(path)
			if sm.isTextFile(ext) {
				content, err := os.ReadFile(path)
				if err != nil {
					return err
				}
				changes.ModifiedFiles[relPath] = content
			}
			
			return nil
		})
		
		if err != nil {
			return nil, err
		}
	}
	
	return changes, nil
}

// isTextFile checks if a file extension indicates a text file
func (sm *SyncManager) isTextFile(ext string) bool {
	textExts := map[string]bool{
		".go": true, ".js": true, ".ts": true, ".py": true, ".rb": true,
		".java": true, ".c": true, ".cpp": true, ".h": true, ".hpp": true,
		".txt": true, ".md": true, ".json": true, ".yaml": true, ".yml": true,
		".xml": true, ".html": true, ".css": true, ".scss": true, ".less": true,
		".sh": true, ".bat": true, ".ps1": true, ".sql": true, ".csv": true,
		".gitignore": true, ".dockerignore": true, "Dockerfile": true,
		".toml": true, ".ini": true, ".cfg": true, ".conf": true,
	}
	return textExts[ext] || ext == ""
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