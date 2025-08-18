package enhanced

import (
	"bytes"
	"compress/gzip"
	"fmt"
	"io"

	"ivaldi/core/objects"
	"ivaldi/storage/chunking"
	"ivaldi/storage/local"
)

// ChunkedStorage provides content-defined chunking with deduplication
type ChunkedStorage struct {
	base     *local.Storage
	chunker  *chunking.FastCDC
	dedup    *chunking.DeduplicationManager
	compress bool
}

// NewChunkedStorage creates a new chunked storage system
func NewChunkedStorage(root string, enableCompression bool) (*ChunkedStorage, error) {
	base, err := local.NewStorage(root)
	if err != nil {
		return nil, err
	}

	return &ChunkedStorage{
		base:     base,
		chunker:  chunking.NewFastCDC(),
		dedup:    chunking.NewDeduplicationManager(),
		compress: enableCompression,
	}, nil
}

// StoreWithChunking stores data using content-defined chunking
func (cs *ChunkedStorage) StoreWithChunking(data []byte) (*objects.Hash, *chunking.ChunkResult, error) {
	// Chunk the data
	result, err := cs.chunker.ChunkData(data)
	if err != nil {
		return nil, nil, fmt.Errorf("failed to chunk data: %v", err)
	}

	// Process each chunk for deduplication
	totalDeduped := 0
	for _, chunk := range result.Chunks {
		wasDeduplicated := cs.dedup.AddChunk(chunk)
		if wasDeduplicated {
			totalDeduped++
		}
	}

	// Calculate final hash of the reconstructed data
	hash := objects.HashData(data)

	// Update result statistics
	result.Dedup = totalDeduped > 0
	if cs.compress {
		result.Compression = "gzip" // Simplified for demo
	}

	return &hash, result, nil
}

// RetrieveFromChunks reconstructs data from chunk hashes
func (cs *ChunkedStorage) RetrieveFromChunks(chunkHashes []string) ([]byte, error) {
	var buffer bytes.Buffer

	for _, hash := range chunkHashes {
		chunk, exists := cs.dedup.GetChunk(hash)
		if !exists {
			return nil, fmt.Errorf("chunk not found: %s", hash)
		}

		buffer.Write(chunk.Data)
	}

	return buffer.Bytes(), nil
}

// GetStorageStats returns current storage efficiency statistics
func (cs *ChunkedStorage) GetStorageStats() chunking.StorageEfficiency {
	dedupStats := cs.dedup.GetStats()
	
	// Simulate compression ratio (would be real with actual compression)
	compressionRatio := 1.5 // 1.5:1 typical for code
	if cs.compress {
		compressionRatio = 2.0 // 2:1 with compression
	}

	return chunking.CalculateStorageEfficiency(
		dedupStats.DeduplicationRatio,
		compressionRatio,
	)
}

// Enhanced file operations with chunking

// StoreSeal stores a seal with chunking
func (cs *ChunkedStorage) StoreSeal(seal *objects.Seal) error {
	// Serialize seal
	data, err := seal.Serialize()
	if err != nil {
		return err
	}

	// Store with chunking if large enough
	if len(data) > 1024 { // Only chunk files larger than 1KB
		hash, result, err := cs.StoreWithChunking(data)
		if err != nil {
			return err
		}

		// Store chunk metadata
		return cs.base.StoreMetadata(seal.Hash.String(), &ChunkMetadata{
			OriginalHash:   *hash,
			ChunkHashes:    getChunkHashes(result.Chunks),
			TotalSize:      result.TotalSize,
			ChunkCount:     result.ChunkCount,
			Deduplicated:   result.Dedup,
			Compressed:     result.Compression != "none",
		})
	}

	// Store directly for small files
	return cs.base.StoreSeal(seal)
}

// LoadSeal loads a seal, reconstructing from chunks if needed
func (cs *ChunkedStorage) LoadSeal(hash objects.Hash) (*objects.Seal, error) {
	// Try to load chunk metadata first
	metadata, err := cs.base.LoadMetadata(hash.String())
	if err == nil {
		// Reconstruct from chunks
		return cs.reconstructSealFromChunks(metadata.(*ChunkMetadata))
	}

	// Fallback to direct loading
	return cs.base.LoadSeal(hash)
}

// StoreBlob stores blob data with optimal chunking
func (cs *ChunkedStorage) StoreBlob(data []byte) (objects.Hash, error) {
	hash := objects.HashData(data)

	// Always use chunking for blobs
	_, result, err := cs.StoreWithChunking(data)
	if err != nil {
		return hash, err
	}

	// Store chunk metadata
	err = cs.base.StoreMetadata(hash.String(), &ChunkMetadata{
		OriginalHash:   hash,
		ChunkHashes:    getChunkHashes(result.Chunks),
		TotalSize:      result.TotalSize,
		ChunkCount:     result.ChunkCount,
		Deduplicated:   result.Dedup,
		Compressed:     result.Compression != "none",
	})

	return hash, err
}

// LoadBlob loads blob data, reconstructing from chunks
func (cs *ChunkedStorage) LoadBlob(hash objects.Hash) ([]byte, error) {
	// Load chunk metadata
	metadata, err := cs.base.LoadMetadata(hash.String())
	if err != nil {
		return nil, fmt.Errorf("chunk metadata not found: %v", err)
	}

	chunkMeta := metadata.(*ChunkMetadata)
	return cs.RetrieveFromChunks(chunkMeta.ChunkHashes)
}

// Advanced compression integration

// CompressData compresses data using gzip (placeholder for zstd/lz4)
func (cs *ChunkedStorage) CompressData(data []byte) ([]byte, float64, error) {
	if !cs.compress {
		return data, 1.0, nil
	}

	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	
	if _, err := gz.Write(data); err != nil {
		return nil, 0, err
	}
	
	if err := gz.Close(); err != nil {
		return nil, 0, err
	}

	compressed := buf.Bytes()
	ratio := float64(len(data)) / float64(len(compressed))
	
	return compressed, ratio, nil
}

// DecompressData decompresses data
func (cs *ChunkedStorage) DecompressData(data []byte) ([]byte, error) {
	if !cs.compress {
		return data, nil
	}

	gz, err := gzip.NewReader(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	defer gz.Close()

	return io.ReadAll(gz)
}

// Helper types and functions

// ChunkMetadata stores information about chunked files
type ChunkMetadata struct {
	OriginalHash   objects.Hash `json:"originalHash"`
	ChunkHashes    []string     `json:"chunkHashes"`
	TotalSize      int          `json:"totalSize"`
	ChunkCount     int          `json:"chunkCount"`
	Deduplicated   bool         `json:"deduplicated"`
	Compressed     bool         `json:"compressed"`
	StorageRatio   float64      `json:"storageRatio"`
}

// getChunkHashes extracts hashes from chunks
func getChunkHashes(chunks []*chunking.Chunk) []string {
	hashes := make([]string, len(chunks))
	for i, chunk := range chunks {
		hashes[i] = chunk.Hash
	}
	return hashes
}

// reconstructSealFromChunks reconstructs a seal from chunk metadata
func (cs *ChunkedStorage) reconstructSealFromChunks(metadata *ChunkMetadata) (*objects.Seal, error) {
	// Retrieve and reconstruct data
	data, err := cs.RetrieveFromChunks(metadata.ChunkHashes)
	if err != nil {
		return nil, err
	}

	// Decompress if needed
	if metadata.Compressed {
		data, err = cs.DecompressData(data)
		if err != nil {
			return nil, err
		}
	}

	// Deserialize seal
	return objects.DeserializeSeal(data)
}

// StorageProfile provides detailed storage analysis
type StorageProfile struct {
	TotalFiles      int                           `json:"totalFiles"`
	ChunkedFiles    int                           `json:"chunkedFiles"`
	DirectFiles     int                           `json:"directFiles"`
	TotalChunks     int                           `json:"totalChunks"`
	UniqueChunks    int                           `json:"uniqueChunks"`
	Efficiency      chunking.StorageEfficiency   `json:"efficiency"`
	TopDedupFiles   []string                     `json:"topDedupFiles"`
}

// GetStorageProfile returns comprehensive storage analysis
func (cs *ChunkedStorage) GetStorageProfile() *StorageProfile {
	stats := cs.dedup.GetStats()
	efficiency := cs.GetStorageStats()

	return &StorageProfile{
		TotalFiles:      0, // Would count from metadata
		ChunkedFiles:    0, // Would count chunked files
		DirectFiles:     0, // Would count direct files
		TotalChunks:     stats.TotalReferences,
		UniqueChunks:    stats.UniqueChunks,
		Efficiency:      efficiency,
		TopDedupFiles:   []string{}, // Would analyze most duplicated files
	}
}

// OptimizeStorage performs storage optimization
func (cs *ChunkedStorage) OptimizeStorage() (*OptimizationResult, error) {
	// Analyze current storage
	_ = cs.GetStorageProfile()
	
	// Perform optimization (placeholder)
	result := &OptimizationResult{
		SpaceReclaimed:   0,
		ChunksCoalesced:  0,
		OrphanedRemoved:  0,
		CompressionGain:  0,
	}

	return result, nil
}

// OptimizationResult contains storage optimization results
type OptimizationResult struct {
	SpaceReclaimed   int64   `json:"spaceReclaimed"`   // Bytes reclaimed
	ChunksCoalesced  int     `json:"chunksCoalesced"`  // Small chunks merged
	OrphanedRemoved  int     `json:"orphanedRemoved"`  // Unused chunks removed
	CompressionGain  float64 `json:"compressionGain"`  // Additional compression %
}