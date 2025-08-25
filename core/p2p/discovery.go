package p2p

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"strings"
	"sync"
	"time"
)

// DiscoveryService handles peer discovery in the P2P network
type DiscoveryService struct {
	p2pNetwork    *P2PNetwork
	broadcastPort int
	ctx           context.Context
	cancel        context.CancelFunc
	knownPeers    map[string]*DiscoveredPeer
	peersMutex    sync.RWMutex
	udpConn       *net.UDPConn
	connMutex     sync.Mutex
}

// DiscoveredPeer represents a peer discovered through the discovery service
type DiscoveredPeer struct {
	NodeID      string    `json:"node_id"`
	Address     string    `json:"address"`
	Port        int       `json:"port"`
	LastSeen    time.Time `json:"last_seen"`
	Repositories []string  `json:"repositories"`
	Version     string    `json:"version"`
}

// DiscoveryAnnouncement is broadcast to announce presence on the network
type DiscoveryAnnouncement struct {
	NodeID       string    `json:"node_id"`
	Port         int       `json:"port"`
	Repositories []string  `json:"repositories"`
	Version      string    `json:"version"`
	Timestamp    time.Time `json:"timestamp"`
}

// NewDiscoveryService creates a new peer discovery service
func NewDiscoveryService(p2pNetwork *P2PNetwork, broadcastPort int) *DiscoveryService {
	ctx, cancel := context.WithCancel(context.Background())
	
	return &DiscoveryService{
		p2pNetwork:    p2pNetwork,
		broadcastPort: broadcastPort,
		ctx:           ctx,
		cancel:        cancel,
		knownPeers:    make(map[string]*DiscoveredPeer),
	}
}

// Start begins the peer discovery service
func (ds *DiscoveryService) Start(repositories []string) error {
	// Start UDP listener for discovery messages
	go ds.listenForAnnouncements()
	
	// Start periodic announcements
	go ds.announcePresence(repositories)
	
	// Start auto-connect service
	go ds.autoConnectService()
	
	fmt.Printf("Peer discovery service started on port %d\n", ds.broadcastPort)
	return nil
}

// Stop shuts down the discovery service
func (ds *DiscoveryService) Stop() {
	ds.cancel()
	
	// Close UDP connection if it exists
	ds.connMutex.Lock()
	if ds.udpConn != nil {
		ds.udpConn.Close()
		ds.udpConn = nil
	}
	ds.connMutex.Unlock()
	
	fmt.Println("Peer discovery service stopped")
}

// listenForAnnouncements listens for UDP broadcast messages from other peers
func (ds *DiscoveryService) listenForAnnouncements() {
	addr, err := net.ResolveUDPAddr("udp", fmt.Sprintf(":%d", ds.broadcastPort))
	if err != nil {
		fmt.Printf("Failed to resolve UDP address: %v\n", err)
		return
	}

	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		fmt.Printf("Failed to listen for UDP announcements: %v\n", err)
		return
	}
	
	// Store connection for proper cleanup
	ds.connMutex.Lock()
	ds.udpConn = conn
	ds.connMutex.Unlock()
	
	defer func() {
		ds.connMutex.Lock()
		if ds.udpConn == conn {
			ds.udpConn = nil
		}
		ds.connMutex.Unlock()
		conn.Close()
	}()

	buffer := make([]byte, 1024)

	for {
		select {
		case <-ds.ctx.Done():
			return
		default:
			conn.SetReadDeadline(time.Now().Add(1 * time.Second))
			n, clientAddr, err := conn.ReadFromUDP(buffer)
			if err != nil {
				if netErr, ok := err.(net.Error); ok && netErr.Timeout() {
					continue
				}
				continue
			}

			var announcement DiscoveryAnnouncement
			if err := json.Unmarshal(buffer[:n], &announcement); err != nil {
				continue
			}

			// Skip our own announcements
			if announcement.NodeID == ds.p2pNetwork.GetNodeID() {
				continue
			}

			ds.handleDiscoveryAnnouncement(&announcement, clientAddr.IP.String())
		}
	}
}

// announcePresence periodically broadcasts our presence to the network
func (ds *DiscoveryService) announcePresence(repositories []string) {
	ticker := time.NewTicker(60 * time.Second) // Announce every minute
	defer ticker.Stop()

	// Announce immediately on start
	ds.broadcastAnnouncement(repositories)

	for {
		select {
		case <-ds.ctx.Done():
			return
		case <-ticker.C:
			ds.broadcastAnnouncement(repositories)
		}
	}
}

// broadcastAnnouncement sends a discovery announcement to the network
func (ds *DiscoveryService) broadcastAnnouncement(repositories []string) {
	announcement := DiscoveryAnnouncement{
		NodeID:       ds.p2pNetwork.GetNodeID(),
		Port:         ds.p2pNetwork.port,
		Repositories: repositories,
		Version:      "1.0",
		Timestamp:    time.Now(),
	}

	data, err := json.Marshal(announcement)
	if err != nil {
		fmt.Printf("Failed to marshal announcement: %v\n", err)
		return
	}

	// Broadcast to local network
	ds.broadcastToLocalNetwork(data)

	// Also broadcast to known subnets
	ds.broadcastToKnownNetworks(data)
}

// broadcastToLocalNetwork sends announcement to the local subnet
func (ds *DiscoveryService) broadcastToLocalNetwork(data []byte) {
	// Get local IP to determine broadcast address
	interfaces, err := net.Interfaces()
	if err != nil {
		return
	}

	for _, iface := range interfaces {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagLoopback != 0 {
			continue
		}

		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}

		for _, addr := range addrs {
			var ip net.IP
			var ipNet *net.IPNet
			switch v := addr.(type) {
			case *net.IPNet:
				ip = v.IP
				ipNet = v
			case *net.IPAddr:
				ip = v.IP
				// Skip IPAddr as it doesn't have network mask information
				continue
			}

			if ip == nil || ip.IsLoopback() || ipNet == nil {
				continue
			}

			ip = ip.To4()
			if ip == nil || len(ip) != 4 {
				continue
			}

			// Get the network mask
			mask := ipNet.Mask
			if len(mask) != 4 {
				continue
			}

			// Calculate broadcast address: (ip & mask) | (^mask)
			// First compute network bytes by ANDing IP with mask
			// Then compute broadcast bytes by ORing with bitwise NOT of mask
			broadcastIP := net.IPv4(
				(ip[0]&mask[0])|(^mask[0]),
				(ip[1]&mask[1])|(^mask[1]),
				(ip[2]&mask[2])|(^mask[2]),
				(ip[3]&mask[3])|(^mask[3]),
			)

			broadcastAddr := &net.UDPAddr{
				IP:   broadcastIP,
				Port: ds.broadcastPort,
			}

			ds.sendUDPMessage(data, broadcastAddr)
		}
	}
}

// broadcastToKnownNetworks sends announcements to known network ranges
func (ds *DiscoveryService) broadcastToKnownNetworks(data []byte) {
	// Common local network ranges
	networks := []string{
		"192.168.1.255",
		"192.168.0.255",
		"10.0.0.255",
		"172.16.0.255",
	}

	for _, network := range networks {
		broadcastAddr := &net.UDPAddr{
			IP:   net.ParseIP(network),
			Port: ds.broadcastPort,
		}
		ds.sendUDPMessage(data, broadcastAddr)
	}
}

// sendUDPMessage sends a UDP message to the specified address
func (ds *DiscoveryService) sendUDPMessage(data []byte, addr *net.UDPAddr) {
	conn, err := net.DialUDP("udp", nil, addr)
	if err != nil {
		return
	}
	defer conn.Close()

	conn.Write(data)
}

// handleDiscoveryAnnouncement processes received discovery announcements
func (ds *DiscoveryService) handleDiscoveryAnnouncement(announcement *DiscoveryAnnouncement, sourceIP string) {
	ds.peersMutex.Lock()
	defer ds.peersMutex.Unlock()

	peer := &DiscoveredPeer{
		NodeID:       announcement.NodeID,
		Address:      sourceIP,
		Port:         announcement.Port,
		LastSeen:     time.Now(),
		Repositories: announcement.Repositories,
		Version:      announcement.Version,
	}

	ds.knownPeers[announcement.NodeID] = peer
	
	fmt.Printf("Discovered peer: %s at %s:%d (repos: %v)\n", 
		announcement.NodeID, sourceIP, announcement.Port, announcement.Repositories)
}

// autoConnectService automatically connects to discovered peers
func (ds *DiscoveryService) autoConnectService() {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ds.ctx.Done():
			return
		case <-ticker.C:
			ds.connectToDiscoveredPeers()
		}
	}
}

// connectToDiscoveredPeers attempts to connect to recently discovered peers
func (ds *DiscoveryService) connectToDiscoveredPeers() {
	ds.peersMutex.RLock()
	discovered := make([]*DiscoveredPeer, 0, len(ds.knownPeers))
	for _, peer := range ds.knownPeers {
		discovered = append(discovered, peer)
	}
	ds.peersMutex.RUnlock()

	connectedPeers := ds.p2pNetwork.GetPeers()
	connectedMap := make(map[string]bool)
	for _, peer := range connectedPeers {
		connectedMap[peer.ID] = true
	}

	for _, peer := range discovered {
		// Skip if already connected
		if connectedMap[peer.NodeID] {
			continue
		}

		// Skip if not seen recently
		if time.Since(peer.LastSeen) > 5*time.Minute {
			continue
		}

		// Attempt connection
		go func(p *DiscoveredPeer) {
			err := ds.p2pNetwork.ConnectToPeer(p.Address, p.Port)
			if err != nil {
				fmt.Printf("Failed to auto-connect to peer %s: %v\n", p.NodeID, err)
			}
		}(peer)
	}
}

// GetDiscoveredPeers returns all discovered peers
func (ds *DiscoveryService) GetDiscoveredPeers() []*DiscoveredPeer {
	ds.peersMutex.RLock()
	defer ds.peersMutex.RUnlock()

	peers := make([]*DiscoveredPeer, 0, len(ds.knownPeers))
	for _, peer := range ds.knownPeers {
		// Create a shallow copy to prevent callers from mutating internal state
		peerCopy := &DiscoveredPeer{
			NodeID:       peer.NodeID,
			Address:      peer.Address,
			Port:         peer.Port,
			LastSeen:     peer.LastSeen,
			Repositories: peer.Repositories, // Note: shallow copy of slice
			Version:      peer.Version,
		}
		peers = append(peers, peerCopy)
	}
	return peers
}

// FindPeersWithRepository returns peers that have a specific repository
func (ds *DiscoveryService) FindPeersWithRepository(repoName string) []*DiscoveredPeer {
	ds.peersMutex.RLock()
	defer ds.peersMutex.RUnlock()

	var matches []*DiscoveredPeer
	for _, peer := range ds.knownPeers {
		for _, repo := range peer.Repositories {
			if strings.Contains(repo, repoName) {
				// Create a shallow copy to prevent callers from mutating internal state
				peerCopy := &DiscoveredPeer{
					NodeID:       peer.NodeID,
					Address:      peer.Address,
					Port:         peer.Port,
					LastSeen:     peer.LastSeen,
					Repositories: peer.Repositories, // Note: shallow copy of slice
					Version:      peer.Version,
				}
				matches = append(matches, peerCopy)
				break
			}
		}
	}
	return matches
}

// CleanupOldPeers removes peers that haven't been seen recently
func (ds *DiscoveryService) CleanupOldPeers() {
	ds.peersMutex.Lock()
	defer ds.peersMutex.Unlock()

	cutoff := time.Now().Add(-10 * time.Minute)
	for nodeID, peer := range ds.knownPeers {
		if peer.LastSeen.Before(cutoff) {
			delete(ds.knownPeers, nodeID)
		}
	}
}

// peerDiscoveryService handles peer discovery protocol messages
func (p2p *P2PNetwork) peerDiscoveryService() {
	ticker := time.NewTicker(2 * time.Minute)
	defer ticker.Stop()

	for {
		select {
		case <-p2p.ctx.Done():
			return
		case <-ticker.C:
			// Request peer lists from connected peers
			p2p.requestPeerLists()
		}
	}
}

// requestPeerLists asks connected peers for their peer lists
func (p2p *P2PNetwork) requestPeerLists() {
	requestData := map[string]interface{}{
		"requesting_peers": true,
		"timestamp":        time.Now(),
	}

	p2p.BroadcastMessage(MessageTypePeerDiscovery, requestData)
}

// handlePeerDiscovery processes peer discovery protocol messages
func (p2p *P2PNetwork) handlePeerDiscovery(peer *Peer, message *Message) error {
	if data, ok := message.Data.(map[string]interface{}); ok {
		if requesting, ok := data["requesting_peers"].(bool); ok && requesting {
			// Send our peer list
			peers := p2p.GetPeers()
			peerList := make([]map[string]interface{}, 0, len(peers))
			
			for _, p := range peers {
				if p.ID != peer.ID { // Don't send back the requester
					peerList = append(peerList, map[string]interface{}{
						"id":      p.ID,
						"address": p.Address,
						"port":    p.Port,
					})
				}
			}

			responseData := map[string]interface{}{
				"peer_list": peerList,
				"timestamp": time.Now(),
			}

			return p2p.SendMessage(peer.ID, MessageTypePeerDiscovery, responseData)
		}

		// Handle received peer list
		if peerList, ok := data["peer_list"].([]interface{}); ok {
			for _, peerData := range peerList {
				if peerMap, ok := peerData.(map[string]interface{}); ok {
					address, _ := peerMap["address"].(string)
					port, _ := peerMap["port"].(float64)
					
					if address != "" && port > 0 {
						// Try to connect to this peer if not already connected
						go func(addr string, p int) {
							p2p.ConnectToPeer(addr, int(p))
						}(address, int(port))
					}
				}
			}
		}
	}

	return nil
}