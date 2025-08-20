package diff

import (
    "os"
    "path/filepath"
    "strings"

    "ivaldi/pkg/storage/objectstore"
)

// StashEntry describes a single file change between the working directory and a base tree.
type StashEntry struct {
    Path    string
    BaseOID objectstore.OID // OID of the file in the base tree, empty if new
    NewOID  objectstore.OID // OID of the file in the working dir, empty if deleted
}

type Stash struct {
    Entries []StashEntry
}

// BuildStash compares the current working directory to a base tree.
// It returns a Stash of new, modified, and deleted files.
func BuildStash(repoRoot string, store *objectstore.ObjectStore, baseTree objectstore.OID) (*Stash, error) {
    base, err := store.GetTree(baseTree)
    if err != nil {
        return nil, err
    }
    baseMap := make(map[string]objectstore.TreeEntry)
    for _, e := range base.Entries {
        baseMap[e.Path] = e
    }

    type ent = objectstore.TreeEntry
    var workEntries []ent
    seen := make(map[string]struct{})

    // Walk working directory
    err = filepath.Walk(repoRoot, func(p string, info os.FileInfo, err error) error {
        if err != nil {
            return err
        }
        rel, _ := filepath.Rel(repoRoot, p)
        if rel == "." {
            return nil
        }
        rel = filepath.ToSlash(rel)

        // Ignore repo metadata
        if strings.HasPrefix(rel, ".ivaldi/") {
            if info.IsDir() {
                return filepath.SkipDir
            }
            return nil
        }

        if info.IsDir() {
            return nil
        }

        content, err := os.ReadFile(p)
        if err != nil {
            return err
        }
        
        // Store the actual file content in the object store
        blobOID, err := store.PutBlob(content)
        if err != nil {
            return err
        }
        
        mode := "100644"
        if info.Mode()&0o111 != 0 {
            mode = "100755"
        }
        workEntries = append(workEntries, ent{Path: rel, Mode: mode, Blob: blobOID, Size: info.Size()})
        seen[rel] = struct{}{}
        return nil
    })
    if err != nil {
        return nil, err
    }

    var stashEntries []StashEntry

    // Detect new and modified files
    for _, e := range workEntries {
        if be, ok := baseMap[e.Path]; ok {
            if be.Blob != e.Blob || be.Mode != e.Mode {
                stashEntries = append(stashEntries, StashEntry{
                    Path:    e.Path,
                    BaseOID: be.Blob,
                    NewOID:  e.Blob,
                })
            }
        } else {
            // new file
            stashEntries = append(stashEntries, StashEntry{
                Path:    e.Path,
                BaseOID: "",
                NewOID:  e.Blob,
            })
        }
    }

    // Detect deletions
    for pth, be := range baseMap {
        if _, ok := seen[pth]; !ok {
            stashEntries = append(stashEntries, StashEntry{
                Path:    pth,
                BaseOID: be.Blob,
                NewOID:  "",
            })
        }
    }

    return &Stash{Entries: stashEntries}, nil
}