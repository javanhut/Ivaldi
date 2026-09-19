//! The fuse *operation*: everything around the merge engine that turns "these
//! two trees merge to this" into a sealed, materialized, undoable fuse.
//!
//! [`crate::fuse`] decides what the merged tree is. This module does the rest,
//! once, for every front end — the CLI and the TUI differ in how they ask
//! about collisions, and in nothing else:
//!
//! 1. [`plan`] — find the merge base, run the engine. Touches nothing.
//! 2. *(front end)* settle `plan.conflicts` into `plan.merged_files`, however
//!    it likes to ask ([`crate::resolve`]). Still nothing touched, so backing
//!    out here is free.
//! 3. [`set_aside`] — snapshot for `ivaldi oops`; park any uncommitted work.
//! 4. [`seal`] — the merge seal, with the source head as a merge parent, and
//!    the merged tree written to the working directory.
//! 5. [`reapply`] — uncommitted work merged back on top ([`crate::carry`]).
//!
//! [`complete`] is 3–5 in one call. A front end that writes conflict markers
//! instead of settling (`fuse --markers`) stops after 3 and leaves the merge
//! open; [`abort`] and the CLI's `--continue` finish that.

use std::collections::BTreeMap;

use crate::carry::CarryReport;
use crate::fsmerkle::FsStore;
use crate::fuse::{Conflict, FuseEngine, Strategy};
use crate::hash::B3Hash;
use crate::repo::{CommitResult, Repo};
use crate::resolve::Resolver;
use crate::workspace::Workspace;

/// A fuse that has been worked out but not performed.
#[derive(Debug)]
pub struct FusePlan {
    pub source: String,
    pub target: String,
    pub source_head: u64,
    pub target_head: u64,
    /// Tree at `target_head`: what the working directory goes back to while
    /// uncommitted work is set aside.
    pub target_tree: B3Hash,
    /// Everything the engine merged by itself. A front end adds its answers
    /// for `conflicts` here before calling [`seal`].
    pub merged_files: BTreeMap<String, B3Hash>,
    /// What the engine could not decide: true collisions only.
    pub conflicts: Vec<Conflict>,
}

pub enum Planned {
    /// The source is already reachable from the target: nothing to do. Also
    /// what makes retrying a fuse that died after its seal a clean no-op.
    AlreadyFused,
    Plan(FusePlan),
}

/// Work out the fuse of `source` into the current timeline. Read-only, apart
/// from merged blobs the engine adds to the CAS.
pub fn plan(repo: &Repo, source: &str, strategy: Strategy) -> Result<Planned, FuseOpError> {
    let target = repo.current_timeline()?;
    let head = |timeline: &str| -> Result<(u64, crate::leaf::Leaf), FuseOpError> {
        let idx = repo
            .get_timeline_head(timeline)?
            .ok_or_else(|| FuseOpError::Other(format!("timeline '{timeline}' has no commits")))?;
        let leaf = repo
            .get_leaf(idx)?
            .ok_or_else(|| FuseOpError::Other(format!("corrupt head on timeline '{timeline}'")))?;
        Ok((idx, leaf))
    };
    let (source_head, source_leaf) = head(source)?;
    let (target_head, target_leaf) = head(&target)?;

    let ws = Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir);
    // The lowest common ancestor's tree is the merge base. Without it every
    // file that differs between the sides would look like a conflict.
    let base_files = match repo.merge_base(target_head, source_head)? {
        Some(base_idx) if base_idx == source_head => return Ok(Planned::AlreadyFused),
        Some(base_idx) => match repo.get_leaf(base_idx)? {
            Some(base_leaf) => ws.list_tree_files(base_leaf.tree_root)?,
            None => BTreeMap::new(),
        },
        None => BTreeMap::new(),
    };
    let ours_files = ws.list_tree_files(target_leaf.tree_root)?;
    let theirs_files = ws.list_tree_files(source_leaf.tree_root)?;

    let store = FsStore::new(&repo.cas);
    let result = FuseEngine::fuse(&store, &base_files, &ours_files, &theirs_files, strategy);
    Ok(Planned::Plan(FusePlan {
        source: source.to_string(),
        target,
        source_head,
        target_head,
        target_tree: target_leaf.tree_root,
        merged_files: result.merged_files,
        conflicts: result.conflicts,
    }))
}

/// Step one of carrying uncommitted work through a fuse (see [`crate::carry`]):
/// snapshot it, park it in the carry file, and put the working directory back
/// to the tip so the fuse has a clean tree. Returns how many changes were set
/// aside; zero means the tree was already clean and nothing was touched.
///
/// The snapshot is recorded for `ivaldi oops` either way — from a clean tree
/// it is what lets `oops` take the merge seal back off the timeline.
///
/// A re-attempt of a fuse left open by `--markers` is exempt: the tree is
/// dirty with the markers that attempt wrote, and it already set the user's
/// own work aside.
pub fn set_aside(repo: &Repo, plan: &FusePlan) -> Result<usize, FuseOpError> {
    if repo.has_merge_in_progress() {
        return Ok(0);
    }
    let snapshot = crate::snapshot::capture(repo, &repo.cas, &format!("fuse {}", plan.source))?;
    crate::snapshot::SnapshotManager::new(&repo.ivaldi_dir).save(&snapshot)?;
    if snapshot.is_clean() {
        return Ok(0);
    }

    // The carry file must be durable before the first working-tree byte is
    // overwritten: from here until it is cleared, it is what any later fuse,
    // `--continue` or `--abort` uses to give the work back.
    crate::snapshot::save_carry(&repo.ivaldi_dir, &snapshot)?;
    crate::failpoint::fail_point("fuse.after_carry_save");

    let mut ws = Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir);
    ws.materialize(plan.target_tree)?;
    ws.staging.clear();
    ws.save()?;

    Ok(snapshot.workspace_changes.len()
        + snapshot.staged_files.len()
        + snapshot.staged_deletions.len())
}

/// The collisions between uncommitted work and what `plan` would make the
/// head — worked out in memory, so a front end can ask about them *before*
/// sealing, while backing out still changes nothing. Call once the plan's own
/// conflicts are settled into `merged_files`, since they decide what the fused
/// files are.
///
/// Give the answers to [`reapply`] / [`complete`] as
/// [`Questions::into_resolver`](crate::resolve::Questions::into_resolver).
pub fn carried_collisions(
    repo: &Repo,
    plan: &FusePlan,
) -> Result<Vec<(String, crate::fuse::Merge3)>, FuseOpError> {
    // As in `set_aside`: a re-attempt of an open fuse carries nothing new.
    if repo.has_merge_in_progress() {
        return Ok(Vec::new());
    }
    // Captured, not saved: this is a look, not the snapshot `oops` restores.
    let dirty = crate::snapshot::capture(repo, &repo.cas, "fuse (preview)")?;
    Ok(crate::carry::preview(
        repo,
        &repo.cas,
        &dirty,
        &plan.merged_files,
    )?)
}

/// Seal `plan.merged_files` as the fuse of source into target, and write the
/// merged tree to the working directory. Every conflict must already be
/// settled into `merged_files`.
pub fn seal(repo: &mut Repo, plan: &FusePlan) -> Result<CommitResult, FuseOpError> {
    let merged_tree = FsStore::new(&repo.cas).build_tree_from_hash_map(&plan.merged_files)?;

    let author = repo.config().author().unwrap_or_else(|| "ivaldi".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let message = format!("Fuse {} into {}", plan.source, plan.target);
    // A raw leaf, so the source head is recorded as a merge parent. That is
    // what makes this a merge rather than a seal that happens to contain the
    // other side's files: the next fuse finds the right base, a sync
    // recognises remote seals as integrated, and uploads keep the topology.
    let mut leaf = crate::leaf::Leaf::new(merged_tree, &plan.target, &author, now, &message);
    leaf.prev_idx = plan.target_head;
    leaf.merge_idxs = vec![plan.source_head];

    // The merged tree nodes (and any blobs a resolution created) must be
    // durable BEFORE the commit record that references them, or power loss
    // could persist a merge seal whose tree is gone.
    repo.cas.flush()?;
    crate::failpoint::fail_point("fuse.before_commit");
    let committed = repo.commit_raw(leaf, &plan.target)?;
    crate::failpoint::fail_point("fuse.after_commit");

    // Without this the seal exists but the working tree still shows
    // pre-merge content — which reads as uncommitted edits undoing the fuse.
    Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir)
        .materialize(merged_tree)
        .map_err(|e| FuseOpError::Other(format!("merge sealed but failed to materialize: {e}")))?;

    // A re-attempt that succeeds settles a merge an earlier one left open.
    repo.clear_merge_state()?;
    // The scratch timeline a diverged sync parks remote seals on has no
    // further use once a merge references them; the seals stay in history.
    if plan.source.starts_with("__sync_") {
        let _ = repo.remove_timeline(&plan.source);
    }
    Ok(committed)
}

/// Step three of the carry: merge set-aside work onto the freshly
/// materialized fused tree. `None` when the fuse carried nothing.
pub fn reapply(
    repo: &Repo,
    source: &str,
    resolver: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Option<CarryReport>, FuseOpError> {
    let Some(carry) = crate::snapshot::load_carry(&repo.ivaldi_dir)? else {
        return Ok(None);
    };
    let report = crate::carry::reapply(
        repo,
        &repo.cas,
        &carry,
        &format!("fused from {source}"),
        resolver,
    )?;
    crate::snapshot::clear_carry(&repo.ivaldi_dir)?;
    Ok(Some(report))
}

/// What [`complete`] did.
pub struct Fused {
    pub seal: CommitResult,
    /// Uncommitted changes carried through (0 for a clean tree).
    pub set_aside: usize,
    pub carry: Option<CarryReport>,
}

/// [`set_aside`], [`seal`] and [`reapply`] in one call, for a front end with
/// nothing to say in between.
pub fn complete(
    repo: &mut Repo,
    plan: &FusePlan,
    resolver: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Fused, FuseOpError> {
    let set_aside = set_aside(repo, plan)?;
    let seal = seal(repo, plan)?;
    crate::failpoint::fail_point("fuse.before_reapply");
    let carry = reapply(repo, &plan.source, resolver)?;
    Ok(Fused {
        seal,
        set_aside,
        carry,
    })
}

/// Give up a fuse left open: put set-aside work back untouched — which also
/// rewrites conflict-marked files back to the tip — and close the merge.
/// Returns whether there was set-aside work to restore.
pub fn abort(repo: &Repo) -> Result<bool, FuseOpError> {
    let restored = match crate::snapshot::load_carry(&repo.ivaldi_dir)? {
        Some(carry) => {
            crate::snapshot::restore(repo, &repo.cas, &carry)?;
            crate::snapshot::clear_carry(&repo.ivaldi_dir)?;
            true
        }
        None => false,
    };
    repo.clear_merge_state()?;
    Ok(restored)
}

/// How [`finish_interrupted`] settled a carry an earlier fuse still owed.
pub enum Recovered {
    /// The merge never happened: the work is back as it was.
    Restored,
    /// The merge seal had landed: the work was re-applied on top of it.
    Reapplied(CarryReport),
}

/// A carry file with no merge in progress means a fuse died between setting
/// work aside and giving it back. Which way to finish depends on whether the
/// merge seal landed: if the head moved, the fuse happened and the work goes
/// on top of it; if not, nothing happened and the work goes back as it was.
///
/// Call before planning a fuse. `None` when there was nothing to recover.
pub fn finish_interrupted(
    repo: &Repo,
    resolver: Option<&mut (dyn Resolver + 'static)>,
) -> Result<Option<Recovered>, FuseOpError> {
    if repo.has_merge_in_progress() {
        return Ok(None); // the carry belongs to that merge; continue/abort settle it
    }
    let Some(carry) = crate::snapshot::load_carry(&repo.ivaldi_dir)? else {
        return Ok(None);
    };
    let timeline = repo.current_timeline()?;
    if timeline != carry.timeline {
        return Err(FuseOpError::Other(format!(
            "an interrupted fuse on timeline '{0}' still holds uncommitted changes. \
             Run 'ivaldi timeline switch {0}' and fuse again there to get them back first.",
            carry.timeline
        )));
    }

    // Whatever is in the working directory now is about to be rewritten; it
    // may include edits made since the crash.
    crate::snapshot::take(repo, &repo.cas, "fuse (recovery)")?;

    let head = repo.get_timeline_head(&timeline)?;
    if head == carry.head {
        crate::snapshot::restore(repo, &repo.cas, &carry)?;
        crate::snapshot::clear_carry(&repo.ivaldi_dir)?;
        return Ok(Some(Recovered::Restored));
    }
    if let Some(idx) = head
        && let Some(leaf) = repo.get_leaf(idx)?
    {
        Workspace::new(&repo.cas, &repo.work_dir, &repo.ivaldi_dir).materialize(leaf.tree_root)?;
    }
    let report = reapply(repo, "the interrupted fuse", resolver)?
        .expect("carry file was just loaded, so there is something to re-apply");
    Ok(Some(Recovered::Reapplied(report)))
}

#[derive(Debug, thiserror::Error)]
pub enum FuseOpError {
    #[error("{0}")]
    Repo(#[from] crate::repo::RepoError),
    #[error("{0}")]
    Workspace(#[from] crate::workspace::WorkspaceError),
    #[error("{0}")]
    Snapshot(#[from] crate::snapshot::SnapshotError),
    #[error("{0}")]
    FsMerkle(#[from] crate::fsmerkle::FsMerkleError),
    #[error("{0}")]
    Cas(#[from] crate::cas::CasError),
    #[error("{0}")]
    Other(String),
}
