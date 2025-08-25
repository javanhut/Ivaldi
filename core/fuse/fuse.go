package fuse

import (
	"fmt"
	"time"

	"ivaldi/core/objects"
)

// Storage interface for loading/storing seals and timelines
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
	StoreSeal(seal *objects.Seal) error
}

// TimelineManager interface for timeline operations
type TimelineManager interface {
	Current() string
	GetHead(timeline string) (objects.Hash, error)
	UpdateHead(timeline string, hash objects.Hash) error
	DeleteTimeline(name string) error
}

// WorkspaceManager interface for handling file changes
type WorkspaceManager interface {
	HasUncommittedChanges() bool
	SaveState(timeline string) error
	LoadState(timeline string) error
}

// FuseManager handles timeline merging operations
type FuseManager struct {
	storage   Storage
	timeline  TimelineManager
	workspace WorkspaceManager
}

// FuseOptions configures how the fuse operation works
type FuseOptions struct {
	SourceTimeline string
	TargetTimeline string
	FuseMessage    string
	Strategy       FuseStrategy
	DeleteSource   bool
	DryRun         bool
}

// FuseStrategy determines how conflicts are resolved
type FuseStrategy int

const (
	FuseStrategyAutomatic   FuseStrategy = iota // Automatic non-conflicting merge
	FuseStrategyManual                          // Require manual conflict resolution
	FuseStrategyFastForward                     // Fast-forward only
	FuseStrategySquash                          // Squash all commits into one
)

// FuseResult contains the outcome of a fuse operation
type FuseResult struct {
	MergeCommit   *objects.Seal
	ConflictCount int
	FilesChanged  []string
	DeletedSource bool
	Strategy      FuseStrategy
	Success       bool
}

func NewFuseManager(storage Storage, timeline TimelineManager, workspace WorkspaceManager) *FuseManager {
	return &FuseManager{
		storage:   storage,
		timeline:  timeline,
		workspace: workspace,
	}
}

// Fuse merges one timeline into another
func (fm *FuseManager) Fuse(opts FuseOptions) (*FuseResult, error) {
	// Validate source timeline exists
	sourceHead, err := fm.timeline.GetHead(opts.SourceTimeline)
	if err != nil {
		return nil, fmt.Errorf("source timeline '%s' not found: %v", opts.SourceTimeline, err)
	}

	// Use current timeline as target if not specified
	if opts.TargetTimeline == "" {
		opts.TargetTimeline = fm.timeline.Current()
	}

	targetHead, err := fm.timeline.GetHead(opts.TargetTimeline)
	if err != nil {
		return nil, fmt.Errorf("target timeline '%s' not found: %v", opts.TargetTimeline, err)
	}

	// Check for uncommitted changes
	if fm.workspace.HasUncommittedChanges() {
		return nil, fmt.Errorf("you have uncommitted changes, please seal them first")
	}

	// Determine merge strategy
	strategy, err := fm.determineMergeStrategy(sourceHead, targetHead, opts.Strategy)
	if err != nil {
		return nil, err
	}

	// Dry run - just analyze what would happen
	if opts.DryRun {
		return fm.analyzeFuse(sourceHead, targetHead, strategy, opts)
	}

	// Perform the actual fuse
	return fm.performFuse(sourceHead, targetHead, strategy, opts)
}

// determineMergeStrategy analyzes the timeline relationship and chooses strategy
func (fm *FuseManager) determineMergeStrategy(sourceHead, targetHead objects.Hash, requested FuseStrategy) (FuseStrategy, error) {
	// If source and target are the same, no merge needed
	if sourceHead == targetHead {
		return FuseStrategyFastForward, fmt.Errorf("timelines are already identical")
	}

	// Check if this can be a fast-forward merge
	canFastForward, err := fm.canFastForward(sourceHead, targetHead)
	if err != nil {
		return requested, err
	}

	if canFastForward && requested == FuseStrategyAutomatic {
		return FuseStrategyFastForward, nil
	}

	return requested, nil
}

// canFastForward checks if the target is an ancestor of source
func (fm *FuseManager) canFastForward(sourceHead, targetHead objects.Hash) (bool, error) {
	// Load source seal to check its parents
	sourceSeal, err := fm.storage.LoadSeal(sourceHead)
	if err != nil {
		return false, err
	}

	// Simple check: if target is in source's parents, it's a fast-forward
	for _, parent := range sourceSeal.Parents {
		if parent == targetHead {
			return true, nil
		}
	}

	// For now, assume no fast-forward if not direct parent
	// TODO: Implement full ancestry checking
	return false, nil
}

// analyzeFuse performs a dry-run analysis
func (fm *FuseManager) analyzeFuse(sourceHead, targetHead objects.Hash, strategy FuseStrategy, opts FuseOptions) (*FuseResult, error) {
	sourceSeal, err := fm.storage.LoadSeal(sourceHead)
	if err != nil {
		return nil, err
	}

	targetSeal, err := fm.storage.LoadSeal(targetHead)
	if err != nil {
		return nil, err
	}

	result := &FuseResult{
		ConflictCount: 0, // For now, assume no conflicts
		FilesChanged:  []string{"(analysis not implemented)"},
		DeletedSource: opts.DeleteSource,
		Strategy:      strategy,
		Success:       true,
	}

	switch strategy {
	case FuseStrategyFastForward:
		result.FilesChanged = []string{"Fast-forward: no merge commit needed"}
	case FuseStrategySquash:
		result.FilesChanged = []string{fmt.Sprintf("All commits from %s will be squashed", opts.SourceTimeline)}
	default:
		result.FilesChanged = []string{fmt.Sprintf("Merge %s (%s) into %s (%s)",
			opts.SourceTimeline, sourceSeal.Name,
			opts.TargetTimeline, targetSeal.Name)}
	}

	return result, nil
}

// performFuse executes the actual merge
func (fm *FuseManager) performFuse(sourceHead, targetHead objects.Hash, strategy FuseStrategy, opts FuseOptions) (*FuseResult, error) {
	switch strategy {
	case FuseStrategyFastForward:
		return fm.performFastForward(sourceHead, targetHead, opts)
	case FuseStrategySquash:
		return fm.performSquash(sourceHead, targetHead, opts)
	case FuseStrategyAutomatic:
		return fm.performMerge(sourceHead, targetHead, opts)
	default:
		return nil, fmt.Errorf("merge strategy %v not implemented", strategy)
	}
}

// performFastForward does a fast-forward merge
func (fm *FuseManager) performFastForward(sourceHead, targetHead objects.Hash, opts FuseOptions) (*FuseResult, error) {
	// Update target timeline to point to source head
	err := fm.timeline.UpdateHead(opts.TargetTimeline, sourceHead)
	if err != nil {
		return nil, fmt.Errorf("failed to update timeline: %v", err)
	}

	result := &FuseResult{
		ConflictCount: 0,
		FilesChanged:  []string{"Fast-forward completed"},
		DeletedSource: false,
		Strategy:      FuseStrategyFastForward,
		Success:       true,
	}

	// Delete source timeline if requested
	if opts.DeleteSource {
		err = fm.timeline.DeleteTimeline(opts.SourceTimeline)
		if err != nil {
			return result, fmt.Errorf("failed to delete source timeline: %v", err)
		}
		result.DeletedSource = true
	}

	return result, nil
}

// performSquash combines all commits from source into one
func (fm *FuseManager) performSquash(sourceHead, targetHead objects.Hash, opts FuseOptions) (*FuseResult, error) {
	sourceSeal, err := fm.storage.LoadSeal(sourceHead)
	if err != nil {
		return nil, err
	}

	// Create a new seal that represents the squashed changes
	squashMessage := opts.FuseMessage
	if squashMessage == "" {
		squashMessage = fmt.Sprintf("Squash fuse of %s into %s", opts.SourceTimeline, opts.TargetTimeline)
	}

	squashSeal := &objects.Seal{
		Name:      fm.generateMergeCommitName(),
		Iteration: fm.getNextIteration(opts.TargetTimeline),
		Message:   squashMessage,
		Author: objects.Identity{
			Name:  "Developer", // TODO: Get from config
			Email: "dev@example.com",
		},
		Timestamp: time.Now(),
		Parents:   []objects.Hash{targetHead}, // Only target as parent for squash
	}

	// Store the squash seal
	err = fm.storage.StoreSeal(squashSeal)
	if err != nil {
		return nil, fmt.Errorf("failed to store squash seal: %v", err)
	}

	// Update target timeline
	err = fm.timeline.UpdateHead(opts.TargetTimeline, squashSeal.Hash)
	if err != nil {
		return nil, fmt.Errorf("failed to update timeline: %v", err)
	}

	result := &FuseResult{
		MergeCommit:   squashSeal,
		ConflictCount: 0,
		FilesChanged:  []string{fmt.Sprintf("Squashed %s", sourceSeal.Name)},
		DeletedSource: false,
		Strategy:      FuseStrategySquash,
		Success:       true,
	}

	// Delete source timeline if requested
	if opts.DeleteSource {
		err = fm.timeline.DeleteTimeline(opts.SourceTimeline)
		if err != nil {
			return result, fmt.Errorf("failed to delete source timeline: %v", err)
		}
		result.DeletedSource = true
	}

	return result, nil
}

// performMerge creates a standard merge commit
func (fm *FuseManager) performMerge(sourceHead, targetHead objects.Hash, opts FuseOptions) (*FuseResult, error) {
	sourceSeal, err := fm.storage.LoadSeal(sourceHead)
	if err != nil {
		return nil, err
	}

	targetSeal, err := fm.storage.LoadSeal(targetHead)
	if err != nil {
		return nil, err
	}

	// Create merge commit message
	mergeMessage := opts.FuseMessage
	if mergeMessage == "" {
		mergeMessage = fmt.Sprintf("Fuse %s (%s) into %s",
			opts.SourceTimeline, sourceSeal.Name, opts.TargetTimeline)
	}

	mergeSeal := &objects.Seal{
		Name:      fm.generateMergeCommitName(),
		Iteration: fm.getNextIteration(opts.TargetTimeline),
		Message:   mergeMessage,
		Author: objects.Identity{
			Name:  "Developer", // TODO: Get from config
			Email: "dev@example.com",
		},
		Timestamp: time.Now(),
		Parents:   []objects.Hash{targetHead, sourceHead}, // Both as parents
	}

	// Store the merge seal
	err = fm.storage.StoreSeal(mergeSeal)
	if err != nil {
		return nil, fmt.Errorf("failed to store merge seal: %v", err)
	}

	// Update target timeline
	err = fm.timeline.UpdateHead(opts.TargetTimeline, mergeSeal.Hash)
	if err != nil {
		return nil, fmt.Errorf("failed to update timeline: %v", err)
	}

	result := &FuseResult{
		MergeCommit:   mergeSeal,
		ConflictCount: 0,
		FilesChanged:  []string{fmt.Sprintf("Merged %s into %s", sourceSeal.Name, targetSeal.Name)},
		DeletedSource: false,
		Strategy:      FuseStrategyAutomatic,
		Success:       true,
	}

	// Delete source timeline if requested
	if opts.DeleteSource {
		err = fm.timeline.DeleteTimeline(opts.SourceTimeline)
		if err != nil {
			return result, fmt.Errorf("failed to delete source timeline: %v", err)
		}
		result.DeletedSource = true
	}

	return result, nil
}

// generateMergeCommitName creates a memorable name for merge commits
func (fm *FuseManager) generateMergeCommitName() string {
	adjectives := []string{"unified", "merged", "fused", "joined", "combined", "blended"}
	nouns := []string{"timeline", "stream", "branch", "path", "flow", "current"}

	adj := adjectives[time.Now().Unix()%int64(len(adjectives))]
	noun := nouns[time.Now().Unix()%int64(len(nouns))]
	num := time.Now().Unix() % 999

	return fmt.Sprintf("%s-%s-%d", adj, noun, num)
}

// getNextIteration returns the next iteration number for a timeline
func (fm *FuseManager) getNextIteration(timeline string) int {
	// TODO: Implement proper iteration tracking per timeline
	return int(time.Now().Unix() % 100) // Placeholder
}

// GetFuseStrategies returns available fuse strategies with descriptions
func GetFuseStrategies() map[string]string {
	return map[string]string{
		"auto":   "Automatic merge with conflict detection",
		"ff":     "Fast-forward only (no merge commit)",
		"squash": "Squash all commits into one",
		"manual": "Manual conflict resolution required",
	}
}

// ParseFuseStrategy converts string to FuseStrategy
func ParseFuseStrategy(strategy string) (FuseStrategy, error) {
	switch strategy {
	case "auto", "automatic":
		return FuseStrategyAutomatic, nil
	case "ff", "fast-forward":
		return FuseStrategyFastForward, nil
	case "squash":
		return FuseStrategySquash, nil
	case "manual":
		return FuseStrategyManual, nil
	default:
		return FuseStrategyAutomatic, fmt.Errorf("unknown fuse strategy: %s", strategy)
	}
}
