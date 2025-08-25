package objects

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"time"

	"lukechampine.com/blake3"
)

// Hash algorithms
type HashAlgorithm int

const (
	BLAKE3 HashAlgorithm = iota
	SHA256
)

var algorithmNames = map[HashAlgorithm]string{
	BLAKE3: "blake3",
	SHA256: "sha256",
}

var algorithmFromName = map[string]HashAlgorithm{
	"blake3": BLAKE3,
	"sha256": SHA256,
}

func (a HashAlgorithm) String() string {
	if name, ok := algorithmNames[a]; ok {
		return name
	}
	return "unknown"
}

func ParseHashAlgorithm(name string) (HashAlgorithm, error) {
	if algo, ok := algorithmFromName[name]; ok {
		return algo, nil
	}
	return 0, fmt.Errorf("unknown hash algorithm: %s", name)
}

// New content-addressed hash type
type CAHash struct {
	Algorithm HashAlgorithm `json:"algorithm"`
	Value     [32]byte      `json:"value"`
}

// NewCAHash creates a hash using the specified algorithm
func NewCAHash(data []byte, algo HashAlgorithm) (CAHash, error) {
	if data == nil {
		return CAHash{}, fmt.Errorf("data cannot be nil")
	}
	if algo < 0 || algo >= HashAlgorithm(len(algorithmNames)) {
		return CAHash{}, fmt.Errorf("invalid hash algorithm: %v", algo)
	}

	var value [32]byte

	switch algo {
	case BLAKE3:
		value = blake3.Sum256(data)
	case SHA256:
		value = sha256.Sum256(data)
	default:
		return CAHash{}, fmt.Errorf("unsupported hash algorithm: %v", algo)
	}

	return CAHash{
		Algorithm: algo,
		Value:     value,
	}, nil
}

func (h CAHash) String() string {
	return hex.EncodeToString(h.Value[:])
}

func (h CAHash) FullString() string {
	return fmt.Sprintf("%s:%s", h.Algorithm.String(), h.String())
}

func (h CAHash) Bytes() []byte {
	return h.Value[:]
}

func (h CAHash) IsZero() bool {
	return h == CAHash{}
}

func (h CAHash) Equal(other CAHash) bool {
	return h.Algorithm == other.Algorithm && h.Value == other.Value
}

func (h CAHash) MarshalJSON() ([]byte, error) {
	return json.Marshal(h.FullString())
}

func (h *CAHash) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}

	parsed, err := ParseCAHash(s)
	if err != nil {
		return err
	}

	*h = parsed
	return nil
}

func ParseCAHash(s string) (CAHash, error) {
	if s == "" {
		return CAHash{}, nil
	}

	for algo, name := range algorithmNames {
		prefix := name + ":"
		if len(s) > len(prefix) && s[:len(prefix)] == prefix {
			hashStr := s[len(prefix):]
			bytes, err := hex.DecodeString(hashStr)
			if err != nil {
				return CAHash{}, fmt.Errorf("invalid hex in hash: %v", err)
			}
			if len(bytes) != 32 {
				return CAHash{}, fmt.Errorf("invalid hash length: %d", len(bytes))
			}

			var value [32]byte
			copy(value[:], bytes)
			return CAHash{Algorithm: algo, Value: value}, nil
		}
	}

	bytes, err := hex.DecodeString(s)
	if err != nil {
		return CAHash{}, fmt.Errorf("invalid hex in hash: %v", err)
	}
	if len(bytes) != 32 {
		return CAHash{}, fmt.Errorf("invalid hash length: %d", len(bytes))
	}

	var value [32]byte
	copy(value[:], bytes)
	return CAHash{Algorithm: BLAKE3, Value: value}, nil
}

func (h CAHash) Verify(data []byte) bool {
	expected, err := NewCAHash(data, h.Algorithm)
	if err != nil {
		return false
	}
	return h.Equal(expected)
}

func (h CAHash) ObjectPath() string {
	hashStr := h.String()
	return fmt.Sprintf("%s/%s/%s", h.Algorithm.String(), hashStr[:2], hashStr[2:])
}

// Legacy hash type for backward compatibility
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
	return Hash(sha256.Sum256(data))
}

func HashData(data []byte) Hash {
	return NewHash(data)
}

func (h Hash) IsZero() bool {
	return h == Hash{}
}

// Identity represents an author or committer
type Identity struct {
	Name  string `json:"name"`
	Email string `json:"email"`
}

// Legacy Seal structure
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

func (s *Seal) Serialize() ([]byte, error) {
	return json.Marshal(s)
}

func DeserializeSeal(data []byte) (*Seal, error) {
	var seal Seal
	err := json.Unmarshal(data, &seal)
	return &seal, err
}

// Legacy Tree structures
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

type ObjectType int

const (
	ObjectTypeBlob ObjectType = iota
	ObjectTypeTree
	ObjectTypeSeal
	ObjectTypeChunk
)

// Blob represents raw binary data
type Blob struct {
	Data []byte
}

func NewBlob(data []byte) *Blob {
	return &Blob{Data: data}
}

// Chunk represents a compressed data chunk
type Chunk struct {
	ID         Hash
	Data       []byte
	Size       int64
	RefCount   int32
	Compressed bool
}

// New content-addressed object types
type ObjectKind int

const (
	KindBlob ObjectKind = iota
	KindTree
	KindSeal
	KindTag
)

// CA TreeEntry represents a single entry in a content-addressed tree
type CATreeEntry struct {
	Mode uint32
	Name string
	Hash CAHash
	Kind ObjectKind
}

// CA Tree represents a directory tree structure
type CATree struct {
	Entries []CATreeEntry
}

func NewCATree(entries []CATreeEntry) *CATree {
	sortedEntries := make([]CATreeEntry, len(entries))
	copy(sortedEntries, entries)

	sort.Slice(sortedEntries, func(i, j int) bool {
		return sortedEntries[i].Name < sortedEntries[j].Name
	})

	return &CATree{Entries: sortedEntries}
}

func (t *CATree) Encode() ([]byte, error) {
	if len(t.Entries) == 0 {
		return []byte{}, nil
	}

	entries := make([]CATreeEntry, len(t.Entries))
	copy(entries, t.Entries)
	sort.Slice(entries, func(i, j int) bool {
		return entries[i].Name < entries[j].Name
	})

	var result []byte

	for _, entry := range entries {
		modeBuf := make([]byte, 4)
		binary.BigEndian.PutUint32(modeBuf, entry.Mode)
		result = append(result, modeBuf...)

		result = append(result, byte(entry.Kind))

		nameBytes := []byte(entry.Name)
		if len(nameBytes) > 65535 {
			return nil, fmt.Errorf("entry name too long: %d bytes", len(nameBytes))
		}
		nameLenBuf := make([]byte, 2)
		binary.BigEndian.PutUint16(nameLenBuf, uint16(len(nameBytes)))
		result = append(result, nameLenBuf...)

		result = append(result, nameBytes...)

		result = append(result, byte(entry.Hash.Algorithm))
		result = append(result, entry.Hash.Value[:]...)
	}

	return result, nil
}

func DecodeCATree(data []byte) (*CATree, error) {
	if len(data) == 0 {
		return &CATree{}, nil
	}

	var entries []CATreeEntry
	offset := 0

	for offset < len(data) {
		if offset+41 > len(data) {
			return nil, fmt.Errorf("incomplete tree entry at offset %d", offset)
		}

		mode := binary.BigEndian.Uint32(data[offset : offset+4])
		offset += 4

		kind := ObjectKind(data[offset])
		offset++

		nameLen := binary.BigEndian.Uint16(data[offset : offset+2])
		offset += 2

		if offset+int(nameLen) > len(data) {
			return nil, fmt.Errorf("incomplete name at offset %d", offset)
		}

		name := string(data[offset : offset+int(nameLen)])
		offset += int(nameLen)

		if offset+33 > len(data) {
			return nil, fmt.Errorf("incomplete hash at offset %d", offset)
		}

		hashAlgo := HashAlgorithm(data[offset])
		offset++

		// Validate hash algorithm
		if hashAlgo < 0 || hashAlgo >= HashAlgorithm(len(algorithmNames)) {
			return nil, fmt.Errorf("unknown hash algorithm %d at offset %d", hashAlgo, offset-33)
		}

		var hashValue [32]byte
		copy(hashValue[:], data[offset:offset+32])
		offset += 32

		hash := CAHash{
			Algorithm: hashAlgo,
			Value:     hashValue,
		}

		entries = append(entries, CATreeEntry{
			Mode: mode,
			Name: name,
			Hash: hash,
			Kind: kind,
		})
	}

	return NewCATree(entries), nil
}

// CA Seal represents a commit-like object
type CASeal struct {
	TreeHash  CAHash
	Parents   []CAHash
	Author    Identity
	Committer Identity
	Message   string
	Timestamp time.Time
}

func NewCASeal(treeHash CAHash, parents []CAHash, author, committer Identity, message string) *CASeal {
	return &CASeal{
		TreeHash:  treeHash,
		Parents:   parents,
		Author:    author,
		Committer: committer,
		Message:   message,
		Timestamp: time.Now().UTC(),
	}
}

func (s *CASeal) Encode() ([]byte, error) {
	var result []byte

	result = append(result, byte(s.TreeHash.Algorithm))
	result = append(result, s.TreeHash.Value[:]...)

	if len(s.Parents) > 65535 {
		return nil, fmt.Errorf("too many parents: %d", len(s.Parents))
	}
	parentCountBuf := make([]byte, 2)
	binary.BigEndian.PutUint16(parentCountBuf, uint16(len(s.Parents)))
	result = append(result, parentCountBuf...)

	for _, parent := range s.Parents {
		result = append(result, byte(parent.Algorithm))
		result = append(result, parent.Value[:]...)
	}

	authorNameBytes := []byte(s.Author.Name)
	if len(authorNameBytes) > 65535 {
		return nil, fmt.Errorf("author name too long: %d bytes", len(authorNameBytes))
	}
	authorNameLenBuf := make([]byte, 2)
	binary.BigEndian.PutUint16(authorNameLenBuf, uint16(len(authorNameBytes)))
	result = append(result, authorNameLenBuf...)
	result = append(result, authorNameBytes...)

	authorEmailBytes := []byte(s.Author.Email)
	if len(authorEmailBytes) > 65535 {
		return nil, fmt.Errorf("author email too long: %d bytes", len(authorEmailBytes))
	}
	authorEmailLenBuf := make([]byte, 2)
	binary.BigEndian.PutUint16(authorEmailLenBuf, uint16(len(authorEmailBytes)))
	result = append(result, authorEmailLenBuf...)
	result = append(result, authorEmailBytes...)

	committerNameBytes := []byte(s.Committer.Name)
	if len(committerNameBytes) > 65535 {
		return nil, fmt.Errorf("committer name too long: %d bytes", len(committerNameBytes))
	}
	committerNameLenBuf := make([]byte, 2)
	binary.BigEndian.PutUint16(committerNameLenBuf, uint16(len(committerNameBytes)))
	result = append(result, committerNameLenBuf...)
	result = append(result, committerNameBytes...)

	committerEmailBytes := []byte(s.Committer.Email)
	if len(committerEmailBytes) > 65535 {
		return nil, fmt.Errorf("committer email too long: %d bytes", len(committerEmailBytes))
	}
	committerEmailLenBuf := make([]byte, 2)
	binary.BigEndian.PutUint16(committerEmailLenBuf, uint16(len(committerEmailBytes)))
	result = append(result, committerEmailLenBuf...)
	result = append(result, committerEmailBytes...)

	timestampBuf := make([]byte, 8)
	binary.BigEndian.PutUint64(timestampBuf, uint64(s.Timestamp.UnixNano()))
	result = append(result, timestampBuf...)

	messageBytes := []byte(s.Message)
	if len(messageBytes) > 4294967295 {
		return nil, fmt.Errorf("message too long: %d bytes", len(messageBytes))
	}
	messageLenBuf := make([]byte, 4)
	binary.BigEndian.PutUint32(messageLenBuf, uint32(len(messageBytes)))
	result = append(result, messageLenBuf...)
	result = append(result, messageBytes...)

	return result, nil
}

func DecodeCASeal(data []byte) (*CASeal, error) {
	if len(data) < 33 {
		return nil, fmt.Errorf("seal data too short: %d bytes", len(data))
	}

	offset := 0

	treeHashAlgo := HashAlgorithm(data[offset])
	offset++

	// Validate hash algorithm
	if treeHashAlgo < 0 || treeHashAlgo >= HashAlgorithm(len(algorithmNames)) {
		return nil, fmt.Errorf("unknown hash algorithm %d at offset %d", treeHashAlgo, offset-1)
	}

	var treeHashValue [32]byte
	copy(treeHashValue[:], data[offset:offset+32])
	offset += 32

	treeHash := CAHash{
		Algorithm: treeHashAlgo,
		Value:     treeHashValue,
	}

	if offset+2 > len(data) {
		return nil, fmt.Errorf("incomplete parent count at offset %d", offset)
	}
	parentCount := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	var parents []CAHash
	for i := 0; i < int(parentCount); i++ {
		if offset+33 > len(data) {
			return nil, fmt.Errorf("incomplete parent hash at offset %d", offset)
		}

		parentHashAlgo := HashAlgorithm(data[offset])
		offset++

		// Validate hash algorithm
		if parentHashAlgo < 0 || parentHashAlgo >= HashAlgorithm(len(algorithmNames)) {
			return nil, fmt.Errorf("unknown hash algorithm %d at offset %d", parentHashAlgo, offset-34)
		}

		var parentHashValue [32]byte
		copy(parentHashValue[:], data[offset:offset+32])
		offset += 32

		parents = append(parents, CAHash{
			Algorithm: parentHashAlgo,
			Value:     parentHashValue,
		})
	}

	if offset+2 > len(data) {
		return nil, fmt.Errorf("incomplete author name length at offset %d", offset)
	}
	authorNameLen := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	if offset+int(authorNameLen) > len(data) {
		return nil, fmt.Errorf("incomplete author name at offset %d", offset)
	}
	authorName := string(data[offset : offset+int(authorNameLen)])
	offset += int(authorNameLen)

	if offset+2 > len(data) {
		return nil, fmt.Errorf("incomplete author email length at offset %d", offset)
	}
	authorEmailLen := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	if offset+int(authorEmailLen) > len(data) {
		return nil, fmt.Errorf("incomplete author email at offset %d", offset)
	}
	authorEmail := string(data[offset : offset+int(authorEmailLen)])
	offset += int(authorEmailLen)

	if offset+2 > len(data) {
		return nil, fmt.Errorf("incomplete committer name length at offset %d", offset)
	}
	committerNameLen := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	if offset+int(committerNameLen) > len(data) {
		return nil, fmt.Errorf("incomplete committer name at offset %d", offset)
	}
	committerName := string(data[offset : offset+int(committerNameLen)])
	offset += int(committerNameLen)

	if offset+2 > len(data) {
		return nil, fmt.Errorf("incomplete committer email length at offset %d", offset)
	}
	committerEmailLen := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	if offset+int(committerEmailLen) > len(data) {
		return nil, fmt.Errorf("incomplete committer email at offset %d", offset)
	}
	committerEmail := string(data[offset : offset+int(committerEmailLen)])
	offset += int(committerEmailLen)

	if offset+8 > len(data) {
		return nil, fmt.Errorf("incomplete timestamp at offset %d", offset)
	}
	timestampNanos := binary.BigEndian.Uint64(data[offset : offset+8])
	offset += 8
	timestamp := time.Unix(0, int64(timestampNanos)).UTC()

	if offset+4 > len(data) {
		return nil, fmt.Errorf("incomplete message length at offset %d", offset)
	}
	messageLen := binary.BigEndian.Uint32(data[offset : offset+4])
	offset += 4

	if offset+int(messageLen) > len(data) {
		return nil, fmt.Errorf("incomplete message at offset %d", offset)
	}
	message := string(data[offset : offset+int(messageLen)])

	return &CASeal{
		TreeHash: treeHash,
		Parents:  parents,
		Author: Identity{
			Name:  authorName,
			Email: authorEmail,
		},
		Committer: Identity{
			Name:  committerName,
			Email: committerEmail,
		},
		Message:   message,
		Timestamp: timestamp,
	}, nil
}

func (s *CASeal) IsRootSeal() bool {
	return len(s.Parents) == 0
}

func (s *CASeal) IsMerge() bool {
	return len(s.Parents) > 1
}

func (s *CASeal) String() string {
	if s.IsRootSeal() {
		return fmt.Sprintf("seal %s (root): %s", s.TreeHash.String()[:8], s.Message)
	}
	if s.IsMerge() {
		return fmt.Sprintf("seal %s (merge): %s", s.TreeHash.String()[:8], s.Message)
	}
	return fmt.Sprintf("seal %s -> %s: %s", s.Parents[0].String()[:8], s.TreeHash.String()[:8], s.Message)
}

// Common file modes
const (
	ModeFile       uint32 = 0o100644
	ModeExecutable uint32 = 0o100755
	ModeSymlink    uint32 = 0o120000
	ModeDirectory  uint32 = 0o040000
)
