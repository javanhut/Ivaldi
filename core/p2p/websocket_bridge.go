package p2p

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// WebSocketBridge provides a Go interface to the Carrion WebSocket P2P server
type WebSocketBridge struct {
	port          int
	nodeID        string
	carrionCmd    *exec.Cmd
	running       bool
	serverURL     string
}

// WebSocketP2PMessage represents a P2P message structure
type WebSocketP2PMessage struct {
	Type string                 `json:"type"`
	Data map[string]interface{} `json:"data"`
}

// WebSocketP2PResponse represents a response from the P2P server
type WebSocketP2PResponse struct {
	Status  string      `json:"status"`
	Message string      `json:"message,omitempty"`
	Data    interface{} `json:"data,omitempty"`
}

// NewWebSocketBridge creates a new WebSocket P2P bridge
func NewWebSocketBridge(port int, nodeID string) *WebSocketBridge {
	return &WebSocketBridge{
		port:      port,
		nodeID:    nodeID,
		running:   false,
		serverURL: fmt.Sprintf("http://localhost:%d", port),
	}
}

// Start launches the Carrion WebSocket P2P server
func (wsb *WebSocketBridge) Start() error {
	if wsb.running {
		return fmt.Errorf("WebSocket P2P bridge is already running")
	}

	// Check if carrion is available
	if _, err := exec.LookPath("carrion"); err != nil {
		return fmt.Errorf("Carrion language not found in PATH. Please install Carrion: %v", err)
	}

	// Launch the Carrion WebSocket server
	carrionScript := "/home/javanstorm/Ivaldi/core/p2p/websocket_server.crl"
	wsb.carrionCmd = exec.Command("carrion", carrionScript, strconv.Itoa(wsb.port))
	
	// Set up process group for clean shutdown
	wsb.carrionCmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	
	// Redirect output for debugging
	wsb.carrionCmd.Stdout = os.Stdout
	wsb.carrionCmd.Stderr = os.Stderr

	// Start the Carrion process
	if err := wsb.carrionCmd.Start(); err != nil {
		return fmt.Errorf("failed to start Carrion WebSocket server: %v", err)
	}

	wsb.running = true

	// Wait a moment for the server to start up
	time.Sleep(2 * time.Second)

	// Verify the server is responding
	if err := wsb.ping(); err != nil {
		wsb.Stop()
		return fmt.Errorf("WebSocket server started but not responding: %v", err)
	}

	fmt.Printf("WebSocket P2P bridge started on port %d (PID: %d)\n", wsb.port, wsb.carrionCmd.Process.Pid)
	return nil
}

// Stop shuts down the Carrion WebSocket P2P server
func (wsb *WebSocketBridge) Stop() error {
	if !wsb.running || wsb.carrionCmd == nil {
		return nil
	}

	// Send SIGTERM to the process group
	if err := syscall.Kill(-wsb.carrionCmd.Process.Pid, syscall.SIGTERM); err != nil {
		// If SIGTERM fails, force kill
		wsb.carrionCmd.Process.Kill()
	}

	// Wait for process to exit
	wsb.carrionCmd.Wait()

	wsb.running = false
	wsb.carrionCmd = nil

	fmt.Printf("WebSocket P2P bridge stopped\n")
	return nil
}

// IsRunning returns whether the WebSocket bridge is running
func (wsb *WebSocketBridge) IsRunning() bool {
	if !wsb.running {
		return false
	}

	// Check if process is still alive
	if wsb.carrionCmd == nil || wsb.carrionCmd.Process == nil {
		wsb.running = false
		return false
	}

	// Send signal 0 to check if process exists
	if err := wsb.carrionCmd.Process.Signal(syscall.Signal(0)); err != nil {
		wsb.running = false
		return false
	}

	return true
}

// SendMessage sends a P2P message to the WebSocket server
func (wsb *WebSocketBridge) SendMessage(messageType string, data map[string]interface{}) (*WebSocketP2PResponse, error) {
	if !wsb.IsRunning() {
		return nil, fmt.Errorf("WebSocket P2P bridge is not running")
	}

	message := WebSocketP2PMessage{
		Type: messageType,
		Data: data,
	}

	jsonData, err := json.Marshal(message)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal message: %v", err)
	}

	// Send HTTP POST request to the Carrion server
	resp, err := http.Post(wsb.serverURL, "application/json", strings.NewReader(string(jsonData)))
	if err != nil {
		return nil, fmt.Errorf("failed to send message: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("server returned status %d", resp.StatusCode)
	}

	responseBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response: %v", err)
	}

	var p2pResponse WebSocketP2PResponse
	if err := json.Unmarshal(responseBody, &p2pResponse); err != nil {
		return nil, fmt.Errorf("failed to unmarshal response: %v", err)
	}

	return &p2pResponse, nil
}

// ConnectToPeer connects to another P2P peer
func (wsb *WebSocketBridge) ConnectToPeer(address string, port int) error {
	data := map[string]interface{}{
		"address": address,
		"port":    port,
	}

	response, err := wsb.SendMessage("connect_peer", data)
	if err != nil {
		return err
	}

	if response.Status != "success" {
		return fmt.Errorf("failed to connect to peer: %s", response.Message)
	}

	fmt.Printf("Successfully connected to peer %s:%d\n", address, port)
	return nil
}

// GetStatus returns the current status of the P2P network
func (wsb *WebSocketBridge) GetStatus() (map[string]interface{}, error) {
	data := map[string]interface{}{
		"peer_id": wsb.nodeID,
	}

	response, err := wsb.SendMessage("get_status", data)
	if err != nil {
		return nil, err
	}

	if statusData, ok := response.Data.(map[string]interface{}); ok {
		return statusData, nil
	}

	return map[string]interface{}{
		"running":    wsb.running,
		"node_id":    wsb.nodeID,
		"port":       wsb.port,
		"peer_count": 0,
	}, nil
}

// DiscoverPeers sends a peer discovery message
func (wsb *WebSocketBridge) DiscoverPeers() error {
	data := map[string]interface{}{
		"peer_id": wsb.nodeID,
		"port":    wsb.port,
	}

	_, err := wsb.SendMessage("peer_discovery", data)
	return err
}

// SyncWithPeer requests synchronization with a specific peer
func (wsb *WebSocketBridge) SyncWithPeer(peerID string, syncType string, target string) error {
	data := map[string]interface{}{
		"peer_id":   peerID,
		"sync_type": syncType,
		"target":    target,
	}

	response, err := wsb.SendMessage("sync_request", data)
	if err != nil {
		return err
	}

	if response.Status != "success" {
		return fmt.Errorf("sync failed: %s", response.Message)
	}

	fmt.Printf("Sync completed with peer %s\n", peerID)
	return nil
}

// ping sends a ping message to verify the server is responding
func (wsb *WebSocketBridge) ping() error {
	data := map[string]interface{}{
		"peer_id": wsb.nodeID,
	}

	response, err := wsb.SendMessage("ping", data)
	if err != nil {
		return err
	}

	if response.Status != "pong" {
		return fmt.Errorf("unexpected ping response: %s", response.Status)
	}

	return nil
}

// BroadcastTopologyUpdate sends topology information to peers
func (wsb *WebSocketBridge) BroadcastTopologyUpdate(topology map[string]interface{}) error {
	data := map[string]interface{}{
		"from_peer": wsb.nodeID,
		"topology":  topology,
	}

	_, err := wsb.SendMessage("topology_update", data)
	return err
}