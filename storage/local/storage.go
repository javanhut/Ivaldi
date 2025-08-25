package local

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/klauspost/compress/zstd"
	"ivaldi/core/objects"
)

type Storage struct {
	root       string
	objectsDir string
	compressor *zstd.Encoder
}

func NewStorage(root string) (*Storage, error) {
	objectsDir := filepath.Join(root, ".ivaldi", "objects")
	if err := os.MkdirAll(objectsDir, 0755); err != nil {
		return nil, err
	}

	compressor, err := zstd.NewWriter(nil)
	if err != nil {
		return nil, err
	}

	return &Storage{
		root:       root,
		objectsDir: objectsDir,
		compressor: compressor,
	}, nil
}

func (s *Storage) StoreObject(obj interface{}) (objects.Hash, error) {
	data, err := json.Marshal(obj)
	if err != nil {
		return objects.Hash{}, err
	}

	hash := objects.NewHash(data)
	return hash, s.writeObject(hash, data, false)
}

func (s *Storage) StoreSeal(seal *objects.Seal) error {
	data, err := json.Marshal(seal)
	if err != nil {
		return err
	}

	seal.Hash = objects.NewHash(data)
	return s.writeObject(seal.Hash, data, false)
}

func (s *Storage) StoreChunk(chunk *objects.Chunk) error {
	var data []byte

	if len(chunk.Data) > 1024 {
		data = s.compressor.EncodeAll(chunk.Data, nil)
		chunk.Compressed = true
	} else {
		data = chunk.Data
		chunk.Compressed = false
	}

	return s.writeObject(chunk.ID, data, chunk.Compressed)
}

func (s *Storage) LoadSeal(hash objects.Hash) (*objects.Seal, error) {
	data, err := s.readObject(hash)
	if err != nil {
		return nil, err
	}

	var seal objects.Seal
	if err := json.Unmarshal(data, &seal); err != nil {
		return nil, err
	}

	// Ensure the seal has the correct hash set
	seal.Hash = hash

	return &seal, nil
}

func (s *Storage) LoadTree(hash objects.Hash) (*objects.Tree, error) {
	data, err := s.readObject(hash)
	if err != nil {
		return nil, err
	}

	var tree objects.Tree
	if err := json.Unmarshal(data, &tree); err != nil {
		return nil, err
	}

	return &tree, nil
}

func (s *Storage) LoadBlob(hash objects.Hash) (*objects.Blob, error) {
	data, err := s.readObject(hash)
	if err != nil {
		return nil, err
	}

	var blob objects.Blob
	if err := json.Unmarshal(data, &blob); err != nil {
		return nil, err
	}

	return &blob, nil
}

func (s *Storage) LoadChunk(hash objects.Hash) (*objects.Chunk, error) {
	data, err := s.readObject(hash)
	if err != nil {
		return nil, err
	}

	chunk := &objects.Chunk{
		ID:   hash,
		Size: int64(len(data)),
		Data: data,
	}

	if s.isCompressed(hash) {
		decoder, err := zstd.NewReader(nil)
		if err != nil {
			return nil, err
		}
		defer decoder.Close()

		decompressed, err := decoder.DecodeAll(data, nil)
		if err != nil {
			return nil, err
		}

		chunk.Data = decompressed
		chunk.Size = int64(len(decompressed))
		chunk.Compressed = true
	}

	return chunk, nil
}

func (s *Storage) Exists(hash objects.Hash) bool {
	path := s.objectPath(hash)
	_, err := os.Stat(path)
	return err == nil
}

func (s *Storage) writeObject(hash objects.Hash, data []byte, compressed bool) error {
	path := s.objectPath(hash)
	dir := filepath.Dir(path)

	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	if compressed {
		path += ".zst"
	}

	return os.WriteFile(path, data, 0644)
}

func (s *Storage) readObject(hash objects.Hash) ([]byte, error) {
	path := s.objectPath(hash)

	if _, err := os.Stat(path); err != nil {
		compressedPath := path + ".zst"
		if _, err := os.Stat(compressedPath); err == nil {
			path = compressedPath
		} else {
			return nil, fmt.Errorf("object not found: %x", hash)
		}
	}

	return os.ReadFile(path)
}

func (s *Storage) isCompressed(hash objects.Hash) bool {
	path := s.objectPath(hash) + ".zst"
	_, err := os.Stat(path)
	return err == nil
}

func (s *Storage) objectPath(hash objects.Hash) string {
	hashStr := fmt.Sprintf("%x", hash)
	return filepath.Join(s.objectsDir, hashStr[:2], hashStr[2:])
}

// GetObjectsDir returns the path to the objects directory for iteration purposes
func (s *Storage) GetObjectsDir() string {
	return s.objectsDir
}

func (s *Storage) GC() error {
	return nil
}

func (s *Storage) Stats() (StorageStats, error) {
	var stats StorageStats

	err := filepath.Walk(s.objectsDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}

		if !info.IsDir() {
			stats.ObjectCount++
			stats.TotalSize += info.Size()
		}

		return nil
	})

	return stats, err
}

func (s *Storage) Backup(writer io.Writer) error {
	return nil
}

func (s *Storage) Restore(reader io.Reader) error {
	return nil
}

func (s *Storage) Close() error {
	if s.compressor != nil {
		s.compressor.Close()
	}
	return nil
}

type StorageStats struct {
	ObjectCount int64
	TotalSize   int64
	ChunkCount  int64
	SealCount   int64
	TreeCount   int64
	BlobCount   int64
}

// StoreMetadata stores metadata for chunked objects
func (s *Storage) StoreMetadata(key string, metadata interface{}) error {
	metadataDir := filepath.Join(s.root, ".ivaldi", "metadata")
	if err := os.MkdirAll(metadataDir, 0755); err != nil {
		return err
	}

	data, err := json.Marshal(metadata)
	if err != nil {
		return err
	}

	path := filepath.Join(metadataDir, key+".meta")
	return os.WriteFile(path, data, 0644)
}

// LoadMetadata loads metadata for chunked objects
func (s *Storage) LoadMetadata(key string) (interface{}, error) {
	path := filepath.Join(s.root, ".ivaldi", "metadata", key+".meta")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}

	var metadata interface{}
	err = json.Unmarshal(data, &metadata)
	return metadata, err
}
