package p2p

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// P2PConfig contains configuration for P2P networking
type P2PConfig struct {
	// Network settings
	Port              int      `json:"port"`
	DiscoveryPort     int      `json:"discovery_port"`
	MaxPeers          int      `json:"max_peers"`
	EnableAutoConnect bool     `json:"enable_auto_connect"`
	KnownPeers        []string `json:"known_peers"`

	// Sync settings
	AutoSyncEnabled  bool          `json:"auto_sync_enabled"`
	SyncInterval     time.Duration `json:"sync_interval"`
	SyncTimeout      time.Duration `json:"sync_timeout"`
	ConflictStrategy string        `json:"conflict_strategy"` // "manual", "auto_merge", "prefer_remote", "prefer_local"

	// Performance settings
	MaxConcurrentSync int           `json:"max_concurrent_sync"`
	MaxMessageSize    int64         `json:"max_message_size"`
	HeartbeatInterval time.Duration `json:"heartbeat_interval"`

	// Security settings
	EnableEncryption bool     `json:"enable_encryption"`
	TrustedPeers     []string `json:"trusted_peers"`
	AllowedNetworks  []string `json:"allowed_networks"`

	// Storage settings
	DataDir       string `json:"data_dir"`
	EnableMetrics bool   `json:"enable_metrics"`
}

// DefaultP2PConfig returns default P2P configuration
func DefaultP2PConfig() *P2PConfig {
	return &P2PConfig{
		Port:              9090,
		DiscoveryPort:     9091,
		MaxPeers:          50,
		EnableAutoConnect: true,
		KnownPeers:        []string{},

		AutoSyncEnabled:  true,
		SyncInterval:     30 * time.Second,
		SyncTimeout:      60 * time.Second,
		ConflictStrategy: "manual",

		MaxConcurrentSync: 5,
		MaxMessageSize:    10 * 1024 * 1024, // 10MB
		HeartbeatInterval: 30 * time.Second,

		EnableEncryption: false,
		TrustedPeers:     []string{},
		AllowedNetworks:  []string{"192.168.0.0/16", "10.0.0.0/8", "172.16.0.0/12"},

		DataDir:       ".ivaldi/p2p",
		EnableMetrics: true,
	}
}

// P2PConfigManager manages P2P configuration
type P2PConfigManager struct {
	configPath string
	config     *P2PConfig
}

// NewP2PConfigManager creates a new config manager
func NewP2PConfigManager(rootDir string) *P2PConfigManager {
	configPath := filepath.Join(rootDir, ".ivaldi", "p2p_config.json")
	return &P2PConfigManager{
		configPath: configPath,
	}
}

// Load loads P2P configuration from file
func (cm *P2PConfigManager) Load() (*P2PConfig, error) {
	if _, err := os.Stat(cm.configPath); os.IsNotExist(err) {
		// Config doesn't exist, create default
		config := DefaultP2PConfig()
		if err := cm.Save(config); err != nil {
			return nil, fmt.Errorf("failed to save default config: %v", err)
		}
		cm.config = config
		return config, nil
	}

	data, err := os.ReadFile(cm.configPath)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file: %v", err)
	}

	var config P2PConfig
	if err := json.Unmarshal(data, &config); err != nil {
		return nil, fmt.Errorf("failed to parse config: %v", err)
	}

	cm.config = &config
	return &config, nil
}

// Save saves P2P configuration to file
func (cm *P2PConfigManager) Save(config *P2PConfig) error {
	// Validate configuration before persisting
	if err := cm.ValidateConfig(config); err != nil {
		return err
	}

	// Ensure directory exists
	dir := filepath.Dir(cm.configPath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("failed to create config directory: %v", err)
	}

	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal config: %v", err)
	}

	if err := os.WriteFile(cm.configPath, data, 0644); err != nil {
		return fmt.Errorf("failed to write config file: %v", err)
	}

	cm.config = config
	return nil
}

// Get returns the current configuration
func (cm *P2PConfigManager) Get() *P2PConfig {
	if cm.config == nil {
		config, _ := cm.Load()
		return config
	}
	return cm.config
}

// UpdatePort updates the P2P port
func (cm *P2PConfigManager) UpdatePort(port int) error {
	config := cm.Get()
	config.Port = port
	return cm.Save(config)
}

// UpdateSyncInterval updates the sync interval
func (cm *P2PConfigManager) UpdateSyncInterval(interval time.Duration) error {
	config := cm.Get()
	config.SyncInterval = interval
	return cm.Save(config)
}

// AddKnownPeer adds a peer to the known peers list
func (cm *P2PConfigManager) AddKnownPeer(peerAddr string) error {
	config := cm.Get()

	// Check if peer already exists
	for _, peer := range config.KnownPeers {
		if peer == peerAddr {
			return nil // Already exists
		}
	}

	config.KnownPeers = append(config.KnownPeers, peerAddr)
	return cm.Save(config)
}

// RemoveKnownPeer removes a peer from the known peers list
func (cm *P2PConfigManager) RemoveKnownPeer(peerAddr string) error {
	config := cm.Get()

	var newPeers []string
	for _, peer := range config.KnownPeers {
		if peer != peerAddr {
			newPeers = append(newPeers, peer)
		}
	}

	config.KnownPeers = newPeers
	return cm.Save(config)
}

// SetAutoSync enables or disables automatic synchronization
func (cm *P2PConfigManager) SetAutoSync(enabled bool) error {
	config := cm.Get()
	config.AutoSyncEnabled = enabled
	return cm.Save(config)
}

// SetConflictStrategy sets the conflict resolution strategy
func (cm *P2PConfigManager) SetConflictStrategy(strategy string) error {
	validStrategies := map[string]bool{
		"manual":        true,
		"auto_merge":    true,
		"prefer_remote": true,
		"prefer_local":  true,
	}

	if !validStrategies[strategy] {
		return fmt.Errorf("invalid conflict strategy: %s", strategy)
	}

	config := cm.Get()
	config.ConflictStrategy = strategy
	return cm.Save(config)
}

// AddTrustedPeer adds a peer to the trusted peers list
func (cm *P2PConfigManager) AddTrustedPeer(peerID string) error {
	config := cm.Get()

	// Check if peer already exists
	for _, peer := range config.TrustedPeers {
		if peer == peerID {
			return nil // Already exists
		}
	}

	config.TrustedPeers = append(config.TrustedPeers, peerID)
	return cm.Save(config)
}

// RemoveTrustedPeer removes a peer from the trusted peers list
func (cm *P2PConfigManager) RemoveTrustedPeer(peerID string) error {
	config := cm.Get()

	var newPeers []string
	for _, peer := range config.TrustedPeers {
		if peer != peerID {
			newPeers = append(newPeers, peer)
		}
	}

	config.TrustedPeers = newPeers
	return cm.Save(config)
}

// ValidateConfig validates the P2P configuration
func (cm *P2PConfigManager) ValidateConfig(config *P2PConfig) error {
	if config.Port <= 0 || config.Port > 65535 {
		return fmt.Errorf("invalid port: %d", config.Port)
	}

	if config.DiscoveryPort <= 0 || config.DiscoveryPort > 65535 {
		return fmt.Errorf("invalid discovery port: %d", config.DiscoveryPort)
	}

	if config.MaxPeers <= 0 {
		return fmt.Errorf("max peers must be positive: %d", config.MaxPeers)
	}

	if config.SyncInterval <= 0 {
		return fmt.Errorf("sync interval must be positive: %v", config.SyncInterval)
	}

	if config.SyncTimeout <= 0 {
		return fmt.Errorf("sync timeout must be positive: %v", config.SyncTimeout)
	}

	validStrategies := map[string]bool{
		"manual":        true,
		"auto_merge":    true,
		"prefer_remote": true,
		"prefer_local":  true,
	}

	if !validStrategies[config.ConflictStrategy] {
		return fmt.Errorf("invalid conflict strategy: %s", config.ConflictStrategy)
	}

	return nil
}

// GetConfigSummary returns a human-readable summary of the configuration
func (cm *P2PConfigManager) GetConfigSummary() string {
	config := cm.Get()

	status := "disabled"
	if config.AutoSyncEnabled {
		status = "enabled"
	}

	autoConnect := "disabled"
	if config.EnableAutoConnect {
		autoConnect = "enabled"
	}

	return fmt.Sprintf(`P2P Configuration:
  Port: %d
  Discovery Port: %d
  Max Peers: %d
  Auto-connect: %s
  Auto-sync: %s (every %v)
  Conflict Strategy: %s
  Known Peers: %d
  Trusted Peers: %d
  Encryption: %t
  Metrics: %t`,
		config.Port,
		config.DiscoveryPort,
		config.MaxPeers,
		autoConnect,
		status,
		config.SyncInterval,
		config.ConflictStrategy,
		len(config.KnownPeers),
		len(config.TrustedPeers),
		config.EnableEncryption,
		config.EnableMetrics,
	)
}

// ResetToDefaults resets configuration to default values
func (cm *P2PConfigManager) ResetToDefaults() error {
	config := DefaultP2PConfig()
	return cm.Save(config)
}
