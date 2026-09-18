//! History-editing commands: rewind, reverse, oops, discard, undo, pluck.

use super::*;

/// `rewind <seal> [--discard]`: move the timeline head back to an earlier
/// seal. Files are left exactly as they are unless `--discard` is given;
/// staged entries are cleared either way (they were gathered against the
/// old head). The seals after the target are orphaned, not deleted — they
/// stay recoverable via `travel --all`.
pub(super) fn cmd_rewind(args: RewindArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let repo = open_repo()?;

    if repo.has_merge_in_progress() {
        return Err(
            "cannot rewind during a merge. Finish with 'ivaldi fuse --continue' or \
             'ivaldi fuse --abort' first."
                .into(),
        );
    }

    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
    let (target_idx, target_leaf) = repo
        .resolve_seal(&args.seal)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("seal not found: {}", args.seal))?;

    let head_idx = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .ok_or("timeline has no seals")?;
    // Already at the target: skip the head move but still run the staging
    // clear (and --discard materialize) below. A rewind that crashed between
    // moving the head and clearing staging must converge on retry instead of
    // silently keeping stale staging gathered against the old head.
    let already_there = target_idx == head_idx;
    if already_there && !quiet {
        println!("Already at that seal — refreshing staging/working state.");
    }
    if !already_there
        && !repo
            .is_ancestor(target_idx, head_idx)
            .map_err(|e| e.to_string())?
        && !quiet
    {
        println!(
            "note: '{}' is not an earlier seal on this timeline; the seals left behind \
             remain recoverable via 'ivaldi travel --all'",
            args.seal
        );
    }

    // Rewind drops the staging area and, with --discard, the working tree.
    // Both go into a snapshot first, along with the head being left behind.
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    crate::snapshot::take(&repo, &cas, &format!("rewind {}", args.seal))
        .map_err(|e| e.to_string())?;

    if !already_there {
        repo.set_timeline_head(&timeline, target_idx)
            .map_err(|e| e.to_string())?;
    }
    crate::failpoint::fail_point("rewind.after_head");

    {
        let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws_mut.staging.clear();
        ws_mut.save().map_err(|e| e.to_string())?;
    }
    crate::failpoint::fail_point("rewind.after_staging_clear");
    if args.discard {
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.materialize(target_leaf.tree_root)
            .map_err(|e| e.to_string())?;
    }

    if !quiet {
        let name = repo
            .get_seal_name(target_leaf.hash())
            .ok()
            .flatten()
            .unwrap_or_else(|| args.seal.clone());
        println!(
            "Rewound '{}' to seal: {} ({})",
            timeline,
            name,
            target_leaf.hash().short8()
        );
        if args.discard {
            println!("Working directory rewritten to match.");
        } else {
            println!("Your files were left unchanged.");
        }
    }
    Ok(())
}

pub(super) fn cmd_reverse(_args: ReverseArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;

    // Materialize workspace from last seal tree.
    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
    if let Some(head_idx) = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        && let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())?
    {
        let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
        // `reverse` is the one command whose whole purpose is to destroy
        // uncommitted work, which makes it the one most worth being able to
        // take back.
        let snapshot =
            crate::snapshot::capture(&repo, &cas, "reverse --all").map_err(|e| e.to_string())?;
        let discarded = !snapshot.is_clean();
        if discarded {
            crate::snapshot::SnapshotManager::new(&ctx.ivaldi_dir)
                .save(&snapshot)
                .map_err(|e| e.to_string())?;
        }
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.materialize(leaf.tree_root).map_err(|e| e.to_string())?;
        // A crash here leaves stale staging behind; retrying `reverse`
        // converges because it always re-runs both steps.
        crate::failpoint::fail_point("reverse.after_materialize");
        // Clear staging
        let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws_mut.staging.clear();
        ws_mut.save().map_err(|e| e.to_string())?;
        if !quiet {
            println!("Reversed all changes. Working directory restored to last seal.");
            if discarded {
                println!("Changed your mind? 'ivaldi oops' brings them back.");
            }
        }
        return Ok(());
    }
    Err("no seals to restore the working directory from".into())
}

/// `oops`: put the repository back the way it was before the last command
/// that rewrote the working directory.
///
/// The state being left is snapshotted in turn, so `oops` is its own inverse:
/// running it twice is a no-op, and nothing it overwrites is ever lost.
pub(super) fn cmd_oops(args: OopsArgs, quiet: bool) -> Result<(), String> {
    use crate::snapshot::{self, SnapshotManager};

    let ctx = find_repo()?;
    let mgr = SnapshotManager::new(&ctx.ivaldi_dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let describe = |s: &snapshot::Snapshot| {
        let changes = s.workspace_changes.len() + s.staged_files.len() + s.staged_deletions.len();
        let what = if changes == 0 {
            "clean tree".to_string()
        } else {
            format!("{} uncommitted change(s)", changes)
        };
        format!(
            "before '{}' on {} — {}, {}",
            s.command,
            color::timeline(&s.timeline),
            what,
            ivaldi_log::relative_time(s.created_at, now)
        )
    };

    if args.list {
        let all = mgr.list().map_err(|e| e.to_string())?;
        if all.is_empty() {
            println!(
                "No snapshots yet. They are taken automatically before fuse, sync, reverse and rewind."
            );
        }
        for s in &all {
            println!("{:>4}  {}", s.id, describe(s));
        }
        return Ok(());
    }

    let target = match args.id {
        Some(id) => mgr
            .load(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no snapshot {}. See 'ivaldi oops --list'.", id))?,
        None => mgr.latest().map_err(|e| e.to_string())?.ok_or(
            "nothing to undo: no snapshots yet. They are taken automatically before \
             fuse, sync, reverse and rewind.",
        )?,
    };

    let repo = open_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;

    // A conflicted fuse has made no seal yet, so there is nothing to redo:
    // undoing it is aborting it. Only the fuse's own snapshot will do — any
    // other would rewrite the tree underneath a merge that is still open.
    if repo.has_merge_in_progress() {
        if args.id.is_some() || !target.command.starts_with("fuse ") {
            return Err(
                "a fuse is in progress. Finish it with 'ivaldi fuse --continue', or give it \
                 up with 'ivaldi fuse --abort' (or a bare 'ivaldi oops' right after it)."
                    .into(),
            );
        }
        snapshot::restore(&repo, &cas, &target).map_err(|e| e.to_string())?;
        repo.clear_merge_state().map_err(|e| e.to_string())?;
        snapshot::clear_carry(&ctx.ivaldi_dir).map_err(|e| e.to_string())?;
        mgr.remove(target.id).map_err(|e| e.to_string())?;
        if !quiet {
            println!("Fuse aborted. Restored: {}", describe(&target));
        }
        return Ok(());
    }

    // Capture what is about to be replaced before replacing it, but only
    // record it once the restore has succeeded: if the restore is interrupted,
    // `target` must still be the latest snapshot so a retry finishes the job.
    // Undoing X leaves a snapshot labelled as the undo of X; restoring *that*
    // is redoing X, and leaves one labelled X again. The labels alternate
    // instead of nesting, however many times the user flips back and forth.
    let redo_label = match target
        .command
        .strip_prefix("oops (undo of '")
        .and_then(|rest| rest.strip_suffix("')"))
    {
        Some(original) => original.to_string(),
        None => format!("oops (undo of '{}')", target.command),
    };
    let redo = snapshot::capture(&repo, &cas, &redo_label).map_err(|e| e.to_string())?;
    snapshot::restore(&repo, &cas, &target).map_err(|e| e.to_string())?;
    crate::failpoint::fail_point("oops.before_redo_save");
    mgr.save(&redo).map_err(|e| e.to_string())?;
    mgr.remove(target.id).map_err(|e| e.to_string())?;
    // If that fuse died still owing its set-aside work, this just paid it.
    if let Some(carry) = snapshot::load_carry(&ctx.ivaldi_dir).map_err(|e| e.to_string())?
        && carry.created_at == target.created_at
        && carry.command == target.command
    {
        snapshot::clear_carry(&ctx.ivaldi_dir).map_err(|e| e.to_string())?;
    }

    if !quiet {
        println!("Restored: {}", describe(&target));
        if redo.head != target.head
            && let Some(left) = redo.head
            && let Some(leaf) = repo.get_leaf(left).map_err(|e| e.to_string())?
        {
            let name = repo
                .get_seal_name(leaf.hash())
                .ok()
                .flatten()
                .unwrap_or_else(|| leaf.hash().short8());
            // Undo steps the head back and orphans the seal it leaves; redo
            // steps forward onto that seal again, leaving nothing behind.
            let stepped_back = match target.head {
                Some(to) => repo.is_ancestor(to, left).map_err(|e| e.to_string())?,
                None => true,
            };
            if stepped_back {
                println!(
                    "  timeline head moved back; seal '{}' is kept, just no longer the head",
                    name
                );
            } else {
                println!("  timeline head moved forward again from '{}'", name);
            }
        }
        println!("Run 'ivaldi oops' again to redo.");
    }
    Ok(())
}

pub(super) fn cmd_discard(args: DiscardArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let mut ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if args.files.is_empty() {
        ws.staging.clear();
        if !quiet {
            println!("All files ungathered");
        }
    } else {
        for file in &args.files {
            if ws.staging.unstage(file) && !quiet {
                println!("  ungathered: {}", file);
            }
        }
    }
    ws.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Shared preamble for undo/pluck: resolve the target seal and refuse in
/// states where a three-way apply would be unsafe or ambiguous.
pub(super) fn resolve_for_pick(
    repo: &Repo,
    ws: &Workspace<'_>,
    seal_query: &str,
    verb: &str,
) -> Result<(u64, crate::leaf::Leaf), String> {
    if repo.has_merge_in_progress() {
        return Err(format!(
            "cannot {} during a merge. Finish with 'ivaldi fuse --continue' or \
             'ivaldi fuse --abort' first.",
            verb
        ));
    }
    if !ws.staging.is_empty() {
        return Err(format!(
            "cannot {} with staged changes. Seal them first or unstage with 'ivaldi discard'.",
            verb
        ));
    }
    let (idx, leaf) = repo
        .resolve_seal(seal_query)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("seal not found: {}", seal_query))?;
    if leaf.is_merge() {
        return Err(format!(
            "cannot {} a merge seal yet (no way to choose which parent's side to keep)",
            verb
        ));
    }
    Ok((idx, leaf))
}

/// Tree file-map of a leaf's parent (empty when the leaf is a timeline's
/// first seal).
pub(super) fn parent_tree_files_of(
    repo: &Repo,
    store: &crate::fsmerkle::FsStore<'_>,
    leaf: &crate::leaf::Leaf,
) -> Result<std::collections::BTreeMap<String, crate::hash::B3Hash>, String> {
    let parent_tree = if leaf.has_parent() {
        repo.get_leaf(leaf.prev_idx)
            .map_err(|e| e.to_string())?
            .map(|l| l.tree_root)
    } else {
        None
    };
    crate::pick::tree_files(store, parent_tree)
}

/// Finish a successful undo/pluck: report or materialize.
pub(super) fn finish_pick(
    outcome: crate::pick::ApplyOutcome,
    cas: &FileCas,
    ctx: &RepoContext,
    verb: &str,
    quiet: bool,
) -> Result<(), String> {
    use crate::pick::ApplyOutcome;
    match outcome {
        ApplyOutcome::Conflicts(paths) => {
            let mut msg = format!("{} conflicts with other changes in:\n", verb);
            for p in &paths {
                msg.push_str(&format!("  {}\n", p));
            }
            msg.push_str("Resolve by editing the files manually and sealing, nothing was changed.");
            Err(msg)
        }
        ApplyOutcome::NoChanges => {
            if !quiet {
                println!("{} produced no changes — nothing to seal.", verb);
            }
            Ok(())
        }
        ApplyOutcome::Applied(result) => {
            // Reflect the new head in the working directory.
            let repo = open_repo()?;
            if let Some(leaf) = repo.get_leaf(result.index).map_err(|e| e.to_string())? {
                let ws = Workspace::new(cas, &ctx.work_dir, &ctx.ivaldi_dir);
                let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
                ws.materialize_with_ignore(leaf.tree_root, &ignore_cache)
                    .map_err(|e| e.to_string())?;
            }
            if !quiet {
                println!(
                    "Created seal: {} ({})",
                    result.seal_name,
                    result.hash.short8()
                );
            }
            Ok(())
        }
    }
}

pub(super) fn cmd_undo(args: UndoArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mut repo = open_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    let (_idx, leaf) = resolve_for_pick(&repo, &ws, &args.seal, "undo")?;
    let store = crate::fsmerkle::FsStore::new(&cas);

    // undo: base = the seal's tree, theirs = its parent's tree.
    let base = crate::pick::tree_files(&store, Some(leaf.tree_root))?;
    let theirs = parent_tree_files_of(&repo, &store, &leaf)?;

    let first_line = leaf.message.lines().next().unwrap_or("").to_string();
    let seal_label = repo
        .get_seal_name(leaf.hash())
        .ok()
        .flatten()
        .unwrap_or_else(|| args.seal.clone());
    let message = args.m.clone().unwrap_or_else(|| {
        format!(
            "Undo \"{}\"\n\nThis undoes seal {} ({}).",
            first_line,
            seal_label,
            leaf.hash().short8()
        )
    });

    let cfg = repo.config();
    let author = cfg.author()
        .ok_or("user.name and user.email not configured. Run:\n  ivaldi config --set user.name \"Your Name\"\n  ivaldi config --set user.email \"you@example.com\"")?;

    let outcome = crate::pick::three_way_seal(&mut repo, &cas, &base, &theirs, &author, &message)?;
    drop(repo);
    finish_pick(outcome, &cas, &ctx, "undo", quiet)
}

pub(super) fn cmd_pluck(args: PluckArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mut repo = open_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    let (_idx, leaf) = resolve_for_pick(&repo, &ws, &args.seal, "pluck")?;
    let store = crate::fsmerkle::FsStore::new(&cas);

    // pluck: base = the seal's parent tree, theirs = the seal's tree.
    let base = parent_tree_files_of(&repo, &store, &leaf)?;
    let theirs = crate::pick::tree_files(&store, Some(leaf.tree_root))?;

    let seal_label = repo
        .get_seal_name(leaf.hash())
        .ok()
        .flatten()
        .unwrap_or_else(|| args.seal.clone());
    let message = args.m.clone().unwrap_or_else(|| {
        format!(
            "{}\n\n(plucked from {} {})",
            leaf.message,
            seal_label,
            leaf.hash().short8()
        )
    });

    let cfg = repo.config();
    let author = cfg.author()
        .ok_or("user.name and user.email not configured. Run:\n  ivaldi config --set user.name \"Your Name\"\n  ivaldi config --set user.email \"you@example.com\"")?;

    let outcome = crate::pick::three_way_seal(&mut repo, &cas, &base, &theirs, &author, &message)?;
    drop(repo);
    finish_pick(outcome, &cas, &ctx, "pluck", quiet)
}
