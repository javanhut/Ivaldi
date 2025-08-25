package references

import (
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"ivaldi/core/objects"
)

// Reference types for natural language resolution
type Reference struct {
	Type   ReferenceType `json:"type"`
	Value  string        `json:"value"`
	Hash   objects.Hash  `json:"hash"`
	Author string        `json:"author,omitempty"`
	Time   time.Time     `json:"time,omitempty"`
}

type ReferenceType int

const (
	RefMemorableName ReferenceType = iota
	RefIteration
	RefCustomAlias
	RefTemporal
	RefAuthor
	RefContent
	RefSHA
)

// Index interface for querying seals
type Index interface {
	FindSealsByAuthor(author string) ([]objects.Hash, error)
	FindSealsByTimeRange(start, end time.Time) ([]objects.Hash, error)
	FindSealsContaining(searchTerm string) ([]objects.Hash, error)
	FindSealByHashPrefix(prefix string) (*objects.Hash, error)
	GetSealByTimeline(timeline string, iteration int) (*objects.Hash, error)
	FindSealByIteration(iteration int) (*objects.Hash, error)
}

type ReferenceManager struct {
	root       string
	references map[string]*Reference
	aliases    map[string]objects.Hash
	iterations map[string]int // timeline -> iteration count
	index      Index
	mu         sync.RWMutex // Protects references, aliases, and iterations
}

func NewReferenceManager(root string) *ReferenceManager {
	return &ReferenceManager{
		root:       root,
		references: make(map[string]*Reference),
		aliases:    make(map[string]objects.Hash),
		iterations: make(map[string]int),
	}
}

func (rm *ReferenceManager) SetIndex(index Index) {
	rm.index = index
}

// Generate memorable name using the spec's adjective-noun-number format
func (rm *ReferenceManager) GenerateMemorableName() string {
	adjectives := []string{
		"bright", "swift", "bold", "calm", "wise", "strong", "gentle", "fierce",
		"noble", "quick", "sharp", "clear", "deep", "warm", "cool", "fresh",
		"steady", "keen", "proud", "pure", "dark", "light", "silver", "golden",
		"ancient", "modern", "quiet", "loud", "smooth", "rough", "soft", "hard",
	}

	nouns := []string{
		"river", "mountain", "forest", "ocean", "star", "moon", "sun", "wind",
		"flame", "stone", "tree", "bird", "wolf", "eagle", "bear", "lion",
		"stream", "valley", "peak", "meadow", "lake", "shore", "path", "bridge",
		"forge", "anvil", "hammer", "shield", "sword", "crown", "gem", "crystal",
	}

	adjective := adjectives[rand.Intn(len(adjectives))]
	noun := nouns[rand.Intn(len(nouns))]
	number := rand.Intn(999) + 1

	name := fmt.Sprintf("%s-%s-%d", adjective, noun, number)

	rm.mu.RLock()
	defer rm.mu.RUnlock()
	// Ensure uniqueness
	for rm.references[name] != nil {
		number = rand.Intn(999) + 1
		name = fmt.Sprintf("%s-%s-%d", adjective, noun, number)
	}

	return name
}

// Resolve natural language references to commit hashes
func (rm *ReferenceManager) Resolve(ref string, currentTimeline string) (objects.Hash, error) {
	ref = strings.TrimSpace(ref)

	rm.mu.RLock()
	defer rm.mu.RUnlock()

	// Direct memorable name
	if rm.references[ref] != nil {
		return rm.references[ref].Hash, nil
	}

	// Custom alias
	if hash, exists := rm.aliases[ref]; exists {
		return hash, nil
	}

	// Iteration number (#42, #-5, main#15)
	if hash, err := rm.resolveIteration(ref, currentTimeline); err == nil {
		return hash, nil
	}

	// Temporal references
	if hash, err := rm.resolveTemporal(ref); err == nil {
		return hash, nil
	}

	// Author references
	if hash, err := rm.resolveAuthor(ref); err == nil {
		return hash, nil
	}

	// Content references
	if hash, err := rm.resolveContent(ref); err == nil {
		return hash, nil
	}

	// SHA prefix (hidden from normal use)
	if hash, err := rm.resolveSHA(ref); err == nil {
		return hash, nil
	}

	return objects.Hash{}, fmt.Errorf("reference '%s' not found", ref)
}

func (rm *ReferenceManager) resolveIteration(ref, currentTimeline string) (objects.Hash, error) {
	// Match patterns like #42, #-5, main#15, feature#7
	patterns := []string{
		`^#(-?\d+)$`,    // #42, #-5
		`^(\w+)#(\d+)$`, // main#15, feature#7
	}

	for _, pattern := range patterns {
		re := regexp.MustCompile(pattern)
		matches := re.FindStringSubmatch(ref)

		if len(matches) > 0 {
			var timeline string
			var iteration int
			var err error

			if len(matches) == 2 {
				// #42 format - use current timeline
				timeline = currentTimeline
				iteration, err = strconv.Atoi(matches[1])
			} else if len(matches) == 3 {
				// main#15 format
				timeline = matches[1]
				iteration, err = strconv.Atoi(matches[2])
			}

			if err != nil {
				continue
			}

			return rm.getHashByIteration(timeline, iteration)
		}
	}

	return objects.Hash{}, fmt.Errorf("invalid iteration format")
}

func (rm *ReferenceManager) resolveTemporal(ref string) (objects.Hash, error) {
	// Natural language time references
	now := time.Now()

	// Simple temporal patterns
	temporalPatterns := map[string]time.Duration{
		"yesterday":    -24 * time.Hour,
		"last hour":    -time.Hour,
		"this morning": -4 * time.Hour, // Approximate
		"last week":    -7 * 24 * time.Hour,
		"last month":   -30 * 24 * time.Hour,
	}

	if duration, exists := temporalPatterns[strings.ToLower(ref)]; exists {
		targetTime := now.Add(duration)
		return rm.findClosestCommitByTime(targetTime)
	}

	// Pattern: "2 hours ago"
	agoPattern := regexp.MustCompile(`^(\d+)\s+(hours?|minutes?|days?)\s+ago$`)
	matches := agoPattern.FindStringSubmatch(strings.ToLower(ref))
	if len(matches) == 3 {
		amount, _ := strconv.Atoi(matches[1])
		unit := matches[2]

		var duration time.Duration
		switch {
		case strings.HasPrefix(unit, "minute"):
			duration = -time.Duration(amount) * time.Minute
		case strings.HasPrefix(unit, "hour"):
			duration = -time.Duration(amount) * time.Hour
		case strings.HasPrefix(unit, "day"):
			duration = -time.Duration(amount) * 24 * time.Hour
		}

		targetTime := now.Add(duration)
		return rm.findClosestCommitByTime(targetTime)
	}

	return objects.Hash{}, fmt.Errorf("temporal reference not understood")
}

func (rm *ReferenceManager) resolveAuthor(ref string) (objects.Hash, error) {
	// Patterns like "Sarah's last commit", "my morning changes"
	authorPatterns := []string{
		`^(\w+)'s last commit$`,
		`^(\w+)'s last change$`,
		`^my last commit$`,
		`^last commit by (\w+)$`,
	}

	for _, pattern := range authorPatterns {
		re := regexp.MustCompile(`(?i)` + pattern)
		matches := re.FindStringSubmatch(ref)

		if len(matches) > 0 {
			var author string
			if matches[1] == "my" || len(matches) == 1 {
				author = rm.getCurrentUser()
			} else {
				author = matches[1]
			}

			return rm.findLastCommitByAuthor(author)
		}
	}

	return objects.Hash{}, fmt.Errorf("author reference not understood")
}

func (rm *ReferenceManager) resolveContent(ref string) (objects.Hash, error) {
	// Patterns like "where I added auth", "the commit about users"
	contentPatterns := []string{
		`^where .*? added (.+)$`,
		`^the commit about (.+)$`,
		`^when (.+) was added$`,
		`^.*? commit .*? (.+)$`,
	}

	for _, pattern := range contentPatterns {
		re := regexp.MustCompile(`(?i)` + pattern)
		matches := re.FindStringSubmatch(ref)

		if len(matches) > 1 {
			searchTerm := matches[1]
			return rm.searchCommitsByContent(searchTerm)
		}
	}

	return objects.Hash{}, fmt.Errorf("content reference not understood")
}

func (rm *ReferenceManager) resolveSHA(ref string) (objects.Hash, error) {
	// Only for advanced debugging - hidden from normal use
	if len(ref) >= 7 && regexp.MustCompile(`^[a-f0-9]+$`).MatchString(ref) {
		return rm.findBySHAPrefix(ref)
	}

	return objects.Hash{}, fmt.Errorf("not a valid SHA")
}

// Helper functions (implementations would connect to storage layer)
func (rm *ReferenceManager) getHashByIteration(timeline string, iteration int) (objects.Hash, error) {
	if rm.index == nil {
		return objects.Hash{}, fmt.Errorf("index not available")
	}

	// Use direct iteration lookup for now
	hash, err := rm.index.FindSealByIteration(iteration)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("iteration #%d not found: %v", iteration, err)
	}

	if hash == nil {
		return objects.Hash{}, fmt.Errorf("iteration #%d not found", iteration)
	}

	return *hash, nil
}

func (rm *ReferenceManager) findClosestCommitByTime(targetTime time.Time) (objects.Hash, error) {
	if rm.index == nil {
		return objects.Hash{}, fmt.Errorf("index not available")
	}

	// Create a time range around the target (±1 hour)
	start := targetTime.Add(-time.Hour)
	end := targetTime.Add(time.Hour)

	hashes, err := rm.index.FindSealsByTimeRange(start, end)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("time lookup failed: %v", err)
	}

	if len(hashes) == 0 {
		// Expand search range (±1 day)
		start = targetTime.Add(-24 * time.Hour)
		end = targetTime.Add(24 * time.Hour)

		hashes, err = rm.index.FindSealsByTimeRange(start, end)
		if err != nil {
			return objects.Hash{}, fmt.Errorf("time lookup failed: %v", err)
		}

		if len(hashes) == 0 {
			return objects.Hash{}, fmt.Errorf("no commits found near target time")
		}
	}

	// Return the first (most recent) match
	return hashes[0], nil
}

func (rm *ReferenceManager) findLastCommitByAuthor(author string) (objects.Hash, error) {
	if rm.index == nil {
		return objects.Hash{}, fmt.Errorf("index not available")
	}

	hashes, err := rm.index.FindSealsByAuthor(author)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("author lookup failed: %v", err)
	}

	if len(hashes) == 0 {
		return objects.Hash{}, fmt.Errorf("no commits found by author '%s'", author)
	}

	// Return the first (most recent) commit by this author
	return hashes[0], nil
}

func (rm *ReferenceManager) searchCommitsByContent(searchTerm string) (objects.Hash, error) {
	if rm.index == nil {
		return objects.Hash{}, fmt.Errorf("index not available")
	}

	hashes, err := rm.index.FindSealsContaining(searchTerm)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("content search failed: %v", err)
	}

	if len(hashes) == 0 {
		return objects.Hash{}, fmt.Errorf("no commits found containing '%s'", searchTerm)
	}

	// Return the first (most recent) match
	return hashes[0], nil
}

func (rm *ReferenceManager) findBySHAPrefix(prefix string) (objects.Hash, error) {
	if rm.index == nil {
		return objects.Hash{}, fmt.Errorf("index not available")
	}

	hash, err := rm.index.FindSealByHashPrefix(prefix)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("SHA prefix lookup failed: %v", err)
	}

	if hash == nil {
		return objects.Hash{}, fmt.Errorf("no commit found with SHA prefix '%s'", prefix)
	}

	return *hash, nil
}

func (rm *ReferenceManager) getCurrentUser() string {
	// Try to get from git config first
	return "Developer" // Default user for now
}

// Register a new memorable name
func (rm *ReferenceManager) RegisterMemorableName(name string, hash objects.Hash, author string) error {
	ref := &Reference{
		Type:   RefMemorableName,
		Value:  name,
		Hash:   hash,
		Author: author,
		Time:   time.Now(),
	}

	rm.mu.Lock()
	defer rm.mu.Unlock()
	rm.references[name] = ref
	return rm.saveUnsafe()
}

// Register a custom alias
func (rm *ReferenceManager) RegisterAlias(alias string, hash objects.Hash) error {
	rm.mu.Lock()
	defer rm.mu.Unlock()
	rm.aliases[alias] = hash
	return rm.saveUnsafe()
}

// Get the next iteration number for a timeline
func (rm *ReferenceManager) GetNextIteration(timeline string) int {
	rm.mu.Lock()
	defer rm.mu.Unlock()
	current := rm.iterations[timeline]
	next := current + 1
	rm.iterations[timeline] = next
	return next
}

// Save references to disk
func (rm *ReferenceManager) saveUnsafe() error {
	refPath := filepath.Join(rm.root, ".ivaldi", "references.json")

	data := struct {
		References map[string]*Reference   `json:"references"`
		Aliases    map[string]objects.Hash `json:"aliases"`
		Iterations map[string]int          `json:"iterations"`
	}{
		References: rm.references,
		Aliases:    rm.aliases,
		Iterations: rm.iterations,
	}

	jsonData, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(refPath, jsonData, 0644)
}

func (rm *ReferenceManager) save() error {
	rm.mu.RLock()
	defer rm.mu.RUnlock()
	return rm.saveUnsafe()
}

// Load references from disk
func (rm *ReferenceManager) Load() error {
	refPath := filepath.Join(rm.root, ".ivaldi", "references.json")

	data, err := os.ReadFile(refPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil // No references file yet
		}
		return err
	}

	var loaded struct {
		References map[string]*Reference   `json:"references"`
		Aliases    map[string]objects.Hash `json:"aliases"`
		Iterations map[string]int          `json:"iterations"`
	}

	if err := json.Unmarshal(data, &loaded); err != nil {
		return err
	}

	rm.mu.Lock()
	defer rm.mu.Unlock()
	rm.references = loaded.References
	if rm.references == nil {
		rm.references = make(map[string]*Reference)
	}

	rm.aliases = loaded.Aliases
	if rm.aliases == nil {
		rm.aliases = make(map[string]objects.Hash)
	}

	rm.iterations = loaded.Iterations
	if rm.iterations == nil {
		rm.iterations = make(map[string]int)
	}

	return nil
}

// References returns the internal references map for access
func (rm *ReferenceManager) References() map[string]*Reference {
	return rm.references
}

// GetMemorableName returns the memorable name for a hash if it exists
func (rm *ReferenceManager) GetMemorableName(hash objects.Hash) (string, bool) {
	for name, ref := range rm.references {
		if ref.Hash == hash && ref.Type == RefMemorableName {
			return name, true
		}
	}
	return "", false
}
