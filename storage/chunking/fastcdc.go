package chunking

import (
	"crypto/sha256"
	"fmt"
	"io"
)

// FastCDC implements Fast Content-Defined Chunking
// Based on the paper "FastCDC: a Fast and Efficient Content-Defined Chunking Approach"
type FastCDC struct {
	avgSize int    // Average chunk size (8KB default)
	minSize int    // Minimum chunk size (512 bytes default)
	maxSize int    // Maximum chunk size (64KB default)
	mask    uint64 // Rolling hash mask
	
	// Rolling hash state
	gear [256]uint64 // Gear table for rolling hash
}

// Chunk represents a content-defined chunk
type Chunk struct {
	Data   []byte `json:"data"`
	Hash   string `json:"hash"`
	Size   int    `json:"size"`
	Offset int64  `json:"offset"`
}

// ChunkResult contains chunking results
type ChunkResult struct {
	Chunks      []*Chunk `json:"chunks"`
	TotalSize   int      `json:"totalSize"`
	ChunkCount  int      `json:"chunkCount"`
	Dedup       bool     `json:"deduplication"`
	Compression string   `json:"compression"`
}

// NewFastCDC creates a new FastCDC chunker with default settings
func NewFastCDC() *FastCDC {
	return NewFastCDCWithSizes(8192, 512, 65536) // 8KB avg, 512B min, 64KB max
}

// NewFastCDCWithSizes creates a FastCDC chunker with custom sizes
func NewFastCDCWithSizes(avgSize, minSize, maxSize int) *FastCDC {
	cdc := &FastCDC{
		avgSize: avgSize,
		minSize: minSize,
		maxSize: maxSize,
		mask:    uint64(avgSize - 1), // For 8KB: 0x1FFF
	}
	
	// Initialize gear table for rolling hash
	cdc.initializeGear()
	
	return cdc
}

// initializeGear initializes the gear table for rolling hash
func (cdc *FastCDC) initializeGear() {
	// Simple gear table initialization
	// In a production implementation, this would use a more sophisticated approach
	for i := 0; i < 256; i++ {
		cdc.gear[i] = uint64(i) * 0x9E3779B97F4A7C15 // Golden ratio constant
	}
}

// ChunkData splits data into content-defined chunks
func (cdc *FastCDC) ChunkData(data []byte) (*ChunkResult, error) {
	if len(data) == 0 {
		return &ChunkResult{
			Chunks:     []*Chunk{},
			TotalSize:  0,
			ChunkCount: 0,
		}, nil
	}
	
	var chunks []*Chunk
	var offset int64 = 0
	
	for offset < int64(len(data)) {
		chunk, err := cdc.findNextChunk(data[offset:])
		if err != nil {
			return nil, err
		}
		
		chunk.Offset = offset
		chunks = append(chunks, chunk)
		offset += int64(chunk.Size)
	}
	
	return &ChunkResult{
		Chunks:     chunks,
		TotalSize:  len(data),
		ChunkCount: len(chunks),
		Dedup:      true,
		Compression: "none", // Would be zstd/lz4 in production
	}, nil
}

// findNextChunk finds the next chunk boundary using rolling hash
func (cdc *FastCDC) findNextChunk(data []byte) (*Chunk, error) {
	if len(data) == 0 {
		return nil, fmt.Errorf("no data to chunk")
	}
	
	// If remaining data is smaller than minSize, take it all
	if len(data) <= cdc.minSize {
		return cdc.createChunk(data), nil
	}
	
	// Rolling hash state
	var hash uint64 = 0
	
	// Build initial hash window (minimum size)
	for i := 0; i < cdc.minSize && i < len(data); i++ {
		hash = cdc.updateHash(hash, data[i])
	}
	
	// Look for chunk boundary starting from minSize
	for pos := cdc.minSize; pos < len(data) && pos < cdc.maxSize; pos++ {
		hash = cdc.updateHash(hash, data[pos])
		
		// Check if this is a chunk boundary
		if cdc.isChunkBoundary(hash) {
			return cdc.createChunk(data[:pos+1]), nil
		}
	}
	
	// If we reach maxSize, force a boundary
	size := cdc.maxSize
	if size > len(data) {
		size = len(data)
	}
	
	return cdc.createChunk(data[:size]), nil
}

// updateHash updates the rolling hash with a new byte
func (cdc *FastCDC) updateHash(hash uint64, b byte) uint64 {
	// Simple rolling hash using gear table
	return ((hash << 1) + cdc.gear[b])
}

// isChunkBoundary determines if the current hash indicates a chunk boundary
func (cdc *FastCDC) isChunkBoundary(hash uint64) bool {
	// Use mask to determine boundary
	// This creates chunks with an average size of avgSize
	return (hash & cdc.mask) == 0
}

// createChunk creates a chunk from data
func (cdc *FastCDC) createChunk(data []byte) *Chunk {
	// Calculate SHA-256 hash of chunk
	hasher := sha256.New()
	hasher.Write(data)
	hash := fmt.Sprintf("%x", hasher.Sum(nil))
	
	// Copy data to avoid reference issues
	chunkData := make([]byte, len(data))
	copy(chunkData, data)
	
	return &Chunk{
		Data: chunkData,
		Hash: hash,
		Size: len(data),
	}
}

// ChunkReader chunks data from an io.Reader
func (cdc *FastCDC) ChunkReader(reader io.Reader) (*ChunkResult, error) {
	// Read all data into memory
	// In a production implementation, this would stream and chunk incrementally
	data, err := io.ReadAll(reader)
	if err != nil {
		return nil, fmt.Errorf("failed to read data: %v", err)
	}
	
	return cdc.ChunkData(data)
}

// DeduplicationManager manages chunk deduplication
type DeduplicationManager struct {
	chunks    map[string]*Chunk // hash -> chunk
	refCounts map[string]int    // hash -> reference count
}

// NewDeduplicationManager creates a new deduplication manager
func NewDeduplicationManager() *DeduplicationManager {
	return &DeduplicationManager{
		chunks:    make(map[string]*Chunk),
		refCounts: make(map[string]int),
	}
}

// AddChunk adds a chunk to the deduplication store
func (dm *DeduplicationManager) AddChunk(chunk *Chunk) bool {
	if _, exists := dm.chunks[chunk.Hash]; exists {
		// Chunk already exists, increment reference count
		dm.refCounts[chunk.Hash]++
		return true // Deduplicated
	}
	
	// New chunk, store it
	dm.chunks[chunk.Hash] = chunk
	dm.refCounts[chunk.Hash] = 1
	return false // Not deduplicated
}

// GetChunk retrieves a chunk by hash
func (dm *DeduplicationManager) GetChunk(hash string) (*Chunk, bool) {
	chunk, exists := dm.chunks[hash]
	return chunk, exists
}

// RemoveChunk removes a reference to a chunk
func (dm *DeduplicationManager) RemoveChunk(hash string) bool {
	if count, exists := dm.refCounts[hash]; exists {
		if count <= 1 {
			// Last reference, remove chunk
			delete(dm.chunks, hash)
			delete(dm.refCounts, hash)
			return true // Chunk deleted
		} else {
			// Decrement reference count
			dm.refCounts[hash]--
			return false // Chunk still referenced
		}
	}
	return false // Chunk not found
}

// GetStats returns deduplication statistics
func (dm *DeduplicationManager) GetStats() DeduplicationStats {
	totalChunks := len(dm.chunks)
	totalRefs := 0
	totalSize := 0
	
	for hash, chunk := range dm.chunks {
		refs := dm.refCounts[hash]
		totalRefs += refs
		totalSize += chunk.Size * refs // Size if not deduplicated
	}
	
	actualSize := 0
	for _, chunk := range dm.chunks {
		actualSize += chunk.Size
	}
	
	var ratio float64 = 1.0
	if actualSize > 0 {
		ratio = float64(totalSize) / float64(actualSize)
	}
	
	return DeduplicationStats{
		UniqueChunks:     totalChunks,
		TotalReferences:  totalRefs,
		DeduplicationRatio: ratio,
		StorageSize:      actualSize,
		LogicalSize:      totalSize,
	}
}

// DeduplicationStats contains deduplication statistics
type DeduplicationStats struct {
	UniqueChunks       int     `json:"uniqueChunks"`
	TotalReferences    int     `json:"totalReferences"`
	DeduplicationRatio float64 `json:"deduplicationRatio"`
	StorageSize        int     `json:"storageSize"`      // Actual storage used
	LogicalSize        int     `json:"logicalSize"`      // Size without deduplication
}

// Compression support (placeholder for zstd/lz4)
type CompressionType int

const (
	CompressionNone CompressionType = iota
	CompressionZstd
	CompressionLZ4
)

// CompressedChunk represents a compressed chunk
type CompressedChunk struct {
	*Chunk
	OriginalSize    int             `json:"originalSize"`
	CompressedSize  int             `json:"compressedSize"`
	CompressionType CompressionType `json:"compressionType"`
	CompressionRatio float64        `json:"compressionRatio"`
}

// CompressChunk compresses a chunk (placeholder implementation)
func CompressChunk(chunk *Chunk, compressionType CompressionType) *CompressedChunk {
	// Placeholder - in production this would use actual zstd/lz4
	originalSize := len(chunk.Data)
	
	// Simulate compression
	var compressedData []byte
	var ratio float64
	
	switch compressionType {
	case CompressionZstd:
		// Simulate zstd compression (typically 60-70% reduction)
		compressedData = make([]byte, int(float64(originalSize)*0.35))
		copy(compressedData, chunk.Data[:len(compressedData)])
		ratio = float64(originalSize) / float64(len(compressedData))
	case CompressionLZ4:
		// Simulate LZ4 compression (typically 40-50% reduction)
		compressedData = make([]byte, int(float64(originalSize)*0.55))
		copy(compressedData, chunk.Data[:len(compressedData)])
		ratio = float64(originalSize) / float64(len(compressedData))
	default:
		compressedData = chunk.Data
		ratio = 1.0
	}
	
	compressedChunk := &Chunk{
		Data: compressedData,
		Hash: chunk.Hash,
		Size: len(compressedData),
		Offset: chunk.Offset,
	}
	
	return &CompressedChunk{
		Chunk:           compressedChunk,
		OriginalSize:    originalSize,
		CompressedSize:  len(compressedData),
		CompressionType: compressionType,
		CompressionRatio: ratio,
	}
}

// StorageEfficiency calculates overall storage efficiency
func CalculateStorageEfficiency(deduplicationRatio, compressionRatio float64) StorageEfficiency {
	totalRatio := deduplicationRatio * compressionRatio
	savings := (1.0 - (1.0 / totalRatio)) * 100.0
	
	return StorageEfficiency{
		DeduplicationRatio: deduplicationRatio,
		CompressionRatio:   compressionRatio,
		TotalRatio:         totalRatio,
		SpaceSavings:       savings,
	}
}

// StorageEfficiency represents overall storage efficiency metrics
type StorageEfficiency struct {
	DeduplicationRatio float64 `json:"deduplicationRatio"` // 10:1 target
	CompressionRatio   float64 `json:"compressionRatio"`   // 2:1 typical
	TotalRatio         float64 `json:"totalRatio"`         // Combined ratio
	SpaceSavings       float64 `json:"spaceSavings"`       // Percentage saved
}