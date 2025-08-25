package local

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"

	"ivaldi/core/objects"
)

// ObjectKind represents the type of object stored
type ObjectKind int

const (
	KindBlob ObjectKind = iota
	KindTree
	KindSeal
	KindTag
)

var kindNames = map[ObjectKind]string{
	KindBlob: "blob",
	KindTree: "tree",
	KindSeal: "seal",
	KindTag:  "tag",
}

func (k ObjectKind) String() string {
	if name, ok := kindNames[k]; ok {
		return name
	}
	return "unknown"
}

// Store is a content-addressed object store
type Store struct {
	root      string
	objectDir string
	algorithm objects.HashAlgorithm
	mu        sync.RWMutex
}

// NewStore creates a new content-addressed object store
func NewStore(root string, algorithm objects.HashAlgorithm) (*Store, error) {
	objectDir := filepath.Join(root, ".ivaldi", "objects")
	if err := os.MkdirAll(objectDir, 0755); err != nil {
		return nil, fmt.Errorf("failed to create objects directory: %v", err)
	}

	return &Store{
		root:      root,
		objectDir: objectDir,
		algorithm: algorithm,
	}, nil
}

// Put stores data with the given kind and returns its hash
func (s *Store) Put(data []byte, kind ObjectKind) (objects.CAHash, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	// Calculate hash
	hash, err := objects.NewCAHash(data, s.algorithm)
	if err != nil {
		return objects.CAHash{}, fmt.Errorf("failed to create hash: %w", err)
	}

	// Check if object already exists
	if s.exists(hash) {
		return hash, nil
	}

	// Create object directory structure
	objectPath := filepath.Join(s.objectDir, hash.ObjectPath())
	objectDir := filepath.Dir(objectPath)
	if err := os.MkdirAll(objectDir, 0755); err != nil {
		return objects.CAHash{}, fmt.Errorf("failed to create object directory: %v", err)
	}

	// Write to temporary file first for atomic operation
	tempPath := objectPath + ".tmp"
	tempFile, err := os.OpenFile(tempPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
	if err != nil {
		return objects.CAHash{}, fmt.Errorf("failed to create temp file: %v", err)
	}

	// Write kind header and data
	if _, err := tempFile.Write([]byte{byte(kind)}); err != nil {
		tempFile.Close()
		os.Remove(tempPath)
		return objects.CAHash{}, fmt.Errorf("failed to write kind: %v", err)
	}

	if _, err := tempFile.Write(data); err != nil {
		tempFile.Close()
		os.Remove(tempPath)
		return objects.CAHash{}, fmt.Errorf("failed to write data: %v", err)
	}

	// Ensure data is written to disk
	if err := tempFile.Sync(); err != nil {
		tempFile.Close()
		os.Remove(tempPath)
		return objects.CAHash{}, fmt.Errorf("failed to sync temp file: %v", err)
	}

	if err := tempFile.Close(); err != nil {
		os.Remove(tempPath)
		return objects.CAHash{}, fmt.Errorf("failed to close temp file: %v", err)
	}

	// Atomic rename
	if err := os.Rename(tempPath, objectPath); err != nil {
		os.Remove(tempPath)
		return objects.CAHash{}, fmt.Errorf("failed to rename object file: %v", err)
	}

	// Sync directory to ensure rename is durable
	dirFile, err := os.Open(objectDir)
	if err == nil {
		dirFile.Sync()
		dirFile.Close()
	}

	return hash, nil
}

// Get retrieves an object by its hash
func (s *Store) Get(hash objects.CAHash) ([]byte, ObjectKind, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	objectPath := filepath.Join(s.objectDir, hash.ObjectPath())

	file, err := os.Open(objectPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, 0, fmt.Errorf("object not found: %s", hash.String())
		}
		return nil, 0, fmt.Errorf("failed to open object: %v", err)
	}
	defer file.Close()

	// Read kind
	kindBuf := make([]byte, 1)
	if _, err := file.Read(kindBuf); err != nil {
		return nil, 0, fmt.Errorf("failed to read object kind: %v", err)
	}
	kind := ObjectKind(kindBuf[0])

	// Read data
	data, err := io.ReadAll(file)
	if err != nil {
		return nil, 0, fmt.Errorf("failed to read object data: %v", err)
	}

	return data, kind, nil
}

// Exists checks if an object exists
func (s *Store) Exists(hash objects.CAHash) bool {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.exists(hash)
}

// exists checks if an object exists (without locking)
func (s *Store) exists(hash objects.CAHash) bool {
	objectPath := filepath.Join(s.objectDir, hash.ObjectPath())
	_, err := os.Stat(objectPath)
	return err == nil
}

// Verify checks if an object's content matches its hash
func (s *Store) Verify(hash objects.CAHash) error {
	data, _, err := s.Get(hash)
	if err != nil {
		return err
	}

	if !hash.Verify(data) {
		return fmt.Errorf("hash verification failed for %s", hash.String())
	}

	return nil
}

// GetAlgorithm returns the hash algorithm used by this store
func (s *Store) GetAlgorithm() objects.HashAlgorithm {
	return s.algorithm
}

// SetAlgorithm changes the hash algorithm (for new objects)
func (s *Store) SetAlgorithm(algo objects.HashAlgorithm) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.algorithm = algo
}

// List returns all object hashes of a specific kind
func (s *Store) List(kind ObjectKind) ([]objects.CAHash, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	var hashes []objects.CAHash

	// Walk through all algorithm directories
	for algo := range map[objects.HashAlgorithm]bool{objects.BLAKE3: true, objects.SHA256: true} {
		algoDir := filepath.Join(s.objectDir, algo.String())
		if _, err := os.Stat(algoDir); os.IsNotExist(err) {
			continue
		}

		err := filepath.WalkDir(algoDir, func(path string, d os.DirEntry, err error) error {
			if err != nil {
				return err
			}

			if d.IsDir() {
				return nil
			}

			// Extract hash from path
			relPath, err := filepath.Rel(algoDir, path)
			if err != nil {
				return err
			}

			// Skip if not in correct format (should be xx/xxxxx...)
			if len(relPath) < 3 || relPath[2] != '/' {
				return nil
			}

			hashStr := relPath[:2] + relPath[3:]
			hash, err := objects.ParseCAHash(algo.String() + ":" + hashStr)
			if err != nil {
				return nil // Skip invalid hashes
			}

			// Check if object has the requested kind
			_, objKind, err := s.Get(hash)
			if err != nil {
				return nil // Skip unreadable objects
			}

			if objKind == kind {
				hashes = append(hashes, hash)
			}

			return nil
		})

		if err != nil {
			return nil, fmt.Errorf("failed to walk objects: %v", err)
		}
	}

	return hashes, nil
}

// GC performs garbage collection by removing unreferenced objects
func (s *Store) GC(referencedHashes map[objects.CAHash]bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	// Walk through all objects and remove unreferenced ones
	for algo := range map[objects.HashAlgorithm]bool{objects.BLAKE3: true, objects.SHA256: true} {
		algoDir := filepath.Join(s.objectDir, algo.String())
		if _, err := os.Stat(algoDir); os.IsNotExist(err) {
			continue
		}

		err := filepath.WalkDir(algoDir, func(path string, d os.DirEntry, err error) error {
			if err != nil {
				return err
			}

			if d.IsDir() {
				return nil
			}

			// Extract hash from path
			relPath, err := filepath.Rel(algoDir, path)
			if err != nil {
				return err
			}

			if len(relPath) < 3 || relPath[2] != '/' {
				return nil
			}

			hashStr := relPath[:2] + relPath[3:]
			hash, err := objects.ParseCAHash(algo.String() + ":" + hashStr)
			if err != nil {
				return nil
			}

			// Remove if not referenced
			if !referencedHashes[hash] {
				os.Remove(path)
			}

			return nil
		})

		if err != nil {
			return fmt.Errorf("failed to walk objects during GC: %v", err)
		}
	}

	return nil
}

// Stats returns statistics about the object store
type StoreStats struct {
	ObjectCount     int64
	TotalSize       int64
	BlobCount       int64
	TreeCount       int64
	SealCount       int64
	TagCount        int64
	AlgorithmCounts map[objects.HashAlgorithm]int64
}

func (s *Store) Stats() (StoreStats, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	stats := StoreStats{
		AlgorithmCounts: make(map[objects.HashAlgorithm]int64),
	}

	err := filepath.WalkDir(s.objectDir, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}

		if d.IsDir() {
			return nil
		}

		info, err := d.Info()
		if err != nil {
			return err
		}

		stats.ObjectCount++
		stats.TotalSize += info.Size()

		// Try to determine object kind and algorithm
		for algo := range map[objects.HashAlgorithm]bool{objects.BLAKE3: true, objects.SHA256: true} {
			algoDir := filepath.Join(s.objectDir, algo.String())
			if filepath.HasPrefix(path, algoDir) {
				stats.AlgorithmCounts[algo]++

				// Try to read object kind
				relPath, err := filepath.Rel(algoDir, path)
				if err == nil && len(relPath) >= 3 && relPath[2] == '/' {
					hashStr := relPath[:2] + relPath[3:]
					hash, err := objects.ParseCAHash(algo.String() + ":" + hashStr)
					if err == nil {
						_, kind, err := s.Get(hash)
						if err == nil {
							switch kind {
							case KindBlob:
								stats.BlobCount++
							case KindTree:
								stats.TreeCount++
							case KindSeal:
								stats.SealCount++
							case KindTag:
								stats.TagCount++
							}
						}
					}
				}
				break
			}
		}

		return nil
	})

	return stats, err
}
