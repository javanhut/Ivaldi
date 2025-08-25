package reshape

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/objects"
)

// Index interface for history operations
type Index interface {
	GetSealHistory(limit int) ([]objects.Hash, error)
}

// Storage interface for loading/storing seals
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
	StoreSeal(seal *objects.Seal) error
}

// OverwriteTracker interface for accountability
type OverwriteTracker interface {
	RecordOverwrite(original, new objects.Hash, justification, category string) error
	RequiresApproval(category string) bool
}

// ReshapeManager handles history modification with full accountability
type ReshapeManager struct {
	index     Index
	storage   Storage
	overwrite OverwriteTracker
}

type ReshapeOptions struct {
	Count         int
	Justification string
	Category      string
	Interactive   bool
	DryRun        bool
}

type ReshapeResult struct {
	OriginalSeals  []*objects.Seal
	NewSeals       []*objects.Seal
	OverwriteID    string
	ArchivedHashes []objects.Hash
}

func NewReshapeManager(index Index, storage Storage, tracker OverwriteTracker) *ReshapeManager {
	return &ReshapeManager{
		index:     index,
		storage:   storage,
		overwrite: tracker,
	}
}

// Reshape modifies the last N commits with full accountability tracking
func (rm *ReshapeManager) Reshape(opts ReshapeOptions) (*ReshapeResult, error) {
	if opts.Count <= 0 {
		return nil, fmt.Errorf("count must be positive")
	}

	if opts.Justification == "" {
		return nil, fmt.Errorf("justification is required for reshape operations")
	}

	// Validate category
	validCategories := []string{"security", "cleanup", "mistake", "refactor", "rebase", "squash", "amend"}
	if !contains(validCategories, opts.Category) {
		return nil, fmt.Errorf("invalid category '%s', must be one of: %s",
			opts.Category, strings.Join(validCategories, ", "))
	}

	// Get the last N seals
	hashes, err := rm.index.GetSealHistory(opts.Count)
	if err != nil {
		return nil, fmt.Errorf("failed to get seal history: %v", err)
	}

	if len(hashes) < opts.Count {
		return nil, fmt.Errorf("only %d seals available, cannot reshape %d", len(hashes), opts.Count)
	}

	// Load the seals to be reshaped
	var originalSeals []*objects.Seal
	for i := 0; i < opts.Count; i++ {
		seal, err := rm.storage.LoadSeal(hashes[i])
		if err != nil {
			return nil, fmt.Errorf("failed to load seal %s: %v", hashes[i], err)
		}
		originalSeals = append(originalSeals, seal)
	}

	// Check if approval is required (only if overwrite tracker is available)
	if rm.overwrite != nil && rm.overwrite.RequiresApproval(opts.Category) {
		return nil, fmt.Errorf("reshape category '%s' requires approval - use approval workflow", opts.Category)
	}

	// Dry run - just show what would happen
	if opts.DryRun {
		return &ReshapeResult{
			OriginalSeals:  originalSeals,
			NewSeals:       []*objects.Seal{}, // Would be computed in real run
			ArchivedHashes: hashes[:opts.Count],
		}, nil
	}

	// Perform the actual reshape
	newSeals, err := rm.performReshape(originalSeals, opts)
	if err != nil {
		return nil, fmt.Errorf("reshape failed: %v", err)
	}

	// Record overwrite for accountability (only if tracker is available)
	overwriteID := ""
	if rm.overwrite != nil && len(newSeals) > 0 && len(originalSeals) > 0 {
		err = rm.overwrite.RecordOverwrite(
			originalSeals[0].Hash,
			newSeals[0].Hash,
			opts.Justification,
			opts.Category)
		if err != nil {
			return nil, fmt.Errorf("failed to record overwrite: %v", err)
		}
	}

	return &ReshapeResult{
		OriginalSeals:  originalSeals,
		NewSeals:       newSeals,
		OverwriteID:    overwriteID,
		ArchivedHashes: hashes[:opts.Count],
	}, nil
}

// performReshape handles different reshape operations
func (rm *ReshapeManager) performReshape(seals []*objects.Seal, opts ReshapeOptions) ([]*objects.Seal, error) {
	switch opts.Category {
	case "squash":
		return rm.squashSeals(seals, opts.Justification)
	case "amend":
		return rm.amendLastSeal(seals, opts.Justification)
	case "rebase":
		return rm.rebaseSeals(seals, opts.Justification)
	case "cleanup":
		return rm.cleanupSeals(seals, opts.Justification)
	default:
		return rm.defaultReshape(seals, opts.Justification)
	}
}

// squashSeals combines multiple seals into one
func (rm *ReshapeManager) squashSeals(seals []*objects.Seal, justification string) ([]*objects.Seal, error) {
	if len(seals) < 2 {
		return nil, fmt.Errorf("need at least 2 seals to squash")
	}

	// Create a new seal that combines all messages
	var messages []string
	for _, seal := range seals {
		messages = append(messages, seal.Message)
	}

	combinedMessage := strings.Join(messages, "; ")

	// Create new squashed seal
	newSeal := &objects.Seal{
		Name:      rm.generateMemorableName(),
		Iteration: seals[0].Iteration, // Keep the first iteration number
		Message:   fmt.Sprintf("SQUASHED: %s", combinedMessage),
		Author:    seals[0].Author, // Use first author
		Timestamp: time.Now(),
		Parents:   seals[len(seals)-1].Parents, // Use parents of last seal
	}

	// Add overwrite record to the seal
	newSeal.Overwrites = []objects.Overwrite{
		{
			PreviousHash: seals[0].Hash,
			Reason:       justification,
			Author:       seals[0].Author,
			Timestamp:    time.Now(),
		},
	}

	// Store the new seal
	if err := rm.storage.StoreSeal(newSeal); err != nil {
		return nil, err
	}

	return []*objects.Seal{newSeal}, nil
}

// amendLastSeal modifies the most recent seal
func (rm *ReshapeManager) amendLastSeal(seals []*objects.Seal, justification string) ([]*objects.Seal, error) {
	if len(seals) != 1 {
		return nil, fmt.Errorf("amend can only modify 1 seal, got %d", len(seals))
	}

	original := seals[0]

	// Create amended seal
	amended := &objects.Seal{
		Name:      rm.generateMemorableName(),
		Iteration: original.Iteration,
		Message:   fmt.Sprintf("AMENDED: %s", original.Message),
		Author:    original.Author,
		Timestamp: time.Now(),
		Parents:   original.Parents,
		Overwrites: []objects.Overwrite{
			{
				PreviousHash: original.Hash,
				Reason:       justification,
				Author:       original.Author,
				Timestamp:    time.Now(),
			},
		},
	}

	if err := rm.storage.StoreSeal(amended); err != nil {
		return nil, err
	}

	return []*objects.Seal{amended}, nil
}

// rebaseSeals replays seals on top of a new base
func (rm *ReshapeManager) rebaseSeals(seals []*objects.Seal, justification string) ([]*objects.Seal, error) {
	// For now, just mark them as rebased
	var newSeals []*objects.Seal

	for i, seal := range seals {
		rebased := &objects.Seal{
			Name:      rm.generateMemorableName(),
			Iteration: seal.Iteration,
			Message:   fmt.Sprintf("REBASED: %s", seal.Message),
			Author:    seal.Author,
			Timestamp: time.Now(),
			Parents:   seal.Parents,
		}

		if i == 0 {
			// First seal gets the overwrite record
			rebased.Overwrites = []objects.Overwrite{
				{
					PreviousHash: seal.Hash,
					Reason:       justification,
					Author:       seal.Author,
					Timestamp:    time.Now(),
				},
			}
		}

		if err := rm.storage.StoreSeal(rebased); err != nil {
			return nil, err
		}

		newSeals = append(newSeals, rebased)
	}

	return newSeals, nil
}

// cleanupSeals performs general cleanup on seals
func (rm *ReshapeManager) cleanupSeals(seals []*objects.Seal, justification string) ([]*objects.Seal, error) {
	var newSeals []*objects.Seal

	for i, seal := range seals {
		cleaned := &objects.Seal{
			Name:      rm.generateMemorableName(),
			Iteration: seal.Iteration,
			Message:   rm.cleanupMessage(seal.Message),
			Author:    seal.Author,
			Timestamp: time.Now(),
			Parents:   seal.Parents,
		}

		if i == 0 {
			cleaned.Overwrites = []objects.Overwrite{
				{
					PreviousHash: seal.Hash,
					Reason:       justification,
					Author:       seal.Author,
					Timestamp:    time.Now(),
				},
			}
		}

		if err := rm.storage.StoreSeal(cleaned); err != nil {
			return nil, err
		}

		newSeals = append(newSeals, cleaned)
	}

	return newSeals, nil
}

// defaultReshape performs a generic reshape
func (rm *ReshapeManager) defaultReshape(seals []*objects.Seal, justification string) ([]*objects.Seal, error) {
	// Default: just add overwrite tracking without changing content
	var newSeals []*objects.Seal

	for i, seal := range seals {
		reshaped := &objects.Seal{
			Name:      seal.Name, // Keep original name
			Iteration: seal.Iteration,
			Message:   seal.Message,
			Author:    seal.Author,
			Timestamp: seal.Timestamp,
			Parents:   seal.Parents,
		}

		if i == 0 {
			reshaped.Overwrites = []objects.Overwrite{
				{
					PreviousHash: seal.Hash,
					Reason:       justification,
					Author:       seal.Author,
					Timestamp:    time.Now(),
				},
			}
		}

		newSeals = append(newSeals, reshaped)
	}

	return newSeals, nil
}

// cleanupMessage improves commit message formatting
func (rm *ReshapeManager) cleanupMessage(message string) string {
	// Basic cleanup: trim whitespace, fix capitalization
	message = strings.TrimSpace(message)
	if len(message) > 0 {
		// Capitalize first letter
		message = strings.ToUpper(string(message[0])) + message[1:]
	}
	return message
}

// generateMemorableName creates a new memorable name for reshaped seals
func (rm *ReshapeManager) generateMemorableName() string {
	adjectives := []string{"bright", "swift", "bold", "calm", "wise", "strong"}
	nouns := []string{"river", "mountain", "forest", "ocean", "star", "moon"}

	// Simple generation for now
	adj := adjectives[time.Now().Unix()%int64(len(adjectives))]
	noun := nouns[time.Now().Unix()%int64(len(nouns))]
	num := time.Now().Unix() % 999

	return fmt.Sprintf("%s-%s-%d", adj, noun, num)
}

// ParseReshapeCommand parses natural language reshape commands
func ParseReshapeCommand(command string) (*ReshapeOptions, error) {
	command = strings.ToLower(strings.TrimSpace(command))

	// Parse "reshape last N"
	if strings.HasPrefix(command, "reshape last ") {
		countStr := strings.TrimPrefix(command, "reshape last ")
		count, err := strconv.Atoi(countStr)
		if err != nil {
			return nil, fmt.Errorf("invalid count: %s", countStr)
		}

		return &ReshapeOptions{
			Count:    count,
			Category: "cleanup",
		}, nil
	}

	// Parse "squash last N"
	if strings.HasPrefix(command, "squash last ") {
		countStr := strings.TrimPrefix(command, "squash last ")
		count, err := strconv.Atoi(countStr)
		if err != nil {
			return nil, fmt.Errorf("invalid count: %s", countStr)
		}

		return &ReshapeOptions{
			Count:    count,
			Category: "squash",
		}, nil
	}

	return nil, fmt.Errorf("unrecognized reshape command: %s", command)
}

// contains checks if a slice contains a string
func contains(slice []string, item string) bool {
	for _, s := range slice {
		if s == item {
			return true
		}
	}
	return false
}

// GetReshapeCategories returns valid reshape categories with descriptions
func GetReshapeCategories() map[string]string {
	return map[string]string{
		"security": "Fix security vulnerabilities",
		"cleanup":  "Clean up commit messages or structure",
		"mistake":  "Fix mistakes in previous commits",
		"refactor": "Refactor without changing functionality",
		"rebase":   "Rebase commits onto new base",
		"squash":   "Combine multiple commits into one",
		"amend":    "Modify the most recent commit",
	}
}
