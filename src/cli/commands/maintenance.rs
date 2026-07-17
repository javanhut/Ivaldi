//! Repository maintenance commands: verify, rescue, recover, doctor, migrate.

use super::*;

pub(super) fn cmd_verify(args: VerifyArgs) -> Result<(), String> {
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

pub(super) fn cmd_rescue(args: RescueArgs) -> Result<(), String> {
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

pub(super) fn cmd_recover(args: RecoverArgs) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    // Locate leniently (objects/ present) so a repo too broken for Repo::open
    // can still be repaired.
    let ivaldi_dir = crate::rescue::find_ivaldi_dir(&cwd)
        .ok_or("no .ivaldi/objects found here or in any parent directory")?;
    let work_dir = ivaldi_dir
        .parent()
        .ok_or("could not resolve repository root")?;

    // recover mutates, so take the exclusive repo lock like seal/fuse do. A
    // dry run writes nothing, so it needs no lock (and won't contend).
    let _lock = if args.dry_run {
        None
    } else {
        Some(crate::lock::RepoLock::acquire(&ivaldi_dir).map_err(|e| e.to_string())?)
    };

    let report = crate::recover::recover(work_dir, args.dry_run);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        report.print_human();
    }
    Ok(())
}

pub(super) fn cmd_doctor(args: DoctorArgs) -> Result<(), String> {
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

pub(super) fn cmd_migrate(args: MigrateArgs, quiet: bool) -> Result<(), String> {
    let ctx = find_repo()?;
    let report = if args.rollback {
        crate::migration::rollback(&ctx.work_dir)
    } else {
        crate::migration::migrate_to_current(&ctx.work_dir)
    }
    .map_err(|e| e.to_string())?;
    if !quiet {
        println!("{}", report.message);
    }
    Ok(())
}
