package mesh

import (
	"encoding/json"
	"fmt"
	"ivaldi/core/logging"
	"os"
	"path/filepath"
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

	if err := os.WriteFile(statePath, data, 0600); err != nil {
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
		oldRunning := state.Running
		state.Running = isProcessAlive(state.PID)

		// Only save state if the running status changed
		if state.Running != oldRunning {
			if err := sm.Save(&state); err != nil {
				logging.Error("Failed to save mesh state after process liveness check",
					"node_id", state.NodeID,
					"pid", state.PID,
					"running", state.Running,
					"error", err)
				return nil, fmt.Errorf("failed to update mesh state: %v", err)
			}
		}
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
