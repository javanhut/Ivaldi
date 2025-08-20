package forge

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/overwrite"
	"ivaldi/storage/local"
	"ivaldi/core/position"
	"ivaldi/core/preservation"
	"ivaldi/core/references"
	"ivaldi/core/timeline"
	"ivaldi/core/workspace"
	"ivaldi/storage/index"
)

// EnhancedRepository integrates all revolutionary features
type EnhancedRepository struct {
	*Repository // Embed existing repository
	
	// Revolutionary feature managers
	references      *references.ReferenceManager
	preservation    *preservation.PreservationManager
	overwriteTracker *overwrite.OverwriteTracker
}

// NewEnhancedRepository creates a repository with all revolutionary features
func NewEnhancedRepository(root string) (*EnhancedRepository, error) {
	// Create base repository
	baseRepo, err := Open(root)
	if err != nil {
		return nil, err
	}
	
	// Initialize revolutionary feature managers
	refManager := references.NewReferenceManager(root)
	
	// Connect reference manager to index for database queries
	refManager.SetIndex(baseRepo.index)
	
	if err := refManager.Load(); err != nil {
		return nil, fmt.Errorf("failed to load references: %v", err)
	}
	
	preservationManager := preservation.NewPreservationManager(root)
	if err := preservationManager.Load(); err != nil {
		return nil, fmt.Errorf("failed to load preservation: %v", err)
	}
	
	overwriteTracker := overwrite.NewOverwriteTracker(root)
	if err := overwriteTracker.Load(); err != nil {
		return nil, fmt.Errorf("failed to load overwrite tracker: %v", err)
	}
	
	return &EnhancedRepository{
		Repository:       baseRepo,
		references:      refManager,
		preservation:    preservationManager,
		overwriteTracker: overwriteTracker,
	}, nil
}

// EnhancedSeal creates a seal with memorable name and full tracking
func (er *EnhancedRepository) EnhancedSeal(message string) (*objects.Seal, error) {
	// Check if we have anything to seal
	if len(er.workspace.AnvilFiles) == 0 {
		return nil, fmt.Errorf("nothing gathered on the anvil to seal")
	}
	
	// Generate memorable name
	memorableName := er.references.GenerateMemorableName()
	
	// Get next iteration for current timeline
	iteration := er.references.GetNextIteration(er.timeline.Current())
	
	// Create enhanced seal with memorable name
	author := objects.Identity{
		Name:  "Developer", // TODO: Get from config
		Email: "dev@example.com",
	}
	
	seal := &objects.Seal{
		Name:      memorableName,
		Iteration: iteration,
		Message:   message,
		Author:    author,
		Timestamp: time.Now(),
		Parents:   []objects.Hash{er.position.Current().Hash},
	}
	
	// Store the seal
	if err := er.storage.StoreSeal(seal); err != nil {
		return nil, err
	}
	
	if err := er.index.IndexSeal(seal); err != nil {
		return nil, err
	}
	
	// Update position
	if err := er.position.SetPosition(seal.Hash, er.timeline.Current()); err != nil {
		return nil, err
	}
	
	// Register memorable name
	if err := er.references.RegisterMemorableName(memorableName, seal.Hash, author.Name); err != nil {
		return nil, err
	}
	
	// Update timeline head
	if err := er.timeline.UpdateHead(er.timeline.Current(), seal.Hash); err != nil {
		return nil, err
	}
	
	// Clear the anvil and reset file status
	er.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	
	// Reset all file statuses to unmodified after sealing
	for path, fileState := range er.workspace.Files {
		if fileState.OnAnvil {
			fileState.OnAnvil = false
			fileState.Status = workspace.StatusUnmodified
			er.workspace.Files[path] = fileState
		}
	}
	
	if err := er.workspace.SaveState(er.timeline.Current()); err != nil {
		return seal, err
	}
	
	return seal, nil
}

// EnhancedTimelineSwitch switches timelines with automatic work preservation
func (er *EnhancedRepository) EnhancedTimelineSwitch(timelineName string) (*preservation.WorkspaceSnapshot, error) {
	currentTimeline := er.timeline.Current()
	
	// Check if we're already on the target timeline
	if currentTimeline == timelineName {
		return nil, fmt.Errorf("already on timeline '%s'", timelineName)
	}
	
	// Check if we have any uncommitted changes to preserve
	var snapshot *preservation.WorkspaceSnapshot
	if er.workspace.HasUncommittedChanges() || len(er.workspace.AnvilFiles) > 0 {
		// Auto-preserve current workspace with a descriptive message
		var err error
		snapshot, err = er.preservation.AutoPreserve(
			er.workspace, 
			currentTimeline, 
			fmt.Sprintf("auto-shelf before switching to %s", timelineName),
		)
		if err != nil {
			return nil, fmt.Errorf("failed to auto-shelf workspace: %v", err)
		}
	}
	
	// Switch timeline using base functionality
	if err := er.SwitchTimeline(timelineName); err != nil {
		return snapshot, err
	}
	
	// Try to restore any previously shelved work for this timeline
	targetSnapshots := er.preservation.GetSnapshotsByTimeline(timelineName)
	if len(targetSnapshots) > 0 {
		// Find the most recent auto-shelf for this timeline
		var latestAutoShelf *preservation.WorkspaceSnapshot
		for _, snap := range targetSnapshots {
			if strings.Contains(snap.Description, "auto-shelf") {
				if latestAutoShelf == nil || snap.Timestamp.After(latestAutoShelf.Timestamp) {
					latestAutoShelf = snap
				}
			}
		}
		
		// Restore the latest auto-shelf if found
		if latestAutoShelf != nil {
			if err := er.preservation.RestoreWorkspace(latestAutoShelf.ID, er.workspace); err == nil {
				// Successfully restored - remove the snapshot since it's been used
				er.preservation.DeleteSnapshot(latestAutoShelf.ID)
			}
			// If restore fails, just continue - don't block the timeline switch
		}
	}
	
	return snapshot, nil
}

// EnhancedJump jumps to any position using natural language references
func (er *EnhancedRepository) EnhancedJump(reference string) error {
	// Resolve natural language reference
	hash, err := er.references.Resolve(reference, er.timeline.Current())
	if err != nil {
		return fmt.Errorf("could not resolve reference '%s': %v", reference, err)
	}
	
	// Use base jump functionality
	return er.Jump(hash.String())
}

// EnhancedReshape modifies history with mandatory overwrite tracking
func (er *EnhancedRepository) EnhancedReshape(count int, justification string, category overwrite.OverwriteCategory) error {
	if justification == "" {
		return fmt.Errorf("justification required for history modification")
	}
	
	// Get current and target positions
	currentHash := er.position.Current().Hash
	currentName, _ := er.references.GetMemorableName(currentHash)
	
	// TODO: Calculate what the new hash would be after reshape
	newHash := currentHash // Placeholder
	newName := er.references.GenerateMemorableName()
	
	// Record the overwrite
	_, err := er.overwriteTracker.RequestOverwrite(
		currentHash,
		currentName,
		newHash,
		newName,
		justification,
		category,
		"current-user", // TODO: Get from config
	)
	
	if err != nil {
		return fmt.Errorf("overwrite request failed: %v", err)
	}
	
	// TODO: Implement actual reshaping logic
	// This would involve:
	// 1. Interactive rebase-like functionality
	// 2. Updating references and memorable names
	// 3. Archiving original versions
	
	return nil
}

// CreateNamedWorkspace creates a named workspace snapshot
func (er *EnhancedRepository) CreateNamedWorkspace(name, description string) (*preservation.WorkspaceSnapshot, error) {
	return er.preservation.CreateNamedWorkspace(name, er.workspace, description)
}

// LoadNamedWorkspace restores a named workspace
func (er *EnhancedRepository) LoadNamedWorkspace(snapshotID string) error {
	return er.preservation.RestoreWorkspace(snapshotID, er.workspace)
}

// GetWorkspaceSnapshots returns available workspace snapshots
func (er *EnhancedRepository) GetWorkspaceSnapshots() []*preservation.WorkspaceSnapshot {
	return er.preservation.GetSnapshots()
}

// GetSnapshotsByTimeline returns snapshots for a specific timeline
func (er *EnhancedRepository) GetSnapshotsByTimeline(timeline string) []*preservation.WorkspaceSnapshot {
	return er.preservation.GetSnapshotsByTimeline(timeline)
}

// ProtectCommit marks a commit as protected from overwrites
func (er *EnhancedRepository) ProtectCommit(reference string) error {
	hash, err := er.references.Resolve(reference, er.timeline.Current())
	if err != nil {
		return err
	}
	
	return er.overwriteTracker.ProtectCommit(hash)
}

// UnprotectCommit removes protection from a commit
func (er *EnhancedRepository) UnprotectCommit(reference string) error {
	hash, err := er.references.Resolve(reference, er.timeline.Current())
	if err != nil {
		return err
	}
	
	return er.overwriteTracker.UnprotectCommit(hash)
}

// GetOverwriteHistory returns overwrite history for a commit
func (er *EnhancedRepository) GetOverwriteHistory(reference string) []*overwrite.OverwriteRecord {
	return er.overwriteTracker.GetOverwriteHistory(reference)
}

// GetOverwriteCount returns number of times a commit was overwritten
func (er *EnhancedRepository) GetOverwriteCount(reference string) int {
	return er.overwriteTracker.GetOverwriteCount(reference)
}

// ExportAuditTrail exports complete audit trail for compliance
func (er *EnhancedRepository) ExportAuditTrail() ([]byte, error) {
	return er.overwriteTracker.ExportAuditTrail()
}

// Enhanced initialization that sets up all revolutionary features
func EnhancedInitialize(root string) (*EnhancedRepository, error) {
	// Create directory structure
	if err := os.MkdirAll(root, 0755); err != nil {
		return nil, err
	}
	
	ivaldiDir := filepath.Join(root, ".ivaldi")
	if err := os.MkdirAll(ivaldiDir, 0755); err != nil {
		return nil, err
	}
	
	// Initialize storage
	storage, err := local.NewStorage(root)
	if err != nil {
		return nil, err
	}
	
	idx, err := index.NewSQLiteIndex(root)
	if err != nil {
		return nil, err
	}
	
	// Initialize core systems
	store, err := local.NewStore(root, objects.BLAKE3)
	if err != nil {
		return nil, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(root, store)
	tm := timeline.NewManager(root)
	pm := position.NewManager(root)
	
	// Create base repository
	baseRepo := &Repository{
		root:      root,
		storage:   storage,
		index:     idx,
		workspace: ws,
		timeline:  tm,
		position:  pm,
	}
	
	// Initialize timeline
	if err := tm.Initialize(); err != nil {
		return nil, err
	}
	
	// Initialize revolutionary features
	refManager := references.NewReferenceManager(root)
	preservationManager := preservation.NewPreservationManager(root)
	overwriteTracker := overwrite.NewOverwriteTracker(root)
	
	return &EnhancedRepository{
		Repository:       baseRepo,
		references:      refManager,
		preservation:    preservationManager,
		overwriteTracker: overwriteTracker,
	}, nil
}

// Enhanced mirror that imports with memorable names
func EnhancedMirror(url, dest string) (*EnhancedRepository, error) {
	// Use base mirror functionality
	_, err := Mirror(url, dest)
	if err != nil {
		return nil, err
	}
	
	// Upgrade to enhanced repository
	enhanced, err := NewEnhancedRepository(dest)
	if err != nil {
		return nil, err
	}
	
	// Import Git history with memorable names
	if err := enhanced.importGitHistoryWithNames(); err != nil {
		return nil, fmt.Errorf("failed to import Git history: %v", err)
	}
	
	return enhanced, nil
}

// Import Git history and assign memorable names to all commits
func (er *EnhancedRepository) importGitHistoryWithNames() error {
	// TODO: Implement Git history import with memorable name assignment
	// This would:
	// 1. Parse Git log to get all commits
	// 2. Assign memorable names to each commit
	// 3. Create reference mappings
	// 4. Build iteration numbers per timeline
	
	return nil
}

// Helper method to get memorable name for a hash
func (er *EnhancedRepository) GetMemorableName(hash objects.Hash) (string, bool) {
	return er.references.GetMemorableName(hash)
}

// Additional helper methods for CLI integration

// GetCurrentTimeline returns the current timeline name
func (er *EnhancedRepository) GetCurrentTimeline() string {
	return er.timeline.Current()
}

// GetCurrentPosition returns the current position hash
func (er *EnhancedRepository) GetCurrentPosition() objects.Hash {
	return er.position.Current().Hash
}

// ListTimelines returns a list of all timelines
func (er *EnhancedRepository) ListTimelines() []string {
	timelines := er.timeline.List()
	names := make([]string, len(timelines))
	for i, t := range timelines {
		names[i] = t.Name
	}
	return names
}

// CreateTimeline creates a new timeline
func (er *EnhancedRepository) CreateTimeline(name string) error {
	// Create the timeline first
	if err := er.timeline.Create(name, "Feature timeline"); err != nil {
		return err
	}
	
	// Save the current timeline state (if any) before creating new one
	currentTimeline := er.timeline.Current()
	if currentTimeline != name {
		if err := er.Repository.saveTimelineState(currentTimeline); err != nil {
			return fmt.Errorf("failed to save current timeline state: %v", err)
		}
	}
	
	// Copy the current timeline's state as the divergence point for the new timeline
	if err := er.Repository.saveTimelineState(name); err != nil {
		return fmt.Errorf("failed to create initial state for timeline: %v", err)
	}
	
	// Save current workspace state first
	if err := er.workspace.SaveState(currentTimeline); err != nil {
		return fmt.Errorf("failed to save current workspace state: %v", err)
	}
	
	// Copy workspace state from current timeline to new timeline
	if err := er.copyWorkspaceState(currentTimeline, name); err != nil {
		return fmt.Errorf("failed to copy workspace state: %v", err)
	}
	
	return nil
}

// copyWorkspaceState copies workspace state from source to target timeline
func (er *EnhancedRepository) copyWorkspaceState(sourceTimeline, targetTimeline string) error {
	// Load the source workspace state
	sourceWorkspace := workspace.New(er.root, er.workspace.Store)
	if err := sourceWorkspace.LoadState(sourceTimeline); err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	
	// Set the target timeline and save
	sourceWorkspace.Timeline = targetTimeline
	if err := sourceWorkspace.SaveState(targetTimeline); err != nil {
		return err
	}
	
	return nil
}

// DeleteTimeline deletes a timeline
func (er *EnhancedRepository) DeleteTimeline(name string) error {
	return er.timeline.Delete(name)
}

func (er *EnhancedRepository) RenameTimeline(oldName, newName string) error {
	return er.Repository.RenameTimeline(oldName, newName)
}

// GetStatus returns the current workspace status
func (er *EnhancedRepository) GetStatus() *WorkspaceStatus {
	// Extract modified and staged files from workspace
	var modified, staged []string
	
	for path, file := range er.workspace.Files {
		if file.Status == workspace.StatusModified {
			modified = append(modified, path)
		}
	}
	
	for path := range er.workspace.AnvilFiles {
		staged = append(staged, path)
	}
	
	return &WorkspaceStatus{
		Modified: modified,
		Staged:   staged,
	}
}

// GetHistory returns recent seals
func (er *EnhancedRepository) GetHistory(limit int) []*objects.Seal {
	// Basic history implementation - would load from storage
	return []*objects.Seal{} // Placeholder
}

// ListPortals returns configured portals
func (er *EnhancedRepository) ListPortals() map[string]string {
	return er.Repository.ListPortals()
}

// AddPortal adds a new portal
func (er *EnhancedRepository) AddPortal(name, url string) error {
	return er.Repository.AddPortal(name, url)
}

// Push pushes to a portal
func (er *EnhancedRepository) Push(portalName string) error {
	return er.Repository.Push(portalName)
}

// PushToBranch pushes to a specific branch
func (er *EnhancedRepository) PushToBranch(portalName, branch string, setUpstream bool) error {
	return er.Repository.PushToBranch(portalName, branch, setUpstream)
}

// PullFromBranch pulls from a specific branch
func (er *EnhancedRepository) PullFromBranch(portalName, branch string) error {
	return er.Repository.PullFromBranch(portalName, branch)
}

// CreateBranchAndMigrate creates a new branch and migrates content
func (er *EnhancedRepository) CreateBranchAndMigrate(newBranch, fromBranch string) error {
	return er.Repository.CreateBranchAndMigrate(newBranch, fromBranch)
}

// UploadToPortal uploads to portal with automatic upstream
func (er *EnhancedRepository) UploadToPortal(portalName, branch string) error {
	return er.Repository.UploadToPortal(portalName, branch)
}

// RenameBranchOnPortal renames a branch on the remote portal
func (er *EnhancedRepository) RenameBranchOnPortal(portalName, oldBranch, newBranch string) error {
	return er.Repository.RenameBranchOnPortal(portalName, oldBranch, newBranch)
}

// WorkspaceStatus represents current workspace state
type WorkspaceStatus struct {
	Modified []string
	Staged   []string
}

// GetWorkspace returns the workspace for analysis
func (er *EnhancedRepository) GetWorkspace() *workspace.Workspace {
	return er.workspace
}

// RefreshIgnorePatterns refreshes the ignore patterns and removes ignored files from anvil
func (er *EnhancedRepository) RefreshIgnorePatterns() error {
	er.workspace.RefreshIgnorePatterns()
	
	// Remove any files that are now ignored from the anvil
	var toRemove []string
	for path := range er.workspace.AnvilFiles {
		if er.workspace.ShouldIgnore(path) {
			toRemove = append(toRemove, path)
		}
	}
	
	for _, path := range toRemove {
		delete(er.workspace.AnvilFiles, path)
		if fileState, exists := er.workspace.Files[path]; exists {
			fileState.OnAnvil = false
			fileState.Status = workspace.StatusUnmodified
		}
	}
	
	return er.workspace.SaveState(er.timeline.Current())
}

// ExcludeFiles adds patterns to .ivaldiignore and removes them from tracking
func (er *EnhancedRepository) ExcludeFiles(patterns []string) error {
	// Read current .ivaldiignore file
	ignorePath := filepath.Join(er.Root(), ".ivaldiignore")
	
	var existingPatterns []string
	if data, err := os.ReadFile(ignorePath); err == nil {
		lines := strings.Split(string(data), "\n")
		for _, line := range lines {
			if strings.TrimSpace(line) != "" {
				existingPatterns = append(existingPatterns, line)
			}
		}
	}
	
	// Add new patterns
	existingPatterns = append(existingPatterns, "")
	existingPatterns = append(existingPatterns, "# Added by ivaldi exclude command")
	for _, pattern := range patterns {
		existingPatterns = append(existingPatterns, pattern)
	}
	
	// Write updated .ivaldiignore
	content := strings.Join(existingPatterns, "\n") + "\n"
	if err := os.WriteFile(ignorePath, []byte(content), 0644); err != nil {
		return fmt.Errorf("failed to update .ivaldiignore: %v", err)
	}
	
	// Refresh ignore patterns and clean workspace
	return er.RefreshIgnorePatterns()
}

// RemoveFiles removes files from repository and optionally excludes them
func (er *EnhancedRepository) RemoveFiles(patterns []string, fromRemoteOnly bool, excludeAfter bool) error {
	var filesToRemove []string
	
	// Expand patterns to actual files
	for _, pattern := range patterns {
		if strings.Contains(pattern, "*") {
			// Handle glob patterns
			matches, err := filepath.Glob(filepath.Join(er.Root(), pattern))
			if err != nil {
				return fmt.Errorf("invalid pattern %s: %v", pattern, err)
			}
			for _, match := range matches {
				relPath, err := filepath.Rel(er.Root(), match)
				if err == nil {
					filesToRemove = append(filesToRemove, relPath)
				}
			}
		} else {
			// Direct file/directory
			filesToRemove = append(filesToRemove, pattern)
		}
	}
	
	// Remove files from filesystem unless --from-remote only
	if !fromRemoteOnly {
		for _, file := range filesToRemove {
			fullPath := filepath.Join(er.Root(), file)
			if err := os.RemoveAll(fullPath); err != nil {
				return fmt.Errorf("failed to remove %s: %v", file, err)
			}
		}
	}
	
	// Stage removal in workspace
	for _, file := range filesToRemove {
		if fileState, exists := er.workspace.Files[file]; exists {
			fileState.Status = workspace.StatusDeleted
			fileState.OnAnvil = true
			er.workspace.AnvilFiles[file] = fileState
		}
	}
	
	// Add to ignore file if requested
	if excludeAfter {
		if err := er.ExcludeFiles(patterns); err != nil {
			return fmt.Errorf("failed to exclude files: %v", err)
		}
	}
	
	// Save workspace state
	return er.workspace.SaveState(er.timeline.Current())
}

// TimelineManager interface implementation for FuseManager
func (er *EnhancedRepository) Current() string {
	return er.timeline.Current()
}

func (er *EnhancedRepository) GetHead(timeline string) (objects.Hash, error) {
	head, err := er.timeline.GetHead(timeline)
	if err != nil {
		return objects.Hash{}, err
	}
	return head, nil
}

func (er *EnhancedRepository) UpdateHead(timeline string, hash objects.Hash) error {
	return er.timeline.UpdateHead(timeline, hash)
}

// WorkspaceManager interface implementation for FuseManager
func (er *EnhancedRepository) HasUncommittedChanges() bool {
	return er.workspace.HasUncommittedChanges()
}

func (er *EnhancedRepository) SaveState(timeline string) error {
	return er.workspace.SaveState(timeline)
}

func (er *EnhancedRepository) LoadState(timeline string) error {
	return er.workspace.LoadState(timeline)
}

// Storage interface implementation for FuseManager
func (er *EnhancedRepository) LoadSeal(hash objects.Hash) (*objects.Seal, error) {
	return er.storage.LoadSeal(hash)
}

func (er *EnhancedRepository) StoreSeal(seal *objects.Seal) error {
	return er.storage.StoreSeal(seal)
}

// GetStorage returns the storage for FuseManager
func (er *EnhancedRepository) GetStorage() *local.Storage {
	return er.storage
}