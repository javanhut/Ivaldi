package metrics

import (
	"fmt"
	"ivaldi/core/logging"
	"sync"
	"time"
)

// MetricType represents the type of metric
type MetricType int

const (
	Counter MetricType = iota
	Gauge
	Histogram
)

// Metric represents a single metric
type Metric struct {
	Name      string
	Type      MetricType
	Value     float64
	Labels    map[string]string
	Timestamp time.Time
}

// CounterMetric represents a counter metric
type CounterMetric struct {
	Name   string
	Value  int64
	Labels map[string]string
}

// GaugeMetric represents a gauge metric
type GaugeMetric struct {
	Name   string
	Value  float64
	Labels map[string]string
}

// HistogramMetric represents a histogram metric
type HistogramMetric struct {
	Name    string
	Buckets map[string]int64
	Sum     float64
	Count   int64
	Labels  map[string]string
}

// MetricsCollector provides metrics collection functionality
type MetricsCollector struct {
	counters   map[string]*CounterMetric
	gauges     map[string]*GaugeMetric
	histograms map[string]*HistogramMetric
	mu         sync.RWMutex
	logger     *logging.Logger
}

// NewMetricsCollector creates a new metrics collector
func NewMetricsCollector() *MetricsCollector {
	return &MetricsCollector{
		counters:   make(map[string]*CounterMetric),
		gauges:     make(map[string]*GaugeMetric),
		histograms: make(map[string]*HistogramMetric),
		logger:     logging.WithPrefix("metrics"),
	}
}

// IncrementCounter increments a counter metric
func (mc *MetricsCollector) IncrementCounter(name string, labels map[string]string) {
	mc.mu.Lock()
	defer mc.mu.Unlock()

	key := mc.makeKey(name, labels)
	if counter, exists := mc.counters[key]; exists {
		counter.Value++
	} else {
		mc.counters[key] = &CounterMetric{
			Name:   name,
			Value:  1,
			Labels: labels,
		}
	}
}

// SetGauge sets a gauge metric value
func (mc *MetricsCollector) SetGauge(name string, value float64, labels map[string]string) {
	mc.mu.Lock()
	defer mc.mu.Unlock()

	key := mc.makeKey(name, labels)
	mc.gauges[key] = &GaugeMetric{
		Name:   name,
		Value:  value,
		Labels: labels,
	}
}

// ObserveHistogram records a value in a histogram metric
func (mc *MetricsCollector) ObserveHistogram(name string, value float64, labels map[string]string) {
	mc.mu.Lock()
	defer mc.mu.Unlock()

	key := mc.makeKey(name, labels)
	histogram, exists := mc.histograms[key]
	if !exists {
		histogram = &HistogramMetric{
			Name:    name,
			Buckets: make(map[string]int64),
			Sum:     0,
			Count:   0,
			Labels:  labels,
		}
		mc.histograms[key] = histogram
	}

	histogram.Sum += value
	histogram.Count++

	// Simple bucket logic - could be made configurable
	bucket := mc.getBucket(value)
	histogram.Buckets[bucket]++
}

// GetCounter returns the current value of a counter
func (mc *MetricsCollector) GetCounter(name string, labels map[string]string) (int64, bool) {
	mc.mu.RLock()
	defer mc.mu.RUnlock()

	key := mc.makeKey(name, labels)
	if counter, exists := mc.counters[key]; exists {
		return counter.Value, true
	}
	return 0, false
}

// GetGauge returns the current value of a gauge
func (mc *MetricsCollector) GetGauge(name string, labels map[string]string) (float64, bool) {
	mc.mu.RLock()
	defer mc.mu.RUnlock()

	key := mc.makeKey(name, labels)
	if gauge, exists := mc.gauges[key]; exists {
		return gauge.Value, true
	}
	return 0, false
}

// GetHistogram returns the current state of a histogram
func (mc *MetricsCollector) GetHistogram(name string, labels map[string]string) (*HistogramMetric, bool) {
	mc.mu.RLock()
	defer mc.mu.RUnlock()

	key := mc.makeKey(name, labels)
	if histogram, exists := mc.histograms[key]; exists {
		return histogram, true
	}
	return nil, false
}

// GetAllMetrics returns all collected metrics
func (mc *MetricsCollector) GetAllMetrics() map[string]interface{} {
	mc.mu.RLock()
	defer mc.mu.RUnlock()

	result := make(map[string]interface{})

	// Add counters
	for key, counter := range mc.counters {
		result["counter_"+key] = counter
	}

	// Add gauges
	for key, gauge := range mc.gauges {
		result["gauge_"+key] = gauge
	}

	// Add histograms
	for key, histogram := range mc.histograms {
		result["histogram_"+key] = histogram
	}

	return result
}

// ResetMetrics resets all metrics to zero
func (mc *MetricsCollector) ResetMetrics() {
	mc.mu.Lock()
	defer mc.mu.Unlock()

	mc.counters = make(map[string]*CounterMetric)
	mc.gauges = make(map[string]*GaugeMetric)
	mc.histograms = make(map[string]*HistogramMetric)

	mc.logger.Info("All metrics reset")
}

// makeKey creates a unique key for a metric with labels
func (mc *MetricsCollector) makeKey(name string, labels map[string]string) string {
	if len(labels) == 0 {
		return name
	}

	key := name
	for k, v := range labels {
		key += fmt.Sprintf("_%s_%s", k, v)
	}
	return key
}

// getBucket determines which bucket a value falls into
func (mc *MetricsCollector) getBucket(value float64) string {
	switch {
	case value < 0.001:
		return "0.001"
	case value < 0.01:
		return "0.01"
	case value < 0.1:
		return "0.1"
	case value < 1:
		return "1"
	case value < 10:
		return "10"
	case value < 100:
		return "100"
	case value < 1000:
		return "1000"
	default:
		return "inf"
	}
}

// MetricsReport provides a comprehensive metrics report
type MetricsReport struct {
	Timestamp  time.Time
	Counters   map[string]*CounterMetric
	Gauges     map[string]*GaugeMetric
	Histograms map[string]*HistogramMetric
	Summary    string
}

// GenerateReport generates a comprehensive metrics report
func (mc *MetricsCollector) GenerateReport() *MetricsReport {
	mc.mu.RLock()
	defer mc.mu.RUnlock()

	// Count metrics by type
	counterCount := len(mc.counters)
	gaugeCount := len(mc.gauges)
	histogramCount := len(mc.histograms)

	summary := fmt.Sprintf("Metrics summary: %d counters, %d gauges, %d histograms",
		counterCount, gaugeCount, histogramCount)

	return &MetricsReport{
		Timestamp:  time.Now(),
		Counters:   mc.copyCounters(),
		Gauges:     mc.copyGauges(),
		Histograms: mc.copyHistograms(),
		Summary:    summary,
	}
}

// copyCounters creates a copy of all counters
func (mc *MetricsCollector) copyCounters() map[string]*CounterMetric {
	result := make(map[string]*CounterMetric)
	for key, counter := range mc.counters {
		counterCopy := *counter
		labelsCopy := make(map[string]string)
		for k, v := range counter.Labels {
			labelsCopy[k] = v
		}
		counterCopy.Labels = labelsCopy
		result[key] = &counterCopy
	}
	return result
}

// copyGauges creates a copy of all gauges
func (mc *MetricsCollector) copyGauges() map[string]*GaugeMetric {
	result := make(map[string]*GaugeMetric)
	for key, gauge := range mc.gauges {
		gaugeCopy := *gauge
		labelsCopy := make(map[string]string)
		for k, v := range gauge.Labels {
			labelsCopy[k] = v
		}
		gaugeCopy.Labels = labelsCopy
		result[key] = &gaugeCopy
	}
	return result
}

// copyHistograms creates a copy of all histograms
func (mc *MetricsCollector) copyHistograms() map[string]*HistogramMetric {
	result := make(map[string]*HistogramMetric)
	for key, histogram := range mc.histograms {
		histogramCopy := *histogram
		bucketsCopy := make(map[string]int64)
		for k, v := range histogram.Buckets {
			bucketsCopy[k] = v
		}
		histogramCopy.Buckets = bucketsCopy
		labelsCopy := make(map[string]string)
		for k, v := range histogram.Labels {
			labelsCopy[k] = v
		}
		histogramCopy.Labels = labelsCopy
		result[key] = &histogramCopy
	}
	return result
}

// StartPeriodicMetricsCollection starts periodic metrics collection
func (mc *MetricsCollector) StartPeriodicMetricsCollection(interval time.Duration) {
	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()

		for range ticker.C {
			report := mc.GenerateReport()
			mc.logger.Info("Periodic metrics collection",
				"timestamp", report.Timestamp,
				"summary", report.Summary)
		}
	}()

	mc.logger.Info("Started periodic metrics collection", "interval", interval)
}

// StopPeriodicMetricsCollection stops periodic metrics collection
func (mc *MetricsCollector) StopPeriodicMetricsCollection() {
	mc.logger.Info("Stopping periodic metrics collection")
}
