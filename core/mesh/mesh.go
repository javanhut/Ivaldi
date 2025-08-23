package mesh

import (
	"context"
	"fmt"
	"sync"
	"time"

	"ivaldi/core/p2p"
)

// MeshNetwork provides true mesh networking capabilities on top of P2P
type MeshNetwork struct {
	p2pManager   *p2p.P2PManager
	nodeID       string
	topology     map[string]*MeshPeer
	routes       map[string][]string
	topologyMutex sync.RWMutex
	routesMutex   sync.RWMutex
	ctx          context.Context
	cancel       context.CancelFunc
	running      bool
	runMutex     sync.Mutex
	
	// Mesh configuration
	maxHops           int
	gossipInterval    time.Duration
	healingInterval   time.Duration
	topologyTTL       time.Duration
	
	// Event handlers
	onPeerJoin        func(peerID string)
	onPeerLeave       func(peerID string)
	onTopologyChange  func()
}

// MeshPeer represents a peer in the mesh network
type MeshPeer struct {
	ID            string              `json:"id"`
	Address       string              `json:"address"`
	Port          int                 `json:"port"`
	LastSeen      time.Time           `json:"last_seen"`
	DirectConnect bool                `json:"direct_connect"`
	Hops          int                 `json:"hops"`
	NextHop       string              `json:"next_hop,omitempty"`
	Peers         map[string]time.Time `json:"peers"` // Peers this node knows about
	Version       string              `json:"version"`
	Capabilities  []string            `json:"capabilities"`
}

// MeshTopologyUpdate represents a topology update message
type MeshTopologyUpdate struct {
	FromPeer    string                   `json:"from_peer"`
	Timestamp   time.Time                `json:"timestamp"`
	Topology    map[string]*MeshPeer     `json:"topology"`
	TTL         int                      `json:"ttl"`
}

// MeshRoute represents a routing path through the mesh
type MeshRoute struct {
	Target      string   `json:"target"`
	Path        []string `json:"path"`
	Hops        int      `json:"hops"`
	LastUpdate  time.Time `json:"last_update"`
}

// MeshMessage wraps messages for mesh routing
type MeshMessage struct {
	MessageID   string                 `json:"message_id"`
	OriginalSender string             `json:"original_sender"`
	FinalTarget    string             `json:"final_target"`
	CurrentHop     int                `json:"current_hop"`
	MaxHops        int                `json:"max_hops"`
	MessageType    string             `json:"message_type"`
	Payload        interface{}        `json:"payload"`
	Timestamp      time.Time          `json:"timestamp"`
	Route          []string           `json:"route"`
}

// NewMeshNetwork creates a new mesh network instance
func NewMeshNetwork(p2pManager *p2p.P2PManager) *MeshNetwork {
	ctx, cancel := context.WithCancel(context.Background())
	
	return &MeshNetwork{
		p2pManager:      p2pManager,
		nodeID:          p2pManager.GetStatus().NodeID,
		topology:        make(map[string]*MeshPeer),
		routes:          make(map[string][]string),
		ctx:             ctx,
		cancel:          cancel,
		maxHops:         5,
		gossipInterval:  30 * time.Second,
		healingInterval: 60 * time.Second,
		topologyTTL:     5 * time.Minute,
	}
}

// Start begins the mesh networking layer
func (mn *MeshNetwork) Start() error {
	mn.runMutex.Lock()
	defer mn.runMutex.Unlock()
	
	if mn.running {
		return fmt.Errorf("mesh network is already running")
	}
	
	// Ensure P2P is running
	if !mn.p2pManager.IsRunning() {
		if err := mn.p2pManager.Start(); err != nil {
			return fmt.Errorf("failed to start underlying P2P network: %v", err)
		}
	}
	
	// Subscribe to P2P events
	mn.p2pManager.Subscribe(p2p.EventTypePeerConnected, mn.handlePeerConnected)
	mn.p2pManager.Subscribe(p2p.EventTypePeerDisconnected, mn.handlePeerDisconnected)
	
	// Add ourselves to topology
	mn.addSelfToTopology()
	
	// Start mesh services
	go mn.topologyGossipService()
	go mn.routeMaintenanceService()
	go mn.networkHealingService()
	go mn.topologyCleanupService()
	
	mn.running = true
	fmt.Println("Mesh network started successfully")
	return nil
}

// Stop shuts down the mesh networking layer
func (mn *MeshNetwork) Stop() error {
	mn.runMutex.Lock()
	defer mn.runMutex.Unlock()
	
	if !mn.running {
		return nil
	}
	
	mn.cancel()
	mn.running = false
	fmt.Println("Mesh network stopped")
	return nil
}

// JoinMesh connects to a bootstrap peer and joins the mesh network
func (mn *MeshNetwork) JoinMesh(bootstrapAddress string, bootstrapPort int) error {
	// Connect to bootstrap peer via P2P
	if err := mn.p2pManager.ConnectToPeer(bootstrapAddress, bootstrapPort); err != nil {
		return fmt.Errorf("failed to connect to bootstrap peer: %v", err)
	}
	
	// Request full topology from bootstrap peer
	if err := mn.requestTopologyFromPeer(fmt.Sprintf("%s:%d", bootstrapAddress, bootstrapPort)); err != nil {
		fmt.Printf("Warning: failed to get topology from bootstrap peer: %v\n", err)
	}
	
	fmt.Printf("Successfully joined mesh network via %s:%d\n", bootstrapAddress, bootstrapPort)
	return nil
}

// GetTopology returns the current network topology
func (mn *MeshNetwork) GetTopology() map[string]*MeshPeer {
	mn.topologyMutex.RLock()
	defer mn.topologyMutex.RUnlock()
	
	topology := make(map[string]*MeshPeer)
	for id, peer := range mn.topology {
		peerCopy := *peer
		topology[id] = &peerCopy
	}
	return topology
}

// GetRoute returns the best route to a target peer
func (mn *MeshNetwork) GetRoute(targetPeerID string) []string {
	mn.routesMutex.RLock()
	defer mn.routesMutex.RUnlock()
	
	if route, exists := mn.routes[targetPeerID]; exists {
		routeCopy := make([]string, len(route))
		copy(routeCopy, route)
		return routeCopy
	}
	return nil
}

// SendMeshMessage sends a message through the mesh network
func (mn *MeshNetwork) SendMeshMessage(targetPeerID string, messageType string, payload interface{}) error {
	message := &MeshMessage{
		MessageID:      generateMessageID(),
		OriginalSender: mn.nodeID,
		FinalTarget:    targetPeerID,
		CurrentHop:     0,
		MaxHops:        mn.maxHops,
		MessageType:    messageType,
		Payload:        payload,
		Timestamp:      time.Now(),
		Route:          []string{mn.nodeID},
	}
	
	return mn.routeMessage(message)
}

// addSelfToTopology adds this node to the topology
func (mn *MeshNetwork) addSelfToTopology() {
	mn.topologyMutex.Lock()
	defer mn.topologyMutex.Unlock()
	
	status := mn.p2pManager.GetStatus()
	mn.topology[mn.nodeID] = &MeshPeer{
		ID:            mn.nodeID,
		Address:       "localhost", // We don't know our external address
		Port:          status.Port,
		LastSeen:      time.Now(),
		DirectConnect: true,
		Hops:          0,
		Peers:         make(map[string]time.Time),
		Version:       "1.0",
		Capabilities:  []string{"sync", "mesh", "routing"},
	}
}

// handlePeerConnected handles P2P peer connection events
func (mn *MeshNetwork) handlePeerConnected(event p2p.Event) error {
	if peer, ok := event.Data.(*p2p.Peer); ok {
		mn.addDirectPeer(peer)
		
		// Request topology from new peer
		go func() {
			time.Sleep(1 * time.Second) // Wait for connection to stabilize
			mn.requestTopologyFromPeer(peer.ID)
		}()
		
		if mn.onPeerJoin != nil {
			mn.onPeerJoin(peer.ID)
		}
	}
	return nil
}

// handlePeerDisconnected handles P2P peer disconnection events
func (mn *MeshNetwork) handlePeerDisconnected(event p2p.Event) error {
	if peer, ok := event.Data.(*p2p.Peer); ok {
		mn.removePeer(peer.ID)
		mn.recalculateRoutes()
		
		if mn.onPeerLeave != nil {
			mn.onPeerLeave(peer.ID)
		}
	}
	return nil
}

// addDirectPeer adds a directly connected peer to topology
func (mn *MeshNetwork) addDirectPeer(peer *p2p.Peer) {
	mn.topologyMutex.Lock()
	defer mn.topologyMutex.Unlock()
	
	mn.topology[peer.ID] = &MeshPeer{
		ID:            peer.ID,
		Address:       peer.Address,
		Port:          peer.Port,
		LastSeen:      time.Now(),
		DirectConnect: true,
		Hops:          1,
		NextHop:       peer.ID,
		Peers:         make(map[string]time.Time),
		Version:       "1.0",
		Capabilities:  []string{"sync", "mesh", "routing"},
	}
	
	mn.recalculateRoutes()
}

// removePeer removes a peer from topology
func (mn *MeshNetwork) removePeer(peerID string) {
	mn.topologyMutex.Lock()
	defer mn.topologyMutex.Unlock()
	
	delete(mn.topology, peerID)
	
	// Remove routes that go through this peer
	mn.routesMutex.Lock()
	defer mn.routesMutex.Unlock()
	
	for target, route := range mn.routes {
		if len(route) > 0 && route[0] == peerID {
			delete(mn.routes, target)
		}
	}
}

// Helper function to generate message IDs
func generateMessageID() string {
	return fmt.Sprintf("%d-%d", time.Now().UnixNano(), time.Now().Nanosecond()%1000000)
}