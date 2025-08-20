package snapshot

import (
    "crypto/sha256"
    "encoding/hex"
    "io/fs"
    "os"
    "path/filepath"
    "strings"

    "ivaldi/pkg/storage/objectstore"
)

// WriteTree scans the working directory rooted at repoRoot and builds a
// content-addressed snapshot (Tree) in the object store. It ignores .ivaldi/.
func WriteTree(repoRoot string, store *objectstore.ObjectStore) (objectstore.OID, error) {
    var entries []objectstore.TreeEntry

    err := filepath.WalkDir(repoRoot, func(p string, d fs.DirEntry, err error) error {
        if err != nil {
            return err
        }
        rel, _ := filepath.Rel(repoRoot, p)
        if rel == "." {
            return nil
        }
        rel = filepath.ToSlash(rel)

        // Never include repository metadata.
        if strings.HasPrefix(rel, ".ivaldi/") {
            if d.IsDir() {
                return filepath.SkipDir
            }
            return nil
        }

        // Only snapshot regular files (directories are implied by paths).
        if d.IsDir() {
            return nil
        }

        info, err := d.Info()
        if err != nil {
            return err
        }
        content, err := os.ReadFile(p)
        if err != nil {
            return err
        }
        blobOID, err := store.PutBlob(content)
        if err != nil {
            return err
        }

        mode := "100644"
        if info.Mode()&0o111 != 0 {
            mode = "100755"
        }

        entries = append(entries, objectstore.TreeEntry{
            Path: rel,
            Mode: mode,
            Blob: blobOID,
            Size: info.Size(),
        })
        return nil
    })
    if err != nil {
        return "", err
    }

    return store.PutTree(&objectstore.Tree{Entries: entries})
}

// HashBytes returns a hex SHA-256 of b.
// Used by diff to compare working files against a base snapshot cheaply.
func HashBytes(b []byte) string {
    sum := sha256.Sum256(b)
    return hex.EncodeToString(sum[:])
}