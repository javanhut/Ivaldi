package timeline

import (
    "encoding/json"
    "errors"
    "os"
    "path/filepath"
    "time"

    "ivaldi/pkg/storage/diff"
    "ivaldi/pkg/storage/objectstore"
    "ivaldi/pkg/storage/snapshot"
    "ivaldi/pkg/storage/worktree"
)

type headsFile struct {
    Current string                     `json:"current"`
    Heads   map[string]objectstore.OID `json:"heads"`
}

func headsPath(repoRoot string) string {
    return filepath.Join(repoRoot, ".ivaldi", "HEADS.json")
}

func readHeads(repoRoot string) (*headsFile, error) {
    b, err := os.ReadFile(headsPath(repoRoot))
    if err != nil {
        if errors.Is(err, os.ErrNotExist) {
            return &headsFile{Current: "main", Heads: map[string]objectstore.OID{}}, nil
        }
        return nil, err
    }
    var h headsFile
    if err := json.Unmarshal(b, &h); err != nil {
        return nil, err
    }
    if h.Heads == nil {
        h.Heads = make(map[string]objectstore.OID)
    }
    if h.Current == "" {
        h.Current = "main"
    }
    return &h, nil
}

func writeHeads(repoRoot string, h *headsFile) error {
    if err := os.MkdirAll(filepath.Join(repoRoot, ".ivaldi"), 0o755); err != nil {
        return err
    }
    b, _ := json.MarshalIndent(h, "", "  ")
    tmp := headsPath(repoRoot) + ".tmp"
    if err := os.WriteFile(tmp, b, 0o644); err != nil {
        return err
    }
    return os.Rename(tmp, headsPath(repoRoot))
}

// Create a new timeline
func Create(repoRoot, name string, store *objectstore.ObjectStore) error {
    h, err := readHeads(repoRoot)
    if err != nil {
        return err
    }
    if _, exists := h.Heads[name]; exists {
        return nil
    }
    baseCommit, ok := h.Heads[h.Current]
    if !ok || baseCommit == "" {
        tree, err := snapshot.WriteTree(repoRoot, store)
        if err != nil {
            return err
        }
        c := &objectstore.Commit{
            Tree:    tree,
            Parents: nil,
            Author:  "ivaldi",
            Message: "initial snapshot",
            TimeUTC: time.Now().UTC().Unix(),
        }
        baseCommit, err = store.PutCommit(c)
        if err != nil {
            return err
        }
        h.Heads[h.Current] = baseCommit
    }
    h.Heads[name] = baseCommit
    return writeHeads(repoRoot, h)
}

// Switch timelines with WAL for crash safety
func Switch(repoRoot, name string, store *objectstore.ObjectStore) error {
    // First, check for any incomplete switches
    if err := worktree.RecoverWAL(repoRoot, store); err != nil {
        return err
    }
    
    h, err := readHeads(repoRoot)
    if err != nil {
        return err
    }
    
    targetCommit, ok := h.Heads[name]
    if !ok {
        return errors.New("timeline does not exist: " + name)
    }
    
    // If switching to the same timeline, do nothing
    if h.Current == name {
        return nil
    }
    
    currentCommit := h.Heads[h.Current]

    var baseTree objectstore.OID
    if currentCommit != "" {
        cm, err := store.GetCommit(currentCommit)
        if err != nil {
            return err
        }
        baseTree = cm.Tree
    } else {
        empty := &objectstore.Tree{}
        baseTree, err = store.PutTree(empty)
        if err != nil {
            return err
        }
    }
    
    // Start WAL for crash recovery
    if err := worktree.WriteWAL(repoRoot, baseTree, targetCommit); err != nil {
        return err
    }

    // Build stash of local changes
    stash, err := diff.BuildStash(repoRoot, store, baseTree)
    if err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }
    
    // Mark stash phase
    if err := worktree.AdvanceWAL(repoRoot, "stashed"); err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }

    // Checkout target tree
    tcm, err := store.GetCommit(targetCommit)
    if err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }
    
    if err := worktree.CheckoutTreeAtomic(repoRoot, store, tcm.Tree); err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }
    
    // Mark checkout phase
    if err := worktree.AdvanceWAL(repoRoot, "checked_out"); err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }

    // Reapply stash
    worktreeStash := &worktree.Stash{
        Entries: make([]worktree.StashEntry, len(stash.Entries)),
    }
    for i, e := range stash.Entries {
        worktreeStash.Entries[i] = worktree.StashEntry{
            Path:    e.Path,
            BaseOID: e.BaseOID,
            NewOID:  e.NewOID,
        }
    }
    
    if err := worktree.ReapplyStash(repoRoot, store, tcm.Tree, worktreeStash); err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }

    // Update HEAD
    h.Current = name
    if err := writeHeads(repoRoot, h); err != nil {
        worktree.ClearWAL(repoRoot)
        return err
    }
    
    // Clear WAL on success
    worktree.ClearWAL(repoRoot)
    return nil
}

// Seal = commit snapshot
func Seal(repoRoot, message string, store *objectstore.ObjectStore) (objectstore.OID, error) {
    h, err := readHeads(repoRoot)
    if err != nil {
        return "", err
    }
    tree, err := snapshot.WriteTree(repoRoot, store)
    if err != nil {
        return "", err
    }
    var parents []objectstore.OID
    if head, ok := h.Heads[h.Current]; ok && head != "" {
        parents = []objectstore.OID{head}
    }
    c := &objectstore.Commit{
        Tree:    tree,
        Parents: parents,
        Author:  "ivaldi",
        Message: message,
        TimeUTC: time.Now().UTC().Unix(),
    }
    commit, err := store.PutCommit(c)
    if err != nil {
        return "", err
    }
    h.Heads[h.Current] = commit
    if err := writeHeads(repoRoot, h); err != nil {
        return "", err
    }
    return commit, nil
}

// List returns all timeline names
func List(repoRoot string) ([]string, error) {
    h, err := readHeads(repoRoot)
    if err != nil {
        return nil, err
    }
    
    var names []string
    for name := range h.Heads {
        names = append(names, name)
    }
    return names, nil
}

// Current returns the current timeline name
func Current(repoRoot string) (string, error) {
    h, err := readHeads(repoRoot)
    if err != nil {
        return "", err
    }
    return h.Current, nil
}