package forge

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/fuse"
	"ivaldi/core/network"
	"ivaldi/core/objects"
	"ivaldi/core/position"
	"ivaldi/core/references"
	"ivaldi/core/sync"
	"ivaldi/core/timeline"
	"ivaldi/core/workspace"
	"ivaldi/storage/index"
	"ivaldi/storage/local"
)

type Repository struct {
	root      string
	storage   *local.Storage
	index     *index.SQLiteIndex
	workspace *workspace.Workspace
	timeline  *timeline.Manager
	position  *position.Manager
	refMgr    *references.ReferenceManager
	syncMgr   *sync.SyncManager
	fuseMgr   *fuse.FuseManager
	network   *network.NetworkManager
}

type Status struct {
	Timeline  string
	Position  string
	Gathered  []string
	Modified  []string
	Untracked []string
}

func Initialize(root string) (*Repository, error) {
	if err := os.MkdirAll(root, 0755); err != nil {
		return nil, err
	}

	if err := os.MkdirAll(filepath.Join(root, ".ivaldi"), 0755); err != nil {
		return nil, err
	}

	storage, err := local.NewStorage(root)
	if err != nil {
		return nil, err
	}

	idx, err := index.NewSQLiteIndex(root)
	if err != nil {
		return nil, err
	}

	store, err := local.NewStore(root, objects.BLAKE3)
	if err != nil {
		return nil, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(root, store)
	tm := timeline.NewManager(root)
	pm := position.NewManager(root)
	rm := references.NewReferenceManager(root)

	// Configure reference manager with index
	rm.SetIndex(idx)
	
	// Configure position manager with reference resolver
	pm.SetReferenceResolver(rm)

	repo := &Repository{
		root:      root,
		storage:   storage,
		index:     idx,
		workspace: ws,
		timeline:  tm,
		position:  pm,
		refMgr:    rm,
	}

	if err := tm.Initialize(); err != nil {
		return nil, err
	}

	return repo, nil
}

func Mirror(url, dest string) (*Repository, error) {
	// Use git-independent download via network manager
	networkMgr := network.NewNetworkManager(dest)
	
	// Download repository contents using API
	if err := networkMgr.DownloadIvaldiRepo(url, dest); err != nil {
		return nil, fmt.Errorf("failed to download repository: %v", err)
	}

	// Initialize Ivaldi repository
	repo, err := Initialize(dest)
	if err != nil {
		return nil, fmt.Errorf("failed to initialize Ivaldi: %v", err)
	}

	// Add origin portal
	if err := repo.AddPortal("origin", url); err != nil {
		return nil, fmt.Errorf("failed to add origin portal: %v", err)
	}

	// Scan workspace to register downloaded files
	if err := repo.workspace.Scan(); err != nil {
		return nil, fmt.Errorf("failed to scan downloaded files: %v", err)
	}

	// Save initial workspace state
	if err := repo.workspace.SaveState(repo.timeline.Current()); err != nil {
		return nil, fmt.Errorf("failed to save workspace state: %v", err)
	}

	return repo, nil
}

func Open(root string) (*Repository, error) {
	storage, err := local.NewStorage(root)
	if err != nil {
		return nil, err
	}

	idx, err := index.NewSQLiteIndex(root)
	if err != nil {
		return nil, err
	}

	store, err := local.NewStore(root, objects.BLAKE3)
	if err != nil {
		return nil, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(root, store)
	tm := timeline.NewManager(root)
	pm := position.NewManager(root)
	rm := references.NewReferenceManager(root)

	// Configure reference manager with index
	rm.SetIndex(idx)
	
	// Configure position manager with reference resolver
	pm.SetReferenceResolver(rm)

	if err := tm.Load(); err != nil {
		return nil, err
	}

	if err := pm.Load(); err != nil {
		return nil, err
	}
	
	if err := rm.Load(); err != nil {
		return nil, err
	}
	
	// Load workspace state for current timeline
	currentTimeline := tm.Current()
	if err := ws.LoadState(currentTimeline); err != nil {
		// Ignore error if state doesn't exist yet
	}
	
	// Ensure workspace has correct root path after loading state
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	ws.Root = absRoot

	// Create fuse manager
	fuseMgr := fuse.NewFuseManager(storage, tm, ws)
	
	// Create sync manager  
	syncMgr := sync.NewSyncManager(storage, tm, fuseMgr, root)
	
	// Create network manager
	networkMgr := network.NewNetworkManager(root)

	repo := &Repository{
		root:      root,
		storage:   storage,
		index:     idx,
		workspace: ws,
		timeline:  tm,
		position:  pm,
		refMgr:    rm,
		syncMgr:   syncMgr,
		fuseMgr:   fuseMgr,
		network:   networkMgr,
	}

	return repo, nil
}

func (r *Repository) Root() string {
	return r.root
}

func (r *Repository) GetWorkspace() *workspace.Workspace {
	return r.workspace
}

func (r *Repository) Gather(patterns []string) error {
	if err := r.workspace.Scan(); err != nil {
		return err
	}

	if err := r.workspace.Gather(patterns); err != nil {
		return err
	}
	
	// Save workspace state after gathering
	return r.workspace.SaveState(r.timeline.Current())
}

func (r *Repository) Discard(patterns []string) (int, error) {
	count, err := r.workspace.Discard(patterns)
	if err != nil {
		return 0, err
	}
	
	// Save workspace state after discarding
	if err := r.workspace.SaveState(r.timeline.Current()); err != nil {
		return count, err
	}
	
	return count, nil
}

func (r *Repository) DiscardAll() int {
	count := len(r.workspace.AnvilFiles)
	r.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	
	// Save workspace state after discarding all
	r.workspace.SaveState(r.timeline.Current())
	
	return count
}

func (r *Repository) Seal(message string) (*objects.Seal, error) {
	if len(r.workspace.AnvilFiles) == 0 {
		return nil, fmt.Errorf("nothing gathered on the anvil to seal")
	}

	// Step 1: Build tree from staged items
	candidateTree := r.workspace.GetCandidateTree()
	if candidateTree == nil {
		// Build tree if not already built
		if err := r.workspace.BuildCandidateTree(); err != nil {
			return nil, fmt.Errorf("failed to build candidate tree: %v", err)
		}
		candidateTree = r.workspace.GetCandidateTree()
		if candidateTree == nil {
			return nil, fmt.Errorf("failed to create tree from staged files")
		}
	}

	// Step 2: Store the tree in content-addressed storage
	treeData, err := candidateTree.Encode()
	if err != nil {
		return nil, fmt.Errorf("failed to encode tree: %v", err)
	}

	treeHash, err := r.workspace.Store.Put(treeData, local.KindTree)
	if err != nil {
		return nil, fmt.Errorf("failed to store tree: %v", err)
	}

	// Step 2.5: Also store tree and blobs in legacy format for RestoreWorkingDirectory
	legacyTree := r.convertCATreeToLegacyTree(candidateTree, treeHash)
	legacyTreeHash, err := r.storage.StoreObject(legacyTree)
	if err != nil {
		return nil, fmt.Errorf("failed to store legacy tree: %v", err)
	}
	fmt.Printf("Debug: stored legacy tree with hash: %s\n", legacyTreeHash.String())

	// Step 2.6: Store blobs in legacy format
	if err := r.storeCABlobsAsLegacy(candidateTree); err != nil {
		return nil, fmt.Errorf("failed to store legacy blobs: %v", err)
	}

	// Step 3: Determine parents (current head or empty for first seal)
	var parents []objects.CAHash
	currentHead, err := r.timeline.GetHead(r.timeline.Current())
	
	// Check if currentHead is zero (empty hash)
	var zeroHash objects.Hash
	isFirstSeal := err != nil || currentHead == zeroHash
	
	if !isFirstSeal {
		// Get the previous seal's content-addressed hash
		previousSealHash, err := r.getPreviousSealCAHash(currentHead)
		if err == nil {
			parents = []objects.CAHash{previousSealHash}
		} else {
			// If we can't find the previous CA seal, start a fresh chain
			parents = []objects.CAHash{}
		}
	} else {
		// First seal - no parents
		parents = []objects.CAHash{}
	}

	// Step 4: Get author information (from config or defaults)
	author, committer := r.getAuthorInfo()

	// Step 5: Create content-addressed seal
	caSeal := objects.NewCASeal(treeHash, parents, author, committer, message)

	// Step 6: Store the seal
	sealData, err := caSeal.Encode()
	if err != nil {
		return nil, fmt.Errorf("failed to encode seal: %v", err)
	}

	sealHash, err := r.workspace.Store.Put(sealData, local.KindSeal)
	if err != nil {
		return nil, fmt.Errorf("failed to store seal: %v", err)
	}

	// Step 7: Create legacy seal for compatibility with existing systems
	name := r.generateMemorableName()
	iteration := r.getNextIteration()
	
	// Use the actual hash returned by storing the legacy tree
	fmt.Printf("Debug: creating seal with Position: %s\n", legacyTreeHash.String())
	legacySeal := &objects.Seal{
		Name:      name,
		Iteration: iteration,
		Position:  legacyTreeHash,  // Points to the actual stored legacy tree
		Message:   message,
		Author:    author,
		Timestamp: caSeal.Timestamp,
		Parents:   []objects.Hash{}, // Start fresh
	}

	// Store legacy seal for compatibility
	if err := r.storage.StoreSeal(legacySeal); err != nil {
		return nil, fmt.Errorf("failed to store legacy seal: %v", err)
	}

	if err := r.index.IndexSeal(legacySeal); err != nil {
		return nil, fmt.Errorf("failed to index seal: %v", err)
	}

	// Step 8: Update position and timeline head atomically
	if err := r.position.SetPosition(legacySeal.Hash, r.timeline.Current()); err != nil {
		return nil, fmt.Errorf("failed to set position: %v", err)
	}

	if err := r.position.SetMemorableName(legacySeal.Hash, name); err != nil {
		return nil, fmt.Errorf("failed to set memorable name: %v", err)
	}
	
	// Register the memorable name with the reference manager
	if err := r.refMgr.RegisterMemorableName(name, legacySeal.Hash, legacySeal.Author.Name); err != nil {
		return nil, fmt.Errorf("failed to register memorable name: %v", err)
	}

	// Update timeline head atomically
	if err := r.timeline.UpdateHead(r.timeline.Current(), legacySeal.Hash); err != nil {
		return nil, fmt.Errorf("failed to update timeline head: %v", err)
	}

	// Update workspace Position to point to content-addressed seal for consistency
	r.workspace.Position = sealHash

	// Store the mapping between legacy and content-addressed seals for chaining
	if err := r.storeSealMapping(legacySeal.Hash, sealHash); err != nil {
		// Log but don't fail - this is for optimization, not critical
		fmt.Printf("Warning: failed to store seal mapping: %v\n", err)
	}

	// Step 9: Update workspace state - clear anvil and mark files as committed
	r.updateWorkspaceAfterSeal()

	// Save the updated workspace state
	if err := r.workspace.SaveState(r.timeline.Current()); err != nil {
		return legacySeal, fmt.Errorf("failed to save workspace state: %v", err)
	}

	return legacySeal, nil
}

// getAuthorInfo retrieves author and committer information from config or defaults
func (r *Repository) getAuthorInfo() (objects.Identity, objects.Identity) {
	// Try to get from config first
	// For now, use defaults - in a real implementation, read from .ivaldi/config
	author := objects.Identity{
		Name:  "Developer",
		Email: "dev@example.com",
	}
	
	committer := author // Same as author for now
	
	return author, committer
}

// updateWorkspaceAfterSeal clears the anvil and updates file statuses after sealing
func (r *Repository) updateWorkspaceAfterSeal() {
	// After sealing, all sealed files should be marked as unchanged
	for path, anvilFile := range r.workspace.AnvilFiles {
		if fileState, exists := r.workspace.Files[path]; exists {
			// Update the file status to reflect that it's been sealed
			fileState.Status = workspace.StatusUnmodified
			fileState.OnAnvil = false
			// Update the base hash to match the working hash since it's now committed
			fileState.Hash = fileState.WorkingHash
		} else if anvilFile.Status == workspace.StatusAdded {
			// For newly added files, add them to the workspace as unchanged
			r.workspace.Files[path] = &workspace.FileState{
				Path:        path,
				Status:      workspace.StatusUnmodified,
				Hash:        anvilFile.WorkingHash,
				WorkingHash: anvilFile.WorkingHash,
				Size:        anvilFile.Size,
				ModTime:     anvilFile.ModTime,
				OnAnvil:     false,
				BlobHash:    anvilFile.BlobHash,
			}
		}
	}
	
	// Clear the anvil
	r.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	r.workspace.CandidateTree = nil // Clear the candidate tree
}

// getPreviousSealCAHash attempts to get the content-addressed hash of the previous seal
func (r *Repository) getPreviousSealCAHash(legacyHash objects.Hash) (objects.CAHash, error) {
	// Try to load the relationship from a mapping file
	// For now, we'll implement a simple approach where we store the mapping
	mappingPath := filepath.Join(r.root, ".ivaldi", "seal_mapping.json")
	
	type SealMapping struct {
		LegacyToCA map[string]string `json:"legacy_to_ca"`
		CAToLegacy map[string]string `json:"ca_to_legacy"`
	}
	
	var mapping SealMapping
	if data, err := os.ReadFile(mappingPath); err == nil {
		json.Unmarshal(data, &mapping)
	} else {
		mapping = SealMapping{
			LegacyToCA: make(map[string]string),
			CAToLegacy: make(map[string]string),
		}
	}
	
	legacyHashStr := hex.EncodeToString(legacyHash[:])
	if caHashStr, exists := mapping.LegacyToCA[legacyHashStr]; exists {
		return objects.ParseCAHash(caHashStr)
	}
	
	return objects.CAHash{}, fmt.Errorf("no content-addressed hash found for legacy hash %s", legacyHashStr)
}

// storeSealMapping stores the relationship between legacy and content-addressed seals
func (r *Repository) storeSealMapping(legacyHash objects.Hash, caHash objects.CAHash) error {
	mappingPath := filepath.Join(r.root, ".ivaldi", "seal_mapping.json")
	
	type SealMapping struct {
		LegacyToCA map[string]string `json:"legacy_to_ca"`
		CAToLegacy map[string]string `json:"ca_to_legacy"`
	}
	
	var mapping SealMapping
	if data, err := os.ReadFile(mappingPath); err == nil {
		json.Unmarshal(data, &mapping)
	} else {
		mapping = SealMapping{
			LegacyToCA: make(map[string]string),
			CAToLegacy: make(map[string]string),
		}
	}
	
	legacyHashStr := hex.EncodeToString(legacyHash[:])
	caHashStr := caHash.FullString()
	
	mapping.LegacyToCA[legacyHashStr] = caHashStr
	mapping.CAToLegacy[caHashStr] = legacyHashStr
	
	data, err := json.MarshalIndent(mapping, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(mappingPath, data, 0644)
}

func (r *Repository) CreateTimeline(name, description string) error {
	fmt.Printf("Creating timeline: %s\n", name)
	
	// Create the timeline
	if err := r.timeline.Create(name, description); err != nil {
		return err
	}
	
	// Save the current timeline state (if any) before creating new one
	currentTimeline := r.timeline.Current()
	if currentTimeline != name {
		fmt.Printf("Saving current state for timeline: %s\n", currentTimeline)
		if err := r.saveTimelineState(currentTimeline); err != nil {
			return fmt.Errorf("failed to save current timeline state: %v", err)
		}
	}
	
	// Copy the current timeline's state as the divergence point for the new timeline
	fmt.Printf("Creating initial state for timeline: %s from current working directory\n", name)
	if err := r.saveTimelineState(name); err != nil {
		return fmt.Errorf("failed to create initial state for timeline: %v", err)
	}
	
	// Copy current workspace state to the new timeline
	fmt.Printf("Saving current workspace state for timeline: %s\n", currentTimeline)
	if err := r.workspace.SaveState(currentTimeline); err != nil {
		return fmt.Errorf("failed to save current workspace state: %v", err)
	}
	
	// Copy workspace state from current timeline to new timeline
	fmt.Printf("Copying workspace state from %s to %s\n", currentTimeline, name)
	if err := r.copyWorkspaceState(currentTimeline, name); err != nil {
		return fmt.Errorf("failed to copy workspace state to new timeline: %v", err)
	}
	
	fmt.Printf("Timeline %s created successfully\n", name)
	return nil
}

func (r *Repository) SwitchTimeline(name string) error {
	currentTimeline := r.timeline.Current()
	
	// If switching to the same timeline, do nothing
	if currentTimeline == name {
		return nil
	}
	
	// Calculate the diff between current state and target timeline
	diff, err := r.calculateTimelineDiff(currentTimeline, name)
	if err != nil {
		return fmt.Errorf("failed to calculate timeline diff: %v", err)
	}
	
	// Save current timeline state before switching
	if err := r.saveTimelineState(currentTimeline); err != nil {
		return fmt.Errorf("failed to save timeline state for %s: %v", currentTimeline, err)
	}
	
	// Save current workspace state before switching
	if err := r.workspace.SaveState(currentTimeline); err != nil {
		return fmt.Errorf("failed to save workspace state: %v", err)
	}

	// Get target timeline's HEAD commit before switching
	targetHead, err := r.timeline.GetHead(name)
	if err != nil {
		return fmt.Errorf("failed to get timeline head: %v", err)
	}
	
	// Debug: check what hash we're getting
	fmt.Printf("Debug: switching to timeline %s with HEAD: %s\n", name, targetHead.String())

	// Switch timeline
	if err := r.timeline.Switch(name); err != nil {
		return err
	}

	// Apply the diff to transform working directory
	if err := r.applyTimelineDiff(diff); err != nil {
		return fmt.Errorf("failed to apply timeline diff: %v", err)
	}

	// Clear workspace state to prepare for new timeline
	r.workspace.Files = make(map[string]*workspace.FileState)
	r.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	r.workspace.Timeline = name

	// Load target timeline's workspace state
	if err := r.workspace.LoadState(name); err != nil {
		// It's okay if state doesn't exist yet for new timelines
		if !os.IsNotExist(err) {
			return fmt.Errorf("failed to load workspace state: %v", err)
		}
	}

	// Force rescan to update file tracking after restoration
	if err := r.workspace.Scan(); err != nil {
		return fmt.Errorf("failed to scan workspace after switch: %v", err)
	}

	return nil
}

// FileOperation represents a file change operation
type FileOperation struct {
	Type     string // "add", "modify", "delete", "unchanged"
	Path     string
	Content  []byte
	Mode     os.FileMode
	Hash     string // For content deduplication
}

// TimelineDiff represents the differences between two timeline states
type TimelineDiff struct {
	Operations []FileOperation
}

// calculateTimelineDiff calculates what changes need to be made to transform from current timeline to target
func (r *Repository) calculateTimelineDiff(currentTimeline, targetTimeline string) (*TimelineDiff, error) {
	// Get current working directory state
	currentFiles := make(map[string]FileOperation)
	err := filepath.Walk(r.root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		
		// Skip .ivaldi directory
		if strings.Contains(path, ".ivaldi") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		
		// Skip directories
		if info.IsDir() {
			return nil
		}
		
		relPath, _ := filepath.Rel(r.root, path)
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		
		currentFiles[relPath] = FileOperation{
			Type:    "current",
			Path:    relPath,
			Content: content,
			Mode:    info.Mode(),
			Hash:    r.hashContent(content),
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	
	// Get target timeline state
	targetFiles, err := r.loadTimelineState(targetTimeline)
	if err != nil {
		return nil, err
	}
	
	var operations []FileOperation
	
	// Find files to add or modify
	for path, targetFile := range targetFiles {
		if currentFile, exists := currentFiles[path]; exists {
			// File exists in both - check if content differs
			if currentFile.Hash != targetFile.Hash {
				operations = append(operations, FileOperation{
					Type:    "modify",
					Path:    path,
					Content: targetFile.Content,
					Mode:    targetFile.Mode,
					Hash:    targetFile.Hash,
				})
			}
			// Mark as processed
			delete(currentFiles, path)
		} else {
			// File only exists in target - add it
			operations = append(operations, FileOperation{
				Type:    "add",
				Path:    path,
				Content: targetFile.Content,
				Mode:    targetFile.Mode,
				Hash:    targetFile.Hash,
			})
		}
	}
	
	// Remaining files in currentFiles need to be deleted
	for path := range currentFiles {
		operations = append(operations, FileOperation{
			Type: "delete",
			Path: path,
		})
	}
	
	return &TimelineDiff{Operations: operations}, nil
}

// saveTimelineState saves the current working directory state using content-addressed storage
func (r *Repository) saveTimelineState(timelineName string) error {
	stateFile := filepath.Join(r.root, ".ivaldi", "timeline_states", timelineName+".json")
	
	// Create state directory
	if err := os.MkdirAll(filepath.Dir(stateFile), 0755); err != nil {
		return err
	}
	
	state := make(map[string]FileOperation)
	
	// Walk through working directory and record file states
	err := filepath.Walk(r.root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		
		// Skip .ivaldi directory
		if strings.Contains(path, ".ivaldi") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		
		// Skip directories
		if info.IsDir() {
			return nil
		}
		
		relPath, _ := filepath.Rel(r.root, path)
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		
		hash := r.hashContent(content)
		
		// Store content in content-addressed storage
		contentPath := filepath.Join(r.root, ".ivaldi", "content", hash)
		if _, err := os.Stat(contentPath); os.IsNotExist(err) {
			if err := os.MkdirAll(filepath.Dir(contentPath), 0755); err != nil {
				return err
			}
			if err := os.WriteFile(contentPath, content, 0644); err != nil {
				return err
			}
		}
		
		state[relPath] = FileOperation{
			Type: "stored",
			Path: relPath,
			Mode: info.Mode(),
			Hash: hash,
		}
		return nil
	})
	if err != nil {
		return err
	}
	
	// Save state metadata
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(stateFile, data, 0644)
}

// loadTimelineState loads a timeline state from storage
func (r *Repository) loadTimelineState(timelineName string) (map[string]FileOperation, error) {
	stateFile := filepath.Join(r.root, ".ivaldi", "timeline_states", timelineName+".json")
	
	// Check if state exists
	if _, err := os.Stat(stateFile); os.IsNotExist(err) {
		// No state yet - return empty state
		return make(map[string]FileOperation), nil
	}
	
	data, err := os.ReadFile(stateFile)
	if err != nil {
		return nil, err
	}
	
	var state map[string]FileOperation
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, err
	}
	
	// Load content for each file
	for path, file := range state {
		contentPath := filepath.Join(r.root, ".ivaldi", "content", file.Hash)
		content, err := os.ReadFile(contentPath)
		if err != nil {
			return nil, fmt.Errorf("failed to load content for %s: %v", path, err)
		}
		file.Content = content
		state[path] = file
	}
	
	return state, nil
}

// applyTimelineDiff applies the calculated diff to transform the working directory
func (r *Repository) applyTimelineDiff(diff *TimelineDiff) error {
	for _, op := range diff.Operations {
		fullPath := filepath.Join(r.root, op.Path)
		
		switch op.Type {
		case "add", "modify":
			// Ensure directory exists
			if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
				return err
			}
			// Write content
			if err := os.WriteFile(fullPath, op.Content, op.Mode); err != nil {
				return err
			}
			
		case "delete":
			// Remove file
			if err := os.Remove(fullPath); err != nil && !os.IsNotExist(err) {
				return err
			}
		}
	}
	
	return nil
}

// hashContent creates a content hash for deduplication
func (r *Repository) hashContent(content []byte) string {
	// Simple hash for now - could use SHA256 or similar
	return fmt.Sprintf("%x", len(content)) + fmt.Sprintf("%x", content[:min(len(content), 32)])
}

// Helper function for min
func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// clearWorkingDirectory removes all files from working directory except .ivaldi (for legacy compatibility)
func (r *Repository) clearWorkingDirectory() error {
	entries, err := os.ReadDir(r.root)
	if err != nil {
		return err
	}
	
	for _, entry := range entries {
		if entry.Name() == ".ivaldi" {
			continue // Skip .ivaldi directory
		}
		
		fullPath := filepath.Join(r.root, entry.Name())
		if err := os.RemoveAll(fullPath); err != nil {
			return err
		}
	}
	
	return nil
}

func (r *Repository) CurrentTimeline() string {
	return r.timeline.Current()
}

func (r *Repository) ListTimelines() []*timeline.Timeline {
	return r.timeline.List()
}

func (r *Repository) Jump(reference string) error {
	hash, err := r.position.ParseReference(reference)
	if err != nil {
		return err
	}

	return r.position.SetPosition(hash, r.timeline.Current())
}

func (r *Repository) Status() Status {
	r.workspace.Scan()

	var gathered, modified, untracked []string

	for path := range r.workspace.AnvilFiles {
		gathered = append(gathered, path)
	}

	for path, file := range r.workspace.Files {
		if file.OnAnvil {
			continue
		}
		
		switch file.Status {
		case workspace.StatusModified:
			modified = append(modified, path)
		case workspace.StatusAdded:
			untracked = append(untracked, path)
		}
	}

	currentPos := r.position.Current()
	positionName := "unknown"
	
	if name, exists := r.position.GetMemorableName(currentPos.Hash); exists {
		positionName = name
	}

	return Status{
		Timeline:  r.timeline.Current(),
		Position:  positionName,
		Gathered:  gathered,
		Modified:  modified,
		Untracked: untracked,
	}
}

func (r *Repository) History(limit int) ([]*objects.Seal, error) {
	hashes, err := r.index.GetSealHistory(limit)
	if err != nil {
		return nil, err
	}

	var seals []*objects.Seal
	for _, hash := range hashes {
		seal, err := r.storage.LoadSeal(hash)
		if err != nil {
			continue
		}
		seals = append(seals, seal)
	}

	return seals, nil
}

func (r *Repository) Close() error {
	if err := r.storage.Close(); err != nil {
		return err
	}
	return r.index.Close()
}

func (r *Repository) generateMemorableName() string {
	adjectives := []string{
		"bright", "swift", "bold", "calm", "wise", "strong", "gentle", "fierce",
		"noble", "quick", "sharp", "clear", "deep", "warm", "cool", "fresh",
		"steady", "keen", "proud", "pure", "dark", "light", "silver", "golden",
	}

	nouns := []string{
		"river", "mountain", "forest", "ocean", "star", "moon", "sun", "wind",
		"flame", "stone", "tree", "bird", "wolf", "eagle", "bear", "lion",
		"stream", "valley", "peak", "meadow", "lake", "shore", "path", "bridge",
	}

	adjective := adjectives[rand.Intn(len(adjectives))]
	noun := nouns[rand.Intn(len(nouns))]
	number := rand.Intn(999) + 1

	return fmt.Sprintf("%s-%s-%d", adjective, noun, number)
}

func (r *Repository) getNextIteration() int {
	seals, err := r.History(1)
	if err != nil || len(seals) == 0 {
		return 1
	}
	return seals[0].Iteration + 1
}

// Portal management
type PortalConfig struct {
	Portals map[string]string `json:"portals"`
}

func (r *Repository) loadPortalConfig() (*PortalConfig, error) {
	configPath := filepath.Join(r.root, ".ivaldi", "portals.json")
	
	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return &PortalConfig{Portals: make(map[string]string)}, nil
		}
		return nil, err
	}
	
	var config PortalConfig
	if err := json.Unmarshal(data, &config); err != nil {
		return nil, err
	}
	
	if config.Portals == nil {
		config.Portals = make(map[string]string)
	}
	
	return &config, nil
}

func (r *Repository) savePortalConfig(config *PortalConfig) error {
	configPath := filepath.Join(r.root, ".ivaldi", "portals.json")
	
	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(configPath, data, 0644)
}

func (r *Repository) AddPortal(name, url string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	config.Portals[name] = url
	
	// Save portal configuration (git-independent)
	return r.savePortalConfig(config)
}

func (r *Repository) RemovePortal(name string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	delete(config.Portals, name)
	
	// Save portal configuration (git-independent)
	return r.savePortalConfig(config)
}

func (r *Repository) ListPortals() map[string]string {
	config, err := r.loadPortalConfig()
	if err != nil {
		return make(map[string]string)
	}
	
	return config.Portals
}

func (r *Repository) Push(portalName string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Use Ivaldi-native push instead of git push
	currentTimeline := r.timeline.Current()
	portalURL := config.Portals[portalName]
	
	return r.syncMgr.Push(portalURL, currentTimeline)
}

func (r *Repository) Scout(portalName string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Use Ivaldi-native fetch without merging
	portalURL := config.Portals[portalName]
	fetchResult, err := r.network.FetchFromPortal(portalURL, "main")
	if err != nil {
		return fmt.Errorf("failed to scout: %v", err)
	}
	
	// Store fetched seals for later use
	for _, seal := range fetchResult.Seals {
		if err := r.storage.StoreSeal(seal); err != nil {
			return fmt.Errorf("failed to store fetched seal: %v", err)
		}
	}
	
	return nil
}

func (r *Repository) Pull(portalName string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Check if we have local changes
	if r.workspace.HasUncommittedChanges() {
		return fmt.Errorf("you have uncommitted changes, please seal them first or discard them")
	}
	
	// Use Ivaldi-native sync instead of git pull
	opts := sync.SyncOptions{
		PortalName:     portalName,
		RemoteTimeline: "main",
		LocalTimeline:  r.timeline.Current(),
		Strategy:       0, // Auto strategy
		Force:          false,
		DryRun:         false,
	}
	
	portalURL := config.Portals[portalName]
	result, err := r.syncMgr.Sync(portalURL, opts)
	if err != nil {
		return fmt.Errorf("failed to sync: %v", err)
	}
	
	if !result.Success {
		return fmt.Errorf("sync failed: %s", result.Message)
	}
	
	return nil
}

// Sync performs Ivaldi-native synchronization with a remote portal
func (r *Repository) Sync(portalName, localTimeline, remoteTimeline string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Check if we have local changes
	if r.workspace.HasUncommittedChanges() {
		return fmt.Errorf("you have uncommitted changes, please seal them first or discard them")
	}
	
	// Use default remote timeline if not specified
	if remoteTimeline == "" {
		remoteTimeline = "main"
	}
	
	// Use default local timeline if not specified
	if localTimeline == "" {
		localTimeline = r.timeline.Current()
	}
	
	// Use Ivaldi-native sync
	opts := sync.SyncOptions{
		PortalName:     portalName,
		RemoteTimeline: remoteTimeline,
		LocalTimeline:  localTimeline,
		Strategy:       0, // Auto strategy - will handle divergent branches properly
		Force:          false,
		DryRun:         false,
	}
	
	portalURL := config.Portals[portalName]
	result, err := r.syncMgr.Sync(portalURL, opts)
	if err != nil {
		return fmt.Errorf("failed to sync: %v", err)
	}
	
	if !result.Success {
		return fmt.Errorf("sync failed: %s", result.Message)
	}
	
	return nil
}

func (r *Repository) exportToGit() error {
	// Add all files to git
	cmd := exec.Command("git", "add", ".")
	cmd.Dir = r.root
	if err := cmd.Run(); err != nil {
		return err
	}
	
	// Get latest seal for commit message
	seals, err := r.History(1)
	if err != nil || len(seals) == 0 {
		return fmt.Errorf("no seals found")
	}
	
	message := fmt.Sprintf("[%s] %s", seals[0].Name, seals[0].Message)
	
	// Commit changes
	cmd = exec.Command("git", "commit", "-m", message)
	cmd.Dir = r.root
	cmd.Run() // Ignore error if nothing to commit
	
	return nil
}

func (r *Repository) importFromGit() error {
	// Scan workspace to pick up changes from git
	if err := r.workspace.Scan(); err != nil {
		return err
	}
	
	// Save workspace state
	return r.workspace.SaveState(r.timeline.Current())
}

func (r *Repository) importFromGitHistory() error {
	// Get the latest git commit
	cmd := exec.Command("git", "log", "-1", "--format=%H|%s|%an|%ae|%at")
	cmd.Dir = r.root
	output, err := cmd.Output()
	if err != nil {
		return err
	}
	
	parts := strings.Split(strings.TrimSpace(string(output)), "|")
	if len(parts) < 5 {
		return fmt.Errorf("unexpected git log format")
	}
	
	_ = parts[0] // gitHash - not used for now
	message := parts[1]
	authorName := parts[2]
	authorEmail := parts[3]
	timestampStr := parts[4]
	
	// Check if we already have this seal
	seals, err := r.History(1)
	if err == nil && len(seals) > 0 {
		if seals[0].Message == message {
			// Already imported
			return r.importFromGit()
		}
	}
	
	// Create a new seal from the git commit
	name := r.generateMemorableName()
	
	author := objects.Identity{
		Name:  authorName,
		Email: authorEmail,
	}
	
	// Parse timestamp
	timestamp := time.Now()
	if ts, err := strconv.ParseInt(timestampStr, 10, 64); err == nil {
		timestamp = time.Unix(ts, 0)
	}
	
	seal := &objects.Seal{
		Name:      name,
		Iteration: r.getNextIteration(),
		Message:   fmt.Sprintf("[Mirrored] %s", message),
		Author:    author,
		Timestamp: timestamp,
		Parents:   []objects.Hash{},
	}
	
	// Store the seal
	if err := r.storage.StoreSeal(seal); err != nil {
		return err
	}
	
	if err := r.index.IndexSeal(seal); err != nil {
		return err
	}
	
	if err := r.position.SetPosition(seal.Hash, r.timeline.Current()); err != nil {
		return err
	}
	
	if err := r.position.SetMemorableName(seal.Hash, name); err != nil {
		return err
	}
	
	if err := r.timeline.UpdateHead(r.timeline.Current(), seal.Hash); err != nil {
		return err
	}
	
	// Scan workspace and save state
	return r.importFromGit()
}

// Version management
type Version struct {
	Tag     string    `json:"tag"`
	Message string    `json:"message"`
	Seal    string    `json:"seal"`
	Date    time.Time `json:"date"`
}

type VersionConfig struct {
	Versions []Version `json:"versions"`
}

func (r *Repository) loadVersionConfig() (*VersionConfig, error) {
	configPath := filepath.Join(r.root, ".ivaldi", "versions.json")
	
	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return &VersionConfig{Versions: []Version{}}, nil
		}
		return nil, err
	}
	
	var config VersionConfig
	if err := json.Unmarshal(data, &config); err != nil {
		return nil, err
	}
	
	return &config, nil
}

func (r *Repository) saveVersionConfig(config *VersionConfig) error {
	configPath := filepath.Join(r.root, ".ivaldi", "versions.json")
	
	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(configPath, data, 0644)
}

func (r *Repository) CreateVersion(tag, message string) error {
	// Validate tag format
	if !strings.HasPrefix(tag, "v") {
		tag = "v" + tag
	}
	
	config, err := r.loadVersionConfig()
	if err != nil {
		return err
	}
	
	// Check if version already exists
	for _, v := range config.Versions {
		if v.Tag == tag {
			return fmt.Errorf("version %s already exists", tag)
		}
	}
	
	// Get current seal
	currentPos := r.position.Current()
	sealName := "unknown"
	if name, exists := r.position.GetMemorableName(currentPos.Hash); exists {
		sealName = name
	}
	
	version := Version{
		Tag:     tag,
		Message: message,
		Seal:    sealName,
		Date:    time.Now(),
	}
	
	config.Versions = append(config.Versions, version)
	
	// Create git tag
	gitDir := filepath.Join(r.root, ".git")
	if _, err := os.Stat(gitDir); err == nil {
		cmd := exec.Command("git", "tag", "-a", tag, "-m", message)
		cmd.Dir = r.root
		if err := cmd.Run(); err != nil {
			// Try without annotation if it fails
			cmd = exec.Command("git", "tag", tag)
			cmd.Dir = r.root
			cmd.Run()
		}
	}
	
	return r.saveVersionConfig(config)
}

func (r *Repository) ListVersions() []Version {
	config, err := r.loadVersionConfig()
	if err != nil {
		return []Version{}
	}
	
	// Sort by date (newest first)
	versions := config.Versions
	for i := 0; i < len(versions)-1; i++ {
		for j := i + 1; j < len(versions); j++ {
			if versions[j].Date.After(versions[i].Date) {
				versions[i], versions[j] = versions[j], versions[i]
			}
		}
	}
	
	return versions
}

func (r *Repository) PushVersion(tag string) error {
	// Ensure we have a portal configured
	portals := r.ListPortals()
	if len(portals) == 0 {
		return fmt.Errorf("no portals configured, use 'ivaldi portal add' first")
	}
	
	// Find the origin portal or use the first one
	portalName := "origin"
	if _, exists := portals[portalName]; !exists {
		for name := range portals {
			portalName = name
			break
		}
	}
	
	// Push the tag to remote
	cmd := exec.Command("git", "push", portalName, tag)
	cmd.Dir = r.root
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to push version %s: %v", tag, err)
	}
	
	return nil
}

func (r *Repository) PushAllVersions() error {
	// Ensure we have a portal configured
	portals := r.ListPortals()
	if len(portals) == 0 {
		return fmt.Errorf("no portals configured, use 'ivaldi portal add' first")
	}
	
	// Find the origin portal or use the first one
	portalName := "origin"
	if _, exists := portals[portalName]; !exists {
		for name := range portals {
			portalName = name
			break
		}
	}
	
	// Push all tags to remote
	cmd := exec.Command("git", "push", portalName, "--tags")
	cmd.Dir = r.root
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to push versions: %v", err)
	}
	
	return nil
}

func (r *Repository) PushToBranch(portalName, branch string, setUpstream bool) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Determine timeline to push
	targetTimeline := branch
	if targetTimeline == "" {
		targetTimeline = r.timeline.Current() // Use current timeline as default
	}
	
	// Use Ivaldi-native push instead of git push
	portalURL := config.Portals[portalName]
	return r.syncMgr.Push(portalURL, targetTimeline)
}

func (r *Repository) PullFromBranch(portalName, branch string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// Check if we have local changes
	if r.workspace.HasUncommittedChanges() {
		return fmt.Errorf("you have uncommitted changes, please seal them first or discard them")
	}
	
	// Determine branch to pull from
	sourceBranch := branch
	if sourceBranch == "" {
		sourceBranch = r.timeline.Current() // Use current timeline as default
	}
	
	// Use Ivaldi-native sync instead of git pull
	opts := sync.SyncOptions{
		PortalName:     portalName,
		RemoteTimeline: sourceBranch,
		LocalTimeline:  r.timeline.Current(),
		Strategy:       0, // Auto strategy
		Force:          false,
		DryRun:         false,
	}
	
	portalURL := config.Portals[portalName]
	result, err := r.syncMgr.Sync(portalURL, opts)
	if err != nil {
		return fmt.Errorf("failed to sync: %v", err)
	}
	
	if !result.Success {
		return fmt.Errorf("sync failed: %s", result.Message)
	}
	
	return nil
}

func (r *Repository) CreateBranchAndMigrate(newBranch, fromBranch string) error {
	// Create new branch from current position
	createCmd := exec.Command("git", "checkout", "-b", newBranch)
	createCmd.Dir = r.root
	if err := createCmd.Run(); err != nil {
		return fmt.Errorf("failed to create branch %s: %v", newBranch, err)
	}
	
	// If we need to migrate from a different branch
	if fromBranch != "" && fromBranch != newBranch {
		// Switch to the from branch to get its content
		checkoutCmd := exec.Command("git", "checkout", fromBranch)
		checkoutCmd.Dir = r.root
		if err := checkoutCmd.Run(); err != nil {
			return fmt.Errorf("failed to checkout %s: %v", fromBranch, err)
		}
		
		// Merge the content into our new branch
		switchBackCmd := exec.Command("git", "checkout", newBranch)
		switchBackCmd.Dir = r.root
		if err := switchBackCmd.Run(); err != nil {
			return fmt.Errorf("failed to switch back to %s: %v", newBranch, err)
		}
		
		mergeCmd := exec.Command("git", "merge", fromBranch)
		mergeCmd.Dir = r.root
		if err := mergeCmd.Run(); err != nil {
			return fmt.Errorf("failed to merge %s into %s: %v", fromBranch, newBranch, err)
		}
	}
	
	return nil
}

func (r *Repository) UploadToPortal(portalName, branch string) error {
	// Simple upload - just push with upstream
	return r.PushToBranch(portalName, branch, true)
}

func (r *Repository) RenameBranchOnPortal(portalName, oldBranch, newBranch string) error {
	config, err := r.loadPortalConfig()
	if err != nil {
		return err
	}
	
	if _, exists := config.Portals[portalName]; !exists {
		return fmt.Errorf("portal '%s' not found", portalName)
	}
	
	// First, create the new branch from the old branch on the remote
	// We need to push the old branch content to the new branch name
	pushCmd := exec.Command("git", "push", portalName, fmt.Sprintf("%s:%s", oldBranch, newBranch))
	pushCmd.Dir = r.root
	if err := pushCmd.Run(); err != nil {
		return fmt.Errorf("failed to create new branch %s from %s: %v", newBranch, oldBranch, err)
	}
	
	// Then delete the old branch on the remote
	deleteCmd := exec.Command("git", "push", portalName, "--delete", oldBranch)
	deleteCmd.Dir = r.root
	if err := deleteCmd.Run(); err != nil {
		return fmt.Errorf("failed to delete old branch %s: %v", oldBranch, err)
	}
	
	return nil
}

// GetIndex returns the repository's index for search operations
func (r *Repository) GetIndex() *index.SQLiteIndex {
	return r.index
}

// GetStorage returns the repository's storage for loading objects
func (r *Repository) GetStorage() *local.Storage {
	return r.storage
}

// TimelineManager interface implementation for FuseManager
func (r *Repository) Current() string {
	return r.timeline.Current()
}

func (r *Repository) GetHead(timeline string) (objects.Hash, error) {
	head, err := r.timeline.GetHead(timeline)
	if err != nil {
		return objects.Hash{}, err
	}
	return head, nil
}

func (r *Repository) UpdateHead(timeline string, hash objects.Hash) error {
	return r.timeline.UpdateHead(timeline, hash)
}

func (r *Repository) DeleteTimeline(name string) error {
	return r.timeline.Delete(name)
}

// WorkspaceManager interface implementation for FuseManager
func (r *Repository) HasUncommittedChanges() bool {
	return r.workspace.HasUncommittedChanges()
}

func (r *Repository) SaveState(timeline string) error {
	return r.workspace.SaveState(timeline)
}

func (r *Repository) LoadState(timeline string) error {
	return r.workspace.LoadState(timeline)
}

// RestoreWorkingDirectory restores the working directory to match a specific commit
func (r *Repository) RestoreWorkingDirectory(targetHash objects.Hash) error {
	// Check if target hash is empty (no commits yet)
	emptyHash := objects.Hash{}
	hashString := targetHash.String()
	
	// If target hash is empty or all zeros, clear working directory
	if targetHash == emptyHash || hashString == "0000000000000000000000000000000000000000000000000000000000000000" {
		return r.clearWorkingDirectory()
	}

	// Check if the seal actually exists in storage
	if !r.storage.Exists(targetHash) {
		// Seal doesn't exist, treat as empty repository
		return r.clearWorkingDirectory()
	}

	// Load the target seal
	seal, err := r.storage.LoadSeal(targetHash)
	if err != nil {
		return fmt.Errorf("failed to load seal: %v", err)
	}

	// Debug: check the seal's position
	fmt.Printf("Debug: seal position: %s\n", seal.Position.String())

	// Check if seal has a valid position set
	emptyPos := objects.Hash{}
	if seal.Position == emptyPos {
		// Seal has no tree stored - treat as empty repository
		return r.clearWorkingDirectory()
	}

	// Load the tree from the seal's position
	tree, err := r.storage.LoadTree(seal.Position)
	if err != nil {
		return fmt.Errorf("failed to load tree: %v", err)
	}

	// Clear working directory first
	if err := r.clearWorkingDirectory(); err != nil {
		return err
	}

	// Restore files from tree
	return r.restoreFromTree(tree, "")
}

// restoreFromTree recursively restores files from a tree object
func (r *Repository) restoreFromTree(tree *objects.Tree, basePath string) error {
	for _, entry := range tree.Entries {
		entryPath := filepath.Join(basePath, entry.Name)
		fullPath := filepath.Join(r.root, entryPath)

		switch entry.Type {
		case objects.ObjectTypeTree:
			// Create directory and recurse
			if err := os.MkdirAll(fullPath, 0755); err != nil {
				return err
			}
			subTree, err := r.storage.LoadTree(entry.Hash)
			if err != nil {
				return err
			}
			if err := r.restoreFromTree(subTree, entryPath); err != nil {
				return err
			}

		case objects.ObjectTypeBlob:
			// Restore file
			blob, err := r.storage.LoadBlob(entry.Hash)
			if err != nil {
				// Try to find actual blob hash using mapping
				actualHash, found := r.loadBlobHashMapping(entry.Hash)
				if found {
					blob, err = r.storage.LoadBlob(actualHash)
					if err == nil {
						fmt.Printf("Successfully loaded blob using hash mapping for file %s\n", entryPath)
					}
				}
				
				if err != nil {
					return fmt.Errorf("failed to load blob %s for file %s: %v", entry.Hash.String(), entryPath, err)
				}
			}
			
			// Ensure directory exists
			dir := filepath.Dir(fullPath)
			if err := os.MkdirAll(dir, 0755); err != nil {
				return err
			}
			
			// Write file
			if err := os.WriteFile(fullPath, blob.Data, os.FileMode(entry.Mode)); err != nil {
				return err
			}
		}
	}
	return nil
}

// isIgnored checks if a file path should be ignored
func (r *Repository) isIgnored(path string) bool {
	for _, pattern := range r.workspace.IgnorePattern {
		if matched, _ := filepath.Match(pattern, path); matched {
			return true
		}
		if matched, _ := filepath.Match(pattern, filepath.Base(path)); matched {
			return true
		}
	}
	return false
}

// convertCATreeToLegacyTree converts a content-addressed tree to legacy Tree format
func (r *Repository) convertCATreeToLegacyTree(caTree *objects.CATree, treeHash objects.CAHash) *objects.Tree {
	var entries []objects.TreeEntry
	
	for _, caEntry := range caTree.Entries {
		// Convert CAHash to legacy Hash
		var legacyHash objects.Hash
		copy(legacyHash[:], caEntry.Hash.Bytes())
		
		// Convert ObjectKind to ObjectType
		var objType objects.ObjectType
		switch caEntry.Kind {
		case objects.KindBlob:
			objType = objects.ObjectTypeBlob
		case objects.KindTree:
			objType = objects.ObjectTypeTree
		default:
			objType = objects.ObjectTypeBlob // Default to blob
		}
		
		entry := objects.TreeEntry{
			Name: caEntry.Name,
			Hash: legacyHash,
			Mode: caEntry.Mode,
			Type: objType,
		}
		entries = append(entries, entry)
	}
	
	// Create legacy hash for tree
	var legacyTreeHash objects.Hash
	copy(legacyTreeHash[:], treeHash.Bytes())
	
	return &objects.Tree{
		Hash:    legacyTreeHash,
		Entries: entries,
	}
}

// storeCABlobsAsLegacy stores all blobs from a CA tree in legacy format
func (r *Repository) storeCABlobsAsLegacy(caTree *objects.CATree) error {
	// Create a mapping to track hash conversions
	r.createBlobHashMapping(caTree)
	
	for _, entry := range caTree.Entries {
		if entry.Kind == objects.KindBlob {
			// Load blob data from CA store
			data, _, err := r.workspace.Store.Get(entry.Hash)
			if err != nil {
				return fmt.Errorf("failed to load CA blob %s: %v", entry.Hash.String(), err)
			}

			// Create legacy blob with the data
			legacyBlob := objects.NewBlob(data)
			
			// Store using StoreObject which will compute hash
			storedHash, err := r.storage.StoreObject(legacyBlob)
			if err != nil {
				return fmt.Errorf("failed to store legacy blob: %v", err)
			}
			
			// Store the hash mapping for restoration
			expectedHash := objects.Hash{}
			copy(expectedHash[:], entry.Hash.Bytes())
			if storedHash != expectedHash {
				// Different hash due to different algorithms - store mapping
				r.storeBlobHashMapping(expectedHash, storedHash, entry.Name)
			}
		}
	}
	return nil
}

// createBlobHashMapping initializes hash mapping storage
func (r *Repository) createBlobHashMapping(caTree *objects.CATree) {
	// Initialize blob hash mapping directory
	mappingDir := filepath.Join(r.root, ".ivaldi", "blob_mappings")
	os.MkdirAll(mappingDir, 0755)
}

// storeBlobHashMapping stores a mapping between expected and actual blob hashes
func (r *Repository) storeBlobHashMapping(expectedHash, actualHash objects.Hash, filename string) {
	mappingPath := filepath.Join(r.root, ".ivaldi", "blob_mappings", "mapping.json")
	
	type BlobMapping struct {
		ExpectedToActual map[string]string `json:"expected_to_actual"`
		ActualToExpected map[string]string `json:"actual_to_expected"`
		FileNames        map[string]string `json:"file_names"`
	}
	
	var mapping BlobMapping
	if data, err := os.ReadFile(mappingPath); err == nil {
		json.Unmarshal(data, &mapping)
	} else {
		mapping = BlobMapping{
			ExpectedToActual: make(map[string]string),
			ActualToExpected: make(map[string]string),
			FileNames:        make(map[string]string),
		}
	}
	
	expectedStr := expectedHash.String()
	actualStr := actualHash.String()
	
	mapping.ExpectedToActual[expectedStr] = actualStr
	mapping.ActualToExpected[actualStr] = expectedStr
	mapping.FileNames[expectedStr] = filename
	
	if data, err := json.MarshalIndent(mapping, "", "  "); err == nil {
		os.WriteFile(mappingPath, data, 0644)
	}
}

// loadBlobHashMapping loads the actual hash for an expected hash
func (r *Repository) loadBlobHashMapping(expectedHash objects.Hash) (objects.Hash, bool) {
	mappingPath := filepath.Join(r.root, ".ivaldi", "blob_mappings", "mapping.json")
	
	type BlobMapping struct {
		ExpectedToActual map[string]string `json:"expected_to_actual"`
		ActualToExpected map[string]string `json:"actual_to_expected"`
		FileNames        map[string]string `json:"file_names"`
	}
	
	data, err := os.ReadFile(mappingPath)
	if err != nil {
		return objects.Hash{}, false
	}
	
	var mapping BlobMapping
	if err := json.Unmarshal(data, &mapping); err != nil {
		return objects.Hash{}, false
	}
	
	expectedStr := expectedHash.String()
	if actualStr, exists := mapping.ExpectedToActual[expectedStr]; exists {
		// Parse actual hash string back to Hash
		bytes, err := hex.DecodeString(actualStr)
		if err != nil || len(bytes) != 32 {
			return objects.Hash{}, false
		}
		
		var actualHash objects.Hash
		copy(actualHash[:], bytes)
		return actualHash, true
	}
	
	return objects.Hash{}, false
}

// copyWorkspaceState copies workspace state from source timeline to target timeline
func (r *Repository) copyWorkspaceState(sourceTimeline, targetTimeline string) error {
	// Load the source workspace state properly
	sourceWorkspace := workspace.New(r.root, r.workspace.Store)
	if err := sourceWorkspace.LoadState(sourceTimeline); err != nil {
		if os.IsNotExist(err) {
			// No state to copy, which is fine
			return nil
		}
		return err
	}
	
	// Update the timeline field and save as target timeline
	sourceWorkspace.Timeline = targetTimeline
	
	// Save the complete workspace state to the target timeline
	return sourceWorkspace.SaveState(targetTimeline)
}

// shelveUncommittedChanges saves uncommitted changes to a shelve for later restoration
func (r *Repository) shelveUncommittedChanges(timeline string) error {
	shelveDir := filepath.Join(r.root, ".ivaldi", "shelves", timeline)
	if err := os.MkdirAll(shelveDir, 0755); err != nil {
		return err
	}
	
	// Create a shelve entry
	shelveTime := time.Now()
	shelveID := fmt.Sprintf("auto-shelve-%d", shelveTime.Unix())
	
	type ShelveEntry struct {
		ID          string                        `json:"id"`
		Timeline    string                        `json:"timeline"`
		Timestamp   time.Time                     `json:"timestamp"`
		Files       map[string]*workspace.FileState `json:"files"`
		AnvilFiles  map[string]*workspace.FileState `json:"anvil_files"`
		Description string                        `json:"description"`
	}
	
	shelve := ShelveEntry{
		ID:          shelveID,
		Timeline:    timeline,
		Timestamp:   shelveTime,
		Files:       make(map[string]*workspace.FileState),
		AnvilFiles:  make(map[string]*workspace.FileState),
		Description: "Auto-shelved before timeline switch",
	}
	
	// Copy uncommitted and anvil files
	for path, fileState := range r.workspace.Files {
		if fileState.Status != workspace.StatusUnmodified {
			shelve.Files[path] = fileState
		}
	}
	
	for path, fileState := range r.workspace.AnvilFiles {
		shelve.AnvilFiles[path] = fileState
	}
	
	// Save shelve to file
	shelveData, err := json.MarshalIndent(shelve, "", "  ")
	if err != nil {
		return err
	}
	
	shelveFile := filepath.Join(shelveDir, shelveID+".json")
	return os.WriteFile(shelveFile, shelveData, 0644)
}

// restoreFromShelve restores the latest auto-shelve for a timeline
func (r *Repository) restoreFromShelve(timeline string) error {
	shelveDir := filepath.Join(r.root, ".ivaldi", "shelves", timeline)
	
	// Find the latest auto-shelve
	files, err := os.ReadDir(shelveDir)
	if err != nil {
		if os.IsNotExist(err) {
			// No shelves exist, which is fine
			return nil
		}
		return err
	}
	
	var latestShelve string
	var latestTime time.Time
	
	for _, file := range files {
		if strings.HasPrefix(file.Name(), "auto-shelve-") && strings.HasSuffix(file.Name(), ".json") {
			shelveData, err := os.ReadFile(filepath.Join(shelveDir, file.Name()))
			if err != nil {
				continue
			}
			
			var shelve map[string]interface{}
			if err := json.Unmarshal(shelveData, &shelve); err != nil {
				continue
			}
			
			if timestampStr, ok := shelve["timestamp"].(string); ok {
				if timestamp, err := time.Parse(time.RFC3339, timestampStr); err == nil {
					if timestamp.After(latestTime) {
						latestTime = timestamp
						latestShelve = file.Name()
					}
				}
			}
		}
	}
	
	if latestShelve == "" {
		// No auto-shelves found
		return nil
	}
	
	// Load and restore the latest shelve
	shelveData, err := os.ReadFile(filepath.Join(shelveDir, latestShelve))
	if err != nil {
		return err
	}
	
	type ShelveEntry struct {
		ID          string                        `json:"id"`
		Timeline    string                        `json:"timeline"`
		Timestamp   time.Time                     `json:"timestamp"`
		Files       map[string]*workspace.FileState `json:"files"`
		AnvilFiles  map[string]*workspace.FileState `json:"anvil_files"`
		Description string                        `json:"description"`
	}
	
	var shelve ShelveEntry
	if err := json.Unmarshal(shelveData, &shelve); err != nil {
		return err
	}
	
	fmt.Printf("Restoring auto-shelved changes from %s\n", shelve.Timestamp.Format("2006-01-02 15:04:05"))
	
	// Merge shelved files back into workspace
	if r.workspace.Files == nil {
		r.workspace.Files = make(map[string]*workspace.FileState)
	}
	if r.workspace.AnvilFiles == nil {
		r.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	}
	
	for path, fileState := range shelve.Files {
		r.workspace.Files[path] = fileState
	}
	
	for path, fileState := range shelve.AnvilFiles {
		r.workspace.AnvilFiles[path] = fileState
	}
	
	return nil
}

// restoreFilesFromWorkspaceState restores files based on workspace state when tree restoration fails
func (r *Repository) restoreFilesFromWorkspaceState() error {
	if r.workspace.Files == nil || len(r.workspace.Files) == 0 {
		return nil // No files to restore
	}
	
	for path, fileState := range r.workspace.Files {
		if fileState.Status == workspace.StatusUnmodified {
			// This file should exist on this timeline
			fullPath := filepath.Join(r.root, path)
			
			// Check if file already exists
			if _, err := os.Stat(fullPath); err == nil {
				continue // File already exists
			}
			
			// Try to restore from CA store using BlobHash
			if !fileState.BlobHash.IsZero() {
				if err := r.restoreFileFromStore(path, fileState.BlobHash); err != nil {
					fmt.Printf("Warning: failed to restore %s from store: %v\n", path, err)
					continue
				}
				fmt.Printf("Restored %s from content store\n", path)
			}
		}
	}
	
	return nil
}

// restoreFileFromStore restores a single file from the content store
func (r *Repository) restoreFileFromStore(path string, hash objects.CAHash) error {
	// Get file data from store
	data, _, err := r.workspace.Store.Get(hash)
	if err != nil {
		return err
	}
	
	fullPath := filepath.Join(r.root, path)
	
	// Create directory if needed
	dir := filepath.Dir(fullPath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	
	// Write file
	return os.WriteFile(fullPath, data, 0644)
}