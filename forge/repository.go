package forge

import (
	"encoding/json"
	"fmt"
	"io/fs"
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

	name := r.generateMemorableName()
	
	author := objects.Identity{
		Name:  "Developer",
		Email: "dev@example.com",
	}

	seal := &objects.Seal{
		Name:      name,
		Iteration: r.getNextIteration(),
		Message:   message,
		Author:    author,
		Timestamp: time.Now(),
		Parents:   []objects.Hash{r.position.Current().Hash},
	}

	if err := r.storage.StoreSeal(seal); err != nil {
		return nil, err
	}

	if err := r.index.IndexSeal(seal); err != nil {
		return nil, err
	}

	if err := r.position.SetPosition(seal.Hash, r.timeline.Current()); err != nil {
		return nil, err
	}

	if err := r.position.SetMemorableName(seal.Hash, name); err != nil {
		return nil, err
	}
	
	// Register the memorable name with the reference manager
	if err := r.refMgr.RegisterMemorableName(name, seal.Hash, seal.Author.Name); err != nil {
		return nil, err
	}

	if err := r.timeline.UpdateHead(r.timeline.Current(), seal.Hash); err != nil {
		return nil, err
	}

	// Clear the anvil and update file statuses
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
			}
		}
	}
	
	// Now clear the anvil
	r.workspace.AnvilFiles = make(map[string]*workspace.FileState)
	
	// Save the updated workspace state
	if err := r.workspace.SaveState(r.timeline.Current()); err != nil {
		return seal, err
	}

	return seal, nil
}

func (r *Repository) CreateTimeline(name, description string) error {
	return r.timeline.Create(name, description)
}

func (r *Repository) SwitchTimeline(name string) error {
	// Save current workspace state if there are uncommitted changes
	if r.workspace.HasUncommittedChanges() {
		if err := r.workspace.SaveState(r.timeline.Current()); err != nil {
			return fmt.Errorf("failed to save workspace state: %v", err)
		}
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

	// Restore working directory to match target timeline's HEAD
	if err := r.RestoreWorkingDirectory(targetHead); err != nil {
		return fmt.Errorf("failed to restore working directory: %v", err)
	}

	// Load target timeline's workspace state
	if err := r.workspace.LoadState(name); err != nil {
		return fmt.Errorf("failed to load workspace state: %v", err)
	}

	// Rescan workspace to update file tracking after restoration
	if err := r.workspace.Scan(); err != nil {
		return fmt.Errorf("failed to scan workspace after switch: %v", err)
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

// clearWorkingDirectory removes all tracked files from working directory
func (r *Repository) clearWorkingDirectory() error {
	// Get all files currently in the working directory (except ignored ones)
	return filepath.WalkDir(r.root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}

		if d.IsDir() {
			// Skip special directories
			if d.Name() == ".ivaldi" || d.Name() == ".git" || d.Name() == "build" {
				return filepath.SkipDir
			}
			return nil
		}

		// Remove file if it's tracked (not ignored)
		relPath, err := filepath.Rel(r.root, path)
		if err != nil {
			return err
		}

		if !r.isIgnored(relPath) {
			return os.Remove(path)
		}

		return nil
	})
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
				return err
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