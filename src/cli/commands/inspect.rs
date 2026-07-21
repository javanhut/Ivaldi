//! Read-only inspection commands: status, whereami, log, whodidit, diff.

use super::*;

pub(super) fn cmd_status(args: StatusArgs) -> Result<(), String> {
    let ctx = find_repo()?;
    let repo = open_repo()?;
    let timeline = repo
        .current_timeline()
        .unwrap_or_else(|_| "detached".into());

    // Get last seal info (and tree hash for comparison) up front so both the
    // human and JSON outputs share the same data.
    let head_leaf = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .map(|idx| repo.get_leaf(idx).map_err(|e| e.to_string()))
        .transpose()?
        .flatten();
    let last_tree = head_leaf.as_ref().map(|leaf| leaf.tree_root);

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
    let status = ws
        .status(last_tree, &ignore_cache)
        .map_err(|e| e.to_string())?;

    if args.json {
        let head = head_leaf.as_ref().map(|leaf| {
            let hash = leaf.hash();
            json::SealRefJson {
                seal_name: seal::generate_seal_name(hash),
                hash: hash.to_hex(),
                short_hash: hash.short8(),
            }
        });
        let staged_deletions: Vec<String> = ws.staging.staged_deletions().iter().cloned().collect();
        let files: Vec<json::FileJson> = status
            .iter()
            .filter(|f| f.state != FileState::Unmodified)
            .map(|f| json::FileJson {
                path: f.path.clone(),
                state: match f.state {
                    FileState::Untracked => "untracked",
                    FileState::Unmodified => "unmodified",
                    FileState::Modified => "modified",
                    FileState::Staged => "staged",
                    FileState::Deleted => "deleted",
                }
                .to_string(),
                hash: f.hash.map(|h| h.to_hex()),
            })
            .collect();
        let out = json::StatusJson {
            timeline,
            head,
            files,
            staged_deletions,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    println!("Timeline: {}", color::timeline(&timeline));

    // Show last seal info
    if let Some(leaf) = &head_leaf {
        let hash = leaf.hash();
        let name = seal::generate_seal_name(hash);
        println!(
            "Last seal: {} ({})",
            color::seal_name(&name),
            color::hash(&hash.short8())
        );
    }

    if !ws.skipped.is_empty() {
        println!(
            "Excluded from staging: {} path(s) ('ivaldi skip --list' to show)",
            ws.skipped.iter().count()
        );
    }

    let staged: Vec<_> = status
        .iter()
        .filter(|f| f.state == FileState::Staged)
        .collect();
    let modified: Vec<_> = status
        .iter()
        .filter(|f| f.state == FileState::Modified)
        .collect();
    let untracked: Vec<_> = status
        .iter()
        .filter(|f| f.state == FileState::Untracked)
        .collect();
    // Deletions that have been staged for the next seal are shown under the
    // Staged section, not under unstaged Changes.
    let staged_deletions: std::collections::BTreeSet<&String> =
        ws.staging.staged_deletions().iter().collect();
    let deleted: Vec<_> = status
        .iter()
        .filter(|f| f.state == FileState::Deleted && !staged_deletions.contains(&f.path))
        .collect();

    let has_changes = !modified.is_empty() || !untracked.is_empty() || !deleted.is_empty();
    let has_staged = !staged.is_empty() || !staged_deletions.is_empty();

    if !has_changes && !has_staged {
        println!("\nWorking directory: clean");
        return Ok(());
    }

    if has_changes {
        println!("\nChanges:");
        for f in &untracked {
            println!("  {} {:<30} (added)", color::bold_green("++"), f.path);
        }
        for f in &modified {
            println!("  {} {:<30} (modified)", color::bold_yellow("~~"), f.path);
        }
        for f in &deleted {
            println!("  {} {:<30} (deleted)", color::bold_red("--"), f.path);
        }
    }

    if has_staged {
        println!("\nStaged:");
        for f in &staged {
            println!("  {} {:<30} (staged)", color::bold_green("++"), f.path);
        }
        for path in &staged_deletions {
            println!(
                "  {} {:<30} (staged for deletion)",
                color::bold_red("--"),
                path
            );
        }
    }

    Ok(())
}

pub(super) fn cmd_whereami() -> Result<(), String> {
    let repo = open_repo()?;
    let timeline = repo
        .current_timeline()
        .unwrap_or_else(|_| "detached".into());

    println!("Timeline: {}", timeline);
    println!("Type: Local Timeline");

    if let Some(head_idx) = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        && let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())?
    {
        let hash = leaf.hash();
        let name = seal::generate_seal_name(hash);
        let rel = ivaldi_log::relative_time(
            leaf.time_unix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        );
        println!("Last Seal: {} ({})", name, hash.short8());
        println!("  Message: \"{}\"", leaf.message);
        println!("  Author: {}", leaf.author);
        println!("  Date: {}", rel);
    }

    println!("Commits: {}", repo.timeline_commit_count(&timeline));
    Ok(())
}

pub(super) fn cmd_log(args: LogArgs) -> Result<(), String> {
    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());

    let entries = if args.all {
        // Collect from all timelines, dedup
        let mut all = Vec::new();
        for (tl_name, _) in repo.list_timelines().map_err(|e| e.to_string())? {
            all.extend(repo.walk_history(&tl_name).map_err(|e| e.to_string())?);
        }
        all.sort_by_key(|e| std::cmp::Reverse(e.time_unix));
        all.dedup_by_key(|e| e.index);
        all
    } else {
        repo.walk_history(&timeline).map_err(|e| e.to_string())?
    };

    let entries = if let Some(limit) = args.limit {
        &entries[..limit.min(entries.len())]
    } else {
        &entries
    };

    let format = args.format.unwrap_or(if args.oneline {
        LogFormat::Short
    } else {
        LogFormat::Medium
    });

    if format == LogFormat::Json {
        let out: Vec<json::LogEntryJson> = entries.iter().map(json::LogEntryJson::from).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if entries.is_empty() {
        println!("No commits yet on timeline '{}'", timeline);
        return Ok(());
    }

    for entry in entries {
        match format {
            LogFormat::Short => {
                println!(
                    "{} {} {}",
                    color::hash(&entry.short_hash),
                    color::seal_name(&entry.seal_name),
                    entry.message
                );
            }
            LogFormat::Medium => {
                println!(
                    "Seal: {} ({})",
                    color::seal_name(&entry.seal_name),
                    color::hash(&entry.short_hash)
                );
                println!("Timeline: {}", color::timeline(&entry.timeline));
                println!("Author: {}", color::author(&entry.author));
                println!("Date: {}", entry.time_unix);
                println!();
                println!("    {}", entry.message);
                println!();
            }
            LogFormat::Full => {
                println!(
                    "Seal: {} ({})",
                    color::seal_name(&entry.seal_name),
                    color::hash(&entry.short_hash)
                );
                println!("Hash: {}", entry.hash.to_hex());
                println!("Timeline: {}", color::timeline(&entry.timeline));
                println!("Author: {}", color::author(&entry.author));
                println!(
                    "Date: {} ({})",
                    format_unix_utc(entry.time_unix),
                    entry.time_unix
                );
                if let Ok(Some(leaf)) = repo.get_leaf(entry.index) {
                    println!("Tree: {}", leaf.tree_root.to_hex());
                    if entry.is_merge {
                        let parents: Vec<String> =
                            leaf.merge_idxs.iter().map(|i| i.to_string()).collect();
                        println!("Merge parents: {}", parents.join(", "));
                    }
                }
                println!();
                println!("    {}", entry.message);
                println!();
            }
            LogFormat::Json => unreachable!("handled above"),
        }
    }
    Ok(())
}

/// Format a unix-second timestamp as an absolute UTC date string
/// (`YYYY-MM-DD HH:MM:SS UTC`). Implements Howard Hinnant's civil-date
/// algorithm so we don't need a date crate.
pub(super) fn format_unix_utc(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let secs_of_day = unix_seconds.rem_euclid(86_400) as u32;
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;

    // Days since 1970-01-01 → civil date (Hinnant).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, m, d, h, mi, s
    )
}

/// Read the contents of a file at a given tree root by walking the path.
/// Returns `Ok(None)` if the path doesn't exist (or isn't a regular file)
/// in that tree.
pub(super) fn read_file_at_tree(
    store: &crate::fsmerkle::FsStore<'_>,
    tree_root: crate::hash::B3Hash,
    path: &str,
) -> Result<Option<Vec<u8>>, String> {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() {
        return Ok(None);
    }

    let mut current_hash = tree_root;
    for (i, part) in parts.iter().enumerate() {
        let tree = match store.load_tree(current_hash) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let entry = match tree.find_entry(part) {
            Some(e) => e,
            None => return Ok(None),
        };
        let last = i == parts.len() - 1;
        match (entry.kind, last) {
            (crate::fsmerkle::NodeKind::Blob, true) => {
                let (_, content) = store
                    .load_blob(entry.hash)
                    .map_err(|e| format!("read blob: {}", e))?;
                return Ok(Some(content));
            }
            (crate::fsmerkle::NodeKind::Tree, false) => {
                current_hash = entry.hash;
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// `ivaldi whodidit <file>` — git-blame analogue.
///
/// For each line of the file at HEAD, find the oldest seal in which the
/// line still appears (content-set membership). The next-older seal does
/// not contain the line, so the oldest-still-present seal is the one that
/// introduced it. This is a position-independent approximation — it
/// handles the common cases (added/edited lines) without trying to track
/// line moves the way more sophisticated blame algorithms do.
pub(super) fn cmd_whodidit(args: WhodiditArgs) -> Result<(), String> {
    use std::collections::HashSet;

    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
    let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;

    if history.is_empty() {
        return Err(format!("no commits on timeline '{}'", timeline));
    }

    let store = crate::fsmerkle::FsStore::new(&repo.cas);

    // File contents at HEAD.
    let head_leaf = repo
        .get_leaf(history[0].index)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "HEAD leaf missing".to_string())?;
    let head_bytes = read_file_at_tree(&store, head_leaf.tree_root, &args.path)?
        .ok_or_else(|| format!("file not found at HEAD: {}", args.path))?;
    let head_text = String::from_utf8_lossy(&head_bytes).into_owned();
    let lines: Vec<&str> = head_text.split_inclusive('\n').collect();

    // Each line starts attributed to HEAD. Walking back, we replace the
    // attribution with an older seal as long as that seal's version of
    // the file still contains the line. The first seal that doesn't
    // contain the line halts further updates for that line.
    let mut attr: Vec<usize> = vec![0; lines.len()]; // index into history
    let mut still_open: Vec<bool> = vec![true; lines.len()];

    for (h_idx, entry) in history.iter().enumerate().skip(1) {
        if !still_open.iter().any(|&b| b) {
            break;
        }
        let leaf = match repo.get_leaf(entry.index).map_err(|e| e.to_string())? {
            Some(l) => l,
            None => continue,
        };
        let bytes = read_file_at_tree(&store, leaf.tree_root, &args.path)?;
        let owned_lines: Vec<String> = match bytes {
            Some(b) => String::from_utf8_lossy(&b)
                .split_inclusive('\n')
                .map(|s| s.to_string())
                .collect(),
            None => Vec::new(),
        };
        let owned_set: HashSet<&str> = owned_lines.iter().map(|s| s.as_str()).collect();

        for (li, line) in lines.iter().enumerate() {
            if !still_open[li] {
                continue;
            }
            if owned_set.contains(*line) {
                attr[li] = h_idx;
            } else {
                still_open[li] = false;
            }
        }
    }

    // Render
    let mut prev_attr: Option<usize> = None;
    let line_no_width = (lines.len().max(1)).to_string().len();

    if args.summary {
        // Compress contiguous regions sharing the same attribution.
        let mut start = 0usize;
        while start < lines.len() {
            let a = attr[start];
            let mut end = start + 1;
            while end < lines.len() && attr[end] == a {
                end += 1;
            }
            let e = &history[a];
            println!(
                "{}-{}  {} {} ({})",
                start + 1,
                end,
                color::hash(&e.short_hash),
                color::seal_name(&e.seal_name),
                color::author(&e.author),
            );
            start = end;
        }
        return Ok(());
    }

    for (li, line) in lines.iter().enumerate() {
        let a = attr[li];
        let e = &history[a];
        let header_changed = prev_attr != Some(a);
        prev_attr = Some(a);

        let line_text = line.strip_suffix('\n').unwrap_or(line);

        if header_changed {
            print!(
                "{} {} ({}) ",
                color::hash(&e.short_hash),
                color::seal_name(&e.seal_name),
                color::author(&e.author),
            );
        } else {
            // Repeat-attribution rows: align with the header above using
            // a faint placeholder so the line numbers stay in a column.
            let pad: String = " ".repeat(8 + 1 + e.seal_name.len() + 2 + e.author.len() + 2);
            print!("{}", pad);
        }
        println!("{:>width$})  {}", li + 1, line_text, width = line_no_width);
    }

    Ok(())
}

pub(super) fn cmd_diff(args: DiffArgs) -> Result<(), String> {
    let repo = open_repo()?;
    let ctx = find_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());

    match args.targets.len() {
        2 => {
            // Timeline-to-timeline or seal-to-seal diff
            let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
            let store = crate::fsmerkle::FsStore::new(&cas);

            let (label_a, tree_a) = resolve_tree(&repo, &args.targets[0])?;
            let (label_b, tree_b) = resolve_tree(&repo, &args.targets[1])?;

            println!("Diff: {} -> {}", label_a, label_b);

            let changes = crate::fsmerkle::diff_trees(tree_a, tree_b, &store)
                .map_err(|e| format!("diff failed: {}", e))?;

            if changes.is_empty() {
                println!("\n  No differences.");
            } else {
                println!();
                let mut added = 0usize;
                let mut modified = 0usize;
                let mut deleted = 0usize;
                for c in &changes {
                    match c.kind {
                        crate::fsmerkle::ChangeKind::Added => added += 1,
                        crate::fsmerkle::ChangeKind::Deleted => deleted += 1,
                        crate::fsmerkle::ChangeKind::Modified
                        | crate::fsmerkle::ChangeKind::TypeChange => modified += 1,
                    }
                }

                if args.stat {
                    // Summary only
                    for c in &changes {
                        let marker_fn: fn(&str) -> String = match c.kind {
                            crate::fsmerkle::ChangeKind::Added => color::bold_green,
                            crate::fsmerkle::ChangeKind::Deleted => color::bold_red,
                            _ => color::bold_yellow,
                        };
                        let kind = match c.kind {
                            crate::fsmerkle::ChangeKind::Added => "added",
                            crate::fsmerkle::ChangeKind::Deleted => "deleted",
                            crate::fsmerkle::ChangeKind::Modified => "modified",
                            crate::fsmerkle::ChangeKind::TypeChange => "type-change",
                        };
                        println!("  {} {}  ({})", marker_fn("|"), c.path, kind);
                    }
                } else {
                    // Per-file markers + line-level hunks for modified text files
                    for c in &changes {
                        match c.kind {
                            crate::fsmerkle::ChangeKind::Added => {
                                println!("  {} {}", color::bold_green("++"), c.path);
                                crate::diff::print_blob_as_added(&store, c.new_hash);
                            }
                            crate::fsmerkle::ChangeKind::Deleted => {
                                println!("  {} {}", color::bold_red("--"), c.path);
                                crate::diff::print_blob_as_deleted(&store, c.old_hash);
                            }
                            crate::fsmerkle::ChangeKind::Modified => {
                                println!("  {} {}", color::bold_yellow("~~"), c.path);
                                crate::diff::print_blob_diff(&store, c.old_hash, c.new_hash);
                            }
                            crate::fsmerkle::ChangeKind::TypeChange => {
                                println!("  {} {}", color::bold_yellow("~~"), c.path);
                            }
                        }
                    }
                }
                println!(
                    "\n{} change(s): {} added, {} modified, {} deleted",
                    changes.len(),
                    added,
                    modified,
                    deleted
                );
            }
        }
        1 => {
            // Single target: resolve as seal and show info, or working dir vs that seal
            let target = &args.targets[0];
            match repo.resolve_seal(target).map_err(|e| e.to_string())? {
                Some((_, leaf)) => {
                    let hash = leaf.hash();
                    println!(
                        "Comparing against seal: {} ({})",
                        seal::generate_seal_name(hash),
                        hash.short8()
                    );
                    println!("  Message: {}", leaf.message);
                }
                None => {
                    // Try as timeline name
                    if repo
                        .get_timeline_head(target)
                        .map_err(|e| e.to_string())?
                        .is_some()
                    {
                        return Err(format!(
                            "'{}' is a timeline. Use two targets for timeline diff: ivaldi diff <a> <b>",
                            target
                        ));
                    }
                    return Err(format!("seal or hash not found: {}", target));
                }
            }
        }
        0 => {
            if args.staged {
                let cas =
                    FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
                let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                if ws.staging.is_empty() {
                    println!("No staged changes.");
                } else {
                    println!("Staged changes:");
                    for path in ws.staging.staged_files().keys() {
                        println!("  {} {}", color::bold_green("++"), path);
                    }
                    for path in ws.staging.staged_deletions() {
                        println!("  {} {}", color::bold_red("--"), path);
                    }
                }
            } else {
                // Working directory changes vs last seal
                let last_tree = repo
                    .get_timeline_head(&timeline)
                    .map_err(|e| e.to_string())?
                    .and_then(|idx| repo.get_leaf(idx).ok().flatten())
                    .map(|l| l.tree_root);

                let cas =
                    FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
                let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
                let status = ws
                    .status(last_tree, &ignore_cache)
                    .map_err(|e| e.to_string())?;

                let changes: Vec<_> = status
                    .iter()
                    .filter(|f| f.state != FileState::Unmodified)
                    .collect();

                if changes.is_empty() {
                    println!("No changes.");
                } else if args.stat {
                    let mod_count = changes
                        .iter()
                        .filter(|f| f.state == FileState::Modified)
                        .count();
                    let add_count = changes
                        .iter()
                        .filter(|f| f.state == FileState::Untracked)
                        .count();
                    let del_count = changes
                        .iter()
                        .filter(|f| f.state == FileState::Deleted)
                        .count();
                    let stg_count = changes
                        .iter()
                        .filter(|f| f.state == FileState::Staged)
                        .count();
                    println!(
                        "{} file(s) changed: {} modified, {} added, {} deleted, {} staged",
                        changes.len(),
                        mod_count,
                        add_count,
                        del_count,
                        stg_count
                    );
                } else {
                    for f in &changes {
                        let (marker, marker_fn): (&str, fn(&str) -> String) = match f.state {
                            FileState::Modified => ("~~", color::bold_yellow),
                            FileState::Untracked => ("++", color::bold_green),
                            FileState::Deleted => ("--", color::bold_red),
                            FileState::Staged => ("++", color::bold_green),
                            _ => ("??", color::dim),
                        };
                        println!("  {} {}", marker_fn(marker), f.path);
                    }
                }
            }
        }
        _ => return Err("too many arguments. Usage: ivaldi diff [<target1> [<target2>]]".into()),
    }
    Ok(())
}

/// Resolve a diff target to a (label, tree_hash) pair.
/// Tries timeline name first, then seal name/hash prefix.
pub(super) fn resolve_tree(
    repo: &Repo,
    target: &str,
) -> Result<(String, crate::hash::B3Hash), String> {
    // Try as timeline name first
    if let Some(head_idx) = repo.get_timeline_head(target).map_err(|e| e.to_string())?
        && let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())?
    {
        return Ok((format!("timeline:{}", target), leaf.tree_root));
    }

    // Try as seal name / hash prefix
    if let Some((_, leaf)) = repo.resolve_seal(target).map_err(|e| e.to_string())? {
        let hash = leaf.hash();
        let name = seal::generate_seal_name(hash);
        return Ok((format!("seal:{}", name), leaf.tree_root));
    }

    Err(format!(
        "could not resolve '{}' as timeline or seal",
        target
    ))
}

/// Format a change marker for display.
/// Returns (marker, color_fn) for the given ChangeKind.
#[cfg(test)]
pub(crate) fn change_marker(
    kind: crate::fsmerkle::ChangeKind,
) -> (&'static str, fn(&str) -> String) {
    match kind {
        crate::fsmerkle::ChangeKind::Added => ("++", color::bold_green),
        crate::fsmerkle::ChangeKind::Deleted => ("--", color::bold_red),
        crate::fsmerkle::ChangeKind::Modified | crate::fsmerkle::ChangeKind::TypeChange => {
            ("~~", color::bold_yellow)
        }
    }
}

/// Format a file state marker for display.
#[cfg(test)]
pub(crate) fn state_marker(state: FileState) -> (&'static str, fn(&str) -> String) {
    match state {
        FileState::Modified => ("~~", color::bold_yellow),
        FileState::Untracked => ("++", color::bold_green),
        FileState::Deleted => ("--", color::bold_red),
        FileState::Staged => ("++", color::bold_green),
        _ => ("??", color::dim),
    }
}
