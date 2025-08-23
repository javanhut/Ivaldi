package mesh

import (
	"fmt"
	"sync"
	"time"

	"ivaldi/core/p2p"
)

// MeshManager manages mesh networking for a repository
type MeshManager struct {
	meshNetwork *MeshNetwork
	p2pManager  *p2p.P2PManager
	running     bool
	mutex       sync.RWMutex
}

// MeshStatus contains mesh network status information
type MeshStatus struct {
	Running       bool                   `json:"running"`
	NodeID        string                 `json:"node_id"`
	PeerCount     int                    `json:"peer_count"`
	DirectPeers   int                    `json:"direct_peers"`
	IndirectPeers int                    `json:"indirect_peers"`
	MaxHops       int                    `json:"max_hops"`
	AvgHops       float64               `json:"avg_hops"`
	Topology      map[string]*MeshPeer  `json:"topology"`
	Routes        map[string][]string   `json:"routes"`
}

// NewMeshManager creates a new mesh manager
func NewMeshManager(p2pManager *p2p.P2PManager) *MeshManager {
	meshNetwork := NewMeshNetwork(p2pManager)
	
	return &MeshManager{
		meshNetwork: meshNetwork,
		p2pManager:  p2pManager,
		running:     false,
	}
}

// Start begins mesh networking
func (mm *MeshManager) Start() error {
	mm.mutex.Lock()
	defer mm.mutex.Unlock()
	
	if mm.running {
		return fmt.Errorf("mesh network is already running")
	}
	
	if err := mm.meshNetwork.Start(); err != nil {
		return fmt.Errorf("failed to start mesh network: %v", err)
	}
	
	mm.running = true
	return nil
}

// Stop shuts down mesh networking
func (mm *MeshManager) Stop() error {
	mm.mutex.Lock()
	defer mm.mutex.Unlock()
	
	if !mm.running {
		return nil
	}
	
	if err := mm.meshNetwork.Stop(); err != nil {
		return fmt.Errorf("failed to stop mesh network: %v", err)
	}
	
	mm.running = false
	return nil
}

// IsRunning returns whether mesh networking is active
func (mm *MeshManager) IsRunning() bool {
	mm.mutex.RLock()
	defer mm.mutex.RUnlock()
	return mm.running
}

// Join connects to a mesh network via a bootstrap peer
func (mm *MeshManager) Join(bootstrapAddress string, bootstrapPort int) error {
	if !mm.running {
		return fmt.Errorf("mesh network is not running")
	}
	
	return mm.meshNetwork.JoinMesh(bootstrapAddress, bootstrapPort)
}

// GetStatus returns current mesh network status
func (mm *MeshManager) GetStatus() *MeshStatus {
	mm.mutex.RLock()
	defer mm.mutex.RUnlock()
	
	status := &MeshStatus{
		Running: mm.running,
	}
	
	if !mm.running {
		return status
	}
	
	// Get basic info
	p2pStatus := mm.p2pManager.GetStatus()
	status.NodeID = p2pStatus.NodeID
	
	// Get topology information
	topology := mm.meshNetwork.GetTopology()
	status.Topology = topology
	status.PeerCount = len(topology) - 1 // Exclude ourselves
	
	// Count direct vs indirect peers
	directCount := 0
	indirectCount := 0
	totalHops := 0
	maxHops := 0
	
	for peerID, peer := range topology {
		if peerID == status.NodeID {
			continue
		}
		
		if peer.DirectConnect {
			directCount++
		} else {
			indirectCount++
		}
		
		totalHops += peer.Hops
		if peer.Hops > maxHops {
			maxHops = peer.Hops
		}
	}
	
	status.DirectPeers = directCount
	status.IndirectPeers = indirectCount
	status.MaxHops = maxHops
	
	if status.PeerCount > 0 {
		status.AvgHops = float64(totalHops) / float64(status.PeerCount)
	}
	
	// Get routing information
	status.Routes = make(map[string][]string)
	
	// Get all routes
	for peerID := range topology {
		if peerID != status.NodeID {
			route := mm.meshNetwork.GetRoute(peerID)
			if len(route) > 0 {
				status.Routes[peerID] = route
			}
		}
	}
	
	return status
}

// GetTopology returns the current mesh topology
func (mm *MeshManager) GetTopology() map[string]*MeshPeer {
	if !mm.running {
		return make(map[string]*MeshPeer)
	}
	
	return mm.meshNetwork.GetTopology()
}

// GetRoute returns the route to a specific peer
func (mm *MeshManager) GetRoute(targetPeerID string) []string {
	if !mm.running {
		return nil
	}
	
	return mm.meshNetwork.GetRoute(targetPeerID)
}

// SendMessage sends a message through the mesh network
func (mm *MeshManager) SendMessage(targetPeerID string, messageType string, payload interface{}) error {
	if !mm.running {
		return fmt.Errorf("mesh network is not running")
	}
	
	return mm.meshNetwork.SendMeshMessage(targetPeerID, messageType, payload)
}

// Ping sends a ping message to a peer via mesh routing
func (mm *MeshManager) Ping(targetPeerID string) error {
	return mm.SendMessage(targetPeerID, "ping", fmt.Sprintf("ping from %s at %s", 
		mm.meshNetwork.nodeID, time.Now().Format(time.RFC3339)))
}

// GetPeers returns information about all peers in the mesh
func (mm *MeshManager) GetPeers() []*MeshPeer {
	topology := mm.GetTopology()
	peers := make([]*MeshPeer, 0, len(topology))
	
	for peerID, peer := range topology {
		if peerID != mm.meshNetwork.nodeID {
			peerCopy := *peer
			peers = append(peers, &peerCopy)
		}
	}
	
	return peers
}

// GetDirectPeers returns only directly connected peers
func (mm *MeshManager) GetDirectPeers() []*MeshPeer {
	peers := mm.GetPeers()
	directPeers := make([]*MeshPeer, 0)
	
	for _, peer := range peers {
		if peer.DirectConnect {
			directPeers = append(directPeers, peer)
		}
	}
	
	return directPeers
}

// GetIndirectPeers returns only indirectly connected peers
func (mm *MeshManager) GetIndirectPeers() []*MeshPeer {
	peers := mm.GetPeers()
	indirectPeers := make([]*MeshPeer, 0)
	
	for _, peer := range peers {
		if !peer.DirectConnect {
			indirectPeers = append(indirectPeers, peer)
		}
	}
	
	return indirectPeers
}

// FindPeersWithCapability finds peers that have a specific capability
func (mm *MeshManager) FindPeersWithCapability(capability string) []*MeshPeer {
	peers := mm.GetPeers()
	matchingPeers := make([]*MeshPeer, 0)
	
	for _, peer := range peers {
		for _, cap := range peer.Capabilities {
			if cap == capability {
				matchingPeers = append(matchingPeers, peer)
				break
			}
		}
	}
	
	return matchingPeers
}

// HealNetwork manually triggers network healing
func (mm *MeshManager) HealNetwork() error {
	if !mm.running {
		return fmt.Errorf("mesh network is not running")
	}
	
	// Force a healing cycle
	go mm.meshNetwork.healNetwork()
	return nil
}

// RefreshTopology manually triggers topology refresh
func (mm *MeshManager) RefreshTopology() error {
	if !mm.running {
		return fmt.Errorf("mesh network is not running")
	}
	
	// Force topology gossip
	go mm.meshNetwork.gossipTopology()
	return nil
}

// SetEventHandlers sets event handlers for mesh events
func (mm *MeshManager) SetEventHandlers(
	onPeerJoin func(peerID string),
	onPeerLeave func(peerID string),
	onTopologyChange func(),
) {
	if mm.meshNetwork != nil {
		mm.meshNetwork.onPeerJoin = onPeerJoin
		mm.meshNetwork.onPeerLeave = onPeerLeave
		mm.meshNetwork.onTopologyChange = onTopologyChange
	}
}