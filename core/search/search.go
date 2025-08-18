package search

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/objects"
)

// Index interface for search queries
type Index interface {
	FindSealsByAuthor(author string) ([]objects.Hash, error)
	FindSealsByTimeRange(start, end time.Time) ([]objects.Hash, error)
	FindSealsContaining(searchTerm string) ([]objects.Hash, error)
	FindSealByHashPrefix(prefix string) (*objects.Hash, error)
	GetSealHistory(limit int) ([]objects.Hash, error)
}

// Storage interface for loading seal details
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
}

type SearchResult struct {
	Seal     *objects.Seal
	Matches  []string // What parts matched the query
	Score    float64  // Relevance score
}

type SearchManager struct {
	index   Index
	storage Storage
}

func NewSearchManager(index Index, storage Storage) *SearchManager {
	return &SearchManager{
		index:   index,
		storage: storage,
	}
}

// Search performs natural language search across commit history
func (sm *SearchManager) Search(query string) ([]*SearchResult, error) {
	query = strings.TrimSpace(query)
	if query == "" {
		return nil, fmt.Errorf("empty search query")
	}

	var results []*SearchResult

	// Try different search strategies based on query patterns
	
	// 1. Author search patterns
	if authorResults, err := sm.searchByAuthor(query); err == nil {
		results = append(results, authorResults...)
	}
	
	// 2. Time-based search patterns
	if timeResults, err := sm.searchByTime(query); err == nil {
		results = append(results, timeResults...)
	}
	
	// 3. Content search (always try this)
	if contentResults, err := sm.searchByContent(query); err == nil {
		results = append(results, contentResults...)
	}
	
	// 4. Hash prefix search
	if hashResults, err := sm.searchByHash(query); err == nil {
		results = append(results, hashResults...)
	}

	if len(results) == 0 {
		return nil, fmt.Errorf("no results found for '%s'", query)
	}

	// Remove duplicates and sort by score
	results = sm.deduplicateAndSort(results)
	
	return results, nil
}

func (sm *SearchManager) searchByAuthor(query string) ([]*SearchResult, error) {
	// Patterns like "by john", "author:john", "john's commits"
	authorPatterns := []string{
		`^by\s+(\w+)$`,
		`^author:(\w+)$`,
		`^(\w+)'s\s+(commits?|changes?)$`,
		`^commits?\s+by\s+(\w+)$`,
	}
	
	var author string
	for _, pattern := range authorPatterns {
		re := regexp.MustCompile(`(?i)` + pattern)
		matches := re.FindStringSubmatch(query)
		if len(matches) > 1 {
			author = matches[1]
			break
		}
	}
	
	if author == "" {
		return nil, fmt.Errorf("not an author query")
	}
	
	hashes, err := sm.index.FindSealsByAuthor(author)
	if err != nil {
		return nil, err
	}
	
	var results []*SearchResult
	for _, hash := range hashes {
		if seal, err := sm.storage.LoadSeal(hash); err == nil {
			results = append(results, &SearchResult{
				Seal:    seal,
				Matches: []string{fmt.Sprintf("author: %s", seal.Author.Name)},
				Score:   0.9, // High score for exact author match
			})
		}
	}
	
	return results, nil
}

func (sm *SearchManager) searchByTime(query string) ([]*SearchResult, error) {
	var start, end time.Time
	now := time.Now()
	
	// Time patterns
	timePatterns := map[string]func() (time.Time, time.Time){
		`today`: func() (time.Time, time.Time) {
			start := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
			end := start.Add(24 * time.Hour)
			return start, end
		},
		`yesterday`: func() (time.Time, time.Time) {
			start := time.Date(now.Year(), now.Month(), now.Day()-1, 0, 0, 0, 0, now.Location())
			end := start.Add(24 * time.Hour)
			return start, end
		},
		`this week`: func() (time.Time, time.Time) {
			start := now.AddDate(0, 0, -7)
			return start, now
		},
		`last week`: func() (time.Time, time.Time) {
			start := now.AddDate(0, 0, -14)
			end := now.AddDate(0, 0, -7)
			return start, end
		},
		`this month`: func() (time.Time, time.Time) {
			start := time.Date(now.Year(), now.Month(), 1, 0, 0, 0, 0, now.Location())
			return start, now
		},
		`last month`: func() (time.Time, time.Time) {
			start := now.AddDate(0, -1, 0)
			end := now.AddDate(0, 0, -30)
			return start, end
		},
	}
	
	// Check for time patterns
	for pattern, timeFn := range timePatterns {
		if matched, _ := regexp.MatchString(`(?i)`+pattern, query); matched {
			start, end = timeFn()
			break
		}
	}
	
	// Check for "X days/hours/minutes ago" patterns
	agoPattern := regexp.MustCompile(`(?i)(\d+)\s+(days?|hours?|minutes?)\s+ago`)
	matches := agoPattern.FindStringSubmatch(query)
	if len(matches) == 3 {
		amount, _ := strconv.Atoi(matches[1])
		unit := strings.ToLower(matches[2])
		
		var duration time.Duration
		switch {
		case strings.HasPrefix(unit, "minute"):
			duration = time.Duration(amount) * time.Minute
		case strings.HasPrefix(unit, "hour"):
			duration = time.Duration(amount) * time.Hour
		case strings.HasPrefix(unit, "day"):
			duration = time.Duration(amount) * 24 * time.Hour
		}
		
		start = now.Add(-duration - time.Hour) // ±1 hour buffer
		end = now.Add(-duration + time.Hour)
	}
	
	if start.IsZero() {
		return nil, fmt.Errorf("not a time query")
	}
	
	hashes, err := sm.index.FindSealsByTimeRange(start, end)
	if err != nil {
		return nil, err
	}
	
	var results []*SearchResult
	for _, hash := range hashes {
		if seal, err := sm.storage.LoadSeal(hash); err == nil {
			results = append(results, &SearchResult{
				Seal:    seal,
				Matches: []string{fmt.Sprintf("time: %s", seal.Timestamp.Format("2006-01-02 15:04"))},
				Score:   0.8, // High score for time match
			})
		}
	}
	
	return results, nil
}

func (sm *SearchManager) searchByContent(query string) ([]*SearchResult, error) {
	// Remove common search prefixes/suffixes
	cleanQuery := query
	cleanQuery = regexp.MustCompile(`(?i)^(where|when|find|search)\s+`).ReplaceAllString(cleanQuery, "")
	cleanQuery = regexp.MustCompile(`(?i)\s+(was|were)\s+(added|removed|changed|modified)$`).ReplaceAllString(cleanQuery, "")
	
	// Split into keywords
	keywords := strings.Fields(cleanQuery)
	if len(keywords) == 0 {
		return nil, fmt.Errorf("no keywords in query")
	}
	
	var allResults []*SearchResult
	
	// Search for each keyword
	for _, keyword := range keywords {
		if len(keyword) < 3 { // Skip very short keywords
			continue
		}
		
		hashes, err := sm.index.FindSealsContaining(keyword)
		if err != nil {
			continue
		}
		
		for _, hash := range hashes {
			if seal, err := sm.storage.LoadSeal(hash); err == nil {
				score := sm.calculateContentScore(seal.Message, keywords)
				allResults = append(allResults, &SearchResult{
					Seal:    seal,
					Matches: []string{fmt.Sprintf("content: %s", keyword)},
					Score:   score,
				})
			}
		}
	}
	
	return allResults, nil
}

func (sm *SearchManager) searchByHash(query string) ([]*SearchResult, error) {
	// Only search by hash if it looks like a hash (hex characters, 6+ chars)
	if len(query) < 6 || !regexp.MustCompile(`^[a-f0-9]+$`).MatchString(query) {
		return nil, fmt.Errorf("not a hash query")
	}
	
	hash, err := sm.index.FindSealByHashPrefix(query)
	if err != nil {
		return nil, err
	}
	
	if hash == nil {
		return nil, fmt.Errorf("no hash found")
	}
	
	seal, err := sm.storage.LoadSeal(*hash)
	if err != nil {
		return nil, err
	}
	
	return []*SearchResult{
		{
			Seal:    seal,
			Matches: []string{fmt.Sprintf("hash: %s", query)},
			Score:   1.0, // Perfect match for hash
		},
	}, nil
}

func (sm *SearchManager) calculateContentScore(message string, keywords []string) float64 {
	message = strings.ToLower(message)
	score := 0.0
	matchCount := 0
	
	for _, keyword := range keywords {
		keyword = strings.ToLower(keyword)
		if strings.Contains(message, keyword) {
			matchCount++
			// Bonus for exact word match vs substring
			if regexp.MustCompile(`\b`+regexp.QuoteMeta(keyword)+`\b`).MatchString(message) {
				score += 0.3
			} else {
				score += 0.1
			}
		}
	}
	
	// Bonus for multiple keyword matches
	if matchCount > 1 {
		score += 0.2 * float64(matchCount-1)
	}
	
	// Normalize score between 0 and 1
	if score > 1.0 {
		score = 1.0
	}
	
	return score
}

func (sm *SearchManager) deduplicateAndSort(results []*SearchResult) []*SearchResult {
	// Remove duplicates by hash
	seen := make(map[objects.Hash]bool)
	var unique []*SearchResult
	
	for _, result := range results {
		if !seen[result.Seal.Hash] {
			seen[result.Seal.Hash] = true
			unique = append(unique, result)
		}
	}
	
	// Sort by score (highest first)
	for i := 0; i < len(unique)-1; i++ {
		for j := i + 1; j < len(unique); j++ {
			if unique[j].Score > unique[i].Score {
				unique[i], unique[j] = unique[j], unique[i]
			}
		}
	}
	
	return unique
}

// SearchSuggestions provides search suggestions based on repository content
func (sm *SearchManager) SearchSuggestions() []string {
	suggestions := []string{
		"today",
		"yesterday", 
		"this week",
		"last week",
		"authentication",
		"bug fix",
		"feature",
		"test",
		"documentation",
		"by Developer",
		"2 hours ago",
		"where auth was added",
		"when tests were added",
	}
	
	return suggestions
}