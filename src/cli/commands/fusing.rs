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
    use crate::fuse::FuseEngine;
    use crate::repo::MergeState;
    use std::collections::BTreeMap;

    let mut repo = open_repo()?;

    if args.abort {
        if repo.has_merge_in_progress() {
            repo.clear_merge_state().map_err(|e| e.to_string())?;
            if !quiet {
                println!("Merge aborted.");
            }
        } else {
            return Err("no merge in progress".into());
        }
        return Ok(());
    }

    if args.continue_merge {
        let state = repo
            .load_merge_state()
            .map_err(|e| e.to_string())?
            .ok_or("no merge in progress")?;
        if state.conflicts.is_empty() {
            repo.clear_merge_state().map_err(|e| e.to_string())?;
            if !quiet {
                println!("Merge completed.");
            }
        } else {
            println!("Unresolved conflicts:");
            for c in &state.conflicts {
                println!("  CONFLICT: {}", c);
            }
            println!("\nResolve conflicts, then run 'ivaldi fuse --continue'");
        }
        return Ok(());
    }

    if repo.has_merge_in_progress() {
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

    let target = repo.current_timeline().map_err(|e| e.to_string())?;

    // Get trees for source and target
    let source_head = repo
        .get_timeline_head(source)
        .map_err(|e| e.to_string())?
        .ok_or(format!("timeline '{}' has no commits", source))?;
    let target_head = repo
        .get_timeline_head(&target)
        .map_err(|e| e.to_string())?
        .ok_or(format!("timeline '{}' has no commits", target))?;

    let source_leaf = repo
        .get_leaf(source_head)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt source head")?;
    let target_leaf = repo
        .get_leaf(target_head)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt target head")?;

    // Build file maps from trees
    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let store = crate::fsmerkle::FsStore::new(&cas);

    let mut base_files = BTreeMap::new();
    let mut ours_files = BTreeMap::new();
    let mut theirs_files = BTreeMap::new();

    // Walk the MMR-backed commit DAG to find the lowest common ancestor of
    // the two heads, and use its tree as the merge base. This is what makes
    // the `auto` strategy actually useful — without it, every file differing
    // between sides would be reported as a conflict.
    if let Some(base_idx) = repo
        .merge_base(target_head, source_head)
        .map_err(|e| e.to_string())?
    {
        // Source already reachable from the target head: nothing to fuse.
        // This also makes retrying a fuse that crashed after its commit a
        // clean no-op instead of a redundant merge seal.
        if base_idx == source_head {
            if !quiet {
                println!(
                    "Timeline '{}' is already fused into '{}' — nothing to do.",
                    source, target
                );
            }
            return Ok(());
        }
        if let Some(base_leaf) = repo.get_leaf(base_idx).map_err(|e| e.to_string())? {
            collect_blob_hashes(&store, base_leaf.tree_root, "", &mut base_files)?;
        }
    }
    collect_blob_hashes(&store, target_leaf.tree_root, "", &mut ours_files)?;
    collect_blob_hashes(&store, source_leaf.tree_root, "", &mut theirs_files)?;

    let result = FuseEngine::fuse(&store, &base_files, &ours_files, &theirs_files, strategy);

    if result.success {
        // Build merged tree (blobs already in CAS, just build tree structure)
        let merged_tree = store
            .build_tree_from_hash_map(&result.merged_files)
            .map_err(|e| e.to_string())?;

        let cfg = repo.config();
        let author = cfg.author().unwrap_or_else(|| "ivaldi".into());
        let message = format!("Fuse {} into {}", source, target);

        // Build a raw leaf so we can record the source head as a merge parent.
        // This preserves merge topology for GitHub uploads.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut fuse_leaf = crate::leaf::Leaf::new(merged_tree, &target, &author, now, &message);
        fuse_leaf.prev_idx = target_head;
        fuse_leaf.merge_idxs = vec![source_head];

        // Make the merged tree nodes (and any union-strategy concat blobs —
        // content that exists nowhere else) durable BEFORE the commit record
        // that references them. Without this, power loss after the store
        // transaction could persist a merge seal whose tree is gone, which
        // `verify --full` would reject.
        cas.flush().map_err(|e| e.to_string())?;
        crate::failpoint::fail_point("fuse.before_commit");
        let commit_result = repo
            .commit_raw(fuse_leaf, &target)
            .map_err(|e| e.to_string())?;
        crate::failpoint::fail_point("fuse.after_commit");

        // Write the merged tree out to the workspace so the user actually
        // sees the resolved files. Without this the seal exists in the MMR
        // but the working tree still shows pre-merge content.
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.materialize(merged_tree)
            .map_err(|e| format!("merge committed but failed to materialize: {}", e))?;

        if !quiet {
            println!("[OK] Merge completed successfully!");
            println!(
                "  Merge seal: {} ({})",
                commit_result.seal_name,
                commit_result.hash.short8()
            );
        }
    } else {
        // Save merge state with conflicts
        let conflict_paths: Vec<String> = result.conflicts.iter().map(|c| c.path.clone()).collect();
        let state = MergeState {
            source_timeline: source.to_string(),
            target_timeline: target.clone(),
            strategy: args.strategy.clone(),
            conflicts: conflict_paths.clone(),
        };
        repo.save_merge_state(&state).map_err(|e| e.to_string())?;
        crate::failpoint::fail_point("fuse.after_merge_state");

        println!("[CONFLICTS] Merge conflicts detected:\n");
        for path in &conflict_paths {
            println!("  CONFLICT: {}", path);
        }
        println!("\n>> {} file(s) with conflicts", conflict_paths.len());
        println!("\nResolution options:");
        println!("  ivaldi fuse --continue          - after manual resolution");
        println!("  ivaldi fuse --strategy=theirs {} to {}", source, target);
        println!("  ivaldi fuse --abort             - abort merge");
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
