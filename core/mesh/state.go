package mesh

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

// MeshState represents the persistent state of the mesh network
type MeshState struct {
	Running       bool      `json:"running"`
	NodeID        string    `json:"node_id"`
	Port          int       `json:"port"`
	StartedAt     time.Time `json:"started_at"`
	PID           int       `json:"pid"`
	TopologyCount int       `json:"topology_count"`
}

// MeshStateManager handles persistent mesh state
type MeshStateManager struct {
	rootDir string
}

// NewMeshStateManager creates a new state manager
func NewMeshStateManager(rootDir string) *MeshStateManager {
	return &MeshStateManager{
		rootDir: rootDir,
	}
}

// GetStatePath returns the path to the state file
func (sm *MeshStateManager) GetStatePath() string {
	return filepath.Join(sm.rootDir, ".ivaldi", "mesh.state")
}

// Save saves the mesh state to disk
func (sm *MeshStateManager) Save(state *MeshState) error {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal mesh state: %v", err)
	}

	statePath := sm.GetStatePath()
	if err := os.MkdirAll(filepath.Dir(statePath), 0755); err != nil {
		return fmt.Errorf("failed to create state directory: %v", err)
	}

	if err := os.WriteFile(statePath, data, 0644); err != nil {
		return fmt.Errorf("failed to write mesh state: %v", err)
	}

	return nil
}

// Load loads the mesh state from disk
func (sm *MeshStateManager) Load() (*MeshState, error) {
	statePath := sm.GetStatePath()
	data, err := os.ReadFile(statePath)
	if err != nil {
		if os.IsNotExist(err) {
			// No state file means mesh is not running
			return &MeshState{Running: false}, nil
		}
		return nil, fmt.Errorf("failed to read mesh state: %v", err)
	}

	var state MeshState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("failed to unmarshal mesh state: %v", err)
	}

	// Check if the process is still running - ignore the "running" field in the state
	// and check if the PID is actually running
	if state.PID > 0 {
		// Check if process exists by trying to send signal 0 (no-op signal)
		process, err := os.FindProcess(state.PID)
		if err != nil {
			// Process doesn't exist, mesh is not running
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

// Clear clears the mesh state
func (sm *MeshStateManager) Clear() error {
	statePath := sm.GetStatePath()
	if err := os.Remove(statePath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("failed to clear mesh state: %v", err)
	}
	return nil
}

// IsRunning checks if mesh is running based on the state file
func (sm *MeshStateManager) IsRunning() bool {
	state, err := sm.Load()
	if err != nil {
		return false
	}
	return state.Running
}

// GetRunningState returns the running mesh state if it exists
func (sm *MeshStateManager) GetRunningState() (*MeshState, bool) {
	state, err := sm.Load()
	if err != nil || !state.Running {
		return nil, false
	}
	return state, true
}