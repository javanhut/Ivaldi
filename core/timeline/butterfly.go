package timeline

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/objects"
)

// ButterflyDelimiter separates base timeline from variant identifier
const ButterflyDelimiter = ":diverged:"

// ButterflyInfo represents information about a butterfly variant
type ButterflyInfo struct {
	Identifier   string    `json:"identifier"`
	FullName     string    `json:"full_name"`
	BaseTimeline string    `json:"base_timeline"`
	CreatedAt    time.Time `json:"created_at"`
	LastModified time.Time `json:"last_modified"`
	HasShelved   bool      `json:"has_shelved"`
}

// UploadRecord tracks upload attempts for variants
type UploadRecord struct {
	Timeline     string       `json:"timeline"`
	CommitHash   objects.Hash `json:"commit_hash"`
	SealName     string       `json:"seal_name"`
	Portal       string       `json:"portal"`
	Timestamp    time.Time    `json:"timestamp"`
	Success      bool         `json:"success"`
	ErrorMessage string       `json:"error_message,omitempty"`
	Author       string       `json:"author"`
}

// VariantUploadTracking tracks upload history for a variant
type VariantUploadTracking struct {
	Timeline      string         `json:"timeline"`
	LastUpload    *UploadRecord  `json:"last_upload"`
	UploadHistory []UploadRecord `json:"upload_history"`
	CreatedAt     time.Time      `json:"created_at"`
	LastModified  time.Time      `json:"last_modified"`
}

// AutoShelve represents auto-shelved changes when switching variants
type AutoShelve struct {
	ID             string                 `json:"id"`
	Timeline       string                 `json:"timeline"`
	Reason         string                 `json:"reason"`
	Timestamp      time.Time              `json:"timestamp"`
	WorkingChanges map[string]interface{} `json:"working_changes"`
	AnvilFiles     map[string]interface{} `json:"anvil_files"`
	Position       objects.Hash           `json:"position"`
	AutoCreated    bool                   `json:"auto_created"`
}

// IsButterflyTimeline checks if a timeline name represents a butterfly variant
func IsButterflyTimeline(name string) bool {
	return strings.Contains(name, ButterflyDelimiter)
}

// GetBaseTimeline extracts the base timeline name from a butterfly timeline
func GetBaseTimeline(name string) string {
	if parts := strings.Split(name, ButterflyDelimiter); len(parts) > 1 {
		return parts[0]
	}
	return name
}

// GetButterflyIdentifier extracts the variant identifier from a butterfly timeline
func GetButterflyIdentifier(name string) string {
	if parts := strings.Split(name, ButterflyDelimiter); len(parts) > 1 {
		return parts[1]
	}
	return ""
}

// BuildButterflyTimelineName constructs a butterfly timeline name
func BuildButterflyTimelineName(baseTimeline, identifier string) string {
	return baseTimeline + ButterflyDelimiter + identifier
}

// CreateButterflyTimeline creates a new butterfly variant timeline
func (m *Manager) CreateButterflyTimeline(baseTimeline, identifier string) error {
	m.mu.Lock()
	defer m.mu.Unlock()

	butterflyName := BuildButterflyTimelineName(baseTimeline, identifier)

	// Check if butterfly timeline already exists
	if _, exists := m.timelines[butterflyName]; exists {
		return fmt.Errorf("butterfly variant '%s' already exists", identifier)
	}

	// Check if base timeline exists
	baseTimelineObj, exists := m.timelines[baseTimeline]
	if !exists {
		return fmt.Errorf("base timeline '%s' does not exist", baseTimeline)
	}

	// Create new butterfly timeline based on the base timeline
	butterflyTimeline := &Timeline{
		Name:        butterflyName,
		Head:        baseTimelineObj.Head,
		CreatedAt:   time.Now(),
		UpdatedAt:   time.Now(),
		Description: fmt.Sprintf("Butterfly variant '%s' of %s", identifier, baseTimeline),
		Parent:      baseTimeline,
	}

	m.timelines[butterflyName] = butterflyTimeline
	return m.save()
}

// ListButterflyTimelines returns all butterfly variants for a base timeline
func (m *Manager) ListButterflyTimelines(baseTimeline string) []string {
	m.mu.RLock()
	defer m.mu.RUnlock()

	var variants []string
	for timelineName := range m.timelines {
		if IsButterflyTimeline(timelineName) && GetBaseTimeline(timelineName) == baseTimeline {
			variants = append(variants, timelineName)
		}
	}
	return variants
}

// GetNextButterflyID generates the next auto-incrementing butterfly ID
func (m *Manager) GetNextButterflyID(baseTimeline string) int {
	m.mu.RLock()
	defer m.mu.RUnlock()

	maxID := 0
	for timelineName := range m.timelines {
		if strings.HasPrefix(timelineName, baseTimeline+ButterflyDelimiter) {
			identifier := GetButterflyIdentifier(timelineName)
			if id, err := strconv.Atoi(identifier); err == nil {
				if id > maxID {
					maxID = id
				}
			}
		}
	}
	return maxID + 1
}

// GetButterflyVariants returns detailed information about all butterfly variants for a base timeline
func (m *Manager) GetButterflyVariants(baseTimeline string) []ButterflyInfo {
	m.mu.RLock()
	defer m.mu.RUnlock()

	var variants []ButterflyInfo
	for timelineName, timeline := range m.timelines {
		if IsButterflyTimeline(timelineName) && GetBaseTimeline(timelineName) == baseTimeline {
			variants = append(variants, ButterflyInfo{
				Identifier:   GetButterflyIdentifier(timelineName),
				FullName:     timelineName,
				BaseTimeline: baseTimeline,
				CreatedAt:    timeline.CreatedAt,
				LastModified: timeline.UpdatedAt,
				HasShelved:   false, // Will be updated when shelving is implemented
			})
		}
	}
	return variants
}

// SwitchToButterflyTimeline switches to a butterfly variant
func (m *Manager) SwitchToButterflyTimeline(baseTimeline, identifier string) error {
	butterflyName := BuildButterflyTimelineName(baseTimeline, identifier)
	return m.Switch(butterflyName)
}
