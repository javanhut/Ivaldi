package chunks

import (
	"crypto/sha256"
	"hash"
	"io"

	"ivaldi/core/objects"
)

const (
	MinChunkSize     = 2 * 1024      // 2KB
	AvgChunkSize     = 8 * 1024      // 8KB
	MaxChunkSize     = 32 * 1024     // 32KB
	NormalizeLevel   = 2
	MaskShort        = 0x751e365
	MaskLong         = 0x74d715ad
)

type Chunker struct {
	hasher hash.Hash
}

func NewChunker() *Chunker {
	return &Chunker{
		hasher: sha256.New(),
	}
}

func (c *Chunker) ChunkData(data []byte) ([]*objects.Chunk, error) {
	var chunks []*objects.Chunk
	offset := 0
	
	for offset < len(data) {
		chunkSize := c.findBoundary(data[offset:])
		if chunkSize == 0 {
			chunkSize = len(data) - offset
		}
		
		chunkData := data[offset : offset+chunkSize]
		hash := objects.NewHash(chunkData)
		
		chunk := &objects.Chunk{
			ID:         hash,
			Data:       chunkData,
			Size:       int64(len(chunkData)),
			RefCount:   1,
			Compressed: false,
		}
		
		chunks = append(chunks, chunk)
		offset += chunkSize
	}
	
	return chunks, nil
}

func (c *Chunker) ChunkReader(reader io.Reader) ([]*objects.Chunk, error) {
	data, err := io.ReadAll(reader)
	if err != nil {
		return nil, err
	}
	return c.ChunkData(data)
}

func (c *Chunker) findBoundary(data []byte) int {
	if len(data) <= MinChunkSize {
		return len(data)
	}
	
	if len(data) >= MaxChunkSize {
		boundary := c.findBoundaryInRange(data[:MaxChunkSize], MaxChunkSize-MinChunkSize, MinChunkSize)
		if boundary > 0 {
			return boundary
		}
		return MaxChunkSize
	}
	
	boundary := c.findBoundaryInRange(data, len(data)-MinChunkSize, MinChunkSize)
	if boundary > 0 {
		return boundary
	}
	
	return len(data)
}

func (c *Chunker) findBoundaryInRange(data []byte, length, start int) int {
	if start >= len(data) || start >= length {
		return 0
	}
	
	if start+1 >= len(data) {
		return 0
	}
	
	fingerprint := uint32(data[start])<<1 + uint32(data[start+1])
	
	for i := start + 2; i < length && i < len(data); i++ {
		fingerprint = (fingerprint << 1) + uint32(data[i])
		
		if i >= AvgChunkSize-1 {
			if fingerprint&MaskLong == 0 {
				return i + 1
			}
		} else if i >= MinChunkSize+NormalizeLevel-1 {
			if fingerprint&MaskShort == 0 {
				return i + 1
			}
		}
	}
	
	return 0
}

func (c *Chunker) ComputeFingerprint(data []byte, start, end int) uint32 {
	if start >= end || start >= len(data) {
		return 0
	}
	
	if end > len(data) {
		end = len(data)
	}
	
	var fingerprint uint32
	for i := start; i < end; i++ {
		fingerprint = (fingerprint << 1) + uint32(data[i])
	}
	
	return fingerprint
}

type DeduplicationStats struct {
	OriginalSize     int64
	CompressedSize   int64
	UniqueChunks     int
	DuplicateChunks  int
	CompressionRatio float64
}

func (c *Chunker) AnalyzeDeduplication(chunks []*objects.Chunk) DeduplicationStats {
	seen := make(map[objects.Hash]bool)
	var originalSize, uniqueSize int64
	var uniqueCount, duplicateCount int
	
	for _, chunk := range chunks {
		originalSize += chunk.Size
		
		if !seen[chunk.ID] {
			seen[chunk.ID] = true
			uniqueSize += chunk.Size
			uniqueCount++
		} else {
			duplicateCount++
		}
	}
	
	ratio := 1.0
	if uniqueSize > 0 {
		ratio = float64(originalSize) / float64(uniqueSize)
	}
	
	return DeduplicationStats{
		OriginalSize:     originalSize,
		CompressedSize:   uniqueSize,
		UniqueChunks:     uniqueCount,
		DuplicateChunks:  duplicateCount,
		CompressionRatio: ratio,
	}
}