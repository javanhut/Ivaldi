package p2p

import (
	"encoding/json"
	"fmt"
	"ivaldi/core/logging"
	"os"
	"path/filepath"
	"time"
)

// P2PState represents the persistent state of the P2P network
type P2PState struct {
	Running       bool      `json:"running"`
	NodeID        string    `json:"node_id"`
	Port          int       `json:"port"`
	DiscoveryPort int       `json:"discovery_port"`
	StartedAt     time.Time `json:"started_at"`
	PID           int       `json:"pid"`
}

// P2PStateManager handles persistent P2P state
type P2PStateManager struct {
	rootDir string
}

// NewP2PStateManager creates a new state manager
func NewP2PStateManager(rootDir string) *P2PStateManager {
	return &P2PStateManager{
		rootDir: rootDir,
	}
}

// GetStatePath returns the path to the state file
func (sm *P2PStateManager) GetStatePath() string {
	return filepath.Join(sm.rootDir, ".ivaldi", "p2p.state")
}

// Save saves the P2P state to disk
func (sm *P2PStateManager) Save(state *P2PState) error {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal P2P state: %v", err)
	}

	statePath := sm.GetStatePath()
	if err := os.MkdirAll(filepath.Dir(statePath), 0755); err != nil {
		return fmt.Errorf("failed to create state directory: %v", err)
	}

	if err := os.WriteFile(statePath, data, 0600); err != nil {
		return fmt.Errorf("failed to write P2P state: %v", err)
	}

	return nil
}

// Load loads the P2P state from disk
func (sm *P2PStateManager) Load() (*P2PState, error) {
	statePath := sm.GetStatePath()
	data, err := os.ReadFile(statePath)
	if err != nil {
		if os.IsNotExist(err) {
			// No state file means P2P is not running
			return &P2PState{Running: false}, nil
		}
		return nil, fmt.Errorf("failed to read P2P state: %v", err)
	}

	var state P2PState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("failed to unmarshal P2P state: %v", err)
	}

	// Check if the process is still running - ignore the "running" field in the state
	// and check if the PID is actually running
	if state.PID > 0 {
		oldRunning := state.Running
		state.Running = isProcessAlive(state.PID)
		
		// Only save state if the running status changed
		if state.Running != oldRunning {
			if err := sm.Save(&state); err != nil {
				logging.Error("Failed to save P2P state after process liveness check",
					"node_id", state.NodeID,
					"pid", state.PID,
					"running", state.Running,
					"error", err)
				return nil, fmt.Errorf("failed to update P2P state: %v", err)
			}
		}
	}

	return &state, nil
}

// Clear clears the P2P state
func (sm *P2PStateManager) Clear() error {
	statePath := sm.GetStatePath()
	if err := os.Remove(statePath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("failed to clear P2P state: %v", err)
	}
	return nil
}

// IsRunning checks if P2P is running based on the state file
func (sm *P2PStateManager) IsRunning() bool {
	state, err := sm.Load()
	if err != nil {
		return false
	}
	return state.Running
}

// GetRunningState returns the running P2P state if it exists
func (sm *P2PStateManager) GetRunningState() (*P2PState, bool) {
	state, err := sm.Load()
	if err != nil || !state.Running {
		return nil, false
	}
	return state, true
}