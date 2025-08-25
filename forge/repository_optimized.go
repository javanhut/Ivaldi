package forge

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"ivaldi/core/objects"
	"ivaldi/core/workspace"
)

// OptimizedImporter handles efficient Git history import
type OptimizedImporter struct {
	repo         *Repository
	blobCache    map[string]objects.Hash // Git SHA -> Ivaldi Hash cache
	treeCache    map[string]objects.Hash // Git tree SHA -> Ivaldi Hash cache
	cacheMutex   sync.RWMutex
	workerPool   chan struct{} // Limits concurrent workers
	progressChan chan string   // Progress reporting
}

// NewOptimizedImporter creates a new optimized importer
func NewOptimizedImporter(repo *Repository) *OptimizedImporter {
	return &OptimizedImporter{
		repo:         repo,
		blobCache:    make(map[string]objects.Hash),
		treeCache:    make(map[string]objects.Hash),
		workerPool:   make(chan struct{}, 8), // 8 concurrent workers
		progressChan: make(chan string, 100),
	}
}

// ImportGitHistoryOptimized efficiently imports Git history without repeated checkouts
func (r *Repository) ImportGitHistoryOptimized() error {
	gitDir := filepath.Join(r.root, ".git")
	if _, err := os.Stat(gitDir); os.IsNotExist(err) {
		return fmt.Errorf("no Git repository found")
	}

	importer := NewOptimizedImporter(r)

	// Start progress reporter
	go importer.reportProgress()

	// Phase 1: Pre-cache all Git blobs to avoid redundant processing
	importer.progressChan <- "Phase 1: Caching Git objects..."
	if err := importer.preCacheGitObjects(); err != nil {
		return fmt.Errorf("failed to cache Git objects: %v", err)
	}

	// Phase 2: Import branches in parallel
	importer.progressChan <- "Phase 2: Importing branches..."
	if err := importer.importBranchesParallel(); err != nil {
		return fmt.Errorf("failed to import branches: %v", err)
	}

	// Phase 3: Set final position
	if err := importer.setFinalPosition(); err != nil {
		return fmt.Errorf("failed to set final position: %v", err)
	}

	close(importer.progressChan)

	// Sync memorable names
	r.position.SyncMemorableNamesFromReference(r.refMgr.GetMemorableName)

	fmt.Printf("Successfully imported Git history (optimized)\n")
	return nil
}

// preCacheGitObjects pre-processes all unique Git objects once
func (oi *OptimizedImporter) preCacheGitObjects() error {
	// Get all unique blob SHAs from Git
	cmd := exec.Command("git", "-C", oi.repo.root, "rev-list", "--objects", "--all")
	output, err := cmd.Output()
	if err != nil {
		return fmt.Errorf("failed to list Git objects: %v", err)
	}

	lines := strings.Split(string(output), "\n")
	uniqueBlobs := make(map[string]bool)

	for _, line := range lines {
		parts := strings.Fields(line)
		if len(parts) == 0 {
			continue
		}
		sha := parts[0]

		// Check object type
		typeCmd := exec.Command("git", "-C", oi.repo.root, "cat-file", "-t", sha)
		typeOutput, err := typeCmd.Output()
		if err != nil {
			continue
		}

		objType := strings.TrimSpace(string(typeOutput))
		if objType == "blob" {
			uniqueBlobs[sha] = true
		}
	}

	oi.progressChan <- fmt.Sprintf("Found %d unique blobs to process", len(uniqueBlobs))

	// Process blobs in parallel batches
	blobList := make([]string, 0, len(uniqueBlobs))
	for sha := range uniqueBlobs {
		blobList = append(blobList, sha)
	}

	// Process in batches of 100
	batchSize := 100
	var wg sync.WaitGroup

	for i := 0; i < len(blobList); i += batchSize {
		end := i + batchSize
		if end > len(blobList) {
			end = len(blobList)
		}

		wg.Add(1)
		batch := blobList[i:end]

		go func(batchBlobs []string) {
			defer wg.Done()
			oi.workerPool <- struct{}{}        // Acquire worker slot
			defer func() { <-oi.workerPool }() // Release worker slot

			for _, sha := range batchBlobs {
				if err := oi.processBlobOptimized(sha); err != nil {
					fmt.Printf("Warning: failed to process blob %s: %v\n", sha[:8], err)
				}
			}
		}(batch)
	}

	wg.Wait()
	oi.progressChan <- fmt.Sprintf("Cached %d blobs", len(oi.blobCache))
	return nil
}

// processBlobOptimized processes a single Git blob and caches the result
func (oi *OptimizedImporter) processBlobOptimized(gitSHA string) error {
	// Check if already cached
	oi.cacheMutex.RLock()
	if _, exists := oi.blobCache[gitSHA]; exists {
		oi.cacheMutex.RUnlock()
		return nil
	}
	oi.cacheMutex.RUnlock()

	// Get blob content using git cat-file (no checkout needed)
	cmd := exec.Command("git", "-C", oi.repo.root, "cat-file", "blob", gitSHA)
	content, err := cmd.Output()
	if err != nil {
		return err
	}

	// Create and store Ivaldi blob
	blob := &objects.Blob{Data: content}
	blobHash, err := oi.repo.storage.StoreObject(blob)
	if err != nil {
		return err
	}

	// Cache the mapping
	oi.cacheMutex.Lock()
	oi.blobCache[gitSHA] = blobHash
	oi.cacheMutex.Unlock()

	return nil
}

// importBranchesParallel imports all branches concurrently
func (oi *OptimizedImporter) importBranchesParallel() error {
	// Get all branches
	branchesCmd := exec.Command("git", "-C", oi.repo.root, "branch", "-r")
	output, err := branchesCmd.Output()
	if err != nil {
		return fmt.Errorf("failed to list branches: %v", err)
	}

	lines := strings.Split(string(output), "\n")
	var branches []string

	for _, line := range lines {
		branchName := strings.TrimSpace(line)
		if branchName == "" || strings.Contains(branchName, "HEAD") {
			continue
		}

		if strings.HasPrefix(branchName, "origin/") {
			branchName = strings.TrimPrefix(branchName, "origin/")
		}
		branches = append(branches, branchName)
	}

	oi.progressChan <- fmt.Sprintf("Importing %d branches", len(branches))

	// Create all timelines first (sequentially to avoid race condition)
	for _, branchName := range branches {
		if !oi.repo.timeline.Exists(branchName) {
			if err := oi.repo.timeline.Create(branchName, fmt.Sprintf("Imported from Git branch %s", branchName)); err != nil {
				fmt.Printf("Warning: failed to create timeline for branch %s: %v\n", branchName, err)
			}
		}
	}

	// Now import commits in parallel (safe since timelines already exist)
	var wg sync.WaitGroup
	branchResults := make(map[string]error)
	var resultsMutex sync.Mutex

	// Limit concurrency to avoid overwhelming the system
	semaphore := make(chan struct{}, 4) // Max 4 concurrent branches

	for _, branch := range branches {
		wg.Add(1)
		go func(branchName string) {
			defer wg.Done()

			// Acquire semaphore
			semaphore <- struct{}{}
			defer func() { <-semaphore }()

			// Import commits for this branch
			err := oi.importBranchCommitsOptimized(branchName)
			resultsMutex.Lock()
			branchResults[branchName] = err
			resultsMutex.Unlock()

			if err == nil {
				oi.progressChan <- fmt.Sprintf("Completed branch: %s", branchName)
			}
		}(branch)
	}

	wg.Wait()

	// Check for errors
	for branch, err := range branchResults {
		if err != nil {
			fmt.Printf("Warning: failed to import branch %s: %v\n", branch, err)
		}
	}

	return nil
}

// importBranchCommitsOptimized imports commits for a branch without checkouts
func (oi *OptimizedImporter) importBranchCommitsOptimized(branchName string) error {
	// Get commit list for branch
	cmd := exec.Command("git", "-C", oi.repo.root, "rev-list", "--reverse", "origin/"+branchName)
	output, err := cmd.Output()
	if err != nil {
		// Try without origin/ prefix
		cmd = exec.Command("git", "-C", oi.repo.root, "rev-list", "--reverse", branchName)
		output, err = cmd.Output()
		if err != nil {
			return fmt.Errorf("failed to get commits for branch %s: %v", branchName, err)
		}
	}

	commitSHAs := strings.Split(strings.TrimSpace(string(output)), "\n")
	commitMap := make(map[string]objects.Hash)

	// Process commits in batches for better performance
	batchSize := 10
	for i := 0; i < len(commitSHAs); i += batchSize {
		end := i + batchSize
		if end > len(commitSHAs) {
			end = len(commitSHAs)
		}

		batch := commitSHAs[i:end]
		batchMap, err := oi.processCommitBatch(batch, branchName, i, commitMap)
		if err != nil {
			return err
		}

		// Merge batch results
		for k, v := range batchMap {
			commitMap[k] = v
		}
	}

	// Update timeline head
	if len(commitMap) > 0 {
		var latestHash objects.Hash
		lastSHA := commitSHAs[len(commitSHAs)-1]
		if hash, exists := commitMap[lastSHA]; exists {
			latestHash = hash
			if err := oi.repo.timeline.UpdateHead(branchName, latestHash); err != nil {
				return fmt.Errorf("failed to update timeline head for %s: %v", branchName, err)
			}
		}
	}

	return nil
}

// processCommitBatch processes a batch of commits efficiently
func (oi *OptimizedImporter) processCommitBatch(commitSHAs []string, branchName string, startIndex int, parentMap map[string]objects.Hash) (map[string]objects.Hash, error) {
	results := make(map[string]objects.Hash)
	sealsToIndex := make([]*objects.Seal, 0, len(commitSHAs))

	for idx, gitSHA := range commitSHAs {
		// Get commit info without checkout
		commitInfo, err := oi.getCommitInfo(gitSHA)
		if err != nil {
			fmt.Printf("Warning: failed to get info for commit %s: %v\n", gitSHA[:8], err)
			continue
		}

		// Create tree from Git tree object (no checkout needed)
		treeHash, err := oi.createTreeFromGitTree(commitInfo.TreeSHA)
		if err != nil {
			fmt.Printf("Warning: failed to create tree for commit %s: %v\n", gitSHA[:8], err)
			continue
		}

		// Convert parent SHAs
		var parentHashes []objects.Hash
		for _, parentSHA := range commitInfo.ParentSHAs {
			if parentHash, exists := parentMap[parentSHA]; exists {
				parentHashes = append(parentHashes, parentHash)
			} else if parentHash, exists := results[parentSHA]; exists {
				parentHashes = append(parentHashes, parentHash)
			}
		}

		// Create seal
		seal := &objects.Seal{
			Name:      oi.repo.generateMemorableName(),
			Iteration: startIndex + idx + 1,
			Position:  treeHash,
			Message:   commitInfo.Message,
			Author:    commitInfo.Author,
			Timestamp: commitInfo.Timestamp,
			Parents:   parentHashes,
		}

		// Store seal
		if err := oi.repo.storage.StoreSeal(seal); err != nil {
			return nil, fmt.Errorf("failed to store seal for commit %s: %v", gitSHA[:8], err)
		}

		// Add to batch for indexing
		sealsToIndex = append(sealsToIndex, seal)

		// Register memorable name
		if err := oi.repo.refMgr.RegisterMemorableName(seal.Name, seal.Hash, seal.Author.Name); err != nil {
			fmt.Printf("Warning: failed to register memorable name for commit %s: %v\n", gitSHA[:8], err)
		}

		oi.repo.position.AddMemorableName(seal.Hash, seal.Name)
		results[gitSHA] = seal.Hash
	}

	// Batch index all seals at once
	if len(sealsToIndex) > 0 {
		if err := oi.repo.index.BatchIndexSeals(sealsToIndex); err != nil {
			// Fall back to individual indexing on batch failure
			for _, seal := range sealsToIndex {
				if err := oi.repo.index.IndexSeal(seal); err != nil {
					fmt.Printf("Warning: failed to index seal %s: %v\n", seal.Name, err)
				}
			}
		}
	}

	return results, nil
}

// CommitInfo holds commit metadata
type CommitInfo struct {
	SHA        string
	TreeSHA    string
	ParentSHAs []string
	Author     objects.Identity
	Timestamp  time.Time
	Message    string
}

// getCommitInfo retrieves commit information without checkout
func (oi *OptimizedImporter) getCommitInfo(gitSHA string) (*CommitInfo, error) {
	// Get commit details using git cat-file
	cmd := exec.Command("git", "-C", oi.repo.root, "cat-file", "commit", gitSHA)
	output, err := cmd.Output()
	if err != nil {
		return nil, err
	}

	info := &CommitInfo{
		SHA:        gitSHA,
		ParentSHAs: []string{},
	}

	lines := strings.Split(string(output), "\n")
	messageStart := false
	var messageLines []string

	for _, line := range lines {
		if messageStart {
			messageLines = append(messageLines, line)
			continue
		}

		if line == "" {
			messageStart = true
			continue
		}

		parts := strings.SplitN(line, " ", 2)
		if len(parts) != 2 {
			continue
		}

		switch parts[0] {
		case "tree":
			info.TreeSHA = parts[1]
		case "parent":
			info.ParentSHAs = append(info.ParentSHAs, parts[1])
		case "author":
			info.Author = oi.parseAuthor(parts[1])
			info.Timestamp = oi.parseTimestamp(parts[1])
		}
	}

	info.Message = strings.TrimSpace(strings.Join(messageLines, "\n"))
	return info, nil
}

// parseAuthor extracts author info from Git author line
func (oi *OptimizedImporter) parseAuthor(authorLine string) objects.Identity {
	// Format: Name <email> timestamp timezone
	parts := strings.Split(authorLine, " <")
	name := parts[0]

	email := ""
	if len(parts) > 1 {
		emailParts := strings.Split(parts[1], ">")
		if len(emailParts) > 0 {
			email = emailParts[0]
		}
	}

	return objects.Identity{
		Name:  name,
		Email: email,
	}
}

// parseTimestamp extracts timestamp from Git author/committer line
func (oi *OptimizedImporter) parseTimestamp(line string) time.Time {
	// Find the timestamp (last two space-separated values)
	parts := strings.Fields(line)
	if len(parts) >= 2 {
		timestampStr := parts[len(parts)-2]
		if ts, err := strconv.ParseInt(timestampStr, 10, 64); err == nil {
			return time.Unix(ts, 0)
		}
	}
	return time.Now()
}

// createTreeFromGitTree creates an Ivaldi tree from a Git tree SHA
func (oi *OptimizedImporter) createTreeFromGitTree(gitTreeSHA string) (objects.Hash, error) {
	// Check cache first
	oi.cacheMutex.RLock()
	if cachedHash, exists := oi.treeCache[gitTreeSHA]; exists {
		oi.cacheMutex.RUnlock()
		return cachedHash, nil
	}
	oi.cacheMutex.RUnlock()

	// Get list of submodule paths to skip
	submodulePaths, err := workspace.GetSubmodulePaths(oi.repo.root)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("failed to get submodule paths for repository at %s: %w", oi.repo.root, err)
	}
	submoduleMap := make(map[string]bool)
	for _, path := range submodulePaths {
		submoduleMap[filepath.ToSlash(path)] = true
	}

	// Get tree contents using git ls-tree
	cmd := exec.Command("git", "-C", oi.repo.root, "ls-tree", "-r", gitTreeSHA)
	output, err := cmd.Output()
	if err != nil {
		return objects.Hash{}, err
	}

	var entries []objects.TreeEntry
	lines := strings.Split(string(output), "\n")

	for _, line := range lines {
		if line == "" {
			continue
		}

		// Format: mode type sha\tpath
		parts := strings.Fields(line)
		if len(parts) < 4 {
			continue
		}

		mode := parts[0]
		objType := parts[1]
		gitSHA := parts[2]

		// Path is after the tab
		tabIdx := strings.Index(line, "\t")
		if tabIdx == -1 {
			continue
		}
		path := line[tabIdx+1:]

		// Skip submodules (mode 160000) and files inside submodules
		if mode == "160000" {
			continue // This is a submodule entry itself
		}

		// Check if file is inside a submodule directory
		cleanPath := filepath.ToSlash(path)
		isInSubmodule := false
		for submodulePath := range submoduleMap {
			if cleanPath == submodulePath || strings.HasPrefix(cleanPath, submodulePath+"/") {
				isInSubmodule = true
				break
			}
		}
		if isInSubmodule {
			continue
		}

		// Get Ivaldi hash from cache
		var ivaldiHash objects.Hash
		if objType == "blob" {
			oi.cacheMutex.RLock()
			if cached, exists := oi.blobCache[gitSHA]; exists {
				ivaldiHash = cached
			}
			oi.cacheMutex.RUnlock()

			// If not cached, process it now
			if ivaldiHash.IsZero() {
				if err := oi.processBlobOptimized(gitSHA); err == nil {
					oi.cacheMutex.RLock()
					ivaldiHash = oi.blobCache[gitSHA]
					oi.cacheMutex.RUnlock()
				}
			}
		}

		if !ivaldiHash.IsZero() {
			modeInt, _ := strconv.ParseUint(mode, 8, 32)
			entry := objects.TreeEntry{
				Name: path,
				Type: objects.ObjectTypeBlob,
				Hash: ivaldiHash,
				Mode: uint32(modeInt),
			}
			entries = append(entries, entry)
		}
	}

	// Create and store tree
	tree := &objects.Tree{
		Entries: entries,
	}

	treeHash, err := oi.repo.storage.StoreObject(tree)
	if err != nil {
		return objects.Hash{}, err
	}

	// Cache the result
	oi.cacheMutex.Lock()
	oi.treeCache[gitTreeSHA] = treeHash
	oi.cacheMutex.Unlock()

	return treeHash, nil
}

// setFinalPosition sets the repository position to current HEAD
func (oi *OptimizedImporter) setFinalPosition() error {
	// Get current branch
	cmd := exec.Command("git", "-C", oi.repo.root, "branch", "--show-current")
	output, err := cmd.Output()
	if err != nil {
		return err
	}

	currentBranch := strings.TrimSpace(string(output))
	if currentBranch == "" {
		currentBranch = "main"
	}

	// Switch to current branch timeline
	if err := oi.repo.timeline.Switch(currentBranch); err != nil {
		fmt.Printf("Warning: failed to switch to timeline %s: %v\n", currentBranch, err)
	}

	// Get HEAD commit
	headCmd := exec.Command("git", "-C", oi.repo.root, "rev-parse", "HEAD")
	headOutput, err := headCmd.Output()
	if err != nil {
		return nil // Not critical
	}

	headSHA := strings.TrimSpace(string(headOutput))

	// Find corresponding Ivaldi hash
	oi.cacheMutex.RLock()
	defer oi.cacheMutex.RUnlock()

	// We need to search through timeline heads to find the right hash
	if head, err := oi.repo.timeline.GetHead(currentBranch); err == nil && !head.IsZero() {
		if err := oi.repo.position.SetPosition(head, currentBranch); err != nil {
			fmt.Printf("Warning: failed to set position to HEAD: %v\n", err)
		}
	}

	oi.progressChan <- fmt.Sprintf("Set position to branch: %s (Git SHA: %s)", currentBranch, headSHA[:8])
	return nil
}

// reportProgress reports import progress
func (oi *OptimizedImporter) reportProgress() {
	for msg := range oi.progressChan {
		fmt.Printf("│ %s\n", msg)
	}
}

// SaveBlobCache saves the blob cache to disk for future use
func (oi *OptimizedImporter) SaveBlobCache() error {
	cacheFile := filepath.Join(oi.repo.root, ".ivaldi", "blob_cache.bin")

	// Create cache directory
	if err := os.MkdirAll(filepath.Dir(cacheFile), 0755); err != nil {
		return err
	}

	// Serialize cache
	var buffer bytes.Buffer

	oi.cacheMutex.RLock()
	defer oi.cacheMutex.RUnlock()

	// Write number of entries
	buffer.Write([]byte(fmt.Sprintf("%d\n", len(oi.blobCache))))

	for gitSHA, ivaldiHash := range oi.blobCache {
		line := fmt.Sprintf("%s:%s\n", gitSHA, hex.EncodeToString(ivaldiHash[:]))
		buffer.Write([]byte(line))
	}

	return os.WriteFile(cacheFile, buffer.Bytes(), 0644)
}

// LoadBlobCache loads a previously saved blob cache
func (oi *OptimizedImporter) LoadBlobCache() error {
	cacheFile := filepath.Join(oi.repo.root, ".ivaldi", "blob_cache.bin")

	data, err := os.ReadFile(cacheFile)
	if err != nil {
		if os.IsNotExist(err) {
			return nil // No cache file, that's ok
		}
		return err
	}

	lines := strings.Split(string(data), "\n")
	if len(lines) < 1 {
		return nil
	}

	// Parse number of entries
	count, err := strconv.Atoi(lines[0])
	if err != nil {
		return err
	}

	oi.cacheMutex.Lock()
	defer oi.cacheMutex.Unlock()

	for i := 1; i <= count && i < len(lines); i++ {
		parts := strings.Split(lines[i], ":")
		if len(parts) != 2 {
			continue
		}

		gitSHA := parts[0]
		hashBytes, err := hex.DecodeString(parts[1])
		if err != nil || len(hashBytes) != 32 {
			continue
		}

		var ivaldiHash objects.Hash
		copy(ivaldiHash[:], hashBytes)
		oi.blobCache[gitSHA] = ivaldiHash
	}

	return nil
}
