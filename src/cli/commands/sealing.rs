//! Seal creation commands: seal, reseal, and message-editor helpers.

use super::*;

/// Resolve which editor to launch for composing a seal message. Honors
/// `$VISUAL`, then `$EDITOR`, and falls back to `vim` when neither is set
/// (or is set to an empty/whitespace-only value).
pub(super) fn resolve_editor() -> String {
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
pub(super) fn strip_message_comments(raw: &str) -> String {
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
pub(super) fn compose_message_via_editor(
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

pub(super) fn cmd_seal(args: SealArgs, quiet: bool) -> Result<(), String> {
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
pub(super) fn cmd_reseal(args: ResealArgs, quiet: bool) -> Result<(), String> {
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
    // A crash here leaves the reseal durable with staging still populated;
    // retrying folds the identical delta onto the resealed head (same tree,
    // one more orphaned leaf) — confusing but never data loss.
    crate::failpoint::fail_point("reseal.after_commit");

    let mut ws_mut = Workspace::new(&cas, &ctx.work_dir, &ctx.ivaldi_dir);
    ws_mut.staging.clear();
    ws_mut.save().map_err(|e| e.to_string())?;

    if !quiet {
        println!("Resealed: {} ({})", result.seal_name, result.hash.short8());
    }
    Ok(())
}
