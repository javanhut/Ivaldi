package enhanced_cli

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/spf13/cobra"
)

// Mesh command implementations

func (ec *EnhancedCLI) createMeshStartCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "start",
		Short: "Start mesh networking",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			ec.output.Info("Starting mesh network...")
			if err := repo.StartMesh(); err != nil {
				return fmt.Errorf("failed to start mesh network: %v", err)
			}

			status := repo.GetMeshStatus()
			ec.output.Success("Mesh network started successfully!")
			ec.output.Info(fmt.Sprintf("Node ID: %s", status.NodeID))
			ec.output.Info("Network will automatically discover and connect to other mesh peers")
			
			// Check if daemon mode is requested
			daemon, _ := cmd.Flags().GetBool("daemon")
			if daemon {
				ec.output.Info("Running in daemon mode. Press Ctrl+C to stop.")
				// Keep the process running
				select {}
			}
			
			return nil
		},
	}
	
	cmd.Flags().BoolP("daemon", "d", false, "Run as daemon (keep process running)")
	return cmd
}

func (ec *EnhancedCLI) createMeshStopCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "stop",
		Short: "Stop mesh networking",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				ec.output.Info("Mesh network is not running")
				return nil
			}

			if err := repo.StopMesh(); err != nil {
				return fmt.Errorf("failed to stop mesh network: %v", err)
			}

			ec.output.Success("Mesh network stopped")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshStatusCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "Show mesh network status",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			status := repo.GetMeshStatus()
			
			if !status.Running {
				ec.output.Info("Mesh Network: STOPPED")
				ec.output.Info("Start with: mesh start")
				return nil
			}

			ec.output.Info("=== Mesh Network Status ===")
			ec.output.Info(fmt.Sprintf("Status: RUNNING"))
			ec.output.Info(fmt.Sprintf("Node ID: %s", status.NodeID))
			ec.output.Info(fmt.Sprintf("Total Peers: %d", status.PeerCount))
			ec.output.Info(fmt.Sprintf("Direct Peers: %d", status.DirectPeers))
			ec.output.Info(fmt.Sprintf("Indirect Peers: %d", status.IndirectPeers))
			ec.output.Info(fmt.Sprintf("Max Hops: %d", status.MaxHops))
			ec.output.Info(fmt.Sprintf("Avg Hops: %.1f", status.AvgHops))

			if status.PeerCount > 0 {
				ec.output.Info("\n=== Connected Peers ===")
				for peerID, peer := range status.Topology {
					if peerID == status.NodeID {
						continue
					}
					connectType := "direct"
					if !peer.DirectConnect {
						connectType = fmt.Sprintf("via %s (%d hops)", peer.NextHop, peer.Hops)
					}
					ec.output.Info(fmt.Sprintf("%s - %s:%d (%s)", 
						peerID[:12], peer.Address, peer.Port, connectType))
				}
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshJoinCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "join <address:port>",
		Short: "Join mesh network via bootstrap peer",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			// Parse address:port
			parts := strings.Split(args[0], ":")
			if len(parts) != 2 {
				return fmt.Errorf("invalid address format, use: address:port")
			}

			address := parts[0]
			port, err := strconv.Atoi(parts[1])
			if err != nil {
				return fmt.Errorf("invalid port number: %v", err)
			}

			// Start mesh if not running
			if !repo.IsMeshRunning() {
				ec.output.Info("Starting mesh network first...")
				if err := repo.StartMesh(); err != nil {
					return fmt.Errorf("failed to start mesh network: %v", err)
				}
			}

			ec.output.Info(fmt.Sprintf("Joining mesh network via %s:%d...", address, port))
			if err := repo.JoinMesh(address, port); err != nil {
				return fmt.Errorf("failed to join mesh: %v", err)
			}

			ec.output.Success("Successfully joined mesh network!")
			ec.output.Info("Discovering peers and building topology...")

			// Show brief status after join
			time.Sleep(2 * time.Second)
			status := repo.GetMeshStatus()
			ec.output.Info(fmt.Sprintf("Discovered %d peers in the mesh", status.PeerCount))

			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshTopologyCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "topology",
		Short: "Show network topology",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			topology := repo.GetMeshTopology()
			status := repo.GetMeshStatus()

			ec.output.Info("=== Mesh Network Topology ===")
			ec.output.Info(fmt.Sprintf("Local Node: %s", status.NodeID))
			
			if len(topology) <= 1 {
				ec.output.Info("No peers discovered yet")
				return nil
			}

			ec.output.Info("\n=== Peer Nodes ===")
			for peerID, peer := range topology {
				if peerID == status.NodeID {
					continue
				}

				connectionInfo := "DIRECT"
				if !peer.DirectConnect {
					connectionInfo = fmt.Sprintf("via %s (%d hops)", peer.NextHop, peer.Hops)
				}

				ec.output.Info(fmt.Sprintf("├─ %s", peerID))
				ec.output.Info(fmt.Sprintf("│  Address: %s:%d", peer.Address, peer.Port))
				ec.output.Info(fmt.Sprintf("│  Connection: %s", connectionInfo))
				ec.output.Info(fmt.Sprintf("│  Last Seen: %s", peer.LastSeen.Format("15:04:05")))
				ec.output.Info(fmt.Sprintf("│  Capabilities: %v", peer.Capabilities))
				ec.output.Info("")
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshRouteCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "route <peer-id>",
		Short: "Show route to specific peer",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			targetPeer := args[0]
			route := repo.GetMeshRoute(targetPeer)

			if len(route) == 0 {
				return fmt.Errorf("no route found to peer %s", targetPeer)
			}

			ec.output.Info(fmt.Sprintf("Route to %s:", targetPeer))
			
			status := repo.GetMeshStatus()
			ec.output.Info(fmt.Sprintf("└─ %s (local)", status.NodeID[:12]))
			
			for i, hop := range route {
				if i == len(route)-1 {
					ec.output.Info(fmt.Sprintf("   └─ %s (target)", hop[:12]))
				} else {
					ec.output.Info(fmt.Sprintf("   └─ %s", hop[:12]))
				}
			}
			
			ec.output.Info(fmt.Sprintf("Total hops: %d", len(route)))
			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshPingCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "ping <peer-id>",
		Short: "Ping peer via mesh routing",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			targetPeer := args[0]
			
			ec.output.Info(fmt.Sprintf("Pinging %s via mesh network...", targetPeer[:12]))
			
			start := time.Now()
			if err := repo.PingMeshPeer(targetPeer); err != nil {
				return fmt.Errorf("ping failed: %v", err)
			}
			duration := time.Since(start)

			route := repo.GetMeshRoute(targetPeer)
			hops := len(route)
			
			ec.output.Success(fmt.Sprintf("Ping successful! (%v, %d hops)", duration, hops))
			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshPeersCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "peers",
		Short: "List all mesh peers",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			peers := repo.GetMeshPeers()
			
			if len(peers) == 0 {
				ec.output.Info("No peers discovered yet")
				return nil
			}

			ec.output.Info(fmt.Sprintf("=== Mesh Peers (%d) ===", len(peers)))
			
			directPeers := repo.GetDirectMeshPeers()
			indirectPeers := repo.GetIndirectMeshPeers()
			
			if len(directPeers) > 0 {
				ec.output.Info(fmt.Sprintf("\nDirect Connections (%d):", len(directPeers)))
				for _, peer := range directPeers {
					ec.output.Info(fmt.Sprintf("  %s - %s:%d (1 hop)", 
						peer.ID[:12], peer.Address, peer.Port))
				}
			}
			
			if len(indirectPeers) > 0 {
				ec.output.Info(fmt.Sprintf("\nIndirect Connections (%d):", len(indirectPeers)))
				for _, peer := range indirectPeers {
					ec.output.Info(fmt.Sprintf("  %s - %s:%d (%d hops via %s)", 
						peer.ID[:12], peer.Address, peer.Port, peer.Hops, peer.NextHop[:12]))
				}
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshHealCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "heal",
		Short: "Manually trigger network healing",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			ec.output.Info("Triggering mesh network healing...")
			if err := repo.HealMeshNetwork(); err != nil {
				return fmt.Errorf("healing failed: %v", err)
			}

			ec.output.Success("Network healing initiated")
			ec.output.Info("Attempting to establish better connections and heal partitions...")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshRefreshCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "refresh",
		Short: "Refresh topology information",
		RunE: func(cmd *cobra.Command, args []string) error {
			repo := ec.currentRepo

			if !repo.IsMeshRunning() {
				return fmt.Errorf("mesh network is not running")
			}

			ec.output.Info("Refreshing mesh topology...")
			if err := repo.RefreshMeshTopology(); err != nil {
				return fmt.Errorf("refresh failed: %v", err)
			}

			ec.output.Success("Topology refresh initiated")
			ec.output.Info("Requesting updated topology from all peers...")
			return nil
		},
	}
}