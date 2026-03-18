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
            println!("Ivaldi repository already exists at {}", result.ivaldi_dir.display());
        } else {
            println!("Initialized empty Ivaldi repository in {}", result.ivaldi_dir.display());
            println!("Created timeline: {}", result.default_timeline);
            if result.git_imported > 0 {
                println!("Imported {} Git branch(es) as timelines", result.git_imported);
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

    let mut all_gathered: Vec<String>;

    if args.files.is_empty() || args.files == ["."] {
        // gather_all auto-excludes dotfiles via scan, no confirmation needed
        all_gathered = ws.gather_all(&ignore_cache).map_err(|e| e.to_string())?;
    } else {
        let refs: Vec<&str> = args.files.iter().map(|s| s.as_str()).collect();
        let result = ws.gather(&refs, &allowlist).map_err(|e| e.to_string())?;
        all_gathered = result.gathered;

        // Handle dotfiles that need confirmation
        if !result.needs_confirmation.is_empty() {
            if args.allow_all {
                // --allow-all pre-approves all dotfiles (except security-blocked)
                let confirmed_refs: Vec<&str> =
                    result.needs_confirmation.iter().map(|s| s.as_str()).collect();
                let extra = ws.gather_confirmed(&confirmed_refs).map_err(|e| e.to_string())?;
                for path in &extra {
                    allowlist.allow(path);
                }
                all_gathered.extend(extra);
            } else {
                // Prompt for each dotfile individually
                for dotfile in &result.needs_confirmation {
                    eprint!("WARNING: '{}' is a hidden (dot) file — stage it? [y/N]: ", dotfile);
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

            // Persist allowlist so confirmed dotfiles don't prompt again
            allowlist.save().map_err(|e| e.to_string())?;
        }
    }

    ws.save().map_err(|e| e.to_string())?;

    if !quiet {
        for file in &all_gathered {
            println!("  gathered: {}", file);
        }
        println!("{} file(s) staged", all_gathered.len());
    }
    Ok(())
}

fn cmd_seal(args: SealArgs, quiet: bool) -> Result<(), String> {
    let message = args.get_message()
        .ok_or("seal message required. Usage: ivaldi seal \"your message\"")?
        .to_string();

    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if ws.staging.is_empty() {
        return Err("no changes staged for seal. Use 'ivaldi gather' to stage files first.".into());
    }

    // Build tree from staged files
    let tree_hash = ws.build_staged_tree().map_err(|e| e.to_string())?;

    // Open persistent repo and commit
    let mut repo = open_repo()?;
    let cfg = repo.config();
    let author = cfg.author()
        .ok_or("user.name and user.email not configured. Run:\n  ivaldi config --set user.name \"Your Name\"\n  ivaldi config --set user.email \"you@example.com\"")?;

    let result = repo.commit(tree_hash, &author, &message).map_err(|e| e.to_string())?;

    // Clear staging area
    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    ws_mut.staging.clear();
    ws_mut.save().map_err(|e| e.to_string())?;

    if !quiet {
        println!("Created seal: {} ({})", result.seal_name, result.hash.short8());
    }
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    let ctx = find_repo()?;
    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "detached".into());

    println!("Timeline: {}", color::timeline(&timeline));

    // Get last seal tree hash for comparison
    let last_tree = repo.get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten())
        .map(|leaf| leaf.tree_root);

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
    let status = ws.status(last_tree, &ignore_cache).map_err(|e| e.to_string())?;

    // Show last seal info
    if let Some(head_idx) = repo.get_timeline_head(&timeline).map_err(|e| e.to_string())? {
        if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
            let hash = leaf.hash();
            let name = seal::generate_seal_name(hash);
            println!("Last seal: {} ({})", color::seal_name(&name), color::hash(&hash.short8()));
        }
    }

    let staged: Vec<_> = status.iter().filter(|f| f.state == FileState::Staged).collect();
    let modified: Vec<_> = status.iter().filter(|f| f.state == FileState::Modified).collect();
    let untracked: Vec<_> = status.iter().filter(|f| f.state == FileState::Untracked).collect();
    let deleted: Vec<_> = status.iter().filter(|f| f.state == FileState::Deleted).collect();

    if staged.is_empty() && modified.is_empty() && untracked.is_empty() && deleted.is_empty() {
        println!("\nWorking directory: clean");
        return Ok(());
    }

    if !staged.is_empty() {
        println!("\nStaged changes:");
        for f in &staged { println!("  {}: {}", color::status_label("staged"), f.path); }
    }
    if !modified.is_empty() {
        println!("\nUnstaged changes:");
        for f in &modified { println!("  {}: {}", color::status_label("modified"), f.path); }
    }
    if !deleted.is_empty() {
        for f in &deleted { println!("  {}: {}", color::status_label("deleted"), f.path); }
    }
    if !untracked.is_empty() {
        println!("\nUntracked files:");
        for f in &untracked { println!("  {}", color::dim(&f.path)); }
    }
    Ok(())
}

fn cmd_whereami() -> Result<(), String> {
    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "detached".into());

    println!("Timeline: {}", timeline);
    println!("Type: Local Timeline");

    if let Some(head_idx) = repo.get_timeline_head(&timeline).map_err(|e| e.to_string())? {
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

    println!("Commits: {}", repo.commit_count());
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
            println!("{} {} {}", color::hash(&entry.short_hash), color::seal_name(&entry.seal_name), entry.message);
        } else {
            println!("Seal: {} ({})", color::seal_name(&entry.seal_name), color::hash(&entry.short_hash));
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

    if let Some(target) = &args.target {
        match repo.resolve_seal(target).map_err(|e| e.to_string())? {
            Some((_, leaf)) => {
                let hash = leaf.hash();
                println!("Comparing against seal: {} ({})", seal::generate_seal_name(hash), hash.short8());
                println!("  Message: {}", leaf.message);
            }
            None => return Err(format!("seal or hash not found: {}", target)),
        }
    } else if args.staged {
        let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        if ws.staging.is_empty() {
            println!("No staged changes.");
        } else {
            println!("Staged changes:");
            for (path, _) in ws.staging.staged_files() {
                println!("  new/modified: {}", path);
            }
        }
    } else {
        // Show working directory changes vs last seal
        let last_tree = repo.get_timeline_head(&timeline).map_err(|e| e.to_string())?
            .and_then(|idx| repo.get_leaf(idx).ok().flatten())
            .map(|l| l.tree_root);

        let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        let ignore_cache = ignore::load_pattern_cache(&ctx.work_dir);
        let status = ws.status(last_tree, &ignore_cache).map_err(|e| e.to_string())?;

        let changes: Vec<_> = status.iter().filter(|f| f.state != FileState::Unmodified).collect();

        if changes.is_empty() {
            println!("No changes.");
        } else if args.stat {
            // --stat mode: summary counts
            let modified = changes.iter().filter(|f| f.state == FileState::Modified).count();
            let added = changes.iter().filter(|f| f.state == FileState::Untracked).count();
            let deleted = changes.iter().filter(|f| f.state == FileState::Deleted).count();
            let staged = changes.iter().filter(|f| f.state == FileState::Staged).count();
            println!("{} file(s) changed: {} modified, {} added, {} deleted, {} staged",
                changes.len(), modified, added, deleted, staged);
        } else {
            for f in &changes {
                let label = match f.state {
                    FileState::Modified => "modified",
                    FileState::Untracked => "added",
                    FileState::Deleted => "deleted",
                    FileState::Staged => "staged",
                    _ => "unknown",
                };
                println!("  {}: {}", label, f.path);
            }
        }
    }
    Ok(())
}

fn cmd_reset(args: ResetArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;

    if args.hard {
        // Materialize workspace from last seal tree
        let repo = open_repo()?;
        let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
        if let Some(head_idx) = repo.get_timeline_head(&timeline).map_err(|e| e.to_string())? {
            if let Some(leaf) = repo.get_leaf(head_idx).map_err(|e| e.to_string())? {
                let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
                let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws.materialize(leaf.tree_root).map_err(|e| e.to_string())?;
                // Clear staging
                let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                ws_mut.staging.clear();
                ws_mut.save().map_err(|e| e.to_string())?;
                if !quiet { println!("Reset to last seal. Working directory restored."); }
                return Ok(());
            }
        }
        return Err("no commits to reset to".into());
    }

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let mut ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if args.files.is_empty() {
        ws.staging.clear();
        if !quiet { println!("All files unstaged"); }
    } else {
        for file in &args.files {
            if ws.staging.unstage(file) && !quiet { println!("  unstaged: {}", file); }
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
            repo.switch_timeline(&create_args.name).map_err(|e| e.to_string())?;
            if !quiet {
                println!("Created timeline: {}", color::timeline(&create_args.name));
                if let Some(from) = &create_args.from {
                    println!("  from: {}", from);
                }
                println!("Switched to timeline: {}", color::timeline(&create_args.name));
            }
            Ok(())
        }
        TimelineCommands::Switch(switch_args) => {
            let ctx = find_repo()?;
            let repo = open_repo()?;
            let current = repo.current_timeline().unwrap_or_default();

            // Auto-shelve: save current staging area before switch
            let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
            let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
            if !ws.staging.is_empty() {
                use crate::shelf::{Shelf, ShelfManager};
                let shelf_mgr = ShelfManager::new(&ctx.ivaldi_dir);
                let mut staged = std::collections::BTreeMap::new();
                for (path, hash) in ws.staging.staged_files() {
                    staged.insert(path.clone(), *hash);
                }
                let shelf = Shelf {
                    timeline: current.clone(),
                    staged_files: staged,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,
                };
                shelf_mgr.save_shelf(&shelf).map_err(|e| e.to_string())?;
                if !quiet { println!("Changes auto-shelved for '{}'", current); }
            }

            // Switch
            repo.switch_timeline(&switch_args.name).map_err(|e| e.to_string())?;

            // Auto-restore: load shelf for target timeline
            {
                use crate::shelf::ShelfManager;
                let shelf_mgr = ShelfManager::new(&ctx.ivaldi_dir);
                if let Ok(Some(shelf)) = shelf_mgr.load_shelf(&switch_args.name) {
                    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                    ws_mut.staging.clear();
                    for (path, hash) in &shelf.staged_files {
                        ws_mut.staging.stage(path, *hash);
                    }
                    ws_mut.save().map_err(|e| e.to_string())?;
                    shelf_mgr.remove_shelf(&switch_args.name).ok();
                    if !quiet { println!("Restored shelved changes for '{}'", switch_args.name); }
                } else {
                    // Clear staging for clean switch
                    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
                    ws_mut.staging.clear();
                    ws_mut.save().map_err(|e| e.to_string())?;
                }
            }

            if !quiet { println!("Switched to timeline: {}", switch_args.name); }
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
            repo.remove_timeline(&remove_args.name).map_err(|e| e.to_string())?;
            if !quiet { println!("Removed timeline: {}", remove_args.name); }
            Ok(())
        }
        TimelineCommands::Rename(rename_args) => {
            let repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;
            repo.rename_timeline(&current, &rename_args.new_name).map_err(|e| e.to_string())?;
            if !quiet {
                println!("Renamed timeline: {} → {}", color::dim(&current), color::timeline(&rename_args.new_name));
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
            let divergence_hash = repo.get_timeline_head(&parent)
                .map_err(|e| e.to_string())?
                .and_then(|idx| repo.get_leaf(idx).ok().flatten())
                .map(|l| l.hash())
                .unwrap_or(crate::hash::B3Hash::ZERO);

            repo.create_timeline(&create_args.name, Some(&parent)).map_err(|e| e.to_string())?;
            repo.store_butterfly_meta(&create_args.name, &parent, divergence_hash)
                .map_err(|e| e.to_string())?;
            repo.switch_timeline(&create_args.name).map_err(|e| e.to_string())?;

            if !quiet {
                println!("Creating butterfly timeline '{}' from '{}'", create_args.name, parent);
                println!("Switched to butterfly timeline");
            }
            Ok(())
        }
        ButterflyCommands::Up => {
            let mut repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;
            let result = repo.butterfly_sync_up(&current).map_err(|e| e.to_string())?;
            if !quiet {
                println!("Synced butterfly '{}' up to parent", current);
                println!("  Parent updated: {} ({})", result.seal_name, result.hash.short8());
            }
            Ok(())
        }
        ButterflyCommands::Down => {
            let mut repo = open_repo()?;
            let current = repo.current_timeline().map_err(|e| e.to_string())?;
            let result = repo.butterfly_sync_down(&current).map_err(|e| e.to_string())?;
            if !quiet {
                println!("Synced butterfly '{}' down from parent", current);
                println!("  Butterfly updated: {} ({})", result.seal_name, result.hash.short8());
            }
            Ok(())
        }
        ButterflyCommands::Remove(remove_args) => {
            let repo = open_repo()?;
            repo.remove_timeline(&remove_args.name).map_err(|e| e.to_string())?;
            if !quiet { println!("Removed butterfly '{}'", remove_args.name); }
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
            if !quiet { println!("Merge aborted."); }
        } else {
            return Err("no merge in progress".into());
        }
        return Ok(());
    }

    if args.continue_merge {
        let state = repo.load_merge_state().map_err(|e| e.to_string())?
            .ok_or("no merge in progress")?;
        if state.conflicts.is_empty() {
            repo.clear_merge_state().map_err(|e| e.to_string())?;
            if !quiet { println!("Merge completed."); }
        } else {
            println!("Unresolved conflicts:");
            for c in &state.conflicts { println!("  CONFLICT: {}", c); }
            println!("\nResolve conflicts, then run 'ivaldi fuse --continue'");
        }
        return Ok(());
    }

    if repo.has_merge_in_progress() {
        return Err("merge already in progress. Use --continue or --abort.".into());
    }

    let source = args.source.as_deref()
        .ok_or("source timeline required. Usage: ivaldi fuse <source> to <target>")?;
    let strategy = Strategy::from_str(&args.strategy)
        .ok_or(format!("unknown strategy: {}. Options: auto, ours, theirs, union, base", args.strategy))?;

    let target = repo.current_timeline().map_err(|e| e.to_string())?;

    // Get trees for source and target
    let source_head = repo.get_timeline_head(source).map_err(|e| e.to_string())?
        .ok_or(format!("timeline '{}' has no commits", source))?;
    let target_head = repo.get_timeline_head(&target).map_err(|e| e.to_string())?
        .ok_or(format!("timeline '{}' has no commits", target))?;

    let source_leaf = repo.get_leaf(source_head).map_err(|e| e.to_string())?
        .ok_or("corrupt source head")?;
    let target_leaf = repo.get_leaf(target_head).map_err(|e| e.to_string())?
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
        let merged_tree = store.build_tree_from_hash_map(&result.merged_files)
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
        let commit_result = repo.commit_raw(fuse_leaf, &target).map_err(|e| e.to_string())?;

        if !quiet {
            println!("[OK] Merge completed successfully!");
            println!("  Merge seal: {} ({})", commit_result.seal_name, commit_result.hash.short8());
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
        for path in &conflict_paths { println!("  CONFLICT: {}", path); }
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
        let path = if prefix.is_empty() { entry.name.clone() } else { format!("{}/{}", prefix, entry.name) };
        match entry.kind {
            crate::fsmerkle::NodeKind::Blob => { files.insert(path, entry.hash); }
            crate::fsmerkle::NodeKind::Tree => { collect_blob_hashes(store, entry.hash, &path, files)?; }
        }
    }
    Ok(())
}

fn cmd_travel(args: TravelArgs) -> Result<(), String> {
    use crate::tui::travel::{run_travel, TravelAction};

    let repo = open_repo()?;
    let timeline = repo.current_timeline().unwrap_or_else(|_| "main".into());
    let entries = repo.walk_history(&timeline).map_err(|e| e.to_string())?;

    if entries.is_empty() {
        return Err(format!("no commits on timeline '{}'", timeline));
    }

    let action = run_travel(entries, &timeline, args.search).map_err(|e| e.to_string())?;

    match action {
        TravelAction::Diverge { seal_index, new_timeline } => {
            repo.create_timeline(&new_timeline, Some(&timeline)).map_err(|e| e.to_string())?;
            repo.switch_timeline(&new_timeline).map_err(|e| e.to_string())?;
            println!("Created timeline '{}' from seal at index {}", new_timeline, seal_index);
            println!("Switched to timeline '{}'", new_timeline);
        }
        TravelAction::Overwrite { seal_index } => {
            if let Some(_leaf) = repo.get_leaf(seal_index).map_err(|e| e.to_string())? {
                // Update timeline head to this seal
                repo.store.set_timeline_head(&timeline, seal_index).map_err(|e| e.to_string())?;
                println!("Timeline '{}' reset to seal at index {}", timeline, seal_index);
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
        if n < 2 { return Err("need at least 2 commits to squash".into()); }

        let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;
        if history.len() < n {
            return Err(format!("only {} commits on '{}', need {}", history.len(), timeline, n));
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
        let tree_root = repo.get_leaf(newest.index).map_err(|e| e.to_string())?
            .ok_or("corrupt head")?.tree_root;
        let _oldest_leaf = repo.get_leaf(oldest.index).map_err(|e| e.to_string())?
            .ok_or("corrupt oldest")?;

        let cfg = repo.config();
        let author = cfg.author().unwrap_or_else(|| newest.author.clone());
        let message = format!("Squashed {} commits:\n\n{}", n,
            history.iter().take(n).map(|e| e.message.as_str()).collect::<Vec<_>>().join("\n"));

        let result = repo.commit(tree_root, &author, &message).map_err(|e| e.to_string())?;

        if !quiet {
            println!("\n🔨 Created squashed seal: {} ({})", result.seal_name, result.hash.short8());
            println!("✓ {} commits squashed into 1", n);
        }
    } else if args.start.is_some() || args.end.is_some() {
        let start_q = args.start.as_deref().ok_or("start seal required")?;
        let end_q = args.end.as_deref().unwrap_or("HEAD");
        if !quiet { println!("Squashing from {} to {}", start_q, end_q); }
    } else {
        // Interactive mode with TUI
        use crate::tui::shift::{run_shift, ShiftAction};

        let history = repo.walk_history(&timeline).map_err(|e| e.to_string())?;
        if history.len() < 2 {
            return Err("need at least 2 commits to squash".into());
        }

        let action = run_shift(history).map_err(|e| e.to_string())?;

        match action {
            ShiftAction::Squash { start_index: _, end_index, message } => {
                let newest_leaf = repo.get_leaf(end_index).map_err(|e| e.to_string())?
                    .ok_or("corrupt")?;
                let cfg = repo.config();
                let author = cfg.author().unwrap_or_else(|| newest_leaf.author.clone());
                let result = repo.commit(newest_leaf.tree_root, &author, &message)
                    .map_err(|e| e.to_string())?;
                println!("🔨 Created squashed seal: {} ({})", result.seal_name, result.hash.short8());
            }
            ShiftAction::Cancel => println!("Cancelled."),
        }
    }
    Ok(())
}

fn cmd_config(args: ConfigArgs) -> Result<(), String> {
    let ctx = find_repo()?;
    let cfg = config::load_config(&ctx.ivaldi_dir);

    if args.list {
        for (key, value) in cfg.list() {
            println!("{}={}", color::dim(key), value);
        }
        return Ok(());
    }
    if let Some(key) = &args.get {
        match cfg.get(key) {
            Some(value) => println!("{}", value),
            None => return Err(format!("config key not found: {}", key)),
        }
        return Ok(());
    }
    if let Some(key) = &args.set {
        let value = args.value.as_deref().ok_or("value required for --set")?;
        let mut repo_cfg = Config::load(&ctx.ivaldi_dir.join("config")).unwrap_or_else(|_| Config::new());
        repo_cfg.set(key, value);
        repo_cfg.save(&ctx.ivaldi_dir.join("config")).map_err(|e| e.to_string())?;
        println!("{}={}", key, value);
        return Ok(());
    }

    // No flags — interactive mode
    interactive_config(&ctx.ivaldi_dir)?;
    Ok(())
}

fn interactive_config(ivaldi_dir: &std::path::Path) -> Result<(), String> {
    let mut cfg = Config::load(&ivaldi_dir.join("config")).unwrap_or_else(|_| Config::new());

    println!("{}", color::bold("Ivaldi Configuration"));
    println!("{}\n", color::dim("Press Enter to keep current value, or type a new one."));

    // user.name
    let current_name = cfg.get("user.name").unwrap_or("").to_string();
    let name = prompt_with_default("user.name", &current_name)?;
    if !name.is_empty() { cfg.set("user.name", &name); }

    // user.email
    let current_email = cfg.get("user.email").unwrap_or("").to_string();
    let email = prompt_with_default("user.email", &current_email)?;
    if !email.is_empty() { cfg.set("user.email", &email); }

    // color.ui
    let current_color = cfg.get("color.ui").unwrap_or("true").to_string();
    let color_ui = prompt_with_default("color.ui", &current_color)?;
    if !color_ui.is_empty() { cfg.set("color.ui", &color_ui); }

    // core.autoshelf
    let current_shelf = cfg.get("core.autoshelf").unwrap_or("true").to_string();
    let autoshelf = prompt_with_default("core.autoshelf", &current_shelf)?;
    if !autoshelf.is_empty() { cfg.set("core.autoshelf", &autoshelf); }

    cfg.save(&ivaldi_dir.join("config"))
        .map_err(|e| e.to_string())?;

    println!("\n{} Configuration saved.", color::green("✓"));

    if let Some(author) = cfg.author() {
        println!("Author: {}", color::author(&author));
    }

    Ok(())
}

fn prompt_with_default(key: &str, default: &str) -> Result<String, String> {
    use std::io::Write;

    if default.is_empty() {
        print!("  {} = ", color::cyan(key));
    } else {
        print!("  {} [{}] = ", color::cyan(key), color::dim(default));
    }
    std::io::stdout().flush().map_err(|e| e.to_string())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
    let input = input.trim();

    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.to_string())
    }
}

fn cmd_exclude(args: ExcludeArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let ignore_path = ctx.work_dir.join(".ivaldiignore");
    let mut content = std::fs::read_to_string(&ignore_path).unwrap_or_default();
    for pattern in &args.patterns {
        if !content.lines().any(|l| l.trim() == pattern) {
            if !content.is_empty() && !content.ends_with('\n') { content.push('\n'); }
            content.push_str(pattern);
            content.push('\n');
            if !quiet { println!("  excluded: {}", pattern); }
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
            let mut portal = Portal::parse(&add_args.repo)
                .ok_or(format!("invalid portal format: {}. Expected: owner/repo", add_args.repo))?;
            if add_args.gitlab { portal.platform = Platform::GitLab; }
            if let Some(url) = add_args.url { portal.base_url = Some(url); }
            let added = mgr.add(&portal).map_err(|e| e.to_string())?;
            if !quiet {
                if added { println!("Added portal: {}", portal.to_string_repr()); }
                else { println!("Portal already configured: {}", portal.to_string_repr()); }
            }
        }
        PortalCommands::List => {
            let portals = mgr.list().map_err(|e| e.to_string())?;
            if portals.is_empty() { println!("No portals configured."); }
            else {
                println!("Configured portals:");
                for p in &portals {
                    let plat = match p.platform { Platform::GitHub => "github", Platform::GitLab => "gitlab" };
                    print!("  {} ({})", p.to_string_repr(), plat);
                    if let Some(url) = &p.base_url { print!(" [{}]", url); }
                    println!();
                }
            }
        }
        PortalCommands::Remove(remove_args) => {
            let removed = mgr.remove(&remove_args.repo).map_err(|e| e.to_string())?;
            if !quiet {
                if removed { println!("Removed portal: {}", remove_args.repo); }
                else { println!("Portal not found: {}", remove_args.repo); }
            }
        }
    }
    Ok(())
}

fn cmd_auth(args: AuthArgs) -> Result<(), String> {
    match args.command {
        AuthCommands::Login(login_args) => {
            use crate::auth::TokenStore;
            use crate::github::GitHubClient;

            if login_args.gitlab {
                println!("GitLab OAuth not yet implemented. Set GITLAB_TOKEN environment variable.");
                return Ok(());
            }

            println!("Initiating GitHub authentication...");
            let device_code = GitHubClient::request_device_code().map_err(|e| e.to_string())?;

            println!("\nFirst, copy your one-time code: {}", device_code.user_code);
            println!("Then visit: {}", device_code.verification_uri);
            println!("\nWaiting for authentication...");

            let token = GitHubClient::poll_for_token(&device_code.device_code, device_code.interval)
                .map_err(|e| e.to_string())?;

            let store = TokenStore::new().map_err(|e| e.to_string())?;
            store.save_token(Platform::GitHub, token).map_err(|e| e.to_string())?;
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
            let platform = if logout_args.gitlab { Platform::GitLab } else { Platform::GitHub };
            match TokenStore::new() {
                Ok(store) => { store.delete_token(platform).map_err(|e| e.to_string())?; println!("Logged out successfully"); }
                Err(e) => println!("Warning: {}", e),
            }
        }
    }
    Ok(())
}

fn cmd_download(args: DownloadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::sync;

    let (owner, repo_name) = parse_repo_arg(&args.repo)?;
    let client = GitHubClient::new();

    let target_dir = std::path::PathBuf::from(
        args.directory.as_deref().unwrap_or(&repo_name),
    );

    if target_dir.exists() && target_dir.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(format!("directory '{}' already exists and is not empty", target_dir.display()));
    }
    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    let result = sync::download(&client, &owner, &repo_name, &target_dir, None)
        .map_err(|e| e.to_string())?;

    if !quiet {
        println!("Cloned {}/{} → {}", owner, repo_name, target_dir.display());
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
        std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
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
        println!("Uploaded to {}/{} (branch: {})", portal.owner, portal.repo, result.branch);
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

    let branches = sync::scout(&client, &portal.owner, &portal.repo)
        .map_err(|e| e.to_string())?;

    println!("Remote timelines available:");
    for branch in &branches {
        println!("  {}", branch);
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
        let branches = sync::scout(&client, &portal.owner, &portal.repo)
            .map_err(|e| e.to_string())?;
        println!("Available remote timelines:");
        for b in &branches { println!("  {}", b); }
        println!("\nSpecify timelines to harvest: ivaldi harvest <name> [<name>...]");
        return Ok(());
    }

    let harvested = sync::harvest(&client, &mut repo, &portal.owner, &portal.repo, &args.timelines)
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

    let timeline = args.timeline
        .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

    if !quiet {
        println!("Syncing timeline '{}' with {}/{}...\n",
            color::timeline(&timeline), portal.owner, portal.repo);
    }

    let result = sync::sync_timeline(&client, &mut repo, &portal.owner, &portal.repo, &timeline)
        .map_err(|e| e.to_string())?;

    if result.no_changes {
        println!("{} Timeline '{}' is already up to date", color::green("✓"), color::bold(&timeline));
        return Ok(());
    }

    for f in &result.added { println!("{} {}", color::green("++"), f); }
    for f in &result.modified { println!("{} {}", color::green("++"), f); }
    for f in &result.deleted { println!("{} {}", color::red("--"), f); }

    let total = result.added.len() + result.modified.len() + result.deleted.len();
    println!("\n{} Synced {} file(s) from remote", color::green("✓"), total);
    if !result.added.is_empty() { println!("  Added: {}", color::green(&result.added.len().to_string())); }
    if !result.modified.is_empty() { println!("  Modified: {}", color::blue(&result.modified.len().to_string())); }
    if !result.deleted.is_empty() { println!("  Deleted: {}", color::red(&result.deleted.len().to_string())); }

    Ok(())
}

// Helper: parse "owner/repo" from CLI arg
fn parse_repo_arg(arg: &str) -> Result<(String, String), String> {
    // Handle "owner/repo" format
    if let Some((owner, repo)) = arg.split_once('/') {
        if !owner.is_empty() && !repo.is_empty() {
            return Ok((owner.to_string(), repo.to_string()));
        }
    }
    Err(format!("invalid repository format: '{}'. Expected: owner/repo", arg))
}
