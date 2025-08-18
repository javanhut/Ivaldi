package semantic

import (
	"fmt"
	"path/filepath"
	"regexp"
	"strings"

	"ivaldi/core/workspace"
)

// CommitMessageGenerator generates semantic commit messages from changes
type CommitMessageGenerator struct {
	patterns map[string][]CommitPattern
	keywords map[string]CommitType
}

// CommitType represents the type of change
type CommitType struct {
	Type        string
	Description string
	Emoji       string
}

// CommitPattern represents patterns for detecting commit types
type CommitPattern struct {
	Pattern     *regexp.Regexp
	Type        string
	Confidence  float64
	Description string
}

// ChangeAnalysis represents analysis of file changes
type ChangeAnalysis struct {
	File         string
	ChangeType   string
	LinesAdded   int
	LinesRemoved int
	Language     string
	Patterns     []string
	Confidence   float64
}

// GeneratedMessage represents a generated commit message
type GeneratedMessage struct {
	Primary     string
	Alternative []string
	Explanation string
	Type        CommitType
	Scope       string
	Confidence  float64
	Analysis    []ChangeAnalysis
}

// NewCommitMessageGenerator creates a new semantic commit message generator
func NewCommitMessageGenerator() *CommitMessageGenerator {
	gen := &CommitMessageGenerator{
		patterns: make(map[string][]CommitPattern),
		keywords: make(map[string]CommitType),
	}

	gen.initializePatterns()
	gen.initializeKeywords()

	return gen
}

// Generate creates a semantic commit message from workspace changes
func (gen *CommitMessageGenerator) Generate(ws *workspace.Workspace) (*GeneratedMessage, error) {
	if len(ws.AnvilFiles) == 0 {
		return nil, fmt.Errorf("no files staged for commit")
	}

	// Analyze each changed file
	var analyses []ChangeAnalysis
	totalConfidence := 0.0

	for filePath, fileState := range ws.AnvilFiles {
		analysis := gen.analyzeFile(filePath, fileState)
		analyses = append(analyses, analysis)
		totalConfidence += analysis.Confidence
	}

	// Determine overall change type
	commitType := gen.determineCommitType(analyses)
	scope := gen.determineScope(analyses)

	// Generate primary message
	primary := gen.generatePrimaryMessage(commitType, scope, analyses)

	// Generate alternatives
	alternatives := gen.generateAlternatives(commitType, scope, analyses)

	// Create explanation
	explanation := gen.generateExplanation(analyses)

	avgConfidence := totalConfidence / float64(len(analyses))

	return &GeneratedMessage{
		Primary:     primary,
		Alternative: alternatives,
		Explanation: explanation,
		Type:        commitType,
		Scope:       scope,
		Confidence:  avgConfidence,
		Analysis:    analyses,
	}, nil
}

// analyzeFile analyzes changes in a single file
func (gen *CommitMessageGenerator) analyzeFile(filePath string, fileState *workspace.FileState) ChangeAnalysis {
	ext := filepath.Ext(filePath)
	language := gen.detectLanguage(ext)
	
	// Simulate change analysis (in real implementation, would diff file contents)
	analysis := ChangeAnalysis{
		File:         filePath,
		ChangeType:   string(fileState.Status),
		LinesAdded:   10,  // Placeholder
		LinesRemoved: 2,   // Placeholder
		Language:     language,
		Patterns:     []string{},
		Confidence:   0.8,
	}

	// Analyze patterns based on file content and path
	patterns := gen.detectPatterns(filePath, fileState)
	analysis.Patterns = patterns

	// Adjust confidence based on patterns found
	if len(patterns) > 0 {
		analysis.Confidence = 0.9
	}

	return analysis
}

// detectLanguage determines programming language from file extension
func (gen *CommitMessageGenerator) detectLanguage(ext string) string {
	languages := map[string]string{
		".go":   "Go",
		".js":   "JavaScript",
		".ts":   "TypeScript", 
		".py":   "Python",
		".java": "Java",
		".rs":   "Rust",
		".cpp":  "C++",
		".c":    "C",
		".rb":   "Ruby",
		".php":  "PHP",
		".md":   "Markdown",
		".yml":  "YAML",
		".json": "JSON",
		".html": "HTML",
		".css":  "CSS",
	}

	if lang, exists := languages[ext]; exists {
		return lang
	}
	return "Text"
}

// detectPatterns identifies code patterns that suggest commit type
func (gen *CommitMessageGenerator) detectPatterns(filePath string, fileState *workspace.FileState) []string {
	var patterns []string

	// File path-based patterns
	if strings.Contains(filePath, "test") || strings.Contains(filePath, "_test.") {
		patterns = append(patterns, "test")
	}
	if strings.Contains(filePath, "doc") || strings.Contains(filePath, "README") {
		patterns = append(patterns, "docs")
	}
	if strings.Contains(filePath, "config") || strings.Contains(filePath, ".yml") || strings.Contains(filePath, ".json") {
		patterns = append(patterns, "config")
	}

	// Change type patterns
	switch fileState.Status {
	case workspace.StatusAdded:
		patterns = append(patterns, "feat")
	case workspace.StatusModified:
		patterns = append(patterns, "update")
	case workspace.StatusDeleted:
		patterns = append(patterns, "remove")
	}

	// TODO: In real implementation, analyze file content for:
	// - New function definitions
	// - Error handling additions
	// - Security changes
	// - Performance optimizations
	// - Bug fix patterns

	return patterns
}

// determineCommitType determines the primary commit type from analyses
func (gen *CommitMessageGenerator) determineCommitType(analyses []ChangeAnalysis) CommitType {
	patternCounts := make(map[string]int)

	// Count pattern occurrences
	for _, analysis := range analyses {
		for _, pattern := range analysis.Patterns {
			patternCounts[pattern]++
		}
	}

	// Determine most common pattern
	var maxPattern string
	maxCount := 0
	for pattern, count := range patternCounts {
		if count > maxCount {
			maxCount = count
			maxPattern = pattern
		}
	}

	// Map pattern to commit type
	commitTypes := map[string]CommitType{
		"feat": {
			Type:        "feat",
			Description: "New feature",
			Emoji:       "",
		},
		"fix": {
			Type:        "fix", 
			Description: "Bug fix",
			Emoji:       "",
		},
		"test": {
			Type:        "test",
			Description: "Add tests",
			Emoji:       "",
		},
		"docs": {
			Type:        "docs",
			Description: "Documentation",
			Emoji:       "",
		},
		"config": {
			Type:        "chore",
			Description: "Configuration",
			Emoji:       "",
		},
		"update": {
			Type:        "refactor",
			Description: "Code refactoring",
			Emoji:       "",
		},
		"remove": {
			Type:        "chore",
			Description: "Remove files",
			Emoji:       "",
		},
	}

	if commitType, exists := commitTypes[maxPattern]; exists {
		return commitType
	}

	// Default to feat for new changes
	return CommitType{
		Type:        "feat",
		Description: "Update",
		Emoji:       "",
	}
}

// determineScope determines the scope from file paths
func (gen *CommitMessageGenerator) determineScope(analyses []ChangeAnalysis) string {
	scopeCounts := make(map[string]int)

	for _, analysis := range analyses {
		// Extract scope from file path
		parts := strings.Split(analysis.File, "/")
		if len(parts) > 1 {
			scope := parts[0]
			// Common scope mappings
			switch scope {
			case "src", "lib":
				if len(parts) > 2 {
					scope = parts[1] // Use subdirectory as scope
				}
			case "cmd":
				scope = "cli"
			case "ui":
				scope = "interface"
			case "core":
				if len(parts) > 2 {
					scope = parts[1]
				}
			case "storage":
				scope = "storage"
			case "docs":
				scope = "docs"
			case "tests":
				scope = "test"
			}
			scopeCounts[scope]++
		}
	}

	// Find most common scope
	var maxScope string
	maxCount := 0
	for scope, count := range scopeCounts {
		if count > maxCount {
			maxCount = count
			maxScope = scope
		}
	}

	return maxScope
}

// generatePrimaryMessage creates the main commit message
func (gen *CommitMessageGenerator) generatePrimaryMessage(commitType CommitType, scope string, analyses []ChangeAnalysis) string {
	var message strings.Builder

	// Add type
	message.WriteString(commitType.Type)

	// Add scope if available
	if scope != "" {
		message.WriteString("(" + scope + ")")
	}

	message.WriteString(": ")

	// Generate description based on changes
	description := gen.generateDescription(commitType, analyses)
	message.WriteString(description)

	return message.String()
}

// generateDescription creates a description of the changes
func (gen *CommitMessageGenerator) generateDescription(commitType CommitType, analyses []ChangeAnalysis) string {
	if len(analyses) == 1 {
		// Single file change
		analysis := analyses[0]
		fileName := filepath.Base(analysis.File)
		
		switch commitType.Type {
		case "feat":
			return fmt.Sprintf("add %s functionality", extractFeatureName(fileName))
		case "fix":
			return fmt.Sprintf("resolve issues in %s", fileName)
		case "docs":
			return fmt.Sprintf("update %s documentation", fileName)
		case "test":
			return fmt.Sprintf("add tests for %s", extractFeatureName(fileName))
		default:
			return fmt.Sprintf("update %s", fileName)
		}
	}

	// Multiple file changes
	switch commitType.Type {
	case "feat":
		return "implement new functionality"
	case "fix":
		return "resolve multiple issues"
	case "docs":
		return "update project documentation"
	case "test":
		return "expand test coverage"
	default:
		return fmt.Sprintf("update %d files", len(analyses))
	}
}

// generateAlternatives creates alternative commit message suggestions
func (gen *CommitMessageGenerator) generateAlternatives(commitType CommitType, scope string, analyses []ChangeAnalysis) []string {
	var alternatives []string

	// Generate variations
	if scope != "" {
		alternatives = append(alternatives, 
			fmt.Sprintf("%s: %s in %s module", commitType.Type, commitType.Description, scope))
	}

	// Add conventional commits format
	alternatives = append(alternatives,
		fmt.Sprintf("%s: %s", commitType.Type, gen.generateDescription(commitType, analyses)))

	// Add descriptive format
	if len(analyses) == 1 {
		fileName := filepath.Base(analyses[0].File)
		alternatives = append(alternatives,
			fmt.Sprintf("Update %s with %s", fileName, strings.ToLower(commitType.Description)))
	}

	return alternatives
}

// generateExplanation creates an explanation of the generated message
func (gen *CommitMessageGenerator) generateExplanation(analyses []ChangeAnalysis) string {
	var explanation strings.Builder

	explanation.WriteString("Analysis of changes:\n")
	
	for _, analysis := range analyses {
		explanation.WriteString(fmt.Sprintf("• %s: %s (%s, +%d/-%d lines)\n",
			analysis.File, analysis.ChangeType, analysis.Language, 
			analysis.LinesAdded, analysis.LinesRemoved))
		
		if len(analysis.Patterns) > 0 {
			explanation.WriteString(fmt.Sprintf("  Patterns: %s\n", 
				strings.Join(analysis.Patterns, ", ")))
		}
	}

	return explanation.String()
}

// extractFeatureName extracts a feature name from filename
func extractFeatureName(fileName string) string {
	// Remove extension
	name := strings.TrimSuffix(fileName, filepath.Ext(fileName))
	
	// Convert camelCase/snake_case to readable form
	name = strings.ReplaceAll(name, "_", " ")
	name = regexp.MustCompile(`([a-z])([A-Z])`).ReplaceAllString(name, `$1 $2`)
	
	return strings.ToLower(name)
}

// initializePatterns sets up regex patterns for commit type detection
func (gen *CommitMessageGenerator) initializePatterns() {
	// TODO: Add sophisticated patterns for different commit types
	// This would include patterns for:
	// - Function additions/removals
	// - Error handling changes
	// - Performance improvements
	// - Security fixes
	// - API changes
	// - Database migrations
	// - Configuration changes
}

// initializeKeywords sets up keyword mappings
func (gen *CommitMessageGenerator) initializeKeywords() {
	gen.keywords = map[string]CommitType{
		"add":      {Type: "feat", Description: "New feature", Emoji: ""},
		"create":   {Type: "feat", Description: "New feature", Emoji: ""},
		"implement": {Type: "feat", Description: "New feature", Emoji: ""},
		"fix":      {Type: "fix", Description: "Bug fix", Emoji: ""},
		"resolve":  {Type: "fix", Description: "Bug fix", Emoji: ""},
		"update":   {Type: "chore", Description: "Update", Emoji: ""},
		"refactor": {Type: "refactor", Description: "Refactor", Emoji: ""},
		"test":     {Type: "test", Description: "Tests", Emoji: ""},
		"doc":      {Type: "docs", Description: "Documentation", Emoji: ""},
	}
}

// Enhanced semantic analysis (for future implementation)

// AnalyzeCodeChanges performs deep code analysis
func (gen *CommitMessageGenerator) AnalyzeCodeChanges(oldContent, newContent string) *CodeAnalysis {
	// This would implement AST-based analysis to detect:
	// - New function/class definitions
	// - Modified algorithms
	// - Error handling additions
	// - Performance optimizations
	// - Security improvements
	// - Breaking changes
	
	return &CodeAnalysis{
		FunctionsAdded:   []string{},
		FunctionsRemoved: []string{},
		FunctionsModified: []string{},
		Complexity:       0,
		BreakingChanges:  false,
		SecurityChanges:  false,
	}
}

// CodeAnalysis represents detailed code change analysis
type CodeAnalysis struct {
	FunctionsAdded    []string
	FunctionsRemoved  []string
	FunctionsModified []string
	Complexity        int
	BreakingChanges   bool
	SecurityChanges   bool
}

// Interactive mode for commit message refinement

// RefineMessage provides interactive refinement of generated messages
func (gen *CommitMessageGenerator) RefineMessage(original *GeneratedMessage, userInput string) *GeneratedMessage {
	// This would implement interactive refinement based on user feedback
	refined := *original
	
	// Parse user input for refinements
	if strings.Contains(userInput, "more specific") {
		// Make message more specific
		refined.Primary = gen.makeMoreSpecific(original.Primary, original.Analysis)
	}
	
	if strings.Contains(userInput, "shorter") {
		// Make message shorter
		refined.Primary = gen.makeShorten(original.Primary)
	}
	
	return &refined
}

// makeMoreSpecific creates a more specific commit message
func (gen *CommitMessageGenerator) makeMoreSpecific(message string, analyses []ChangeAnalysis) string {
	// Add more specific details based on analysis
	return message // Placeholder
}

// makeShorten creates a shorter commit message
func (gen *CommitMessageGenerator) makeShorten(message string) string {
	// Simplify the message
	return message // Placeholder
}