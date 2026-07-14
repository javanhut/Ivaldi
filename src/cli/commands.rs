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

mod dispatch;
pub use dispatch::run_command;

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

fn cmd_verify(args: VerifyArgs) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = forge::find_repo_root(&cwd)
        .ok_or("not an Ivaldi repository (or any parent). Run 'ivaldi forge' to initialize.")?;

    let report = crate::verify::verify(&root, args.full);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        report.print_human();
    }

    // Report already printed; signal failure through the exit code.
    if !report.ok {
        process::exit(1);
    }
    Ok(())
}

fn cmd_rescue(args: RescueArgs) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let ivaldi_dir = crate::rescue::find_ivaldi_dir(&cwd)
        .ok_or("no .ivaldi/objects found here or in any parent directory")?;

    let report = crate::rescue::rescue(&ivaldi_dir, &args.out).map_err(|e| e.to_string())?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        report.print_human(&args.out);
    }
    Ok(())
}

fn cmd_doctor(args: DoctorArgs) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    // Locate the repo leniently (objects/ present) so we can diagnose one that
    // is too broken for Repo::open to succeed.
    let ivaldi_dir = crate::rescue::find_ivaldi_dir(&cwd)
        .ok_or("no .ivaldi/objects found here or in any parent directory")?;
    let work_dir = ivaldi_dir
        .parent()
        .ok_or("could not resolve repository root")?;

    let report = crate::verify::verify(work_dir, !args.quick);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        report.print_human();
        println!();
        println!("{}", color::bold("Diagnosis:"));
        for line in report.guidance() {
            println!("  {line}");
        }
    }

    if !report.ok {
        process::exit(1);
    }
    Ok(())
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

        // For each requested path that wasn't gathered (because it's missing
        // from disk), record it as a deletion if it was present in the parent
        // tree; otherwise it matched nothing at all.
        let mut unmatched: Vec<&str> = Vec::new();
        for path in &refs {
            if ws.staging.is_staged(path) {
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

    ws.save().map_err(|e| e.to_string())?;

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
enum PatchAnswer {
    Yes,
    No,
    AllRest,
    NoneRest,
    Quit,
}

fn read_patch_answer(
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
fn run_patch_session(
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

/// Resolve which editor to launch for composing a seal message. Honors
/// `$VISUAL`, then `$EDITOR`, and falls back to `vim` when neither is set
/// (or is set to an empty/whitespace-only value).
fn resolve_editor() -> String {
    std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "vim".to_string())
}

/// Strip comment lines (those starting with `#`) and surrounding blank lines
/// from an editor buffer, yielding the seal message body. Internal newlines
/// between paragraphs are preserved.
fn strip_message_comments(raw: &str) -> String {
    raw.lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Open the user's editor so they can compose a multi-line seal message, the
/// way `git commit` does when no `-m` is given. Writes a template listing the
/// staged changes to `.ivaldi/SEAL_EDITMSG`, launches the editor on it, then
/// reads the result back with comment lines stripped. An empty message aborts
/// the seal.
fn compose_message_via_editor(
    ivaldi_dir: &std::path::Path,
    ws: &Workspace,
) -> Result<String, String> {
    use std::io::IsTerminal;

    // Launching a full-screen editor only makes sense with a real terminal.
    // In a pipe or script, fail loudly so the user reaches for -m instead of
    // hanging on an editor that can't draw.
    if !std::io::stdin().is_terminal() {
        return Err("no seal message given and no terminal to open an editor. \
             Pass one with: ivaldi seal -m \"your message\""
            .into());
    }

    let editmsg_path = ivaldi_dir.join("SEAL_EDITMSG");

    // First line is left blank for the message; the rest is commented guidance
    // listing what will be sealed. Everything commented is dropped on read.
    let mut template = String::from(
        "\n# Please enter the message for your seal. Lines starting with '#'\n\
         # will be ignored, and an empty message aborts the seal.\n#\n\
         # Changes to be sealed:\n",
    );
    for path in ws.staging.staged_files().keys() {
        template.push_str(&format!("#\tchanged: {}\n", path));
    }
    for path in ws.staging.staged_deletions() {
        template.push_str(&format!("#\tdeleted: {}\n", path));
    }

    std::fs::write(&editmsg_path, &template)
        .map_err(|e| format!("could not write seal message template: {e}"))?;

    let editor = resolve_editor();
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vim");
    let status = process::Command::new(program)
        .args(parts)
        .arg(&editmsg_path)
        .status()
        .map_err(|e| format!("could not launch editor '{editor}': {e}"))?;

    if !status.success() {
        let _ = std::fs::remove_file(&editmsg_path);
        return Err(format!("editor '{editor}' exited abnormally; seal aborted"));
    }

    let raw = std::fs::read_to_string(&editmsg_path)
        .map_err(|e| format!("could not read seal message: {e}"))?;
    let _ = std::fs::remove_file(&editmsg_path);

    let message = strip_message_comments(&raw);
    if message.is_empty() {
        return Err("aborting seal due to empty message".into());
    }
    Ok(message)
}

fn cmd_seal(args: SealArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    if ws.staging.is_empty() {
        return Err(
            "no changes staged for seal. Use 'ivaldi gather .' to stage files first, \
             or 'ivaldi reseal' to redo the previous seal."
                .into(),
        );
    }

    // Message comes from the -m/positional arg, or—when omitted—an editor
    // session over the staged changes (like `git commit` with no -m).
    let message = match args.get_message() {
        Some(m) => m.to_string(),
        None => compose_message_via_editor(&ctx.ivaldi_dir, &ws)?,
    };

    // Open persistent repo first so we can resolve the current timeline's
    // parent tree, then build the seal tree as parent + staging.
    let mut repo = open_repo()?;
    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
    let parent_tree = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .and_then(|idx| repo.get_leaf(idx).ok().flatten())
        .map(|l| l.tree_root);

    let tree_hash = ws.build_seal_tree(parent_tree).map_err(|e| e.to_string())?;

    // Make the seal's blobs and tree nodes durable before the commit record
    // that references them lands in the store.
    cas.flush().map_err(|e| e.to_string())?;

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

/// `reseal`: redo the most recent seal, folding in staged changes (if any)
/// and/or a new message.
fn cmd_reseal(args: ResealArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mut repo = open_repo()?;

    if repo.has_merge_in_progress() {
        return Err(
            "cannot reseal during a merge. Finish with 'ivaldi fuse --continue' or \
             'ivaldi fuse --abort' first."
                .into(),
        );
    }

    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
    let head_idx = repo
        .get_timeline_head(&timeline)
        .map_err(|e| e.to_string())?
        .ok_or("nothing to reseal: timeline has no seals")?;
    let old_leaf = repo
        .get_leaf(head_idx)
        .map_err(|e| e.to_string())?
        .ok_or("corrupt head: leaf missing")?;

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);

    let message = match args.get_message() {
        Some(m) => m.to_string(),
        None => {
            if ws.staging.is_empty() {
                return Err("nothing to reseal: no staged changes and no new message. \
                     Use 'ivaldi gather' to stage files or pass a message."
                    .into());
            }
            old_leaf.message.clone()
        }
    };

    // Staging was gathered against the current head, so head tree + staged
    // delta is exactly the resealed tree. With empty staging this is a
    // message-only reseal reusing the old tree.
    let tree_hash = if ws.staging.is_empty() {
        old_leaf.tree_root
    } else {
        let tree = ws
            .build_seal_tree(Some(old_leaf.tree_root))
            .map_err(|e| e.to_string())?;
        cas.flush().map_err(|e| e.to_string())?;
        tree
    };

    // Heuristic warning: the mapping may also exist from a download, so this
    // is advisory, not a refusal.
    if crate::remote::HashMapping::new(&ctx.ivaldi_dir)
        .get_sha1(old_leaf.hash())
        .is_some()
    {
        eprintln!(
            "warning: the seal being redone was already uploaded; \
             the next 'ivaldi upload' may need to rewrite the remote timeline."
        );
    }

    let cfg = repo.config();
    let author = cfg.author()
        .ok_or("user.name and user.email not configured. Run:\n  ivaldi config --set user.name \"Your Name\"\n  ivaldi config --set user.email \"you@example.com\"")?;

    let result = repo
        .reseal_head(tree_hash, &author, &message)
        .map_err(|e| e.to_string())?;

    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    ws_mut.staging.clear();
    ws_mut.save().map_err(|e| e.to_string())?;

    if !quiet {
        println!("Resealed: {} ({})", result.seal_name, result.hash.short8());
    }
    Ok(())
}

fn cmd_status(args: StatusArgs) -> Result<(), String> {
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

fn cmd_log(args: LogArgs) -> Result<(), String> {
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
fn format_unix_utc(unix_seconds: i64) -> String {
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
fn read_file_at_tree(
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
fn cmd_whodidit(args: WhodiditArgs) -> Result<(), String> {
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
fn resolve_tree(repo: &Repo, target: &str) -> Result<(String, crate::hash::B3Hash), String> {
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

/// `rewind <seal> [--discard]`: move the timeline head back to an earlier
/// seal. Files are left exactly as they are unless `--discard` is given;
/// staged entries are cleared either way (they were gathered against the
/// old head). The seals after the target are orphaned, not deleted — they
/// stay recoverable via `travel --all`.
fn cmd_rewind(args: RewindArgs, quiet: bool) -> Result<(), String> {
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
    if target_idx == head_idx {
        if !quiet {
            println!("Already at that seal — nothing to rewind.");
        }
        return Ok(());
    }
    if !repo
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

    repo.set_timeline_head(&timeline, target_idx)
        .map_err(|e| e.to_string())?;

    let cas = FileCas::new(ctx.ivaldi_dir.join("objects")).map_err(|e| e.to_string())?;
    {
        let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws_mut.staging.clear();
        ws_mut.save().map_err(|e| e.to_string())?;
    }
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

fn cmd_reverse(_args: ReverseArgs, quiet: bool) -> Result<(), String> {
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
        let ws = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.materialize(leaf.tree_root).map_err(|e| e.to_string())?;
        // Clear staging
        let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws_mut.staging.clear();
        ws_mut.save().map_err(|e| e.to_string())?;
        if !quiet {
            println!("Reversed all changes. Working directory restored to last seal.");
        }
        return Ok(());
    }
    Err("no seals to restore the working directory from".into())
}

fn cmd_discard(args: DiscardArgs, quiet: bool) -> Result<(), String> {
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
fn resolve_for_pick(
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
fn parent_tree_files_of(
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
fn finish_pick(
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

fn cmd_undo(args: UndoArgs, quiet: bool) -> Result<(), String> {
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

fn cmd_pluck(args: PluckArgs, quiet: bool) -> Result<(), String> {
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

/// Refuse to run while an interrupted timeline switch needs recovery.
/// Called by mutating commands (except `timeline switch`, which handles
/// resume itself). Read-only commands stay usable for orientation.
fn ensure_no_interrupted_switch(ivaldi_dir: &std::path::Path) -> Result<(), String> {
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
/// the shelf holds the only copies), clear staging, write the journal, then
/// do the destructive-but-idempotent steps (HEAD rewrite, materialize,
/// shelf restore) and clear the journal. A crash after the journal is
/// written is recovered by re-running the switch toward `to` (complete) or
/// `from` (roll back); both replay only idempotent steps.
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
    // Every step is idempotent, so this is safe to replay on resume.
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
            shelf_mgr.remove_shelf(dest).ok();
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
        switch_journal::clear(ivaldi_dir).map_err(|e| e.to_string())?;
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
        let ref_path = ivaldi_dir.join("refs/heads").join(target);
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
    cas.flush().map_err(|e| e.to_string())?;

    // The staged entries belong to `current` and now live in its shelf;
    // clear staging so a crash can't leave the target timeline pointing at
    // the source's stale staging entries.
    {
        let mut ws_mut = Workspace::new(&cas, work_dir, ivaldi_dir);
        ws_mut.staging.clear();
        ws_mut.save().map_err(|e| e.to_string())?;
    }

    // ---- Journal, then the idempotent destructive steps ----
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

    let restored_summary = finish(target)?;
    switch_journal::clear(ivaldi_dir).map_err(|e| e.to_string())?;

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
        && let Some(base_leaf) = repo.get_leaf(base_idx).map_err(|e| e.to_string())?
    {
        collect_blob_hashes(&store, base_leaf.tree_root, "", &mut base_files)?;
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
        let commit_result = repo
            .commit_raw(fuse_leaf, &target)
            .map_err(|e| e.to_string())?;

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
fn cmd_weld(args: WeldArgs, quiet: bool) -> Result<(), String> {
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

    // Build the welded leaf and append on the current timeline. `commit_raw`
    // assigns a fresh MMR index, parents at our chosen `prev_idx`, and updates
    // the timeline head to point at the new seal.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut welded_leaf = Leaf::new(end_leaf.tree_root, &timeline, &author, now, &message);
    welded_leaf.prev_idx = welded_prev;

    let welded_result = repo
        .commit_raw(welded_leaf, &timeline)
        .map_err(|e| e.to_string())?;

    // Replay trailing seals on top of the welded seal, oldest-first, each
    // parented on the previous replay (or on the welded seal itself for the
    // first one). Tree, author, message, and timestamp are preserved; only
    // `prev_idx` changes, which produces a new hash — there is no way to
    // keep the original seal hashes when their parent linkage changes.
    let mut prev_for_next = welded_result.index;
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
        replayed.prev_idx = prev_for_next;
        replayed.merge_idxs = original.merge_idxs.clone();
        let r = repo
            .commit_raw(replayed, &timeline)
            .map_err(|e| e.to_string())?;
        prev_for_next = r.index;
    }

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

fn cmd_config(args: ConfigArgs) -> Result<(), String> {
    // Resolve which config file to target and whether we're in a repo.
    let repo_ctx = find_repo().ok();
    let use_global = args.global || repo_ctx.is_none();
    let target_path = if use_global {
        let path =
            config::global_config_path().ok_or("cannot locate global config: $HOME is not set")?;
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
        let local = repo_ctx.as_ref().filter(|_| !args.global).map(|ctx| {
            Config::load(&ctx.ivaldi_dir.join("config")).unwrap_or_else(|_| Config::new())
        });

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
            println!(
                "{} = {} {}",
                color::cyan(key),
                value,
                color::dim(&format!("({})", provenance))
            );
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
            None => {
                return Err(format!(
                    "config key not found: {}\nKnown keys:\n{}",
                    key,
                    config::known_keys_help()
                ));
            }
        }
        return Ok(());
    }
    if let Some(key) = &args.set {
        let value = args.value.as_deref().ok_or_else(|| {
            format!(
                "value required for --set. Usage: ivaldi config --set <key> <value>\nKnown keys:\n{}",
                config::known_keys_help()
            )
        })?;
        if let Some(warning) = config::validate_set(key, value)? {
            eprintln!("{}", color::dim(&format!("warning: {}", warning)));
        }
        let mut cfg = Config::load(&target_path).unwrap_or_else(|_| Config::new());
        cfg.set(key, value);
        cfg.save(&target_path).map_err(|e| e.to_string())?;
        let scope = if use_global { "global" } else { "local" };
        println!("{}={} ({})", key, value, scope);
        return Ok(());
    }

    // No flags — launch the interactive form. The form itself carries a
    // local/global scope selector; --global only picks the starting scope.
    let global_path =
        config::global_config_path().ok_or("cannot locate global config: $HOME is not set")?;
    let local_path = repo_ctx.as_ref().map(|ctx| ctx.ivaldi_dir.join("config"));
    crate::tui::config_form::run(local_path.as_deref(), &global_path, args.global)
        .map_err(|e| e.to_string())?;
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
    crate::atomic_io::atomic_write(&ignore_path, content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_portal(args: PortalArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let mgr = PortalManager::new(&ctx.ivaldi_dir);
    match args.command {
        PortalCommands::Add(add_args) => {
            // `ivaldi://` URLs synthesize a portal directly (P2P has no
            // owner/repo). Other inputs go through the parse_repo_spec
            // pipeline and preserve SSH origin in base_url for transport
            // dispatch.
            let mut portal = if let Some(p) = Portal::parse(&add_args.repo) {
                if p.base_url.is_some() {
                    p
                } else {
                    let spec = parse_repo_arg(&add_args.repo)?;
                    Portal {
                        owner: spec.owner,
                        repo: spec.repo,
                        platform: spec.platform,
                        base_url: None,
                    }
                }
            } else {
                let spec = parse_repo_arg(&add_args.repo)?;
                Portal {
                    owner: spec.owner,
                    repo: spec.repo,
                    platform: spec.platform,
                    base_url: None,
                }
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
        PortalCommands::List(list_args) => {
            let portals = mgr.list().map_err(|e| e.to_string())?;
            if list_args.json {
                let out: Vec<json::PortalJson> = portals
                    .iter()
                    .map(|p| json::PortalJson {
                        repo: p.to_string_repr(),
                        platform: match p.platform {
                            Platform::GitHub => "github",
                            Platform::GitLab => "gitlab",
                        }
                        .to_string(),
                        url: p.base_url.clone(),
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
                );
            } else if portals.is_empty() {
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
            use crate::gitlab;

            if login_args.gitlab {
                let host = gitlab::resolve_host(login_args.gitlab_host.as_deref());
                println!("Initiating GitLab authentication against {}...", host);
                let device_code = gitlab::request_device_code(&host).map_err(|e| e.to_string())?;
                println!(
                    "\nFirst, copy your one-time code: {}",
                    device_code.user_code
                );
                let url = device_code.browser_url();
                if open_in_browser(url) {
                    println!("Opened {} in your browser.", url);
                    println!("(If nothing opened, visit the URL above manually.)");
                } else {
                    println!("Then visit: {}", url);
                }
                println!("\nWaiting for authentication...");
                let token =
                    gitlab::poll_for_token(&host, &device_code.device_code, device_code.interval)
                        .map_err(|e| e.to_string())?;
                let store = TokenStore::new().map_err(|e| e.to_string())?;
                store
                    .save_token(Platform::GitLab, token)
                    .map_err(|e| e.to_string())?;
                println!("\nAuthentication successful!");
                return Ok(());
            }

            // PAT path: store a Personal Access Token read from stdin instead of
            // running the device flow. PATs are independent per device and are
            // immune to GitHub's 10-token-per-OAuth-app eviction, so this is the
            // most reliable option when ivaldi is used on many machines.
            if login_args.with_token {
                use std::io::BufRead;
                eprintln!(
                    "Paste a GitHub Personal Access Token (needs 'repo' scope) and press Enter:"
                );
                let mut input = String::new();
                std::io::stdin()
                    .lock()
                    .read_line(&mut input)
                    .map_err(|e| e.to_string())?;
                let pat = input.trim().to_string();
                if pat.is_empty() {
                    return Err("no token provided on stdin".into());
                }
                if GitHubClient::with_token(pat.clone()).verify_token() == Some(false) {
                    return Err(
                        "GitHub rejected that token. Check it is valid and has 'repo' scope."
                            .into(),
                    );
                }
                let token = crate::auth::Token {
                    access_token: pat,
                    token_type: "pat".to_string(),
                    scope: String::new(),
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                };
                let store = TokenStore::new().map_err(|e| e.to_string())?;
                store
                    .save_token(Platform::GitHub, token)
                    .map_err(|e| e.to_string())?;
                println!("Stored your GitHub Personal Access Token for this device.");
                return Ok(());
            }

            // Reuse-aware: don't mint another OAuth token if a usable one
            // already resolves. Every extra token counts against GitHub's
            // 10-per-OAuth-app/scope limit and can silently evict an older
            // device, which is the root cause of the cross-device logouts.
            // `--force` skips this entirely.
            if !login_args.force {
                // If our own stored token is no longer accepted by GitHub, drop
                // it so we can fall back to gh / env / .netrc instead of piling
                // on yet another token. Only delete on a definitive rejection,
                // never on a transient network error (verify_token -> None).
                if let Some(existing) = crate::auth::resolve_auth(Platform::GitHub)
                    && existing.name == "ivaldi"
                    && GitHubClient::with_token(existing.token.clone()).verify_token()
                        == Some(false)
                    && let Ok(store) = TokenStore::new()
                {
                    let _ = store.delete_token(Platform::GitHub);
                }

                // A surviving credential (a valid ivaldi token, or a gh / env /
                // .netrc one) means there is nothing to do.
                if let Some(existing) = crate::auth::resolve_auth(Platform::GitHub) {
                    let trustworthy = existing.name != "ivaldi"
                        || GitHubClient::with_token(existing.token.clone()).verify_token()
                            != Some(false);
                    if trustworthy {
                        println!("Already authenticated with GitHub via {}.", existing.name);
                        println!("  {}", existing.description);
                        println!(
                            "Ivaldi will use this credential automatically — no new login needed."
                        );
                        println!(
                            "(Run 'ivaldi auth login --force' to mint a separate ivaldi token, or"
                        );
                        println!(
                            " 'ivaldi auth login --with-token' to paste a Personal Access Token.)"
                        );
                        return Ok(());
                    }
                }
            }

            println!("Initiating GitHub authentication...");
            let device_code = GitHubClient::request_device_code().map_err(|e| e.to_string())?;

            println!(
                "\nFirst, copy your one-time code: {}",
                device_code.user_code
            );
            if open_in_browser(&device_code.verification_uri) {
                println!("Opened {} in your browser.", device_code.verification_uri);
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
    use crate::ssh_transport::SshTarget;
    use crate::sync;

    // `ivaldi://host[:port][/timeline]` — peer-to-peer transport, no
    // GitHub / GitLab in the loop.
    if let Some(url) = crate::p2p::PeerUrl::parse(&args.repo) {
        use crate::identity;
        use crate::known_peers::TofuPolicy;
        let id_path =
            identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
        let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
        let default_dir = url.timeline.clone().unwrap_or_else(|| url.host.clone());
        let target_dir =
            std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&default_dir));
        let policy = if args.accept_new_peer {
            TofuPolicy::AcceptAll
        } else if args.strict_peer {
            TofuPolicy::StrictKnown
        } else {
            TofuPolicy::Prompt
        };
        let summary = crate::p2p::fetch_into_with_policy(&url, &target_dir, &id, policy)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Cloned ivaldi://{}:{} → {}",
                url.host,
                url.port,
                target_dir.display()
            );
            println!(
                "  timeline: {}, {} leaves, {} objects",
                summary.timeline, summary.leaves_imported, summary.blobs_imported
            );
        }
        return Ok(());
    }

    // Try SSH next — `git@host:owner/repo.git` and `ssh://...` go straight
    // to the SSH transport. Anything that doesn't parse as SSH falls
    // through to the existing HTTPS / GitHub flow.
    if let Some(target) = SshTarget::parse(&args.repo) {
        let default_dir = target
            .repo_path
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or("repo")
            .to_string();
        let target_dir =
            std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&default_dir));
        let result = sync::download_ssh(&target, &target_dir, None).map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Cloned {}@{}:{} → {}",
                target.user,
                target.host,
                target.repo_path,
                target_dir.display()
            );
            println!("  {} files downloaded", result.files_downloaded);
        }
        return Ok(());
    }

    // Generic Git smart-HTTP host (AUR, Gitea, cgit, self-hosted, ...).
    // GitHub/GitLab return None and keep their existing REST-aware path below.
    if let Some((base, owner, repo)) = parse_generic_git_url(&args.repo) {
        let target_dir = std::path::PathBuf::from(args.directory.as_deref().unwrap_or(&repo));
        let result = sync::download_url(&base, &owner, &repo, &target_dir, None, None)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!("Cloned {} → {}", base, target_dir.display());
            println!("  {} files downloaded", result.files_downloaded);
        }
        return Ok(());
    }

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

/// Parse a generic Git smart-HTTP URL into `(base_url, owner, repo)`.
///
/// Returns `None` for non-URLs (bare `owner/repo` shorthand) and for
/// github.com/gitlab.com, which keep their platform-specific download path.
/// The base URL is returned verbatim (trailing slash trimmed); owner/repo are
/// display/portal labels derived from the path (host stands in for owner when
/// the path is a single segment, e.g. AUR's `/yay.git`).
fn parse_generic_git_url(raw: &str) -> Option<(String, String, String)> {
    let raw = raw.trim();
    let after = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))?;
    let (host, path) = after.split_once('/')?;
    if host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("gitlab.com") {
        return None;
    }
    let path_clean = path.trim_end_matches('/');
    let path_clean = path_clean.strip_suffix(".git").unwrap_or(path_clean);
    let segs: Vec<&str> = path_clean.split('/').filter(|s| !s.is_empty()).collect();
    let (owner, repo) = match segs.as_slice() {
        [] => return None,
        [one] => (host.to_string(), (*one).to_string()),
        many => (
            many[many.len() - 2].to_string(),
            many[many.len() - 1].to_string(),
        ),
    };
    Some((raw.trim_end_matches('/').to_string(), owner, repo))
}

fn cmd_upload(args: UploadArgs, quiet: bool) -> Result<(), String> {
    use crate::github::GitHubClient;
    use crate::portal::Transport;

    let mut repo = open_repo()?;

    let portal_mgr = PortalManager::new(&repo.ivaldi_dir);
    let portal = portal_mgr
        .get_default()
        .map_err(|e| e.to_string())?
        .ok_or("no portal configured. Run 'ivaldi portal add owner/repo'.")?;

    // SSH push — bypass the GitHub auth check entirely.
    if let Transport::Ssh(target) = portal.transport() {
        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));
        if args.force {
            print!(
                "WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: "
            );
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
        let report = crate::ssh_transport::SshClient::new(target)
            .push_repo(&mut repo, &timeline, args.force)
            .map_err(|e| e.to_string())?;

        if !report.unpack_ok {
            return Err(format!(
                "remote rejected pack: {}",
                report.unpack_error.unwrap_or_else(|| "unknown".into())
            ));
        }
        let mut had_failure = false;
        for r in &report.refs {
            match &r.error {
                Some(reason) => {
                    println!("  {} REJECTED: {}", r.name, reason);
                    had_failure = true;
                }
                None => {
                    if !quiet {
                        println!("  {} updated", r.name);
                    }
                }
            }
        }
        if had_failure {
            return Err("one or more refs were rejected".into());
        }
        if !quiet {
            println!("Pushed timeline '{}' over SSH.", timeline);
        }
        return Ok(());
    }

    // Peer-to-peer push — branch out before the GitHub auth check.
    if let Transport::Peer(peer_url) = portal.transport() {
        use crate::identity;
        use crate::known_peers::TofuPolicy;

        let id_path =
            identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
        let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));
        let summary = crate::p2p::push_to(&peer_url, &mut repo, &id, &timeline, TofuPolicy::Prompt)
            .map_err(|e| e.to_string())?;
        if !quiet {
            println!(
                "Pushed timeline '{}' to ivaldi://{}:{}",
                timeline, peer_url.host, peer_url.port
            );
            println!(
                "  landed as: {} ({} leaves, {} objects)",
                summary.landed_as, summary.leaves_sent, summary.objects_sent
            );
        }
        return Ok(());
    }

    // Generic Git smart-HTTP host (AUR, Gitea, cgit, self-hosted). Push
    // straight to the portal's URL; auth resolves per-host (env / .netrc).
    if let Transport::GenericHttps(base_url) = portal.transport() {
        let host = crate::portal::http_host(&base_url).unwrap_or_default();
        let token = crate::auth::generic_git_token(&host);

        if args.force {
            print!(
                "WARNING: Force push will OVERWRITE remote history! Type 'force push' to confirm: "
            );
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

        let timeline = args
            .branch
            .clone()
            .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

        let report = crate::git_remote::SmartHttpClient::new(token.as_deref())
            .push_repo_url(&mut repo, &base_url, &timeline, args.force)
            .map_err(|e| e.to_string())?;

        if !report.unpack_ok {
            return Err(format!(
                "remote rejected pack: {}",
                report.unpack_error.unwrap_or_else(|| "unknown".into())
            ));
        }
        let mut had_failure = false;
        for r in &report.refs {
            match &r.error {
                Some(reason) => {
                    println!("  {} REJECTED: {}", r.name, reason);
                    had_failure = true;
                }
                None => {
                    if !quiet {
                        println!("  {} updated", r.name);
                    }
                }
            }
        }
        if had_failure {
            return Err("one or more refs were rejected".into());
        }
        if !quiet {
            println!("Uploaded timeline '{}' to {}.", timeline, base_url);
        }
        return Ok(());
    }

    let client = GitHubClient::new();
    if !client.is_authenticated() {
        return Err("not authenticated. Run 'ivaldi auth login' or set GITHUB_TOKEN.".into());
    }

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

    // Push the timeline as a single packfile over smart-HTTP
    // `git-receive-pack` (one request), instead of the old per-object REST
    // upload that fired hundreds of requests and tripped GitHub's rate limit.
    let timeline = args
        .branch
        .clone()
        .unwrap_or_else(|| repo.current_timeline().unwrap_or_else(|_| "main".into()));

    let report = crate::git_remote::SmartHttpClient::new(client.token())
        .push_repo(
            &mut repo,
            &portal.owner,
            &portal.repo,
            &timeline,
            args.force,
        )
        .map_err(|e| e.to_string())?;

    if !report.unpack_ok {
        return Err(format!(
            "remote rejected pack: {}",
            report.unpack_error.unwrap_or_else(|| "unknown".into())
        ));
    }
    let mut had_failure = false;
    for r in &report.refs {
        match &r.error {
            Some(reason) => {
                println!("  {} REJECTED: {}", r.name, reason);
                had_failure = true;
            }
            None => {
                if !quiet {
                    println!("  {} updated", r.name);
                }
            }
        }
    }
    if had_failure {
        return Err("one or more refs were rejected".into());
    }
    if !quiet {
        println!(
            "Uploaded timeline '{}' to {}/{}.",
            timeline, portal.owner, portal.repo
        );
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

    let branches = sync::scout_with_status(&client, &repo, &portal).map_err(|e| e.to_string())?;

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
        let branches = sync::scout(&client, &portal).map_err(|e| e.to_string())?;
        println!("Available remote timelines:");
        for b in &branches {
            println!("  {}", b);
        }
        println!("\nSpecify timelines to harvest: ivaldi harvest <name> [<name>...]");
        return Ok(());
    }

    let harvested =
        sync::harvest(&client, &mut repo, &portal, &args.timelines).map_err(|e| e.to_string())?;

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
                        status_str
                            .parse::<ReviewStatus>()
                            .map_err(|_| format!("unknown status: {}", status_str))?,
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

fn cmd_serve(args: ServeArgs, _quiet: bool) -> Result<(), String> {
    use crate::identity;
    use crate::p2p;

    let ctx = find_repo()?;

    let id_path =
        identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
    let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
    let peer_store_path = ctx.ivaldi_dir.join("authorized_peers");
    p2p::serve(&args.bind, ctx.work_dir.clone(), &id, peer_store_path)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_peer(args: PeerArgs, _quiet: bool) -> Result<(), String> {
    use crate::identity;
    use crate::peers::{PeerStore, decode_pubkey};

    match args.command {
        PeerCommands::Whoami => {
            let id_path =
                identity::default_path().ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
            let id = identity::Identity::load_or_create(&id_path).map_err(|e| e.to_string())?;
            println!("{}", id.pubkey_hex());
            Ok(())
        }
        PeerCommands::Trust(t) => {
            let ctx = find_repo()?;
            let pubkey = decode_pubkey(&t.pubkey)?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            store
                .trust(pubkey, t.name.as_deref())
                .map_err(|e| e.to_string())?;
            println!(
                "Trusted peer {}{}",
                t.pubkey,
                t.name
                    .as_ref()
                    .map(|n| format!(" ({})", n))
                    .unwrap_or_default()
            );
            Ok(())
        }
        PeerCommands::List => {
            let ctx = find_repo()?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            let entries = store.list().map_err(|e| e.to_string())?;
            if entries.is_empty() {
                println!("(no trusted peers)");
            } else {
                for e in entries {
                    let key = e.pubkey_hex();
                    match e.name {
                        Some(n) => println!("{}  {}", key, n),
                        None => println!("{}", key),
                    }
                }
            }
            Ok(())
        }
        PeerCommands::Forget(f) => {
            let ctx = find_repo()?;
            let store = PeerStore::repo_local(&ctx.ivaldi_dir);
            match store.forget(&f.prefix).map_err(|e| e.to_string())? {
                Some(removed) => {
                    println!("Forgot {}", removed.pubkey_hex());
                    Ok(())
                }
                None => Err(format!("no trusted peer matches '{}'", f.prefix)),
            }
        }
        PeerCommands::Known(known) => {
            use crate::known_peers::{KnownPeers, fingerprint};
            let store = KnownPeers::default_for_user()
                .ok_or("could not resolve $HOME for ~/.ivaldi/known_peers")?;
            match known.command {
                PeerKnownCommands::List => {
                    let entries = store.list().map_err(|e| e.to_string())?;
                    if entries.is_empty() {
                        println!("(no known peers)");
                    } else {
                        for (key, pk) in entries {
                            println!("{}  {}", key, fingerprint(&pk));
                        }
                    }
                    Ok(())
                }
                PeerKnownCommands::Forget(f) => {
                    let (host, port) = match f.host.rsplit_once(':') {
                        Some((h, p)) => match p.parse::<u16>() {
                            Ok(n) => (h.to_string(), n),
                            Err(_) => (f.host.clone(), crate::p2p::DEFAULT_PORT),
                        },
                        None => (f.host.clone(), crate::p2p::DEFAULT_PORT),
                    };
                    let removed = store.forget(&host, port).map_err(|e| e.to_string())?;
                    if removed {
                        println!("Forgot {}:{}", host, port);
                        Ok(())
                    } else {
                        Err(format!("no known peer at {}:{}", host, port))
                    }
                }
            }
        }
    }
}

/// `ivaldi completions <shell>` — write a completion script to stdout.
/// Requires no repository and mutates nothing.
fn cmd_completions(args: CompletionsArgs) -> Result<(), String> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    match args.shell.clap_shell() {
        Some(shell) => {
            clap_complete::generate(shell, &mut cmd, "ivaldi", &mut std::io::stdout());
        }
        None => {
            // RavenShell consumes a JSON completion spec rather than a shell
            // script. Build it from the same command tree the other shells use.
            let spec = render_raven_spec(&cmd);
            let json = serde_json::to_string_pretty(&spec)
                .map_err(|e| format!("encode raven completions: {e}"))?;
            println!("{json}");
        }
    }
    Ok(())
}

/// Build a RavenShell completion spec from the clap command tree. The result is
/// the JSON that belongs at `~/.config/ravenshell/completions/ivaldi.json`.
///
/// RavenShell's file format (see RavenShell `completion/spec_file.go`) supports
/// one level of subcommands, each with its own flags and an argument source.
/// Ivaldi's grouped commands (`timeline`, `review`, …) carry a second level, so
/// their sub-subcommand names are emitted as that subcommand's static argument
/// candidates — enough for RavenShell to offer `ivaldi timeline <TAB>` →
/// create/switch/…. File completion is suppressed there since those slots take
/// a sub-subcommand, not a path.
fn render_raven_spec(cmd: &clap::Command) -> serde_json::Value {
    use serde_json::json;

    let mut subcommands = Vec::new();
    for sub in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
        let mut entry = json!({ "name": sub.get_name() });
        if let Some(about) = sub.get_about() {
            entry["desc"] = json!(first_line(&about.to_string()));
        }
        let flags = raven_flags(sub);
        if !flags.is_empty() {
            entry["flags"] = json!(flags);
        }
        let nested: Vec<serde_json::Value> = sub
            .get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| {
                let desc = c.get_about().map(|a| first_line(&a.to_string()));
                json!({ "text": c.get_name(), "desc": desc.unwrap_or_default() })
            })
            .collect();
        if !nested.is_empty() {
            entry["args"] = json!({ "static": nested, "noFiles": true });
        }
        subcommands.push(entry);
    }

    let mut spec = json!({ "subcommands": subcommands });
    let flags = raven_flags(cmd);
    if !flags.is_empty() {
        spec["flags"] = json!(flags);
    }
    spec
}

/// Collect a command's optional `--long` flags as RavenShell `{text, desc}`
/// items. Positional arguments and hidden flags are skipped.
fn raven_flags(cmd: &clap::Command) -> Vec<serde_json::Value> {
    use serde_json::json;
    cmd.get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter_map(|arg| {
            let long = arg.get_long()?;
            let desc = arg.get_help().map(|h| first_line(&h.to_string()));
            Some(json!({ "text": format!("--{long}"), "desc": desc.unwrap_or_default() }))
        })
        .collect()
}

/// First line of a (possibly multi-line) help string, for tidy one-line descs.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// `ivaldi man --out DIR` — render `ivaldi.1` plus one `ivaldi-<name>.1`
/// page per subcommand into DIR. Requires no repository and mutates nothing.
fn cmd_man(args: ManArgs, quiet: bool) -> Result<(), String> {
    std::fs::create_dir_all(&args.out)
        .map_err(|e| format!("create {}: {}", args.out.display(), e))?;

    let cmd = <Cli as clap::CommandFactory>::command();

    let write_page = |cmd: clap::Command, file: &str| -> Result<(), String> {
        let mut buf: Vec<u8> = Vec::new();
        clap_mangen::Man::new(cmd)
            .render(&mut buf)
            .map_err(|e| e.to_string())?;
        let path = args.out.join(file);
        std::fs::write(&path, &buf).map_err(|e| format!("write {}: {}", path.display(), e))
    };

    write_page(cmd.clone(), "ivaldi.1")?;
    let mut pages = 1;
    for sub in cmd.get_subcommands() {
        let name = sub.get_name().to_string();
        write_page(sub.clone(), &format!("ivaldi-{}.1", name))?;
        pages += 1;
    }

    if !quiet {
        println!("Wrote {} man page(s) to {}", pages, args.out.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsmerkle::ChangeKind;
    use crate::workspace::FileState;

    #[test]
    fn generic_git_url_parsing() {
        // AUR: single-segment path → host stands in for owner.
        assert_eq!(
            parse_generic_git_url("https://aur.archlinux.org/yay.git"),
            Some((
                "https://aur.archlinux.org/yay.git".into(),
                "aur.archlinux.org".into(),
                "yay".into()
            ))
        );
        // Gitea-style owner/repo, no .git, trailing slash.
        assert_eq!(
            parse_generic_git_url("https://gitea.example.com/foo/bar/"),
            Some((
                "https://gitea.example.com/foo/bar".into(),
                "foo".into(),
                "bar".into()
            ))
        );
        // GitHub/GitLab keep their own path.
        assert_eq!(
            parse_generic_git_url("https://github.com/torvalds/linux"),
            None
        );
        assert_eq!(parse_generic_git_url("https://gitlab.com/foo/bar"), None);
        // Bare shorthand and SSH are handled elsewhere.
        assert_eq!(parse_generic_git_url("owner/repo"), None);
        assert_eq!(parse_generic_git_url("git@example.com:foo/bar.git"), None);
    }

    mod editor_message {
        use super::super::*;

        #[test]
        fn strips_comment_lines_and_trims_blanks() {
            let raw = "\nFix the parser\n\nHandle empty input\n# Please enter the message\n#\tchanged: src/parse.rs\n";
            assert_eq!(
                strip_message_comments(raw),
                "Fix the parser\n\nHandle empty input"
            );
        }

        #[test]
        fn keeps_hash_not_at_line_start() {
            // A literal '#' mid-line (e.g. an issue ref) is not a comment.
            assert_eq!(
                strip_message_comments("Closes issue #42"),
                "Closes issue #42"
            );
        }

        #[test]
        fn all_comments_yields_empty() {
            assert_eq!(strip_message_comments("# only\n# comments\n"), "");
        }

        #[test]
        fn editor_prefers_visual_then_editor_then_vim() {
            // SAFETY: single-threaded test; we restore the env afterward.
            let saved_visual = std::env::var_os("VISUAL");
            let saved_editor = std::env::var_os("EDITOR");

            unsafe {
                std::env::remove_var("VISUAL");
                std::env::remove_var("EDITOR");
            }
            assert_eq!(resolve_editor(), "vim");

            unsafe { std::env::set_var("EDITOR", "nano") };
            assert_eq!(resolve_editor(), "nano");

            unsafe { std::env::set_var("VISUAL", "code --wait") };
            assert_eq!(resolve_editor(), "code --wait");

            // Whitespace-only values are ignored in favor of the next source.
            unsafe { std::env::set_var("VISUAL", "  ") };
            assert_eq!(resolve_editor(), "nano");

            unsafe {
                match saved_visual {
                    Some(v) => std::env::set_var("VISUAL", v),
                    None => std::env::remove_var("VISUAL"),
                }
                match saved_editor {
                    Some(v) => std::env::set_var("EDITOR", v),
                    None => std::env::remove_var("EDITOR"),
                }
            }
        }
    }

    mod patch_session {
        use super::super::*;
        use crate::cas::Cas;
        use crate::fsmerkle::FsStore;
        use crate::workspace::DotfileAllowlist;
        use std::collections::BTreeMap;
        use std::fs;
        use std::path::PathBuf;

        /// Forge a repo whose head seals a.txt with 14 numbered lines, then
        /// modify lines 2 and 12 on disk (two distinct hunks).
        fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
            let dir = tempfile::tempdir().unwrap();
            let work_dir = dir.path().to_path_buf();
            let ivaldi_dir = work_dir.join(".ivaldi");
            forge::forge(&work_dir).unwrap();
            let mut cfg = Config::new();
            cfg.set("user.name", "Test");
            cfg.set("user.email", "t@ivaldi.dev");
            cfg.save(&ivaldi_dir.join("config")).unwrap();

            let old: Vec<String> = (1..=14).map(|i| format!("line {}", i)).collect();
            let old_content = old.join("\n") + "\n";
            fs::write(work_dir.join("a.txt"), &old_content).unwrap();

            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            let mut ws = Workspace::new(&cas, &work_dir, &ivaldi_dir);
            ws.gather(&["a.txt"], &DotfileAllowlist::load(&ivaldi_dir))
                .unwrap();
            let tree = ws.build_seal_tree(None).unwrap();
            let mut repo = Repo::open(&work_dir).unwrap();
            repo.commit(tree, "Test", "initial").unwrap();
            ws.staging.clear();
            ws.save().unwrap();

            let mut new = old.clone();
            new[1] = "line 2 CHANGED".into();
            new[11] = "line 12 CHANGED".into();
            fs::write(work_dir.join("a.txt"), new.join("\n") + "\n").unwrap();

            (dir, work_dir, ivaldi_dir, old_content)
        }

        fn run(
            work_dir: &std::path::Path,
            ivaldi_dir: &std::path::Path,
            answers: &str,
        ) -> (Vec<String>, BTreeMap<String, crate::hash::B3Hash>) {
            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            let mut ws = Workspace::new(&cas, work_dir, ivaldi_dir);
            let repo = Repo::open(work_dir).unwrap();
            let timeline = repo.current_timeline().unwrap();
            let parent_tree = repo
                .get_timeline_head(&timeline)
                .unwrap()
                .and_then(|idx| repo.get_leaf(idx).unwrap())
                .map(|l| l.tree_root)
                .unwrap();
            let parent_files = ws.list_tree_files(parent_tree).unwrap();
            drop(repo);

            let ignore_cache = ignore::load_pattern_cache(work_dir);
            let mut input = std::io::Cursor::new(answers.as_bytes().to_vec());
            let staged = run_patch_session(
                &mut ws,
                &cas,
                &parent_files,
                &[],
                &ignore_cache,
                &mut input,
                true,
            )
            .unwrap();
            ws.save().unwrap();

            let mut staged_hashes = BTreeMap::new();
            for (path, hash) in ws.staging.staged_files() {
                staged_hashes.insert(path.clone(), *hash);
            }
            (staged, staged_hashes)
        }

        fn blob_text(ivaldi_dir: &std::path::Path, hash: crate::hash::B3Hash) -> String {
            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            let store = FsStore::new(&cas);
            let (_, content) = store.load_blob(hash).unwrap();
            String::from_utf8(content).unwrap()
        }

        #[test]
        fn stage_first_hunk_only() {
            let (_dir, work_dir, ivaldi_dir, _old) = setup();
            let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\nn\n");
            assert_eq!(staged, vec!["a.txt"]);

            let text = blob_text(&ivaldi_dir, hashes["a.txt"]);
            assert!(text.contains("line 2 CHANGED"));
            assert!(!text.contains("line 12 CHANGED"));
            assert!(text.contains("line 12\n"));
        }

        #[test]
        fn quit_stages_nothing() {
            let (_dir, work_dir, ivaldi_dir, _old) = setup();
            let (staged, hashes) = run(&work_dir, &ivaldi_dir, "q\n");
            assert!(staged.is_empty());
            assert!(hashes.is_empty());
        }

        #[test]
        fn all_rest_stages_full_change() {
            let (_dir, work_dir, ivaldi_dir, _old) = setup();
            let (staged, hashes) = run(&work_dir, &ivaldi_dir, "a\n");
            assert_eq!(staged, vec!["a.txt"]);
            let text = blob_text(&ivaldi_dir, hashes["a.txt"]);
            assert!(text.contains("line 2 CHANGED"));
            assert!(text.contains("line 12 CHANGED"));
        }

        #[test]
        fn untracked_file_gets_whole_file_prompt() {
            let (_dir, work_dir, ivaldi_dir, old) = setup();
            // Restore a.txt so only the new untracked file is a candidate.
            fs::write(work_dir.join("a.txt"), &old).unwrap();
            fs::write(work_dir.join("b.txt"), "fresh\n").unwrap();

            let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\n");
            assert_eq!(staged, vec!["b.txt"]);
            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            assert!(cas.has(hashes["b.txt"]).unwrap());
        }

        #[test]
        fn binary_file_gets_whole_file_prompt() {
            let (_dir, work_dir, ivaldi_dir, _old) = setup();
            fs::write(work_dir.join("a.txt"), b"bin\x00ary new").unwrap();
            // Seal tree's a.txt is text, disk is binary → whole-file prompt.
            let (staged, hashes) = run(&work_dir, &ivaldi_dir, "y\n");
            assert_eq!(staged, vec!["a.txt"]);
            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            let store = FsStore::new(&cas);
            let (_, content) = store.load_blob(hashes["a.txt"]).unwrap();
            assert_eq!(content, b"bin\x00ary new");
        }
    }

    mod switch_recovery {
        use super::super::*;
        use crate::switch_journal::{self, SwitchJournal};
        use crate::workspace::DotfileAllowlist;
        use std::fs;
        use std::path::{Path, PathBuf};

        /// Forge a repo with one seal on `main` (a.txt) and a `feature`
        /// timeline branched from it.
        fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
            let dir = tempfile::tempdir().unwrap();
            let work_dir = dir.path().to_path_buf();
            let ivaldi_dir = work_dir.join(".ivaldi");
            forge::forge(&work_dir).unwrap();
            let mut cfg = Config::new();
            cfg.set("user.name", "Test");
            cfg.set("user.email", "t@ivaldi.dev");
            cfg.save(&ivaldi_dir.join("config")).unwrap();

            fs::write(work_dir.join("a.txt"), "main content").unwrap();
            seal_all(&work_dir, &ivaldi_dir, "initial");

            let repo = Repo::open(&work_dir).unwrap();
            repo.create_timeline("feature", None).unwrap();
            (dir, work_dir, ivaldi_dir)
        }

        fn seal_all(work_dir: &Path, ivaldi_dir: &Path, message: &str) {
            let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
            let mut ws = Workspace::new(&cas, work_dir, ivaldi_dir);
            ws.gather(&["a.txt"], &DotfileAllowlist::load(ivaldi_dir))
                .unwrap();
            let mut repo = Repo::open(work_dir).unwrap();
            let timeline = repo.current_timeline().unwrap();
            let parent = repo
                .get_timeline_head(&timeline)
                .unwrap()
                .and_then(|idx| repo.get_leaf(idx).unwrap())
                .map(|l| l.tree_root);
            let tree = ws.build_seal_tree(parent).unwrap();
            repo.commit(tree, "Test <t@ivaldi.dev>", message).unwrap();
            ws.staging.clear();
            ws.save().unwrap();
        }

        fn no_tmp_files(dir: &Path) -> bool {
            !fs::read_dir(dir)
                .unwrap()
                .any(|e| e.unwrap().file_name().to_string_lossy().contains(".tmp."))
        }

        #[test]
        fn happy_path_shelves_and_restores() {
            let (_dir, work_dir, ivaldi_dir) = setup();

            // Dirty the worktree on main.
            fs::write(work_dir.join("a.txt"), "dirty edit").unwrap();
            fs::write(work_dir.join("b.txt"), "untracked").unwrap();

            do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
            assert!(no_tmp_files(&ivaldi_dir));
            let repo = Repo::open(&work_dir).unwrap();
            assert_eq!(repo.current_timeline().unwrap(), "feature");
            // Worktree reverted to the sealed tree; dirt is shelved on main.
            assert_eq!(
                fs::read_to_string(work_dir.join("a.txt")).unwrap(),
                "main content"
            );
            assert!(!work_dir.join("b.txt").exists());
            drop(repo);

            // Switching back restores the shelf.
            do_timeline_switch(&work_dir, &ivaldi_dir, "main", true).unwrap();
            assert_eq!(
                fs::read_to_string(work_dir.join("a.txt")).unwrap(),
                "dirty edit"
            );
            assert_eq!(
                fs::read_to_string(work_dir.join("b.txt")).unwrap(),
                "untracked"
            );
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
        }

        #[test]
        fn resume_completes_interrupted_switch() {
            let (_dir, work_dir, ivaldi_dir) = setup();

            // Simulate a crash right after the journal write: shelve phase
            // done (clean worktree → no shelf), journal present, HEAD and
            // worktree untouched.
            switch_journal::write(
                &ivaldi_dir,
                &SwitchJournal {
                    from: "main".into(),
                    to: "feature".into(),
                    shelf_saved: false,
                    started_at: 0,
                },
            )
            .unwrap();

            do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
            let repo = Repo::open(&work_dir).unwrap();
            assert_eq!(repo.current_timeline().unwrap(), "feature");
        }

        #[test]
        fn rollback_returns_to_source() {
            let (_dir, work_dir, ivaldi_dir) = setup();
            switch_journal::write(
                &ivaldi_dir,
                &SwitchJournal {
                    from: "main".into(),
                    to: "feature".into(),
                    shelf_saved: false,
                    started_at: 0,
                },
            )
            .unwrap();

            do_timeline_switch(&work_dir, &ivaldi_dir, "main", true).unwrap();
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
            let repo = Repo::open(&work_dir).unwrap();
            assert_eq!(repo.current_timeline().unwrap(), "main");
        }

        #[test]
        fn third_timeline_refused_while_journal_exists() {
            let (_dir, work_dir, ivaldi_dir) = setup();
            let repo = Repo::open(&work_dir).unwrap();
            repo.create_timeline("third", None).unwrap();
            repo.switch_timeline("main").unwrap();
            drop(repo);

            switch_journal::write(
                &ivaldi_dir,
                &SwitchJournal {
                    from: "main".into(),
                    to: "feature".into(),
                    shelf_saved: false,
                    started_at: 0,
                },
            )
            .unwrap();

            let err = do_timeline_switch(&work_dir, &ivaldi_dir, "third", true).unwrap_err();
            assert!(err.contains("interrupted timeline switch"));
            // Journal untouched.
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_some());
        }

        #[test]
        fn guard_blocks_mutations_while_journal_exists() {
            let (_dir, _work_dir, ivaldi_dir) = setup();
            assert!(ensure_no_interrupted_switch(&ivaldi_dir).is_ok());

            switch_journal::write(
                &ivaldi_dir,
                &SwitchJournal {
                    from: "main".into(),
                    to: "feature".into(),
                    shelf_saved: true,
                    started_at: 0,
                },
            )
            .unwrap();
            let err = ensure_no_interrupted_switch(&ivaldi_dir).unwrap_err();
            assert!(err.contains("timeline switch feature"));
        }

        #[test]
        fn already_on_timeline_still_resumes_when_journal_targets_it() {
            // Crash right after the HEAD write: HEAD already points at the
            // target, but materialize/restore never ran. Re-running the
            // switch must complete it, not early-return "already on".
            let (_dir, work_dir, ivaldi_dir) = setup();
            let repo = Repo::open(&work_dir).unwrap();
            repo.switch_timeline("feature").unwrap();
            drop(repo);
            fs::write(work_dir.join("a.txt"), "mid-transition garbage").unwrap();

            switch_journal::write(
                &ivaldi_dir,
                &SwitchJournal {
                    from: "main".into(),
                    to: "feature".into(),
                    shelf_saved: false,
                    started_at: 0,
                },
            )
            .unwrap();

            do_timeline_switch(&work_dir, &ivaldi_dir, "feature", true).unwrap();
            assert!(switch_journal::load(&ivaldi_dir).unwrap().is_none());
            // Materialize ran: the sealed content is back.
            assert_eq!(
                fs::read_to_string(work_dir.join("a.txt")).unwrap(),
                "main content"
            );
        }
    }

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
