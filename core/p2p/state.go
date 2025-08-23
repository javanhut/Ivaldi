package p2p

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
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

	if err := os.WriteFile(statePath, data, 0644); err != nil {
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
		// Check if process exists by trying to send signal 0 (no-op signal)
		process, err := os.FindProcess(state.PID)
		if err != nil {
			// Process doesn't exist, P2P is not running
			state.Running = false
		} else {
			// Try to send signal 0 to check if process is alive
			// On Unix systems, signal 0 can be used to check if a process exists
			err := process.Signal(syscall.Signal(0))
			if err != nil {
				// Process is not running or not accessible
				state.Running = false
			} else {
				// Process is running
				state.Running = true
			}
		}
		// Update state file with corrected running status
		sm.Save(&state)
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