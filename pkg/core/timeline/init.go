package timeline

import (
    "ivaldi/pkg/storage/objectstore"
    "ivaldi/pkg/storage/worktree"
)

// Initialize ensures the timeline system is ready and recovers from any incomplete operations
func Initialize(repoRoot string) error {
    store := objectstore.New(repoRoot)
    
    // Check for and recover from any incomplete timeline switches
    if err := worktree.RecoverWAL(repoRoot, store); err != nil {
        return err
    }
    
    // Ensure the heads file exists
    h, err := readHeads(repoRoot)
    if err != nil {
        return err
    }
    
    // If no timelines exist, create the main timeline
    if len(h.Heads) == 0 {
        if err := Create(repoRoot, "main", store); err != nil {
            return err
        }
    }
    
    return nil
}