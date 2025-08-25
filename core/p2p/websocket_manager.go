package p2p

import (
	"fmt"
	"sync"
	"time"

	"ivaldi/core/objects"
)

// WebSocketP2PManager manages P2P networking through Carrion WebSocket servers
type WebSocketP2PManager struct {
	bridge    *WebSocketBridge
	nodeID    string
	port      int
	rootDir   string
	running   bool
	stateMgr  *P2PStateManager
	mutex     sync.RWMutex
	peers     map[string]*Peer
	peerMutex sync.RWMutex
}

// NewWebSocketP2PManager creates a new WebSocket-based P2P manager
func NewWebSocketP2PManager(rootDir string, port int) (*WebSocketP2PManager, error) {
	nodeID, err := generateNodeID()
	if err != nil {
		return nil, fmt.Errorf("failed to generate node ID: %v", err)
	}
	bridge := NewWebSocketBridge(port, nodeID)
	stateMgr := NewP2PStateManager(rootDir)

	return &WebSocketP2PManager{
		bridge:   bridge,
		nodeID:   nodeID,
		port:     port,
		rootDir:  rootDir,
		running:  false,
		stateMgr: stateMgr,
		peers:    make(map[string]*Peer),
	}, nil
}

// Start begins the WebSocket P2P networking
func (wpm *WebSocketP2PManager) Start() error {
	wpm.mutex.Lock()
	defer wpm.mutex.Unlock()

	if wpm.running {
		return fmt.Errorf("WebSocket P2P manager is already running")
	}

	// Start the Carrion WebSocket bridge
	if err := wpm.bridge.Start(); err != nil {
		return fmt.Errorf("failed to start WebSocket bridge: %v", err)
	}

	wpm.running = true

	// Save state
	state := &P2PState{
		Running:       true,
		NodeID:        wpm.nodeID,
		Port:          wpm.port,
		DiscoveryPort: wpm.port + 1, // Use next port for compatibility
		StartedAt:     time.Now(),
		PID:           0, // WebSocket bridge handles process management
	}

	if err := wpm.stateMgr.Save(state); err != nil {
		fmt.Printf("Warning: failed to save P2P state: %v\n", err)
	}

	// Start peer discovery
	go wpm.startPeerDiscovery()

	fmt.Printf("WebSocket P2P manager started successfully\n")
	fmt.Printf("Node ID: %s, Port: %d\n", wpm.nodeID, wpm.port)

	return nil
}

// Stop shuts down the WebSocket P2P networking
func (wpm *WebSocketP2PManager) Stop() error {
	wpm.mutex.Lock()
	defer wpm.mutex.Unlock()

	if !wpm.running {
		return nil
	}

	// Stop the WebSocket bridge
	if err := wpm.bridge.Stop(); err != nil {
		fmt.Printf("Warning: error stopping WebSocket bridge: %v\n", err)
	}

	wpm.running = false

	// Clear state
	if err := wpm.stateMgr.Clear(); err != nil {
		fmt.Printf("Warning: failed to clear P2P state: %v\n", err)
	}

	// Clear peers
	wpm.peerMutex.Lock()
	wpm.peers = make(map[string]*Peer)
	wpm.peerMutex.Unlock()

	fmt.Printf("WebSocket P2P manager stopped\n")
	return nil
}

// IsRunning returns whether the P2P manager is active
func (wpm *WebSocketP2PManager) IsRunning() bool {
	wpm.mutex.RLock()
	defer wpm.mutex.RUnlock()

	if !wpm.running {
		return false
	}

	// Verify the bridge is actually running
	return wpm.bridge.IsRunning()
}

// GetStatus returns current P2P status
func (wpm *WebSocketP2PManager) GetStatus() *P2PStatus {
	wpm.mutex.RLock()
	defer wpm.mutex.RUnlock()

	status := &P2PStatus{
		Running:         wpm.running && wpm.bridge.IsRunning(),
		NodeID:          wpm.nodeID,
		Port:            wpm.port,
		ConnectedPeers:  len(wpm.peers),
		DiscoveredPeers: 0, // Will be updated by discovery
	}

	return status
}

// ConnectToPeer connects to a specific peer
func (wpm *WebSocketP2PManager) ConnectToPeer(address string, port int) error {
	if !wpm.IsRunning() {
		return fmt.Errorf("WebSocket P2P manager is not running")
	}

	// Use the bridge to connect to the peer
	if err := wpm.bridge.ConnectToPeer(address, port); err != nil {
		return fmt.Errorf("failed to connect to peer %s:%d: %v", address, port, err)
	}

	// Add peer to our local list
	peerID := fmt.Sprintf("%s:%d", address, port)
	peer := &Peer{
		ID:        peerID,
		Address:   address,
		Port:      port,
		Status:   PeerStatusConnected,
		LastSeen:  time.Now(),
	}

	wpm.peerMutex.Lock()
	wpm.peers[peerID] = peer
	wpm.peerMutex.Unlock()

	fmt.Printf("Successfully connected to peer %s:%d\n", address, port)
	return nil
}

// GetPeers returns a list of connected peers
func (wpm *WebSocketP2PManager) GetPeers() []*Peer {
	wpm.peerMutex.RLock()
	defer wpm.peerMutex.RUnlock()

	peers := make([]*Peer, 0, len(wpm.peers))
	for _, peer := range wpm.peers {
		peerCopy := *peer
		peers = append(peers, &peerCopy)
	}

	return peers
}

// SyncWithPeer synchronizes data with a specific peer
func (wpm *WebSocketP2PManager) SyncWithPeer(peerID string) error {
	if !wpm.IsRunning() {
		return fmt.Errorf("WebSocket P2P manager is not running")
	}

	// Use the bridge to sync with the peer
	return wpm.bridge.SyncWithPeer(peerID, "repository", "all")
}

// SendMessage sends a message to a specific peer
func (wpm *WebSocketP2PManager) SendMessage(peerID string, msgType MessageType, data interface{}) error {
	if !wpm.IsRunning() {
		return fmt.Errorf("WebSocket P2P manager is not running")
	}

	// Convert data to map[string]interface{} for JSON marshaling
	dataMap := make(map[string]interface{})
	dataMap["message_type"] = msgType
	dataMap["data"] = data
	dataMap["from_peer"] = wpm.nodeID
	dataMap["to_peer"] = peerID

	// Use the bridge to send message to specific peer
	_, err := wpm.bridge.SendMessage(peerID, dataMap)
	return err
}

// BroadcastMessage sends a message to all connected peers
func (wpm *WebSocketP2PManager) BroadcastMessage(msgType MessageType, data interface{}) error {
	if !wpm.IsRunning() {
		return fmt.Errorf("WebSocket P2P manager is not running")
	}

	// Convert data to map[string]interface{} for JSON marshaling
	dataMap := make(map[string]interface{})
	dataMap["message_type"] = msgType
	dataMap["data"] = data
	dataMap["from_peer"] = wpm.nodeID

	// Use topology update as a broadcast mechanism
	return wpm.bridge.BroadcastTopologyUpdate(dataMap)
}

// startPeerDiscovery runs peer discovery in a background goroutine
func (wpm *WebSocketP2PManager) startPeerDiscovery() {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()

	for wpm.IsRunning() {
		select {
		case <-ticker.C:
			if err := wpm.bridge.DiscoverPeers(); err != nil {
				fmt.Printf("Peer discovery failed: %v\n", err)
			}
		default:
			time.Sleep(1 * time.Second)
		}
	}
}

// Subscribe is a compatibility method for event handling (placeholder)
func (wpm *WebSocketP2PManager) Subscribe(eventType string, handler EventHandler) {
	// WebSocket-based P2P uses a different event model
	// This is a placeholder for compatibility with existing mesh code
	fmt.Printf("WebSocket P2P: Event subscription for %v (placeholder)\n", eventType)
}

// GetConfig returns the P2P configuration
func (wpm *WebSocketP2PManager) GetConfig() *P2PConfig {
	return &P2PConfig{
		Port:              wpm.port,
		DiscoveryPort:     wpm.port + 1,
		MaxPeers:          50,
		EnableAutoConnect: true,
		KnownPeers:        []string{},
		AutoSyncEnabled:   true,
		SyncInterval:      30 * time.Second,
		SyncTimeout:       60 * time.Second,
		ConflictStrategy:  "manual",
		MaxConcurrentSync: 5,
	}
}

// EnableAutoSync enables/disables automatic synchronization
func (wpm *WebSocketP2PManager) EnableAutoSync(enabled bool) error {
	// WebSocket P2P handles sync differently
	fmt.Printf("WebSocket P2P: Auto-sync %v (always enabled)\n", enabled)
	return nil
}

// SetSyncInterval sets the synchronization interval
func (wpm *WebSocketP2PManager) SetSyncInterval(interval time.Duration) error {
	// WebSocket P2P uses fixed intervals for now
	fmt.Printf("WebSocket P2P: Sync interval set to %v (placeholder)\n", interval)
	return nil
}

// GetDiscoveredPeers returns discovered peers (placeholder for WebSocket P2P)
func (wpm *WebSocketP2PManager) GetDiscoveredPeers() []*DiscoveredPeer {
	// WebSocket P2P uses a different discovery model
	// Convert connected peers to discovered peer format
	discovered := make([]*DiscoveredPeer, 0, len(wpm.peers))
	wpm.peerMutex.RLock()
	defer wpm.peerMutex.RUnlock()
	
	for _, peer := range wpm.peers {
		discovered = append(discovered, &DiscoveredPeer{
			NodeID:       peer.ID,
			Address:      peer.Address,
			Port:         peer.Port,
			LastSeen:     peer.LastSeen,
			Repositories: []string{"unknown"}, // Placeholder
			Version:      "websocket-p2p",
		})
	}
	
	return discovered
}

// FindPeersWithRepository finds peers that have a specific repository
func (wpm *WebSocketP2PManager) FindPeersWithRepository(repoName string) []*DiscoveredPeer {
	// For WebSocket P2P, return all discovered peers for now
	return wpm.GetDiscoveredPeers()
}

// GetSyncState returns synchronization state for all peers
func (wpm *WebSocketP2PManager) GetSyncState() map[string]*PeerSyncState {
	syncState := make(map[string]*PeerSyncState)
	wpm.peerMutex.RLock()
	defer wpm.peerMutex.RUnlock()
	
	for id, peer := range wpm.peers {
		syncState[id] = &PeerSyncState{
			PeerID:           peer.ID,
			LastSync:         peer.LastSeen,
			TimelineHeads:    make(map[string]objects.Hash), // Placeholder
			SyncedSeals:      make(map[string]time.Time),
			ConflictCount:    0,
			BytesTransferred: 0,
			AutoSyncEnabled:  true,
		}
	}
	
	return syncState
}

// SyncWithAllPeers performs synchronization with all connected peers
func (wpm *WebSocketP2PManager) SyncWithAllPeers() error {
	if !wpm.IsRunning() {
		return fmt.Errorf("WebSocket P2P manager is not running")
	}
	
	wpm.peerMutex.RLock()
	peerIDs := make([]string, 0, len(wpm.peers))
	for id := range wpm.peers {
		peerIDs = append(peerIDs, id)
	}
	wpm.peerMutex.RUnlock()
	
	// Sync with each peer
	for _, peerID := range peerIDs {
		if err := wpm.SyncWithPeer(peerID); err != nil {
			fmt.Printf("Failed to sync with peer %s: %v\n", peerID, err)
		}
	}
	
	return nil
}