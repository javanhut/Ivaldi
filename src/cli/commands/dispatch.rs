//! Top-level command dispatch, locking, and error-to-exit handling.

use std::process;

use clap::CommandFactory;

use super::*;

/// True for commands that mutate repository state and therefore need the
/// exclusive process lock. Read-only commands stay lock-free. `Download` is
/// excluded because it may target a fresh clone outside any repository.
fn command_mutates(cmd: &Commands) -> bool {
    match cmd {
        Commands::Gather(_)
        | Commands::Seal(_)
        | Commands::Reseal(_)
        | Commands::Discard(_)
        | Commands::Reverse(_)
        | Commands::Rewind(_)
        | Commands::Undo(_)
        | Commands::Pluck(_)
        | Commands::Fuse(_)
        | Commands::Travel(_)
        | Commands::Weld(_)
        | Commands::Harvest(_)
        | Commands::Sync(_)
        | Commands::Upload(_)
        | Commands::Exclude(_)
        | Commands::Migrate(_) => true,
        Commands::Timeline(args) => !matches!(args.command, TimelineCommands::List(_)),
        Commands::Review(args) => !matches!(
            args.command,
            ReviewCommands::List(_) | ReviewCommands::Show(_) | ReviewCommands::Diff(_)
        ),
        _ => false,
    }
}

pub fn run_command(cli: Cli) {
    let Some(cmd) = cli.command else {
        let _ = Cli::command().print_help();
        return;
    };

    let _lock = if command_mutates(&cmd) {
        let setup = find_repo().and_then(|ctx| {
            let lock =
                crate::lock::RepoLock::acquire(&ctx.ivaldi_dir).map_err(|e| e.to_string())?;
            let is_switch = matches!(
                &cmd,
                Commands::Timeline(args) if matches!(args.command, TimelineCommands::Switch(_))
            );
            if !is_switch {
                ensure_no_interrupted_switch(&ctx.ivaldi_dir)?;
            }
            let is_sync = matches!(&cmd, Commands::Sync(_));
            if !is_sync && ctx.ivaldi_dir.join("sync-journal.json").exists() {
                return Err(
                    "an interrupted sync must be finalized before other mutations; run `ivaldi sync` again"
                        .into(),
                );
            }
            Ok(lock)
        });
        match setup {
            Ok(lock) => Some(lock),
            Err(e) => exit_with_error(&e),
        }
    } else {
        None
    };

    // Once a format migration completes, automatic rollback is safe only
    // until the first attempted mutation. Mark before dispatch so a failing or
    // interrupted command cannot be silently overwritten by rollback.
    if command_mutates(&cmd) && !matches!(&cmd, Commands::Migrate(_)) {
        let marked = find_repo().and_then(|ctx| {
            crate::migration::mark_changed_after_migration(&ctx.ivaldi_dir)
                .map_err(|e| e.to_string())
        });
        if let Err(e) = marked {
            exit_with_error(&e);
        }
    }

    let result = match cmd {
        Commands::Forge => cmd_forge(cli.quiet),
        Commands::Gather(args) => cmd_gather(args, cli.quiet),
        Commands::Seal(args) => cmd_seal(args, cli.quiet),
        Commands::Reseal(args) => cmd_reseal(args, cli.quiet),
        Commands::Status(args) => cmd_status(args),
        Commands::Whereami => cmd_whereami(),
        Commands::Log(args) => cmd_log(args),
        Commands::Whodidit(args) => cmd_whodidit(args),
        Commands::Diff(args) => cmd_diff(args),
        Commands::Discard(args) => cmd_discard(args, cli.quiet),
        Commands::Reverse(args) => cmd_reverse(args, cli.quiet),
        Commands::Rewind(args) => cmd_rewind(args, cli.quiet),
        Commands::Undo(args) => cmd_undo(args, cli.quiet),
        Commands::Pluck(args) => cmd_pluck(args, cli.quiet),
        Commands::Timeline(args) => cmd_timeline(args, cli.quiet),
        Commands::Fuse(args) => cmd_fuse(args, cli.quiet),
        Commands::Travel(args) => cmd_travel(args),
        Commands::Weld(args) => cmd_weld(args, cli.quiet),
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
        Commands::Serve(args) => cmd_serve(args, cli.quiet),
        Commands::Peer(args) => cmd_peer(args, cli.quiet),
        Commands::Completions(args) => cmd_completions(args),
        Commands::Man(args) => cmd_man(args, cli.quiet),
        Commands::Verify(args) => cmd_verify(args),
        Commands::Prove(args) => cmd_prove(args),
        Commands::Rescue(args) => cmd_rescue(args),
        Commands::Recover(args) => cmd_recover(args),
        Commands::Doctor(args) => cmd_doctor(args),
        Commands::Migrate(args) => cmd_migrate(args, cli.quiet),
    };
    if let Err(e) = result {
        exit_with_error(&e);
    }
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("{}", color::error(message));
    process::exit(1);
}
