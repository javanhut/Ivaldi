package p2p

// P2PManagerInterface defines the interface that all P2P managers must implement
type P2PManagerInterface interface {
	// Core lifecycle methods
	Start() error
	Stop() error
	IsRunning() bool
	
	// Status and configuration
	GetStatus() *P2PStatus
	GetConfig() *P2PConfig
	
	// Peer management
	ConnectToPeer(address string, port int) error
	GetPeers() []*Peer
	GetDiscoveredPeers() []*DiscoveredPeer
	FindPeersWithRepository(repoName string) []*DiscoveredPeer
	
	// Synchronization
	SyncWithPeer(peerID string) error
	SyncWithAllPeers() error
	GetSyncState() map[string]*PeerSyncState
	
	// Event handling
	Subscribe(eventType string, handler EventHandler)
	
	// Configuration methods
	EnableAutoSync(enabled bool) error
	SetSyncInterval(interval string) error
}