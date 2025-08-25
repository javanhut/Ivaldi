# Ivaldi P2P Features Documentation

## Overview

Ivaldi implements a comprehensive peer-to-peer (P2P) networking system that enables decentralized collaboration and synchronization between multiple Ivaldi repositories. The P2P system is built on top of TCP networking with UDP-based discovery and provides automatic synchronization, conflict detection, and mesh networking capabilities.

## Core Components

### P2P Manager (`core/p2p/manager.go`)

The P2PManager is the central coordinator for all P2P functionality, managing:

- **Network Layer**: TCP-based peer connections
- **Synchronization**: Automatic timeline and seal synchronization
- **Discovery Service**: UDP-based peer discovery
- **Configuration**: Persistent P2P settings
- **Event System**: Event-driven communication between components

Key Features:
- Automatic startup and shutdown of all P2P services
- Connection management to known and discovered peers
- Configurable sync intervals and conflict strategies
- Real-time status monitoring and metrics

### Network Layer (`core/p2p/network.go`)

Handles low-level peer-to-peer communication:

#### Connection Management
- TCP-based persistent connections between peers
- Automatic handshake protocol for peer identification
- Connection pooling and lifecycle management
- Heartbeat system for connection health monitoring

#### Message Types
- `handshake`: Initial peer authentication and capability exchange
- `sync_request`: Timeline synchronization requests
- `sync_response`: Timeline synchronization data responses
- `timeline_update`: Real-time timeline change broadcasts
- `seal_broadcast`: New seal announcements
- `heartbeat`: Connection keepalive messages
- `peer_discovery`: Peer list exchange
- `conflict_resolve`: Conflict resolution coordination

#### Protocol Features
- JSON-based message serialization
- Unique message IDs for tracking and deduplication
- Timestamped messages for ordering
- Broadcast and unicast message delivery

### Peer Discovery (`core/p2p/discovery.go`)

UDP-based peer discovery system for automatic network formation:

#### Discovery Protocol
- Periodic UDP broadcasts announcing node presence
- Repository-based peer filtering and matching
- Multi-subnet broadcast support for larger networks
- Automatic connection establishment to discovered peers

#### Peer Information
- Node ID and network address
- Available repositories and versions
- Last seen timestamps
- Connection capabilities

#### Auto-Connect Features
- Automatic connection to recently discovered peers
- Configurable connection limits and timeouts
- Cleanup of stale peer information

### Synchronization Engine (`core/p2p/sync.go`)

Comprehensive timeline and object synchronization:

#### Sync Operations
- **Incremental Sync**: Transfer only missing seals and objects
- **Full Sync**: Complete timeline reconstruction
- **Real-time Updates**: Live timeline change propagation
- **Conflict Detection**: Automatic divergence identification

#### Sync State Management
- Per-peer synchronization state tracking
- Timeline head comparison and synchronization
- Bytes transferred and sync frequency metrics
- Automatic and manual sync triggering

#### Conflict Resolution
- Conflict detection for diverged timelines
- Multiple resolution strategies:
  - `manual`: User intervention required
  - `auto_merge`: Automatic merge attempts
  - `prefer_remote`: Always accept remote changes
  - `prefer_local`: Always keep local changes

#### Object Transfer
- Seal-based timeline synchronization
- Tree and blob dependency resolution
- Efficient object packing and transfer
- Integrity verification and error handling

### Configuration System (`core/p2p/config.go`)

Persistent configuration management for P2P settings:

#### Network Configuration
- TCP port for peer connections (default: 9090)
- UDP discovery port (default: 9091)
- Maximum peer connections (default: 50)
- Known peer addresses for bootstrapping

#### Synchronization Settings
- Auto-sync enable/disable (default: enabled)
- Sync interval (default: 30 seconds)
- Sync timeout (default: 60 seconds)
- Conflict resolution strategy (default: manual)

#### Performance Tuning
- Maximum concurrent syncs (default: 5)
- Maximum message size (default: 10MB)
- Heartbeat interval (default: 30 seconds)

#### Security Options
- Encryption enable/disable (default: disabled)
- Trusted peer whitelist
- Allowed network ranges
- Data directory configuration

### Event System (`core/p2p/events.go`)

Event-driven architecture for P2P operations:

#### Event Types
- `peer_connected`: New peer joins the network
- `peer_disconnected`: Peer leaves the network
- `timeline_updated`: Timeline changes detected
- `sync_started`: Synchronization begins
- `sync_completed`: Synchronization finishes
- `sync_failed`: Synchronization errors
- `conflict_detected`: Timeline conflicts found
- `object_received`: New objects synchronized

#### Event Bus Features
- Asynchronous event processing
- Multiple subscriber support per event type
- Event queuing with overflow protection
- Automatic event statistics and metrics

### Mesh Networking (`core/mesh/mesh.go`)

Advanced mesh networking layer built on P2P foundation:

#### Mesh Topology
- Multi-hop routing through peer networks
- Automatic path discovery and optimization
- Dynamic topology updates and healing
- Peer capability and version management

#### Routing Features
- Shortest path calculation between nodes
- Load balancing across multiple routes
- Route caching and TTL management
- Automatic failover and redundancy

## Command Line Interface

### P2P Commands (`ui/enhanced_cli/cli.go`)

Comprehensive CLI interface for P2P operations:

#### Basic Commands
```bash
# Start P2P networking
ivaldi p2p start

# Stop P2P networking
ivaldi p2p stop

# Check P2P status
ivaldi p2p status

# List connected peers
ivaldi p2p peers

# List discovered peers
ivaldi p2p discovered
```

#### Connection Management
```bash
# Connect to a specific peer
ivaldi p2p connect <address> <port>

# Disconnect from a peer
ivaldi p2p disconnect <peer-id>

# Add known peer for auto-connect
ivaldi p2p add-peer <address:port>

# Remove known peer
ivaldi p2p remove-peer <address:port>
```

#### Synchronization Controls
```bash
# Sync with all peers
ivaldi p2p sync

# Sync with specific peer
ivaldi p2p sync --peer <peer-id>

# Enable/disable auto-sync
ivaldi p2p auto-sync on|off

# Set sync interval
ivaldi p2p interval <duration>

# Check sync status
ivaldi p2p sync-status
```

#### Configuration Management
```bash
# Show current configuration
ivaldi p2p config

# Set P2P port
ivaldi p2p config --port <port>

# Set discovery port
ivaldi p2p config --discovery-port <port>

# Set conflict strategy
ivaldi p2p config --conflict-strategy <strategy>

# Reset to default configuration
ivaldi p2p config --reset
```

## Storage Integration

### Storage Adapter (`core/p2p/adapters.go`)

Seamless integration with Ivaldi's storage system:

#### Storage Interface
- Object existence checking (`HasObject`)
- Seal, tree, and blob storage operations
- Object listing and enumeration
- Storage adapter pattern for different backends

#### Timeline Integration
- Timeline listing and metadata access
- Head tracking and updates
- Timeline creation and management
- Metadata synchronization between peers

## Testing Framework

Comprehensive test suite covering all P2P functionality:

### Test Categories
- **Basic P2P Tests** (`tests/p2p_test.go`): Core networking functionality
- **Manager Tests** (`tests/p2p_manager_test.go`): P2P manager operations
- **Conflict Tests** (`tests/p2p_conflict_test.go`): Conflict detection and resolution
- **Stress Tests** (`tests/p2p_stress_test.go`): Performance and reliability
- **Robust Tests** (`tests/robust_p2p_test.go`): Error handling and recovery

### Test Features
- Multi-repository test scenarios
- Network simulation and partitioning
- Conflict generation and resolution testing
- Performance benchmarking and metrics
- Automatic cleanup and teardown

## Security Features

### Network Security
- TCP connection encryption (configurable)
- Peer authentication via handshake protocol
- Trusted peer whitelisting
- Network range restrictions for discovery

### Data Integrity
- Object hash verification during transfer
- Message integrity checking
- Conflict detection for data corruption
- Automatic retry and recovery mechanisms

### Access Control
- Repository-based access control
- Peer capability negotiation
- Timeline access restrictions
- Configurable security policies

## Performance Optimizations

### Network Efficiency
- Connection pooling and reuse
- Message batching and compression
- Efficient object transfer protocols
- Bandwidth throttling and QoS

### Synchronization Performance
- Incremental sync with merkle-tree like diffing
- Parallel sync operations
- Smart conflict detection algorithms
- Optimistic sync with rollback capability

### Memory Management
- Streaming object transfers for large files
- Connection state cleanup
- Event queue overflow protection
- Garbage collection optimization

## Monitoring and Diagnostics

### Status Information
- Real-time peer connection status
- Synchronization metrics and statistics
- Network topology visualization
- Error rate and performance tracking

### Debugging Features
- Detailed logging for all P2P operations
- Message tracing and inspection
- Connection state debugging
- Performance profiling tools

## Future Enhancements

### Planned Features
- Enhanced encryption and key management
- Distributed consensus algorithms
- Cross-platform mobile support
- WebRTC-based browser connectivity
- Advanced mesh routing algorithms
- Blockchain-based trust networks

### Scalability Improvements
- Hierarchical peer organization
- Content-addressable networking
- Distributed hash tables (DHT)
- Load balancing and sharding
- Geographic peer optimization

This documentation provides a comprehensive overview of all P2P features in the Ivaldi system. Each component is designed to work together seamlessly to provide a robust, scalable, and user-friendly peer-to-peer collaboration experience.