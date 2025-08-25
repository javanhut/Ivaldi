package p2p

import (
	"sync"
	"sync/atomic"
	"time"
)

// SubscriptionID is an opaque handle for managing event subscriptions
type SubscriptionID uint64

// subscription tracks a handler with its ID for unsubscribe functionality
type subscription struct {
	id      SubscriptionID
	handler EventHandler
}

// EventBus manages event distribution in the P2P network
type EventBus struct {
	subscribers map[string][]subscription
	mutex       sync.RWMutex
	eventQueue  chan Event
	ctx         chan struct{}
	running     bool
	runMutex    sync.Mutex
	nextSubID   uint64 // atomic counter for generating subscription IDs
}

// EventHandler processes events
type EventHandler func(event Event) error

// Event represents a P2P network event
type Event struct {
	Type      string      `json:"type"`
	Source    string      `json:"source"`
	Target    string      `json:"target"`
	Data      interface{} `json:"data"`
	Timestamp time.Time   `json:"timestamp"`
	ID        string      `json:"id"`
}

// Event types
const (
	EventTypePeerConnected    = "peer_connected"
	EventTypePeerDisconnected = "peer_disconnected"
	EventTypeTimelineUpdated  = "timeline_updated"
	EventTypeSyncStarted      = "sync_started"
	EventTypeSyncCompleted    = "sync_completed"
	EventTypeSyncFailed       = "sync_failed"
	EventTypeConflictDetected = "conflict_detected"
	EventTypeObjectReceived   = "object_received"
)

// NewEventBus creates a new event bus
func NewEventBus() *EventBus {
	return &EventBus{
		subscribers: make(map[string][]subscription),
		eventQueue:  make(chan Event, 1000),
		ctx:         make(chan struct{}),
	}
}

// Start begins processing events
func (eb *EventBus) Start() {
	eb.runMutex.Lock()
	defer eb.runMutex.Unlock()

	if !eb.running {
		eb.running = true
		go eb.processEvents()
	}
}

// Stop shuts down the event bus
func (eb *EventBus) Stop() {
	eb.runMutex.Lock()
	defer eb.runMutex.Unlock()

	if eb.running {
		eb.running = false
		close(eb.ctx)
	}
}

// Subscribe adds an event handler for a specific event type and returns a subscription handle
func (eb *EventBus) Subscribe(eventType string, handler EventHandler) SubscriptionID {
	eb.mutex.Lock()
	defer eb.mutex.Unlock()

	// Generate unique subscription ID using atomic counter
	subID := SubscriptionID(atomic.AddUint64(&eb.nextSubID, 1))

	// Create subscription and add to subscribers list
	sub := subscription{
		id:      subID,
		handler: handler,
	}
	eb.subscribers[eventType] = append(eb.subscribers[eventType], sub)

	return subID
}

// Unsubscribe removes an event handler using its subscription handle
func (eb *EventBus) Unsubscribe(eventType string, subID SubscriptionID) {
	eb.mutex.Lock()
	defer eb.mutex.Unlock()

	// Find and remove the subscription with matching ID
	subs := eb.subscribers[eventType]
	for i, sub := range subs {
		if sub.id == subID {
			// Remove this subscription by swapping with last element and truncating
			subs[i] = subs[len(subs)-1]
			eb.subscribers[eventType] = subs[:len(subs)-1]
			break
		}
	}
}

// Publish sends an event to all subscribers
func (eb *EventBus) Publish(event Event) {
	select {
	case eb.eventQueue <- event:
	default:
		// Queue is full, drop event
	}
}

// processEvents handles events from the queue
func (eb *EventBus) processEvents() {
	for {
		select {
		case <-eb.ctx:
			return
		case event := <-eb.eventQueue:
			eb.dispatchEvent(event)
		}
	}
}

// dispatchEvent sends an event to all relevant subscribers
func (eb *EventBus) dispatchEvent(event Event) {
	eb.mutex.RLock()
	subs := eb.subscribers[event.Type]
	eb.mutex.RUnlock()

	for _, sub := range subs {
		go func(handler EventHandler) {
			if err := handler(event); err != nil {
				// Log error but continue processing
			}
		}(sub.handler)
	}
}

// PublishPeerConnected publishes a peer connected event
func (eb *EventBus) PublishPeerConnected(peerID string, peer *Peer) {
	eb.Publish(Event{
		Type:      EventTypePeerConnected,
		Source:    peerID,
		Data:      peer,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// PublishPeerDisconnected publishes a peer disconnected event
func (eb *EventBus) PublishPeerDisconnected(peerID string, peer *Peer) {
	eb.Publish(Event{
		Type:      EventTypePeerDisconnected,
		Source:    peerID,
		Data:      peer,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// PublishTimelineUpdated publishes a timeline updated event
func (eb *EventBus) PublishTimelineUpdated(timeline string, update *TimelineUpdate) {
	eb.Publish(Event{
		Type:      EventTypeTimelineUpdated,
		Source:    timeline,
		Data:      update,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// PublishSyncStarted publishes a sync started event
func (eb *EventBus) PublishSyncStarted(peerID string, timelines []string) {
	eb.Publish(Event{
		Type:      EventTypeSyncStarted,
		Source:    peerID,
		Data:      timelines,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// PublishSyncCompleted publishes a sync completed event
func (eb *EventBus) PublishSyncCompleted(peerID string, stats SyncStats) {
	eb.Publish(Event{
		Type:      EventTypeSyncCompleted,
		Source:    peerID,
		Data:      stats,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// PublishConflictDetected publishes a conflict detected event
func (eb *EventBus) PublishConflictDetected(peerID string, conflict ConflictInfo) {
	eb.Publish(Event{
		Type:      EventTypeConflictDetected,
		Source:    peerID,
		Data:      conflict,
		Timestamp: time.Now(),
		ID:        generateMessageID(),
	})
}

// SyncStats contains synchronization statistics
type SyncStats struct {
	PeerID           string        `json:"peer_id"`
	Duration         time.Duration `json:"duration"`
	SealsTransferred int           `json:"seals_transferred"`
	BytesTransferred int64         `json:"bytes_transferred"`
	ConflictsFound   int           `json:"conflicts_found"`
	TimelinesSync    []string      `json:"timelines_sync"`
}
