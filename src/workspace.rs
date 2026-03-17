//! Workspace management for Ivaldi VCS.
//!
//! Handles:
//! - Scanning the working directory for file states
//! - Staging area (gather/reset)
//! - Workspace materialization (applying tree state to disk)
//! - File state tracking (untracked, modified, staged, ignored)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cas::{Cas, CasError};
#[cfg(test)]
use crate::fsmerkle::{Entry, MODE_DIR, MODE_FILE};
use crate::fsmerkle::{self, BlobNode, FsStore, NodeKind};
use crate::hash::B3Hash;
use crate::ignore::PatternCache;

/// File state in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// New file not yet tracked.
    Untracked,
    /// File matches the last seal.
    Unmodified,
    /// File changed since last seal but not staged.
    Modified,
    /// File staged for the next seal.
    Staged,
    /// File marked for deletion.
    Deleted,
}

/// A file entry in the workspace with its state.
#[derive(Debug, Clone)]
pub struct WorkspaceFile {
    pub path: String,
    pub state: FileState,
    pub hash: Option<B3Hash>,
}

/// The staging area tracks files gathered for the next seal.
#[derive(Debug, Clone, Default)]
pub struct StagingArea {
    /// Files staged for the next seal: path → content hash.
    staged: BTreeMap<String, B3Hash>,
}

impl StagingArea {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a file by path and content hash.
    pub fn stage(&mut self, path: impl Into<String>, hash: B3Hash) {
        self.staged.insert(path.into(), hash);
    }

    /// Unstage a specific file.
    pub fn unstage(&mut self, path: &str) -> bool {
        self.staged.remove(path).is_some()
    }

    /// Unstage all files.
    pub fn clear(&mut self) {
        self.staged.clear();
    }

    /// Check if a file is staged.
    pub fn is_staged(&self, path: &str) -> bool {
        self.staged.contains_key(path)
    }

    /// Get all staged files.
    pub fn staged_files(&self) -> &BTreeMap<String, B3Hash> {
        &self.staged
    }

    /// Number of staged files.
    pub fn len(&self) -> usize {
        self.staged.len()
    }

    /// Check if the staging area is empty.
    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    /// Save staging area to disk.
    pub fn save(&self, ivaldi_dir: &Path) -> Result<(), std::io::Error> {
        let stage_dir = ivaldi_dir.join("stage");
        fs::create_dir_all(&stage_dir)?;

        let stage_file = stage_dir.join("files");
        let mut file = fs::File::create(&stage_file)?;

        for (path, hash) in &self.staged {
            writeln!(file, "{} {}", hash, path)?;
        }

        Ok(())
    }

    /// Load staging area from disk.
    pub fn load(ivaldi_dir: &Path) -> Self {
        let stage_file = ivaldi_dir.join("stage").join("files");
        let content = match fs::read_to_string(&stage_file) {
            Ok(c) => c,
            Err(_) => return Self::new(),
        };

        let mut staging = Self::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((hash_str, path)) = line.split_once(' ') {
                if let Some(hash) = B3Hash::from_hex(hash_str) {
                    staging.stage(path, hash);
                }
            }
        }
        staging
    }
}

/// Workspace scanner and manager.
pub struct Workspace<'a> {
    cas: &'a dyn Cas,
    work_dir: PathBuf,
    ivaldi_dir: PathBuf,
    pub staging: StagingArea,
}

impl<'a> Workspace<'a> {
    pub fn new(cas: &'a dyn Cas, work_dir: impl AsRef<Path>, ivaldi_dir: impl AsRef<Path>) -> Self {
        let ivaldi_dir = ivaldi_dir.as_ref().to_path_buf();
        Self {
            cas,
            work_dir: work_dir.as_ref().to_path_buf(),
            ivaldi_dir: ivaldi_dir.clone(),
            staging: StagingArea::load(&ivaldi_dir),
        }
    }

    /// Scan the working directory and return all file paths (relative),
    /// respecting ignore patterns. Skips `.ivaldi/` directory.
    pub fn scan(&self, ignore: &PatternCache) -> Result<Vec<String>, WorkspaceError> {
        let mut files = Vec::new();
        self.scan_dir(&self.work_dir, "", ignore, &mut files)?;
        files.sort();
        Ok(files)
    }

    fn scan_dir(
        &self,
        dir: &Path,
        prefix: &str,
        ignore: &PatternCache,
        files: &mut Vec<String>,
    ) -> Result<(), WorkspaceError> {
        let entries = fs::read_dir(dir).map_err(WorkspaceError::Io)?;

        for entry in entries {
            let entry = entry.map_err(WorkspaceError::Io)?;
            let name = entry.file_name().to_string_lossy().to_string();

            let rel_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };

            // Skip .ivaldi directory
            if rel_path == ".ivaldi" || rel_path.starts_with(".ivaldi/") {
                continue;
            }

            let file_type = entry.file_type().map_err(WorkspaceError::Io)?;

            if file_type.is_dir() {
                if ignore.is_dir_ignored(&rel_path) {
                    continue;
                }
                self.scan_dir(&entry.path(), &rel_path, ignore, files)?;
            } else if file_type.is_file() {
                if !ignore.is_ignored(&rel_path) {
                    files.push(rel_path);
                }
            }
        }

        Ok(())
    }

    /// Gather (stage) files for the next seal.
    /// Reads file content, stores in CAS, and adds to staging area.
    pub fn gather(&mut self, paths: &[&str]) -> Result<Vec<String>, WorkspaceError> {
        let mut gathered = Vec::new();

        for &path in paths {
            let full_path = self.work_dir.join(path);
            if !full_path.exists() {
                continue;
            }

            let content = fs::read(&full_path).map_err(WorkspaceError::Io)?;
            let canonical = BlobNode::canonical_bytes(&content);
            let hash = B3Hash::digest(&canonical);
            self.cas.put(hash, &canonical).map_err(WorkspaceError::Cas)?;

            self.staging.stage(path, hash);
            gathered.push(path.to_string());
        }

        Ok(gathered)
    }

    /// Gather all files in the workspace (respecting ignore patterns).
    pub fn gather_all(&mut self, ignore: &PatternCache) -> Result<Vec<String>, WorkspaceError> {
        let files = self.scan(ignore)?;
        let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        self.gather(&refs)
    }

    /// Build a tree from currently staged files and return the root hash.
    pub fn build_staged_tree(&self) -> Result<B3Hash, WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut file_map = BTreeMap::new();

        for (path, hash) in self.staging.staged_files() {
            // Load blob content from CAS
            let (_, content) = store.load_blob(*hash).map_err(WorkspaceError::FsMerkle)?;
            file_map.insert(path.clone(), content);
        }

        store
            .build_tree_from_map(&file_map)
            .map_err(WorkspaceError::FsMerkle)
    }

    /// Compute workspace status by comparing working directory against last seal tree.
    pub fn status(
        &self,
        last_tree: Option<B3Hash>,
        ignore: &PatternCache,
    ) -> Result<Vec<WorkspaceFile>, WorkspaceError> {
        let disk_files = self.scan(ignore)?;
        let mut result = Vec::new();

        // Build set of known files from last seal
        let mut known_files: BTreeMap<String, B3Hash> = BTreeMap::new();
        if let Some(tree_hash) = last_tree {
            if tree_hash != B3Hash::ZERO {
                let store = FsStore::new(self.cas);
                self.collect_tree_files(&store, tree_hash, "", &mut known_files)?;
            }
        }

        let disk_set: BTreeSet<&str> = disk_files.iter().map(|s| s.as_str()).collect();

        // Check each file on disk
        for path in &disk_files {
            let full_path = self.work_dir.join(path);
            let content = fs::read(&full_path).map_err(WorkspaceError::Io)?;
            let current_hash = BlobNode::hash_content(&content);

            let state = if self.staging.is_staged(path) {
                FileState::Staged
            } else if let Some(known_hash) = known_files.get(path.as_str()) {
                if *known_hash == current_hash {
                    FileState::Unmodified
                } else {
                    FileState::Modified
                }
            } else {
                FileState::Untracked
            };

            result.push(WorkspaceFile {
                path: path.clone(),
                state,
                hash: Some(current_hash),
            });
        }

        // Check for deleted files (in last seal but not on disk)
        for (path, hash) in &known_files {
            if !disk_set.contains(path.as_str()) {
                result.push(WorkspaceFile {
                    path: path.clone(),
                    state: FileState::Deleted,
                    hash: Some(*hash),
                });
            }
        }

        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    /// Materialize a tree hash to the working directory.
    /// Only modifies files that differ from current state.
    pub fn materialize(&self, tree_hash: B3Hash) -> Result<(), WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut target_files = BTreeMap::new();
        self.collect_tree_files(&store, tree_hash, "", &mut target_files)?;

        // Collect current files
        let ignore = PatternCache::new(&[]);
        let current_files = self.scan(&ignore).unwrap_or_default();
        let current_set: BTreeSet<String> = current_files.into_iter().collect();

        // Write/update files
        for (path, blob_hash) in &target_files {
            let full_path = self.work_dir.join(path);
            let (_, content) = store.load_blob(*blob_hash).map_err(WorkspaceError::FsMerkle)?;

            // Only write if different
            let should_write = if full_path.exists() {
                let existing = fs::read(&full_path).map_err(WorkspaceError::Io)?;
                existing != content
            } else {
                true
            };

            if should_write {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent).map_err(WorkspaceError::Io)?;
                }
                fs::write(&full_path, &content).map_err(WorkspaceError::Io)?;
            }
        }

        // Remove files not in target tree
        let target_set: BTreeSet<&str> = target_files.keys().map(|s| s.as_str()).collect();
        for path in &current_set {
            if !target_set.contains(path.as_str()) {
                let full_path = self.work_dir.join(path);
                let _ = fs::remove_file(&full_path);
            }
        }

        Ok(())
    }

    /// Collect all blob file paths and hashes from a tree recursively.
    fn collect_tree_files(
        &self,
        store: &FsStore<'_>,
        tree_hash: B3Hash,
        prefix: &str,
        files: &mut BTreeMap<String, B3Hash>,
    ) -> Result<(), WorkspaceError> {
        let tree = store.load_tree(tree_hash).map_err(WorkspaceError::FsMerkle)?;

        for entry in &tree.entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };

            match entry.kind {
                NodeKind::Blob => {
                    files.insert(path, entry.hash);
                }
                NodeKind::Tree => {
                    self.collect_tree_files(store, entry.hash, &path, files)?;
                }
            }
        }

        Ok(())
    }

    /// Save workspace state to disk.
    pub fn save(&self) -> Result<(), WorkspaceError> {
        self.staging.save(&self.ivaldi_dir).map_err(WorkspaceError::Io)
    }
}

/// Workspace errors.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CAS error: {0}")]
    Cas(#[from] CasError),
    #[error("filesystem merkle error: {0}")]
    FsMerkle(#[from] fsmerkle::FsMerkleError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::MemoryCas;
    use crate::ignore::PatternCache;

    fn setup_workspace() -> (tempfile::TempDir, MemoryCas) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".ivaldi")).unwrap();
        (dir, MemoryCas::new())
    }

    #[test]
    fn staging_area_basic() {
        let mut staging = StagingArea::new();
        assert!(staging.is_empty());

        let hash = B3Hash::digest(b"content");
        staging.stage("file.txt", hash);
        assert_eq!(staging.len(), 1);
        assert!(staging.is_staged("file.txt"));
        assert!(!staging.is_staged("other.txt"));

        staging.unstage("file.txt");
        assert!(staging.is_empty());
    }

    #[test]
    fn staging_area_clear() {
        let mut staging = StagingArea::new();
        staging.stage("a.txt", B3Hash::digest(b"a"));
        staging.stage("b.txt", B3Hash::digest(b"b"));
        assert_eq!(staging.len(), 2);

        staging.clear();
        assert!(staging.is_empty());
    }

    #[test]
    fn staging_area_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut staging = StagingArea::new();
        staging.stage("file.txt", B3Hash::digest(b"content"));
        staging.stage("src/main.rs", B3Hash::digest(b"fn main()"));
        staging.save(&ivaldi_dir).unwrap();

        let loaded = StagingArea::load(&ivaldi_dir);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.is_staged("file.txt"));
        assert!(loaded.is_staged("src/main.rs"));
    }

    #[test]
    fn scan_workspace() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main()").unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let files = ws.scan(&ignore).unwrap();

        assert!(files.contains(&"file.txt".to_string()));
        assert!(files.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn scan_skips_ivaldi_dir() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        fs::write(dir.path().join(".ivaldi/config"), "data").unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let files = ws.scan(&ignore).unwrap();

        assert!(files.contains(&"file.txt".to_string()));
        assert!(!files.iter().any(|f| f.starts_with(".ivaldi")));
    }

    #[test]
    fn scan_respects_ignore() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        fs::write(dir.path().join("debug.log"), "log data").unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&["*.log"]);
        let files = ws.scan(&ignore).unwrap();

        assert!(files.contains(&"file.txt".to_string()));
        assert!(!files.contains(&"debug.log".to_string()));
    }

    #[test]
    fn gather_files() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let gathered = ws.gather(&["file.txt"]).unwrap();

        assert_eq!(gathered, vec!["file.txt"]);
        assert!(ws.staging.is_staged("file.txt"));
        assert_eq!(cas.len(), 1); // Content stored in CAS
    }

    #[test]
    fn gather_all() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let gathered = ws.gather_all(&ignore).unwrap();

        assert_eq!(gathered.len(), 2);
        assert_eq!(ws.staging.len(), 2);
    }

    #[test]
    fn build_staged_tree() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main()").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["file.txt", "src/main.rs"]).unwrap();

        let tree_hash = ws.build_staged_tree().unwrap();
        assert_ne!(tree_hash, B3Hash::ZERO);

        // Verify tree structure
        let store = FsStore::new(&cas);
        let tree = store.load_tree(tree_hash).unwrap();
        assert_eq!(tree.entries.len(), 2); // file.txt + src/
    }

    #[test]
    fn status_untracked() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("new.txt"), "new file").unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let status = ws.status(None, &ignore).unwrap();

        assert_eq!(status.len(), 1);
        assert_eq!(status[0].path, "new.txt");
        assert_eq!(status[0].state, FileState::Untracked);
    }

    #[test]
    fn status_modified() {
        let (dir, cas) = setup_workspace();

        // Create initial tree with file
        fs::write(dir.path().join("file.txt"), "original").unwrap();
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        ws.gather_all(&ignore).unwrap();
        let tree_hash = ws.build_staged_tree().unwrap();
        ws.staging.clear();

        // Modify the file
        fs::write(dir.path().join("file.txt"), "modified").unwrap();

        let status = ws.status(Some(tree_hash), &ignore).unwrap();
        let file_status = status.iter().find(|f| f.path == "file.txt").unwrap();
        assert_eq!(file_status.state, FileState::Modified);
    }

    #[test]
    fn status_unmodified() {
        let (dir, cas) = setup_workspace();

        fs::write(dir.path().join("file.txt"), "content").unwrap();
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        ws.gather_all(&ignore).unwrap();
        let tree_hash = ws.build_staged_tree().unwrap();
        ws.staging.clear();

        // File unchanged
        let status = ws.status(Some(tree_hash), &ignore).unwrap();
        let file_status = status.iter().find(|f| f.path == "file.txt").unwrap();
        assert_eq!(file_status.state, FileState::Unmodified);
    }

    #[test]
    fn status_deleted() {
        let (dir, cas) = setup_workspace();

        fs::write(dir.path().join("file.txt"), "content").unwrap();
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        ws.gather_all(&ignore).unwrap();
        let tree_hash = ws.build_staged_tree().unwrap();

        // Delete the file
        fs::remove_file(dir.path().join("file.txt")).unwrap();

        let ws2 = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let status = ws2.status(Some(tree_hash), &ignore).unwrap();
        let file_status = status.iter().find(|f| f.path == "file.txt").unwrap();
        assert_eq!(file_status.state, FileState::Deleted);
    }

    #[test]
    fn status_staged() {
        let (dir, cas) = setup_workspace();

        fs::write(dir.path().join("file.txt"), "content").unwrap();
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["file.txt"]).unwrap();

        let ignore = PatternCache::new(&[]);
        let status = ws.status(None, &ignore).unwrap();
        let file_status = status.iter().find(|f| f.path == "file.txt").unwrap();
        assert_eq!(file_status.state, FileState::Staged);
    }

    #[test]
    fn materialize_tree() {
        let (dir, cas) = setup_workspace();

        // Build a tree with files
        let store = FsStore::new(&cas);
        let (h1, _) = store.put_blob(b"hello world").unwrap();
        let (h2, _) = store.put_blob(b"fn main() {}").unwrap();

        let sub_tree = store
            .put_tree(vec![Entry {
                name: "main.rs".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: h2,
            }])
            .unwrap();

        let root = store
            .put_tree(vec![
                Entry {
                    name: "README.md".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: h1,
                },
                Entry {
                    name: "src".into(),
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: sub_tree,
                },
            ])
            .unwrap();

        // Materialize to workspace
        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.materialize(root).unwrap();

        // Verify files exist with correct content
        assert_eq!(
            fs::read_to_string(dir.path().join("README.md")).unwrap(),
            "hello world"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
    }

    #[test]
    fn materialize_removes_extra_files() {
        let (dir, cas) = setup_workspace();

        // Create an extra file
        fs::write(dir.path().join("extra.txt"), "should be removed").unwrap();

        // Build a tree without extra.txt
        let store = FsStore::new(&cas);
        let (h, _) = store.put_blob(b"keep me").unwrap();
        let root = store
            .put_tree(vec![Entry {
                name: "keep.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: h,
            }])
            .unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.materialize(root).unwrap();

        assert!(dir.path().join("keep.txt").exists());
        assert!(!dir.path().join("extra.txt").exists());
    }
}
