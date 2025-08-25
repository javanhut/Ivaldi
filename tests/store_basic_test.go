package tests

import (
	"bytes"
	"crypto/rand"
	"os"
	"testing"

	"ivaldi/core/objects"
	"ivaldi/storage/local"
)

func TestBasicStoreOperations(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-basic-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	// Test BLAKE3 store
	store, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create store: %v", err)
	}

	// Test 1KB blob
	testData1KB := make([]byte, 1024)
	if _, err := rand.Read(testData1KB); err != nil {
		t.Fatalf("Failed to generate test data: %v", err)
	}

	hash1KB, err := store.Put(testData1KB, local.KindBlob)
	if err != nil {
		t.Fatalf("Failed to put 1KB blob: %v", err)
	}

	// Verify hash algorithm
	if hash1KB.Algorithm != objects.BLAKE3 {
		t.Errorf("Expected BLAKE3 algorithm, got %v", hash1KB.Algorithm)
	}

	// Test retrieval
	retrievedData, kind, err := store.Get(hash1KB)
	if err != nil {
		t.Fatalf("Failed to get 1KB blob: %v", err)
	}

	if kind != local.KindBlob {
		t.Errorf("Expected kind %v, got %v", local.KindBlob, kind)
	}

	if !bytes.Equal(retrievedData, testData1KB) {
		t.Errorf("Retrieved data does not match original")
	}

	// Test exists
	if !store.Exists(hash1KB) {
		t.Errorf("Store should report blob exists")
	}

	// Test verify
	if err := store.Verify(hash1KB); err != nil {
		t.Errorf("Failed to verify blob: %v", err)
	}

	t.Logf("Successfully stored and retrieved 1KB blob with hash: %s", hash1KB.FullString())
}

func TestLargeBlob(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-large-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	store, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create store: %v", err)
	}

	// Test 5MB blob
	testData5MB := make([]byte, 5*1024*1024)
	if _, err := rand.Read(testData5MB); err != nil {
		t.Fatalf("Failed to generate test data: %v", err)
	}

	hash5MB, err := store.Put(testData5MB, local.KindBlob)
	if err != nil {
		t.Fatalf("Failed to put 5MB blob: %v", err)
	}

	retrievedData, kind, err := store.Get(hash5MB)
	if err != nil {
		t.Fatalf("Failed to get 5MB blob: %v", err)
	}

	if kind != local.KindBlob {
		t.Errorf("Expected kind %v, got %v", local.KindBlob, kind)
	}

	if !bytes.Equal(retrievedData, testData5MB) {
		t.Errorf("Retrieved 5MB data does not match original")
	}

	t.Logf("Successfully stored and retrieved 5MB blob with hash: %s", hash5MB.FullString())
}

func TestHashAlgorithms(t *testing.T) {
	// Create temporary directory for testing
	tempDir, err := os.MkdirTemp("", "ivaldi-algo-test-")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	testData := []byte("test data for hash algorithm comparison")

	// Test BLAKE3
	storeBLAKE3, err := local.NewStore(tempDir, objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create BLAKE3 store: %v", err)
	}

	hashBLAKE3, err := storeBLAKE3.Put(testData, local.KindBlob)
	if err != nil {
		t.Fatalf("Failed to put data with BLAKE3: %v", err)
	}

	// Test SHA256
	storeSHA256, err := local.NewStore(tempDir, objects.SHA256)
	if err != nil {
		t.Fatalf("Failed to create SHA256 store: %v", err)
	}

	hashSHA256, err := storeSHA256.Put(testData, local.KindBlob)
	if err != nil {
		t.Fatalf("Failed to put data with SHA256: %v", err)
	}

	// Verify algorithms
	if hashBLAKE3.Algorithm != objects.BLAKE3 {
		t.Errorf("Expected BLAKE3, got %v", hashBLAKE3.Algorithm)
	}

	if hashSHA256.Algorithm != objects.SHA256 {
		t.Errorf("Expected SHA256, got %v", hashSHA256.Algorithm)
	}

	// Hashes should be different
	if hashBLAKE3.Equal(hashSHA256) {
		t.Errorf("BLAKE3 and SHA256 hashes should be different")
	}

	// Both stores should be able to retrieve both objects (same underlying storage)
	data1, _, err := storeBLAKE3.Get(hashBLAKE3)
	if err != nil {
		t.Fatalf("Failed to get BLAKE3 data: %v", err)
	}

	data2, _, err := storeSHA256.Get(hashSHA256)
	if err != nil {
		t.Fatalf("Failed to get SHA256 data: %v", err)
	}

	if !bytes.Equal(data1, testData) || !bytes.Equal(data2, testData) {
		t.Errorf("Retrieved data does not match original")
	}

	t.Logf("BLAKE3 hash: %s", hashBLAKE3.FullString())
	t.Logf("SHA256 hash: %s", hashSHA256.FullString())
}

func TestTreeEncoding(t *testing.T) {
	// Create test tree
	hash1, err := objects.NewCAHash([]byte("file1"), objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create hash1: %v", err)
	}
	hash2, err := objects.NewCAHash([]byte("file2"), objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create hash2: %v", err)
	}

	entries := []objects.CATreeEntry{
		{Mode: objects.ModeFile, Name: "file1.txt", Hash: hash1, Kind: objects.KindBlob},
		{Mode: objects.ModeFile, Name: "file2.txt", Hash: hash2, Kind: objects.KindBlob},
	}

	tree := objects.NewCATree(entries)

	// Test encoding stability
	encoded1, err := tree.Encode()
	if err != nil {
		t.Fatalf("Failed to encode tree: %v", err)
	}

	decoded, err := objects.DecodeCATree(encoded1)
	if err != nil {
		t.Fatalf("Failed to decode tree: %v", err)
	}

	encoded2, err := decoded.Encode()
	if err != nil {
		t.Fatalf("Failed to encode tree again: %v", err)
	}

	if !bytes.Equal(encoded1, encoded2) {
		t.Errorf("Tree encoding is not stable")
	}

	t.Logf("Tree encoding test passed - %d bytes", len(encoded1))
}

func TestSealEncoding(t *testing.T) {
	// Create test seal
	treeHash, err := objects.NewCAHash([]byte("tree"), objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create treeHash: %v", err)
	}
	parentHash, err := objects.NewCAHash([]byte("parent"), objects.BLAKE3)
	if err != nil {
		t.Fatalf("Failed to create parentHash: %v", err)
	}

	author := objects.Identity{Name: "Test Author", Email: "test@example.com"}
	committer := objects.Identity{Name: "Test Committer", Email: "committer@example.com"}

	seal := objects.NewCASeal(treeHash, []objects.CAHash{parentHash}, author, committer, "Test commit")

	// Test encoding stability
	encoded1, err := seal.Encode()
	if err != nil {
		t.Fatalf("Failed to encode seal: %v", err)
	}

	decoded, err := objects.DecodeCASeal(encoded1)
	if err != nil {
		t.Fatalf("Failed to decode seal: %v", err)
	}

	encoded2, err := decoded.Encode()
	if err != nil {
		t.Fatalf("Failed to encode seal again: %v", err)
	}

	if !bytes.Equal(encoded1, encoded2) {
		t.Errorf("Seal encoding is not stable")
	}

	// Verify fields
	if !decoded.TreeHash.Equal(treeHash) {
		t.Errorf("Tree hash mismatch")
	}

	if len(decoded.Parents) != 1 || !decoded.Parents[0].Equal(parentHash) {
		t.Errorf("Parent hash mismatch")
	}

	if decoded.Author.Name != author.Name || decoded.Author.Email != author.Email {
		t.Errorf("Author mismatch")
	}

	t.Logf("Seal encoding test passed - %d bytes", len(encoded1))
}
