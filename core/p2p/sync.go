package p2p

import (
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"ivaldi/core/objects"
)

// P2PSyncManager handles continuous synchronization between peers
type P2PSyncManager struct {
	p2pNetwork      *P2PNetwork
	storage         Storage
	timelineManager TimelineManager
	syncState       map[string]*PeerSyncState
	syncMutex       sync.RWMutex
	autoSync        bool
	syncInterval    time.Duration
	configMutex     sync.RWMutex // Protects autoSync and syncInterval
}

// Storage interface for P2P sync operations
type Storage interface {
	LoadSeal(hash objects.Hash) (*objects.Seal, error)
	StoreSeal(seal *objects.Seal) error
	LoadTree(hash objects.Hash) (*objects.Tree, error)
	LoadBlob(hash objects.Hash) (*objects.Blob, error)
	StoreTree(tree *objects.Tree) error
	StoreBlob(blob *objects.Blob) error
	HasObject(hash objects.Hash) bool
	ListSeals() ([]objects.Hash, error)
}

// TimelineManager interface for timeline operations
type TimelineManager interface {
	Current() string
	GetHead(timeline string) (objects.Hash, error)
	UpdateHead(timeline string, hash objects.Hash) error
	Create(name, description string) error
	Switch(name string) error
	ListTimelines() []string
	GetTimelineMetadata(name string) (*TimelineMetadata, error)
}

// PeerSyncState tracks synchronization state with each peer
type PeerSyncState struct {
	PeerID           string                  `json:"peer_id"`
	LastSync         time.Time               `json:"last_sync"`
	TimelineHeads    map[string]objects.Hash `json:"timeline_heads"`
	SyncedSeals      map[string]time.Time    `json:"synced_seals"`
	ConflictCount    int                     `json:"conflict_count"`
	BytesTransferred int64                   `json:"bytes_transferred"`
	AutoSyncEnabled  bool                    `json:"auto_sync_enabled"`
}

// TimelineMetadata contains information about a timeline
type TimelineMetadata struct {
	Name        string           `json:"name"`
	Description string           `json:"description"`
	Head        objects.Hash     `json:"head"`
	LastUpdate  time.Time        `json:"last_update"`
	Author      objects.Identity `json:"author"`
}

// SyncRequest represents a request to synchronize timelines
type SyncRequest struct {
	Timelines   []string                `json:"timelines"`
	LocalHeads  map[string]objects.Hash `json:"local_heads"`
	RequestType string                  `json:"request_type"` // "full", "incremental", "check"
	Timestamp   time.Time               `json:"timestamp"`
}

// SyncResponse contains the response to a sync request
type SyncResponse struct {
	Timelines    []string                `json:"timelines"`
	RemoteHeads  map[string]objects.Hash `json:"remote_heads"`
	MissingSeals []objects.Hash          `json:"missing_seals"`
	SealsData    []*objects.Seal         `json:"seals_data"`
	TreesData    []*objects.Tree         `json:"trees_data"`
	BlobsData    []*objects.Blob         `json:"blobs_data"`
	Conflicts    []ConflictInfo          `json:"conflicts"`
	Timestamp    time.Time               `json:"timestamp"`
}

// ConflictInfo represents a synchronization conflict
type ConflictInfo struct {
	Timeline     string       `json:"timeline"`
	LocalHead    objects.Hash `json:"local_head"`
	RemoteHead   objects.Hash `json:"remote_head"`
	ConflictType string       `json:"conflict_type"` // "diverged", "missing_parent"
}

// TimelineUpdate represents a real-time timeline update
type TimelineUpdate struct {
	Timeline  string        `json:"timeline"`
	NewHead   objects.Hash  `json:"new_head"`
	Seal      *objects.Seal `json:"seal"`
	Author    string        `json:"author"`
	Timestamp time.Time     `json:"timestamp"`
	Message   string        `json:"message"`
}

// NewP2PSyncManager creates a new P2P sync manager
func NewP2PSyncManager(p2pNetwork *P2PNetwork, storage Storage, timelineManager TimelineManager) *P2PSyncManager {
	return &P2PSyncManager{
		p2pNetwork:      p2pNetwork,
		storage:         storage,
		timelineManager: timelineManager,
		syncState:       make(map[string]*PeerSyncState),
		autoSync:        true,
		syncInterval:    30 * time.Second,
	}
}

// Start begins the P2P sync service
func (psm *P2PSyncManager) Start() error {
	// Start auto-sync service for continuous synchronization
	go psm.autoSyncService()

	// Start timeline watch service for real-time updates
	go psm.timelineWatchService()

	fmt.Println("P2P sync manager started with continuous sync enabled")
	return nil
}

// EnableAutoSync enables or disables automatic synchronization
func (psm *P2PSyncManager) EnableAutoSync(enabled bool) {
	psm.configMutex.Lock()
	defer psm.configMutex.Unlock()
	psm.autoSync = enabled
	fmt.Printf("Auto-sync %s\n", map[bool]string{true: "enabled", false: "disabled"}[enabled])
}

// SetSyncInterval sets the interval for automatic synchronization
func (psm *P2PSyncManager) SetSyncInterval(interval time.Duration) {
	psm.configMutex.Lock()
	defer psm.configMutex.Unlock()
	psm.syncInterval = interval
	fmt.Printf("Sync interval set to %v\n", interval)
}

// autoSyncService continuously syncs with peers to keep them updated
func (psm *P2PSyncManager) autoSyncService() {
	psm.configMutex.RLock()
	interval := psm.syncInterval
	psm.configMutex.RUnlock()

	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			psm.configMutex.RLock()
			autoSync := psm.autoSync
			psm.configMutex.RUnlock()

			if autoSync {
				psm.syncWithAllPeers()
			}
		}
	}
}

// timelineWatchService monitors for local timeline changes and broadcasts them
func (psm *P2PSyncManager) timelineWatchService() {
	ticker := time.NewTicker(5 * time.Second) // Check for changes every 5 seconds
	defer ticker.Stop()

	lastKnownHeads := make(map[string]objects.Hash)

	for {
		select {
		case <-ticker.C:
			psm.checkForTimelineUpdates(lastKnownHeads)
		}
	}
}

// checkForTimelineUpdates detects local timeline changes and broadcasts them
func (psm *P2PSyncManager) checkForTimelineUpdates(lastKnownHeads map[string]objects.Hash) {
	timelines := psm.timelineManager.ListTimelines()

	for _, timeline := range timelines {
		currentHead, err := psm.timelineManager.GetHead(timeline)
		if err != nil {
			continue
		}

		lastHead, exists := lastKnownHeads[timeline]
		if !exists || lastHead != currentHead {
			// Timeline has changed, broadcast update
			psm.broadcastTimelineUpdate(timeline, currentHead)
			lastKnownHeads[timeline] = currentHead
		}
	}
}

// broadcastTimelineUpdate sends a timeline update to all connected peers
func (psm *P2PSyncManager) broadcastTimelineUpdate(timeline string, newHead objects.Hash) {
	// Skip broadcast if the head is empty or invalid
	if newHead.IsZero() {
		return
	}

	seal, err := psm.storage.LoadSeal(newHead)
	if err != nil {
		// Use logging instead of fmt.Printf
		// fmt.Printf("Failed to load seal for timeline update broadcast: %v\n", err)
		return
	}

	update := TimelineUpdate{
		Timeline:  timeline,
		NewHead:   newHead,
		Seal:      seal,
		Author:    seal.Author.Name,
		Timestamp: time.Now(),
		Message:   fmt.Sprintf("Timeline %s updated", timeline),
	}

	psm.p2pNetwork.BroadcastMessage(MessageTypeTimelineUpdate, update)
	fmt.Printf("Broadcasted timeline update: %s -> %s\n", timeline, newHead.String()[:8])
}

// syncWithAllPeers performs synchronization with all connected peers
func (psm *P2PSyncManager) syncWithAllPeers() {
	peers := psm.p2pNetwork.GetPeers()
	if len(peers) == 0 {
		return
	}

	fmt.Printf("Auto-syncing with %d peers...\n", len(peers))

	for _, peer := range peers {
		go psm.syncWithPeer(peer.ID)
	}
}

// syncWithPeer performs synchronization with a specific peer
func (psm *P2PSyncManager) syncWithPeer(peerID string) error {
	// Get local timeline heads
	timelines := psm.timelineManager.ListTimelines()
	localHeads := make(map[string]objects.Hash)

	for _, timeline := range timelines {
		head, err := psm.timelineManager.GetHead(timeline)
		if err != nil {
			continue
		}
		localHeads[timeline] = head
	}

	// Create sync request
	request := SyncRequest{
		Timelines:   timelines,
		LocalHeads:  localHeads,
		RequestType: "incremental",
		Timestamp:   time.Now(),
	}

	// Send sync request
	err := psm.p2pNetwork.SendMessage(peerID, MessageTypeSyncRequest, request)
	if err != nil {
		return fmt.Errorf("failed to send sync request to peer %s: %v", peerID, err)
	}

	// Update sync state
	psm.updatePeerSyncState(peerID, localHeads)

	return nil
}

// handleSyncRequest processes incoming synchronization requests
func (psm *P2PSyncManager) handleSyncRequest(peer *Peer, message *Message) error {
	var request SyncRequest
	requestData, _ := json.Marshal(message.Data)
	if err := json.Unmarshal(requestData, &request); err != nil {
		return fmt.Errorf("failed to unmarshal sync request: %v", err)
	}

	// Build sync response
	response := SyncResponse{
		Timelines:    []string{},
		RemoteHeads:  make(map[string]objects.Hash),
		MissingSeals: []objects.Hash{},
		SealsData:    []*objects.Seal{},
		TreesData:    []*objects.Tree{},
		BlobsData:    []*objects.Blob{},
		Conflicts:    []ConflictInfo{},
		Timestamp:    time.Now(),
	}

	// Check each requested timeline
	for _, timeline := range request.Timelines {
		localHead, err := psm.timelineManager.GetHead(timeline)
		if err != nil {
			// Timeline doesn't exist locally
			continue
		}

		response.Timelines = append(response.Timelines, timeline)
		response.RemoteHeads[timeline] = localHead

		// Check if peer's head is different
		if peerHead, exists := request.LocalHeads[timeline]; exists {
			if peerHead != localHead {
				// Timeline has diverged, determine what to send
				missingSeals, err := psm.findMissingSeals(peerHead, localHead)
				if err != nil {
					// Add conflict info
					response.Conflicts = append(response.Conflicts, ConflictInfo{
						Timeline:     timeline,
						LocalHead:    localHead,
						RemoteHead:   peerHead,
						ConflictType: "diverged",
					})
					continue
				}

				// Add missing seals and their dependencies
				for _, sealHash := range missingSeals {
					seal, err := psm.storage.LoadSeal(sealHash)
					if err != nil {
						continue
					}
					response.SealsData = append(response.SealsData, seal)
					response.MissingSeals = append(response.MissingSeals, sealHash)

					// Add trees and blobs referenced by this seal
					if !seal.Position.IsZero() {
						tree, err := psm.storage.LoadTree(seal.Position)
						if err == nil {
							response.TreesData = append(response.TreesData, tree)
							psm.addReferencedBlobs(tree, &response)
						}
					}
				}
			}
		} else {
			// Peer doesn't have this timeline, send recent history
			recentSeals := psm.getRecentSeals(timeline, 10)
			for _, sealHash := range recentSeals {
				seal, err := psm.storage.LoadSeal(sealHash)
				if err != nil {
					continue
				}
				response.SealsData = append(response.SealsData, seal)
			}
		}
	}

	// Send response
	err := psm.p2pNetwork.SendMessage(peer.ID, MessageTypeSyncResponse, response)
	if err != nil {
		return err
	}

	// Update sync state for this peer since we just processed a sync request from them
	localHeads := make(map[string]objects.Hash)
	for timeline := range response.RemoteHeads {
		if head, err := psm.timelineManager.GetHead(timeline); err == nil {
			localHeads[timeline] = head
		}
	}
	psm.updatePeerSyncState(peer.ID, localHeads)

	return nil
}

// handleSyncResponse processes incoming synchronization responses
func (psm *P2PSyncManager) handleSyncResponse(peer *Peer, message *Message) error {
	var response SyncResponse
	responseData, _ := json.Marshal(message.Data)
	if err := json.Unmarshal(responseData, &response); err != nil {
		return fmt.Errorf("failed to unmarshal sync response: %v", err)
	}

	fmt.Printf("Received sync response from peer %s: %d seals, %d conflicts\n",
		peer.ID, len(response.SealsData), len(response.Conflicts))

	// Store received seals
	for _, seal := range response.SealsData {
		if err := psm.storage.StoreSeal(seal); err != nil {
			fmt.Printf("Failed to store received seal: %v\n", err)
		}
	}

	// Store received trees
	for _, tree := range response.TreesData {
		if err := psm.storage.StoreTree(tree); err != nil {
			fmt.Printf("Failed to store received tree: %v\n", err)
		}
	}

	// Store received blobs
	for _, blob := range response.BlobsData {
		if err := psm.storage.StoreBlob(blob); err != nil {
			fmt.Printf("Failed to store received blob: %v\n", err)
		}
	}

	// Update timeline heads if appropriate
	for timeline, remoteHead := range response.RemoteHeads {
		localHead, err := psm.timelineManager.GetHead(timeline)
		if err != nil {
			// Timeline doesn't exist locally, create it
			if err := psm.timelineManager.Create(timeline, "Synced from peer"); err != nil {
				fmt.Printf("Failed to create timeline %s: %v\n", timeline, err)
				continue
			}
		}

		// Check if we should update to remote head
		if psm.shouldUpdateToRemoteHead(timeline, localHead, remoteHead) {
			if err := psm.timelineManager.UpdateHead(timeline, remoteHead); err != nil {
				fmt.Printf("Failed to update timeline %s head: %v\n", timeline, err)
			} else {
				fmt.Printf("Updated timeline %s head to %s\n", timeline, remoteHead.String()[:8])
			}
		}
	}

	// Handle conflicts
	for _, conflict := range response.Conflicts {
		psm.handleSyncConflict(peer.ID, conflict)
	}

	// Update peer sync state
	psm.updatePeerSyncStateFromResponse(peer.ID, &response)

	return nil
}

// handleTimelineUpdate processes real-time timeline updates from peers
func (psm *P2PSyncManager) handleTimelineUpdate(peer *Peer, message *Message) error {
	var update TimelineUpdate
	updateData, _ := json.Marshal(message.Data)
	if err := json.Unmarshal(updateData, &update); err != nil {
		return fmt.Errorf("failed to unmarshal timeline update: %v", err)
	}

	fmt.Printf("Received timeline update from peer %s: %s -> %s\n",
		peer.ID, update.Timeline, update.NewHead.String()[:8])

	// Store the new seal
	if update.Seal != nil {
		if err := psm.storage.StoreSeal(update.Seal); err != nil {
			fmt.Printf("Failed to store updated seal: %v\n", err)
			return err
		}
	}

	// Check if we should update our timeline
	localHead, err := psm.timelineManager.GetHead(update.Timeline)
	if err != nil {
		// Timeline doesn't exist locally, create it
		if err := psm.timelineManager.Create(update.Timeline, "Synced from peer"); err != nil {
			return fmt.Errorf("failed to create timeline: %v", err)
		}
		localHead = objects.Hash{} // Empty hash for new timeline
	}

	// Update if the remote head is newer or we don't have the timeline
	if localHead.IsZero() || psm.shouldUpdateToRemoteHead(update.Timeline, localHead, update.NewHead) {
		// Request missing data before updating
		go psm.requestMissingData(peer.ID, update.Timeline, localHead, update.NewHead)
	}

	return nil
}

// requestMissingData requests missing seals/trees/blobs needed for a timeline update
func (psm *P2PSyncManager) requestMissingData(peerID string, timeline string, localHead, remoteHead objects.Hash) {
	// Create a targeted sync request for this specific timeline
	request := SyncRequest{
		Timelines:   []string{timeline},
		LocalHeads:  map[string]objects.Hash{timeline: localHead},
		RequestType: "incremental",
		Timestamp:   time.Now(),
	}

	psm.p2pNetwork.SendMessage(peerID, MessageTypeSyncRequest, request)
}

// Utility functions

// findMissingSeals finds seals that need to be sent to bring peer from oldHead to newHead
func (psm *P2PSyncManager) findMissingSeals(oldHead, newHead objects.Hash) ([]objects.Hash, error) {
	// Simple implementation: return the new head
	// In a real implementation, this would traverse the seal chain
	if oldHead.IsZero() {
		return []objects.Hash{newHead}, nil
	}

	// TODO: Implement proper ancestry traversal
	return []objects.Hash{newHead}, nil
}

// getRecentSeals returns recent seals for a timeline
func (psm *P2PSyncManager) getRecentSeals(timeline string, count int) []objects.Hash {
	// Simple implementation: return head seal
	head, err := psm.timelineManager.GetHead(timeline)
	if err != nil {
		return []objects.Hash{}
	}
	return []objects.Hash{head}
}

// addReferencedBlobs adds blobs referenced by a tree to the sync response
func (psm *P2PSyncManager) addReferencedBlobs(tree *objects.Tree, response *SyncResponse) {
	for _, entry := range tree.Entries {
		if entry.Type == objects.ObjectTypeBlob {
			blob, err := psm.storage.LoadBlob(entry.Hash)
			if err == nil {
				response.BlobsData = append(response.BlobsData, blob)
			}
		} else if entry.Type == objects.ObjectTypeTree {
			subtree, err := psm.storage.LoadTree(entry.Hash)
			if err == nil {
				response.TreesData = append(response.TreesData, subtree)
				psm.addReferencedBlobs(subtree, response)
			}
		}
	}
}

// shouldUpdateToRemoteHead determines if we should update to a remote head
func (psm *P2PSyncManager) shouldUpdateToRemoteHead(timeline string, localHead, remoteHead objects.Hash) bool {
	// Simple strategy: update if remote head is different and we have the seal
	if localHead == remoteHead {
		return false
	}

	return psm.storage.HasObject(remoteHead)
}

// handleSyncConflict handles synchronization conflicts
func (psm *P2PSyncManager) handleSyncConflict(peerID string, conflict ConflictInfo) {
	fmt.Printf("Sync conflict with peer %s on timeline %s: %s\n",
		peerID, conflict.Timeline, conflict.ConflictType)

	// Update conflict count in peer sync state
	psm.syncMutex.Lock()
	if state, exists := psm.syncState[peerID]; exists {
		state.ConflictCount++
	}
	psm.syncMutex.Unlock()

	// TODO: Implement conflict resolution strategies
}

// updatePeerSyncState updates synchronization state for a peer
func (psm *P2PSyncManager) updatePeerSyncState(peerID string, localHeads map[string]objects.Hash) {
	psm.syncMutex.Lock()
	defer psm.syncMutex.Unlock()

	if state, exists := psm.syncState[peerID]; exists {
		state.LastSync = time.Now()
		state.TimelineHeads = localHeads
	} else {
		psm.syncState[peerID] = &PeerSyncState{
			PeerID:          peerID,
			LastSync:        time.Now(),
			TimelineHeads:   localHeads,
			SyncedSeals:     make(map[string]time.Time),
			AutoSyncEnabled: true,
		}
	}
}

// updatePeerSyncStateFromResponse updates sync state based on sync response
func (psm *P2PSyncManager) updatePeerSyncStateFromResponse(peerID string, response *SyncResponse) {
	psm.syncMutex.Lock()
	defer psm.syncMutex.Unlock()

	if state, exists := psm.syncState[peerID]; exists {
		state.LastSync = time.Now()
		state.BytesTransferred += int64(len(response.SealsData) * 1024) // Rough estimate

		// Update synced seals
		for _, seal := range response.SealsData {
			state.SyncedSeals[seal.Name] = time.Now()
		}
	}
}

// GetPeerSyncState returns synchronization state for a peer
func (psm *P2PSyncManager) GetPeerSyncState(peerID string) *PeerSyncState {
	psm.syncMutex.RLock()
	defer psm.syncMutex.RUnlock()

	if state, exists := psm.syncState[peerID]; exists {
		return state
	}
	return nil
}

// GetAllPeerSyncStates returns synchronization state for all peers
func (psm *P2PSyncManager) GetAllPeerSyncStates() map[string]*PeerSyncState {
	psm.syncMutex.RLock()
	defer psm.syncMutex.RUnlock()

	states := make(map[string]*PeerSyncState)
	for peerID, state := range psm.syncState {
		states[peerID] = state
	}
	return states
}
