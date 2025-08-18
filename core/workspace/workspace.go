package workspace

import (
	"bufio"
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"

	"ivaldi/core/objects"
)

type FileStatus int

const (
	StatusUnmodified FileStatus = iota
	StatusModified
	StatusAdded
	StatusDeleted
	StatusGathered
)

type FileState struct {
	Path         string
	Status       FileStatus
	Hash         objects.Hash
	Size         int64
	ModTime      time.Time
	OnAnvil      bool
	WorkingHash  objects.Hash
}

type Workspace struct {
	Root         string
	Files        map[string]*FileState
	Timeline     string
	Position     objects.Hash
	AnvilFiles   map[string]*FileState
	IgnorePattern []string
}

func New(root string) *Workspace {
	ws := &Workspace{
		Root:         root,
		Files:        make(map[string]*FileState),
		AnvilFiles:   make(map[string]*FileState),
		IgnorePattern: []string{},
	}
	ws.loadIgnorePatterns()
	return ws
}

func (w *Workspace) Scan() error {
	// Preserve anvil files before scanning
	anvilBackup := make(map[string]*FileState)
	for k, v := range w.AnvilFiles {
		anvilBackup[k] = v
	}
	
	// Track which files we've seen during scan
	seenFiles := make(map[string]bool)
	
	err := filepath.WalkDir(w.Root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}

		if d.IsDir() {
			if d.Name() == ".ivaldi" || d.Name() == ".git" || d.Name() == "build" {
				return filepath.SkipDir
			}
			return nil
		}

		relPath, err := filepath.Rel(w.Root, path)
		if err != nil {
			return err
		}
		
		// Skip ignored files
		if w.shouldIgnore(relPath) {
			return nil
		}

		info, err := d.Info()
		if err != nil {
			return err
		}

		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}

		hash := objects.NewHash(data)
		
		// Check if file was on anvil
		onAnvil := false
		status := StatusUnmodified
		
		// First determine the actual status based on changes
		if existing, exists := w.Files[relPath]; exists {
			if existing.Hash != hash {
				status = StatusModified
			} else {
				status = StatusUnmodified
			}
		} else {
			status = StatusAdded
		}
		
		// Then restore anvil state if it was previously gathered
		if anvilFile, wasOnAnvil := anvilBackup[relPath]; wasOnAnvil {
			onAnvil = true
			// Preserve the actual change status, don't override with StatusGathered
			w.AnvilFiles[relPath] = anvilFile
		}
		
		fileState := &FileState{
			Path:        relPath,
			Status:      status,
			Hash:        hash,
			Size:        info.Size(),
			ModTime:     info.ModTime(),
			WorkingHash: hash,
			OnAnvil:     onAnvil,
		}

		w.Files[relPath] = fileState
		seenFiles[relPath] = true
		return nil
	})
	
	if err != nil {
		return err
	}
	
	// Check for deleted files - files that were tracked but no longer exist
	for path, fileState := range w.Files {
		if !seenFiles[path] && fileState.Status != StatusDeleted {
			// File was tracked but no longer exists - mark as deleted
			fileState.Status = StatusDeleted
			// Keep it in Files map but marked as deleted
			w.Files[path] = fileState
			
			// If it was on the anvil, update there too
			if fileState.OnAnvil {
				w.AnvilFiles[path] = fileState
			}
		}
	}
	
	return nil
}

func (w *Workspace) Gather(patterns []string) error {
	// Special case for "all" - gather only changed files
	if len(patterns) == 1 && patterns[0] == "." {
		return w.GatherChanged()
	}
	
	for _, pattern := range patterns {
		absPattern := filepath.Join(w.Root, pattern)
		
		// Check if it's a file or directory
		info, err := os.Stat(absPattern)
		if err != nil {
			// Try glob pattern
			matches, err := filepath.Glob(absPattern)
			if err != nil {
				return err
			}
			
			for _, match := range matches {
				if err := w.gatherPath(match); err != nil {
					return err
				}
			}
		} else if info.IsDir() {
			// Recursively gather all files in directory
			err := filepath.WalkDir(absPattern, func(path string, d fs.DirEntry, err error) error {
				if err != nil {
					return err
				}
				
				if d.IsDir() {
					if d.Name() == ".ivaldi" || d.Name() == ".git" {
						return filepath.SkipDir
					}
					return nil
				}
				
				return w.gatherPath(path)
			})
			if err != nil {
				return err
			}
		} else {
			// Single file
			if err := w.gatherPath(absPattern); err != nil {
				return err
			}
		}
	}
	return nil
}

// GatherChanged gathers only files that have been modified, added, or deleted
func (w *Workspace) GatherChanged() error {
	for path, fileState := range w.Files {
		if fileState.Status == StatusModified || fileState.Status == StatusAdded || fileState.Status == StatusDeleted {
			fileState.OnAnvil = true
			w.AnvilFiles[path] = fileState
		}
	}
	return nil
}

func (w *Workspace) gatherPath(path string) error {
	relPath, err := filepath.Rel(w.Root, path)
	if err != nil {
		return err
	}
	
	// Skip ignored files
	if w.shouldIgnore(relPath) {
		return nil
	}
	
	// First, ensure we've scanned for changes
	if fileState, exists := w.Files[relPath]; exists {
		// Only gather files that have actually changed
		if fileState.Status == StatusModified || fileState.Status == StatusAdded || fileState.Status == StatusDeleted {
			fileState.OnAnvil = true
			// Keep the original status (Modified/Added/Deleted) instead of changing to Gathered
			w.AnvilFiles[relPath] = fileState
		}
		// If file is unmodified, don't gather it
	} else {
		// File not in workspace yet - it's new, scan it first
		info, err := os.Stat(path)
		if err != nil {
			return nil // File might have been deleted, skip
		}
		
		if !info.IsDir() {
			data, err := os.ReadFile(path)
			if err != nil {
				return nil
			}
			
			hash := objects.NewHash(data)
			fileState := &FileState{
				Path:        relPath,
				Status:      StatusAdded, // New file
				Hash:        hash,
				Size:        info.Size(),
				ModTime:     info.ModTime(),
				WorkingHash: hash,
				OnAnvil:     true,
			}
			
			w.Files[relPath] = fileState
			w.AnvilFiles[relPath] = fileState
		}
	}
	return nil
}

func (w *Workspace) SaveState(timeline string) error {
	stateDir := filepath.Join(w.Root, ".ivaldi", "workspace", timeline)
	if err := os.MkdirAll(stateDir, 0755); err != nil {
		return err
	}

	statePath := filepath.Join(stateDir, "state.json")
	data, err := json.Marshal(w)
	if err != nil {
		return err
	}

	return os.WriteFile(statePath, data, 0644)
}

func (w *Workspace) LoadState(timeline string) error {
	statePath := filepath.Join(w.Root, ".ivaldi", "workspace", timeline, "state.json")
	data, err := os.ReadFile(statePath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}

	return json.Unmarshal(data, w)
}

func (w *Workspace) HasUncommittedChanges() bool {
	for _, file := range w.Files {
		if file.Status == StatusModified || file.Status == StatusAdded || file.Status == StatusDeleted {
			return true
		}
	}
	return len(w.AnvilFiles) > 0
}

func (w *Workspace) loadIgnorePatterns() {
	ignorePath := filepath.Join(w.Root, ".ivaldiignore")
	file, err := os.Open(ignorePath)
	if err != nil {
		// Default ignore patterns
		w.IgnorePattern = []string{
			".ivaldi/*",
			".git/*",
			"build/*",
			"*.tmp",
			"*.log",
			".DS_Store",
			"Thumbs.db",
		}
		return
	}
	defer file.Close()

	var patterns []string
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line != "" && !strings.HasPrefix(line, "#") {
			patterns = append(patterns, line)
		}
	}
	
	// Add default patterns that should always be ignored
	patterns = append(patterns, ".ivaldi/*", ".git/*")
	w.IgnorePattern = patterns
}

func (w *Workspace) shouldIgnore(path string) bool {
	// Clean path and convert to forward slashes for consistent matching
	cleanPath := filepath.ToSlash(path)
	
	for _, pattern := range w.IgnorePattern {
		// Clean pattern and convert to forward slashes
		cleanPattern := filepath.ToSlash(pattern)
		
		// Handle directory patterns (ending with /)
		if strings.HasSuffix(cleanPattern, "/") {
			dirPattern := strings.TrimSuffix(cleanPattern, "/")
			// Check if path is within this directory
			if cleanPath == dirPattern || strings.HasPrefix(cleanPath, dirPattern+"/") {
				return true
			}
			continue
		}
		
		// Handle wildcard patterns
		if strings.Contains(cleanPattern, "*") {
			matched, _ := filepath.Match(cleanPattern, cleanPath)
			if matched {
				return true
			}
			// Also check if any parent path component matches
			pathParts := strings.Split(cleanPath, "/")
			for i := range pathParts {
				partialPath := strings.Join(pathParts[:i+1], "/")
				matched, _ := filepath.Match(cleanPattern, partialPath)
				if matched {
					return true
				}
			}
			continue
		}
		
		// Exact match
		if cleanPath == cleanPattern {
			return true
		}
		
		// Check if it's a file in a directory that should be ignored
		if strings.Contains(cleanPattern, "/") {
			if strings.HasPrefix(cleanPath, cleanPattern+"/") || cleanPath == cleanPattern {
				return true
			}
		}
	}
	return false
}

// RefreshIgnorePatterns reloads the ignore patterns from .ivaldiignore
func (w *Workspace) RefreshIgnorePatterns() {
	w.loadIgnorePatterns()
}

// ShouldIgnore returns true if the given path should be ignored
func (w *Workspace) ShouldIgnore(path string) bool {
	return w.shouldIgnore(path)
}

func (w *Workspace) Discard(patterns []string) (int, error) {
	count := 0
	
	for _, pattern := range patterns {
		absPattern := filepath.Join(w.Root, pattern)
		
		// Check if it's a file or directory
		info, err := os.Stat(absPattern)
		if err != nil {
			// Try glob pattern
			matches, err := filepath.Glob(absPattern)
			if err != nil {
				continue
			}
			
			for _, match := range matches {
				if c := w.discardPath(match); c > 0 {
					count += c
				}
			}
		} else if info.IsDir() {
			// Recursively discard all files in directory
			err := filepath.WalkDir(absPattern, func(path string, d fs.DirEntry, err error) error {
				if err != nil {
					return err
				}
				
				if !d.IsDir() {
					if c := w.discardPath(path); c > 0 {
						count += c
					}
				}
				return nil
			})
			if err != nil {
				continue
			}
		} else {
			// Single file
			if c := w.discardPath(absPattern); c > 0 {
				count += c
			}
		}
	}
	
	return count, nil
}

func (w *Workspace) discardPath(path string) int {
	relPath, err := filepath.Rel(w.Root, path)
	if err != nil {
		return 0
	}
	
	if _, exists := w.AnvilFiles[relPath]; exists {
		delete(w.AnvilFiles, relPath)
		
		// Update file state
		if fileState, exists := w.Files[relPath]; exists {
			fileState.OnAnvil = false
			if fileState.Status == StatusGathered {
				fileState.Status = StatusModified // or StatusAdded based on original state
			}
		}
		
		return 1
	}
	
	return 0
}