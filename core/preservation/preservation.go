package preservation

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/workspace"
)

// WorkspaceSnapshot represents a preserved workspace state
type WorkspaceSnapshot struct {
	ID          string                   `json:"id"`
	Name        string                   `json:"name"`
	Timeline    string                   `json:"timeline"`
	Position    objects.CAHash           `json:"position"`
	Timestamp   time.Time                `json:"timestamp"`
	Files       map[string]*FileSnapshot `json:"files"`
	AnvilFiles  map[string]*FileSnapshot `json:"anvilFiles"`
	Description string                   `json:"description"`
	AutoSaved   bool                     `json:"autoSaved"`
}

type FileSnapshot struct {
	Path    string    `json:"path"`
	Content []byte    `json:"content"`
	Status  int       `json:"status"`
	Hash    string    `json:"hash"`
	ModTime time.Time `json:"modTime"`
	Size    int64     `json:"size"`
}

// PreservationManager handles automatic work preservation
type PreservationManager struct {
	root      string
	snapshots map[string]*WorkspaceSnapshot
}

func NewPreservationManager(root string) *PreservationManager {
	return &PreservationManager{
		root:      root,
		snapshots: make(map[string]*WorkspaceSnapshot),
	}
}

// AutoPreserve automatically saves current workspace before timeline switch
func (pm *PreservationManager) AutoPreserve(ws *workspace.Workspace, fromTimeline string, reason string) (*WorkspaceSnapshot, error) {
	// Check if there's anything to preserve
	hasChanges := pm.hasUncommittedChanges(ws)
	hasAnvilFiles := len(ws.AnvilFiles) > 0

	if !hasChanges && !hasAnvilFiles {
		return nil, nil // Nothing to preserve
	}

	// Create snapshot
	snapshot := &WorkspaceSnapshot{
		ID:          pm.generateSnapshotID(),
		Name:        fmt.Sprintf("auto-%s-%d", fromTimeline, time.Now().Unix()),
		Timeline:    fromTimeline,
		Position:    ws.Position,
		Timestamp:   time.Now(),
		Files:       make(map[string]*FileSnapshot),
		AnvilFiles:  make(map[string]*FileSnapshot),
		Description: fmt.Sprintf("Auto-preserved before %s", reason),
		AutoSaved:   true,
	}

	// Snapshot all modified files
	for path, fileState := range ws.Files {
		if pm.shouldPreserveFile(fileState) {
			content, err := pm.readFileContent(filepath.Join(ws.Root, path))
			if err != nil {
				continue // Skip files we can't read
			}

			snapshot.Files[path] = &FileSnapshot{
				Path:    path,
				Content: content,
				Status:  int(fileState.Status),
				Hash:    fileState.Hash.String(),
				ModTime: fileState.ModTime,
				Size:    fileState.Size,
			}
		}
	}

	// Snapshot anvil files
	for path, fileState := range ws.AnvilFiles {
		content, err := pm.readFileContent(filepath.Join(ws.Root, path))
		if err != nil {
			continue
		}

		snapshot.AnvilFiles[path] = &FileSnapshot{
			Path:    path,
			Content: content,
			Status:  int(fileState.Status),
			Hash:    fileState.Hash.String(),
			ModTime: fileState.ModTime,
			Size:    fileState.Size,
		}
	}

	// Store snapshot
	pm.snapshots[snapshot.ID] = snapshot

	if err := pm.saveSnapshot(snapshot); err != nil {
		return nil, fmt.Errorf("failed to save snapshot: %v", err)
	}

	return snapshot, nil
}

// RestoreWorkspace restores a preserved workspace
func (pm *PreservationManager) RestoreWorkspace(snapshotID string, ws *workspace.Workspace) error {
	snapshot, exists := pm.snapshots[snapshotID]
	if !exists {
		return fmt.Errorf("snapshot %s not found", snapshotID)
	}

	// Restore files
	for path, fileSnapshot := range snapshot.Files {
		fullPath := filepath.Join(ws.Root, path)

		// Ensure directory exists
		if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
			return err
		}

		// Write file content
		if err := os.WriteFile(fullPath, fileSnapshot.Content, 0644); err != nil {
			return err
		}

		// Restore file state
		hash, err := objects.ParseCAHash(fileSnapshot.Hash)
		if err != nil {
			continue // Skip invalid hashes
		}

		ws.Files[path] = &workspace.FileState{
			Path:        path,
			Status:      workspace.FileStatus(fileSnapshot.Status),
			Hash:        hash,
			Size:        fileSnapshot.Size,
			ModTime:     fileSnapshot.ModTime,
			WorkingHash: hash,
			BlobHash:    objects.CAHash{}, // Will be computed when needed
		}
	}

	// Restore anvil files
	ws.AnvilFiles = make(map[string]*workspace.FileState)
	for path, fileSnapshot := range snapshot.AnvilFiles {
		hash, err := objects.ParseCAHash(fileSnapshot.Hash)
		if err != nil {
			continue // Skip invalid hashes
		}

		ws.AnvilFiles[path] = &workspace.FileState{
			Path:        path,
			Status:      workspace.FileStatus(fileSnapshot.Status),
			Hash:        hash,
			Size:        fileSnapshot.Size,
			ModTime:     fileSnapshot.ModTime,
			WorkingHash: hash,
			BlobHash:    objects.CAHash{}, // Will be computed when needed
			OnAnvil:     true,
		}
	}

	return nil
}

// GetSnapshots returns all available snapshots
func (pm *PreservationManager) GetSnapshots() []*WorkspaceSnapshot {
	var snapshots []*WorkspaceSnapshot
	for _, snapshot := range pm.snapshots {
		snapshots = append(snapshots, snapshot)
	}
	return snapshots
}

// GetSnapshotsByTimeline returns snapshots for a specific timeline
func (pm *PreservationManager) GetSnapshotsByTimeline(timeline string) []*WorkspaceSnapshot {
	var snapshots []*WorkspaceSnapshot
	for _, snapshot := range pm.snapshots {
		if snapshot.Timeline == timeline {
			snapshots = append(snapshots, snapshot)
		}
	}
	return snapshots
}

// DeleteSnapshot removes a snapshot
func (pm *PreservationManager) DeleteSnapshot(snapshotID string) error {
	if _, exists := pm.snapshots[snapshotID]; !exists {
		return fmt.Errorf("snapshot %s not found", snapshotID)
	}

	delete(pm.snapshots, snapshotID)

	// Remove from disk
	snapshotPath := filepath.Join(pm.root, ".ivaldi", "snapshots", snapshotID+".json")
	return os.Remove(snapshotPath)
}

// CreateNamedWorkspace creates a named workspace from current state
func (pm *PreservationManager) CreateNamedWorkspace(name string, ws *workspace.Workspace, description string) (*WorkspaceSnapshot, error) {
	snapshot := &WorkspaceSnapshot{
		ID:          pm.generateSnapshotID(),
		Name:        name,
		Timeline:    ws.Timeline,
		Position:    ws.Position,
		Timestamp:   time.Now(),
		Files:       make(map[string]*FileSnapshot),
		AnvilFiles:  make(map[string]*FileSnapshot),
		Description: description,
		AutoSaved:   false,
	}

	// Similar preservation logic as AutoPreserve
	for path, fileState := range ws.Files {
		if pm.shouldPreserveFile(fileState) {
			content, err := pm.readFileContent(filepath.Join(ws.Root, path))
			if err != nil {
				continue
			}

			snapshot.Files[path] = &FileSnapshot{
				Path:    path,
				Content: content,
				Status:  int(fileState.Status),
				Hash:    fileState.Hash.String(),
				ModTime: fileState.ModTime,
				Size:    fileState.Size,
			}
		}
	}

	for path, fileState := range ws.AnvilFiles {
		content, err := pm.readFileContent(filepath.Join(ws.Root, path))
		if err != nil {
			continue
		}

		snapshot.AnvilFiles[path] = &FileSnapshot{
			Path:    path,
			Content: content,
			Status:  int(fileState.Status),
			Hash:    fileState.Hash.String(),
			ModTime: fileState.ModTime,
			Size:    fileState.Size,
		}
	}

	pm.snapshots[snapshot.ID] = snapshot

	if err := pm.saveSnapshot(snapshot); err != nil {
		return nil, err
	}

	return snapshot, nil
}

// Helper functions

func (pm *PreservationManager) hasUncommittedChanges(ws *workspace.Workspace) bool {
	for _, file := range ws.Files {
		if file.Status == workspace.StatusModified || file.Status == workspace.StatusAdded {
			return true
		}
	}
	return false
}

func (pm *PreservationManager) shouldPreserveFile(fileState *workspace.FileState) bool {
	return fileState.Status == workspace.StatusModified ||
		fileState.Status == workspace.StatusAdded ||
		fileState.Status == workspace.StatusGathered
}

func (pm *PreservationManager) readFileContent(path string) ([]byte, error) {
	return os.ReadFile(path)
}

func (pm *PreservationManager) generateSnapshotID() string {
	return fmt.Sprintf("ws_%d", time.Now().UnixNano())
}

func (pm *PreservationManager) saveSnapshot(snapshot *WorkspaceSnapshot) error {
	snapshotDir := filepath.Join(pm.root, ".ivaldi", "snapshots")
	if err := os.MkdirAll(snapshotDir, 0755); err != nil {
		return err
	}

	snapshotPath := filepath.Join(snapshotDir, snapshot.ID+".json")

	data, err := json.MarshalIndent(snapshot, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(snapshotPath, data, 0644)
}

// Load all snapshots from disk
func (pm *PreservationManager) Load() error {
	snapshotDir := filepath.Join(pm.root, ".ivaldi", "snapshots")

	if _, err := os.Stat(snapshotDir); os.IsNotExist(err) {
		return nil // No snapshots directory yet
	}

	files, err := os.ReadDir(snapshotDir)
	if err != nil {
		return err
	}

	for _, file := range files {
		if !strings.HasSuffix(file.Name(), ".json") {
			continue
		}

		snapshotPath := filepath.Join(snapshotDir, file.Name())
		data, err := os.ReadFile(snapshotPath)
		if err != nil {
			continue // Skip corrupted files
		}

		var snapshot WorkspaceSnapshot
		if err := json.Unmarshal(data, &snapshot); err != nil {
			continue // Skip corrupted files
		}

		pm.snapshots[snapshot.ID] = &snapshot
	}

	return nil
}
