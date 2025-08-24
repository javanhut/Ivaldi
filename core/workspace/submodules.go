package workspace

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// SubmoduleInfo represents a Git submodule
type SubmoduleInfo struct {
	Name   string
	Path   string
	URL    string
	Branch string
}

// ParseGitmodules parses the .gitmodules file and returns submodule information
func ParseGitmodules(root string) (map[string]*SubmoduleInfo, error) {
	gitmodulesPath := filepath.Join(root, ".gitmodules")
	
	// If .gitmodules doesn't exist, return empty map (no submodules)
	if _, err := os.Stat(gitmodulesPath); os.IsNotExist(err) {
		return make(map[string]*SubmoduleInfo), nil
	}
	
	file, err := os.Open(gitmodulesPath)
	if err != nil {
		return nil, fmt.Errorf("failed to open .gitmodules: %v", err)
	}
	defer file.Close()
	
	submodules := make(map[string]*SubmoduleInfo)
	var currentSubmodule *SubmoduleInfo
	
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		
		// Skip empty lines and comments
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		
		// Check for submodule section
		if strings.HasPrefix(line, "[submodule") {
			// Extract submodule name from [submodule "name"]
			start := strings.Index(line, "\"")
			end := strings.LastIndex(line, "\"")
			if start != -1 && end != -1 && start < end {
				name := line[start+1 : end]
				currentSubmodule = &SubmoduleInfo{Name: name}
			}
		} else if currentSubmodule != nil {
			// Parse key = value pairs
			parts := strings.SplitN(line, "=", 2)
			if len(parts) == 2 {
				key := strings.TrimSpace(parts[0])
				value := strings.TrimSpace(parts[1])
				
				switch key {
				case "path":
					currentSubmodule.Path = value
					// Use path as the key for the map
					submodules[value] = currentSubmodule
				case "url":
					currentSubmodule.URL = value
				case "branch":
					currentSubmodule.Branch = value
				}
			}
		}
	}
	
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("error reading .gitmodules: %v", err)
	}
	
	return submodules, nil
}

// IsSubmodule checks if a given path is a Git submodule
func IsSubmodule(root, path string) bool {
	// Check if this is a submodule by looking for it in .gitmodules
	submodules, err := ParseGitmodules(root)
	if err != nil {
		return false
	}
	
	relPath, err := filepath.Rel(root, path)
	if err != nil {
		return false
	}
	
	// Normalize path separators
	relPath = filepath.ToSlash(relPath)
	
	// Check if this path is a submodule
	_, isSubmodule := submodules[relPath]
	return isSubmodule
}

// GetSubmodulePaths returns all submodule paths in the repository
func GetSubmodulePaths(root string) ([]string, error) {
	submodules, err := ParseGitmodules(root)
	if err != nil {
		return nil, err
	}
	
	paths := make([]string, 0, len(submodules))
	for path := range submodules {
		paths = append(paths, path)
	}
	
	return paths, nil
}