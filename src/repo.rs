//! Persistent repository context for Ivaldi VCS.
//!
//! Wires the persistent `Store` to the in-memory engines (MMR, timelines, seals).
//! This is the main entry point for CLI commands that need to read/write
//! commit history that survives across sessions.

use std::path::{Path, PathBuf};

use crate::cas::FileCas;
use crate::config::{self, Config};
use crate::forge::{self, HeadRef};
use crate::hash::B3Hash;
use crate::leaf::{self, Leaf, NO_PARENT};
use crate::mmr::Mmr;
use crate::seal;
use crate::store::{Store, StoreError};

/// A persistent repository backed by redb + file CAS.
pub struct Repo {
    pub work_dir: PathBuf,
    pub ivaldi_dir: PathBuf,
    pub store: Store,
    pub cas: FileCas,
    mmr: Mmr,
}

impl Repo {
    /// Open an existing Ivaldi repository.
    pub fn open(work_dir: &Path) -> Result<Self, RepoError> {
        let ivaldi_dir = work_dir.join(".ivaldi");
        if !ivaldi_dir.join("HEAD").exists() {
            return Err(RepoError::NotARepo);
        }

        let store = Store::open(&ivaldi_dir.join("store.db")).map_err(RepoError::Store)?;
        let cas = FileCas::new(ivaldi_dir.join("objects")).map_err(|e| {
            RepoError::Other(format!("failed to open CAS: {}", e))
        })?;

        // Rebuild in-memory MMR from persisted leaves
        let mut mmr = Mmr::new();
        let count = store.leaf_count().map_err(RepoError::Store)?;
        for idx in 0..count {
            if let Some(data) = store.get_leaf(idx).map_err(RepoError::Store)? {
                let parsed_leaf = leaf::parse_leaf(&data)
                    .map_err(|e| RepoError::Other(format!("corrupt leaf {}: {}", idx, e)))?;
                mmr.append_leaf(parsed_leaf);
            }
        }

        Ok(Self {
            work_dir: work_dir.to_path_buf(),
            ivaldi_dir,
            store,
            cas,
            mmr,
        })
    }

    /// Get the current HEAD timeline name.
    pub fn current_timeline(&self) -> Result<String, RepoError> {
        let head = forge::read_head(&self.ivaldi_dir)
            .map_err(|e| RepoError::Other(e.to_string()))?;
        match head {
            HeadRef::Timeline(name) => Ok(name),
            HeadRef::Detached(hash) => Err(RepoError::Other(format!("HEAD is detached at {}", hash))),
        }
    }

    /// Create a new commit (seal) on the current timeline.
    ///
    /// Persists the leaf to the store, updates MMR, timeline head, and seal name registry.
    pub fn commit(
        &mut self,
        tree_root: B3Hash,
        author: &str,
        message: &str,
    ) -> Result<CommitResult, RepoError> {
        let timeline = self.current_timeline()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Build leaf with parent from current timeline head
        let prev_idx = self
            .store
            .get_timeline_head(&timeline)
            .map_err(RepoError::Store)?
            .unwrap_or(NO_PARENT);

        let mut new_leaf = Leaf::new(tree_root, &timeline, author, now, message);
        new_leaf.prev_idx = prev_idx;

        // Compute hash and seal name
        let leaf_hash = new_leaf.hash();
        let seal_name = seal::generate_seal_name(leaf_hash);

        // Persist leaf canonical bytes
        let canonical = new_leaf.canonical_bytes();
        let idx = self.mmr.size();
        self.store
            .put_leaf(idx, &canonical)
            .map_err(RepoError::Store)?;

        // Append to in-memory MMR
        let (leaf_idx, root) = self.mmr.append_leaf(new_leaf);

        // Update timeline head
        self.store
            .set_timeline_head(&timeline, leaf_idx)
            .map_err(RepoError::Store)?;

        // Store seal name mapping
        self.store
            .put_seal_name(&seal_name, leaf_hash)
            .map_err(RepoError::Store)?;

        // Store MMR size
        self.store
            .set_meta("mmr.size", &self.mmr.size().to_string())
            .map_err(RepoError::Store)?;

        Ok(CommitResult {
            index: leaf_idx,
            hash: leaf_hash,
            seal_name,
            root,
            timeline,
        })
    }

    /// Create a raw commit (seal) with a pre-built Leaf on a specific timeline.
    ///
    /// Unlike `commit()`, this does NOT read the current timeline, timestamp, or
    /// auto-set `prev_idx`. The caller is responsible for setting all Leaf fields.
    /// Used for history import where commit metadata comes from an external source.
    pub fn commit_raw(&mut self, leaf: Leaf, timeline: &str) -> Result<CommitResult, RepoError> {
        // Compute hash and seal name
        let leaf_hash = leaf.hash();
        let seal_name = seal::generate_seal_name(leaf_hash);

        // Persist leaf canonical bytes
        let canonical = leaf.canonical_bytes();
        let idx = self.mmr.size();
        self.store
            .put_leaf(idx, &canonical)
            .map_err(RepoError::Store)?;

        // Append to in-memory MMR
        let (leaf_idx, root) = self.mmr.append_leaf(leaf);

        // Update timeline head
        self.store
            .set_timeline_head(timeline, leaf_idx)
            .map_err(RepoError::Store)?;

        // Store seal name mapping
        self.store
            .put_seal_name(&seal_name, leaf_hash)
            .map_err(RepoError::Store)?;

        // Store MMR size
        self.store
            .set_meta("mmr.size", &self.mmr.size().to_string())
            .map_err(RepoError::Store)?;

        Ok(CommitResult {
            index: leaf_idx,
            hash: leaf_hash,
            seal_name,
            root,
            timeline: timeline.to_string(),
        })
    }

    /// Get a leaf by index.
    pub fn get_leaf(&self, idx: u64) -> Result<Option<Leaf>, RepoError> {
        match self.store.get_leaf(idx).map_err(RepoError::Store)? {
            Some(data) => {
                let parsed = leaf::parse_leaf(&data)
                    .map_err(|e| RepoError::Other(format!("corrupt leaf: {}", e)))?;
                Ok(Some(parsed))
            }
            None => Ok(None),
        }
    }

    /// Get the head leaf index for a timeline.
    pub fn get_timeline_head(&self, name: &str) -> Result<Option<u64>, RepoError> {
        self.store.get_timeline_head(name).map_err(RepoError::Store)
    }

    /// List all timelines with their head indices.
    pub fn list_timelines(&self) -> Result<Vec<(String, u64)>, RepoError> {
        self.store.list_timeline_heads().map_err(RepoError::Store)
    }

    /// Create a new timeline forking from the current one (or a named source).
    pub fn create_timeline(
        &self,
        name: &str,
        source: Option<&str>,
    ) -> Result<(), RepoError> {
        // Check if already exists
        if self.store.get_timeline_head(name).map_err(RepoError::Store)?.is_some() {
            return Err(RepoError::Other(format!("timeline '{}' already exists", name)));
        }

        let source_name = match source {
            Some(s) => s.to_string(),
            None => self.current_timeline()?,
        };

        // Copy head from source
        if let Some(head_idx) = self
            .store
            .get_timeline_head(&source_name)
            .map_err(RepoError::Store)?
        {
            self.store
                .set_timeline_head(name, head_idx)
                .map_err(RepoError::Store)?;
        }

        // Create ref file
        let ref_path = self.ivaldi_dir.join("refs/heads").join(name);
        if let Some(parent) = ref_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&ref_path, "").ok();

        Ok(())
    }

    /// Switch to a different timeline (updates HEAD).
    pub fn switch_timeline(&self, name: &str) -> Result<(), RepoError> {
        // Verify timeline exists
        if self.store.get_timeline_head(name).map_err(RepoError::Store)?.is_none() {
            // Check if ref file exists even without a head (newly created, no commits)
            let ref_path = self.ivaldi_dir.join("refs/heads").join(name);
            if !ref_path.exists() {
                return Err(RepoError::Other(format!("timeline '{}' not found", name)));
            }
        }

        forge::write_head(&self.ivaldi_dir, &HeadRef::Timeline(name.to_string()))
            .map_err(|e| RepoError::Other(e.to_string()))?;
        Ok(())
    }

    /// Remove a timeline.
    pub fn remove_timeline(&self, name: &str) -> Result<(), RepoError> {
        let current = self.current_timeline()?;
        if current == name {
            return Err(RepoError::Other("cannot remove current timeline".into()));
        }

        self.store
            .remove_timeline_head(name)
            .map_err(RepoError::Store)?;

        let ref_path = self.ivaldi_dir.join("refs/heads").join(name);
        let _ = std::fs::remove_file(&ref_path);

        Ok(())
    }

    /// Rename a timeline. If it's the current timeline, HEAD is updated too.
    pub fn rename_timeline(&self, old_name: &str, new_name: &str) -> Result<(), RepoError> {
        if old_name == new_name {
            return Ok(());
        }

        // Check new name doesn't already exist
        if self.store.get_timeline_head(new_name).map_err(RepoError::Store)?.is_some() {
            return Err(RepoError::Other(format!("timeline '{}' already exists", new_name)));
        }

        // Copy head to new name
        let head_idx = self.store.get_timeline_head(old_name).map_err(RepoError::Store)?
            .ok_or_else(|| RepoError::Other(format!("timeline '{}' not found", old_name)))?;
        self.store.set_timeline_head(new_name, head_idx).map_err(RepoError::Store)?;

        // Remove old name
        self.store.remove_timeline_head(old_name).map_err(RepoError::Store)?;

        // Update ref files
        let old_ref = self.ivaldi_dir.join("refs/heads").join(old_name);
        let new_ref = self.ivaldi_dir.join("refs/heads").join(new_name);
        if old_ref.exists() {
            let _ = std::fs::rename(&old_ref, &new_ref);
        } else {
            let _ = std::fs::write(&new_ref, "");
        }

        // Update HEAD if this was the current timeline
        let current = self.current_timeline()?;
        if current == old_name {
            forge::write_head(&self.ivaldi_dir, &HeadRef::Timeline(new_name.to_string()))
                .map_err(|e| RepoError::Other(e.to_string()))?;
        }

        Ok(())
    }

    /// Walk commit history from a timeline head backwards.
    pub fn walk_history(&self, timeline: &str) -> Result<Vec<HistoryEntry>, RepoError> {
        let head_idx = match self.get_timeline_head(timeline)? {
            Some(idx) => idx,
            None => return Ok(Vec::new()),
        };

        let mut entries = Vec::new();
        let mut current = Some(head_idx);

        while let Some(idx) = current {
            let leaf = match self.get_leaf(idx)? {
                Some(l) => l,
                None => break,
            };

            let leaf_hash = leaf.hash();
            entries.push(HistoryEntry {
                index: idx,
                hash: leaf_hash,
                seal_name: seal::generate_seal_name(leaf_hash),
                short_hash: leaf_hash.short8(),
                author: leaf.author.clone(),
                message: leaf.message.clone(),
                time_unix: leaf.time_unix,
                timeline: leaf.timeline_id.clone(),
                is_merge: leaf.is_merge(),
            });

            current = if leaf.has_parent() {
                Some(leaf.prev_idx)
            } else {
                None
            };
        }

        Ok(entries)
    }

    /// Get the seal name for a hash.
    pub fn get_seal_name(&self, hash: B3Hash) -> Result<Option<String>, RepoError> {
        self.store.get_seal_name_by_hash(hash).map_err(RepoError::Store)
    }

    /// Resolve a seal name or hash prefix to a leaf index.
    pub fn resolve_seal(&self, query: &str) -> Result<Option<(u64, Leaf)>, RepoError> {
        // Try seal name prefix match
        let matches = self
            .store
            .find_seal_names_by_prefix(query)
            .map_err(RepoError::Store)?;

        if matches.len() == 1 {
            if let Some(hash) = self
                .store
                .get_hash_by_seal_name(&matches[0])
                .map_err(RepoError::Store)?
            {
                return self.find_leaf_by_hash(hash);
            }
        }

        // Try hash prefix match
        let count = self.mmr.size();
        for idx in 0..count {
            if let Some(leaf) = self.get_leaf(idx)? {
                let h = leaf.hash();
                if h.matches_prefix(query) {
                    return Ok(Some((idx, leaf)));
                }
            }
        }

        Ok(None)
    }

    fn find_leaf_by_hash(&self, hash: B3Hash) -> Result<Option<(u64, Leaf)>, RepoError> {
        let count = self.mmr.size();
        for idx in 0..count {
            if let Some(leaf) = self.get_leaf(idx)? {
                if leaf.hash() == hash {
                    return Ok(Some((idx, leaf)));
                }
            }
        }
        Ok(None)
    }

    /// Get the loaded config.
    pub fn config(&self) -> Config {
        config::load_config(&self.ivaldi_dir)
    }

    /// Number of commits in the repository.
    pub fn commit_count(&self) -> u64 {
        self.mmr.size()
    }

    /// MMR root hash.
    pub fn root(&self) -> B3Hash {
        self.mmr.root()
    }
}

/// Result of a commit operation.
#[derive(Debug)]
pub struct CommitResult {
    pub index: u64,
    pub hash: B3Hash,
    pub seal_name: String,
    pub root: B3Hash,
    pub timeline: String,
}

/// A display-ready history entry.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub index: u64,
    pub hash: B3Hash,
    pub seal_name: String,
    pub short_hash: String,
    pub author: String,
    pub message: String,
    pub time_unix: i64,
    pub timeline: String,
    pub is_merge: bool,
}

/// State of an in-progress merge (fuse).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MergeState {
    pub source_timeline: String,
    pub target_timeline: String,
    pub strategy: String,
    pub conflicts: Vec<String>,
}

impl Repo {
    // -- Merge state management --

    /// Save a merge-in-progress state.
    pub fn save_merge_state(&self, state: &MergeState) -> Result<(), RepoError> {
        let path = self.ivaldi_dir.join("MERGE_STATE");
        let data = serde_json::to_string_pretty(state)
            .map_err(|e| RepoError::Other(e.to_string()))?;
        std::fs::write(&path, data).map_err(|e| RepoError::Other(e.to_string()))?;
        Ok(())
    }

    /// Load merge-in-progress state, if any.
    pub fn load_merge_state(&self) -> Result<Option<MergeState>, RepoError> {
        let path = self.ivaldi_dir.join("MERGE_STATE");
        match std::fs::read_to_string(&path) {
            Ok(data) => {
                let state = serde_json::from_str(&data)
                    .map_err(|e| RepoError::Other(e.to_string()))?;
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::Other(e.to_string())),
        }
    }

    /// Clear merge-in-progress state (after --continue or --abort).
    pub fn clear_merge_state(&self) -> Result<(), RepoError> {
        let path = self.ivaldi_dir.join("MERGE_STATE");
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(RepoError::Other(e.to_string())),
        }
    }

    /// Check if a merge is in progress.
    pub fn has_merge_in_progress(&self) -> bool {
        self.ivaldi_dir.join("MERGE_STATE").exists()
    }

    // -- Review persistence --

    /// Path to the reviews directory.
    pub fn reviews_dir(&self) -> std::path::PathBuf {
        self.ivaldi_dir.join("reviews")
    }

    /// Get the next review ID and increment the counter.
    pub fn next_review_id(&self) -> Result<u64, RepoError> {
        let current = self
            .store
            .get_meta("review.next_id")
            .map_err(RepoError::Store)?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);
        self.store
            .set_meta("review.next_id", &(current + 1).to_string())
            .map_err(RepoError::Store)?;
        Ok(current)
    }

    /// Save a review to its JSON file.
    pub fn save_review(&self, review: &crate::review::Review) -> Result<(), RepoError> {
        let dir = self.reviews_dir();
        std::fs::create_dir_all(&dir).map_err(|e| RepoError::Other(e.to_string()))?;
        let path = dir.join(format!("{}.json", review.id));
        let data = serde_json::to_string_pretty(review)
            .map_err(|e| RepoError::Other(e.to_string()))?;
        std::fs::write(&path, data).map_err(|e| RepoError::Other(e.to_string()))?;
        Ok(())
    }

    /// Load a review by ID.
    pub fn load_review(&self, id: u64) -> Result<Option<crate::review::Review>, RepoError> {
        let path = self.reviews_dir().join(format!("{}.json", id));
        match std::fs::read_to_string(&path) {
            Ok(data) => {
                let review = serde_json::from_str(&data)
                    .map_err(|e| RepoError::Other(e.to_string()))?;
                Ok(Some(review))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::Other(e.to_string())),
        }
    }

    /// List all reviews.
    pub fn list_reviews(&self) -> Result<Vec<crate::review::Review>, RepoError> {
        let dir = self.reviews_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut reviews = Vec::new();
        let entries = std::fs::read_dir(&dir).map_err(|e| RepoError::Other(e.to_string()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let data =
                    std::fs::read_to_string(&path).map_err(|e| RepoError::Other(e.to_string()))?;
                let review: crate::review::Review =
                    serde_json::from_str(&data).map_err(|e| RepoError::Other(e.to_string()))?;
                reviews.push(review);
            }
        }
        Ok(reviews)
    }

    // -- Butterfly sync --

    /// Sync butterfly up: merge butterfly changes into parent timeline.
    /// Uses the fuse engine with auto strategy (fast-forward preferred).
    pub fn butterfly_sync_up(
        &mut self,
        butterfly_name: &str,
    ) -> Result<CommitResult, RepoError> {
        // Get butterfly and parent head trees
        let bf_head = self.get_timeline_head(butterfly_name)?
            .ok_or_else(|| RepoError::Other(format!("butterfly '{}' has no commits", butterfly_name)))?;
        let bf_leaf = self.get_leaf(bf_head)?
            .ok_or_else(|| RepoError::Other("corrupt butterfly head".into()))?;

        // Find parent name from store metadata
        let parent_data = self.store.get_butterfly(butterfly_name)
            .map_err(RepoError::Store)?
            .ok_or_else(|| RepoError::Other(format!("'{}' is not a butterfly", butterfly_name)))?;
        let parent_name: String = serde_json::from_slice::<serde_json::Value>(&parent_data)
            .map_err(|e| RepoError::Other(e.to_string()))?
            .get("parent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RepoError::Other("corrupt butterfly metadata".into()))?
            .to_string();

        // Use butterfly's tree as the merge result (fast-forward)
        let tree_root = bf_leaf.tree_root;
        let author = bf_leaf.author.clone();
        let message = format!("Merged butterfly '{}' into '{}'", butterfly_name, parent_name);

        // Switch to parent and commit
        let prev_current = self.current_timeline()?;
        self.switch_timeline(&parent_name)?;
        let result = self.commit(tree_root, &author, &message)?;
        // Switch back if we were on the butterfly
        if prev_current == butterfly_name {
            self.switch_timeline(butterfly_name)?;
        }

        Ok(result)
    }

    /// Sync butterfly down: merge parent changes into butterfly.
    pub fn butterfly_sync_down(
        &mut self,
        butterfly_name: &str,
    ) -> Result<CommitResult, RepoError> {
        let parent_data = self.store.get_butterfly(butterfly_name)
            .map_err(RepoError::Store)?
            .ok_or_else(|| RepoError::Other(format!("'{}' is not a butterfly", butterfly_name)))?;
        let parent_name: String = serde_json::from_slice::<serde_json::Value>(&parent_data)
            .map_err(|e| RepoError::Other(e.to_string()))?
            .get("parent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RepoError::Other("corrupt butterfly metadata".into()))?
            .to_string();

        let parent_head = self.get_timeline_head(&parent_name)?
            .ok_or_else(|| RepoError::Other(format!("parent '{}' has no commits", parent_name)))?;
        let parent_leaf = self.get_leaf(parent_head)?
            .ok_or_else(|| RepoError::Other("corrupt parent head".into()))?;

        // Use parent's tree as the merge result (fast-forward down)
        let tree_root = parent_leaf.tree_root;
        let author = parent_leaf.author.clone();
        let message = format!("Synced from parent '{}' into '{}'", parent_name, butterfly_name);

        let prev_current = self.current_timeline()?;
        self.switch_timeline(butterfly_name)?;
        let result = self.commit(tree_root, &author, &message)?;
        if prev_current != butterfly_name {
            self.switch_timeline(&prev_current)?;
        }

        Ok(result)
    }

    /// Store butterfly metadata in redb for persistence.
    pub fn store_butterfly_meta(
        &self,
        name: &str,
        parent_name: &str,
        divergence_hash: B3Hash,
    ) -> Result<(), RepoError> {
        let data = serde_json::json!({
            "name": name,
            "parent_name": parent_name,
            "divergence_hash": divergence_hash.to_hex(),
            "is_orphaned": false,
        });
        let bytes = serde_json::to_vec(&data).map_err(|e| RepoError::Other(e.to_string()))?;
        self.store.put_butterfly(name, &bytes).map_err(RepoError::Store)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not an Ivaldi repository")]
    NotARepo,
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge;

    fn setup_repo() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        // Set up config so commits work
        let mut cfg = Config::new();
        cfg.set("user.name", "Test User");
        cfg.set("user.email", "test@ivaldi.dev");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        (dir, repo)
    }

    #[test]
    fn open_repo() {
        let (_dir, repo) = setup_repo();
        assert_eq!(repo.current_timeline().unwrap(), "main");
        assert_eq!(repo.commit_count(), 0);
    }

    #[test]
    fn open_nonexistent_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Repo::open(dir.path()).is_err());
    }

    #[test]
    fn commit_and_read_back() {
        let (_dir, mut repo) = setup_repo();
        let tree = B3Hash::digest(b"tree root 1");

        let result = repo.commit(tree, "Alice <a@b.com>", "First commit").unwrap();

        assert_eq!(result.index, 0);
        assert_eq!(result.timeline, "main");
        assert!(!result.seal_name.is_empty());
        assert_eq!(repo.commit_count(), 1);

        // Read leaf back
        let leaf = repo.get_leaf(0).unwrap().unwrap();
        assert_eq!(leaf.message, "First commit");
        assert_eq!(leaf.timeline_id, "main");
        assert_eq!(leaf.prev_idx, NO_PARENT);
    }

    #[test]
    fn commit_chain() {
        let (_dir, mut repo) = setup_repo();

        let r1 = repo.commit(B3Hash::digest(b"t1"), "A", "Commit 1").unwrap();
        let r2 = repo.commit(B3Hash::digest(b"t2"), "A", "Commit 2").unwrap();
        let r3 = repo.commit(B3Hash::digest(b"t3"), "A", "Commit 3").unwrap();

        assert_eq!(r1.index, 0);
        assert_eq!(r2.index, 1);
        assert_eq!(r3.index, 2);

        // Check chain
        let leaf2 = repo.get_leaf(2).unwrap().unwrap();
        assert_eq!(leaf2.prev_idx, 1);
        let leaf1 = repo.get_leaf(1).unwrap().unwrap();
        assert_eq!(leaf1.prev_idx, 0);
        let leaf0 = repo.get_leaf(0).unwrap().unwrap();
        assert_eq!(leaf0.prev_idx, NO_PARENT);
    }

    #[test]
    fn commits_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        let mut cfg = Config::new();
        cfg.set("user.name", "Tester");
        cfg.set("user.email", "t@t.com");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        // First session: create commits
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"t1"), "A", "First").unwrap();
            repo.commit(B3Hash::digest(b"t2"), "A", "Second").unwrap();
        }

        // Second session: verify persistence
        {
            let repo = Repo::open(dir.path()).unwrap();
            assert_eq!(repo.commit_count(), 2);

            let leaf0 = repo.get_leaf(0).unwrap().unwrap();
            assert_eq!(leaf0.message, "First");

            let leaf1 = repo.get_leaf(1).unwrap().unwrap();
            assert_eq!(leaf1.message, "Second");
            assert_eq!(leaf1.prev_idx, 0);

            assert_eq!(repo.get_timeline_head("main").unwrap(), Some(1));
        }
    }

    #[test]
    fn seal_name_persists() {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        let mut cfg = Config::new();
        cfg.set("user.name", "T");
        cfg.set("user.email", "t@t");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        let seal_name;
        let hash;
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            let result = repo.commit(B3Hash::digest(b"t"), "A", "msg").unwrap();
            seal_name = result.seal_name.clone();
            hash = result.hash;
        }

        {
            let repo = Repo::open(dir.path()).unwrap();
            assert_eq!(repo.get_seal_name(hash).unwrap(), Some(seal_name));
        }
    }

    #[test]
    fn walk_history() {
        let (_dir, mut repo) = setup_repo();

        repo.commit(B3Hash::digest(b"t1"), "A", "First").unwrap();
        repo.commit(B3Hash::digest(b"t2"), "A", "Second").unwrap();
        repo.commit(B3Hash::digest(b"t3"), "A", "Third").unwrap();

        let history = repo.walk_history("main").unwrap();
        assert_eq!(history.len(), 3);
        // Newest first
        assert_eq!(history[0].message, "Third");
        assert_eq!(history[1].message, "Second");
        assert_eq!(history[2].message, "First");

        for entry in &history {
            assert!(!entry.seal_name.is_empty());
            assert_eq!(entry.short_hash.len(), 8);
        }
    }

    #[test]
    fn walk_empty_history() {
        let (_dir, repo) = setup_repo();
        let history = repo.walk_history("main").unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn create_and_switch_timeline() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t1"), "A", "Base").unwrap();

        repo.create_timeline("feature", None).unwrap();
        assert_eq!(repo.get_timeline_head("feature").unwrap(), Some(0));

        repo.switch_timeline("feature").unwrap();
        assert_eq!(repo.current_timeline().unwrap(), "feature");

        // Commit on feature
        repo.commit(B3Hash::digest(b"t2"), "A", "Feature work").unwrap();
        assert_eq!(repo.get_timeline_head("feature").unwrap(), Some(1));
        // Main still at 0
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(0));
    }

    #[test]
    fn list_timelines() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
        repo.create_timeline("alpha", None).unwrap();
        repo.create_timeline("beta", None).unwrap();

        let list = repo.list_timelines().unwrap();
        let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn remove_timeline() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
        repo.create_timeline("feature", None).unwrap();

        repo.remove_timeline("feature").unwrap();
        assert!(repo.get_timeline_head("feature").unwrap().is_none());
    }

    #[test]
    fn cannot_remove_current_timeline() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();

        let result = repo.remove_timeline("main");
        assert!(result.is_err());
    }

    #[test]
    fn create_timeline_auto_switch() {
        // Simulates the CLI behavior: create + switch in sequence
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();

        // Before: on main
        assert_eq!(repo.current_timeline().unwrap(), "main");

        // Create and switch (as the CLI does)
        repo.create_timeline("feature", None).unwrap();
        repo.switch_timeline("feature").unwrap();

        // After: on feature
        assert_eq!(repo.current_timeline().unwrap(), "feature");

        // Feature should have same head as main
        assert_eq!(
            repo.get_timeline_head("feature").unwrap(),
            repo.get_timeline_head("main").unwrap()
        );

        // Commits on feature should not affect main
        repo.commit(B3Hash::digest(b"feat"), "A", "Feature work").unwrap();
        assert_ne!(
            repo.get_timeline_head("feature").unwrap(),
            repo.get_timeline_head("main").unwrap()
        );
    }

    #[test]
    fn create_timeline_from_source_and_switch() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t1"), "A", "main commit 1").unwrap();
        repo.commit(B3Hash::digest(b"t2"), "A", "main commit 2").unwrap();

        // Create from main (which has 2 commits) and switch
        repo.create_timeline("hotfix", Some("main")).unwrap();
        repo.switch_timeline("hotfix").unwrap();

        assert_eq!(repo.current_timeline().unwrap(), "hotfix");
        // Hotfix head should match main's head
        assert_eq!(repo.get_timeline_head("hotfix").unwrap(), Some(1));

        // Commit on hotfix
        repo.commit(B3Hash::digest(b"fix"), "A", "hotfix").unwrap();
        assert_eq!(repo.get_timeline_head("hotfix").unwrap(), Some(2));
        // Main unchanged
        assert_eq!(repo.get_timeline_head("main").unwrap(), Some(1));
    }

    #[test]
    fn create_timeline_persists_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        let mut cfg = Config::new();
        cfg.set("user.name", "T");
        cfg.set("user.email", "t@t");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        // Session 1: create timeline and switch
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
            repo.create_timeline("feature", None).unwrap();
            repo.switch_timeline("feature").unwrap();
            assert_eq!(repo.current_timeline().unwrap(), "feature");
        }

        // Session 2: verify persistence
        {
            let repo = Repo::open(dir.path()).unwrap();
            assert_eq!(repo.current_timeline().unwrap(), "feature");
            assert!(repo.get_timeline_head("feature").unwrap().is_some());
            assert!(repo.get_timeline_head("main").unwrap().is_some());
        }
    }

    #[test]
    fn rename_current_timeline() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();

        assert_eq!(repo.current_timeline().unwrap(), "main");
        let head_before = repo.get_timeline_head("main").unwrap();

        repo.rename_timeline("main", "primary").unwrap();

        // HEAD should now point to "primary"
        assert_eq!(repo.current_timeline().unwrap(), "primary");
        // Same head index
        assert_eq!(repo.get_timeline_head("primary").unwrap(), head_before);
        // Old name gone
        assert!(repo.get_timeline_head("main").unwrap().is_none());
    }

    #[test]
    fn rename_non_current_timeline() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
        repo.create_timeline("feature", None).unwrap();

        repo.rename_timeline("feature", "feat-auth").unwrap();

        // Current unchanged
        assert_eq!(repo.current_timeline().unwrap(), "main");
        // New name exists, old gone
        assert!(repo.get_timeline_head("feat-auth").unwrap().is_some());
        assert!(repo.get_timeline_head("feature").unwrap().is_none());
    }

    #[test]
    fn rename_to_existing_name_fails() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
        repo.create_timeline("feature", None).unwrap();

        let result = repo.rename_timeline("main", "feature");
        assert!(result.is_err());
    }

    #[test]
    fn rename_nonexistent_fails() {
        let (_dir, repo) = setup_repo();
        let result = repo.rename_timeline("nope", "something");
        assert!(result.is_err());
    }

    #[test]
    fn rename_same_name_is_noop() {
        let (_dir, mut repo) = setup_repo();
        repo.commit(B3Hash::digest(b"t"), "A", "init").unwrap();
        repo.rename_timeline("main", "main").unwrap(); // should not error
        assert_eq!(repo.current_timeline().unwrap(), "main");
    }

    #[test]
    fn resolve_seal_by_name_prefix() {
        let (_dir, mut repo) = setup_repo();
        let result = repo.commit(B3Hash::digest(b"t"), "A", "msg").unwrap();

        // Resolve by first word of seal name
        let first_word = result.seal_name.split('-').next().unwrap();
        let resolved = repo.resolve_seal(first_word).unwrap();
        assert!(resolved.is_some());
    }

    #[test]
    fn resolve_seal_by_hash_prefix() {
        let (_dir, mut repo) = setup_repo();
        let result = repo.commit(B3Hash::digest(b"t"), "A", "msg").unwrap();

        let prefix = &result.hash.short(4);
        let resolved = repo.resolve_seal(prefix).unwrap();
        assert!(resolved.is_some());
        let (idx, leaf) = resolved.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(leaf.message, "msg");
    }

    #[test]
    fn divergent_timelines_persist() {
        let dir = tempfile::tempdir().unwrap();
        forge::forge(dir.path()).unwrap();

        let mut cfg = Config::new();
        cfg.set("user.name", "T");
        cfg.set("user.email", "t@t");
        cfg.save(&dir.path().join(".ivaldi/config")).unwrap();

        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"base"), "A", "Base").unwrap();
            repo.create_timeline("feature", None).unwrap();
            repo.switch_timeline("feature").unwrap();
            repo.commit(B3Hash::digest(b"feat"), "A", "Feature").unwrap();
            repo.switch_timeline("main").unwrap();
            repo.commit(B3Hash::digest(b"main2"), "A", "Main2").unwrap();
        }

        {
            let repo = Repo::open(dir.path()).unwrap();
            assert_eq!(repo.commit_count(), 3);
            assert_eq!(repo.get_timeline_head("main").unwrap(), Some(2));
            assert_eq!(repo.get_timeline_head("feature").unwrap(), Some(1));

            let main_hist = repo.walk_history("main").unwrap();
            assert_eq!(main_hist.len(), 2);
            assert_eq!(main_hist[0].message, "Main2");
            assert_eq!(main_hist[1].message, "Base");

            let feat_hist = repo.walk_history("feature").unwrap();
            assert_eq!(feat_hist.len(), 2);
            assert_eq!(feat_hist[0].message, "Feature");
            assert_eq!(feat_hist[1].message, "Base");
        }
    }

    #[test]
    fn commit_raw_preserves_custom_timestamp() {
        let (_dir, mut repo) = setup_repo();
        let tree = B3Hash::digest(b"raw tree");
        let custom_time: i64 = 1600000000;

        let leaf = Leaf::new(tree, "main", "Imported <imp@test>", custom_time, "Historical commit");
        let result = repo.commit_raw(leaf, "main").unwrap();

        let stored = repo.get_leaf(result.index).unwrap().unwrap();
        assert_eq!(stored.time_unix, custom_time);
        assert_eq!(stored.author, "Imported <imp@test>");
        assert_eq!(stored.message, "Historical commit");
        assert_eq!(stored.timeline_id, "main");
    }

    #[test]
    fn commit_raw_with_merge_idxs() {
        let (_dir, mut repo) = setup_repo();

        // Create two parent commits
        let r0 = repo.commit(B3Hash::digest(b"t0"), "A", "Parent 0").unwrap();
        let r1 = repo.commit(B3Hash::digest(b"t1"), "A", "Parent 1").unwrap();

        // Create a raw merge commit referencing both parents
        let mut merge_leaf = Leaf::new(
            B3Hash::digest(b"merged tree"),
            "main",
            "Merger <m@test>",
            1700000000,
            "Merge commit",
        );
        merge_leaf.prev_idx = r1.index;
        merge_leaf.merge_idxs = vec![r0.index];

        let result = repo.commit_raw(merge_leaf, "main").unwrap();

        let stored = repo.get_leaf(result.index).unwrap().unwrap();
        assert!(stored.is_merge());
        assert_eq!(stored.prev_idx, r1.index);
        assert_eq!(stored.merge_idxs, vec![r0.index]);

        // Roundtrip: canonical bytes → parse → same merge_idxs
        let bytes = stored.canonical_bytes();
        let parsed = crate::leaf::parse_leaf(&bytes).unwrap();
        assert_eq!(parsed.merge_idxs, vec![r0.index]);
    }

    #[test]
    fn commit_raw_sequential_ordering() {
        let (_dir, mut repo) = setup_repo();

        let leaf0 = Leaf::new(B3Hash::digest(b"t0"), "main", "A", 1000, "First");
        let r0 = repo.commit_raw(leaf0, "main").unwrap();

        let mut leaf1 = Leaf::new(B3Hash::digest(b"t1"), "main", "A", 2000, "Second");
        leaf1.prev_idx = r0.index;
        let r1 = repo.commit_raw(leaf1, "main").unwrap();

        assert_eq!(r0.index, 0);
        assert_eq!(r1.index, 1);
        assert_eq!(repo.commit_count(), 2);

        // Walk history should show correct chain
        let history = repo.walk_history("main").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].message, "Second");
        assert_eq!(history[1].message, "First");
    }
}
