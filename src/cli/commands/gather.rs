//! Repository init and staging commands: forge, gather (with patch mode), exclude.

use super::*;

pub(super) fn cmd_forge(quiet: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let result = forge::forge(&cwd).map_err(|e| e.to_string())?;
    if !quiet {
        if result.already_existed {
            println!(
                "Ivaldi repository already exists at {}",
                result.ivaldi_dir.display()
            );
        } else {
            println!(
                "Initialized empty Ivaldi repository in {}",
                result.ivaldi_dir.display()
            );
            println!("Created timeline: {}", result.default_timeline);
            if result.git_imported > 0 {
                println!(
                    "Imported {} Git branch(es) as timelines",
                    result.git_imported
                );
            }
        }
    }
    Ok(())
}

pub(super) fn cmd_gather(args: GatherArgs, quiet: bool) -> Result<(), String> {
    use crate::workspace::DotfileAllowlist;

    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let mut ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
    let mut allowlist = DotfileAllowlist::load(&ctx.ivaldi_dir);

    // Resolve the current timeline's tip tree so we can detect deletions
    // (paths in the parent tree that are missing from disk). For a brand-new
    // timeline with no parent commits the map is simply empty.
    let parent_tree_files: std::collections::BTreeMap<String, crate::hash::B3Hash> = {
        let repo = open_repo()?;
        let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
        match repo
            .get_timeline_head(&timeline)
            .map_err(|e| e.to_string())?
            .and_then(|idx| repo.get_leaf(idx).ok().flatten())
            .map(|l| l.tree_root)
        {
            Some(root) => ws.list_tree_files(root).map_err(|e| e.to_string())?,
            None => std::collections::BTreeMap::new(),
        }
    };

    if args.patch {
        let staged = run_patch_session(
            &mut ws,
            &cas,
            &parent_tree_files,
            &args.files,
            &ignore_cache,
            &mut std::io::stdin().lock(),
            quiet,
        )?;
        ws.save().map_err(|e| e.to_string())?;
        if !quiet {
            for path in &staged {
                println!("  gathered: {}", path);
            }
            println!("{} file(s) staged", staged.len());
        }
        return Ok(());
    }

    let mut all_gathered: Vec<String>;
    let mut all_deleted: Vec<String> = Vec::new();

    if args.files.is_empty() || args.files == ["."] {
        // Scan first (under a spinner) so we know how many files are about to
        // be hashed; the same listing also drives deletion detection below.
        let scan_spinner = (!quiet).then(|| crate::progress::spinner("Scanning workspace..."));
        let on_disk: std::collections::BTreeSet<String> = ws
            .scan(&ignore_cache)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        if let Some(sp) = scan_spinner {
            sp.finish_and_clear();
        }

        let bar = (!quiet && on_disk.len() > 1)
            .then(|| crate::progress::file_bar(on_disk.len() as u64, "Gathering"));
        let result = ws
            .gather_all_with_progress(&ignore_cache, &mut |_path| {
                if let Some(b) = &bar {
                    b.inc(1);
                }
            })
            .map_err(|e| e.to_string())?;
        if let Some(b) = bar {
            b.finish_and_clear();
        }
        all_gathered = result.gathered;

        // Anything in the parent tree but missing from disk is a deletion.
        // Skipped paths are never staged as deletions: they were filtered
        // out of the scan, so they would otherwise look missing here.
        for path in parent_tree_files.keys() {
            if !on_disk.contains(path.as_str()) && !ws.skipped.covers(path) {
                ws.staging.stage_deletion(path.clone());
                all_deleted.push(path.clone());
            }
        }

        if !quiet && !result.needs_confirmation.is_empty() {
            eprintln!(
                "Skipped {} hidden (dot) file(s):",
                result.needs_confirmation.len()
            );
            for dotfile in &result.needs_confirmation {
                eprintln!("  {}", dotfile);
            }
            eprintln!("  Use 'ivaldi gather <file>' to stage specific dotfiles");
        }
    } else {
        let refs: Vec<&str> = args.files.iter().map(|s| s.as_str()).collect();
        let bar = (!quiet && refs.len() > 1)
            .then(|| crate::progress::file_bar(refs.len() as u64, "Gathering"));
        let result = ws
            .gather_with_progress(&refs, &allowlist, &mut |_path| {
                if let Some(b) = &bar {
                    b.inc(1);
                }
            })
            .map_err(|e| e.to_string())?;
        if let Some(b) = bar {
            b.finish_and_clear();
        }
        all_gathered = result.gathered;

        for path in &result.skipped {
            eprintln!(
                "skipped: {} (excluded from staging; 'ivaldi unskip {}' to re-enable)",
                path, path
            );
        }

        // For each requested path that wasn't gathered (because it's missing
        // from disk), record it as a deletion if it was present in the parent
        // tree; otherwise it matched nothing at all. Skipped paths are
        // excluded from both treatments.
        let mut unmatched: Vec<&str> = Vec::new();
        for path in &refs {
            if ws.staging.is_staged(path) || ws.skipped.covers(path) {
                continue;
            }
            let full_path = ws.work_dir().join(path);
            if !full_path.exists() {
                if parent_tree_files.contains_key(*path) {
                    ws.staging.stage_deletion(path.to_string());
                    all_deleted.push(path.to_string());
                } else {
                    unmatched.push(path);
                }
            }
        }
        if !unmatched.is_empty() {
            if all_gathered.is_empty()
                && all_deleted.is_empty()
                && result.needs_confirmation.is_empty()
            {
                return Err(format!(
                    "pathspec '{}' did not match any files",
                    unmatched.join("', '")
                ));
            }
            for path in &unmatched {
                eprintln!("warning: pathspec '{}' did not match any files", path);
            }
        }

        if !result.needs_confirmation.is_empty() {
            if args.allow_all {
                let confirmed_refs: Vec<&str> = result
                    .needs_confirmation
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                let extra = ws
                    .gather_confirmed(&confirmed_refs)
                    .map_err(|e| e.to_string())?;
                for path in &extra {
                    allowlist.allow(path);
                }
                all_gathered.extend(extra);
            } else {
                for dotfile in &result.needs_confirmation {
                    eprint!(
                        "WARNING: '{}' is a hidden (dot) file — stage it? [y/N]: ",
                        dotfile
                    );
                    std::io::stderr().flush().map_err(|e| e.to_string())?;

                    let mut input = String::new();
                    std::io::stdin()
                        .read_line(&mut input)
                        .map_err(|e| e.to_string())?;

                    if input.trim().eq_ignore_ascii_case("y") {
                        let extra = ws
                            .gather_confirmed(&[dotfile.as_str()])
                            .map_err(|e| e.to_string())?;
                        allowlist.allow(dotfile);
                        all_gathered.extend(extra);
                    } else {
                        eprintln!("  skipped: {}", dotfile);
                    }
                }
            }

            allowlist.save().map_err(|e| e.to_string())?;
        }
    }

    // Classify gathered paths against the parent tree so we stage and report
    // only real changes — added or modified — rather than every file in the
    // workspace. Files identical to the last seal are dropped from staging:
    // `build_seal_tree` inherits untouched files from the parent tree, so
    // staging them changes nothing and only adds noise here and in `status`.
    let mut added: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    for path in &all_gathered {
        let staged_hash = ws.staging.staged_files().get(path).copied();
        match parent_tree_files.get(path) {
            None => added.push(path.clone()),
            Some(parent_hash) => {
                if staged_hash.as_ref() == Some(parent_hash) {
                    ws.staging.unstage(path);
                } else {
                    modified.push(path.clone());
                }
            }
        }
    }

    // The single commit point: every staging mutation above is in-memory
    // until this atomic_write lands, so a crash is old-or-new by design.
    crate::failpoint::fail_point("gather.before_stage_save");
    ws.save().map_err(|e| e.to_string())?;
    crate::failpoint::fail_point("gather.after_stage_save");

    if !quiet {
        for file in &added {
            println!("  added:    {}", file);
        }
        for file in &modified {
            println!("  modified: {}", file);
        }
        for file in &all_deleted {
            println!("  removed:  {}", file);
        }
        let total = added.len() + modified.len() + all_deleted.len();
        if total == 0 {
            println!("Nothing to gather — workspace matches the last seal.");
        } else {
            println!(
                "{} change(s) staged ({} added, {} modified, {} deleted)",
                total,
                added.len(),
                modified.len(),
                all_deleted.len()
            );
        }
    }
    Ok(())
}

/// Answer to a per-hunk or per-file prompt.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum PatchAnswer {
    Yes,
    No,
    AllRest,
    NoneRest,
    Quit,
}

pub(super) fn read_patch_answer(
    prompt: &str,
    input: &mut dyn std::io::BufRead,
) -> Result<PatchAnswer, String> {
    loop {
        print!("{} ", prompt);
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(PatchAnswer::Quit); // EOF
        }
        match line.trim() {
            "y" | "Y" => return Ok(PatchAnswer::Yes),
            "n" | "N" => return Ok(PatchAnswer::No),
            "a" | "A" => return Ok(PatchAnswer::AllRest),
            "d" | "D" => return Ok(PatchAnswer::NoneRest),
            "q" | "Q" => return Ok(PatchAnswer::Quit),
            "?" => {
                println!("y - stage this hunk");
                println!("n - do not stage this hunk");
                println!("a - stage this and all remaining hunks in this file");
                println!("d - skip this and all remaining hunks in this file");
                println!("q - quit; stage nothing further");
                println!("(splitting and editing hunks is not supported yet)");
            }
            _ => {}
        }
    }
}

/// Interactive hunk staging (`gather --patch`).
///
/// For each modified file, shows every hunk and asks which to stage. The
/// selected hunks are applied to the parent version in memory, the synthetic
/// blob goes into the CAS, and the staging area records its hash — the
/// working tree is never touched. Untracked and binary files get a single
/// whole-file prompt. Dotfiles keep their separate confirmation flow and are
/// skipped here. Reads answers from `input` so tests can script the session.
///
/// Known limitation: a partially staged file shows as `Staged` in status;
/// the remaining unstaged delta is not yet reported separately.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_patch_session(
    ws: &mut Workspace<'_>,
    cas: &FileCas,
    parent_tree_files: &std::collections::BTreeMap<String, crate::hash::B3Hash>,
    file_filter: &[String],
    ignore_cache: &ignore::PatternCache,
    input: &mut dyn std::io::BufRead,
    quiet: bool,
) -> Result<Vec<String>, String> {
    use crate::cas::Cas;
    use crate::diff::{LineOp, apply_selected_hunks, compute_hunks, compute_ops};
    use crate::fsmerkle::BlobNode;

    let store = crate::fsmerkle::FsStore::new(cas);
    let mut staged_paths: Vec<String> = Vec::new();

    // Candidates: files on disk that differ from the parent tree. Dotfiles
    // keep their dedicated confirmation flow — not part of --patch.
    let candidates: Vec<String> = ws
        .scan(ignore_cache)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|path| {
            let basename = path.rsplit('/').next().unwrap_or(path);
            !basename.starts_with('.') || basename == ".ivaldiignore"
        })
        .filter(|path| !ws.skipped.covers(path))
        .filter(|path| file_filter.is_empty() || file_filter.iter().any(|f| f == path || f == "."))
        .collect();

    'files: for path in candidates {
        if crate::ignore::is_security_blocked(&path) {
            continue;
        }
        let disk_content = std::fs::read(ws.work_dir().join(&path)).map_err(|e| e.to_string())?;

        let old_content: Option<Vec<u8>> = match parent_tree_files.get(&path) {
            Some(hash) => Some(store.load_blob(*hash).map_err(|e| e.to_string())?.1),
            None => None,
        };

        // Unchanged files aren't candidates.
        if let Some(old) = &old_content
            && *old == disk_content
        {
            continue;
        }

        let whole_file_only = old_content.is_none()
            || crate::diff::is_binary(&disk_content)
            || old_content.as_deref().is_some_and(crate::diff::is_binary);

        if whole_file_only {
            let label = if old_content.is_none() {
                "untracked"
            } else {
                "binary"
            };
            match read_patch_answer(&format!("Stage {} ({}) [y,n,q]?", path, label), input)? {
                PatchAnswer::Yes | PatchAnswer::AllRest => {
                    let canonical = BlobNode::canonical_bytes(&disk_content);
                    let hash = crate::hash::B3Hash::digest(&canonical);
                    cas.put(hash, &canonical).map_err(|e| e.to_string())?;
                    ws.staging.stage(&path, hash);
                    staged_paths.push(path.clone());
                }
                PatchAnswer::Quit => break 'files,
                _ => {}
            }
            continue;
        }

        let old_text = String::from_utf8_lossy(old_content.as_deref().unwrap()).into_owned();
        let new_text = String::from_utf8_lossy(&disk_content).into_owned();
        let ops = compute_ops(&old_text, &new_text);
        let hunks = compute_hunks(&ops, 3);
        if hunks.is_empty() {
            continue; // e.g. trailing-newline-only difference
        }

        if !quiet {
            println!("{}", color::bold(&path));
        }
        let mut selected = vec![false; hunks.len()];
        let mut blanket: Option<bool> = None;
        for (h_idx, hunk) in hunks.iter().enumerate() {
            if blanket.is_none() {
                println!("Hunk {}/{}:", h_idx + 1, hunks.len());
                for op in &ops[hunk.ops_range.clone()] {
                    match op {
                        LineOp::Context(s) => println!("    {}", color::dim(&format!(" {}", s))),
                        LineOp::Add(s) => {
                            println!("    {}", color::bold_green(&format!("+{}", s)))
                        }
                        LineOp::Remove(s) => {
                            println!("    {}", color::bold_red(&format!("-{}", s)))
                        }
                    }
                }
            }
            selected[h_idx] = match blanket {
                Some(b) => b,
                None => match read_patch_answer("Stage this hunk [y,n,a,d,q,?]?", input)? {
                    PatchAnswer::Yes => true,
                    PatchAnswer::No => false,
                    PatchAnswer::AllRest => {
                        blanket = Some(true);
                        true
                    }
                    PatchAnswer::NoneRest => {
                        blanket = Some(false);
                        false
                    }
                    PatchAnswer::Quit => break 'files,
                },
            };
        }

        if selected.iter().any(|&s| s) {
            let synthetic = apply_selected_hunks(&old_text, &new_text, &ops, &hunks, &selected);
            let canonical = BlobNode::canonical_bytes(synthetic.as_bytes());
            let hash = crate::hash::B3Hash::digest(&canonical);
            cas.put(hash, &canonical).map_err(|e| e.to_string())?;
            ws.staging.stage(&path, hash);
            staged_paths.push(path.clone());
        }
    }

    cas.flush().map_err(|e| e.to_string())?;
    Ok(staged_paths)
}

pub(super) fn cmd_exclude(args: ExcludeArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let ignore_path = ctx.work_dir.join(".ivaldiignore");
    let mut content = std::fs::read_to_string(&ignore_path).unwrap_or_default();
    for pattern in &args.patterns {
        if !content.lines().any(|l| l.trim() == pattern) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(pattern);
            content.push('\n');
            if !quiet {
                println!("  excluded: {}", pattern);
            }
        }
    }
    crate::atomic_io::atomic_write(&ignore_path, content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Normalize a user-supplied path for the skip list: drop a leading `./` and
/// any trailing slashes so directory entries match `SkipSet::covers`.
fn normalize_skip_path(raw: &str) -> &str {
    let path = raw.strip_prefix("./").unwrap_or(raw);
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { path } else { trimmed }
}

pub(super) fn cmd_skip(args: SkipArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mut skipped = crate::workspace::SkipSet::load(&ctx.ivaldi_dir);

    if args.list {
        if skipped.is_empty() {
            if !quiet {
                println!("No paths skipped from staging.");
            }
        } else {
            for path in skipped.iter() {
                println!("{}", path);
            }
        }
        return Ok(());
    }

    for raw in &args.paths {
        let path = normalize_skip_path(raw);
        skipped.add(path);
        if !quiet {
            println!("  skipped: {}", path);
        }
    }
    skipped.save().map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn cmd_unskip(args: UnskipArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mut skipped = crate::workspace::SkipSet::load(&ctx.ivaldi_dir);

    for raw in &args.paths {
        let path = normalize_skip_path(raw);
        if skipped.remove(path) {
            if !quiet {
                println!("  unskipped: {}", path);
            }
        } else {
            eprintln!("warning: '{}' was not skipped", path);
        }
    }
    skipped.save().map_err(|e| e.to_string())?;
    Ok(())
}
