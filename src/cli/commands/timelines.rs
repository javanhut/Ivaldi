//! Timeline commands: switch machinery, timeline management, travel.

use super::*;

/// Refuse to run while an interrupted timeline switch needs recovery.
/// Called by mutating commands (except `timeline switch`, which handles
/// resume itself). Read-only commands stay usable for orientation.
pub(super) fn ensure_no_interrupted_switch(ivaldi_dir: &std::path::Path) -> Result<(), String> {
    match crate::switch_journal::load(ivaldi_dir) {
        Ok(None) => Ok(()),
        Ok(Some(j)) => Err(format!(
            "an interrupted timeline switch from '{}' to '{}' needs recovery.\n\
             Run 'ivaldi timeline switch {}' to complete it, or 'ivaldi timeline switch {}' to roll back.",
            j.from, j.to, j.to, j.from
        )),
        Err(e) => Err(format!(
            "cannot read .ivaldi/{}: {}.\n\
             Inspect (and if invalid, delete) that file, then retry.",
            crate::switch_journal::JOURNAL_FILE,
            e
        )),
    }
}

/// Switch timelines with crash-recovery journaling.
///
/// Ordering: shelve the current timeline's dirty state (and flush the CAS —
/// the shelf holds the only copies), write the journal, then do the
/// destructive-but-idempotent steps (HEAD rewrite, materialize, staging
/// clear + shelf restore), clear the journal, and only then remove the
/// restored shelf. A crash before the journal leaves the worktree and
/// staging intact (retry re-captures); a crash after it is recovered by
/// re-running the switch toward `to` (complete) or `from` (roll back), and
/// every replayed step draws from the still-present shelves, so no window
/// loses shelved or staged content.
pub(crate) fn do_timeline_switch(
    work_dir: &std::path::Path,
    ivaldi_dir: &std::path::Path,
    target: &str,
    quiet: bool,
) -> Result<(), String> {
    use crate::shelf::{Shelf, ShelfManager, WorkspaceChange};
    use crate::switch_journal::{self, SwitchJournal};

    let repo = Repo::open(work_dir).map_err(|e| e.to_string())?;
    let current = repo.current_timeline().unwrap_or_default();
    let cas = FileCas::new(ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ignore_cache = ignore::load_pattern_cache(work_dir);
    let shelf_mgr = ShelfManager::new(ivaldi_dir);

    fn change_counts(changes: &[WorkspaceChange], staged: usize) -> Vec<String> {
        let mut counts = (0usize, 0usize, 0usize);
        for c in changes {
            match c {
                WorkspaceChange::Modified { .. } => counts.0 += 1,
                WorkspaceChange::Untracked { .. } => counts.1 += 1,
                WorkspaceChange::Deleted { .. } => counts.2 += 1,
            }
        }
        let mut summary = Vec::new();
        if counts.0 > 0 {
            summary.push(format!("{} modified", counts.0));
        }
        if counts.1 > 0 {
            summary.push(format!("{} untracked", counts.1));
        }
        if counts.2 > 0 {
            summary.push(format!("{} deleted", counts.2));
        }
        if staged > 0 {
            summary.push(format!("{} staged", staged));
        }
        summary
    }

    // Finish a switch to `dest`: HEAD rewrite, materialize, shelf restore.
    // Every step is idempotent, so this is safe to replay on resume. The
    // dest shelf is deliberately NOT removed here: while the journal exists
    // the shelf is the authoritative source of the restored staging entries,
    // so a crash anywhere inside `finish` (even right after `save`) replays
    // against an intact shelf. The caller removes it only after the journal
    // is cleared; a leftover shelf is harmless — the next switch away from
    // `dest` rewrites or removes it.
    let finish = |dest: &str| -> Result<Vec<String>, String> {
        repo.switch_timeline(dest).map_err(|e| e.to_string())?;

        if let Some(idx) = repo.get_timeline_head(dest).map_err(|e| e.to_string())?
            && let Some(leaf) = repo.get_leaf(idx).map_err(|e| e.to_string())?
        {
            let ws_mat = Workspace::new(&cas, work_dir, ivaldi_dir);
            ws_mat
                .materialize_with_ignore(leaf.tree_root, &ignore_cache)
                .map_err(|e| format!("failed to materialize timeline: {}", e))?;
        }
        crate::failpoint::fail_point("switch.after_materialize");

        let mut restored_summary = Vec::new();
        let mut ws_mut = Workspace::new(&cas, work_dir, ivaldi_dir);
        ws_mut.staging.clear();
        if let Ok(Some(shelf)) = shelf_mgr.load_shelf(dest) {
            if !shelf.workspace_changes.is_empty() {
                ws_mut
                    .apply_changes(&shelf.workspace_changes)
                    .map_err(|e| format!("failed to restore shelved changes: {}", e))?;
            }
            for (path, hash) in &shelf.staged_files {
                ws_mut.staging.stage(path, *hash);
            }
            restored_summary = change_counts(&shelf.workspace_changes, shelf.staged_files.len());
        }
        ws_mut.save().map_err(|e| e.to_string())?;
        Ok(restored_summary)
    };

    // ---- Resume / rollback of an interrupted switch ----
    if let Some(j) = switch_journal::load(ivaldi_dir).map_err(|e| {
        format!(
            "cannot read .ivaldi/{}: {}. Inspect (and if invalid, delete) that file, then retry.",
            switch_journal::JOURNAL_FILE,
            e
        )
    })? {
        if target != j.to && target != j.from {
            return Err(format!(
                "an interrupted timeline switch from '{}' to '{}' needs recovery.\n\
                 Run 'ivaldi timeline switch {}' to complete it, or 'ivaldi timeline switch {}' to roll back,\n\
                 before switching elsewhere.",
                j.from, j.to, j.to, j.from
            ));
        }
        // Do NOT re-capture the worktree: it is mid-transition; the source
        // timeline's dirty state is already in its shelf.
        if !quiet {
            if target == j.to {
                println!("Completing interrupted switch '{}' → '{}'", j.from, j.to);
            } else {
                println!("Rolling back interrupted switch '{}' → '{}'", j.from, j.to);
            }
        }
        let restored_summary = finish(target)?;
        crate::failpoint::fail_point("switch.before_journal_clear");
        switch_journal::clear(ivaldi_dir).map_err(|e| e.to_string())?;
        crate::failpoint::fail_point("switch.before_shelf_remove");
        shelf_mgr.remove_shelf(target).ok();
        if !quiet {
            println!("Switched to timeline: {}", target);
            if !restored_summary.is_empty() {
                println!(
                    "Restored shelved changes for '{}': {}",
                    target,
                    restored_summary.join(", ")
                );
            }
        }
        return Ok(());
    }

    if current == target {
        if !quiet {
            println!("Already on timeline: {}", target);
        }
        return Ok(());
    }

    // Verify target exists before any side effects
    let target_head_idx = repo.get_timeline_head(target).map_err(|e| e.to_string())?;
    if target_head_idx.is_none() {
        let ref_path =
            crate::refname::timeline_ref_path(ivaldi_dir, target).map_err(|e| e.to_string())?;
        if !ref_path.exists() {
            return Err(format!("timeline '{}' not found", target));
        }
    }

    // ---- Auto-shelve: capture everything dirty about `current` ----
    //
    // Staging + working-tree changes (Modified, Untracked, Deleted) go into
    // a shelf keyed by `current`. This must happen BEFORE materialize, which
    // rewrites the working tree to look like the target timeline.
    let mut shelved_summary: Vec<String> = Vec::new();
    let shelf_saved;
    {
        let ws = Workspace::new(&cas, work_dir, ivaldi_dir);
        let current_tree = repo
            .get_timeline_head(&current)
            .map_err(|e| e.to_string())?
            .and_then(|idx| repo.get_leaf(idx).ok().flatten())
            .map(|l| l.tree_root);
        let workspace_changes = ws
            .capture_changes(current_tree, &ignore_cache)
            .map_err(|e| format!("failed to auto-shelve: {}", e))?;

        let mut staged = std::collections::BTreeMap::new();
        for (path, hash) in ws.staging.staged_files() {
            staged.insert(path.clone(), *hash);
        }

        if !staged.is_empty() || !workspace_changes.is_empty() {
            shelved_summary = change_counts(&workspace_changes, staged.len());
            let shelf = Shelf {
                timeline: current.clone(),
                staged_files: staged,
                workspace_changes,
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            };
            shelf_mgr.save_shelf(&shelf).map_err(|e| e.to_string())?;
            shelf_saved = true;
        } else {
            // No dirty state — clear any stale shelf so we don't
            // reapply stale changes if the user switches back.
            shelf_mgr.remove_shelf(&current).ok();
            shelf_saved = false;
        }
    }

    // The shelf references blobs that capture_changes just wrote — the CAS
    // now holds the only copies of the user's uncommitted content, and
    // materialize is about to destroy the working-tree copies.
    crate::failpoint::fail_point("switch.after_shelf_save");
    cas.flush().map_err(|e| e.to_string())?;

    // ---- Journal, then the idempotent destructive steps ----
    //
    // The journal must land BEFORE staging is touched: everything after it
    // (including the staging clear inside `finish`) is replayed on resume
    // from the shelves, so no window can lose the staged entries. (Clearing
    // staging before the journal would let a crash + retry re-capture with
    // an empty staging area and overwrite the shelf, dropping staged files
    // whose content diverged from the working tree.) Until `finish` saves,
    // the staging file still names `current`'s entries — harmless, because
    // the journal blocks every mutating command until the switch resumes.
    switch_journal::write(
        ivaldi_dir,
        &SwitchJournal {
            from: current.clone(),
            to: target.to_string(),
            shelf_saved,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        },
    )
    .map_err(|e| e.to_string())?;
    crate::failpoint::fail_point("switch.after_journal");

    let restored_summary = finish(target)?;
    crate::failpoint::fail_point("switch.before_journal_clear");
    switch_journal::clear(ivaldi_dir).map_err(|e| e.to_string())?;
    crate::failpoint::fail_point("switch.before_shelf_remove");
    shelf_mgr.remove_shelf(target).ok();

    if !quiet {
        if !shelved_summary.is_empty() {
            println!(
                "Auto-shelved on '{}': {}",
                current,
                shelved_summary.join(", ")
            );
        }
        println!("Switched to timeline: {}", target);
        if !restored_summary.is_empty() {
            println!(
                "Restored shelved changes for '{}': {}",
                target,
                restored_summary.join(", ")
            );
        }
    }
    Ok(())
}

pub(super) fn cmd_timeline(args: TimelineArgs, quiet: bool) -> Result<(), String> {
    match args.command {
        TimelineCommands::Create(create_args) => {
            let repo = open_repo()?;
            repo.create_timeline(&create_args.name, create_args.from.as_deref())
                .map_err(|e| e.to_string())?;
            repo.switch_timeline(&create_args.name)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!("Created timeline: {}", color::timeline(&create_args.name));
                if let Some(from) = &create_args.from {
                    println!("  from: {}", from);
                }
                println!(
                    "Switched to timeline: {}",
                    color::timeline(&create_args.name)
                );
            }
            Ok(())
        }
        TimelineCommands::Switch(switch_args) => {
            let ctx = find_repo()?;
            do_timeline_switch(&ctx.work_dir, &ctx.ivaldi_dir, &switch_args.name, quiet)
        }
        TimelineCommands::List(list_args) => {
            let repo = open_repo()?;
            let current = repo.current_timeline().unwrap_or_default();
            let timelines = repo.list_timelines().map_err(|e| e.to_string())?;

            if list_args.json {
                let out: Vec<json::TimelineJson> = if timelines.is_empty() {
                    vec![json::TimelineJson {
                        name: current.clone(),
                        current: true,
                    }]
                } else {
                    timelines
                        .iter()
                        .map(|(name, _)| json::TimelineJson {
                            name: name.clone(),
                            current: name == &current,
                        })
                        .collect()
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
                );
            } else if timelines.is_empty() {
                println!("* {}", current);
            } else {
                for (name, _) in &timelines {
                    let marker = if name == &current { "*" } else { " " };
                    println!("{} {}", marker, name);
                }
            }
            Ok(())
        }
        TimelineCommands::Remove(remove_args) => {
            let repo = open_repo()?;
            repo.remove_timeline(&remove_args.name)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!("Removed timeline: {}", remove_args.name);
            }
            Ok(())
        }
        TimelineCommands::Rename(rename_args) => {
            let repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;

            // Accept three forms:
            //   tl rename NEW              → rename current to NEW
            //   tl rename OLD NEW          → rename OLD to NEW
            //   tl rename OLD to NEW       → same as above with `to` connector
            let (old, new) = match rename_args.names.as_slice() {
                [new] => (current.clone(), new.clone()),
                [old, new] => (old.clone(), new.clone()),
                [old, mid, new] if mid.eq_ignore_ascii_case("to") => (old.clone(), new.clone()),
                [_, mid, _] => {
                    return Err(format!(
                        "expected `tl rename OLD to NEW` (got `{}` between names)",
                        mid
                    ));
                }
                _ => return Err("usage: tl rename [OLD [to]] NEW".into()),
            };

            repo.rename_timeline(&old, &new)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!(
                    "Renamed timeline: {} → {}",
                    color::dim(&old),
                    color::timeline(&new)
                );
            }
            Ok(())
        }
        TimelineCommands::Butterfly(bf_args) => {
            let ctx = find_repo()?;
            cmd_butterfly(bf_args, &ctx, quiet)
        }
    }
}

pub(super) fn cmd_travel(args: TravelArgs) -> Result<(), String> {
    use crate::tui::travel::{TravelAction, run_travel};

    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());

    let entries = if args.all {
        // Walk every leaf in the MMR — useful when the head was welded
        // and most seals are orphaned from the current timeline.
        repo.walk_all_leaves().map_err(|e| e.to_string())?
    } else {
        repo.walk_history(&timeline).map_err(|e| e.to_string())?
    };

    if entries.is_empty() {
        return Err(format!("no commits on timeline '{}'", timeline));
    }

    let action = run_travel(entries, &timeline, args.search).map_err(|e| e.to_string())?;

    match action {
        TravelAction::Diverge {
            seal_index,
            new_timeline,
        } => {
            repo.create_timeline(&new_timeline, Some(&timeline))
                .map_err(|e| e.to_string())?;
            repo.switch_timeline(&new_timeline)
                .map_err(|e| e.to_string())?;
            println!(
                "Created timeline '{}' from seal at index {}",
                new_timeline, seal_index
            );
            println!("Switched to timeline '{}'", new_timeline);
        }
        TravelAction::Overwrite { seal_index } => {
            if let Some(_leaf) = repo.get_leaf(seal_index).map_err(|e| e.to_string())? {
                // Update timeline head to this seal
                repo.store
                    .set_timeline_head(&timeline, seal_index)
                    .map_err(|e| e.to_string())?;
                // Staged entries were gathered against the old head; a later
                // seal would silently commit old-head content onto the moved
                // head, so clear them with the same ordering `rewind` uses.
                let ctx = find_repo()?;
                let cas =
                    FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
                let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws_mut.staging.clear();
                ws_mut.save().map_err(|e| e.to_string())?;
                println!(
                    "Timeline '{}' moved back to seal at index {}",
                    timeline, seal_index
                );
            }
        }
        TravelAction::Cancel => {
            println!("Cancelled.");
        }
    }
    Ok(())
}
