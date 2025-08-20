package worktree

import (
    "fmt"
    "os"
    "path/filepath"
    "sort"
    "strings"

    "ivaldi/pkg/storage/objectstore"
)

// CheckoutTreeAtomic replaces the working directory contents with the snapshot
// from a given tree. It ensures that only files in the target tree exist after
// the operation, and it applies changes atomically per-file.
func CheckoutTreeAtomic(repoRoot string, store *objectstore.ObjectStore, target objectstore.OID) error {
    if err := objectstore.EnsureRepo(repoRoot); err != nil {
        return err
    }

    tree, err := store.GetTree(target)
    if err != nil {
        return err
    }

    // Stage files into a temporary overlay directory.
    tmpOverlay := filepath.Join(repoRoot, ".ivaldi", "tmp", "overlay")
    if err := os.RemoveAll(tmpOverlay); err != nil {
        return err
    }
    if err := os.MkdirAll(tmpOverlay, 0o755); err != nil {
        return err
    }

    for _, e := range tree.Entries {
        dst := filepath.Join(tmpOverlay, filepath.FromSlash(e.Path))
        if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
            return err
        }
        content, err := store.GetBlob(e.Blob)
        if err != nil {
            return err
        }
        mode := os.FileMode(0o644)
        if e.Mode == "100755" {
            mode = 0o755
        }
        if err := os.WriteFile(dst, content, mode); err != nil {
            return err
        }
    }

    // Remove obsolete files from the workdir.
    targetSet := make(map[string]struct{}, len(tree.Entries))
    for _, e := range tree.Entries {
        targetSet[filepath.Join(repoRoot, filepath.FromSlash(e.Path))] = struct{}{}
    }

    var toDelete []string
    err = filepath.Walk(repoRoot, func(p string, info os.FileInfo, err error) error {
        if err != nil {
            return err
        }
        rel, _ := filepath.Rel(repoRoot, p)
        rel = filepath.ToSlash(rel)

        // Skip metadata
        if strings.HasPrefix(rel, ".ivaldi/") {
            if info.IsDir() {
                return filepath.SkipDir
            }
            return nil
        }

        if info.IsDir() {
            return nil
        }

        if _, ok := targetSet[p]; !ok {
            toDelete = append(toDelete, p)
        }
        return nil
    })
    if err != nil {
        return err
    }

    // Delete deeper paths first.
    sort.Slice(toDelete, func(i, j int) bool { return len(toDelete[i]) > len(toDelete[j]) })
    for _, p := range toDelete {
        if err := os.Remove(p); err != nil && !os.IsNotExist(err) {
            return err
        }
    }

    // Move overlay files into place.
    err = filepath.Walk(tmpOverlay, func(p string, info os.FileInfo, err error) error {
        if err != nil {
            return err
        }
        if info.IsDir() {
            return nil
        }
        rel, _ := filepath.Rel(tmpOverlay, p)
        dst := filepath.Join(repoRoot, rel)
        if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
            return err
        }
        tmp := dst + ".swp"
        if err := os.Rename(p, tmp); err != nil {
            return err
        }
        if err := os.Rename(tmp, dst); err != nil {
            return err
        }
        return nil
    })
    if err != nil {
        return err
    }

    _ = os.RemoveAll(filepath.Join(repoRoot, ".ivaldi", "tmp"))
    return nil
}

// --- Stash support ---

type StashEntry struct {
    Path    string
    BaseOID objectstore.OID // OID from base tree
    NewOID  objectstore.OID // OID for new content, or "" if deleted
}

type Stash struct {
    Entries []StashEntry
}

// ReapplyStash applies the stash entries on top of the given tree in the workdir.
// If both the stash and target tree modified the same file, a conflict marker file is written.
func ReapplyStash(repoRoot string, store *objectstore.ObjectStore, targetTree objectstore.OID, stash *Stash) error {
    tree, err := store.GetTree(targetTree)
    if err != nil {
        return err
    }
    targetMap := make(map[string]objectstore.OID)
    for _, e := range tree.Entries {
        targetMap[e.Path] = e.Blob
    }

    for _, se := range stash.Entries {
        dst := filepath.Join(repoRoot, filepath.FromSlash(se.Path))
        switch {
        case se.NewOID == "":
            // deletion
            _ = os.Remove(dst)

        default:
            newBytes, err := store.GetBlob(se.NewOID)
            if err != nil {
                return err
            }
            // Detect conflict
            if se.BaseOID != "" {
                if tgtOID, ok := targetMap[se.Path]; ok && tgtOID != se.BaseOID {
                    // Conflict: write file with markers
                    tgtBytes, _ := store.GetBlob(tgtOID)
                    baseBytes, _ := store.GetBlob(se.BaseOID)
                    conflict := buildConflict(tgtBytes, baseBytes, newBytes)
                    if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
                        return err
                    }
                    if err := os.WriteFile(dst, conflict, 0o644); err != nil {
                        return err
                    }
                    continue
                }
            }
            if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
                return err
            }
            if err := os.WriteFile(dst, newBytes, 0o644); err != nil {
                return err
            }
        }
    }
    return nil
}

func buildConflict(target, base, new []byte) []byte {
    return []byte(fmt.Sprintf(
        "<<<<<<< target\n%s=======\n%s>>>>>>> your_changes\n",
        string(target), string(new),
    ))
}