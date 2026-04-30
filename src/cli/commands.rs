//! Command implementations for Ivaldi CLI.
//!
//! Each function handles one CLI command, wiring user input to the core modules.
//! Commands that need persistent state use `Repo`; others use lightweight helpers.

use std::io::Write;
use std::path::PathBuf;
use std::process;

use crate::cas::FileCas;
use crate::color;
use crate::config::{self, Config};
use crate::forge;
use crate::fuse::Strategy;
use crate::ignore;
use crate::log as ivaldi_log;
use crate::portal::{Platform, Portal, PortalManager};
use crate::repo::Repo;
use crate::seal;
use crate::workspace::{FileState, Workspace};

use super::*;

pub fn run_command(cli: Cli) {
    if let Some(cmd) = cli.command {
        let result = match cmd {
            Commands::Forge => cmd_forge(cli.quiet),
            Commands::Gather(args) => cmd_gather(args, cli.quiet),
            Commands::Seal(args) => cmd_seal(args, cli.quiet),
            Commands::Status => cmd_status(),
            Commands::Whereami => cmd_whereami(),
            Commands::Log(args) => cmd_log(args),
            Commands::Diff(args) => cmd_diff(args),
            Commands::Reset(args) => cmd_reset(args, cli.quiet),
            Commands::Timeline(args) => cmd_timeline(args, cli.quiet),
            Commands::Fuse(args) => cmd_fuse(args, cli.quiet),
            Commands::Travel(args) => cmd_travel(args),
            Commands::Shift(args) => cmd_shift(args, cli.quiet),
            Commands::Config(args) => cmd_config(args),
            Commands::Exclude(args) => cmd_exclude(args, cli.quiet),
            Commands::Portal(args) => cmd_portal(args, cli.quiet),
            Commands::Auth(args) => cmd_auth(args),
            Commands::Download(args) => cmd_download(args, cli.quiet),
            Commands::Upload(args) => cmd_upload(args, cli.quiet),
            Commands::Scout(args) => cmd_scout(args),
            Commands::Harvest(args) => cmd_harvest(args, cli.quiet),
            Commands::Sync(args) => cmd_sync(args, cli.quiet),
            Commands::Review(args) => cmd_review(args, cli.quiet),
            Commands::Tui => cmd_tui(),
        };
        if let Err(e) = result {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    } else {
        let _ = Cli::try_parse_from(["ivaldi", "--help"]);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct RepoContext {
    work_dir: PathBuf,
    ivaldi_dir: PathBuf,
}

fn find_repo() -> Result<RepoContext, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = forge::find_repo_root(&cwd)
        .ok_or("not an Ivaldi repository (or any parent). Run 'ivaldi forge' to initialize.")?;
    Ok(RepoContext {
        ivaldi_dir: root.join(".ivaldi"),
        work_dir: root,
    })
}

fn open_repo() -> Result<Repo, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = forge::find_repo_root(&cwd)
        .ok_or("not an Ivaldi repository (or any parent). Run 'ivaldi forge' to initialize.")?;
    Repo::open(&root).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_forge(quiet: bool) -> Result<(), String> {
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

fn cmd_gather(args: GatherArgs, quiet: bool) -> Result<(), String> {
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

    let mut all_gathered: Vec<String>;
    let mut all_deleted: Vec<String> = Vec::new();

    if args.files.is_empty() || args.files == ["."] {
        let result = ws.gather_all(&ignore_cache).map_err(|e| e.to_string())?;
        all_gathered = result.gathered;

        // Anything in the parent tree but missing from disk is a deletion.
        let on_disk: std::collections::BTreeSet<String> =
            ws.scan(&ignore_cache).map_err(|e| e.to_string())?.into_iter().collect();
        for path in parent_tree_files.keys() {
            if !on_disk.contains(path.as_str()) {
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
        let result = ws.gather(&refs, &allowlist).map_err(|e| e.to_string())?;
        all_gathered = result.gathered;

        // For each requested path that wasn't gathered (because it's missing
        // from disk), record it as a deletion if it was present in the parent
        // tree. Paths that aren't in the parent tree and aren't on disk are
        // silently skipped, matching the prior behaviour.
        for path in &refs {
            if ws.staging.is_staged(path) {
                continue;
            }
            let full_path = ws.work_dir().join(path);
            if !full_path.exists() && parent_tree_files.contains_key(*path) {
                ws.staging.stage_deletion(path.to_string());
                all_deleted.push(path.to_string());
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

    ws.save().map_err(|e| e.to_string())?;

    if !quiet {
        for file in &all_gathered {
            println!("  gathered: {}", file);
        }
        for file in &all_deleted {
            println!("  removed:  {}", file);
        }
        let total = all_gathered.len() + all_deleted.len();
        if all_deleted.is_empty() {
            println!("{} file(s) staged", total);
        } else {
            println!(
                "{} file(s) staged ({} added, {} deleted)",
                total,
                all_gathered.len(),
                all_deleted.len()
            );
        }
    }
    Ok(())
}

fn cmd_seal(args: SealArgs, quiet: bool) -> Result<(), String> {
    let message = args
        .get_message()
        .ok_or("seal message required. Usage: ivaldi seal \"your message\"")?
        .to_string();

    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if ws.staging.is_empty() {
        return Err("no changes staged for seal. Use 'ivaldi gather' to stage files first.".into());
    }

    // Open persistent repo first so we can resolve the current timeline's
    // parent tree, then build the seal tree as parent + staging.
    let mut repo = open_repo()?;
    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
    let parent_tree = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten())
        .map(|l| l.tree_root);

    let tree_hash = ws
        .build_seal_tree(parent_tree)
        .map_err(|e| e.to_string())?;

    let cfg = repo.config();
    let author = cfg.author()
        .ok_or("user.name and user.email not configured. Run:\n  ivaldi config --set user.name \"Your Name\"\n  ivaldi config --set user.email \"you@example.com\"")?;

    let result = repo
        .commit(tree_hash, &author, &message)
        .map_err(|e| e.to_string())?;

    // Clear staging area
    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    ws_mut.staging.clear();
    ws_mut.save().map_err(|e| e.to_string())?;

    if !quiet {
        println!(
            "Created seal: {} ({})",
            result.seal_name,
            result.hash.short8()
        );
    }
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    let ctx = find_repo()?;
    let repo = open_repo()?;
    let timeline = repo
        .current_timeline()
        .unwrap_or_else(|_| "detached".into());

    println!("Timeline: {}", color::timeline(&timeline));

    // Get last seal tree hash for comparison
    let last_tree = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten())
        .map(|leaf| leaf.tree_root);

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
    let status = ws
        .status(last_tree, &ignore_cache)
        .map_err(|e| e.to_string())?;

    // Show last seal info
    if let Some(head_idx) = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
    {
        if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
            let hash = leaf.hash();
            let name = seal::generate_seal_name(hash);
            println!(
                "Last seal: {} ({})",
                color::seal_name(&name),
                color::hash(&hash.short8())
            );
        }
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

fn cmd_whereami() -> Result<(), String> {
    let repo = open_repo()?;
    let timeline = repo
        .current_timeline()
        .unwrap_or_else(|_| "detached".into());

    println!("Timeline: {}", timeline);
    println!("Type: Local Timeline");

    if let Some(head_idx) = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
    {
        if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
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
    }

    println!("Commits: {}", repo.timeline_commit_count(&timeline));
    Ok(())
}

fn cmd_log(args: LogArgs) -> Result<(), String> {
    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());

    let entries = if args.all {
        // Collect from all timelines, dedup
        let mut all = Vec::new();
        for (tl_name, _) in repo.list_timelines().map_err(|e| e.to_string())? {
            all.extend(repo.walk_history(&tl_name).map_err(|e| e.to_string())?);
        }
        all.sort_by(|a, b| b.time_unix.cmp(&a.time_unix));
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

    if entries.is_empty() {
        println!("No commits yet on timeline '{}'", timeline);
        return Ok(());
    }

    for entry in entries {
        if args.oneline {
            println!(
                "{} {} {}",
                color::hash(&entry.short_hash),
                color::seal_name(&entry.seal_name),
                entry.message
            );
        } else {
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
    }
    Ok(())
}

fn cmd_diff(args: DiffArgs) -> Result<(), String> {
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
                    for (path, _) in ws.staging.staged_files() {
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
fn resolve_tree(repo: &Repo, target: &str) -> Result<(String, crate::hash::B3Hash), String> {
    // Try as timeline name first
    if let Some(head_idx) = repo.get_timeline_head(target).map_err(|e| e.to_string())? {
        if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
            return Ok((format!("timeline:{}", target), leaf.tree_root));
        }
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

fn cmd_reset(args: ResetArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;

    if args.hard {
        // Materialize workspace from last seal tree
        let repo = open_repo()?;
        let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
        if let Some(head_idx) = repo
            .get_timeline_head(&timeline)
            .map_err(|e| e.to_string())?
        {
            if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
                let cas =
                    FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
                let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws.materialize(leaf.tree_root).map_err(|e| e.to_string())?;
                // Clear staging
                let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws_mut.staging.clear();
                ws_mut.save().map_err(|e| e.to_string())?;
                if !quiet {
                    println!("Reset to last seal. Working directory restored.");
                }
                return Ok(());
            }
        }
        return Err("no commits to reset to".into());
    }

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let mut ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if args.files.is_empty() {
        ws.staging.clear();
        if !quiet {
            println!("All files unstaged");
        }
    } else {
        for file in &args.files {
            if ws.staging.unstage(file) && !quiet {
                println!("  unstaged: {}", file);
            }
        }
    }
    ws.save().map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_timeline(args: TimelineArgs, quiet: bool) -> Result<(), String> {
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
            let repo = open_repo()?;
            let current = repo.current_timeline().unwrap_or_default();

            if current == switch_args.name {
                if !quiet {
                    println!("Already on timeline: {}", switch_args.name);
                }
                return Ok(());
            }

            // Verify target exists before any side effects
            let target_head_idx = repo
                .get_timeline_head(&switch_args.name)
                .map_err(|e| e.to_string())?;
            if target_head_idx.is_none() {
                let ref_path = ctx.ivaldi_dir.join("refs/heads").join(&switch_args.name);
                if !ref_path.exists() {
                    return Err(format!("timeline '{}' not found", switch_args.name));
                }
            }

            let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
            let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);

            // ---- Auto-shelve: capture everything dirty about `current` ----
            //
            // We save staging + working-tree changes (Modified, Untracked,
            // Deleted) into a shelf keyed by `current`. This must happen
            // BEFORE materialize, since materialize will rewrite the working
            // tree to look like the target timeline.
            use crate::shelf::{Shelf, ShelfManager, WorkspaceChange};
            let shelf_mgr = ShelfManager::new(&ctx.ivaldi_dir);
            let mut shelved_summary: Vec<String> = Vec::new();

            {
                let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
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
                    let mut counts = (0usize, 0usize, 0usize, staged.len());
                    for c in &workspace_changes {
                        match c {
                            WorkspaceChange::Modified { .. } => counts.0 += 1,
                            WorkspaceChange::Untracked { .. } => counts.1 += 1,
                            WorkspaceChange::Deleted { .. } => counts.2 += 1,
                        }
                    }
                    if counts.0 > 0 {
                        shelved_summary.push(format!("{} modified", counts.0));
                    }
                    if counts.1 > 0 {
                        shelved_summary.push(format!("{} untracked", counts.1));
                    }
                    if counts.2 > 0 {
                        shelved_summary.push(format!("{} deleted", counts.2));
                    }
                    if counts.3 > 0 {
                        shelved_summary.push(format!("{} staged", counts.3));
                    }

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
                } else {
                    // No dirty state — clear any stale shelf so we don't
                    // reapply stale changes if the user switches back.
                    shelf_mgr.remove_shelf(&current).ok();
                }
            }

            // ---- Update HEAD and materialize target tree ----
            repo.switch_timeline(&switch_args.name)
                .map_err(|e| e.to_string())?;

            if let Some(idx) = target_head_idx {
                if let Some(leaf) = repo.get_leaf(idx).map_err(|e| e.to_string())? {
                    let ws_mat = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                    ws_mat
                        .materialize_with_ignore(leaf.tree_root, &ignore_cache)
                        .map_err(|e| format!("failed to materialize timeline: {}", e))?;
                }
            }

            // ---- Restore target's shelf (if any) ----
            let mut restored_summary: Vec<String> = Vec::new();
            {
                let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws_mut.staging.clear();

                if let Ok(Some(shelf)) = shelf_mgr.load_shelf(&switch_args.name) {
                    if !shelf.workspace_changes.is_empty() {
                        ws_mut
                            .apply_changes(&shelf.workspace_changes)
                            .map_err(|e| format!("failed to restore shelved changes: {}", e))?;
                    }
                    for (path, hash) in &shelf.staged_files {
                        ws_mut.staging.stage(path, *hash);
                    }
                    let mut counts = (0usize, 0usize, 0usize, shelf.staged_files.len());
                    for c in &shelf.workspace_changes {
                        match c {
                            WorkspaceChange::Modified { .. } => counts.0 += 1,
                            WorkspaceChange::Untracked { .. } => counts.1 += 1,
                            WorkspaceChange::Deleted { .. } => counts.2 += 1,
                        }
                    }
                    if counts.0 > 0 {
                        restored_summary.push(format!("{} modified", counts.0));
                    }
                    if counts.1 > 0 {
                        restored_summary.push(format!("{} untracked", counts.1));
                    }
                    if counts.2 > 0 {
                        restored_summary.push(format!("{} deleted", counts.2));
                    }
                    if counts.3 > 0 {
                        restored_summary.push(format!("{} staged", counts.3));
                    }
                    shelf_mgr.remove_shelf(&switch_args.name).ok();
                }
                ws_mut.save().map_err(|e| e.to_string())?;
            }

            if !quiet {
                if !shelved_summary.is_empty() {
                    println!(
                        "Auto-shelved on '{}': {}",
                        current,
                        shelved_summary.join(", ")
                    );
                }
                println!("Switched to timeline: {}", switch_args.name);
                if !restored_summary.is_empty() {
                    println!(
                        "Restored shelved changes for '{}': {}",
                        switch_args.name,
                        restored_summary.join(", ")
                    );
                }
            }
            Ok(())
        }
        TimelineCommands::List => {
            let repo = open_repo()?;
            let current = repo.current_timeline().unwrap_or_default();
            let timelines = repo.list_timelines().map_err(|e| e.to_string())?;

            if timelines.is_empty() {
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
            repo.rename_timeline(&current, &rename_args.new_name)
                .map_err(|e| e.to_string())?;
            if !quiet {
                println!(
                    "Renamed timeline: {} → {}",
                    color::dim(&current),
                    color::timeline(&rename_args.new_name)
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

fn cmd_butterfly(args: ButterflyArgs, _ctx: &RepoContext, quiet: bool) -> Result<(), String> {
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

fn cmd_fuse(args: FuseArgs, quiet: bool) -> Result<(), String> {
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
    let strategy = Strategy::from_str(&args.strategy).ok_or(format!(
        "unknown strategy: {}. Options: auto, ours, theirs, union, base",
        args.strategy
    ))?;

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

    let base_files = BTreeMap::new();
    let mut ours_files = BTreeMap::new();
    let mut theirs_files = BTreeMap::new();

    // For simplicity, use empty base (treat as adding all). LCA-based base is ideal but requires
    // walking history which we have. For now, use target as "ours" and source as "theirs".
    collect_blob_hashes(&store, target_leaf.tree_root, "", &mut ours_files)?;
    collect_blob_hashes(&store, source_leaf.tree_root, "", &mut theirs_files)?;

    let result = FuseEngine::fuse(&base_files, &ours_files, &theirs_files, strategy);

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
        let commit_result = repo
            .commit_raw(fuse_leaf, &target)
            .map_err(|e| e.to_string())?;

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

fn collect_blob_hashes(
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

fn cmd_travel(args: TravelArgs) -> Result<(), String> {
    use crate::tui::travel::{TravelAction, run_travel};

    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
    let entries = repo.walk_history(&timeline).map_err(|e| e.to_string())?;

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
                println!(
                    "Timeline '{}' reset to seal at index {}",
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

fn cmd_shift(args: ShiftArgs, quiet: bool) -> Result<(), String> {
    let mut repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());

    if let Some(n) = args.last {
        if n < 2 {
            return Err("need at least 2 commits to squash".into());
        }

        let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;
        if history.len() < n {
            return Err(format!(
                "only {} commits on '{}', need {}",
                history.len(),
                timeline,
                n
            ));
        }

        if !quiet {
            println!("Commits to squash:");
            for entry in history.iter().take(n) {
                println!("  {} - {}", entry.short_hash, entry.message);
            }
        }

        // Use the newest commit's tree and oldest's parent
        let newest = &history[0];
        let oldest = &history[n - 1];
        let tree_root = repo
            .get_leaf(newest.index)
            .map_err(|e| e.to_string())?
            .ok_or("corrupt head")?
            .tree_root;
        let _oldest_leaf = repo
            .get_leaf(oldest.index)
            .map_err(|e| e.to_string())?
            .ok_or("corrupt oldest")?;

        let cfg = repo.config();
        let author = cfg.author().unwrap_or_else(|| newest.author.clone());
        let message = format!(
            "Squashed {} commits:\n\n{}",
            n,
            history
                .iter()
                .take(n)
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );

        let result = repo
            .commit(tree_root, &author, &message)
            .map_err(|e| e.to_string())?;

        if !quiet {
            println!(
                "\n🔨 Created squashed seal: {} ({})",
                result.seal_name,
                result.hash.short8()
            );
            println!("✓ {} commits squashed into 1", n);
        }
    } else if args.start.is_some() || args.end.is_some() {
        let start_q = args.start.as_deref().ok_or("start seal required")?;
        let end_q = args.end.as_deref().unwrap_or("HEAD");
        if !quiet {
            println!("Squashing from {} to {}", start_q, end_q);
        }
    } else {
        // Interactive mode with TUI
        use crate::tui::shift::{ShiftAction, run_shift};

        let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;
        if history.len() < 2 {
            return Err("need at least 2 commits to squash".into());
        }

        let action = run_shift(history).map_err(|e| e.to_string())?;

        match action {
            ShiftAction::Squash {
                start_index: _,
                end_index,
                message,
            } => {
                let newest_leaf = repo
                    .get_leaf(end_index)
                    .map_err(|e| e.to_string())?
                    .ok_or("corrupt")?;
                let cfg = repo.config();
                let author = cfg.author().unwrap_or_else(|| newest_leaf.author.clone());
                let result = repo
                    .commit(newest_leaf.tree_root, &author, &message)
                    .map_err(|e| e.to_string())?;
                println!(
                    "🔨 Created squashed seal: {} ({})",
                    result.seal_name,
                    result.hash.short8()
                );
            }
            ShiftAction::Cancel => println!("Cancelled."),
        }
    }
    Ok(())
}

fn cmd_config(args: ConfigArgs) -> Result<(), String> {
    // Resolve which config file to target and whether we're in a repo.
    let repo_ctx = find_repo().ok();
    let use_global = args.global || repo_ctx.is_none();
    let target_path = if use_global {
        let path = config::global_config_path()
            .ok_or("cannot locate global config: $HOME is not set")?;
        if !args.global && args.set.is_some() {
            // Auto-fallback: the user didn't pass --global but we're outside a repo.
            eprintln!(
                "{}",
                color::dim(&format!(
                    "not in an Ivaldi repo — using global config at {}",
                    path.display()
                ))
            );
        }
        path
    } else {
        // Safe unwrap: repo_ctx is Some when use_global is false.
        repo_ctx.as_ref().unwrap().ivaldi_dir.join("config")
    };

    if args.list {
        // Merged view when inside a repo; annotate provenance.
        let global = config::load_global();
        let local = repo_ctx
            .as_ref()
            .filter(|_| !args.global)
            .map(|ctx| Config::load(&ctx.ivaldi_dir.join("config")).unwrap_or_else(|_| Config::new()));

        let mut merged = Config::new();
        merged.merge(&global);
        if let Some(l) = &local {
            merged.merge(l);
        }

        for (key, value) in merged.list() {
            let provenance = match (&local, global.get(key)) {
                (Some(l), _) if l.get(key).is_some() => "local",
                (_, Some(_)) => "global",
                _ => "default",
            };
            println!("{} = {} {}", color::cyan(key), value, color::dim(&format!("({})", provenance)));
        }
        return Ok(());
    }
    if let Some(key) = &args.get {
        let cfg = if use_global {
            config::load_global()
        } else {
            // Safe unwrap: repo_ctx is Some when use_global is false.
            config::load_config(&repo_ctx.as_ref().unwrap().ivaldi_dir)
        };
        match cfg.get(key) {
            Some(value) => println!("{}", value),
            None => return Err(format!("config key not found: {}", key)),
        }
        return Ok(());
    }
    if let Some(key) = &args.set {
        let value = args.value.as_deref().ok_or("value required for --set")?;
        let mut cfg = Config::load(&target_path).unwrap_or_else(|_| Config::new());
        cfg.set(key, value);
        cfg.save(&target_path).map_err(|e| e.to_string())?;
        let scope = if use_global { "global" } else { "local" };
        println!("{}={} ({})", key, value, scope);
        return Ok(());
    }

    // No flags — launch the interactive form.
    let inside_repo = repo_ctx.is_some() && !args.global;
    crate::tui::config_form::run(&target_path, inside_repo).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_exclude(args: ExcludeArgs, quiet: bool) -> Result<(), String> {
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
    std::fs::write(&ignore_path, &content).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_portal(args: PortalArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mgr = PortalManager::new(&ctx.ivaldi_dir);
    match args.command {
        PortalCommands::Add(add_args) => {
            let spec = parse_repo_arg(&add_args.repo)?;
            let mut portal = Portal {
                owner: spec.owner,
                repo: spec.repo,
                platform: spec.platform,
                base_url: None,
            };
            if add_args.gitlab {
                portal.platform = Platform::GitLab;
            }
            if let Some(url) = add_args.url {
                portal.base_url = Some(url);
            }
            let added = mgr.add(&portal).map_err(|e| e.to_string())?;
            if !quiet {
                if added {
                    println!("Added portal: {}", portal.to_string_repr());
                } else {
                    println!("Portal already configured: {}", portal.to_string_repr());
                }
            }
        }
        PortalCommands::List => {
            let portals = mgr.list().map_err(|e| e.to_string())?;
            if portals.is_empty() {
                println!("No portals configured.");
            } else {
                println!("Configured portals:");
                for p in &portals {
                    let plat = match p.platform {
                        Platform::GitHub => "github",
                        Platform::GitLab => "gitlab",
                    };
                    print!("  {} ({})", p.to_string_repr(), plat);
                    if let Some(url) = &p.base_url {
                        print!(" [{}]", url);
                    }
                    println!();
                }
            }
        }
        PortalCommands::Remove(remove_args) => {
            let removed = mgr.remove(&remove_args.repo).map_err(|e| e.to_string())?;
            if !quiet {
                if removed {
                    println!("Removed portal: {}", remove_args.repo);
                } else {
                    println!("Portal not found: {}", remove_args.repo);
                }
            }
        }
    }
    Ok(())
}

/// Try to open `url` in the user's default browser. Best-effort: returns
/// false when no opener exists, the environment is headless, or the user
/// opted out via IVALDI_NO_BROWSER. The caller still prints the URL so the
/// user can fall back to copy/paste.
fn open_in_browser(url: &str) -> bool {
    if std::env::var_os("IVALDI_NO_BROWSER").is_some() {
        return false;
    }
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .is_ok()
}

fn cmd_auth(args: AuthArgs) -> Result<(), String> {
    match args.command {
        AuthCommands::Login(login_args) => {
            use crate::auth::TokenStore;
            use crate::github::GitHubClient;

            if login_args.gitlab {
                println!(
                    "GitLab OAuth not yet implemented. Set GITLAB_TOKEN environment variable."
                );
                return Ok(());
            }

            println!("Initiating GitHub authentication...");
            let device_code = GitHubClient::request_device_code().map_err(|e| e.to_string())?;

            println!(
                "\nFirst, copy your one-time code: {}",
                device_code.user_code
            );
            if open_in_browser(&device_code.verification_uri) {
                println!(
                    "Opened {} in your browser.",
                    device_code.verification_uri
                );
                println!("(If nothing opened, visit the URL above manually.)");
            } else {
                println!("Then visit: {}", device_code.verification_uri);
            }
            println!("\nWaiting for authentication...");

            let token =
                GitHubClient::poll_for_token(&device_code.device_code, device_code.interval)
                    .map_err(|e| e.to_string())?;

            let store = TokenStore::new().map_err(|e| e.to_string())?;
            store
                .save_token(Platform::GitHub, token)
                .map_err(|e| e.to_string())?;
            println!("\nAuthentication successful!");
        }
        AuthCommands::Status => {
            use crate::auth;
            for (platform, name) in &[(Platform::GitHub, "GitHub"), (Platform::GitLab, "GitLab")] {
                if let Some(method) = auth::resolve_auth(*platform) {
                    println!("{}: {}", name, method.description);
                } else {
                    println!("{}: Not authenticated", name);
                }
            }
        }
        AuthCommands::Logout(logout_args) => {
            use crate::auth::TokenStore;
            let platform = if logout_args.gitlab {
                Platform::GitLab
            } else {
                Platform::GitHub
            };
            match TokenStore::new() {
                Ok(store) => {
                    store.delete_token(platform).map_err(|e| e.to_string())?;
                    println!("Logged out successfully");
                }
                Err(e) => println!("Warning: {}", e),
            }
        }
    }
    Ok(())
}

fn cmd_download(args: DownloadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let spec = parse_repo_arg(&args.repo)?;
    let client = GitHubClient::new();

    let target_dir = std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&spec.repo));
    let branch = spec.branch_hint.as_deref();

    let result = sync::download(&client, &spec.owner, &spec.repo, &target_dir, branch)
        .map_err(|e| e.to_string())?;

    if !quiet {
        println!(
            "Cloned {}/{} → {}",
            spec.owner,
            spec.repo,
            target_dir.display()
        );
        println!("  {} files downloaded", result.files_downloaded);
    }
    Ok(())
}

fn cmd_upload(args: UploadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let repo = open_repo()?;
    let client = GitHubClient::new();

    if !client.is_authenticated() {
        return Err("not authenticated. Run 'ivaldi auth login' or set GITHUB_TOKEN.".into());
    }

    // Get portal
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    if args.force {
        print!("WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: ");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|e| e.to_string())?;
        if input.trim() != "force push" {
            println!("Aborted.");
            return Ok(());
        }
    }

    let result = sync::upload(
        &client,
        &repo,
        &portal.owner,
        &portal.repo,
        args.branch.as_deref(),
        args.force,
    )
    .map_err(|e| e.to_string())?;

    if !quiet {
        println!(
            "Uploaded to {}/{} (branch: {})",
            portal.owner, portal.repo, result.branch
        );
        println!("  {} files uploaded", result.files_uploaded);
        println!("  commit: {}", result.commit_sha);
    }
    Ok(())
}

fn cmd_scout(_args: ScoutArgs) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    let branches = sync::scout_with_status(&client, &repo, &portal.owner, &portal.repo)
        .map_err(|e| e.to_string())?;

    println!("Remote timelines available:");
    for branch in &branches {
        let status = match branch.state {
            sync::RemoteTimelineState::NotDownloaded => "not downloaded",
            sync::RemoteTimelineState::UpToDate => "local, up to date",
            sync::RemoteTimelineState::OutOfSync => "local, out of sync",
            sync::RemoteTimelineState::LocalOnly => "local, no remote mapping",
        };
        println!("  {} [{}]", branch.name, status);
    }
    println!("\nUse 'ivaldi harvest <name>' to download");
    Ok(())
}

fn cmd_harvest(args: HarvestArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let mut repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    if args.timelines.is_empty() {
        // List available and prompt
        let branches =
            sync::scout(&client, &portal.owner, &portal.repo).map_err(|e| e.to_string())?;
        println!("Available remote timelines:");
        for b in &branches {
            println!("  {}", b);
        }
        println!("\nSpecify timelines to harvest: ivaldi harvest <name> [<name>...]");
        return Ok(());
    }

    let harvested = sync::harvest(
        &client,
        &mut repo,
        &portal.owner,
        &portal.repo,
        &args.timelines,
    )
    .map_err(|e| e.to_string())?;

    if !quiet {
        for name in &harvested {
            println!("Harvested timeline: {}", name);
        }
    }
    Ok(())
}

fn cmd_sync(args: SyncArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let mut repo = open_repo()?;
    let client = GitHubClient::new();
    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    let timeline = args
        .timeline
        .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

    if !quiet {
        println!(
            "Syncing timeline '{}' with {}/{}...\n",
            color::timeline(&timeline),
            portal.owner,
            portal.repo
        );
    }

    let result = sync::sync_timeline(&client, &mut repo, &portal.owner, &portal.repo, &timeline)
        .map_err(|e| e.to_string())?;

    if result.no_changes {
        println!(
            "{} Timeline '{}' is already up to date",
            color::green("✓"),
            color::bold(&timeline)
        );
        return Ok(());
    }

    for f in &result.added {
        println!("{} {}", color::green("++"), f);
    }
    for f in &result.modified {
        println!("{} {}", color::green("++"), f);
    }
    for f in &result.deleted {
        println!("{} {}", color::red("--"), f);
    }

    let total = result.added.len() + result.modified.len() + result.deleted.len();
    println!(
        "\n{} Synced {} file(s) from remote",
        color::green("✓"),
        total
    );
    if !result.added.is_empty() {
        println!("  Added: {}", color::green(&result.added.len().to_string()));
    }
    if !result.modified.is_empty() {
        println!(
            "  Modified: {}",
            color::blue(&result.modified.len().to_string())
        );
    }
    if !result.deleted.is_empty() {
        println!(
            "  Deleted: {}",
            color::red(&result.deleted.len().to_string())
        );
    }

    Ok(())
}

/// Format a change marker for display.
/// Returns (marker, color_fn) for the given ChangeKind.
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
pub(crate) fn state_marker(state: FileState) -> (&'static str, fn(&str) -> String) {
    match state {
        FileState::Modified => ("~~", color::bold_yellow),
        FileState::Untracked => ("++", color::bold_green),
        FileState::Deleted => ("--", color::bold_red),
        FileState::Staged => ("++", color::bold_green),
        _ => ("??", color::dim),
    }
}

// Helper: parse a repo identifier from a CLI arg.
// Accepts owner/repo, full URLs, SSH URLs, github:/gitlab: shorthand.
fn parse_repo_arg(arg: &str) -> Result<crate::portal::RepoSpec, String> {
    crate::portal::parse_repo_spec(arg).map_err(|e| {
        format!(
            "invalid repository: '{}' ({})\n  accepted formats:\n    owner/repo\n    https://github.com/owner/repo\n    git@github.com:owner/repo.git\n    github:owner/repo",
            arg, e
        )
    })
}

fn cmd_review(args: ReviewArgs, quiet: bool) -> Result<(), String> {
    use crate::review::{self, ReviewFilter, ReviewStatus};

    match args.command {
        ReviewCommands::Create(create_args) => {
            let repo = open_repo()?;
            let review = review::create_review(
                &repo,
                &create_args.title,
                &create_args.description,
                &create_args.source,
                &create_args.target,
                &create_args.strategy,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!(
                    "Created review #{}: {} ({} -> {})",
                    review.id, review.title, review.source_timeline, review.target_timeline
                );
            }
            Ok(())
        }
        ReviewCommands::List(list_args) => {
            let repo = open_repo()?;
            let filter = if list_args.all {
                ReviewFilter::default()
            } else if let Some(ref status_str) = list_args.status {
                ReviewFilter {
                    status: Some(
                        ReviewStatus::from_str(status_str)
                            .ok_or(format!("unknown status: {}", status_str))?,
                    ),
                }
            } else {
                // Default: show non-merged, non-closed
                ReviewFilter::default()
            };

            let reviews = review::list_reviews(&repo, &filter).map_err(|e| e.to_string())?;

            // When not --all and no explicit status, filter out merged/closed
            let reviews: Vec<_> = if !list_args.all && list_args.status.is_none() {
                reviews
                    .into_iter()
                    .filter(|r| {
                        r.status != ReviewStatus::Merged && r.status != ReviewStatus::Closed
                    })
                    .collect()
            } else {
                reviews
            };

            if reviews.is_empty() {
                println!("No reviews found.");
            } else {
                for r in &reviews {
                    println!(
                        "[{}] #{} {} ({} -> {}) by {}",
                        r.status.symbol(),
                        r.id,
                        r.title,
                        r.source_timeline,
                        r.target_timeline,
                        r.author,
                    );
                }
                println!("\n{} review(s)", reviews.len());
            }
            Ok(())
        }
        ReviewCommands::Show(show_args) => {
            let repo = open_repo()?;
            let review = repo
                .load_review(show_args.id)
                .map_err(|e| e.to_string())?
                .ok_or(format!("review #{} not found", show_args.id))?;

            println!("Review #{}: {}", review.id, review.title);
            println!("Status:  {}", review.status);
            println!("Author:  {}", review.author);
            println!(
                "Source:  {} ({})",
                review.source_timeline, review.source_head_seal
            );
            println!(
                "Target:  {} ({})",
                review.target_timeline, review.target_head_seal
            );
            println!("Strategy: {}", review.fuse_strategy);
            if let Some(ref seal) = review.merge_seal {
                println!("Merged:  {}", seal);
            }
            if !review.description.is_empty() {
                println!("\n{}", review.description);
            }

            if !review.comments.is_empty() {
                println!("\n--- Comments ({}) ---", review.comments.len());
                for c in &review.comments {
                    let location = if let Some(line) = c.line {
                        format!("{}:{}", c.path, line)
                    } else {
                        c.path.clone()
                    };
                    let reply = if let Some(rid) = c.reply_to {
                        format!(" (reply to #{})", rid)
                    } else {
                        String::new()
                    };
                    println!("  [{}] {} @ {}{}", c.id, c.author, location, reply);
                    println!("    {}", c.body);
                }
            }

            if !review.verdicts.is_empty() {
                println!("\n--- Verdicts ({}) ---", review.verdicts.len());
                for v in &review.verdicts {
                    println!(
                        "  {} - {} {}",
                        v.status,
                        v.author,
                        if v.body.is_empty() { "" } else { &v.body }
                    );
                }
            }
            Ok(())
        }
        ReviewCommands::Diff(diff_args) => {
            let repo = open_repo()?;
            let changes = review::review_diff(&repo, diff_args.id).map_err(|e| e.to_string())?;

            if changes.is_empty() {
                println!("No changes between source and target.");
                return Ok(());
            }

            if diff_args.stat {
                let mut added = 0usize;
                let mut deleted = 0usize;
                let mut modified = 0usize;
                for c in &changes {
                    match c.kind {
                        crate::fsmerkle::ChangeKind::Added => added += 1,
                        crate::fsmerkle::ChangeKind::Deleted => deleted += 1,
                        crate::fsmerkle::ChangeKind::Modified
                        | crate::fsmerkle::ChangeKind::TypeChange => modified += 1,
                    }
                }
                println!(
                    "{} file(s) changed: {} added, {} deleted, {} modified",
                    changes.len(),
                    added,
                    deleted,
                    modified
                );
            } else {
                for c in &changes {
                    let marker = match c.kind {
                        crate::fsmerkle::ChangeKind::Added => "++",
                        crate::fsmerkle::ChangeKind::Deleted => "--",
                        crate::fsmerkle::ChangeKind::Modified
                        | crate::fsmerkle::ChangeKind::TypeChange => "~~",
                    };
                    println!("{} {}", marker, c.path);
                }
            }
            Ok(())
        }
        ReviewCommands::Comment(comment_args) => {
            let repo = open_repo()?;
            review::add_comment(
                &repo,
                comment_args.id,
                &comment_args.file,
                comment_args.line,
                &comment_args.body,
                comment_args.reply_to,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Comment added to review #{}", comment_args.id);
            }
            Ok(())
        }
        ReviewCommands::Approve(approve_args) => {
            let repo = open_repo()?;
            review::submit_verdict(
                &repo,
                approve_args.id,
                ReviewStatus::Approved,
                &approve_args.body,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} approved", approve_args.id);
            }
            Ok(())
        }
        ReviewCommands::RequestChanges(rc_args) => {
            let repo = open_repo()?;
            review::submit_verdict(
                &repo,
                rc_args.id,
                ReviewStatus::ChangesRequested,
                &rc_args.body,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Changes requested on review #{}", rc_args.id);
            }
            Ok(())
        }
        ReviewCommands::Merge(merge_args) => {
            let mut repo = open_repo()?;

            // Optionally override the strategy stored in the review
            if let Some(ref strategy) = merge_args.strategy {
                let mut review = repo
                    .load_review(merge_args.id)
                    .map_err(|e| e.to_string())?
                    .ok_or(format!("review #{} not found", merge_args.id))?;
                review.fuse_strategy = strategy.clone();
                repo.save_review(&review).map_err(|e| e.to_string())?;
            }

            let review =
                review::merge_review(&mut repo, merge_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!(
                    "Review #{} merged! Seal: {}",
                    review.id,
                    review.merge_seal.as_deref().unwrap_or("unknown")
                );
            }
            Ok(())
        }
        ReviewCommands::Close(close_args) => {
            let repo = open_repo()?;
            review::close_review(&repo, close_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} closed", close_args.id);
            }
            Ok(())
        }
        ReviewCommands::Reopen(reopen_args) => {
            let repo = open_repo()?;
            review::reopen_review(&repo, reopen_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} reopened", reopen_args.id);
            }
            Ok(())
        }
    }
}

fn cmd_tui() -> Result<(), String> {
    if let Ok(ctx) = find_repo() {
        return crate::tui::app::run(&ctx.work_dir, &ctx.ivaldi_dir);
    }

    // No repository here — drop the user into the launcher so they can
    // download, forge, or open one without going back to the CLI.
    use crate::tui::launcher::{self, LauncherChoice};
    let choice = launcher::run().map_err(|e| format!("Launcher error: {}", e))?;

    match choice {
        LauncherChoice::Quit => Ok(()),
        LauncherChoice::Download {
            repo_arg,
            target_dir,
        } => {
            // Same path the CLI's `cmd_download` takes, just inlined so we can
            // chain into the dashboard on success.
            let spec = parse_repo_arg(&repo_arg)?;
            let client = crate::github::GitHubClient::new();
            let branch = spec.branch_hint.as_deref();
            crate::sync::download(&client, &spec.owner, &spec.repo, &target_dir, branch)
                .map_err(|e| e.to_string())?;
            crate::tui::app::run(&target_dir, &target_dir.join(".ivaldi"))
        }
        LauncherChoice::Forge { target_dir } => {
            std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
            forge::forge(&target_dir).map_err(|e| e.to_string())?;
            crate::tui::app::run(&target_dir, &target_dir.join(".ivaldi"))
        }
        LauncherChoice::Open { target_dir } => {
            let ivaldi_dir = target_dir.join(".ivaldi");
            if !ivaldi_dir.join("HEAD").exists() {
                return Err(format!(
                    "{} is not an Ivaldi repository",
                    target_dir.display()
                ));
            }
            crate::tui::app::run(&target_dir, &ivaldi_dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsmerkle::ChangeKind;
    use crate::workspace::FileState;

    #[test]
    fn status_markers_added() {
        let (marker, _) = state_marker(FileState::Untracked);
        assert_eq!(marker, "++");
    }

    #[test]
    fn status_markers_modified() {
        let (marker, _) = state_marker(FileState::Modified);
        assert_eq!(marker, "~~");
    }

    #[test]
    fn status_markers_deleted() {
        let (marker, _) = state_marker(FileState::Deleted);
        assert_eq!(marker, "--");
    }

    #[test]
    fn status_markers_staged() {
        let (marker, _) = state_marker(FileState::Staged);
        assert_eq!(marker, "++");
    }

    #[test]
    fn change_marker_added() {
        let (marker, _) = change_marker(ChangeKind::Added);
        assert_eq!(marker, "++");
    }

    #[test]
    fn change_marker_deleted() {
        let (marker, _) = change_marker(ChangeKind::Deleted);
        assert_eq!(marker, "--");
    }

    #[test]
    fn change_marker_modified() {
        let (marker, _) = change_marker(ChangeKind::Modified);
        assert_eq!(marker, "~~");
    }

    #[test]
    fn change_marker_typechange() {
        let (marker, _) = change_marker(ChangeKind::TypeChange);
        assert_eq!(marker, "~~");
    }

    #[test]
    fn diff_two_trees_with_markers() {
        use crate::cas::MemoryCas;
        use crate::fsmerkle::{self, FsStore};
        use std::collections::BTreeMap;

        let cas = MemoryCas::new();
        let store = FsStore::new(&cas);

        let mut files_a = BTreeMap::new();
        files_a.insert("kept.txt".into(), b"same".to_vec());
        files_a.insert("changed.txt".into(), b"old content".to_vec());
        files_a.insert("removed.txt".into(), b"gone".to_vec());
        let tree_a = store.build_tree_from_map(&files_a).unwrap();

        let mut files_b = BTreeMap::new();
        files_b.insert("kept.txt".into(), b"same".to_vec());
        files_b.insert("changed.txt".into(), b"new content".to_vec());
        files_b.insert("added.txt".into(), b"fresh".to_vec());
        let tree_b = store.build_tree_from_map(&files_b).unwrap();

        let changes = fsmerkle::diff_trees(tree_a, tree_b, &store).unwrap();
        assert_eq!(changes.len(), 3);

        let mut added = 0;
        let mut modified = 0;
        let mut deleted = 0;
        for c in &changes {
            let (marker, _) = change_marker(c.kind);
            match marker {
                "++" => added += 1,
                "~~" => modified += 1,
                "--" => deleted += 1,
                _ => panic!("unexpected marker"),
            }
        }
        assert_eq!(added, 1);
        assert_eq!(modified, 1);
        assert_eq!(deleted, 1);
    }
}
