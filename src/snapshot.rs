//! Workspace snapshots: the safety net behind `ivaldi oops`.
//!
//! Seals are permanent, but the work *between* seals is not: several commands
//! rewrite the working directory (`fuse`, `sync`, `reverse`, `rewind
//! --discard`), and an uncommitted edit that is in the way is simply gone.
//! Before any of them touches a file, it records a snapshot of everything that
//! is not in a seal yet, so one command can put it all back.
//!
//! A snapshot is the same thing a [shelf](crate::shelf) is — staged entries
//! plus working-tree changes against the timeline's tip, with the changed
//! content hashed into the CAS — and additionally the timeline and head it was
//! taken against, because the commands it guards move the head as well as the
//! files. Undoing them means putting both back.
//!
//! The expensive part is already paid for: content lives in the CAS, which is
//! content-addressed and shared with every seal, so a snapshot itself is a few
//! lines of text.
//!
//! Storage: `.ivaldi/snapshots/<id>.snap`, newest id highest. Only the most
//! recent [`KEEP`] are retained.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cas::FileCas;
use crate::hash::B3Hash;
use crate::repo::Repo;
use crate::shelf::WorkspaceChange;
use crate::workspace::Workspace;

/// How many snapshots are retained. Older ones are pruned on save.
pub const KEEP: usize = 20;

/// Everything about a working directory that is not in a seal, plus where the
/// timeline stood, at the moment before a command rewrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Assigned by [`SnapshotManager::save`]; 0 until then.
    pub id: u64,
    /// The command this snapshot was taken for, as the user typed it
    /// (`fuse main`, `sync`). Shown by `oops` so they know what is undone.
    pub command: String,
    /// Timeline that was current.
    pub timeline: String,
    /// Head of `timeline` at the time; `None` for a timeline with no seals.
    pub head: Option<u64>,
    /// Unix seconds.
    pub created_at: i64,
    /// Staged additions/modifications: path → content hash.
    pub staged_files: BTreeMap<String, B3Hash>,
    /// Paths staged for deletion.
    pub staged_deletions: BTreeSet<String>,
    /// Working-tree changes vs the tree at `head`.
    pub workspace_changes: Vec<WorkspaceChange>,
}

impl Snapshot {
    /// True if there was no uncommitted state at all. Such a snapshot still
    /// matters: it records the head, which is what undoing a fuse or sync
    /// needs even from a clean tree.
    pub fn is_clean(&self) -> bool {
        self.staged_files.is_empty()
            && self.staged_deletions.is_empty()
            && self.workspace_changes.is_empty()
    }
}

/// Capture the current timeline's head and all uncommitted state.
///
/// The returned snapshot is in memory only, but the content it names is
/// already durable in the CAS. Call this *before* the command touches
/// anything, then [`SnapshotManager::save`] it.
pub fn capture(repo: &Repo, cas: &FileCas, command: &str) -> Result<Snapshot, SnapshotError> {
    let timeline = repo.current_timeline()?;
    let head = repo.get_timeline_head(&timeline)?;
    let head_tree = match head {
        Some(idx) => repo.get_leaf(idx)?.map(|leaf| leaf.tree_root),
        None => None,
    };

    let ws = Workspace::new(cas, &repo.work_dir, &repo.ivaldi_dir);
    let ignore = crate::ignore::load_pattern_cache(&repo.work_dir);
    let workspace_changes = ws.capture_changes(head_tree, &ignore)?;
    let staged_files = ws.staging.staged_files().clone();
    let staged_deletions = ws.staging.staged_deletions().clone();

    // The CAS may now hold the only copy of the user's uncommitted content,
    // and the caller is about to overwrite the working-tree copy.
    cas.flush()?;

    Ok(Snapshot {
        id: 0,
        command: command.to_string(),
        timeline,
        head,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
        staged_files,
        staged_deletions,
        workspace_changes,
    })
}

/// [`capture`] and persist in one step: what a command calls on its way in.
pub fn take(repo: &Repo, cas: &FileCas, command: &str) -> Result<u64, SnapshotError> {
    let snapshot = capture(repo, cas, command)?;
    SnapshotManager::new(&repo.ivaldi_dir).save(&snapshot)
}

/// Put the repository back the way `snapshot` found it: timeline head, files,
/// and staging.
///
/// Every step is idempotent and the snapshot is not consumed, so a restore
/// interrupted halfway is finished by running it again.
pub fn restore(repo: &Repo, cas: &FileCas, snapshot: &Snapshot) -> Result<(), SnapshotError> {
    let current = repo.current_timeline()?;
    if current != snapshot.timeline {
        return Err(SnapshotError::WrongTimeline {
            snapshot: snapshot.timeline.clone(),
            current,
        });
    }

    let ws = Workspace::new(cas, &repo.work_dir, &repo.ivaldi_dir);
    if let Some(head) = snapshot.head {
        let leaf = repo
            .get_leaf(head)?
            .ok_or(SnapshotError::MissingHead(head))?;
        if repo.get_timeline_head(&snapshot.timeline)? != Some(head) {
            // The seals this leaves behind are orphaned, not deleted — the
            // history is append-only — so they stay reachable by name.
            repo.set_timeline_head(&snapshot.timeline, head)?;
        }
        crate::failpoint::fail_point("oops.after_head");
        ws.materialize(leaf.tree_root)?;
    }
    crate::failpoint::fail_point("oops.after_materialize");
    ws.apply_changes(&snapshot.workspace_changes)?;

    let mut ws = ws;
    ws.staging.clear();
    for (path, hash) in &snapshot.staged_files {
        ws.staging.stage(path.clone(), *hash);
    }
    for path in &snapshot.staged_deletions {
        ws.staging.stage_deletion(path.clone());
    }
    ws.save()?;
    Ok(())
}

/// Where a fuse parks the uncommitted work it is carrying across the merge.
///
/// It is a snapshot like any other, but kept apart from the numbered ones: it
/// is not history to undo, it is state a fuse in progress still owes back to
/// the working directory, and it must survive pruning and `oops` until then.
fn carry_path(ivaldi_dir: &Path) -> PathBuf {
    ivaldi_dir.join("fuse-carry.snap")
}

pub fn save_carry(ivaldi_dir: &Path, snapshot: &Snapshot) -> Result<(), SnapshotError> {
    crate::atomic_io::atomic_write(&carry_path(ivaldi_dir), encode(snapshot).as_bytes())?;
    Ok(())
}

pub fn load_carry(ivaldi_dir: &Path) -> Result<Option<Snapshot>, SnapshotError> {
    match fs::read_to_string(carry_path(ivaldi_dir)) {
        Ok(content) => decode(0, &content).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SnapshotError::Io(e)),
    }
}

pub fn clear_carry(ivaldi_dir: &Path) -> Result<(), SnapshotError> {
    match fs::remove_file(carry_path(ivaldi_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SnapshotError::Io(e)),
    }
}

fn encode(snapshot: &Snapshot) -> String {
    let mut lines = vec![
        format!("command {}", snapshot.command),
        format!("timeline {}", snapshot.timeline),
        format!("created_at {}", snapshot.created_at),
    ];
    if let Some(head) = snapshot.head {
        lines.push(format!("head {}", head));
    }
    for (path, hash) in &snapshot.staged_files {
        lines.push(format!("staged {} {}", hash, path));
    }
    for path in &snapshot.staged_deletions {
        lines.push(format!("staged_deletion {}", path));
    }
    for change in &snapshot.workspace_changes {
        lines.push(match change {
            WorkspaceChange::Modified { path, hash } => format!("modified {} {}", hash, path),
            WorkspaceChange::Untracked { path, hash } => {
                format!("untracked {} {}", hash, path)
            }
            WorkspaceChange::Deleted { path } => format!("deleted {}", path),
        });
    }
    // Written last, checked on load: a snapshot cut short by a crash must
    // not be mistaken for one that simply had fewer changes.
    lines.push("end".to_string());
    lines.join("\n")
}

fn decode(id: u64, content: &str) -> Result<Snapshot, SnapshotError> {
    let mut snapshot = Snapshot {
        id,
        command: String::new(),
        timeline: String::new(),
        head: None,
        created_at: 0,
        staged_files: BTreeMap::new(),
        staged_deletions: BTreeSet::new(),
        workspace_changes: Vec::new(),
    };
    let mut complete = false;

    let hashed = |rest: &str| -> Option<(B3Hash, String)> {
        let (hash, path) = rest.split_once(' ')?;
        Some((B3Hash::from_hex(hash)?, path.to_string()))
    };

    for line in content.lines() {
        // Paths may legitimately end in whitespace; only the line ending
        // is not part of the record.
        let line = line.trim_end_matches('\r');
        let malformed = || SnapshotError::Malformed(id, line.to_string());
        if line == "end" {
            complete = true;
        } else if let Some(rest) = line.strip_prefix("command ") {
            snapshot.command = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("timeline ") {
            snapshot.timeline = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("created_at ") {
            snapshot.created_at = rest.parse().map_err(|_| malformed())?;
        } else if let Some(rest) = line.strip_prefix("head ") {
            snapshot.head = Some(rest.parse().map_err(|_| malformed())?);
        } else if let Some(rest) = line.strip_prefix("staged_deletion ") {
            snapshot.staged_deletions.insert(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("staged ") {
            let (hash, path) = hashed(rest).ok_or_else(malformed)?;
            snapshot.staged_files.insert(path, hash);
        } else if let Some(rest) = line.strip_prefix("modified ") {
            let (hash, path) = hashed(rest).ok_or_else(malformed)?;
            snapshot
                .workspace_changes
                .push(WorkspaceChange::Modified { path, hash });
        } else if let Some(rest) = line.strip_prefix("untracked ") {
            let (hash, path) = hashed(rest).ok_or_else(malformed)?;
            snapshot
                .workspace_changes
                .push(WorkspaceChange::Untracked { path, hash });
        } else if let Some(rest) = line.strip_prefix("deleted ") {
            snapshot.workspace_changes.push(WorkspaceChange::Deleted {
                path: rest.to_string(),
            });
        } else if !line.is_empty() {
            // Unlike a shelf, a snapshot is restored *over* the user's
            // files. Guessing at a line we cannot read is how a restore
            // quietly drops a file, so refuse instead.
            return Err(malformed());
        }
    }

    if !complete {
        return Err(SnapshotError::Truncated(id));
    }
    Ok(snapshot)
}

/// Reads and writes `.ivaldi/snapshots`.
pub struct SnapshotManager {
    dir: PathBuf,
}

impl SnapshotManager {
    pub fn new(ivaldi_dir: &Path) -> Self {
        Self {
            dir: ivaldi_dir.join("snapshots"),
        }
    }

    /// Persist `snapshot` under the next id, prune beyond [`KEEP`], and return
    /// the id. The caller must flush the CAS first: the snapshot is only as
    /// durable as the blobs it names.
    pub fn save(&self, snapshot: &Snapshot) -> Result<u64, SnapshotError> {
        fs::create_dir_all(&self.dir)?;
        let ids = self.ids()?;
        let id = ids.last().map_or(1, |last| last + 1);

        crate::atomic_io::atomic_write(&self.path(id), encode(snapshot).as_bytes())?;

        let excess = (ids.len() + 1).saturating_sub(KEEP);
        for old in ids.into_iter().take(excess) {
            let _ = fs::remove_file(self.path(old));
        }
        Ok(id)
    }

    /// The most recent snapshot, if any.
    pub fn latest(&self) -> Result<Option<Snapshot>, SnapshotError> {
        match self.ids()?.last() {
            Some(&id) => self.load(id),
            None => Ok(None),
        }
    }

    /// All snapshots, newest first.
    pub fn list(&self) -> Result<Vec<Snapshot>, SnapshotError> {
        let mut out = Vec::new();
        for id in self.ids()?.into_iter().rev() {
            if let Some(snapshot) = self.load(id)? {
                out.push(snapshot);
            }
        }
        Ok(out)
    }

    pub fn load(&self, id: u64) -> Result<Option<Snapshot>, SnapshotError> {
        let content = match fs::read_to_string(self.path(id)) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SnapshotError::Io(e)),
        };

        decode(id, &content).map(Some)
    }

    pub fn remove(&self, id: u64) -> Result<(), SnapshotError> {
        match fs::remove_file(self.path(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SnapshotError::Io(e)),
        }
    }

    /// Snapshot ids on disk, ascending.
    fn ids(&self) -> Result<Vec<u64>, SnapshotError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(SnapshotError::Io(e)),
        };
        let mut ids: Vec<u64> = entries
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()?
                    .strip_suffix(".snap")?
                    .parse()
                    .ok()
            })
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{:010}.snap", id))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Repo(#[from] crate::repo::RepoError),
    #[error("{0}")]
    Workspace(#[from] crate::workspace::WorkspaceError),
    #[error("{0}")]
    Cas(#[from] crate::cas::CasError),
    #[error(
        "that snapshot was taken on timeline '{snapshot}', but you are on '{current}'. \
         Run 'ivaldi timeline switch {snapshot}' first."
    )]
    WrongTimeline { snapshot: String, current: String },
    #[error("snapshot refers to seal #{0}, which is not in this repository")]
    MissingHead(u64),
    #[error("snapshot {0} is incomplete (interrupted while being written)")]
    Truncated(u64),
    #[error("snapshot {0} has a line that cannot be read: {1:?}")]
    Malformed(u64, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(command: &str) -> Snapshot {
        let mut staged_files = BTreeMap::new();
        staged_files.insert("src/staged file.rs".to_string(), B3Hash::digest(b"staged"));
        let mut staged_deletions = BTreeSet::new();
        staged_deletions.insert("gone.txt".to_string());
        Snapshot {
            id: 0,
            command: command.to_string(),
            timeline: "main".to_string(),
            head: Some(7),
            created_at: 1_700_000_000,
            staged_files,
            staged_deletions,
            workspace_changes: vec![
                WorkspaceChange::Modified {
                    path: "a b.txt".to_string(),
                    hash: B3Hash::digest(b"modified"),
                },
                WorkspaceChange::Untracked {
                    path: "new.txt".to_string(),
                    hash: B3Hash::digest(b"untracked"),
                },
                WorkspaceChange::Deleted {
                    path: "old.txt".to_string(),
                },
            ],
        }
    }

    #[test]
    fn round_trips_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        let id = mgr.save(&sample("fuse main")).unwrap();
        assert_eq!(id, 1);

        let mut expected = sample("fuse main");
        expected.id = 1;
        assert_eq!(mgr.load(1).unwrap().unwrap(), expected);
        assert!(!expected.is_clean());
    }

    #[test]
    fn headless_clean_snapshot_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        let snapshot = Snapshot {
            head: None,
            staged_files: BTreeMap::new(),
            staged_deletions: BTreeSet::new(),
            workspace_changes: Vec::new(),
            ..sample("sync")
        };
        mgr.save(&snapshot).unwrap();
        let loaded = mgr.latest().unwrap().unwrap();
        assert_eq!(loaded.head, None);
        assert!(loaded.is_clean());
    }

    #[test]
    fn latest_and_list_are_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        assert!(mgr.latest().unwrap().is_none());
        assert!(mgr.list().unwrap().is_empty());

        mgr.save(&sample("first")).unwrap();
        mgr.save(&sample("second")).unwrap();
        assert_eq!(mgr.latest().unwrap().unwrap().command, "second");
        let commands: Vec<String> = mgr.list().unwrap().into_iter().map(|s| s.command).collect();
        assert_eq!(commands, ["second", "first"]);
    }

    #[test]
    fn ids_keep_climbing_after_a_remove() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        mgr.save(&sample("one")).unwrap();
        let two = mgr.save(&sample("two")).unwrap();
        mgr.remove(1).unwrap();
        mgr.remove(1).unwrap(); // removing a missing snapshot is fine
        assert_eq!(mgr.save(&sample("three")).unwrap(), two + 1);
    }

    #[test]
    fn prunes_to_keep() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        for i in 0..KEEP + 5 {
            mgr.save(&sample(&format!("op {i}"))).unwrap();
        }
        let all = mgr.list().unwrap();
        assert_eq!(all.len(), KEEP);
        assert_eq!(all[0].command, format!("op {}", KEEP + 4));
        assert_eq!(all[KEEP - 1].command, "op 5");
    }

    #[test]
    fn truncated_snapshot_is_refused_not_half_restored() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        mgr.save(&sample("fuse main")).unwrap();
        let path = mgr.path(1);
        let full = fs::read_to_string(&path).unwrap();
        fs::write(&path, full.trim_end_matches("end")).unwrap();
        assert!(matches!(mgr.load(1), Err(SnapshotError::Truncated(1))));
    }

    #[test]
    fn unreadable_line_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SnapshotManager::new(dir.path());
        mgr.save(&sample("fuse main")).unwrap();
        let path = mgr.path(1);
        let full = fs::read_to_string(&path).unwrap();
        fs::write(&path, full.replace("modified ", "modified nothex ")).unwrap();
        assert!(matches!(mgr.load(1), Err(SnapshotError::Malformed(1, _))));
    }
}
