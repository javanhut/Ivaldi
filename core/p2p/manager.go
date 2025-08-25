package p2p

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"

	"ivaldi/core/objects"
)

// P2PManager coordinates all P2P functionality
type P2PManager struct {
	network         *P2PNetwork
	syncManager     *P2PSyncManager
	discovery       *DiscoveryService
	configManager   *P2PConfigManager
	stateManager    *P2PStateManager
	eventBus        *EventBus
	storage         Storage
	timelineManager TimelineManager
	rootDir         string
	running         bool
	mutex           sync.RWMutex
}

// NewP2PManager creates a new P2P manager
func NewP2PManager(rootDir string, storage Storage, timelineManager TimelineManager) (*P2PManager, error) {
	configManager := NewP2PConfigManager(rootDir)
	config, err := configManager.Load()
	if err != nil {
		return nil, fmt.Errorf("failed to load P2P config: %v", err)
	}

	// Create event bus
	eventBus := NewEventBus()

	// Create P2P network with sync callback (syncManager will be set later)
	var syncManager *P2PSyncManager

	network, err := NewP2PNetwork(config.Port, eventBus, func(peerID string, event SyncEvent, data interface{}) error {
		// Handle sync events and publish to event bus
		switch event {
		case SyncEventPeerJoined:
			if peer, ok := data.(*Peer); ok {
				eventBus.PublishPeerConnected(peerID, peer)
			}
		case SyncEventPeerLeft:
			if peer, ok := data.(*Peer); ok {
				eventBus.PublishPeerDisconnected(peerID, peer)
			}
		case SyncEventTimelineUpdate:
			if update, ok := data.(*TimelineUpdate); ok {
				eventBus.PublishTimelineUpdated(update.Timeline, update)
			}
		case SyncEventConflict:
			if conflict, ok := data.(ConflictInfo); ok {
				eventBus.PublishConflictDetected(peerID, conflict)
			}
		case SyncEventSyncRequest:
			if ctx, ok := data.(*SyncRequestContext); ok && syncManager != nil {
				return syncManager.handleSyncRequest(ctx.Peer, ctx.Message)
			}
		case SyncEventSyncResponse:
			if ctx, ok := data.(*SyncResponseContext); ok && syncManager != nil {
				return syncManager.handleSyncResponse(ctx.Peer, ctx.Message)
			}
		}
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("failed to create P2P network: %v", err)
	}

	// Create sync manager
	syncManager = NewP2PSyncManager(network, storage, timelineManager)
	syncManager.EnableAutoSync(config.AutoSyncEnabled)
	syncManager.SetSyncInterval(config.SyncInterval)

	// Create discovery service
	discovery := NewDiscoveryService(network, config.DiscoveryPort)

	// Set up message handlers (subscription IDs not stored as these are internal handlers)
	eventBus.Subscribe(EventTypePeerConnected, func(event Event) error {
		fmt.Printf("Peer connected: %s\n", event.Source)
		return nil
	})

	eventBus.Subscribe(EventTypePeerDisconnected, func(event Event) error {
		fmt.Printf("Peer disconnected: %s\n", event.Source)
		return nil
	})

	// Create state manager and check if P2P is already running
	stateManager := NewP2PStateManager(rootDir)

	pm := &P2PManager{
		network:         network,
		syncManager:     syncManager,
		discovery:       discovery,
		configManager:   configManager,
		stateManager:    stateManager,
		eventBus:        eventBus,
		storage:         storage,
		timelineManager: timelineManager,
		rootDir:         rootDir,
		running:         false,
	}

	// Check if P2P is already running from a previous session
	if state, ok := stateManager.GetRunningState(); ok {
		pm.running = true
		// Note: The actual network connections would need to be re-established
		// This just indicates that P2P was started in a previous command
		fmt.Printf("P2P network already running (NodeID: %s, Port: %d)\n", state.NodeID, state.Port)
	}

	return pm, nil
}

// Start begins all P2P services
func (pm *P2PManager) Start() error {
	pm.mutex.Lock()
	defer pm.mutex.Unlock()

	if pm.running {
		return fmt.Errorf("P2P manager is already running")
	}

	// Start event bus
	pm.eventBus.Start()

	// Start P2P network
	if err := pm.network.Start(); err != nil {
		return fmt.Errorf("failed to start P2P network: %v", err)
	}

	// Start sync manager
	if err := pm.syncManager.Start(); err != nil {
		pm.network.Stop()
		return fmt.Errorf("failed to start sync manager: %v", err)
	}

	// Start discovery service
	repositories := pm.getRepositoryList()
	if err := pm.discovery.Start(repositories); err != nil {
		pm.syncManager = nil // Stop sync manager
		pm.network.Stop()
		return fmt.Errorf("failed to start discovery service: %v", err)
	}

	// Connect to known peers
	config := pm.configManager.Get()
	for _, peerAddr := range config.KnownPeers {
		go pm.connectToKnownPeer(peerAddr)
	}

	pm.running = true

	// Save state to disk
	state := &P2PState{
		Running:       true,
		NodeID:        pm.network.GetNodeID(),
		Port:          pm.network.port,
		DiscoveryPort: pm.discovery.broadcastPort,
		StartedAt:     time.Now(),
		PID:           os.Getpid(),
	}
	if err := pm.stateManager.Save(state); err != nil {
		fmt.Printf("Warning: Failed to save P2P state: %v\n", err)
	}

	fmt.Println("P2P manager started successfully")
	return nil
}

// Stop shuts down all P2P services
func (pm *P2PManager) Stop() error {
	pm.mutex.Lock()
	defer pm.mutex.Unlock()

	if !pm.running {
		return nil
	}

	// Stop services in reverse order
	if pm.discovery != nil {
		pm.discovery.Stop()
	}

	if pm.network != nil {
		pm.network.Stop()
	}

	if pm.eventBus != nil {
		pm.eventBus.Stop()
	}

	pm.running = false

	// Clear state from disk
	if err := pm.stateManager.Clear(); err != nil {
		fmt.Printf("Warning: Failed to clear P2P state: %v\n", err)
	}

	fmt.Println("P2P manager stopped")
	return nil
}

// IsRunning returns whether the P2P manager is currently running
func (pm *P2PManager) IsRunning() bool {
	pm.mutex.RLock()
	defer pm.mutex.RUnlock()

	// First check in-memory state
	if pm.running {
		return true
	}

	// Then check persistent state
	return pm.stateManager.IsRunning()
}

// GetPeers returns all connected peers
func (pm *P2PManager) GetPeers() []*Peer {
	if pm.network == nil {
		return []*Peer{}
	}
	return pm.network.GetPeers()
}

// GetDiscoveredPeers returns all discovered peers
func (pm *P2PManager) GetDiscoveredPeers() []*DiscoveredPeer {
	if pm.discovery == nil {
		return []*DiscoveredPeer{}
	}
	return pm.discovery.GetDiscoveredPeers()
}

// ConnectToPeer connects to a specific peer
func (pm *P2PManager) ConnectToPeer(address string, port int) error {
	if pm.network == nil {
		return fmt.Errorf("P2P network not started")
	}

	err := pm.network.ConnectToPeer(address, port)
	if err != nil {
		return err
	}

	// Add to known peers
	peerAddr := fmt.Sprintf("%s:%d", address, port)
	pm.configManager.AddKnownPeer(peerAddr)

	return nil
}

// DisconnectFromPeer disconnects from a specific peer
func (pm *P2PManager) DisconnectFromPeer(peerID string) error {
	// Implementation would remove peer from active connections
	// For now, we'll just remove from known peers
	peers := pm.GetPeers()
	for _, peer := range peers {
		if peer.ID == peerID {
			peerAddr := fmt.Sprintf("%s:%d", peer.Address, peer.Port)
			return pm.configManager.RemoveKnownPeer(peerAddr)
		}
	}
	return fmt.Errorf("peer not found: %s", peerID)
}

// SyncWithPeer performs manual synchronization with a specific peer
func (pm *P2PManager) SyncWithPeer(peerID string) error {
	if pm.syncManager == nil {
		return fmt.Errorf("sync manager not started")
	}

	return pm.syncManager.syncWithPeer(peerID)
}

// SyncWithAllPeers performs synchronization with all connected peers
func (pm *P2PManager) SyncWithAllPeers() error {
	if pm.syncManager == nil {
		return fmt.Errorf("sync manager not started")
	}

	pm.syncManager.syncWithAllPeers()
	return nil
}

// GetSyncState returns synchronization state for all peers
func (pm *P2PManager) GetSyncState() map[string]*PeerSyncState {
	if pm.syncManager == nil {
		return make(map[string]*PeerSyncState)
	}

	return pm.syncManager.GetAllPeerSyncStates()
}

// EnableAutoSync enables or disables automatic synchronization
func (pm *P2PManager) EnableAutoSync(enabled bool) error {
	if pm.syncManager != nil {
		pm.syncManager.EnableAutoSync(enabled)
	}

	return pm.configManager.SetAutoSync(enabled)
}

// SetSyncInterval sets the synchronization interval
func (pm *P2PManager) SetSyncInterval(interval time.Duration) error {
	if pm.syncManager != nil {
		pm.syncManager.SetSyncInterval(interval)
	}

	return pm.configManager.UpdateSyncInterval(interval)
}

// GetConfig returns the current P2P configuration
func (pm *P2PManager) GetConfig() *P2PConfig {
	return pm.configManager.Get()
}

// UpdateConfig updates the P2P configuration
func (pm *P2PManager) UpdateConfig(config *P2PConfig) error {
	if err := pm.configManager.ValidateConfig(config); err != nil {
		return err
	}

	return pm.configManager.Save(config)
}

// GetStatus returns current P2P status information
func (pm *P2PManager) GetStatus() *P2PStatus {
	status := &P2PStatus{
		Running:         pm.IsRunning(),
		ConnectedPeers:  len(pm.GetPeers()),
		DiscoveredPeers: len(pm.GetDiscoveredPeers()),
		AutoSyncEnabled: pm.GetConfig().AutoSyncEnabled,
		SyncInterval:    pm.GetConfig().SyncInterval,
	}

	if pm.network != nil {
		status.NodeID = pm.network.GetNodeID()
		status.Port = pm.network.port
	}

	// Add sync statistics
	syncStates := pm.GetSyncState()
	for _, state := range syncStates {
		status.TotalSyncs += state.BytesTransferred
		status.ConflictCount += int64(state.ConflictCount)
	}

	return status
}

// P2PStatus contains current status information
type P2PStatus struct {
	Running         bool          `json:"running"`
	NodeID          string        `json:"node_id"`
	Port            int           `json:"port"`
	ConnectedPeers  int           `json:"connected_peers"`
	DiscoveredPeers int           `json:"discovered_peers"`
	AutoSyncEnabled bool          `json:"auto_sync_enabled"`
	SyncInterval    time.Duration `json:"sync_interval"`
	TotalSyncs      int64         `json:"total_syncs"`
	ConflictCount   int64         `json:"conflict_count"`
}

// FindPeersWithRepository finds peers that have a specific repository
func (pm *P2PManager) FindPeersWithRepository(repoName string) []*DiscoveredPeer {
	if pm.discovery == nil {
		return []*DiscoveredPeer{}
	}

	return pm.discovery.FindPeersWithRepository(repoName)
}

// BroadcastTimelineUpdate sends a timeline update to all peers
func (pm *P2PManager) BroadcastTimelineUpdate(timeline string, newHead objects.Hash) error {
	if pm.syncManager == nil {
		return fmt.Errorf("sync manager not started")
	}

	pm.syncManager.broadcastTimelineUpdate(timeline, newHead)
	return nil
}

// Subscribe to P2P events and return a subscription handle
func (pm *P2PManager) Subscribe(eventType string, handler EventHandler) SubscriptionID {
	if pm.eventBus != nil {
		return pm.eventBus.Subscribe(eventType, handler)
	}
	return 0 // Invalid subscription ID when no event bus
}

// Unsubscribe from P2P events using the subscription handle
func (pm *P2PManager) Unsubscribe(eventType string, subID SubscriptionID) {
	if pm.eventBus != nil {
		pm.eventBus.Unsubscribe(eventType, subID)
	}
}

// SendMessage sends a message to a specific peer
func (pm *P2PManager) SendMessage(peerID string, msgType MessageType, data interface{}) error {
	if pm.network == nil {
		return fmt.Errorf("P2P network not started")
	}
	return pm.network.SendMessage(peerID, msgType, data)
}

// BroadcastMessage sends a message to all connected peers
func (pm *P2PManager) BroadcastMessage(msgType MessageType, data interface{}) error {
	if pm.network == nil {
		return fmt.Errorf("P2P network not started")
	}
	return pm.network.BroadcastMessage(msgType, data)
}

// Helper functions

// connectToKnownPeer attempts to connect to a known peer
func (pm *P2PManager) connectToKnownPeer(peerAddr string) {
	// Parse address and port
	var address string
	var port int
	if _, err := fmt.Sscanf(peerAddr, "%s:%d", &address, &port); err != nil {
		return
	}

	// Attempt connection
	err := pm.network.ConnectToPeer(address, port)
	if err != nil {
		fmt.Printf("Failed to connect to known peer %s: %v\n", peerAddr, err)
	}
}

// getRepositoryList returns list of repositories for discovery announcements
func (pm *P2PManager) getRepositoryList() []string {
	// Get current repository name from root directory
	repoName := filepath.Base(pm.rootDir)
	if repoName == "." || repoName == "/" {
		repoName = "unnamed"
	}

	return []string{repoName}
}

// Sync context types for passing to sync callback
type SyncRequestContext struct {
	Peer    *Peer
	Message *Message
}

type SyncResponseContext struct {
	Peer    *Peer
	Message *Message
}

// P2P message handlers (called by network layer)
