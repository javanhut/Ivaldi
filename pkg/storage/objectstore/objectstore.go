package objectstore

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

type OID string

type ObjectStore struct {
	Root string
}

func New(repoRoot string) *ObjectStore {
	return &ObjectStore{Root: repoRoot}
}

func (s *ObjectStore) objDir() string      { return filepath.Join(s.Root, ".ivaldi", "objects") }
func (s *ObjectStore) ensure() error       { return os.MkdirAll(s.objDir(), 0o755) }
func (s *ObjectStore) path(oid OID) string { return filepath.Join(s.objDir(), string(oid)) }

func hashBytes(b []byte) OID {
	h := sha256.Sum256(b)
	return OID(hex.EncodeToString(h[:]))
}

// PutBlob stores raw file content.
func (s *ObjectStore) PutBlob(content []byte) (OID, error) {
	if err := s.ensure(); err != nil {
		return "", err
	}
	oid := hashBytes(content)
	p := s.path(oid)
	if _, err := os.Stat(p); errors.Is(err, os.ErrNotExist) {
		tmp := p + ".tmp"
		if err := os.WriteFile(tmp, content, 0o644); err != nil {
			return "", err
		}
		if err := os.Rename(tmp, p); err != nil {
			return "", err
		}
	}
	return oid, nil
}

func (s *ObjectStore) GetBlob(oid OID) ([]byte, error) {
	return os.ReadFile(s.path(oid))
}

// Tree = a directory snapshot
type Tree struct {
	Entries []TreeEntry `json:"entries"`
}

type TreeEntry struct {
	Path string `json:"path"`
	Mode string `json:"mode"`
	Blob OID    `json:"blob"`
	Size int64  `json:"size"`
}

func (s *ObjectStore) PutTree(t *Tree) (OID, error) {
	if err := s.ensure(); err != nil {
		return "", err
	}
	b, err := json.Marshal(t)
	if err != nil {
		return "", err
	}
	oid := hashBytes(b)
	p := s.path(oid)
	if _, err := os.Stat(p); errors.Is(err, os.ErrNotExist) {
		tmp := p + ".tmp"
		if err := os.WriteFile(tmp, b, 0o644); err != nil {
			return "", err
		}
		if err := os.Rename(tmp, p); err != nil {
			return "", err
		}
	}
	return oid, nil
}

func (s *ObjectStore) GetTree(oid OID) (*Tree, error) {
	b, err := os.ReadFile(s.path(oid))
	if err != nil {
		return nil, err
	}
	var t Tree
	if err := json.Unmarshal(b, &t); err != nil {
		return nil, err
	}
	return &t, nil
}

type Commit struct {
	Tree    OID               `json:"tree"`
	Parents []OID             `json:"parents"`
	Author  string            `json:"author"`
	Message string            `json:"message"`
	TimeUTC int64             `json:"time_utc"`
	Meta    map[string]string `json:"meta,omitempty"`
}

func (s *ObjectStore) PutCommit(c *Commit) (OID, error) {
	if err := s.ensure(); err != nil {
		return "", err
	}
	b, err := json.Marshal(c)
	if err != nil {
		return "", err
	}
	oid := hashBytes(b)
	p := s.path(oid)
	if _, err := os.Stat(p); errors.Is(err, os.ErrNotExist) {
		tmp := p + ".tmp"
		if err := os.WriteFile(tmp, b, 0o644); err != nil {
			return "", err
		}
		if err := os.Rename(tmp, p); err != nil {
			return "", err
		}
	}
	return oid, nil
}

func (s *ObjectStore) GetCommit(oid OID) (*Commit, error) {
	b, err := os.ReadFile(s.path(oid))
	if err != nil {
		return nil, err
	}
	var c Commit
	if err := json.Unmarshal(b, &c); err != nil {
		return nil, err
	}
	return &c, nil
}

func CopyFileAtomic(dst, src string, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
		return err
	}
	tmp := dst + ".tmp"
	out, err := os.OpenFile(tmp, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	in, err := os.Open(src)
	if err != nil {
		out.Close()
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		in.Close()
		out.Close()
		return err
	}
	in.Close()
	if err := out.Sync(); err != nil {
		out.Close()
		return err
	}
	if err := out.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, dst)
}

func RepoDir(root string) string { return filepath.Join(root, ".ivaldi") }

func EnsureRepo(root string) error {
	return os.MkdirAll(RepoDir(root), 0o755)
}

func IsInsideRepo(root string) error {
	if _, err := os.Stat(RepoDir(root)); err != nil {
		return fmt.Errorf("not an Ivaldi repo: %w", err)
	}
	return nil
}
