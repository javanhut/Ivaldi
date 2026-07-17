//! Assorted commands: config, tui, completions, man.

use super::*;

pub(super) fn cmd_config(args: ConfigArgs) -> Result<(), String> {
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

pub(super) fn cmd_tui() -> Result<(), String> {
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

/// `ivaldi completions <shell>` — write a completion script to stdout.
/// Requires no repository and mutates nothing.
pub(super) fn cmd_completions(args: CompletionsArgs) -> Result<(), String> {
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
pub(super) fn render_raven_spec(cmd: &clap::Command) -> serde_json::Value {
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
pub(super) fn raven_flags(cmd: &clap::Command) -> Vec<serde_json::Value> {
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
pub(super) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// `ivaldi man --out DIR` — render `ivaldi.1` plus one `ivaldi-<name>.1`
/// page per subcommand into DIR. Requires no repository and mutates nothing.
pub(super) fn cmd_man(args: ManArgs, quiet: bool) -> Result<(), String> {
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
