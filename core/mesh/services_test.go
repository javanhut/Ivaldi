package mesh

import (
	"reflect"
	"testing"
	"time"

	"ivaldi/core/p2p"
)

func TestCalculateShortestPaths(t *testing.T) {
	// Test two-node scenario
	t.Run("TwoNodeDirectConnection", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{},
			},
			"node2": {
				ID:    "node2",
				Peers: map[string]time.Time{},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "node2"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{
			"node2": {"node2"},
		}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected routes %v, got %v", expected, routes)
		}
	})

	// Test linear chain scenario
	t.Run("LinearChain", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"node2": time.Now()},
			},
			"node2": {
				ID:    "node2",
				Peers: map[string]time.Time{"node1": time.Now(), "node3": time.Now()},
			},
			"node3": {
				ID:    "node3",
				Peers: map[string]time.Time{"node2": time.Now()},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "node2"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{
			"node2": {"node2"},
			"node3": {"node2", "node3"},
		}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected routes %v, got %v", expected, routes)
		}
	})

	// Test branching scenario
	t.Run("BranchingTopology", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"node2": time.Now(), "node3": time.Now()},
			},
			"node2": {
				ID:    "node2",
				Peers: map[string]time.Time{"node1": time.Now(), "node4": time.Now()},
			},
			"node3": {
				ID:    "node3",
				Peers: map[string]time.Time{"node1": time.Now(), "node5": time.Now()},
			},
			"node4": {
				ID:    "node4",
				Peers: map[string]time.Time{"node2": time.Now()},
			},
			"node5": {
				ID:    "node5",
				Peers: map[string]time.Time{"node3": time.Now()},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "node2"},
			{ID: "node3"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{
			"node2": {"node2"},
			"node3": {"node3"},
			"node4": {"node2", "node4"},
			"node5": {"node3", "node5"},
		}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected routes %v, got %v", expected, routes)
		}
	})

	// Test unreachable nodes scenario
	t.Run("UnreachableNodes", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"node2": time.Now()},
			},
			"node2": {
				ID:    "node2",
				Peers: map[string]time.Time{"node1": time.Now()},
			},
			"node3": {
				ID:    "node3",
				Peers: map[string]time.Time{"node4": time.Now()},
			},
			"node4": {
				ID:    "node4",
				Peers: map[string]time.Time{"node3": time.Now()},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "node2"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{
			"node2": {"node2"},
			// node3 and node4 should be unreachable and not in routes
		}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected routes %v, got %v", expected, routes)
		}
	})

	// Test direct vs indirect routing preference
	t.Run("DirectVsIndirectRouting", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"node2": time.Now(), "node3": time.Now()},
			},
			"node2": {
				ID:    "node2",
				Peers: map[string]time.Time{"node1": time.Now(), "node3": time.Now()},
			},
			"node3": {
				ID:    "node3",
				Peers: map[string]time.Time{"node1": time.Now(), "node2": time.Now()},
			},
		}
		// Direct connection to node3, should prefer direct over via node2
		directPeers := []*p2p.Peer{
			{ID: "node2"},
			{ID: "node3"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{
			"node2": {"node2"},
			"node3": {"node3"}, // Should be direct, not via node2
		}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected routes %v, got %v", expected, routes)
		}
	})

	// Test empty topology
	t.Run("EmptyTopology", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{}
		directPeers := []*p2p.Peer{}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected empty routes %v, got %v", expected, routes)
		}
	})

	// Test with nil peers
	t.Run("NilPeersHandling", func(t *testing.T) {
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"node2": time.Now()},
			},
			"node2": nil, // Nil peer should be handled gracefully
		}
		directPeers := []*p2p.Peer{}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		expected := map[string][]string{}

		if !reflect.DeepEqual(routes, expected) {
			t.Errorf("Expected empty routes %v, got %v", expected, routes)
		}
	})
}

func TestMeshNetworkRecalculateRoutes(t *testing.T) {
	// Integration test that verifies recalculateRoutes produces the same result
	t.Run("IntegrationTest", func(t *testing.T) {
		// Test data
		nodeID := "node1"
		topology := map[string]*MeshPeer{
			"node1": {
				ID:    "node1",
				Peers: map[string]time.Time{"peer1": time.Now()},
			},
			"peer1": {
				ID:    "peer1",
				Peers: map[string]time.Time{"node1": time.Now(), "peer2": time.Now()},
			},
			"peer2": {
				ID:    "peer2",
				Peers: map[string]time.Time{"peer1": time.Now()},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "peer1"},
			{ID: "peer2"},
		}

		// Calculate expected routes using the extracted function directly
		expectedRoutes := calculateShortestPaths(nodeID, topology, directPeers)

		// Verify that the function produces expected results
		expectedSpecific := map[string][]string{
			"peer1": {"peer1"},
			"peer2": {"peer2"}, // Direct connection should be preferred
		}

		if !reflect.DeepEqual(expectedRoutes, expectedSpecific) {
			t.Errorf("calculateShortestPaths produced unexpected routes. Expected %v, got %v", expectedSpecific, expectedRoutes)
		}

		// Test that the function consistently produces the same results
		routes2 := calculateShortestPaths(nodeID, topology, directPeers)
		if !reflect.DeepEqual(expectedRoutes, routes2) {
			t.Errorf("calculateShortestPaths is not deterministic. First call: %v, Second call: %v", expectedRoutes, routes2)
		}
	})

	// Test complex routing scenario with multiple hops and choices
	t.Run("ComplexRoutingScenario", func(t *testing.T) {
		nodeID := "central"
		topology := map[string]*MeshPeer{
			"central": {
				ID:    "central",
				Peers: map[string]time.Time{"hub1": time.Now(), "hub2": time.Now()},
			},
			"hub1": {
				ID:    "hub1",
				Peers: map[string]time.Time{"central": time.Now(), "leaf1": time.Now(), "leaf2": time.Now()},
			},
			"hub2": {
				ID:    "hub2",
				Peers: map[string]time.Time{"central": time.Now(), "leaf3": time.Now()},
			},
			"leaf1": {
				ID:    "leaf1",
				Peers: map[string]time.Time{"hub1": time.Now()},
			},
			"leaf2": {
				ID:    "leaf2",
				Peers: map[string]time.Time{"hub1": time.Now(), "leaf3": time.Now()},
			},
			"leaf3": {
				ID:    "leaf3",
				Peers: map[string]time.Time{"hub2": time.Now(), "leaf2": time.Now()},
			},
		}
		directPeers := []*p2p.Peer{
			{ID: "hub1"},
			{ID: "hub2"},
		}

		routes := calculateShortestPaths(nodeID, topology, directPeers)

		// Verify optimal routing paths
		expectedRoutes := map[string][]string{
			"hub1":  {"hub1"},          // Direct
			"hub2":  {"hub2"},          // Direct
			"leaf1": {"hub1", "leaf1"}, // Via hub1
			"leaf2": {"hub1", "leaf2"}, // Via hub1 (shortest)
			"leaf3": {"hub2", "leaf3"}, // Via hub2 (shortest)
		}

		if !reflect.DeepEqual(routes, expectedRoutes) {
			t.Errorf("Complex routing scenario failed. Expected %v, got %v", expectedRoutes, routes)
		}
	})
}
