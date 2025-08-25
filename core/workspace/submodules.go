package workspace

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// SubmoduleInfo represents an Ivaldi submodule
type SubmoduleInfo struct {
	Name   string
	Path   string
	URL    string
	Branch string
	Type   string // "git" or "ivaldi"
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

// IsSubmodule checks if a given path is a submodule
func IsSubmodule(root, path string) bool {
	// Check if this is a submodule by looking in .ivaldimodules or .gitmodules
	submodules, err := ParseSubmodules(root)
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
	submodules, err := ParseSubmodules(root)
	if err != nil {
		return nil, err
	}

	paths := make([]string, 0, len(submodules))
	for path := range submodules {
		paths = append(paths, path)
	}

	return paths, nil
}

// ParseIvaldimodules parses the .ivaldimodules file and returns submodule information
func ParseIvaldimodules(root string) (map[string]*SubmoduleInfo, error) {
	ivaldimodulesPath := filepath.Join(root, ".ivaldimodules")

	// If .ivaldimodules doesn't exist, return empty map (no submodules)
	if _, err := os.Stat(ivaldimodulesPath); os.IsNotExist(err) {
		return make(map[string]*SubmoduleInfo), nil
	}

	file, err := os.Open(ivaldimodulesPath)
	if err != nil {
		return nil, fmt.Errorf("failed to open .ivaldimodules: %v", err)
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
				currentSubmodule = &SubmoduleInfo{Name: name, Type: "ivaldi"}
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
				case "type":
					currentSubmodule.Type = value
				}
			}
		}
	}

	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("error reading .ivaldimodules: %v", err)
	}

	return submodules, nil
}

// ParseSubmodules parses both .ivaldimodules and .gitmodules files, with .ivaldimodules taking precedence
func ParseSubmodules(root string) (map[string]*SubmoduleInfo, error) {
	// First try .ivaldimodules
	ivaldiSubmodules, err := ParseIvaldimodules(root)
	if err == nil && len(ivaldiSubmodules) > 0 {
		return ivaldiSubmodules, nil
	}

	// Fallback to .gitmodules for backward compatibility
	gitSubmodules, err := ParseGitmodules(root)
	if err != nil {
		return nil, err
	}

	// Mark git submodules with type
	for _, submodule := range gitSubmodules {
		submodule.Type = "git"
	}

	return gitSubmodules, nil
}

// CreateIvaldimodulesFromGitmodules migrates .gitmodules to .ivaldimodules
func CreateIvaldimodulesFromGitmodules(root string) error {
	gitmodulesPath := filepath.Join(root, ".gitmodules")
	ivaldimodulesPath := filepath.Join(root, ".ivaldimodules")

	// Check if .gitmodules exists
	if _, err := os.Stat(gitmodulesPath); os.IsNotExist(err) {
		return nil // No .gitmodules to migrate
	}

	// Check if .ivaldimodules already exists
	if _, err := os.Stat(ivaldimodulesPath); err == nil {
		return nil // .ivaldimodules already exists, don't overwrite
	}

	// Parse .gitmodules
	submodules, err := ParseGitmodules(root)
	if err != nil {
		return fmt.Errorf("failed to parse .gitmodules: %v", err)
	}

	// Create .ivaldimodules file
	file, err := os.Create(ivaldimodulesPath)
	if err != nil {
		return fmt.Errorf("failed to create .ivaldimodules: %v", err)
	}
	defer file.Close()

	// Write header
	fmt.Fprintln(file, "# Ivaldi submodules configuration")
	fmt.Fprintln(file, "# Migrated from .gitmodules")
	fmt.Fprintln(file, "")

	// Write each submodule
	for _, submodule := range submodules {
		fmt.Fprintf(file, "[submodule \"%s\"]\n", submodule.Name)
		fmt.Fprintf(file, "\tpath = %s\n", submodule.Path)
		fmt.Fprintf(file, "\turl = %s\n", submodule.URL)
		if submodule.Branch != "" {
			fmt.Fprintf(file, "\tbranch = %s\n", submodule.Branch)
		}
		fmt.Fprintf(file, "\ttype = git\n")
		fmt.Fprintln(file, "")
	}

	fmt.Printf("Created .ivaldimodules from .gitmodules with %d submodules\n", len(submodules))
	return nil
}
