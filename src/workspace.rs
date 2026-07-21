//! Workspace management for Ivaldi VCS.
//!
//! Handles:
//! - Scanning the working directory for file states
//! - Staging area (gather/reset)
//! - Workspace materialization (applying tree state to disk)
//! - File state tracking (untracked, modified, staged, ignored)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cas::{Cas, CasError};
use crate::fsmerkle::{self, BlobNode, FsStore, NodeKind};
#[cfg(test)]
use crate::fsmerkle::{Entry, MODE_DIR, MODE_FILE};
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
///
/// A staged change is either an addition/modification (a `path → blob hash`
/// entry in `staged`) or a deletion (a `path` in `deletions`). The two sets
/// are mutually exclusive — staging a path as one removes it from the other
/// — so the staging area unambiguously describes the next seal's diff
/// against the parent tree.
#[derive(Debug, Clone, Default)]
pub struct StagingArea {
    /// Files staged for addition or modification: path → content hash.
    staged: BTreeMap<String, B3Hash>,
    /// Paths staged for deletion in the next seal.
    deletions: BTreeSet<String>,
}

impl StagingArea {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a file by path and content hash. Cancels any pending deletion
    /// for the same path.
    pub fn stage(&mut self, path: impl Into<String>, hash: B3Hash) {
        let p = path.into();
        self.deletions.remove(&p);
        self.staged.insert(p, hash);
    }

    /// Stage a path for deletion in the next seal. Cancels any pending
    /// addition for the same path.
    pub fn stage_deletion(&mut self, path: impl Into<String>) {
        let p = path.into();
        self.staged.remove(&p);
        self.deletions.insert(p);
    }

    /// Unstage a specific path (whether it was staged for addition or
    /// deletion). Returns true if anything was actually removed.
    pub fn unstage(&mut self, path: &str) -> bool {
        let was_staged = self.staged.remove(path).is_some();
        let was_deletion = self.deletions.remove(path);
        was_staged || was_deletion
    }

    /// Clear all staged additions and deletions.
    pub fn clear(&mut self) {
        self.staged.clear();
        self.deletions.clear();
    }

    /// Check if a file is staged for addition.
    pub fn is_staged(&self, path: &str) -> bool {
        self.staged.contains_key(path)
    }

    /// Check if a path is staged for deletion.
    pub fn is_staged_for_deletion(&self, path: &str) -> bool {
        self.deletions.contains(path)
    }

    /// Get all files staged for addition.
    pub fn staged_files(&self) -> &BTreeMap<String, B3Hash> {
        &self.staged
    }

    /// Get all paths staged for deletion.
    pub fn staged_deletions(&self) -> &BTreeSet<String> {
        &self.deletions
    }

    /// Number of staged entries (additions + deletions).
    pub fn len(&self) -> usize {
        self.staged.len() + self.deletions.len()
    }

    /// Check if the staging area is empty.
    pub fn is_empty(&self) -> bool {
        self.staged.is_empty() && self.deletions.is_empty()
    }

    /// Save staging area to disk.
    ///
    /// Format is line-oriented and human-readable:
    /// - `<hash> <path>` for additions/modifications (existing format)
    /// - `del <path>` for deletions (new)
    pub fn save(&self, ivaldi_dir: &Path) -> Result<(), std::io::Error> {
        let stage_dir = ivaldi_dir.join("stage");
        fs::create_dir_all(&stage_dir)?;

        let stage_file = stage_dir.join("files");
        let mut content = String::new();

        for (path, hash) in &self.staged {
            content.push_str(&format!("{} {}\n", hash, path));
        }
        for path in &self.deletions {
            content.push_str(&format!("del {}\n", path));
        }

        crate::atomic_io::atomic_write(&stage_file, content.as_bytes())
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
            if let Some(rest) = line.strip_prefix("del ") {
                staging.stage_deletion(rest);
            } else if let Some((hash_str, path)) = line.split_once(' ')
                && let Some(hash) = B3Hash::from_hex(hash_str)
            {
                staging.stage(path, hash);
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
    pub skipped: SkipSet,
}

impl<'a> Workspace<'a> {
    pub fn new(cas: &'a dyn Cas, work_dir: impl AsRef<Path>, ivaldi_dir: impl AsRef<Path>) -> Self {
        let ivaldi_dir = ivaldi_dir.as_ref().to_path_buf();
        Self {
            cas,
            work_dir: work_dir.as_ref().to_path_buf(),
            ivaldi_dir: ivaldi_dir.clone(),
            staging: StagingArea::load(&ivaldi_dir),
            skipped: SkipSet::load(&ivaldi_dir),
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
            } else if file_type.is_file() && !ignore.is_ignored(&rel_path) {
                files.push(rel_path);
            }
        }

        Ok(())
    }

    /// Gather (stage) files for the next seal.
    /// Reads file content, stores in CAS, and adds to staging area.
    ///
    /// Dotfiles require explicit confirmation via the `DotfileAllowlist`.
    /// Unconfirmed dotfiles are returned in `GatherResult::needs_confirmation`
    /// rather than being staged. Security-blocked files (`.env`, `.venv`)
    /// are always rejected with an error.
    pub fn gather(
        &mut self,
        paths: &[&str],
        allowlist: &DotfileAllowlist,
    ) -> Result<GatherResult, WorkspaceError> {
        self.gather_with_progress(paths, allowlist, &mut |_| {})
    }

    /// Like [`Workspace::gather`], but invokes `on` with each gathered path
    /// right after its content is stored in the CAS. Used by the CLI to drive
    /// a progress bar during hashing.
    pub fn gather_with_progress(
        &mut self,
        paths: &[&str],
        allowlist: &DotfileAllowlist,
        on: &mut dyn FnMut(&str),
    ) -> Result<GatherResult, WorkspaceError> {
        let mut gathered = Vec::new();
        let mut needs_confirmation = Vec::new();
        let mut skipped = Vec::new();
        // Loaded lazily the first time a directory argument needs expanding.
        let mut ignore: Option<PatternCache> = None;

        for &path in paths {
            // Hard block: security-pattern files can never be staged
            if crate::ignore::is_security_blocked(path) {
                return Err(WorkspaceError::SecurityBlocked(path.to_string()));
            }

            // Paths marked with `ivaldi skip` are never staged, even when
            // named explicitly; the caller reports them as a warning.
            if self.skipped.covers(path) {
                skipped.push(path.to_string());
                continue;
            }

            let full_path = self.work_dir.join(path);
            if !full_path.exists() {
                continue;
            }

            // Dotfiles and dot-directories need explicit confirmation unless
            // already allowed. The trailing slash a user may type on a
            // directory is trimmed before extracting the basename.
            let basename = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(path);
            if basename.starts_with('.')
                && basename != ".ivaldiignore"
                && !allowlist.is_allowed(path)
            {
                needs_confirmation.push(path.to_string());
                continue;
            }

            // Directory arguments expand into the (non-ignored) files beneath
            // them, mirroring `gather .`. Without this, the fs::read in
            // stage_path would fail with "Is a directory (os error 21)".
            if full_path.is_dir() {
                let cache = &*ignore
                    .get_or_insert_with(|| crate::ignore::load_pattern_cache(&self.work_dir));
                for rel in self.expand_dir(path, cache)? {
                    if self.skipped.covers(&rel) {
                        continue;
                    }
                    self.stage_path(&rel, on)?;
                    gathered.push(rel);
                }
                continue;
            }

            self.stage_path(path, on)?;
            gathered.push(path.to_string());
        }

        Ok(GatherResult {
            gathered,
            needs_confirmation,
            skipped,
        })
    }

    /// Expand a directory argument into the workspace-relative paths of the
    /// non-ignored files beneath it, sorted. Mirrors `scan`'s traversal so
    /// ignore rules and dotfile exclusion apply exactly as for `gather .`.
    /// Returns an empty vec when the directory itself is ignored.
    fn expand_dir(
        &self,
        rel_dir: &str,
        cache: &PatternCache,
    ) -> Result<Vec<String>, WorkspaceError> {
        let rel_dir = rel_dir.trim_end_matches('/');
        if cache.is_dir_ignored(rel_dir) {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        self.scan_dir(&self.work_dir.join(rel_dir), rel_dir, cache, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// Read a single file at workspace-relative `rel`, store its canonical
    /// blob in the CAS, and record it in the staging area. `on` is invoked
    /// with `rel` right after the blob is stored (drives progress bars).
    fn stage_path(&mut self, rel: &str, on: &mut dyn FnMut(&str)) -> Result<(), WorkspaceError> {
        let full_path = self.work_dir.join(rel);
        let content = fs::read(&full_path).map_err(WorkspaceError::Io)?;
        let canonical = BlobNode::canonical_bytes(&content);
        let hash = B3Hash::digest(&canonical);
        self.cas
            .put(hash, &canonical)
            .map_err(WorkspaceError::Cas)?;
        on(rel);
        self.staging.stage(rel, hash);
        Ok(())
    }

    /// Stage specific files unconditionally (used after user confirms dotfiles).
    /// Still rejects security-blocked files.
    pub fn gather_confirmed(&mut self, paths: &[&str]) -> Result<Vec<String>, WorkspaceError> {
        let mut gathered = Vec::new();
        // Loaded lazily the first time a confirmed directory needs expanding.
        let mut ignore: Option<PatternCache> = None;
        for &path in paths {
            if crate::ignore::is_security_blocked(path) {
                return Err(WorkspaceError::SecurityBlocked(path.to_string()));
            }

            // `ivaldi skip` wins over a dotfile confirmation.
            if self.skipped.covers(path) {
                continue;
            }

            let full_path = self.work_dir.join(path);
            if !full_path.exists() {
                continue;
            }

            // A confirmed directory (e.g. a dot-directory the user approved at
            // the prompt) expands the same way as in `gather_with_progress`.
            if full_path.is_dir() {
                let cache = &*ignore
                    .get_or_insert_with(|| crate::ignore::load_pattern_cache(&self.work_dir));
                for rel in self.expand_dir(path, cache)? {
                    if self.skipped.covers(&rel) {
                        continue;
                    }
                    self.stage_path(&rel, &mut |_| {})?;
                    gathered.push(rel);
                }
                continue;
            }

            self.stage_path(path, &mut |_| {})?;
            gathered.push(path.to_string());
        }
        Ok(gathered)
    }

    /// Gather all files in the workspace (respecting ignore patterns).
    /// Dotfiles are auto-excluded by the ignore cache during scan.
    /// Returns a `GatherResult` with skipped dotfiles in `needs_confirmation`
    /// so the caller can report them to the user.
    pub fn gather_all(&mut self, ignore: &PatternCache) -> Result<GatherResult, WorkspaceError> {
        self.gather_all_with_progress(ignore, &mut |_| {})
    }

    /// Like [`Workspace::gather_all`], but invokes `on` with each gathered
    /// path right after its content is stored in the CAS.
    pub fn gather_all_with_progress(
        &mut self,
        ignore: &PatternCache,
        on: &mut dyn FnMut(&str),
    ) -> Result<GatherResult, WorkspaceError> {
        let files: Vec<String> = self
            .scan(ignore)?
            .into_iter()
            .filter(|path| !self.skipped.covers(path))
            .collect();
        // scan() already excludes dotfiles via is_ignored(), so no allowlist needed
        let allowlist = DotfileAllowlist::load(&self.ivaldi_dir);
        let result = self.gather_with_progress(
            &files.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &allowlist,
            on,
        )?;

        // Discover dotfiles that were skipped so the caller can report them
        let skipped_dotfiles = self.find_dotfiles(ignore)?;

        Ok(GatherResult {
            gathered: result.gathered,
            needs_confirmation: skipped_dotfiles,
            skipped: result.skipped,
        })
    }

    /// Walk the workspace and return dotfile paths that exist on disk but were
    /// excluded from `scan()`. Skips `.ivaldi/`, `.ivaldiignore`, security-blocked
    /// files, and ignored directories.
    pub fn find_dotfiles(&self, ignore: &PatternCache) -> Result<Vec<String>, WorkspaceError> {
        let mut dotfiles = Vec::new();
        self.find_dotfiles_in(&self.work_dir, "", ignore, &mut dotfiles)?;
        dotfiles.sort();
        Ok(dotfiles)
    }

    fn find_dotfiles_in(
        &self,
        dir: &Path,
        prefix: &str,
        ignore: &PatternCache,
        dotfiles: &mut Vec<String>,
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
                self.find_dotfiles_in(&entry.path(), &rel_path, ignore, dotfiles)?;
            } else if file_type.is_file() {
                let basename = name.as_str();
                // Only collect dotfiles that aren't .ivaldiignore and aren't security-blocked
                if basename.starts_with('.')
                    && basename != ".ivaldiignore"
                    && !crate::ignore::is_security_blocked(&rel_path)
                {
                    dotfiles.push(rel_path);
                }
            }
        }

        Ok(())
    }

    /// Build a tree from currently staged files and return the root hash.
    ///
    /// Note: this builds a tree from the staging area in isolation, with no
    /// notion of a parent commit. Callers that want the tree of the *next
    /// seal* (= parent tree + staged additions − staged deletions) must use
    /// [`Self::build_seal_tree`] instead. This method remains useful for
    /// status previews and for tests that work in isolation.
    pub fn build_staged_tree(&self) -> Result<B3Hash, WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut file_map = BTreeMap::new();

        for (path, hash) in self.staging.staged_files() {
            let (_, content) = store.load_blob(*hash).map_err(WorkspaceError::FsMerkle)?;
            file_map.insert(path.clone(), content);
        }

        store
            .build_tree_from_map(&file_map)
            .map_err(WorkspaceError::FsMerkle)
    }

    /// Build the tree for the next seal: parent tree + staged additions
    /// minus staged deletions.
    ///
    /// Pass `None` for the very first seal in a brand-new repository. For
    /// every other seal, the parent tree must be supplied so that files not
    /// touched by the current staging area are inherited from the parent
    /// rather than silently dropped.
    pub fn build_seal_tree(&self, parent_tree: Option<B3Hash>) -> Result<B3Hash, WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut file_map: BTreeMap<String, B3Hash> = BTreeMap::new();

        if let Some(parent_hash) = parent_tree
            && parent_hash != B3Hash::ZERO
        {
            self.collect_tree_files(&store, parent_hash, "", &mut file_map)?;
        }

        for path in self.staging.staged_deletions() {
            file_map.remove(path);
        }
        for (path, hash) in self.staging.staged_files() {
            file_map.insert(path.clone(), *hash);
        }

        store
            .build_tree_from_hash_map(&file_map)
            .map_err(WorkspaceError::FsMerkle)
    }

    /// List all blob paths in a stored tree as a `path → hash` map.
    ///
    /// Used by the gather command to detect deletions (paths present in the
    /// parent tree but missing from the working directory).
    pub fn list_tree_files(
        &self,
        tree_hash: B3Hash,
    ) -> Result<BTreeMap<String, B3Hash>, WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut files = BTreeMap::new();
        if tree_hash != B3Hash::ZERO {
            self.collect_tree_files(&store, tree_hash, "", &mut files)?;
        }
        Ok(files)
    }

    /// Public accessor for the working directory root.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Compute workspace status by comparing working directory against last seal tree.
    pub fn status(
        &self,
        last_tree: Option<B3Hash>,
        ignore: &PatternCache,
    ) -> Result<Vec<WorkspaceFile>, WorkspaceError> {
        // Paths marked with `ivaldi skip` are hidden from status entirely.
        let disk_files: Vec<String> = self
            .scan(ignore)?
            .into_iter()
            .filter(|path| !self.skipped.covers(path))
            .collect();
        let mut result = Vec::new();

        // Build set of known files from last seal
        let mut known_files: BTreeMap<String, B3Hash> = BTreeMap::new();
        if let Some(tree_hash) = last_tree
            && tree_hash != B3Hash::ZERO
        {
            let store = FsStore::new(self.cas);
            self.collect_tree_files(&store, tree_hash, "", &mut known_files)?;
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

        // Check for deleted files (in last seal but not on disk). Skipped
        // paths never report as deleted even though the filtered scan above
        // doesn't list them.
        for (path, hash) in &known_files {
            if !disk_set.contains(path.as_str()) && !self.skipped.covers(path) {
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
    ///
    /// Writes any files in `tree_hash` that differ from disk and removes
    /// non-ignored files that aren't in the tree. Ignored files (build
    /// artifacts, editor swap files, etc.) are left untouched so a timeline
    /// switch doesn't wipe out unrelated working state.
    pub fn materialize(&self, tree_hash: B3Hash) -> Result<(), WorkspaceError> {
        let ignore = crate::ignore::load_pattern_cache(&self.work_dir);
        self.materialize_with_ignore(tree_hash, &ignore)
    }

    /// Like [`Self::materialize`] but with a caller-supplied ignore cache.
    pub fn materialize_with_ignore(
        &self,
        tree_hash: B3Hash,
        ignore: &PatternCache,
    ) -> Result<(), WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut target_files: BTreeMap<String, (B3Hash, u32)> = BTreeMap::new();
        self.collect_tree_blobs(&store, tree_hash, "", &mut target_files)?;

        // Scan the working dir respecting ignores; any file not in this set
        // is either ignored or absent — either way we won't delete it below.
        let current_files = self.scan(ignore).unwrap_or_default();
        let current_set: BTreeSet<String> = current_files.into_iter().collect();

        // Write/update files
        for (path, (blob_hash, mode)) in &target_files {
            let full_path = self.work_dir.join(path);
            let (_, content) = store
                .load_blob(*blob_hash)
                .map_err(WorkspaceError::FsMerkle)?;

            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).map_err(WorkspaceError::Io)?;
            }

            self.write_entry(&full_path, &content, *mode)?;
        }

        // Remove non-ignored files not in target tree
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
        let tree = store
            .load_tree(tree_hash)
            .map_err(WorkspaceError::FsMerkle)?;

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

    /// Collect all blob file paths with their hash and mode, recursively.
    fn collect_tree_blobs(
        &self,
        store: &FsStore<'_>,
        tree_hash: B3Hash,
        prefix: &str,
        files: &mut BTreeMap<String, (B3Hash, u32)>,
    ) -> Result<(), WorkspaceError> {
        let tree = store
            .load_tree(tree_hash)
            .map_err(WorkspaceError::FsMerkle)?;

        for entry in &tree.entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };

            match entry.kind {
                NodeKind::Blob => {
                    files.insert(path, (entry.hash, entry.mode));
                }
                NodeKind::Tree => {
                    self.collect_tree_blobs(store, entry.hash, &path, files)?;
                }
            }
        }

        Ok(())
    }

    /// Write a single materialized entry to disk, honoring its file mode:
    /// symlinks become real symlinks, executables get the exec bit set.
    /// On non-unix platforms the content is written as a plain file.
    fn write_entry(
        &self,
        full_path: &std::path::Path,
        content: &[u8],
        mode: u32,
    ) -> Result<(), WorkspaceError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            if mode == crate::fsmerkle::MODE_SYMLINK {
                // Blob content is the link target. Recreate unconditionally so a
                // changed target is reflected (fs::read would follow the link).
                let target = String::from_utf8_lossy(content).into_owned();
                if full_path.symlink_metadata().is_ok() {
                    let _ = fs::remove_file(full_path);
                }
                std::os::unix::fs::symlink(&target, full_path).map_err(WorkspaceError::Io)?;
                return Ok(());
            }

            // Regular or executable file: skip the write if content already matches.
            let needs_write = match fs::read(full_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                // If a symlink currently occupies this path, drop it first.
                if full_path
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    let _ = fs::remove_file(full_path);
                }
                fs::write(full_path, content).map_err(WorkspaceError::Io)?;
            }
            let perm = if mode == crate::fsmerkle::MODE_EXEC {
                0o755
            } else {
                0o644
            };
            fs::set_permissions(full_path, fs::Permissions::from_mode(perm))
                .map_err(WorkspaceError::Io)?;
            Ok(())
        }

        #[cfg(not(unix))]
        {
            let _ = mode;
            let needs_write = match fs::read(full_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                fs::write(full_path, content).map_err(WorkspaceError::Io)?;
            }
            Ok(())
        }
    }

    /// Save workspace state to disk.
    pub fn save(&self) -> Result<(), WorkspaceError> {
        self.staging
            .save(&self.ivaldi_dir)
            .map_err(WorkspaceError::Io)
    }

    /// Capture working-tree changes vs `base_tree` for auto-shelving.
    ///
    /// For Modified and Untracked files, the disk content is hashed into the
    /// CAS so the bytes survive a subsequent `materialize` (which will
    /// overwrite the working tree). For Deleted files only the path is
    /// recorded — the original bytes are recoverable from `base_tree` itself.
    pub fn capture_changes(
        &self,
        base_tree: Option<B3Hash>,
        ignore: &PatternCache,
    ) -> Result<Vec<crate::shelf::WorkspaceChange>, WorkspaceError> {
        let store = FsStore::new(self.cas);
        let mut known_files: BTreeMap<String, B3Hash> = BTreeMap::new();
        if let Some(tree_hash) = base_tree
            && tree_hash != B3Hash::ZERO
        {
            self.collect_tree_files(&store, tree_hash, "", &mut known_files)?;
        }

        let disk_files = self.scan(ignore)?;
        let disk_set: BTreeSet<&str> = disk_files.iter().map(|s| s.as_str()).collect();

        let mut changes = Vec::new();

        for path in &disk_files {
            let full_path = self.work_dir.join(path);
            let content = fs::read(&full_path).map_err(WorkspaceError::Io)?;
            let current_hash = BlobNode::hash_content(&content);

            match known_files.get(path.as_str()) {
                Some(known_hash) if *known_hash == current_hash => {
                    // Unmodified — nothing to capture.
                }
                Some(_) => {
                    // Modified — store blob in CAS so it survives materialize.
                    store.put_blob(&content).map_err(WorkspaceError::FsMerkle)?;
                    changes.push(crate::shelf::WorkspaceChange::Modified {
                        path: path.clone(),
                        hash: current_hash,
                    });
                }
                None => {
                    // Untracked — store blob and record.
                    store.put_blob(&content).map_err(WorkspaceError::FsMerkle)?;
                    changes.push(crate::shelf::WorkspaceChange::Untracked {
                        path: path.clone(),
                        hash: current_hash,
                    });
                }
            }
        }

        // Files in the base tree but missing from disk are Deleted.
        for path in known_files.keys() {
            if !disk_set.contains(path.as_str()) {
                changes.push(crate::shelf::WorkspaceChange::Deleted { path: path.clone() });
            }
        }

        Ok(changes)
    }

    /// Re-apply previously-captured working-tree changes after switching to
    /// a timeline. Called after `materialize` has written the target tip
    /// tree to disk.
    pub fn apply_changes(
        &self,
        changes: &[crate::shelf::WorkspaceChange],
    ) -> Result<(), WorkspaceError> {
        let store = FsStore::new(self.cas);
        for change in changes {
            match change {
                crate::shelf::WorkspaceChange::Modified { path, hash }
                | crate::shelf::WorkspaceChange::Untracked { path, hash } => {
                    let (_, content) = store.load_blob(*hash).map_err(WorkspaceError::FsMerkle)?;
                    let full_path = self.work_dir.join(path);
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent).map_err(WorkspaceError::Io)?;
                    }
                    fs::write(&full_path, &content).map_err(WorkspaceError::Io)?;
                }
                crate::shelf::WorkspaceChange::Deleted { path } => {
                    let full_path = self.work_dir.join(path);
                    let _ = fs::remove_file(&full_path);
                    // Best-effort cleanup of empty parents.
                    if let Some(mut parent) = full_path.parent().map(Path::to_path_buf) {
                        while parent.starts_with(&self.work_dir) && parent != self.work_dir {
                            if fs::remove_dir(&parent).is_err() {
                                break;
                            }
                            parent = match parent.parent() {
                                Some(p) => p.to_path_buf(),
                                None => break,
                            };
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Result of a gather operation, separating successfully gathered files
/// from dotfiles that require explicit user confirmation.
#[derive(Debug, Clone)]
pub struct GatherResult {
    /// Files that were successfully staged.
    pub gathered: Vec<String>,
    /// Dotfiles that need explicit user confirmation before staging.
    pub needs_confirmation: Vec<String>,
    /// Explicitly requested paths refused because they are marked with
    /// `ivaldi skip`. Bulk scans exclude skipped paths silently instead.
    pub skipped: Vec<String>,
}

/// Manages the persistent allowlist of dotfiles the user has explicitly
/// confirmed for staging. Stored in `.ivaldi/dotfile-allowlist`.
pub struct DotfileAllowlist {
    allowed: BTreeSet<String>,
    path: PathBuf,
}

impl DotfileAllowlist {
    pub fn load(ivaldi_dir: &Path) -> Self {
        let path = ivaldi_dir.join("dotfile-allowlist");
        let allowed = match fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => BTreeSet::new(),
        };
        Self { allowed, path }
    }

    pub fn is_allowed(&self, path: &str) -> bool {
        self.allowed.contains(path)
    }

    pub fn allow(&mut self, path: &str) {
        self.allowed.insert(path.to_string());
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let content: String = self.allowed.iter().map(|s| format!("{}\n", s)).collect();
        crate::atomic_io::atomic_write(&self.path, content.as_bytes())
    }
}

/// Manages the persistent set of paths temporarily excluded from staging
/// (`ivaldi skip` / `ivaldi unskip`). Stored in `.ivaldi/skipped`, one path
/// per line. The file is repo-local and never committed, so the exclusion
/// never leaks to clones or remotes.
pub struct SkipSet {
    paths: BTreeSet<String>,
    path: PathBuf,
}

impl SkipSet {
    pub fn load(ivaldi_dir: &Path) -> Self {
        let path = ivaldi_dir.join("skipped");
        let paths = match fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => BTreeSet::new(),
        };
        Self { paths, path }
    }

    /// True when `path` is excluded from staging: either an exact entry or
    /// covered by a skipped directory prefix.
    pub fn covers(&self, path: &str) -> bool {
        self.paths
            .iter()
            .any(|entry| path == entry || path.starts_with(&format!("{}/", entry)))
    }

    pub fn add(&mut self, path: &str) {
        self.paths.insert(path.to_string());
    }

    /// Remove a path from the set. Returns true if it was present.
    pub fn remove(&mut self, path: &str) -> bool {
        self.paths.remove(path)
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.paths.iter()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let content: String = self.paths.iter().map(|s| format!("{}\n", s)).collect();
        crate::atomic_io::atomic_write(&self.path, content.as_bytes())
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
    #[error("security blocked: {0} matches a protected pattern and cannot be staged")]
    SecurityBlocked(String),
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

    fn empty_allowlist(dir: &tempfile::TempDir) -> DotfileAllowlist {
        DotfileAllowlist::load(&dir.path().join(".ivaldi"))
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
    fn staging_area_load_tolerates_truncated_file() {
        // A crash mid-write could historically truncate the stage file.
        // load() must skip bad lines without panicking.
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(ivaldi_dir.join("stage")).unwrap();

        let good_hash = B3Hash::digest(b"content");
        let content = format!("{} file.txt\ndel old.txt\nabc12", good_hash);
        fs::write(ivaldi_dir.join("stage/files"), content).unwrap();

        let loaded = StagingArea::load(&ivaldi_dir);
        assert!(loaded.is_staged("file.txt"));
        assert!(loaded.is_staged_for_deletion("old.txt"));
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn staging_area_deletion_and_addition_are_mutually_exclusive() {
        let mut staging = StagingArea::new();
        let h = B3Hash::digest(b"x");

        staging.stage("X", h);
        assert!(staging.is_staged("X"));
        assert!(!staging.is_staged_for_deletion("X"));

        staging.stage_deletion("X");
        assert!(!staging.is_staged("X"));
        assert!(staging.is_staged_for_deletion("X"));

        staging.stage("X", h);
        assert!(staging.is_staged("X"));
        assert!(!staging.is_staged_for_deletion("X"));
    }

    #[test]
    fn staging_area_unstage_handles_deletion() {
        let mut staging = StagingArea::new();
        staging.stage_deletion("doomed.txt");
        assert!(staging.is_staged_for_deletion("doomed.txt"));
        assert!(staging.unstage("doomed.txt"));
        assert!(staging.is_empty());
    }

    #[test]
    fn staging_area_save_load_round_trips_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut staging = StagingArea::new();
        staging.stage("file.txt", B3Hash::digest(b"content"));
        staging.stage_deletion("old.txt");
        staging.stage_deletion("legacy/dir.json");
        staging.save(&ivaldi_dir).unwrap();

        let loaded = StagingArea::load(&ivaldi_dir);
        assert_eq!(loaded.len(), 3);
        assert!(loaded.is_staged("file.txt"));
        assert!(loaded.is_staged_for_deletion("old.txt"));
        assert!(loaded.is_staged_for_deletion("legacy/dir.json"));
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

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&["file.txt"], &allowlist).unwrap();

        assert_eq!(result.gathered, vec!["file.txt"]);
        assert!(result.needs_confirmation.is_empty());
        assert!(ws.staging.is_staged("file.txt"));
        assert_eq!(cas.len(), 1); // Content stored in CAS
    }

    #[test]
    fn gather_expands_directory_argument() {
        let (dir, cas) = setup_workspace();
        fs::create_dir_all(dir.path().join("solutions/01_variables")).unwrap();
        fs::write(dir.path().join("solutions/a.oxi"), "a").unwrap();
        fs::write(dir.path().join("solutions/01_variables/b.oxi"), "b").unwrap();
        // A sibling file outside the directory must not be gathered.
        fs::write(dir.path().join("outside.oxi"), "x").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        // Trailing slash, as a user would type it, must not corrupt paths.
        let result = ws.gather(&["solutions/"], &allowlist).unwrap();

        assert_eq!(
            result.gathered,
            vec!["solutions/01_variables/b.oxi", "solutions/a.oxi"]
        );
        assert!(ws.staging.is_staged("solutions/a.oxi"));
        assert!(ws.staging.is_staged("solutions/01_variables/b.oxi"));
        assert!(!ws.staging.is_staged("outside.oxi"));
    }

    #[test]
    fn gather_dot_directory_needs_confirmation() {
        let (dir, cas) = setup_workspace();
        fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        fs::write(dir.path().join(".github/workflows/ci.yml"), "ci").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&[".github/"], &allowlist).unwrap();

        // A dot-directory is held for confirmation, not expanded outright.
        assert!(result.gathered.is_empty());
        assert_eq!(result.needs_confirmation, vec![".github/"]);
        assert!(!ws.staging.is_staged(".github/workflows/ci.yml"));

        // Confirming expands it, staging the non-dot files beneath.
        let confirmed = ws.gather_confirmed(&[".github/"]).unwrap();
        assert_eq!(confirmed, vec![".github/workflows/ci.yml"]);
        assert!(ws.staging.is_staged(".github/workflows/ci.yml"));
    }

    #[test]
    fn gather_with_progress_fires_callback_per_file() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();
        fs::write(dir.path().join("c.txt"), "ccc").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));

        let mut seen = Vec::new();
        let result = ws
            .gather_with_progress(
                &["a.txt", "b.txt", "c.txt", "missing.txt"],
                &allowlist,
                &mut |p| seen.push(p.to_string()),
            )
            .unwrap();

        // Callback fires exactly once per gathered file; missing files are
        // skipped without a callback.
        assert_eq!(seen, vec!["a.txt", "b.txt", "c.txt"]);
        assert_eq!(result.gathered, seen);
    }

    #[test]
    fn gather_all() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let result = ws.gather_all(&ignore).unwrap();

        assert_eq!(result.gathered.len(), 2);
        assert!(result.needs_confirmation.is_empty());
        assert_eq!(ws.staging.len(), 2);
    }

    #[test]
    fn build_staged_tree() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main()").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["file.txt", "src/main.rs"], &allowlist).unwrap();

        let tree_hash = ws.build_staged_tree().unwrap();
        assert_ne!(tree_hash, B3Hash::ZERO);

        let store = FsStore::new(&cas);
        let tree = store.load_tree(tree_hash).unwrap();
        assert_eq!(tree.entries.len(), 2);
    }

    #[test]
    fn build_seal_tree_with_no_parent_matches_staging() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["a.txt", "b.txt"], &allowlist).unwrap();

        let tree_hash = ws.build_seal_tree(None).unwrap();
        let store = FsStore::new(&cas);
        let tree = store.load_tree(tree_hash).unwrap();
        let names: Vec<&str> = tree.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[test]
    fn build_seal_tree_inherits_parent_files() {
        // Parent tree has a, b, c. Stage only d. New tree must have a, b, c, d.
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();
        fs::write(dir.path().join("c.txt"), "ccc").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["a.txt", "b.txt", "c.txt"], &allowlist).unwrap();
        let parent_tree = ws.build_seal_tree(None).unwrap();
        ws.staging.clear();

        // Now stage only a brand-new file and "seal" with the parent tree.
        fs::write(dir.path().join("d.txt"), "ddd").unwrap();
        ws.gather(&["d.txt"], &allowlist).unwrap();

        let new_tree = ws.build_seal_tree(Some(parent_tree)).unwrap();
        let mut files = BTreeMap::new();
        ws.collect_tree_files(&FsStore::new(&cas), new_tree, "", &mut files)
            .unwrap();

        assert!(
            files.contains_key("a.txt"),
            "parent file a.txt should survive"
        );
        assert!(
            files.contains_key("b.txt"),
            "parent file b.txt should survive"
        );
        assert!(
            files.contains_key("c.txt"),
            "parent file c.txt should survive"
        );
        assert!(
            files.contains_key("d.txt"),
            "newly staged d.txt should be present"
        );
    }

    #[test]
    fn build_seal_tree_modifies_existing_path() {
        // Parent has X at hash A; stage X at hash B. New tree has X at B.
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("X"), "old").unwrap();
        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["X"], &allowlist).unwrap();
        let parent_tree = ws.build_seal_tree(None).unwrap();
        ws.staging.clear();

        fs::write(dir.path().join("X"), "new").unwrap();
        ws.gather(&["X"], &allowlist).unwrap();
        let new_tree = ws.build_seal_tree(Some(parent_tree)).unwrap();

        let mut files = BTreeMap::new();
        ws.collect_tree_files(&FsStore::new(&cas), new_tree, "", &mut files)
            .unwrap();
        let new_hash = files.get("X").expect("X should exist in new tree");
        let store = FsStore::new(&cas);
        let (_, content) = store.load_blob(*new_hash).unwrap();
        assert_eq!(content, b"new");
    }

    #[test]
    fn build_seal_tree_omits_staged_deletions() {
        // Parent has a, b, c. Stage deletion of b. New tree has a, c only.
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb").unwrap();
        fs::write(dir.path().join("c.txt"), "ccc").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["a.txt", "b.txt", "c.txt"], &allowlist).unwrap();
        let parent_tree = ws.build_seal_tree(None).unwrap();
        ws.staging.clear();

        ws.staging.stage_deletion("b.txt");
        let new_tree = ws.build_seal_tree(Some(parent_tree)).unwrap();

        let mut files = BTreeMap::new();
        ws.collect_tree_files(&FsStore::new(&cas), new_tree, "", &mut files)
            .unwrap();
        assert!(files.contains_key("a.txt"));
        assert!(
            !files.contains_key("b.txt"),
            "b.txt should have been removed"
        );
        assert!(files.contains_key("c.txt"));
    }

    #[test]
    fn list_tree_files_returns_full_map() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        fs::create_dir_all(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/b.txt"), "bbb").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["a.txt", "nested/b.txt"], &allowlist).unwrap();
        let tree = ws.build_seal_tree(None).unwrap();

        let map = ws.list_tree_files(tree).unwrap();
        assert!(map.contains_key("a.txt"));
        assert!(map.contains_key("nested/b.txt"));
        assert_eq!(map.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn materialize_applies_exec_bit_and_symlink() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, cas) = setup_workspace();
        let store = FsStore::new(&cas);
        let (regular, _) = store.put_blob(b"plain\n").unwrap();
        let (script, _) = store.put_blob(b"#!/bin/sh\n").unwrap();
        let (link, _) = store.put_blob(b"regular.txt").unwrap();

        let tree = store
            .put_tree(vec![
                Entry {
                    name: "regular.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: regular,
                },
                Entry {
                    name: "run.sh".into(),
                    mode: crate::fsmerkle::MODE_EXEC,
                    kind: NodeKind::Blob,
                    hash: script,
                },
                Entry {
                    name: "link".into(),
                    mode: crate::fsmerkle::MODE_SYMLINK,
                    kind: NodeKind::Blob,
                    hash: link,
                },
            ])
            .unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.materialize_with_ignore(tree, &PatternCache::new(&[]))
            .unwrap();

        // Executable bit set on the script.
        let exec = fs::metadata(dir.path().join("run.sh")).unwrap();
        assert!(exec.permissions().mode() & 0o111 != 0, "exec bit set");

        // Symlink created pointing at its target (not a regular file).
        let lmeta = fs::symlink_metadata(dir.path().join("link")).unwrap();
        assert!(lmeta.file_type().is_symlink(), "link is a real symlink");
        assert_eq!(
            fs::read_link(dir.path().join("link"))
                .unwrap()
                .to_str()
                .unwrap(),
            "regular.txt"
        );

        // Regular file is not executable.
        let reg = fs::metadata(dir.path().join("regular.txt")).unwrap();
        assert!(reg.permissions().mode() & 0o111 == 0, "regular not exec");
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
        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(&["file.txt"], &allowlist).unwrap();

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
    fn gather_rejects_env_file() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".env"), "SECRET=abc").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&[".env"], &allowlist);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, WorkspaceError::SecurityBlocked(_)),
            "expected SecurityBlocked, got: {err}"
        );
    }

    #[test]
    fn gather_rejects_env_even_if_allowlisted() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".env"), "SECRET=abc").unwrap();

        let mut allowlist = empty_allowlist(&dir);
        allowlist.allow(".env"); // try to force-allow it
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&[".env"], &allowlist);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkspaceError::SecurityBlocked(_)
        ));
    }

    #[test]
    fn gather_dotfile_needs_confirmation_without_allowlist() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".prettierrc"), "{}").unwrap();

        let allowlist = empty_allowlist(&dir);
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&[".prettierrc"], &allowlist).unwrap();

        assert!(result.gathered.is_empty());
        assert_eq!(result.needs_confirmation, vec![".prettierrc"]);
        assert!(!ws.staging.is_staged(".prettierrc"));
    }

    #[test]
    fn gather_allows_dotfile_with_allowlist() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".prettierrc"), "{}").unwrap();

        let mut allowlist = empty_allowlist(&dir);
        allowlist.allow(".prettierrc");
        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather(&[".prettierrc"], &allowlist).unwrap();

        assert_eq!(result.gathered, vec![".prettierrc"]);
        assert!(result.needs_confirmation.is_empty());
        assert!(ws.staging.is_staged(".prettierrc"));
    }

    #[test]
    fn gather_confirmed_stages_dotfile() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".prettierrc"), "{}").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let gathered = ws.gather_confirmed(&[".prettierrc"]).unwrap();
        assert_eq!(gathered, vec![".prettierrc"]);
        assert!(ws.staging.is_staged(".prettierrc"));
    }

    #[test]
    fn gather_confirmed_still_rejects_env() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".env"), "SECRET=abc").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let result = ws.gather_confirmed(&[".env"]);
        assert!(matches!(
            result.unwrap_err(),
            WorkspaceError::SecurityBlocked(_)
        ));
    }

    #[test]
    fn dotfile_allowlist_persistence() {
        let (dir, _cas) = setup_workspace();
        let ivaldi_dir = dir.path().join(".ivaldi");

        let mut allowlist = DotfileAllowlist::load(&ivaldi_dir);
        assert!(!allowlist.is_allowed(".prettierrc"));

        allowlist.allow(".prettierrc");
        allowlist.allow(".editorconfig");
        allowlist.save().unwrap();

        // Reload from disk
        let reloaded = DotfileAllowlist::load(&ivaldi_dir);
        assert!(reloaded.is_allowed(".prettierrc"));
        assert!(reloaded.is_allowed(".editorconfig"));
        assert!(!reloaded.is_allowed(".npmrc"));
    }

    #[test]
    fn gather_all_skips_dotfiles() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".npmrc"), "registry=...").unwrap();
        fs::write(dir.path().join("foo.txt"), "hello").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let result = ws.gather_all(&ignore).unwrap();

        assert_eq!(result.gathered, vec!["foo.txt"]);
    }

    #[test]
    fn gather_all_reports_skipped_dotfiles() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".npmrc"), "registry=...").unwrap();
        fs::write(dir.path().join("foo.txt"), "hello").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let result = ws.gather_all(&ignore).unwrap();

        assert_eq!(result.gathered, vec!["foo.txt"]);
        assert_eq!(result.needs_confirmation, vec![".npmrc"]);
    }

    #[test]
    fn gather_all_skips_env_from_report() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".env"), "SECRET=abc").unwrap();
        fs::write(dir.path().join(".npmrc"), "registry=...").unwrap();
        fs::write(dir.path().join("foo.txt"), "hello").unwrap();

        let mut ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let result = ws.gather_all(&ignore).unwrap();

        assert_eq!(result.gathered, vec!["foo.txt"]);
        // .env should NOT appear — it's security-blocked, not just a dotfile
        assert_eq!(result.needs_confirmation, vec![".npmrc"]);
        assert!(!result.needs_confirmation.contains(&".env".to_string()));
    }

    #[test]
    fn find_dotfiles_discovers_hidden_files() {
        let (dir, cas) = setup_workspace();
        fs::write(dir.path().join(".prettierrc"), "{}").unwrap();
        fs::write(dir.path().join(".editorconfig"), "[*]").unwrap();
        fs::write(dir.path().join("normal.txt"), "hi").unwrap();
        // .ivaldiignore should NOT be reported
        fs::write(dir.path().join(".ivaldiignore"), "*.log").unwrap();

        let ws = Workspace::new(&cas, dir.path(), dir.path().join(".ivaldi"));
        let ignore = PatternCache::new(&[]);
        let dotfiles = ws.find_dotfiles(&ignore).unwrap();

        assert!(dotfiles.contains(&".prettierrc".to_string()));
        assert!(dotfiles.contains(&".editorconfig".to_string()));
        assert!(!dotfiles.contains(&"normal.txt".to_string()));
        assert!(!dotfiles.contains(&".ivaldiignore".to_string()));
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
