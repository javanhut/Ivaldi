package mesh

import (
	"fmt"
	"math"
	"time"
)

// topologyGossipService periodically shares topology information
func (mn *MeshNetwork) topologyGossipService() {
	ticker := time.NewTicker(mn.gossipInterval)
	defer ticker.Stop()
	
	for {
		select {
		case <-mn.ctx.Done():
			return
		case <-ticker.C:
			mn.gossipTopology()
		}
	}
}

// routeMaintenanceService maintains routing tables
func (mn *MeshNetwork) routeMaintenanceService() {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()
	
	for {
		select {
		case <-mn.ctx.Done():
			return
		case <-ticker.C:
			mn.recalculateRoutes()
		}
	}
}

// networkHealingService attempts to heal network partitions
func (mn *MeshNetwork) networkHealingService() {
	ticker := time.NewTicker(mn.healingInterval)
	defer ticker.Stop()
	
	for {
		select {
		case <-mn.ctx.Done():
			return
		case <-ticker.C:
			mn.healNetwork()
		}
	}
}

// topologyCleanupService removes stale topology entries
func (mn *MeshNetwork) topologyCleanupService() {
	ticker := time.NewTicker(2 * time.Minute)
	defer ticker.Stop()
	
	for {
		select {
		case <-mn.ctx.Done():
			return
		case <-ticker.C:
			mn.cleanupStaleEntries()
		}
	}
}

// gossipTopology shares our topology knowledge with connected peers
func (mn *MeshNetwork) gossipTopology() {
	mn.topologyMutex.RLock()
	topology := make(map[string]*MeshPeer)
	for id, peer := range mn.topology {
		peerCopy := *peer
		topology[id] = &peerCopy
	}
	mn.topologyMutex.RUnlock()
	
	update := &MeshTopologyUpdate{
		FromPeer:  mn.nodeID,
		Timestamp: time.Now(),
		Topology:  topology,
		TTL:       3, // Limit gossip propagation
	}
	
	// Send to all directly connected peers
	connectedPeers := mn.p2pManager.GetPeers()
	for _, peer := range connectedPeers {
		go mn.sendTopologyUpdate(peer.ID, update)
	}
}

// sendTopologyUpdate sends a topology update to a specific peer
func (mn *MeshNetwork) sendTopologyUpdate(peerID string, update *MeshTopologyUpdate) {
	// Use P2P to send the topology update
	// This would need to be integrated with the P2P message system
	fmt.Printf("Sending topology update to peer %s (would implement P2P message sending)\n", peerID)
}

// requestTopologyFromPeer requests topology information from a peer
func (mn *MeshNetwork) requestTopologyFromPeer(peerID string) error {
	// Send topology request via P2P
	fmt.Printf("Requesting topology from peer %s (would implement P2P message sending)\n", peerID)
	return nil
}

// handleTopologyUpdate processes received topology updates
func (mn *MeshNetwork) HandleTopologyUpdate(update *MeshTopologyUpdate) {
	if update.TTL <= 0 {
		return // Don't process expired updates
	}
	
	mn.topologyMutex.Lock()
	defer mn.topologyMutex.Unlock()
	
	updated := false
	
	// Merge received topology with our knowledge
	for peerID, remotePeer := range update.Topology {
		if peerID == mn.nodeID {
			continue // Skip ourselves
		}
		
		localPeer, exists := mn.topology[peerID]
		
		if !exists {
			// New peer discovered
			newPeer := *remotePeer
			newPeer.DirectConnect = false
			newPeer.Hops = remotePeer.Hops + 1
			newPeer.NextHop = update.FromPeer
			mn.topology[peerID] = &newPeer
			updated = true
			fmt.Printf("Discovered new peer via mesh: %s (via %s, %d hops)\n", 
				peerID, update.FromPeer, newPeer.Hops)
		} else if remotePeer.LastSeen.After(localPeer.LastSeen) && 
			remotePeer.Hops+1 < localPeer.Hops {
			// Better route found
			localPeer.Hops = remotePeer.Hops + 1
			localPeer.NextHop = update.FromPeer
			localPeer.LastSeen = remotePeer.LastSeen
			updated = true
			fmt.Printf("Found better route to %s: %d hops via %s\n", 
				peerID, localPeer.Hops, update.FromPeer)
		}
	}
	
	if updated {
		mn.recalculateRoutes()
		
		// Propagate update to other peers (with decremented TTL)
		if update.TTL > 1 {
			go mn.propagateTopologyUpdate(update)
		}
		
		if mn.onTopologyChange != nil {
			mn.onTopologyChange()
		}
	}
}

// propagateTopologyUpdate forwards topology updates to other peers
func (mn *MeshNetwork) propagateTopologyUpdate(originalUpdate *MeshTopologyUpdate) {
	newUpdate := &MeshTopologyUpdate{
		FromPeer:  mn.nodeID,
		Timestamp: originalUpdate.Timestamp,
		Topology:  originalUpdate.Topology,
		TTL:       originalUpdate.TTL - 1,
	}
	
	connectedPeers := mn.p2pManager.GetPeers()
	for _, peer := range connectedPeers {
		if peer.ID != originalUpdate.FromPeer { // Don't send back to sender
			go mn.sendTopologyUpdate(peer.ID, newUpdate)
		}
	}
}

// recalculateRoutes recalculates routing tables using shortest path
func (mn *MeshNetwork) recalculateRoutes() {
	mn.topologyMutex.RLock()
	defer mn.topologyMutex.RUnlock()
	
	mn.routesMutex.Lock()
	defer mn.routesMutex.Unlock()
	
	// Clear existing routes
	mn.routes = make(map[string][]string)
	
	// Use Dijkstra-like algorithm to find shortest paths
	distances := make(map[string]int)
	previous := make(map[string]string)
	unvisited := make(map[string]bool)
	
	// Initialize
	for peerID := range mn.topology {
		if peerID == mn.nodeID {
			distances[peerID] = 0
		} else {
			distances[peerID] = math.MaxInt32
		}
		unvisited[peerID] = true
	}
	
	for len(unvisited) > 0 {
		// Find unvisited node with minimum distance
		var current string
		minDistance := math.MaxInt32
		for peerID := range unvisited {
			if distances[peerID] < minDistance {
				minDistance = distances[peerID]
				current = peerID
			}
		}
		
		if minDistance == math.MaxInt32 {
			break // No more reachable nodes
		}
		
		delete(unvisited, current)
		
		// Update distances to neighbors
		currentPeer := mn.topology[current]
		for neighborID := range currentPeer.Peers {
			if !unvisited[neighborID] {
				continue
			}
			
			neighborPeer := mn.topology[neighborID]
			if neighborPeer == nil {
				continue
			}
			
			distance := distances[current] + 1
			if distance < distances[neighborID] {
				distances[neighborID] = distance
				previous[neighborID] = current
			}
		}
		
		// Check direct connections for current node
		if current == mn.nodeID {
			connectedPeers := mn.p2pManager.GetPeers()
			for _, peer := range connectedPeers {
				if !unvisited[peer.ID] {
					continue
				}
				
				distance := 1
				if distance < distances[peer.ID] {
					distances[peer.ID] = distance
					previous[peer.ID] = current
				}
			}
		}
	}
	
	// Build routing table
	for peerID := range mn.topology {
		if peerID == mn.nodeID {
			continue
		}
		
		if distances[peerID] == math.MaxInt32 {
			continue // Unreachable
		}
		
		// Trace back the path
		path := []string{}
		current := peerID
		for current != mn.nodeID && current != "" {
			path = append([]string{current}, path...)
			current = previous[current]
		}
		
		if len(path) > 0 {
			mn.routes[peerID] = path
		}
	}
}

// routeMessage routes a message through the mesh network
func (mn *MeshNetwork) routeMessage(message *MeshMessage) error {
	if message.CurrentHop >= message.MaxHops {
		return fmt.Errorf("message exceeded max hops (%d)", message.MaxHops)
	}
	
	// Check if we're the final destination
	if message.FinalTarget == mn.nodeID {
		return mn.handleMeshMessage(message)
	}
	
	// Find route to target
	route := mn.GetRoute(message.FinalTarget)
	if len(route) == 0 {
		return fmt.Errorf("no route to target peer %s", message.FinalTarget)
	}
	
	// Get next hop
	nextHop := route[0]
	message.CurrentHop++
	message.Route = append(message.Route, mn.nodeID)
	
	// Forward message via P2P
	return mn.forwardMeshMessage(nextHop, message)
}

// forwardMeshMessage forwards a mesh message to the next hop
func (mn *MeshNetwork) forwardMeshMessage(nextHopPeerID string, message *MeshMessage) error {
	// This would integrate with the P2P messaging system
	fmt.Printf("Forwarding mesh message from %s to %s via %s\n", 
		message.OriginalSender, message.FinalTarget, nextHopPeerID)
	return nil
}

// handleMeshMessage processes a message that reached its destination
func (mn *MeshNetwork) handleMeshMessage(message *MeshMessage) error {
	fmt.Printf("Received mesh message: %s from %s (hops: %d)\n", 
		message.MessageType, message.OriginalSender, message.CurrentHop)
	
	// Handle different message types
	switch message.MessageType {
	case "topology_request":
		return mn.handleTopologyRequest(message.OriginalSender)
	case "topology_update":
		if update, ok := message.Payload.(*MeshTopologyUpdate); ok {
			mn.HandleTopologyUpdate(update)
		}
	case "ping":
		return mn.sendMeshMessage(message.OriginalSender, "pong", "pong")
	default:
		fmt.Printf("Unknown mesh message type: %s\n", message.MessageType)
	}
	
	return nil
}

// handleTopologyRequest responds to topology requests
func (mn *MeshNetwork) handleTopologyRequest(requesterID string) error {
	topology := mn.GetTopology()
	update := &MeshTopologyUpdate{
		FromPeer:  mn.nodeID,
		Timestamp: time.Now(),
		Topology:  topology,
		TTL:       1, // Don't propagate responses
	}
	
	return mn.SendMeshMessage(requesterID, "topology_update", update)
}

// sendMeshMessage is a helper for sending simple messages
func (mn *MeshNetwork) sendMeshMessage(targetPeerID string, messageType string, payload interface{}) error {
	return mn.SendMeshMessage(targetPeerID, messageType, payload)
}

// healNetwork attempts to heal network partitions
func (mn *MeshNetwork) healNetwork() {
	// Look for peers we know about but aren't connected to
	mn.topologyMutex.RLock()
	knownPeers := make(map[string]*MeshPeer)
	for id, peer := range mn.topology {
		if !peer.DirectConnect && peer.Hops <= 2 {
			knownPeers[id] = peer
		}
	}
	mn.topologyMutex.RUnlock()
	
	// Try to establish direct connections to close peers
	for peerID, peer := range knownPeers {
		if peer.Address != "" && peer.Address != "localhost" {
			go func(id, address string, port int) {
				err := mn.p2pManager.ConnectToPeer(address, port)
				if err == nil {
					fmt.Printf("Healed network: established direct connection to %s\n", id)
				}
			}(peerID, peer.Address, peer.Port)
		}
	}
}

// cleanupStaleEntries removes old topology entries
func (mn *MeshNetwork) cleanupStaleEntries() {
	mn.topologyMutex.Lock()
	defer mn.topologyMutex.Unlock()
	
	cutoff := time.Now().Add(-mn.topologyTTL)
	for peerID, peer := range mn.topology {
		if peerID == mn.nodeID {
			continue
		}
		
		if peer.LastSeen.Before(cutoff) && !peer.DirectConnect {
			delete(mn.topology, peerID)
			fmt.Printf("Cleaned up stale topology entry: %s\n", peerID)
		}
	}
	
	// Recalculate routes after cleanup
	mn.recalculateRoutes()
}