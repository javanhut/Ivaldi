package position

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/objects"
)

type Position struct {
	Hash      objects.Hash
	Timeline  string
	Timestamp time.Time
}

// ReferenceResolver interface for natural language resolution
type ReferenceResolver interface {
	Resolve(ref string, currentTimeline string) (objects.Hash, error)
}

type Manager struct {
	root      string
	current   Position
	history   []Position
	aliases   map[string]objects.Hash
	nameMap   map[objects.Hash]string
	refMgr    ReferenceResolver
}

func NewManager(root string) *Manager {
	return &Manager{
		root:    root,
		aliases: make(map[string]objects.Hash),
		nameMap: make(map[objects.Hash]string),
	}
}

func (m *Manager) SetReferenceResolver(refMgr ReferenceResolver) {
	m.refMgr = refMgr
}

func (m *Manager) SetPosition(hash objects.Hash, timeline string) error {
	m.current = Position{
		Hash:      hash,
		Timeline:  timeline,
		Timestamp: time.Now(),
	}
	
	m.history = append(m.history, m.current)
	return m.save()
}

func (m *Manager) Current() Position {
	return m.current
}

func (m *Manager) ParseReference(ref string) (objects.Hash, error) {
	ref = strings.TrimSpace(ref)
	
	// Check local aliases first
	if hash, exists := m.aliases[ref]; exists {
		return hash, nil
	}
	
	if ref == "position" || ref == "" {
		return m.current.Hash, nil
	}
	
	// Try the advanced reference manager first if available
	if m.refMgr != nil {
		if hash, err := m.refMgr.Resolve(ref, m.current.Timeline); err == nil {
			return hash, nil
		}
	}
	
	// Fall back to local resolution for basic cases
	if strings.HasPrefix(ref, "#-") {
		return m.parseRelativeIteration(ref[2:])
	}
	
	if strings.HasPrefix(ref, "#") {
		return m.parseIteration(ref[1:])
	}
	
	if name, err := m.parseMemorableName(ref); err == nil {
		return name, nil
	}
	
	if time, err := m.parseNaturalLanguage(ref); err == nil {
		return time, nil
	}
	
	return objects.Hash{}, fmt.Errorf("unknown reference: %s", ref)
}

func (m *Manager) parseIteration(iter string) (objects.Hash, error) {
	num, err := strconv.Atoi(iter)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("invalid iteration number: %s", iter)
	}
	
	if num < 0 || num >= len(m.history) {
		return objects.Hash{}, fmt.Errorf("iteration %d out of range", num)
	}
	
	return m.history[num].Hash, nil
}

func (m *Manager) parseRelativeIteration(iter string) (objects.Hash, error) {
	num, err := strconv.Atoi(iter)
	if err != nil {
		return objects.Hash{}, fmt.Errorf("invalid relative iteration: %s", iter)
	}
	
	if num <= 0 {
		return objects.Hash{}, fmt.Errorf("relative iteration must be positive: -%d", num)
	}
	
	if len(m.history) == 0 {
		return objects.Hash{}, fmt.Errorf("no history available")
	}
	
	idx := len(m.history) - 1 - num
	if idx < 0 {
		return objects.Hash{}, fmt.Errorf("relative iteration -%d out of range (only %d entries available)", num, len(m.history))
	}
	
	return m.history[idx].Hash, nil
}

func (m *Manager) parseMemorableName(name string) (objects.Hash, error) {
	for hash, memorable := range m.nameMap {
		if memorable == name {
			return hash, nil
		}
	}
	return objects.Hash{}, fmt.Errorf("memorable name not found: %s", name)
}

func (m *Manager) parseNaturalLanguage(ref string) (objects.Hash, error) {
	ref = strings.ToLower(ref)
	now := time.Now()
	
	switch {
	case strings.Contains(ref, "yesterday"):
		target := now.AddDate(0, 0, -1)
		return m.findClosestByTime(target)
	case strings.Contains(ref, "last week"):
		target := now.AddDate(0, 0, -7)
		return m.findClosestByTime(target)
	case strings.Contains(ref, "last month"):
		target := now.AddDate(0, -1, 0)
		return m.findClosestByTime(target)
	}
	
	return objects.Hash{}, fmt.Errorf("unsupported natural language reference: %s", ref)
}

func (m *Manager) findClosestByTime(target time.Time) (objects.Hash, error) {
	if len(m.history) == 0 {
		return objects.Hash{}, fmt.Errorf("no history available")
	}
	
	closest := m.history[0]
	minDiff := target.Sub(closest.Timestamp)
	if minDiff < 0 {
		minDiff = -minDiff
	}
	
	for _, pos := range m.history[1:] {
		diff := target.Sub(pos.Timestamp)
		if diff < 0 {
			diff = -diff
		}
		if diff < minDiff {
			minDiff = diff
			closest = pos
		}
	}
	
	return closest.Hash, nil
}

func (m *Manager) AddAlias(alias string, hash objects.Hash) error {
	m.aliases[alias] = hash
	return m.save()
}

func (m *Manager) SetMemorableName(hash objects.Hash, name string) error {
	m.nameMap[hash] = name
	return m.save()
}

func (m *Manager) GetMemorableName(hash objects.Hash) (string, bool) {
	name, exists := m.nameMap[hash]
	return name, exists
}

func (m *Manager) GetHistory() []Position {
	return m.history
}

func (m *Manager) Load() error {
	configPath := filepath.Join(m.root, ".ivaldi", "position", "config.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}

	type SerializableNameEntry struct {
		Hash string `json:"hash"`
		Name string `json:"name"`
	}

	var config struct {
		Current     Position                `json:"current"`
		History     []Position              `json:"history"`
		Aliases     map[string]objects.Hash `json:"aliases"`
		NameEntries []SerializableNameEntry `json:"nameEntries"`
	}

	if err := json.Unmarshal(data, &config); err != nil {
		return err
	}

	m.current = config.Current
	m.history = config.History
	m.aliases = config.Aliases
	
	m.nameMap = make(map[objects.Hash]string)
	for _, entry := range config.NameEntries {
		var hash objects.Hash
		if err := hash.UnmarshalJSON([]byte(fmt.Sprintf("\"%s\"", entry.Hash))); err == nil {
			m.nameMap[hash] = entry.Name
		}
	}
	
	return nil
}

func (m *Manager) save() error {
	positionDir := filepath.Join(m.root, ".ivaldi", "position")
	if err := os.MkdirAll(positionDir, 0755); err != nil {
		return err
	}

	configPath := filepath.Join(positionDir, "config.json")
	
	type SerializableNameEntry struct {
		Hash string `json:"hash"`
		Name string `json:"name"`
	}
	
	var nameEntries []SerializableNameEntry
	for hash, name := range m.nameMap {
		nameEntries = append(nameEntries, SerializableNameEntry{
			Hash: hash.String(),
			Name: name,
		})
	}
	
	config := struct {
		Current     Position                `json:"current"`
		History     []Position              `json:"history"`
		Aliases     map[string]objects.Hash `json:"aliases"`
		NameEntries []SerializableNameEntry `json:"nameEntries"`
	}{
		Current:     m.current,
		History:     m.history,
		Aliases:     m.aliases,
		NameEntries: nameEntries,
	}

	data, err := json.Marshal(config)
	if err != nil {
		return err
	}

	return os.WriteFile(configPath, data, 0644)
}

// AddMemorableName adds a memorable name mapping for a hash
func (m *Manager) AddMemorableName(hash objects.Hash, name string) {
	m.nameMap[hash] = name
}

// SyncMemorableNamesFromReference syncs memorable names from reference manager
func (m *Manager) SyncMemorableNamesFromReference(getNameFunc func(objects.Hash) (string, bool)) {
	// Sync names for all hashes in our history
	for _, pos := range m.history {
		if name, exists := getNameFunc(pos.Hash); exists {
			m.nameMap[pos.Hash] = name
		}
	}
	
	// Sync current position name
	if name, exists := getNameFunc(m.current.Hash); exists {
		m.nameMap[m.current.Hash] = name
	}
}