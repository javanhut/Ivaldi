package health

import (
	"context"
	"fmt"
	"ivaldi/core/logging"
	"sync"
	"time"
)

// HealthStatus represents the overall health of a component
type HealthStatus int

const (
	Healthy HealthStatus = iota
	Degraded
	Unhealthy
)

func (h HealthStatus) String() string {
	switch h {
	case Healthy:
		return "healthy"
	case Degraded:
		return "degraded"
	case Unhealthy:
		return "unhealthy"
	default:
		return "unknown"
	}
}

// ComponentHealth represents the health of a specific component
type ComponentHealth struct {
	Name      string
	Status    HealthStatus
	Message   string
	LastCheck time.Time
	Details   map[string]interface{}
}

// HealthChecker provides health checking functionality
type HealthChecker struct {
	components map[string]*ComponentHealth
	mu         sync.RWMutex
	logger     *logging.Logger
	// Periodic health check control
	cancel context.CancelFunc
	wg     sync.WaitGroup
	mutex  sync.Mutex // protects cancel and wg operations
}

// NewHealthChecker creates a new health checker
func NewHealthChecker() *HealthChecker {
	return &HealthChecker{
		components: make(map[string]*ComponentHealth),
		logger:     logging.WithPrefix("health"),
	}
}

// RegisterComponent registers a component for health checking
func (hc *HealthChecker) RegisterComponent(name string) {
	hc.mu.Lock()
	defer hc.mu.Unlock()

	hc.components[name] = &ComponentHealth{
		Name:      name,
		Status:    Healthy,
		Message:   "Component registered",
		LastCheck: time.Now(),
		Details:   make(map[string]interface{}),
	}

	hc.logger.Info("Component registered for health checking", "component", name)
}

// UpdateComponentHealth updates the health status of a component
func (hc *HealthChecker) UpdateComponentHealth(name string, status HealthStatus, message string, details map[string]interface{}) {
	hc.mu.Lock()
	defer hc.mu.Unlock()

	if component, exists := hc.components[name]; exists {
		component.Status = status
		component.Message = message
		component.LastCheck = time.Now()
		if details != nil {
			// Create defensive copy of details map to prevent external mutations
			detailsCopy := make(map[string]interface{}, len(details))
			for k, v := range details {
				detailsCopy[k] = v
			}
			component.Details = detailsCopy
		} else {
			component.Details = nil
		}

		hc.logger.Info("Component health updated",
			"component", name,
			"status", status.String(),
			"message", message)
	} else {
		hc.logger.Warn("Attempted to update health for unregistered component", "component", name)
	}
}

// GetComponentHealth returns the health status of a specific component
func (hc *HealthChecker) GetComponentHealth(name string) (*ComponentHealth, bool) {
	hc.mu.RLock()
	defer hc.mu.RUnlock()

	component, exists := hc.components[name]
	if !exists {
		return nil, false
	}

	// Create a deep copy of the component
	copy := &ComponentHealth{
		Name:      component.Name,
		Status:    component.Status,
		Message:   component.Message,
		LastCheck: component.LastCheck,
		Details:   nil,
	}

	// Deep copy the Details map if it's not nil
	if component.Details != nil {
		copy.Details = make(map[string]interface{}, len(component.Details))
		for k, v := range component.Details {
			copy.Details[k] = v
		}
	}

	return copy, true
}

// GetOverallHealth returns the overall health status
func (hc *HealthChecker) GetOverallHealth() HealthStatus {
	hc.mu.RLock()
	defer hc.mu.RUnlock()

	if len(hc.components) == 0 {
		return Healthy
	}

	overallStatus := Healthy
	unhealthyCount := 0
	degradedCount := 0

	for _, component := range hc.components {
		switch component.Status {
		case Unhealthy:
			unhealthyCount++
		case Degraded:
			degradedCount++
		}
	}

	// If any component is unhealthy, overall status is unhealthy
	if unhealthyCount > 0 {
		overallStatus = Unhealthy
	} else if degradedCount > 0 {
		overallStatus = Degraded
	}

	return overallStatus
}

// GetAllComponents returns all registered components and their health status
func (hc *HealthChecker) GetAllComponents() map[string]*ComponentHealth {
	hc.mu.RLock()
	defer hc.mu.RUnlock()

	result := make(map[string]*ComponentHealth)
	for name, component := range hc.components {
		// Create a deep copy of each component
		copy := &ComponentHealth{
			Name:      component.Name,
			Status:    component.Status,
			Message:   component.Message,
			LastCheck: component.LastCheck,
			Details:   make(map[string]interface{}),
		}

		// Deep copy the Details map
		for k, v := range component.Details {
			copy.Details[k] = v
		}

		result[name] = copy
	}

	return result
}

// CheckHealth performs a health check on all components
func (hc *HealthChecker) CheckHealth() map[string]*ComponentHealth {
	hc.mu.RLock()
	components := make(map[string]*ComponentHealth)
	for name, component := range hc.components {
		// Create a deep copy of each component
		copy := &ComponentHealth{
			Name:      component.Name,
			Status:    component.Status,
			Message:   component.Message,
			LastCheck: component.LastCheck,
			Details:   make(map[string]interface{}),
		}

		// Deep copy the Details map
		for k, v := range component.Details {
			copy.Details[k] = v
		}

		components[name] = copy
	}
	hc.mu.RUnlock()

	// Perform health checks (this would be implemented by specific components)
	hc.logger.Info("Performing health check", "component_count", len(components))

	return components
}

// HealthReport provides a comprehensive health report
type HealthReport struct {
	OverallStatus HealthStatus
	Timestamp     time.Time
	Components    map[string]*ComponentHealth
	Summary       string
}

// GenerateReport generates a comprehensive health report
func (hc *HealthChecker) GenerateReport() *HealthReport {
	hc.mu.RLock()
	defer hc.mu.RUnlock()

	overallStatus := hc.GetOverallHealth()

	// Count statuses
	healthyCount := 0
	degradedCount := 0
	unhealthyCount := 0

	for _, component := range hc.components {
		switch component.Status {
		case Healthy:
			healthyCount++
		case Degraded:
			degradedCount++
		case Unhealthy:
			unhealthyCount++
		}
	}

	// Generate summary
	var summary string
	if unhealthyCount > 0 {
		summary = fmt.Sprintf("System unhealthy: %d unhealthy, %d degraded, %d healthy",
			unhealthyCount, degradedCount, healthyCount)
	} else if degradedCount > 0 {
		summary = fmt.Sprintf("System degraded: %d degraded, %d healthy",
			degradedCount, healthyCount)
	} else {
		summary = fmt.Sprintf("System healthy: %d components healthy", healthyCount)
	}

	return &HealthReport{
		OverallStatus: overallStatus,
		Timestamp:     time.Now(),
		Components:    hc.getAllComponentsCopy(),
		Summary:       summary,
	}
}

// getAllComponentsCopy creates a copy of all components
func (hc *HealthChecker) getAllComponentsCopy() map[string]*ComponentHealth {
	result := make(map[string]*ComponentHealth)
	for name, component := range hc.components {
		componentCopy := *component
		detailsCopy := make(map[string]interface{})
		for k, v := range component.Details {
			detailsCopy[k] = v
		}
		componentCopy.Details = detailsCopy
		result[name] = &componentCopy
	}
	return result
}

// StartPeriodicHealthCheck starts periodic health checking
func (hc *HealthChecker) StartPeriodicHealthCheck(interval time.Duration) {
	hc.mutex.Lock()
	defer hc.mutex.Unlock()

	// If already running, stop the existing one first
	if hc.cancel != nil {
		hc.cancel()
		hc.wg.Wait()
	}

	// Create new context for this health check session
	ctx, cancel := context.WithCancel(context.Background())
	hc.cancel = cancel

	hc.wg.Add(1)
	go func() {
		defer hc.wg.Done()
		ticker := time.NewTicker(interval)
		defer ticker.Stop()

		for {
			select {
			case <-ticker.C:
				hc.CheckHealth()
			case <-ctx.Done():
				hc.logger.Info("Periodic health checking stopped")
				return
			}
		}
	}()

	hc.logger.Info("Started periodic health checking", "interval", interval)
}

// StopPeriodicHealthCheck stops periodic health checking
func (hc *HealthChecker) StopPeriodicHealthCheck() {
	hc.mutex.Lock()
	defer hc.mutex.Unlock()

	if hc.cancel != nil {
		hc.logger.Info("Stopping periodic health checking")
		hc.cancel()
		hc.wg.Wait() // Wait for the goroutine to finish
		hc.cancel = nil
	}
}
