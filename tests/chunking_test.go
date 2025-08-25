package tests

import (
	"bytes"
	"testing"

	"ivaldi/core/chunks"
)

func TestFastCDCBasicChunking(t *testing.T) {
	chunker := chunks.NewChunker()

	data := make([]byte, 64*1024)
	for i := range data {
		data[i] = byte(i % 256)
	}

	chunkList, err := chunker.ChunkData(data)
	if err != nil {
		t.Fatalf("Failed to chunk data: %v", err)
	}

	if len(chunkList) == 0 {
		t.Error("Expected at least one chunk")
	}

	var totalSize int64
	for _, chunk := range chunkList {
		totalSize += chunk.Size

		if chunk.Size < chunks.MinChunkSize && chunk != chunkList[len(chunkList)-1] {
			t.Errorf("Chunk size %d is below minimum %d", chunk.Size, chunks.MinChunkSize)
		}

		if chunk.Size > chunks.MaxChunkSize {
			t.Errorf("Chunk size %d exceeds maximum %d", chunk.Size, chunks.MaxChunkSize)
		}
	}

	if totalSize != int64(len(data)) {
		t.Errorf("Total chunk size %d doesn't match original data size %d", totalSize, len(data))
	}
}

func TestChunkDeduplication(t *testing.T) {
	chunker := chunks.NewChunker()

	basePattern := []byte("Hello, this is a test pattern for deduplication! ")

	data := make([]byte, 0)
	for i := 0; i < 20; i++ {
		data = append(data, basePattern...)
	}

	chunkList, err := chunker.ChunkData(data)
	if err != nil {
		t.Fatalf("Failed to chunk data: %v", err)
	}

	stats := chunker.AnalyzeDeduplication(chunkList)

	if stats.OriginalSize != int64(len(data)) {
		t.Errorf("Expected original size %d, got %d", len(data), stats.OriginalSize)
	}

	if stats.UniqueChunks == 0 {
		t.Error("Expected at least one unique chunk")
	}
}

func TestChunkReaderInterface(t *testing.T) {
	chunker := chunks.NewChunker()

	data := []byte("Hello, this is a test for the chunk reader interface!")
	reader := bytes.NewReader(data)

	chunkList, err := chunker.ChunkReader(reader)
	if err != nil {
		t.Fatalf("Failed to chunk from reader: %v", err)
	}

	if len(chunkList) == 0 {
		t.Error("Expected at least one chunk")
	}

	var reconstructed []byte
	for _, chunk := range chunkList {
		reconstructed = append(reconstructed, chunk.Data...)
	}

	if !bytes.Equal(data, reconstructed) {
		t.Error("Reconstructed data doesn't match original")
	}
}

func TestEmptyDataChunking(t *testing.T) {
	chunker := chunks.NewChunker()

	chunkList, err := chunker.ChunkData([]byte{})
	if err != nil {
		t.Fatalf("Failed to chunk empty data: %v", err)
	}

	if len(chunkList) != 0 {
		t.Errorf("Expected 0 chunks for empty data, got %d", len(chunkList))
	}
}

func TestSmallDataChunking(t *testing.T) {
	chunker := chunks.NewChunker()

	data := []byte("small")
	chunkList, err := chunker.ChunkData(data)
	if err != nil {
		t.Fatalf("Failed to chunk small data: %v", err)
	}

	if len(chunkList) != 1 {
		t.Errorf("Expected 1 chunk for small data, got %d", len(chunkList))
	}

	if !bytes.Equal(chunkList[0].Data, data) {
		t.Error("Chunk data doesn't match original small data")
	}
}
