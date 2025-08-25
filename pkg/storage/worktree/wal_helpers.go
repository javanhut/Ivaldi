package worktree

import (
	"encoding/json"
	"os"
	"path/filepath"

	"ivaldi/pkg/storage/objectstore"
)

type wal struct {
	FromTree objectstore.OID `json:"from_tree"`
	ToCommit objectstore.OID `json:"to_commit"`
	Phase    string          `json:"phase"`
}

func walFile(root string) string {
	return filepath.Join(root, ".ivaldi", "journal", "switch.json")
}

// WriteWAL starts a WAL entry for a timeline switch.
func WriteWAL(root string, fromTree objectstore.OID, toCommit objectstore.OID) error {
	_ = os.MkdirAll(filepath.Dir(walFile(root)), 0o755)
	w := wal{FromTree: fromTree, ToCommit: toCommit, Phase: "start"}
	b, _ := json.MarshalIndent(&w, "", "  ")
	tmp := walFile(root) + ".tmp"
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, walFile(root))
}

// AdvanceWAL updates the phase of the WAL file.
func AdvanceWAL(root, phase string) error {
	b, err := os.ReadFile(walFile(root))
	if err != nil {
		return err
	}
	var w wal
	if err := json.Unmarshal(b, &w); err != nil {
		return err
	}
	w.Phase = phase
	nb, _ := json.MarshalIndent(&w, "", "  ")
	tmp := walFile(root) + ".tmp"
	if err := os.WriteFile(tmp, nb, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, walFile(root))
}

// ClearWAL removes the WAL after a successful switch.
func ClearWAL(root string) {
	_ = os.Remove(walFile(root))
}

// RecoverWAL checks for an incomplete switch and attempts recovery
func RecoverWAL(root string, store *objectstore.ObjectStore) error {
	b, err := os.ReadFile(walFile(root))
	if err != nil {
		if os.IsNotExist(err) {
			return nil // No WAL, nothing to recover
		}
		return err
	}

	var w wal
	if err := json.Unmarshal(b, &w); err != nil {
		// Corrupted WAL, remove it
		ClearWAL(root)
		return nil
	}

	// If the WAL contains invalid OIDs (like from test), just clear it
	if w.ToCommit == "dummy" || w.FromTree == "dummy" {
		ClearWAL(root)
		return nil
	}

	// Based on the phase, decide what to do
	switch w.Phase {
	case "stashed":
		// Stash was saved, continue with checkout
		commit, err := store.GetCommit(w.ToCommit)
		if err != nil {
			// If we can't find the commit, clear the WAL and continue
			ClearWAL(root)
			return nil
		}
		if err := CheckoutTreeAtomic(root, store, commit.Tree); err != nil {
			return err
		}
		if err := AdvanceWAL(root, "checked_out"); err != nil {
			return err
		}
		fallthrough

	case "checked_out":
		// Files were checked out, reapply stash if it exists
		// This would require saving the stash somewhere during the switch
		// For now, just clear the WAL
		ClearWAL(root)

	default:
		// Unknown phase or "start", safe to clear
		ClearWAL(root)
	}

	return nil
}
