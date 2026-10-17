//! Butterfly timelines for Ivaldi VCS.
//!
//! Butterflies are experimental sandbox timelines that branch from a parent.
//! They enable safe experimentation without polluting the parent timeline's history.
//!
//! Features:
//! - Branch from a parent timeline at a specific divergence point
//! - Sync changes bidirectionally (up to parent, down from parent)
//! - Nested butterflies (butterfly → butterfly → butterfly)
//! - Cascade delete of nested butterflies
//! - Orphan detection when parent is deleted
//!
//! [`ButterflyManager`] is the model. The repository keeps one JSON record per
//! butterfly in the store; [`load`] builds the model from them, and every
//! change goes back in one store transaction. [`create`], [`up`], [`down`]
//! and [`remove`] are the operations the CLI and TUI share, and
//! [`on_timeline_removed`] / [`on_timeline_renamed`] keep the records in step
//! when a timeline goes away or changes name by any route.
//!
//! Syncing is a fuse ([`crate::fuse_op`]): changes merge per file and per
//! line, and only what collides is settled, by the layer rules
//! ([`LayerResolver`]).

use std::collections::BTreeMap;

use crate::carry::CarryReport;
use crate::fsmerkle::FsStore;
use crate::fuse::{Resolution, Strategy};
use crate::fuse_op::{FuseOpError, FusePlan, Planned};
use crate::hash::B3Hash;
use crate::repo::{CommitResult, Repo, RepoError};
use crate::resolve::{Labels, Region, Resolver, Side, Stop, WholeFile};
use crate::store::{Store, StoreError};

/// A butterfly timeline record.
#[derive(Debug, Clone)]
pub struct Butterfly {
    /// Name of this butterfly timeline.
    pub name: String,
    /// Name of the parent timeline.
    pub parent_name: String,
    /// Hash at which the butterfly diverged from the parent.
    pub divergence_hash: B3Hash,
    /// Unix timestamp when the butterfly was created.
    pub created_at: i64,
    /// Whether the parent has been deleted (orphaned).
    pub is_orphaned: bool,
    /// Original parent name (preserved after orphaning).
    pub original_parent: Option<String>,
}

/// Metadata about a timeline's butterfly status.
#[derive(Debug, Clone)]
pub struct ButterflyMetadata {
    /// Timeline name.
    pub timeline: String,
    /// Whether this timeline is a butterfly.
    pub is_butterfly: bool,
    /// Butterfly info (if is_butterfly).
    pub butterfly: Option<Butterfly>,
    /// Names of child butterflies.
    pub children: Vec<String>,
}

/// Manages butterfly timeline metadata.
pub struct ButterflyManager {
    /// Butterfly records: name → Butterfly.
    butterflies: BTreeMap<String, Butterfly>,
    /// Parent → children mapping.
    children: BTreeMap<String, Vec<String>>,
}

impl ButterflyManager {
    pub fn new() -> Self {
        Self {
            butterflies: BTreeMap::new(),
            children: BTreeMap::new(),
        }
    }

    /// Create a new butterfly from a parent timeline.
    pub fn create(
        &mut self,
        name: &str,
        parent_name: &str,
        divergence_hash: B3Hash,
        created_at: i64,
    ) -> Result<(), ButterflyError> {
        if self.butterflies.contains_key(name) {
            return Err(ButterflyError::AlreadyExists(name.to_string()));
        }

        let bf = Butterfly {
            name: name.to_string(),
            parent_name: parent_name.to_string(),
            divergence_hash,
            created_at,
            is_orphaned: false,
            original_parent: None,
        };

        self.butterflies.insert(name.to_string(), bf);
        self.children
            .entry(parent_name.to_string())
            .or_default()
            .push(name.to_string());

        Ok(())
    }

    /// Check if a timeline is a butterfly.
    pub fn is_butterfly(&self, name: &str) -> bool {
        self.butterflies.contains_key(name)
    }

    /// Get butterfly info.
    pub fn get(&self, name: &str) -> Option<&Butterfly> {
        self.butterflies.get(name)
    }

    /// Get the parent name of a butterfly.
    pub fn get_parent(&self, name: &str) -> Option<&str> {
        self.butterflies.get(name).map(|bf| bf.parent_name.as_str())
    }

    /// Get children of a timeline (butterflies that branch from it).
    pub fn get_children(&self, name: &str) -> Vec<String> {
        self.children.get(name).cloned().unwrap_or_default()
    }

    /// Get the divergence hash for a butterfly.
    pub fn get_divergence(&self, name: &str) -> Option<B3Hash> {
        self.butterflies.get(name).map(|bf| bf.divergence_hash)
    }

    /// Update the divergence point after a sync operation.
    pub fn update_divergence(
        &mut self,
        name: &str,
        new_divergence: B3Hash,
    ) -> Result<(), ButterflyError> {
        let bf = self
            .butterflies
            .get_mut(name)
            .ok_or_else(|| ButterflyError::NotFound(name.to_string()))?;
        bf.divergence_hash = new_divergence;
        Ok(())
    }

    /// Delete a butterfly. If cascade is true, recursively delete children.
    /// If cascade is false, children become orphaned.
    pub fn delete(&mut self, name: &str, cascade: bool) -> Result<Vec<String>, ButterflyError> {
        let bf = self
            .butterflies
            .get(name)
            .ok_or_else(|| ButterflyError::NotFound(name.to_string()))?
            .clone();

        let children = self.get_children(name);
        let mut deleted = Vec::new();

        if cascade {
            // Recursively delete children first
            for child in &children {
                let child_deleted = self.delete(child, true)?;
                deleted.extend(child_deleted);
            }
        } else {
            self.orphan_children(name, Some(&bf.parent_name));
        }

        // Remove from parent's children list
        if !bf.is_orphaned
            && let Some(siblings) = self.children.get_mut(&bf.parent_name)
        {
            siblings.retain(|c| c != name);
            if siblings.is_empty() {
                self.children.remove(&bf.parent_name);
            }
        }

        // Remove children mapping for this butterfly
        self.children.remove(name);

        // Remove the butterfly itself
        self.butterflies.remove(name);
        deleted.push(name.to_string());

        Ok(deleted)
    }

    /// Get full metadata for a timeline.
    pub fn get_metadata(&self, timeline: &str) -> ButterflyMetadata {
        let butterfly = self.butterflies.get(timeline).cloned();
        let children = self.get_children(timeline);

        ButterflyMetadata {
            timeline: timeline.to_string(),
            is_butterfly: butterfly.is_some(),
            butterfly,
            children,
        }
    }

    /// List all butterflies.
    pub fn list_all(&self) -> Vec<&Butterfly> {
        self.butterflies.values().collect()
    }

    /// List orphaned butterflies.
    pub fn list_orphaned(&self) -> Vec<&Butterfly> {
        self.butterflies
            .values()
            .filter(|bf| bf.is_orphaned)
            .collect()
    }

    /// Get the full butterfly tree starting from a root timeline.
    pub fn get_tree(&self, root: &str) -> Vec<(String, usize)> {
        let mut result = Vec::new();
        self.collect_tree(root, 0, &mut result);
        result
    }

    fn collect_tree(&self, name: &str, depth: usize, result: &mut Vec<(String, usize)>) {
        result.push((name.to_string(), depth));
        for child in self.get_children(name) {
            self.collect_tree(&child, depth + 1, result);
        }
    }

    /// Put back a record read from storage. Unlike [`create`](Self::create)
    /// it keeps the orphan state, and an orphan is nobody's child: the name
    /// of the parent it lost may since have been given to a new timeline.
    pub fn restore(&mut self, bf: Butterfly) {
        if !bf.is_orphaned {
            self.children
                .entry(bf.parent_name.clone())
                .or_default()
                .push(bf.name.clone());
        }
        self.butterflies.insert(bf.name.clone(), bf);
    }

    /// Every butterfly nested under `name`, each before its own children.
    pub fn descendants(&self, name: &str) -> Vec<String> {
        self.get_tree(name)
            .into_iter()
            .skip(1)
            .map(|(name, _)| name)
            .collect()
    }

    /// `parent` is going away: its children stay, orphaned, each recording
    /// `original_parent`.
    fn orphan_children(&mut self, parent: &str, original_parent: Option<&str>) {
        for child in self.children.remove(parent).unwrap_or_default() {
            if let Some(bf) = self.butterflies.get_mut(&child) {
                bf.is_orphaned = true;
                bf.original_parent = original_parent.map(str::to_string);
            }
        }
    }

    /// Timeline `name` was removed: forget it if it was a butterfly, and
    /// orphan the butterflies made from it either way.
    pub fn timeline_removed(&mut self, name: &str) {
        if self.is_butterfly(name) {
            self.delete(name, false)
                .expect("a butterfly that exists can be deleted");
        } else {
            self.orphan_children(name, None);
        }
    }

    /// Timeline `old` is now called `new`: its own record, its place among
    /// its parent's children, and its children's parent link all follow.
    pub fn timeline_renamed(&mut self, old: &str, new: &str) {
        if let Some(mut bf) = self.butterflies.remove(old) {
            if !bf.is_orphaned
                && let Some(siblings) = self.children.get_mut(&bf.parent_name)
            {
                for sibling in siblings.iter_mut().filter(|s| *s == old) {
                    *sibling = new.to_string();
                }
            }
            bf.name = new.to_string();
            self.butterflies.insert(new.to_string(), bf);
        }
        if let Some(children) = self.children.remove(old) {
            for child in &children {
                if let Some(bf) = self.butterflies.get_mut(child) {
                    bf.parent_name = new.to_string();
                }
            }
            self.children.insert(new.to_string(), children);
        }
    }
}

// -- Storage ------------------------------------------------------------------

impl Butterfly {
    fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "name": self.name,
            "parent_name": self.parent_name,
            "divergence_hash": self.divergence_hash.to_hex(),
            "created_at": self.created_at,
            "is_orphaned": self.is_orphaned,
            "original_parent": self.original_parent,
        }))
        .expect("a JSON object always serializes")
    }

    /// Records written before `created_at` and `original_parent` were stored
    /// read with those empty.
    fn from_json(bytes: &[u8]) -> Result<Self, ButterflyError> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| ButterflyError::Corrupt(e.to_string()))?;
        let text = |key: &str| value.get(key).and_then(|v| v.as_str());
        let missing = |key: &str| ButterflyError::Corrupt(format!("no valid '{key}'"));
        Ok(Self {
            name: text("name").ok_or_else(|| missing("name"))?.to_string(),
            parent_name: text("parent_name")
                .ok_or_else(|| missing("parent_name"))?
                .to_string(),
            divergence_hash: text("divergence_hash")
                .and_then(B3Hash::from_hex)
                .ok_or_else(|| missing("divergence_hash"))?,
            created_at: value
                .get("created_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            is_orphaned: value
                .get("is_orphaned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            original_parent: text("original_parent").map(str::to_string),
        })
    }
}

/// Every butterfly record in the store.
pub fn load(store: &Store) -> Result<ButterflyManager, ButterflyError> {
    let mut mgr = ButterflyManager::new();
    for name in store.list_butterflies()? {
        if let Some(bytes) = store.get_butterfly(&name)? {
            mgr.restore(Butterfly::from_json(&bytes)?);
        }
    }
    Ok(mgr)
}

/// Load the records, apply `change`, and write back what it changed in one
/// store transaction.
fn update<T>(
    store: &Store,
    change: impl FnOnce(&mut ButterflyManager) -> T,
) -> Result<T, ButterflyError> {
    let mut mgr = load(store)?;
    let before: BTreeMap<String, Vec<u8>> = mgr
        .butterflies
        .iter()
        .map(|(name, bf)| (name.clone(), bf.to_json()))
        .collect();
    let out = change(&mut mgr);

    let puts: Vec<(String, Vec<u8>)> = mgr
        .butterflies
        .iter()
        .map(|(name, bf)| (name.clone(), bf.to_json()))
        .filter(|(name, json)| before.get(name) != Some(json))
        .collect();
    let removes: Vec<String> = before
        .keys()
        .filter(|name| !mgr.butterflies.contains_key(*name))
        .cloned()
        .collect();
    if !puts.is_empty() || !removes.is_empty() {
        store.write_butterflies(&puts, &removes)?;
    }
    Ok(out)
}

/// Record `name` as a new butterfly of `parent`. A record already under that
/// name is stale (its timeline is gone, or creating `name` would have failed),
/// and so is any parent link to the name: those children are orphans, not the
/// new butterfly's.
pub fn record_new(
    store: &Store,
    name: &str,
    parent: &str,
    divergence_hash: B3Hash,
) -> Result<Butterfly, ButterflyError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    update(store, |mgr| {
        mgr.timeline_removed(name);
        mgr.create(name, parent, divergence_hash, now)?;
        Ok(mgr.get(name).cloned().expect("just created"))
    })?
}

/// Keep the records in step with a timeline removed by any route.
pub fn on_timeline_removed(store: &Store, name: &str) -> Result<(), ButterflyError> {
    update(store, |mgr| mgr.timeline_removed(name))
}

/// Keep the records in step with a timeline renamed by any route.
pub fn on_timeline_renamed(store: &Store, old: &str, new: &str) -> Result<(), ButterflyError> {
    update(store, |mgr| mgr.timeline_renamed(old, new))
}

// -- Operations ---------------------------------------------------------------

/// Make a butterfly of the current timeline and switch to it. The new
/// timeline starts at the parent's tip, so the working directory already
/// matches it: nothing there changes, and uncommitted work stays put.
pub fn create(repo: &Repo, name: &str) -> Result<Butterfly, ButterflyError> {
    let parent = repo.current_timeline()?;
    repo.create_timeline(name, Some(&parent))?;
    let divergence_hash = match repo.get_timeline_head(&parent)? {
        Some(idx) => repo.get_leaf(idx)?.map_or(B3Hash::ZERO, |leaf| leaf.hash()),
        None => B3Hash::ZERO,
    };
    let bf = record_new(&repo.store, name, &parent, divergence_hash)?;
    repo.switch_timeline(name)?;
    Ok(bf)
}

/// What a sync did.
pub enum Synced {
    /// The other side already had everything: nothing was sealed.
    Nothing,
    Sealed {
        seal: CommitResult,
        /// Files both sides changed in ways that collide, settled by the
        /// layer rules.
        layered: Vec<String>,
        /// Uncommitted work carried through, when the side that received the
        /// sync is checked out and had some.
        carry: Option<CarryReport>,
    },
}

/// Merge butterfly `name`'s sealed work into its parent. Uncommitted work is
/// not sealed work: it stays on the butterfly. `carry` settles collisions
/// between uncommitted work and the result when the parent is checked out
/// (`None` writes conflict markers into those working files).
pub fn up(
    repo: &mut Repo,
    name: &str,
    carry: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Synced, ButterflyError> {
    let bf = syncable(repo, name)?;
    sync(repo, name, &bf.parent_name, name, carry)
}

/// Merge the parent's sealed work into butterfly `name`. `carry` is as for
/// [`up`], for when the butterfly is checked out.
pub fn down(
    repo: &mut Repo,
    name: &str,
    carry: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Synced, ButterflyError> {
    let bf = syncable(repo, name)?;
    sync(repo, &bf.parent_name, name, name, carry)
}

/// The record for `name`, if it is a butterfly with a parent to sync with.
fn syncable(repo: &Repo, name: &str) -> Result<Butterfly, ButterflyError> {
    let bf = load(&repo.store)?
        .get(name)
        .cloned()
        .ok_or_else(|| ButterflyError::NotAButterfly(name.to_string()))?;
    if bf.is_orphaned || !repo.timeline_exists(&bf.parent_name)? {
        return Err(ButterflyError::Orphaned(name.to_string()));
    }
    Ok(bf)
}

/// Fuse `source` into `target`, settling collisions by the layer rules, and
/// move `butterfly`'s divergence point to the source head both now share.
fn sync(
    repo: &mut Repo,
    source: &str,
    target: &str,
    butterfly: &str,
    mut carry: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Synced, ButterflyError> {
    let checked_out = repo.current_timeline()? == target;
    if checked_out {
        if repo.has_merge_in_progress() {
            return Err(ButterflyError::Other(
                "a fuse is in progress. Finish it with 'ivaldi fuse --continue' or \
                 'ivaldi fuse --abort' first."
                    .into(),
            ));
        }
        // Give back anything an interrupted fuse still holds before setting
        // the working directory aside again.
        crate::fuse_op::finish_interrupted(repo, carry.as_deref_mut())?;
    }

    let mut plan = match crate::fuse_op::plan_into(repo, source, target, Strategy::Auto)? {
        Planned::AlreadyFused => return Ok(Synced::Nothing),
        Planned::Plan(plan) => plan,
    };
    let layered = layer(repo, &mut plan)?;
    let source_hash = repo
        .get_leaf(plan.source_head)?
        .ok_or_else(|| ButterflyError::Other(format!("corrupt head on timeline '{source}'")))?
        .hash();

    let (seal, carry) = if checked_out {
        let fused = crate::fuse_op::complete(repo, &plan, carry)?;
        (fused.seal, fused.carry)
    } else {
        (crate::fuse_op::seal_onto(repo, &plan)?, None)
    };
    update(&repo.store, |mgr| {
        mgr.update_divergence(butterfly, source_hash)
    })??;
    Ok(Synced::Sealed {
        seal,
        layered,
        carry,
    })
}

/// Settle every collision in `plan` by the layer rules. Returns the paths.
fn layer(repo: &Repo, plan: &mut FusePlan) -> Result<Vec<String>, ButterflyError> {
    let store = FsStore::new(&repo.cas);
    let (target, source) = (plan.target.clone(), plan.source.clone());
    let labels = Labels {
        mine: &target,
        theirs: &source,
        on_quit: "cancels the sync; nothing has been changed",
    };
    let conflicts = std::mem::take(&mut plan.conflicts);
    for conflict in &conflicts {
        match crate::resolve::resolve_conflict(&store, conflict, labels, &mut LayerResolver) {
            Ok(Some(hash)) => {
                plan.merged_files.insert(conflict.path.clone(), hash);
            }
            Ok(None) => {
                plan.merged_files.remove(&conflict.path);
            }
            Err(Stop::Unresolvable(why)) => {
                return Err(ButterflyError::Other(format!(
                    "{why}\nNothing was changed."
                )));
            }
            Err(Stop::Cancelled) => unreachable!("the layer rules never back out"),
        }
    }
    Ok(conflicts.into_iter().map(|c| c.path).collect())
}

/// Remove butterfly `name`. With `cascade`, everything nested under it goes
/// too; without, its children stay, orphaned. Refused before anything is
/// removed if that would take the checked-out timeline, or, unless `force`
/// discards it, shelved work on any timeline that would go. Returns what was
/// removed, `name` first.
pub fn remove(
    repo: &Repo,
    name: &str,
    cascade: bool,
    force: bool,
) -> Result<Vec<String>, ButterflyError> {
    let mgr = load(&repo.store)?;
    if !mgr.is_butterfly(name) {
        return Err(ButterflyError::NotAButterfly(name.to_string()));
    }
    let mut doomed = vec![name.to_string()];
    if cascade {
        doomed.extend(mgr.descendants(name));
    }
    let current = repo.current_timeline()?;
    if doomed.contains(&current) {
        return Err(ButterflyError::Other(format!(
            "'{current}' is checked out. Switch to a timeline that is not being removed first."
        )));
    }
    if !force {
        repo.ensure_nothing_shelved(&doomed)?;
    }
    // Deepest first, so no removal orphans a butterfly that is about to go
    // anyway. The records follow through `on_timeline_removed`.
    for timeline in doomed.iter().rev() {
        if force {
            repo.remove_timeline_discarding_shelf(timeline)?;
        } else {
            repo.remove_timeline(timeline)?;
        }
    }
    Ok(doomed)
}

/// The layer rules for settling what a sync cannot merge (`docs/butterfly.md`):
/// the incoming side ("theirs": the butterfly going up, the parent coming
/// down) goes on top of a collision, including a file both sides added, and
/// an edit beats a deletion whichever side made it.
pub struct LayerResolver;

impl Resolver for LayerResolver {
    fn region(&mut self, _region: &Region<'_>) -> Result<Resolution, Stop> {
        Ok(Resolution::Theirs)
    }

    fn whole_file(
        &mut self,
        _path: &str,
        why: WholeFile,
        _labels: Labels<'_>,
    ) -> Result<Side, Stop> {
        Ok(match why {
            WholeFile::Binary => Side::Theirs,
            WholeFile::DeletedByTheirs => Side::Mine,
            WholeFile::DeletedByMine => Side::Theirs,
        })
    }
}

impl Default for ButterflyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ButterflyError {
    #[error("butterfly already exists: {0}")]
    AlreadyExists(String),
    #[error("butterfly not found: {0}")]
    NotFound(String),
    #[error("cannot sync orphaned butterfly '{0}': the timeline it was made from is gone")]
    Orphaned(String),
    #[error("'{0}' is not a butterfly")]
    NotAButterfly(String),
    #[error("corrupt butterfly record: {0}")]
    Corrupt(String),
    #[error("{0}")]
    Repo(#[from] RepoError),
    #[error("{0}")]
    FuseOp(#[from] FuseOpError),
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_butterfly() {
        let mut mgr = ButterflyManager::new();
        let hash = B3Hash::digest(b"diverge point");

        mgr.create("experiment", "main", hash, 1700000000).unwrap();

        assert!(mgr.is_butterfly("experiment"));
        assert!(!mgr.is_butterfly("main"));

        let bf = mgr.get("experiment").unwrap();
        assert_eq!(bf.name, "experiment");
        assert_eq!(bf.parent_name, "main");
        assert_eq!(bf.divergence_hash, hash);
        assert!(!bf.is_orphaned);
    }

    #[test]
    fn create_duplicate_fails() {
        let mut mgr = ButterflyManager::new();
        let hash = B3Hash::digest(b"point");

        mgr.create("exp", "main", hash, 0).unwrap();
        let result = mgr.create("exp", "main", hash, 0);
        assert!(matches!(result, Err(ButterflyError::AlreadyExists(_))));
    }

    #[test]
    fn get_parent() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.get_parent("exp"), Some("main"));
        assert_eq!(mgr.get_parent("main"), None);
    }

    #[test]
    fn children_tracking() {
        let mut mgr = ButterflyManager::new();
        mgr.create("feat-a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-b", "main", B3Hash::ZERO, 0).unwrap();

        let children = mgr.get_children("main");
        assert_eq!(children.len(), 2);
        assert!(children.contains(&"feat-a".to_string()));
        assert!(children.contains(&"feat-b".to_string()));
    }

    #[test]
    fn nested_butterflies() {
        let mut mgr = ButterflyManager::new();
        mgr.create("develop", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feature", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub-feature", "feature", B3Hash::ZERO, 0)
            .unwrap();

        assert_eq!(mgr.get_children("main"), vec!["develop"]);
        assert_eq!(mgr.get_children("develop"), vec!["feature"]);
        assert_eq!(mgr.get_children("feature"), vec!["sub-feature"]);
    }

    #[test]
    fn delete_without_cascade_orphans_children() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child-a", "parent-bf", B3Hash::ZERO, 0).unwrap();
        mgr.create("child-b", "parent-bf", B3Hash::ZERO, 0).unwrap();

        let deleted = mgr.delete("parent-bf", false).unwrap();
        assert_eq!(deleted, vec!["parent-bf"]);

        // Children still exist but are orphaned
        assert!(mgr.is_butterfly("child-a"));
        assert!(mgr.get("child-a").unwrap().is_orphaned);
        assert_eq!(
            mgr.get("child-a").unwrap().original_parent,
            Some("main".to_string())
        );

        assert!(mgr.get("child-b").unwrap().is_orphaned);
    }

    #[test]
    fn delete_with_cascade() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child-a", "parent-bf", B3Hash::ZERO, 0).unwrap();
        mgr.create("grandchild", "child-a", B3Hash::ZERO, 0)
            .unwrap();

        let deleted = mgr.delete("parent-bf", true).unwrap();
        assert_eq!(deleted.len(), 3);
        assert!(deleted.contains(&"parent-bf".to_string()));
        assert!(deleted.contains(&"child-a".to_string()));
        assert!(deleted.contains(&"grandchild".to_string()));

        assert!(!mgr.is_butterfly("parent-bf"));
        assert!(!mgr.is_butterfly("child-a"));
        assert!(!mgr.is_butterfly("grandchild"));
    }

    #[test]
    fn delete_nonexistent_fails() {
        let mut mgr = ButterflyManager::new();
        let result = mgr.delete("nope", false);
        assert!(matches!(result, Err(ButterflyError::NotFound(_))));
    }

    #[test]
    fn update_divergence() {
        let mut mgr = ButterflyManager::new();
        let hash1 = B3Hash::digest(b"point1");
        let hash2 = B3Hash::digest(b"point2");

        mgr.create("exp", "main", hash1, 0).unwrap();
        assert_eq!(mgr.get_divergence("exp"), Some(hash1));

        mgr.update_divergence("exp", hash2).unwrap();
        assert_eq!(mgr.get_divergence("exp"), Some(hash2));
    }

    #[test]
    fn get_metadata_butterfly() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "exp", B3Hash::ZERO, 0).unwrap();

        let meta = mgr.get_metadata("exp");
        assert!(meta.is_butterfly);
        assert_eq!(meta.children, vec!["sub"]);
        assert_eq!(meta.butterfly.unwrap().parent_name, "main");
    }

    #[test]
    fn get_metadata_non_butterfly() {
        let mgr = ButterflyManager::new();
        let meta = mgr.get_metadata("main");
        assert!(!meta.is_butterfly);
        assert!(meta.butterfly.is_none());
        assert!(meta.children.is_empty());
    }

    #[test]
    fn list_all() {
        let mut mgr = ButterflyManager::new();
        mgr.create("a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("b", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("c", "a", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.list_all().len(), 3);
    }

    #[test]
    fn list_orphaned() {
        let mut mgr = ButterflyManager::new();
        mgr.create("parent-bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child", "parent-bf", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.list_orphaned().len(), 0);

        mgr.delete("parent-bf", false).unwrap();
        assert_eq!(mgr.list_orphaned().len(), 1);
        assert_eq!(mgr.list_orphaned()[0].name, "child");
    }

    #[test]
    fn get_tree() {
        let mut mgr = ButterflyManager::new();
        mgr.create("develop", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-a", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("feat-b", "develop", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "feat-a", B3Hash::ZERO, 0).unwrap();

        let tree = mgr.get_tree("main");
        // main(0) → develop(1) → feat-a(2) → sub(3), feat-b(2)
        assert_eq!(tree[0], ("main".to_string(), 0));
        assert_eq!(tree[1], ("develop".to_string(), 1));
        // Children order depends on BTreeMap
        assert!(tree.len() == 5);
    }

    #[test]
    fn delete_removes_from_parent_children() {
        let mut mgr = ButterflyManager::new();
        mgr.create("a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("b", "main", B3Hash::ZERO, 0).unwrap();

        assert_eq!(mgr.get_children("main").len(), 2);

        mgr.delete("a", false).unwrap();
        let children = mgr.get_children("main");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], "b");
    }

    #[test]
    fn descendants_lists_every_nested_butterfly_but_not_the_root() {
        let mut mgr = ButterflyManager::new();
        mgr.create("a", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("b", "a", B3Hash::ZERO, 0).unwrap();
        mgr.create("c", "b", B3Hash::ZERO, 0).unwrap();
        mgr.create("d", "a", B3Hash::ZERO, 0).unwrap();

        let mut below = mgr.descendants("a");
        below.sort();
        assert_eq!(below, vec!["b", "c", "d"]);
        assert!(mgr.descendants("c").is_empty());
    }

    #[test]
    fn removing_an_ordinary_parent_orphans_its_butterflies() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "feature", B3Hash::ZERO, 0).unwrap();

        mgr.timeline_removed("feature");

        let bf = mgr.get("exp").unwrap();
        assert!(bf.is_orphaned);
        assert_eq!(bf.parent_name, "feature");
        assert!(mgr.get_children("feature").is_empty());
    }

    #[test]
    fn removing_a_butterfly_timeline_forgets_it() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "exp", B3Hash::ZERO, 0).unwrap();

        mgr.timeline_removed("exp");

        assert!(!mgr.is_butterfly("exp"));
        assert!(mgr.get("sub").unwrap().is_orphaned);
        assert!(mgr.get_children("main").is_empty());
    }

    #[test]
    fn renaming_moves_the_record_and_the_parent_links() {
        let mut mgr = ButterflyManager::new();
        mgr.create("exp", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("sub", "exp", B3Hash::ZERO, 0).unwrap();

        mgr.timeline_renamed("exp", "experiment");
        mgr.timeline_renamed("main", "trunk");

        assert!(!mgr.is_butterfly("exp"));
        assert_eq!(mgr.get_parent("experiment"), Some("trunk"));
        assert_eq!(mgr.get_parent("sub"), Some("experiment"));
        assert_eq!(mgr.get_children("trunk"), vec!["experiment"]);
        assert_eq!(mgr.get_children("experiment"), vec!["sub"]);
        assert!(mgr.get_children("main").is_empty());
    }

    #[test]
    fn a_restored_orphan_is_nobodys_child() {
        let mut mgr = ButterflyManager::new();
        mgr.restore(Butterfly {
            name: "lost".into(),
            parent_name: "gone".into(),
            divergence_hash: B3Hash::ZERO,
            created_at: 0,
            is_orphaned: true,
            original_parent: None,
        });

        assert!(mgr.is_butterfly("lost"));
        assert!(mgr.get_children("gone").is_empty());
    }

    #[test]
    fn records_round_trip_through_json() {
        let bf = Butterfly {
            name: "exp".into(),
            parent_name: "main".into(),
            divergence_hash: B3Hash::digest(b"point"),
            created_at: 1_700_000_000,
            is_orphaned: true,
            original_parent: Some("trunk".into()),
        };

        let back = Butterfly::from_json(&bf.to_json()).unwrap();

        assert_eq!(back.name, bf.name);
        assert_eq!(back.parent_name, bf.parent_name);
        assert_eq!(back.divergence_hash, bf.divergence_hash);
        assert_eq!(back.created_at, bf.created_at);
        assert!(back.is_orphaned);
        assert_eq!(back.original_parent, bf.original_parent);
    }

    #[test]
    fn records_from_before_created_at_still_load() {
        let old = serde_json::to_vec(&serde_json::json!({
            "name": "exp",
            "parent_name": "main",
            "divergence_hash": B3Hash::ZERO.to_hex(),
            "is_orphaned": false,
        }))
        .unwrap();

        let bf = Butterfly::from_json(&old).unwrap();

        assert_eq!(bf.parent_name, "main");
        assert_eq!(bf.created_at, 0);
        assert_eq!(bf.original_parent, None);
    }

    #[test]
    fn a_record_without_a_parent_is_corrupt() {
        let bad = br#"{"name":"exp","divergence_hash":""}"#;
        assert!(matches!(
            Butterfly::from_json(bad),
            Err(ButterflyError::Corrupt(_))
        ));
    }

    #[test]
    fn layer_rules_put_the_incoming_side_on_top_and_keep_edits_over_deletions() {
        let labels = Labels {
            mine: "main",
            theirs: "exp",
            on_quit: "",
        };
        let collision = crate::fuse::Collision {
            base: vec!["old".into()],
            ours: vec!["mine".into()],
            theirs: vec!["theirs".into()],
            ours_line: 1,
        };
        let region = Region {
            path: "f",
            index: 1,
            total: 1,
            collision: &collision,
            before: &[],
            after: &[],
            labels,
        };
        let mut layer = LayerResolver;

        assert_eq!(layer.region(&region), Ok(Resolution::Theirs));
        assert_eq!(
            layer.whole_file("f", WholeFile::Binary, labels),
            Ok(Side::Theirs)
        );
        assert_eq!(
            layer.whole_file("f", WholeFile::DeletedByTheirs, labels),
            Ok(Side::Mine)
        );
        assert_eq!(
            layer.whole_file("f", WholeFile::DeletedByMine, labels),
            Ok(Side::Theirs)
        );
    }

    #[test]
    fn delete_cleans_up_children_mapping() {
        let mut mgr = ButterflyManager::new();
        mgr.create("bf", "main", B3Hash::ZERO, 0).unwrap();
        mgr.create("child", "bf", B3Hash::ZERO, 0).unwrap();

        mgr.delete("bf", true).unwrap();
        assert!(mgr.get_children("bf").is_empty());
    }
}
