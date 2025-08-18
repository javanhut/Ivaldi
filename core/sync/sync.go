package sync

import (
	"fmt"
	"time"

	"ivaldi/core/fuse"
	"ivaldi/core/network"
	"ivaldi/core/objects"
)

// SyncManager handles synchronization with remote portals
type SyncManager struct {
	network   *network.NetworkManager
	fuse      *fuse.FuseManager
	timeline  TimelineManager
	storage   Storage
}

// Storage interface for loading/storing seals
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
	StoreSeal(seal *objects.Seal) error
}

// TimelineManager interface for timeline operations
type TimelineManager interface {
	Current() string
	GetHead(timeline string) (objects.Hash, error)
	UpdateHead(timeline string, hash objects.Hash) error
	Create(name, description string) error
	Switch(name string) error
}

// SyncOptions configures synchronization behavior
type SyncOptions struct {
	PortalName     string
	RemoteTimeline string
	LocalTimeline  string
	Strategy       fuse.FuseStrategy
	Force          bool
	DryRun         bool
}

// SyncResult contains the outcome of a sync operation
type SyncResult struct {
	FetchedSeals   int
	MergeResult    *fuse.FuseResult
	ConflictCount  int
	Success        bool
	Message        string
}

// NewSyncManager creates a new sync manager
func NewSyncManager(storage Storage, timeline TimelineManager, fuseManager *fuse.FuseManager, root string) *SyncManager {
	return &SyncManager{
		network:  network.NewNetworkManager(root),
		fuse:     fuseManager,
		timeline: timeline,
		storage:  storage,
	}
}

// Sync performs a complete synchronization with a remote portal
func (sm *SyncManager) Sync(portalURL string, opts SyncOptions) (*SyncResult, error) {
	// Step 1: Fetch remote changes
	fetchResult, err := sm.network.FetchFromPortal(portalURL, opts.RemoteTimeline)
	if err != nil {
		return nil, fmt.Errorf("failed to fetch from portal: %v", err)
	}

	// If no remote changes, we're up to date
	if len(fetchResult.Seals) == 0 {
		return &SyncResult{
			FetchedSeals:  0,
			ConflictCount: 0,
			Success:       true,
			Message:       "Already up to date",
		}, nil
	}

	// Step 2: Store fetched seals
	for _, seal := range fetchResult.Seals {
		if err := sm.storage.StoreSeal(seal); err != nil {
			return nil, fmt.Errorf("failed to store fetched seal: %v", err)
		}
	}

	// Step 3: Create temporary timeline for remote changes
	remoteTimelineName := fmt.Sprintf("temp-remote-%d", time.Now().Unix())
	if err := sm.timeline.Create(remoteTimelineName, "Temporary timeline for remote changes"); err != nil {
		return nil, fmt.Errorf("failed to create temporary timeline: %v", err)
	}

	// Step 4: Update temporary timeline with remote head
	if len(fetchResult.Refs) > 0 {
		remoteHead := fetchResult.Refs[0].Hash // Use first ref as head
		if err := sm.timeline.UpdateHead(remoteTimelineName, remoteHead); err != nil {
			return nil, fmt.Errorf("failed to update remote timeline head: %v", err)
		}
	}

	// Step 5: Determine target timeline
	targetTimeline := opts.LocalTimeline
	if targetTimeline == "" {
		targetTimeline = sm.timeline.Current()
	}

	// Step 6: Check for divergent branches
	localHead, err := sm.timeline.GetHead(targetTimeline)
	if err != nil {
		return nil, fmt.Errorf("failed to get local head: %v", err)
	}

	remoteHead := fetchResult.Refs[0].Hash
	if localHead == remoteHead {
		return &SyncResult{
			FetchedSeals:  len(fetchResult.Seals),
			ConflictCount: 0,
			Success:       true,
			Message:       "Already synchronized",
		}, nil
	}

	// Step 7: Determine sync strategy
	strategy := opts.Strategy
	if strategy == 0 { // Default to automatic
		strategy = fuse.FuseStrategyAutomatic
	}

	// Step 8: Perform the fuse operation
	fuseOpts := fuse.FuseOptions{
		SourceTimeline: remoteTimelineName,
		TargetTimeline: targetTimeline,
		FuseMessage:    fmt.Sprintf("Sync with %s", opts.PortalName),
		Strategy:       strategy,
		DeleteSource:   true, // Clean up temporary timeline
		DryRun:         opts.DryRun,
	}

	fuseResult, err := sm.fuse.Fuse(fuseOpts)
	if err != nil {
		return nil, fmt.Errorf("failed to fuse remote changes: %v", err)
	}

	return &SyncResult{
		FetchedSeals:  len(fetchResult.Seals),
		MergeResult:   fuseResult,
		ConflictCount: fuseResult.ConflictCount,
		Success:       true,
		Message:       "Sync completed successfully",
	}, nil
}

// Push uploads local changes to a remote portal
func (sm *SyncManager) Push(portalURL, timeline string) error {
	// Get seals to upload (this would be more sophisticated in practice)
	localHead, err := sm.timeline.GetHead(timeline)
	if err != nil {
		return fmt.Errorf("failed to get local head: %v", err)
	}

	// For now, just upload the head seal
	seal, err := sm.storage.LoadSeal(localHead)
	if err != nil {
		return fmt.Errorf("failed to load seal for upload: %v", err)
	}

	return sm.network.UploadToPortal(portalURL, timeline, []*objects.Seal{seal})
}

// DetectDivergence checks if local and remote timelines have diverged
func (sm *SyncManager) DetectDivergence(localHead, remoteHead objects.Hash) (bool, error) {
	// Simple divergence detection - in practice this would check ancestry
	return localHead != remoteHead, nil
}

// CreateSyncStrategy determines the best sync strategy based on the situation
func (sm *SyncManager) CreateSyncStrategy(localHead, remoteHead objects.Hash, force bool) (fuse.FuseStrategy, error) {
	// Check if this can be a fast-forward
	if localHead == (objects.Hash{}) {
		// No local changes, can fast-forward
		return fuse.FuseStrategyFastForward, nil
	}

	if force {
		// Force merge with manual resolution
		return fuse.FuseStrategyManual, nil
	}

	// Default to automatic merge
	return fuse.FuseStrategyAutomatic, nil
}

// ValidateSync checks if a sync operation is safe to perform
func (sm *SyncManager) ValidateSync(timeline string) error {
	// Check for uncommitted changes would go here
	// For now, assume it's always safe
	return nil
}