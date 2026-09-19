//! Merge-family commands: butterfly, fuse, weld.

use super::*;

pub(super) fn cmd_butterfly(
    args: ButterflyArgs,
    _ctx: &RepoContext,
    quiet: bool,
) -> Result<(), String> {
    match args.command {
        ButterflyCommands::Create(create_args) => {
            let repo = open_repo()?;
            let parent = repo.current_timeline().unwrap_or_else(|_| "main".into());

            // Store butterfly metadata
            let divergence_hash = repo
                .get_timeline_head(&parent)
                .map_err(|e| e.to_string())?
                .and_then(|idx| repo.get_leaf(idx).ok().flatten())
                .map(|l| l.hash())
                .unwrap_or(crate::hash::B3Hash::ZERO);

            repo.create_timeline(&create_args.name, Some(&parent))
                .map_err(|e| e.to_string())?;
            repo.store_butterfly_meta(&create_args.name, &parent, divergence_hash)
                .map_err(|e| e.to_string())?;
            repo.switch_timeline(&create_args.name)
                .map_err(|e| e.to_string())?;

            if !quiet {
                println!(
                    "Creating butterfly timeline '{}' from '{}'",
                    create_args.name, parent
                );
                println!("Switched to butterfly timeline");
            }
            Ok(())
        }
        ButterflyCommands::Up => {
            let mut repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;
            let result = repo
                .butterfly_sync_up(&current)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!("Synced butterfly '{}' up to parent", current);
                println!(
                    "  Parent updated: {} ({})",
                    result.seal_name,
                    result.hash.short8()
                );
            }
            Ok(())
        }
        ButterflyCommands::Down => {
            let mut repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;
            let result = repo
                .butterfly_sync_down(&current)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!("Synced butterfly '{}' down from parent", current);
                println!(
                    "  Butterfly updated: {} ({})",
                    result.seal_name,
                    result.hash.short8()
                );
            }
            Ok(())
        }
        ButterflyCommands::Remove(remove_args) => {
            let repo = open_repo()?;
            repo.remove_timeline(&remove_args.name)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!("Removed butterfly '{}'", remove_args.name);
            }
            Ok(())
        }
    }
}

pub(super) fn cmd_fuse(args: FuseArgs, quiet: bool) -> Result<(), String> {
    use crate::repo::MergeState;

    let mut repo = open_repo()?;

    if args.abort {
        if repo.has_merge_in_progress() {
            let restored = crate::fuse_op::abort(&repo).map_err(|e| e.to_string())?;
            if !quiet {
                println!("Merge aborted.");
                if restored {
                    println!("Your uncommitted changes are back as they were.");
                }
            }
        } else {
            return Err("no merge in progress".into());
        }
        return Ok(());
    }

    let prefer = args
        .prefer
        .as_deref()
        .map(str::parse::<crate::resolve::Prefer>)
        .transpose()?;

    if args.continue_merge {
        return continue_merge(&mut repo, prefer, quiet);
    }

    // A merge in progress only blocks *bare* re-invocations. Naming a source
    // again is a deliberate re-attempt — usually the `--strategy=theirs`
    // escape hatch printed with the conflicts — so let it through and let the
    // new outcome replace the recorded merge state.
    if repo.has_merge_in_progress() && args.source.is_none() {
        return Err("merge already in progress. Use --continue or --abort.".into());
    }

    let source = args
        .source
        .as_deref()
        .ok_or("source timeline required. Usage: ivaldi fuse <source> to <target>")?;
    let strategy = args.strategy.parse::<Strategy>().map_err(|_| {
        format!(
            "unknown strategy: {}. Options: auto, ours, theirs, union, base",
            args.strategy
        )
    })?;

    // Uncommitted work has no side in a merge of seals, and the fused tree
    // written over the working directory would erase it. Finish anything an
    // interrupted fuse still owes before starting another.
    let mut resolver = make_resolver(prefer);
    if let Some(recovered) = crate::fuse_op::finish_interrupted(&repo, resolver.as_deref_mut())
        .map_err(|e| e.to_string())?
        && !quiet
    {
        println!("Recovered uncommitted changes from an interrupted fuse.");
        if let crate::fuse_op::Recovered::Reapplied(report) = &recovered {
            print_carry_report(report);
        }
    }

    let mut plan = match crate::fuse_op::plan(&repo, source, strategy).map_err(|e| e.to_string())? {
        crate::fuse_op::Planned::Plan(plan) => plan,
        crate::fuse_op::Planned::AlreadyFused => {
            if !quiet {
                println!(
                    "Timeline '{}' is already fused into '{}' — nothing to do.",
                    source,
                    repo.current_timeline().map_err(|e| e.to_string())?
                );
            }
            return Ok(());
        }
    };
    let target = plan.target.clone();
    let store = crate::fsmerkle::FsStore::new(&repo.cas);

    // What the engine could not decide has no right answer in the text. Put
    // it to a resolver now, in memory: until every collision is settled,
    // nothing — working tree, staging, history — has been touched, so backing
    // out is free.
    if !plan.conflicts.is_empty() && !args.markers {
        let Some(resolver) = resolver.as_deref_mut() else {
            return Err(unattended_refusal(&store, &plan.conflicts, source));
        };
        let labels = crate::resolve::Labels {
            mine: &target,
            theirs: source,
            on_quit: "cancels the fuse; nothing has been changed",
        };
        for conflict in &plan.conflicts {
            match crate::resolve::resolve_conflict(&store, conflict, labels, resolver) {
                Ok(Some(hash)) => {
                    plan.merged_files.insert(conflict.path.clone(), hash);
                }
                Ok(None) => {
                    plan.merged_files.remove(&conflict.path);
                }
                Err(crate::resolve::Stop::Cancelled) => {
                    return Err("fuse cancelled — nothing was changed".into());
                }
                Err(crate::resolve::Stop::Unresolvable(why)) => {
                    return Err(format!("{why}\nNothing was changed."));
                }
            }
        }
        if !quiet {
            println!("Settled {} conflicted file(s).", plan.conflicts.len());
        }
    }
    let settled = plan.conflicts.is_empty() || !args.markers;

    // With the fused files now known, so is whether uncommitted work collides
    // with them. Ask about that too while backing out is still free; the
    // answers are replayed when the work is merged back on top. With nobody
    // to ask, those files get conflict markers instead — harmless, since they
    // are uncommitted working files and no merge is left open.
    let mut carried_answers = None;
    if settled && let Some(resolver) = resolver.as_deref_mut() {
        let collisions =
            crate::fuse_op::carried_collisions(&repo, &plan).map_err(|e| e.to_string())?;
        let mut questions = crate::resolve::Questions::from_merges(collisions);
        if !questions.is_empty() {
            let theirs = format!("fused from {source}");
            let labels = crate::resolve::Labels {
                mine: crate::carry::MINE_LABEL,
                theirs: &theirs,
                on_quit: "cancels the fuse; nothing has been changed",
            };
            match questions.ask(resolver, labels) {
                Ok(()) => carried_answers = Some(questions.into_resolver()),
                Err(crate::resolve::Stop::Cancelled) => {
                    return Err("fuse cancelled — nothing was changed".into());
                }
                Err(crate::resolve::Stop::Unresolvable(why)) => {
                    return Err(format!("{why}\nNothing was changed."));
                }
            }
        }
    }

    let set_aside = crate::fuse_op::set_aside(&repo, &plan).map_err(|e| e.to_string())?;
    if set_aside > 0 && !quiet {
        println!(
            "Carrying {} uncommitted change(s) through the fuse...",
            set_aside
        );
    }

    if settled {
        let sealed = crate::fuse_op::seal(&mut repo, &plan).map_err(|e| e.to_string())?;
        if !quiet {
            println!("[OK] Merge completed successfully!");
            println!(
                "  Merge seal: {} ({})",
                sealed.seal_name,
                sealed.hash.short8()
            );
        }
        crate::failpoint::fail_point("fuse.before_reapply");
        let replay = carried_answers
            .as_mut()
            .map(|r| r as &mut (dyn crate::resolve::Resolver + 'static));
        let report = crate::fuse_op::reapply(&repo, source, replay)
            .map_err(|e| carry_failure(&e.to_string(), source))?;
        if !quiet {
            if let Some(report) = &report {
                print_carry_report(report);
            }
            println!("  Not what you wanted? 'ivaldi oops' undoes the whole fuse.");
        }
    } else {
        // Save merge state with conflicts
        let conflict_paths: Vec<String> = plan.conflicts.iter().map(|c| c.path.clone()).collect();
        let state = MergeState {
            source_timeline: source.to_string(),
            target_timeline: target.clone(),
            strategy: args.strategy.clone(),
            conflicts: conflict_paths.clone(),
        };
        repo.save_merge_state(&state).map_err(|e| e.to_string())?;
        crate::failpoint::fail_point("fuse.after_merge_state");

        // Put both sides in front of the user, in the files themselves.
        let (marked, skipped) = crate::fuse::write_conflict_markers(
            &store,
            &repo.work_dir,
            &plan.conflicts,
            &target,
            source,
        );

        println!("[CONFLICTS] Merge conflicts detected:\n");
        for path in &marked {
            println!("  CONFLICT: {} (conflict markers written)", path);
        }
        for path in &skipped {
            println!("  CONFLICT: {} (binary — choose a side)", path);
        }
        println!("\nResolution options:");
        println!("  edit the marked files, then 'ivaldi fuse --continue'");
        println!(
            "  ivaldi fuse --strategy=theirs {}   - take the source wholesale",
            source
        );
        println!("  ivaldi fuse --abort  (or 'ivaldi oops')  - give up the fuse");
        if set_aside > 0 {
            println!(
                "\nYour {} uncommitted change(s) are set aside until then: '--continue' \
                 merges them back on top, '--abort' puts them back untouched.",
                set_aside
            );
        }

        // No merge seal was created and the timeline head has not moved, so
        // this is a failure — exiting 0 here is what lets a half-done merge
        // pass for a finished one in scripts and in the next command.
        return Err(format!(
            "{} file(s) with conflicts — merge not completed",
            conflict_paths.len()
        ));
    }

    Ok(())
}

/// Tell the user what became of the uncommitted work a fuse carried.
fn print_carry_report(report: &crate::carry::CarryReport) {
    use crate::carry::Outcome;

    let attention: Vec<_> = report.attention().collect();
    if attention.is_empty() {
        println!(
            "[OK] Your {} uncommitted change(s) are back on top, still unsealed.",
            report.outcomes.len()
        );
    } else {
        println!(
            "[OK] Re-applied your uncommitted changes: {} clean, {} need a look",
            report.clean_count(),
            attention.len()
        );
        for (path, outcome) in attention {
            let why = match outcome {
                Outcome::Conflict => "collides with the fuse — conflict markers written",
                Outcome::BinaryKeptFused => {
                    "binary, changed on both sides — fused version kept; yours is in the snapshot"
                }
                Outcome::KeptDeletedByFuse => {
                    "the fuse deleted it — your version kept as a new file"
                }
                Outcome::KeptChangedByFuse => {
                    "you deleted it, the fuse changed it — fused version kept"
                }
                Outcome::Clean | Outcome::Settled | Outcome::AlreadyFused => {
                    unreachable!("not attention outcomes")
                }
            };
            println!("       {}  ({})", path, why);
        }
    }
    if report.ungathered > 0 {
        println!(
            "  {} gathered file(s) were un-gathered: they were gathered against the old tip. \
             Gather again when you are ready to seal.",
            report.ungathered
        );
    }
}

/// The fuse is sealed by the time a re-apply can fail, so say so — and that
/// the work is safe, and how to get it.
fn carry_failure(error: &str, source: &str) -> String {
    format!(
        "the fuse is sealed, but re-applying your uncommitted changes failed: {error}\n\
         They are safe — run 'ivaldi fuse {source}' again to retry, or 'ivaldi oops' to \
         go back to before the fuse."
    )
}

/// Whether there is a person to ask. `IVALDI_INTERACTIVE=1` says there is
/// when the streams say otherwise — a driver feeding answers on stdin.
pub(super) fn interactive() -> bool {
    use std::io::IsTerminal;
    match std::env::var("IVALDI_INTERACTIVE").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
    }
}

/// Who settles collisions: the standing `--prefer` answer if one was given,
/// else the person at the terminal, else nobody.
pub(super) fn make_resolver(
    prefer: Option<crate::resolve::Prefer>,
) -> Option<Box<dyn crate::resolve::Resolver>> {
    use crate::resolve::{PreferResolver, PromptResolver, system_editor};
    match prefer {
        Some(prefer) => Some(Box::new(PreferResolver(prefer))),
        None if interactive() => Some(Box::new(PromptResolver::new(
            std::io::BufReader::new(std::io::stdin()),
            std::io::stdout(),
            system_editor(),
        ))),
        None => None,
    }
}

/// The error for a fuse with collisions and nobody to ask. Picking a side by
/// rule is never the silent default: a rule-picked region can compile and
/// still be wrong.
fn unattended_refusal(
    store: &crate::fsmerkle::FsStore<'_>,
    conflicts: &[crate::fuse::Conflict],
    source: &str,
) -> String {
    let mut lines = vec![format!(
        "{} file(s) changed on both sides in ways that collide, and there is no terminal \
         to ask which to keep:",
        conflicts.len()
    )];
    lines.extend(
        crate::resolve::describe_conflicts(store, conflicts)
            .into_iter()
            .map(|line| format!("  {line}")),
    );
    lines.push(format!(
        "\nNothing was changed. Choose how to settle them:\n  \
         ivaldi fuse {source} --prefer mine|theirs|both   settle every collision that way\n  \
         ivaldi fuse {source} --markers                   write conflict markers, resolve by hand\n  \
         ivaldi fuse {source}                             from a terminal, to be asked one at a time"
    ));
    lines.join("\n")
}

/// Finish a conflicted fuse. Re-runs the three-way merge to recover everything
/// that auto-resolved, then takes the conflicted paths from the workspace,
/// where the user resolved them. Both parents are recorded, so the merge is a
/// real merge seal — which is also what makes a diverged `upload` fast-forward
/// again.
fn continue_merge(
    repo: &mut Repo,
    prefer: Option<crate::resolve::Prefer>,
    quiet: bool,
) -> Result<(), String> {
    use crate::fuse::FuseEngine;
    use std::collections::BTreeMap;

    let state = repo
        .load_merge_state()
        .map_err(|e| e.to_string())?
        .ok_or("no merge in progress")?;

    let target = repo.current_timeline().map_err(|e| e.to_string())?;
    if target != state.target_timeline {
        return Err(format!(
            "the merge in progress targets '{}' but the current timeline is '{}' — \
             switch back, or run 'ivaldi fuse --abort'",
            state.target_timeline, target
        ));
    }

    let source_head = repo
        .get_timeline_head(&state.source_timeline)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "source timeline '{}' no longer exists — run 'ivaldi fuse --abort' and redo the merge",
                state.source_timeline
            )
        })?;
    let target_head = repo
        .get_timeline_head(&target)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("timeline '{}' has no seals", target))?;
    let source_leaf = repo
        .get_leaf(source_head)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt source head")?;
    let target_leaf = repo
        .get_leaf(target_head)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt target head")?;

    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let store = crate::fsmerkle::FsStore::new(&cas);

    let mut base_files = BTreeMap::new();
    let mut ours_files = BTreeMap::new();
    let mut theirs_files = BTreeMap::new();
    if let Some(base_idx) = repo
        .merge_base(target_head, source_head)
        .map_err(|e| e.to_string())?
        && let Some(base_leaf) = repo.get_leaf(base_idx).map_err(|e| e.to_string())?
    {
        collect_blob_hashes(&store, base_leaf.tree_root, "", &mut base_files)?;
    }
    collect_blob_hashes(&store, target_leaf.tree_root, "", &mut ours_files)?;
    collect_blob_hashes(&store, source_leaf.tree_root, "", &mut theirs_files)?;

    let strategy = state.strategy.parse::<Strategy>().unwrap_or(Strategy::Auto);
    let result = FuseEngine::fuse(&store, &base_files, &ours_files, &theirs_files, strategy);
    let mut merged = result.merged_files;

    // Everything the engine could not decide is read back from the workspace.
    let mut unresolved = Vec::new();
    for c in &result.conflicts {
        match std::fs::read(ctx.work_dir.join(&c.path)) {
            Ok(bytes) => {
                if crate::fuse::has_conflict_markers(&bytes) {
                    unresolved.push(c.path.clone());
                    continue;
                }
                let (hash, _) = store.put_blob(&bytes).map_err(|e| e.to_string())?;
                merged.insert(c.path.clone(), hash);
            }
            // Deleting the file is a valid resolution: it stays out of the tree.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot read {}: {}", c.path, e)),
        }
    }
    if !unresolved.is_empty() {
        println!("Unresolved conflicts:");
        for path in &unresolved {
            println!("  CONFLICT: {}", path);
        }
        return Err(format!(
            "{} file(s) still contain conflict markers — resolve them, then rerun \
             'ivaldi fuse --continue'",
            unresolved.len()
        ));
    }

    let merged_tree = store
        .build_tree_from_hash_map(&merged)
        .map_err(|e| e.to_string())?;

    let author = repo.config().author().unwrap_or_else(|| "ivaldi".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let message = format!("Fuse {} into {}", state.source_timeline, target);
    let mut fuse_leaf = crate::leaf::Leaf::new(merged_tree, &target, &author, now, &message);
    fuse_leaf.prev_idx = target_head;
    fuse_leaf.merge_idxs = vec![source_head];

    // Durable tree before the commit record that points at it (same ordering
    // as the clean-fuse path).
    cas.flush().map_err(|e| e.to_string())?;
    let commit_result = repo
        .commit_raw(fuse_leaf, &target)
        .map_err(|e| e.to_string())?;

    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    ws.materialize(merged_tree)
        .map_err(|e| format!("merge committed but failed to materialize: {}", e))?;

    repo.clear_merge_state().map_err(|e| e.to_string())?;
    // The scratch timeline sync created to hold the remote side has no further
    // use once the merge references it; its seals stay in the history.
    if state.source_timeline.starts_with("__sync_") {
        let _ = repo.remove_timeline(&state.source_timeline);
    }

    if !quiet {
        println!("[OK] Merge completed successfully!");
        println!(
            "  Merge seal: {} ({})",
            commit_result.seal_name,
            commit_result.hash.short8()
        );
    }
    crate::failpoint::fail_point("fuse.before_reapply");
    let report = crate::fuse_op::reapply(
        repo,
        &state.source_timeline,
        make_resolver(prefer).as_deref_mut(),
    )
    .map_err(|e| carry_failure(&e.to_string(), &state.source_timeline))?;
    if !quiet && let Some(report) = &report {
        print_carry_report(report);
    }
    Ok(())
}

pub(super) fn collect_blob_hashes(
    store: &crate::fsmerkle::FsStore<'_>,
    tree_hash: crate::hash::B3Hash,
    prefix: &str,
    files: &mut std::collections::BTreeMap<String, crate::hash::B3Hash>,
) -> Result<(), String> {
    let tree = store.load_tree(tree_hash).map_err(|e| e.to_string())?;
    for entry in &tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            crate::fsmerkle::NodeKind::Blob => {
                files.insert(path, entry.hash);
            }
            crate::fsmerkle::NodeKind::Tree => {
                collect_blob_hashes(store, entry.hash, &path, files)?;
            }
        }
    }
    Ok(())
}

/// `ivaldi weld` — combine a contiguous range of seals on the current
/// timeline into a single new seal that replaces them in the linear chain.
///
/// Three invocation forms:
///   * `ivaldi weld --last N [-m MSG]`               — last N seals
///   * `ivaldi weld START [-m MSG]`                  — START..HEAD
///   * `ivaldi weld START to END [-m MSG]`           — explicit range
///   * `ivaldi weld START END [-m MSG]`              — same, no connector
///   * `ivaldi weld` (no args)                       — interactive TUI picker
///
/// Semantics: the range is replaced by one new leaf whose `prev_idx` is
/// the parent of the oldest seal in the range. The original leaves stay
/// in the MMR for content-addressed integrity but become unreachable
/// from the timeline head. Tree content matches the newest seal in the
/// range (no merging — these were already linear).
pub(super) fn cmd_weld(args: WeldArgs, quiet: bool) -> Result<(), String> {
    use crate::leaf::{Leaf, NO_PARENT};

    let mut repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
    let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;

    if history.len() < 2 {
        return Err("need at least 2 seals to weld".into());
    }

    // Resolve range: returns (range_indices_newest_first, optional_message).
    // `range_indices_newest_first[0]` is END (newest), `[len-1]` is START (oldest).
    let (range, picker_message): (Vec<u64>, Option<String>) = if let Some(n) = args.last {
        if n < 2 {
            return Err("need at least 2 seals to weld".into());
        }
        if history.len() < n {
            return Err(format!(
                "only {} seals on '{}', need {}",
                history.len(),
                timeline,
                n
            ));
        }
        (history.iter().take(n).map(|e| e.index).collect(), None)
    } else if let Some(start_q) = &args.start {
        // Normalize the optional `to` connector: accept
        //   weld START          → END = HEAD
        //   weld START END
        //   weld START to END
        let end_q: Option<&str> = match (&args.second, &args.end) {
            (None, None) => None,
            (Some(s), None) => Some(s.as_str()),
            (Some(mid), Some(e)) if mid.eq_ignore_ascii_case("to") => Some(e.as_str()),
            (Some(mid), Some(_)) => {
                return Err(format!(
                    "expected `weld START to END` (got `{}` between names)",
                    mid
                ));
            }
            (None, Some(_)) => unreachable!("clap fills `second` before `end`"),
        };

        let (start_idx, _) = repo
            .resolve_seal(start_q)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("seal not found: {}", start_q))?;
        let end_idx = match end_q {
            Some(q) => {
                repo.resolve_seal(q)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("seal not found: {}", q))?
                    .0
            }
            None => history[0].index,
        };

        // Walk from end back along the timeline chain, collecting until we hit start.
        let mut indices = Vec::new();
        for entry in &history {
            indices.push(entry.index);
            if entry.index == start_idx {
                break;
            }
        }
        // Trim leading entries above `end_idx` if `end_idx` isn't the head.
        let found_start;
        if let Some(end_pos) = indices.iter().position(|i| *i == end_idx) {
            indices = indices[end_pos..].to_vec();
            // After trimming we still need start_idx in what remains.
            found_start = indices.contains(&start_idx);
        } else {
            return Err(format!(
                "end seal {} is not reachable from current timeline head",
                end_q.unwrap_or("HEAD")
            ));
        }
        if !found_start {
            return Err(format!(
                "start seal {} is not an ancestor of {} on '{}'",
                start_q,
                end_q.unwrap_or("HEAD"),
                timeline
            ));
        }
        if indices.len() < 2 {
            return Err("range must contain at least 2 seals to weld".into());
        }
        (indices, None)
    } else {
        // Interactive picker.
        use crate::tui::shift::{ShiftAction, run_shift};
        let action = run_shift(history.clone()).map_err(|e| e.to_string())?;
        match action {
            ShiftAction::Cancel => {
                println!("Cancelled.");
                return Ok(());
            }
            ShiftAction::Squash {
                start_index,
                end_index,
                message,
            } => {
                let mut indices = Vec::new();
                let mut started = false;
                for entry in &history {
                    if entry.index == end_index {
                        started = true;
                    }
                    if started {
                        indices.push(entry.index);
                    }
                    if entry.index == start_index {
                        break;
                    }
                }
                if indices.len() < 2 {
                    return Err("interactive picker returned an empty range".into());
                }
                (indices, Some(message))
            }
        }
    };

    // `range` is newest-first: [END, ..., START]. Validate contiguity on the
    // timeline chain — each entry's prev_idx must equal the next entry's idx.
    // Render seal names (not MMR indices) in errors.
    let seal_label = |idx: u64| -> String {
        repo.get_leaf(idx)
            .ok()
            .flatten()
            .and_then(|l| repo.get_seal_name(l.hash()).ok().flatten())
            .unwrap_or_else(|| format!("seal #{}", idx))
    };
    for w in range.windows(2) {
        let leaf = repo
            .get_leaf(w[0])
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("corrupt seal in range: {}", seal_label(w[0])))?;
        if leaf.prev_idx != w[1] {
            return Err(format!(
                "range is not contiguous on '{}': {} is not the parent of {}",
                timeline,
                seal_label(w[1]),
                seal_label(w[0])
            ));
        }
    }

    let end_idx = range[0];
    let start_idx = *range.last().unwrap();

    let end_leaf = repo
        .get_leaf(end_idx)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt end leaf")?;
    let start_leaf = repo
        .get_leaf(start_idx)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt start leaf")?;

    // The new welded seal takes the parent of the oldest seal in the range.
    let welded_prev = if start_leaf.has_parent() {
        start_leaf.prev_idx
    } else {
        NO_PARENT
    };

    // Compose the message.
    let cfg = repo.config();
    let author = cfg.author().unwrap_or_else(|| end_leaf.author.clone());
    let message = if let Some(m) = args.m {
        m
    } else if let Some(m) = picker_message {
        m
    } else {
        // Oldest → newest summary so the welded message reads in chronological order.
        let mut bullets: Vec<String> = Vec::new();
        for idx in range.iter().rev() {
            if let Some(leaf) = repo.get_leaf(*idx).map_err(|e| e.to_string())? {
                let first_line = leaf.message.lines().next().unwrap_or("").trim().to_string();
                bullets.push(format!("- {}", first_line));
            }
        }
        format!("Welded {} seals:\n\n{}", range.len(), bullets.join("\n"))
    };

    if !quiet {
        println!("Welding {} seals on '{}':", range.len(), timeline);
        for idx in range.iter().rev() {
            if let Some(leaf) = repo.get_leaf(*idx).map_err(|e| e.to_string())? {
                let short = leaf.hash().short8();
                let first_line = leaf.message.lines().next().unwrap_or("").trim();
                println!("  {} {}", short, first_line);
            }
        }
    }

    // Trailing seals = anything between END (exclusive) and the timeline head
    // (inclusive). They must be replayed on top of the welded seal so the
    // linear chain stays intact. With `--last N` the END is always the head,
    // so this list is empty; with a middle-range weld it isn't.
    // `history` is newest-first, so trailing seals appear before END.
    let trailing: Vec<u64> = history
        .iter()
        .map(|e| e.index)
        .take_while(|idx| *idx != end_idx)
        .collect();

    // Build the welded leaf plus every replayed trailing seal up front, then
    // commit the whole chain in ONE store transaction. Committing them one by
    // one would let a crash land the welded seal (or a partial replay chain)
    // with the remaining trailing seals silently orphaned from the head —
    // invisible loss, since orphaned MMR leaves are legal. Batch indices are
    // assigned consecutively from `commit_count`, so each replayed leaf can
    // chain onto its predecessor's predicted index.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut welded_leaf = Leaf::new(end_leaf.tree_root, &timeline, &author, now, &message);
    welded_leaf.prev_idx = welded_prev;

    let base_idx = repo.commit_count();
    let mut batch = vec![welded_leaf];

    // Replay trailing seals on top of the welded seal, oldest-first, each
    // parented on the previous replay (or on the welded seal itself for the
    // first one). Tree, author, message, and timestamp are preserved; only
    // `prev_idx` changes, which produces a new hash — there is no way to
    // keep the original seal hashes when their parent linkage changes.
    for trailing_idx in trailing.iter().rev() {
        let original = repo
            .get_leaf(*trailing_idx)
            .map_err(|e| e.to_string())?
            .ok_or("corrupt trailing leaf")?;
        let mut replayed = Leaf::new(
            original.tree_root,
            &timeline,
            &original.author,
            original.time_unix,
            &original.message,
        );
        replayed.prev_idx = base_idx + batch.len() as u64 - 1;
        replayed.merge_idxs = original.merge_idxs.clone();
        batch.push(replayed);
    }

    crate::failpoint::fail_point("weld.before_commit");
    let results = repo
        .commit_batch_raw(batch, &timeline)
        .map_err(|e| e.to_string())?;
    crate::failpoint::fail_point("weld.after_commit");
    let welded_result = &results[0];

    if !quiet {
        println!(
            "\nCreated welded seal: {} ({})",
            welded_result.seal_name,
            welded_result.hash.short8()
        );
        if trailing.is_empty() {
            println!(
                "{} seals welded into 1 on '{}'",
                range.len(),
                color::timeline(&timeline)
            );
        } else {
            println!(
                "{} seals welded into 1; {} trailing seal(s) replayed on top of '{}'",
                range.len(),
                trailing.len(),
                color::timeline(&timeline)
            );
        }
    }
    Ok(())
}
