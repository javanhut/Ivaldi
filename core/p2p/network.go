package p2p

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"net"
	"sync"
	"sync/atomic"
	"time"
)

// P2PNetwork manages peer-to-peer connections and communication
type P2PNetwork struct {
	nodeID       string
	port         int
	peers        map[string]*Peer
	peersMutex   sync.RWMutex
	listener     net.Listener
	ctx          context.Context
	cancel       context.CancelFunc
	eventBus     *EventBus
	syncCallback SyncCallback
}

// Peer represents a connected peer in the P2P network
type Peer struct {
	ID       string    `json:"id"`
	Address  string    `json:"address"`
	Port     int       `json:"port"`
	LastSeen time.Time `json:"last_seen"`
	Status   PeerStatus `json:"status"`
	conn     net.Conn
	encoder  *json.Encoder
	decoder  *json.Decoder
	metrics  *PeerMetrics
}

// PeerStatus represents the current status of a peer
type PeerStatus string

const (
	PeerStatusConnected    PeerStatus = "connected"
	PeerStatusConnecting   PeerStatus = "connecting"
	PeerStatusDisconnected PeerStatus = "disconnected"
	PeerStatusSyncing      PeerStatus = "syncing"
)

// PeerMetrics tracks performance metrics for a peer
type PeerMetrics struct {
	BytesSent     int64
	BytesReceived int64
	SyncCount     int64
	LastSync      time.Time
	Latency       time.Duration
}

// SyncCallback is called when sync events occur
type SyncCallback func(peerID string, event SyncEvent, data interface{}) error

// SyncEvent represents different types of sync events
type SyncEvent string

const (
	SyncEventTimelineUpdate SyncEvent = "timeline_update"
	SyncEventNewSeal        SyncEvent = "new_seal"
	SyncEventPeerJoined     SyncEvent = "peer_joined"
	SyncEventPeerLeft       SyncEvent = "peer_left"
	SyncEventConflict       SyncEvent = "conflict"
	SyncEventSyncRequest    SyncEvent = "sync_request"
	SyncEventSyncResponse   SyncEvent = "sync_response"
)

// Message represents a P2P network message
type Message struct {
	Type      MessageType `json:"type"`
	From      string      `json:"from"`
	To        string      `json:"to"`
	Timestamp time.Time   `json:"timestamp"`
	Data      interface{} `json:"data"`
	ID        string      `json:"id"`
}

// MessageType represents different types of P2P messages
type MessageType string

const (
	MessageTypeHandshake         MessageType = "handshake"
	MessageTypePeerDiscovery     MessageType = "peer_discovery"
	MessageTypeSyncRequest       MessageType = "sync_request"
	MessageTypeSyncResponse      MessageType = "sync_response"
	MessageTypeTimelineUpdate    MessageType = "timeline_update"
	MessageTypeSealBroadcast     MessageType = "seal_broadcast"
	MessageTypeHeartbeat         MessageType = "heartbeat"
	MessageTypeConflictResolve   MessageType = "conflict_resolve"
	MessageTypeMeshTopology      MessageType = "mesh_topology"
	MessageTypeMeshTopologyReq   MessageType = "mesh_topology_request"
)

// NewP2PNetwork creates a new P2P network instance
func NewP2PNetwork(port int, eventBus *EventBus, syncCallback SyncCallback) (*P2PNetwork, error) {
	nodeID, err := generateNodeID()
	if err != nil {
		return nil, fmt.Errorf("failed to generate node ID: %v", err)
	}

	ctx, cancel := context.WithCancel(context.Background())

	return &P2PNetwork{
		nodeID:       nodeID,
		port:         port,
		peers:        make(map[string]*Peer),
		ctx:          ctx,
		cancel:       cancel,
		eventBus:     eventBus,
		syncCallback: syncCallback,
	}, nil
}

// Start begins listening for P2P connections
func (p2p *P2PNetwork) Start() error {
	listener, err := net.Listen("tcp", fmt.Sprintf(":%d", p2p.port))
	if err != nil {
		return fmt.Errorf("failed to start P2P listener: %v", err)
	}

	p2p.listener = listener
	fmt.Printf("P2P network started on port %d with node ID: %s\n", p2p.port, p2p.nodeID)

	// Start accepting connections
	go p2p.acceptConnections()

	// Start heartbeat service for keeping peers synced
	go p2p.heartbeatService()

	// Start peer discovery service
	go p2p.peerDiscoveryService()

	return nil
}

// Stop shuts down the P2P network
func (p2p *P2PNetwork) Stop() error {
	p2p.cancel()

	if p2p.listener != nil {
		p2p.listener.Close()
	}

	// Disconnect all peers
	p2p.peersMutex.Lock()
	for _, peer := range p2p.peers {
		if peer.conn != nil {
			peer.conn.Close()
		}
	}
	p2p.peers = make(map[string]*Peer)
	p2p.peersMutex.Unlock()

	fmt.Println("P2P network stopped")
	return nil
}

// ConnectToPeer establishes a connection to a peer
func (p2p *P2PNetwork) ConnectToPeer(address string, port int) error {
	peerAddr := fmt.Sprintf("%s:%d", address, port)
	
	conn, err := net.DialTimeout("tcp", peerAddr, 10*time.Second)
	if err != nil {
		return fmt.Errorf("failed to connect to peer %s: %v", peerAddr, err)
	}

	peer := &Peer{
		Address:  address,
		Port:     port,
		Status:   PeerStatusConnecting,
		LastSeen: time.Now(),
		conn:     conn,
		encoder:  json.NewEncoder(conn),
		decoder:  json.NewDecoder(conn),
		metrics:  &PeerMetrics{},
	}

	// Send handshake
	handshake := Message{
		Type:      MessageTypeHandshake,
		From:      p2p.nodeID,
		Timestamp: time.Now(),
		Data: map[string]interface{}{
			"version": "1.0",
			"port":    p2p.port,
		},
		ID: generateMessageID(),
	}

	if err := peer.encoder.Encode(handshake); err != nil {
		conn.Close()
		return fmt.Errorf("failed to send handshake: %v", err)
	}

	// Wait for handshake response
	var response Message
	if err := peer.decoder.Decode(&response); err != nil {
		conn.Close()
		return fmt.Errorf("failed to receive handshake response: %v", err)
	}

	if response.Type != MessageTypeHandshake {
		conn.Close()
		return fmt.Errorf("unexpected handshake response type: %s", response.Type)
	}

	peer.ID = response.From
	peer.Status = PeerStatusConnected
	
	p2p.peersMutex.Lock()
	p2p.peers[peer.ID] = peer
	p2p.peersMutex.Unlock()

	// Start handling messages from this peer
	go p2p.handlePeerConnection(peer)

	// Notify about new peer
	if p2p.syncCallback != nil {
		p2p.syncCallback(peer.ID, SyncEventPeerJoined, peer)
	}

	fmt.Printf("Connected to peer: %s (%s:%d)\n", peer.ID, address, port)
	return nil
}

// acceptConnections handles incoming peer connections
func (p2p *P2PNetwork) acceptConnections() {
	for {
		select {
		case <-p2p.ctx.Done():
			return
		default:
			conn, err := p2p.listener.Accept()
			if err != nil {
				if p2p.ctx.Err() != nil {
					return // Context cancelled
				}
				fmt.Printf("Failed to accept connection: %v\n", err)
				continue
			}

			go p2p.handleIncomingConnection(conn)
		}
	}
}

// handleIncomingConnection processes new incoming connections
func (p2p *P2PNetwork) handleIncomingConnection(conn net.Conn) {
	decoder := json.NewDecoder(conn)
	encoder := json.NewEncoder(conn)

	// Wait for handshake
	var handshake Message
	if err := decoder.Decode(&handshake); err != nil {
		conn.Close()
		return
	}

	if handshake.Type != MessageTypeHandshake {
		conn.Close()
		return
	}

	// Send handshake response
	response := Message{
		Type:      MessageTypeHandshake,
		From:      p2p.nodeID,
		To:        handshake.From,
		Timestamp: time.Now(),
		Data: map[string]interface{}{
			"version": "1.0",
			"port":    p2p.port,
		},
		ID: generateMessageID(),
	}

	if err := encoder.Encode(response); err != nil {
		conn.Close()
		return
	}

	// Create peer
	peer := &Peer{
		ID:       handshake.From,
		Status:   PeerStatusConnected,
		LastSeen: time.Now(),
		conn:     conn,
		encoder:  encoder,
		decoder:  decoder,
		metrics:  &PeerMetrics{},
	}

	// Extract peer address info from handshake data
	if data, ok := handshake.Data.(map[string]interface{}); ok {
		if port, ok := data["port"].(float64); ok {
			peer.Port = int(port)
		}
	}

	p2p.peersMutex.Lock()
	p2p.peers[peer.ID] = peer
	p2p.peersMutex.Unlock()

	// Start handling messages from this peer
	go p2p.handlePeerConnection(peer)

	// Notify about new peer
	if p2p.syncCallback != nil {
		p2p.syncCallback(peer.ID, SyncEventPeerJoined, peer)
	}

	fmt.Printf("Peer connected: %s\n", peer.ID)
}

// handlePeerConnection manages communication with a connected peer
func (p2p *P2PNetwork) handlePeerConnection(peer *Peer) {
	defer func() {
		peer.conn.Close()
		
		p2p.peersMutex.Lock()
		delete(p2p.peers, peer.ID)
		p2p.peersMutex.Unlock()

		// Notify about peer leaving
		if p2p.syncCallback != nil {
			p2p.syncCallback(peer.ID, SyncEventPeerLeft, peer)
		}

		fmt.Printf("Peer disconnected: %s\n", peer.ID)
	}()

	for {
		select {
		case <-p2p.ctx.Done():
			return
		default:
			var message Message
			if err := peer.decoder.Decode(&message); err != nil {
				return // Connection error
			}

			peer.LastSeen = time.Now()
			
			// Calculate message size by marshaling back to JSON
			if messageBytes, err := json.Marshal(message); err == nil {
				atomic.AddInt64(&peer.metrics.BytesReceived, int64(len(messageBytes)))
			} else {
				// Fallback: estimate message size if marshaling fails
				atomic.AddInt64(&peer.metrics.BytesReceived, 100) // Conservative estimate
			}

			if err := p2p.handleMessage(peer, &message); err != nil {
				fmt.Printf("Error handling message from peer %s: %v\n", peer.ID, err)
			}
		}
	}
}

// handleMessage processes incoming messages from peers
func (p2p *P2PNetwork) handleMessage(peer *Peer, message *Message) error {
	switch message.Type {
	case MessageTypeHeartbeat:
		return p2p.handleHeartbeat(peer, message)
	case MessageTypeSyncRequest:
		return p2p.handleSyncRequest(peer, message)
	case MessageTypeSyncResponse:
		return p2p.handleSyncResponse(peer, message)
	case MessageTypeTimelineUpdate:
		return p2p.handleTimelineUpdate(peer, message)
	case MessageTypeSealBroadcast:
		return p2p.handleSealBroadcast(peer, message)
	case MessageTypePeerDiscovery:
		return p2p.handlePeerDiscovery(peer, message)
	default:
		return fmt.Errorf("unknown message type: %s", message.Type)
	}
}

// SendMessage sends a message to a specific peer
func (p2p *P2PNetwork) SendMessage(peerID string, msgType MessageType, data interface{}) error {
	p2p.peersMutex.RLock()
	peer, exists := p2p.peers[peerID]
	p2p.peersMutex.RUnlock()

	if !exists {
		return fmt.Errorf("peer not found: %s", peerID)
	}

	message := Message{
		Type:      msgType,
		From:      p2p.nodeID,
		To:        peerID,
		Timestamp: time.Now(),
		Data:      data,
		ID:        generateMessageID(),
	}

	if err := peer.encoder.Encode(message); err != nil {
		return fmt.Errorf("failed to send message to peer %s: %v", peerID, err)
	}

	// Calculate actual message size by marshaling to JSON
	if messageBytes, err := json.Marshal(message); err == nil {
		atomic.AddInt64(&peer.metrics.BytesSent, int64(len(messageBytes)))
	} else {
		// Fallback: estimate message size if marshaling fails
		atomic.AddInt64(&peer.metrics.BytesSent, 100) // Conservative estimate
	}
	return nil
}

// BroadcastMessage sends a message to all connected peers
func (p2p *P2PNetwork) BroadcastMessage(msgType MessageType, data interface{}) error {
	p2p.peersMutex.RLock()
	peers := make([]*Peer, 0, len(p2p.peers))
	for _, peer := range p2p.peers {
		peers = append(peers, peer)
	}
	p2p.peersMutex.RUnlock()

	var errors []error
	for _, peer := range peers {
		if err := p2p.SendMessage(peer.ID, msgType, data); err != nil {
			errors = append(errors, err)
		}
	}

	if len(errors) > 0 {
		return fmt.Errorf("failed to broadcast to %d peers", len(errors))
	}

	return nil
}

// GetPeers returns a list of all connected peers
func (p2p *P2PNetwork) GetPeers() []*Peer {
	p2p.peersMutex.RLock()
	defer p2p.peersMutex.RUnlock()

	peers := make([]*Peer, 0, len(p2p.peers))
	for _, peer := range p2p.peers {
		peers = append(peers, peer)
	}
	return peers
}

// GetNodeID returns the current node's ID
func (p2p *P2PNetwork) GetNodeID() string {
	return p2p.nodeID
}

// heartbeatService maintains connections and sync state with peers
func (p2p *P2PNetwork) heartbeatService() {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-p2p.ctx.Done():
			return
		case <-ticker.C:
			p2p.sendHeartbeats()
		}
	}
}

// sendHeartbeats sends heartbeat messages to all peers
func (p2p *P2PNetwork) sendHeartbeats() {
	heartbeatData := map[string]interface{}{
		"timestamp": time.Now(),
		"node_id":   p2p.nodeID,
	}

	p2p.BroadcastMessage(MessageTypeHeartbeat, heartbeatData)
}

// handleHeartbeat processes heartbeat messages from peers
func (p2p *P2PNetwork) handleHeartbeat(peer *Peer, message *Message) error {
	peer.LastSeen = time.Now()
	
	// Respond to heartbeat to show we're alive
	responseData := map[string]interface{}{
		"timestamp": time.Now(),
		"node_id":   p2p.nodeID,
		"response":  true,
	}

	return p2p.SendMessage(peer.ID, MessageTypeHeartbeat, responseData)
}

// handleSyncRequest processes sync requests from peers
func (p2p *P2PNetwork) handleSyncRequest(peer *Peer, message *Message) error {
	// Basic implementation - could be enhanced to delegate to sync manager
	fmt.Printf("Received sync request from peer %s\n", peer.ID)
	return nil
}

// handleSyncResponse processes sync responses from peers
func (p2p *P2PNetwork) handleSyncResponse(peer *Peer, message *Message) error {
	// Basic implementation - could be enhanced to delegate to sync manager
	fmt.Printf("Received sync response from peer %s\n", peer.ID)
	return nil
}

// handleTimelineUpdate processes timeline updates from peers
func (p2p *P2PNetwork) handleTimelineUpdate(peer *Peer, message *Message) error {
	// Basic implementation - could be enhanced to delegate to sync manager
	fmt.Printf("Received timeline update from peer %s\n", peer.ID)
	return nil
}

// handleSealBroadcast processes seal broadcasts from peers
func (p2p *P2PNetwork) handleSealBroadcast(peer *Peer, message *Message) error {
	// Basic implementation - could be enhanced to delegate to sync manager
	fmt.Printf("Received seal broadcast from peer %s\n", peer.ID)
	return nil
}

// generateNodeID creates a unique identifier for this node
func generateNodeID() (string, error) {
	bytes := make([]byte, 16)
	if _, err := rand.Read(bytes); err != nil {
		return "", err
	}
	return fmt.Sprintf("%x", bytes), nil
}

// generateMessageID creates a unique identifier for messages
func generateMessageID() string {
	bytes := make([]byte, 8)
	rand.Read(bytes)
	return fmt.Sprintf("%x", bytes)
}