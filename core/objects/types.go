package objects

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"time"
)

type Hash [32]byte

func (h Hash) String() string {
	return fmt.Sprintf("%x", h[:])
}

func (h Hash) MarshalJSON() ([]byte, error) {
	return json.Marshal(fmt.Sprintf("%x", h[:]))
}

func (h *Hash) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	
	bytes, err := hex.DecodeString(s)
	if err != nil {
		return err
	}
	
	if len(bytes) != 32 {
		return fmt.Errorf("invalid hash length: %d", len(bytes))
	}
	
	copy(h[:], bytes)
	return nil
}

func NewHash(data []byte) Hash {
	return sha256.Sum256(data)
}

// HashData is an alias for NewHash for consistency
func HashData(data []byte) Hash {
	return NewHash(data)
}

type Identity struct {
	Name  string
	Email string
}

type Overwrite struct {
	PreviousHash Hash
	Reason       string
	Author       Identity
	Timestamp    time.Time
}

type Seal struct {
	Hash       Hash
	Name       string
	Iteration  int
	Position   Hash
	Parents    []Hash
	Message    string
	Author     Identity
	Timestamp  time.Time
	Overwrites []Overwrite
}

// Serialize converts seal to JSON bytes
func (s *Seal) Serialize() ([]byte, error) {
	return json.Marshal(s)
}

// DeserializeSeal creates a seal from JSON bytes
func DeserializeSeal(data []byte) (*Seal, error) {
	var seal Seal
	err := json.Unmarshal(data, &seal)
	return &seal, err
}

type TreeEntry struct {
	Name string
	Hash Hash
	Mode uint32
	Type ObjectType
}

type Tree struct {
	Hash    Hash
	Entries []TreeEntry
}

type Blob struct {
	Hash Hash
	Size int64
	Data []byte
}

type Chunk struct {
	ID         Hash
	Data       []byte
	Size       int64
	RefCount   int32
	Compressed bool
}

type ObjectType int

const (
	ObjectTypeBlob ObjectType = iota
	ObjectTypeTree
	ObjectTypeSeal
	ObjectTypeChunk
)