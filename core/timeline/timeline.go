package timeline

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"ivaldi/core/objects"
	"ivaldi/pkg/storage/objectstore"
	"ivaldi/pkg/storage/worktree"
)

type Timeline struct {
	Name        string
	Head        objects.Hash
	CreatedAt   time.Time
	UpdatedAt   time.Time
	Description string
	Parent      string
}

type Manager struct {
	root      string
	current   string
	timelines map[string]*Timeline
}

func NewManager(root string) *Manager {
	return &Manager{
		root:      root,
		timelines: make(map[string]*Timeline),
	}
}

func (m *Manager) Initialize() error {
	// Check for and recover from any incomplete timeline switches
	store := objectstore.New(m.root)
	if err := worktree.RecoverWAL(m.root, store); err != nil {
		return fmt.Errorf("failed to recover from incomplete switch: %v", err)
	}
	
	timelineDir := filepath.Join(m.root, ".ivaldi", "timelines")
	if err := os.MkdirAll(timelineDir, 0755); err != nil {
		return err
	}

	mainTimeline := &Timeline{
		Name:        "main",
		CreatedAt:   time.Now(),
		UpdatedAt:   time.Now(),
		Description: "Main development timeline",
	}

	m.timelines["main"] = mainTimeline
	m.current = "main"

	return m.save()
}

func (m *Manager) Create(name, description string) error {
	if _, exists := m.timelines[name]; exists {
		return fmt.Errorf("timeline '%s' already exists", name)
	}

	currentTimeline := m.timelines[m.current]
	newTimeline := &Timeline{
		Name:        name,
		Head:        currentTimeline.Head,
		CreatedAt:   time.Now(),
		UpdatedAt:   time.Now(),
		Description: description,
		Parent:      m.current,
	}

	m.timelines[name] = newTimeline
	return m.save()
}

func (m *Manager) Switch(name string) error {
	if _, exists := m.timelines[name]; !exists {
		return fmt.Errorf("timeline '%s' does not exist", name)
	}

	// Simple switch - just update the current timeline
	// The actual file handling is done by forge/repository.go
	m.current = name
	return m.save()
}

func (m *Manager) Current() string {
	return m.current
}

func (m *Manager) List() []*Timeline {
	var result []*Timeline
	for _, timeline := range m.timelines {
		result = append(result, timeline)
	}
	return result
}

func (m *Manager) Get(name string) (*Timeline, bool) {
	timeline, exists := m.timelines[name]
	return timeline, exists
}

func (m *Manager) UpdateHead(name string, hash objects.Hash) error {
	timeline, exists := m.timelines[name]
	if !exists {
		return fmt.Errorf("timeline '%s' does not exist", name)
	}

	timeline.Head = hash
	timeline.UpdatedAt = time.Now()
	return m.save()
}

func (m *Manager) Load() error {
	configPath := filepath.Join(m.root, ".ivaldi", "timelines", "config.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}

	var config struct {
		Current   string               `json:"current"`
		Timelines map[string]*Timeline `json:"timelines"`
	}

	if err := json.Unmarshal(data, &config); err != nil {
		return err
	}

	m.current = config.Current
	m.timelines = config.Timelines
	return nil
}

func (m *Manager) GetHead(name string) (objects.Hash, error) {
	timeline, exists := m.timelines[name]
	if !exists {
		return objects.Hash{}, fmt.Errorf("timeline '%s' does not exist", name)
	}
	return timeline.Head, nil
}

func (m *Manager) Delete(name string) error {
	if name == "main" {
		return fmt.Errorf("cannot delete main timeline")
	}
	
	if name == m.current {
		return fmt.Errorf("cannot delete current timeline, switch to another timeline first")
	}
	
	if _, exists := m.timelines[name]; !exists {
		return fmt.Errorf("timeline '%s' does not exist", name)
	}
	
	delete(m.timelines, name)
	return m.save()
}

// DeleteTimeline is an alias for Delete to match the fuse interface
func (m *Manager) DeleteTimeline(name string) error {
	return m.Delete(name)
}

func (m *Manager) save() error {
	configPath := filepath.Join(m.root, ".ivaldi", "timelines", "config.json")
	
	config := struct {
		Current   string               `json:"current"`
		Timelines map[string]*Timeline `json:"timelines"`
	}{
		Current:   m.current,
		Timelines: m.timelines,
	}

	data, err := json.Marshal(config)
	if err != nil {
		return err
	}

	return os.WriteFile(configPath, data, 0644)
}