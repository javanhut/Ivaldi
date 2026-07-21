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

pub(super) fn cmd_prove(args: ProveArgs) -> Result<(), String> {
    use crate::proof::InclusionReceipt;

    if let Some(check_path) = args.check {
        return cmd_prove_check(&check_path, args.root.as_deref());
    }

    // clap guarantees `seal` is present when --check is absent.
    let seal_query = args.seal.expect("seal is required unless --check");
    let repo = open_repo()?;
    let (idx, _leaf) = repo
        .resolve_seal(&seal_query)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("seal not found: {seal_query}"))?;
    let receipt = InclusionReceipt::build(&repo, idx).map_err(|e| e.to_string())?;
    println!("{}", receipt.to_json().map_err(|e| e.to_string())?);
    Ok(())
}

/// `prove --check`: verify a receipt. Works outside a repository — trust
/// comes from `--root` or out-of-band comparison, not from local state.
fn cmd_prove_check(check_path: &str, root_hex: Option<&str>) -> Result<(), String> {
    use crate::proof::InclusionReceipt;

    let text = if check_path == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut buf)
            .map_err(|e| e.to_string())?;
        buf
    } else {
        std::fs::read_to_string(check_path)
            .map_err(|e| format!("cannot read receipt '{check_path}': {e}"))?
    };
    let receipt = InclusionReceipt::from_json(&text).map_err(|e| e.to_string())?;

    let pinned = root_hex
        .map(|hex| {
            crate::hash::B3Hash::from_hex(hex)
                .ok_or_else(|| format!("--root is not a valid BLAKE3 hex hash: {hex}"))
        })
        .transpose()?;

    let check = crate::proof::verify_receipt(&receipt, pinned).map_err(|e| e.to_string())?;

    let leaf_label = receipt.seal.as_deref().unwrap_or(&receipt.leaf_hash);
    if check.proof_valid {
        println!(
            "{} proof valid — '{}' is leaf #{} under root",
            color::green("✓"),
            leaf_label,
            receipt.leaf_index
        );
        println!("  root: {}", receipt.root);
    } else {
        println!(
            "{} proof INVALID — '{}' does not verify under the receipt's root",
            color::red("✗"),
            leaf_label
        );
    }
    match check.root_matches_pin {
        Some(true) => println!(
            "{} receipt root matches the trusted --root",
            color::green("✓")
        ),
        Some(false) => println!(
            "{} receipt root does NOT match the trusted --root",
            color::red("✗")
        ),
        None => {}
    }

    // Informational only: inside a repo, note when the receipt pins exactly
    // the current local root. A mismatch means nothing by itself — history
    // grows, and older receipts keep older roots.
    if check.proof_valid
        && let Ok(repo) = open_repo()
        && repo.root().to_hex() == receipt.root
    {
        println!("  (root matches this repository's current history)");
    }

    if !check.proof_valid || check.root_matches_pin == Some(false) {
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
