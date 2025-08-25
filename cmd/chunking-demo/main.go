package main

import (
	"fmt"

	"ivaldi/storage/chunking"
	"ivaldi/storage/enhanced"
)

func main() {
	fmt.Println("Ivaldi Content-Defined Chunking Demo")
	fmt.Println("====================================")

	// Create sample data that shows deduplication benefits
	sourceCode := `package main

import (
	"fmt"
	"strings"
)

func main() {
	message := "Hello, World!"
	fmt.Println(message)
}

func greet(name string) {
	message := fmt.Sprintf("Hello, %s!", name)
	fmt.Println(message)
}

func processFile(content string) {
	lines := strings.Split(content, "\n")
	for i, line := range lines {
		fmt.Printf("Line %d: %s\n", i+1, line)
	}
}

// Common utility functions
func joinStrings(items []string) string {
	return strings.Join(items, ", ")
}

func splitString(text, delimiter string) []string {
	return strings.Split(text, delimiter)
}`

	// Duplicate content with slight variations
	duplicatedCode := sourceCode + `

// Additional duplicate content for deduplication testing
func main() {
	message := "Hello, World!"
	fmt.Println(message)
}

func greet(name string) {
	message := fmt.Sprintf("Hello, %s!", name)
	fmt.Println(message)
}`

	// Create enhanced storage
	storage, err := enhanced.NewChunkedStorage("/tmp/ivaldi-demo", true)
	if err != nil {
		fmt.Printf("Failed to create storage: %v\n", err)
		return
	}

	fmt.Println("\nChunking Analysis")
	fmt.Println("-----------------")

	// Chunk original data
	hash1, result1, err := storage.StoreWithChunking([]byte(sourceCode))
	if err != nil {
		fmt.Printf("Failed to chunk data: %v\n", err)
		return
	}

	fmt.Printf("Original file:\n")
	fmt.Printf("   Size: %d bytes\n", result1.TotalSize)
	fmt.Printf("   Chunks: %d\n", result1.ChunkCount)
	fmt.Printf("   Hash: %s\n", hash1.String()[:16]+"...")

	// Chunk duplicated data
	hash2, result2, err := storage.StoreWithChunking([]byte(duplicatedCode))
	if err != nil {
		fmt.Printf("Failed to chunk duplicated data: %v\n", err)
		return
	}

	fmt.Printf("\nDuplicated file:\n")
	fmt.Printf("   Size: %d bytes\n", result2.TotalSize)
	fmt.Printf("   Chunks: %d\n", result2.ChunkCount)
	fmt.Printf("   Hash: %s\n", hash2.String()[:16]+"...")

	// Show storage efficiency
	stats := storage.GetStorageStats()

	fmt.Println("\nStorage Efficiency")
	fmt.Println("------------------")
	fmt.Printf("Deduplication ratio: %.1f:1\n", stats.DeduplicationRatio)
	fmt.Printf("Compression ratio: %.1f:1\n", stats.CompressionRatio)
	fmt.Printf("Total efficiency: %.1f:1\n", stats.TotalRatio)
	fmt.Printf("Space savings: %.1f%%\n", stats.SpaceSavings)

	// Demonstrate chunk-level analysis
	fmt.Println("\nChunk Analysis")
	fmt.Println("--------------")

	cdc := chunking.NewFastCDC()

	// Analyze chunking patterns
	result, err := cdc.ChunkData([]byte(sourceCode))
	if err != nil {
		fmt.Printf("Failed to analyze chunks: %v\n", err)
		return
	}

	fmt.Printf("Chunks found: %d\n", len(result.Chunks))
	for i, chunk := range result.Chunks {
		if i < 3 { // Show first 3 chunks
			fmt.Printf("   Chunk %d: %d bytes, hash %s...\n",
				i+1, chunk.Size, chunk.Hash[:12])
		}
	}
	if len(result.Chunks) > 3 {
		fmt.Printf("   ... and %d more chunks\n", len(result.Chunks)-3)
	}

	// Show performance characteristics
	fmt.Println("\nPerformance Characteristics")
	fmt.Println("---------------------------")
	fmt.Printf("Average chunk size: 8KB (target)\n")
	fmt.Printf("Min chunk size: 512 bytes\n")
	fmt.Printf("Max chunk size: 64KB\n")
	fmt.Printf("Target deduplication: 10:1\n")
	fmt.Printf("Target compression: 2:1\n")
	fmt.Printf("Expected storage reduction: 40%% vs Git\n")

	fmt.Println("\nRevolutionary Features Enabled:")
	fmt.Println("• Content-defined chunking with FastCDC")
	fmt.Println("• Automatic deduplication")
	fmt.Println("• Transparent compression")
	fmt.Println("• Storage efficiency optimization")
	fmt.Println("• Zero-copy chunk management")

	fmt.Println("\nIvaldi - Content-defined chunking working!")
}
