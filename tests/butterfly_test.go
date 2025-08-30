package tests

import (
	"os"
	"testing"

	"ivaldi/core/timeline"
	"ivaldi/forge"
)

func TestButterflyTimelineNaming(t *testing.T) {
	tests := []struct {
		name         string
		timelineName string
		expected     bool
	}{
		{"Regular timeline", "main", false},
		{"Butterfly timeline", "main:diverged:1", true},
		{"Named butterfly", "feature:diverged:experiment", true},
		{"Empty string", "", false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := timeline.IsButterflyTimeline(tt.timelineName)
			if result != tt.expected {
				t.Errorf("IsButterflyTimeline(%q) = %v, expected %v", tt.timelineName, result, tt.expected)
			}
		})
	}
}

func TestGetBaseTimeline(t *testing.T) {
	tests := []struct {
		name         string
		timelineName string
		expected     string
	}{
		{"Regular timeline", "main", "main"},
		{"Butterfly timeline", "main:diverged:1", "main"},
		{"Named butterfly", "feature:diverged:experiment", "feature"},
		{"Complex name", "auth-system:diverged:jwt_approach", "auth-system"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := timeline.GetBaseTimeline(tt.timelineName)
			if result != tt.expected {
				t.Errorf("GetBaseTimeline(%q) = %q, expected %q", tt.timelineName, result, tt.expected)
			}
		})
	}
}

func TestGetButterflyIdentifier(t *testing.T) {
	tests := []struct {
		name         string
		timelineName string
		expected     string
	}{
		{"Regular timeline", "main", ""},
		{"Butterfly timeline", "main:diverged:1", "1"},
		{"Named butterfly", "feature:diverged:experiment", "experiment"},
		{"Complex identifier", "auth-system:diverged:jwt_approach", "jwt_approach"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := timeline.GetButterflyIdentifier(tt.timelineName)
			if result != tt.expected {
				t.Errorf("GetButterflyIdentifier(%q) = %q, expected %q", tt.timelineName, result, tt.expected)
			}
		})
	}
}

func TestBuildButterflyTimelineName(t *testing.T) {
	tests := []struct {
		name         string
		baseTimeline string
		identifier   string
		expected     string
	}{
		{"Simple", "main", "1", "main:diverged:1"},
		{"Named variant", "feature", "experiment", "feature:diverged:experiment"},
		{"Complex base", "auth-system", "jwt_approach", "auth-system:diverged:jwt_approach"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := timeline.BuildButterflyTimelineName(tt.baseTimeline, tt.identifier)
			if result != tt.expected {
				t.Errorf("BuildButterflyTimelineName(%q, %q) = %q, expected %q",
					tt.baseTimeline, tt.identifier, result, tt.expected)
			}
		})
	}
}

func TestButterflyTimelineCreation(t *testing.T) {
	// Create temporary directory for test
	tempDir, err := os.MkdirTemp("", "ivaldi_test")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}

	// Test creating a butterfly timeline
	err = repo.CreateButterflyTimeline("experiment")
	if err != nil {
		t.Fatalf("Failed to create butterfly timeline: %v", err)
	}

	// Verify the butterfly timeline exists
	timelineManager := repo.Timeline()
	if !timelineManager.Exists("main:diverged:experiment") {
		t.Error("Butterfly timeline was not created")
	}

	// Test that we can list butterfly variants
	variants := repo.ListButterflyTimelines()
	if len(variants) != 2 { // base + 1 variant
		t.Errorf("Expected 2 variants (base + 1), got %d", len(variants))
	}

	// Verify base timeline info
	baseFound := false
	variantFound := false
	for _, variant := range variants {
		if variant.Identifier == "" && variant.FullName == "main" {
			baseFound = true
		}
		if variant.Identifier == "experiment" && variant.FullName == "main:diverged:experiment" {
			variantFound = true
		}
	}

	if !baseFound {
		t.Error("Base timeline not found in variants list")
	}
	if !variantFound {
		t.Error("Butterfly variant not found in variants list")
	}
}

func TestAutoNumberedButterflyCreation(t *testing.T) {
	// Create temporary directory for test
	tempDir, err := os.MkdirTemp("", "ivaldi_test")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Initialize repository
	repo, err := forge.Initialize(tempDir)
	if err != nil {
		t.Fatalf("Failed to initialize repository: %v", err)
	}

	timelineManager := repo.Timeline()

	// Test auto-numbered butterfly creation
	nextID := timelineManager.GetNextButterflyID("main")
	if nextID != 1 {
		t.Errorf("Expected first butterfly ID to be 1, got %d", nextID)
	}

	// Create first butterfly
	err = repo.CreateButterflyTimeline("1")
	if err != nil {
		t.Fatalf("Failed to create first butterfly timeline: %v", err)
	}

	// Test next ID incremented
	nextID = timelineManager.GetNextButterflyID("main")
	if nextID != 2 {
		t.Errorf("Expected next butterfly ID to be 2, got %d", nextID)
	}

	// Create second butterfly
	err = repo.CreateButterflyTimeline("2")
	if err != nil {
		t.Fatalf("Failed to create second butterfly timeline: %v", err)
	}

	// Verify both exist
	if !timelineManager.Exists("main:diverged:1") {
		t.Error("First butterfly timeline doesn't exist")
	}
	if !timelineManager.Exists("main:diverged:2") {
		t.Error("Second butterfly timeline doesn't exist")
	}
}
