//! Safe, non-destructive repository repair (`ivaldi recover`).
//!
//! Every repair here is provably safe: it either writes state that is fully
//! derivable from data already present (the MMR checkpoint, timeline ref files)
//! or it *moves* — never deletes — anything it cannot trust. Corrupt or unknown
//! CAS objects are quarantined under `.ivaldi/quarantine/`, so no user data is
//! ever discarded. If an operation cannot be made safe it is skipped and
//! reported rather than forced.
//!
//! `--dry-run` reports exactly what would happen without touching disk.

use std::path::Path;

use crate::atomic_io::atomic_write;
use crate::hash::B3Hash;
use crate::leaf::parse_leaf;
use crate::mmr::Mmr;
use crate::refname::timeline_ref_path;
use crate::store::{MMR_ROOT_KEY, MMR_SIZE_KEY, Store};

/// One repair the recover pass considered. `done` is false in `--dry-run` (the
/// action was only *planned*) and also false for actions deliberately skipped.
#[derive(serde::Serialize)]
pub struct Action {
    /// Category: `mmr-checkpoint`, `timeline-ref`, or `quarantine`.
    pub kind: String,
    /// What the action is about (a name, a path, a hash).
    pub target: String,
    pub detail: String,
    pub done: bool,
}

/// Result of a recover pass. `problems` holds things surfaced but not repaired
/// (e.g. a checkpoint mismatch we refuse to overwrite).
#[derive(serde::Serialize, Default)]
pub struct RecoverReport {
    pub dry_run: bool,
    pub actions: Vec<Action>,
    pub problems: Vec<String>,
}

impl RecoverReport {
    fn act(
        &mut self,
        kind: &str,
        target: impl Into<String>,
        detail: impl Into<String>,
        done: bool,
    ) {
        self.actions.push(Action {
            kind: kind.into(),
            target: target.into(),
            detail: detail.into(),
            done,
        });
    }

    pub fn print_human(&self) {
        if self.dry_run {
            println!(
                "{}",
                crate::color::bold("recover --dry-run (no changes written)")
            );
        }
        if self.actions.is_empty() && self.problems.is_empty() {
            println!("Nothing to repair; repository looks consistent.");
            return;
        }
        for a in &self.actions {
            let mark = if a.done {
                crate::color::green("done")
            } else if self.dry_run {
                crate::color::yellow("would")
            } else {
                crate::color::yellow("skip")
            };
            println!("[{mark}] {}: {} ({})", a.kind, a.target, a.detail);
        }
        if !self.problems.is_empty() {
            println!();
            println!("{}", crate::color::bold_red("surfaced (not repaired):"));
            for p in &self.problems {
                println!("  - {p}");
            }
        }
    }
}

/// Run every safe repair against the repository at `work_dir`. Never errors: an
/// unrepairable condition becomes a reported problem so the summary always
/// prints. `dry_run` plans without writing.
pub fn recover(work_dir: &Path, dry_run: bool) -> RecoverReport {
    let ivaldi_dir = work_dir.join(".ivaldi");
    let mut report = RecoverReport {
        dry_run,
        ..Default::default()
    };

    // Never call Store::open on a missing file — redb would create a fresh
    // empty store inside the repo we are repairing. The store-backed repairs
    // just don't run; the filesystem quarantine sweep still does.
    let store_path = ivaldi_dir.join("store.db");
    let store = if store_path.exists() {
        match Store::open(&store_path) {
            Ok(s) => Some(s),
            Err(e) => {
                report.problems.push(format!("cannot open store: {e}"));
                None
            }
        }
    } else {
        report
            .problems
            .push("no store.db; skipping MMR and ref repairs".into());
        None
    };

    if let Some(store) = &store {
        recover_mmr_checkpoint(store, &mut report, dry_run);
        recover_timeline_refs(store, &ivaldi_dir, &mut report, dry_run);
    }

    quarantine_corrupt_objects(&ivaldi_dir, &mut report, dry_run);

    report
}

/// Re-establish the MMR size/root checkpoint by rebuilding from the store's
/// leaves — but only when it is missing. A checkpoint that is present and
/// *disagrees* with the rebuild is surfaced, never overwritten: overwriting it
/// would silently mask the exact tampering `Repo::open` fails closed on.
fn recover_mmr_checkpoint(store: &Store, report: &mut RecoverReport, dry_run: bool) {
    let indices = match store.all_leaf_indices() {
        Ok(i) => i,
        Err(e) => {
            report.problems.push(format!("cannot list leaves: {e}"));
            return;
        }
    };
    // A gap means the append-only sequence is broken; we cannot prove any
    // rebuilt root is authoritative, so we refuse to write one.
    for (expected, actual) in indices.iter().copied().enumerate() {
        if actual != expected as u64 {
            report.problems.push(format!(
                "MMR leaf index gap (expected {expected}, found {actual}); cannot rebuild checkpoint"
            ));
            return;
        }
    }
    let actual_size = indices.len() as u64;

    let mut mmr = Mmr::new();
    for idx in &indices {
        let data = match store.get_leaf(*idx) {
            Ok(Some(d)) => d,
            Ok(None) => {
                report
                    .problems
                    .push(format!("leaf {idx} missing; cannot rebuild checkpoint"));
                return;
            }
            Err(e) => {
                report.problems.push(format!("leaf {idx} unreadable: {e}"));
                return;
            }
        };
        match parse_leaf(&data) {
            Ok(l) => {
                mmr.append_leaf(l);
            }
            Err(e) => {
                report.problems.push(format!(
                    "leaf {idx} corrupt ({e}); cannot rebuild checkpoint"
                ));
                return;
            }
        }
    }
    let actual_root = mmr.root();

    let stored_size = store.get_meta(MMR_SIZE_KEY).ok().flatten();
    let stored_root = store.get_meta(MMR_ROOT_KEY).ok().flatten();

    // A stored value that disagrees with the rebuild is a real inconsistency;
    // surface it, do not "fix" it by clobbering.
    if let Some(s) = &stored_size
        && s.parse::<u64>().ok() != Some(actual_size)
    {
        report.problems.push(format!(
            "MMR size checkpoint is {s} but store has {actual_size} leaves; refusing to overwrite (run verify/rescue)"
        ));
        return;
    }
    if let Some(r) = &stored_root
        && B3Hash::from_hex(r) != Some(actual_root)
    {
        report.problems.push(format!(
            "MMR root checkpoint {r} disagrees with rebuilt root {actual_root}; refusing to overwrite (run verify/rescue)"
        ));
        return;
    }

    if stored_size.is_some() && stored_root.is_some() {
        return; // already present and consistent
    }

    // Missing (or partially missing) checkpoint: safe to establish, exactly
    // like Repo::open's one-time migration.
    if dry_run {
        report.act(
            "mmr-checkpoint",
            format!("size={actual_size}"),
            format!("would establish root {actual_root}"),
            false,
        );
        return;
    }
    crate::failpoint::fail_point("recover.before_checkpoint");
    match store.set_mmr_checkpoint(actual_size, actual_root) {
        Ok(()) => report.act(
            "mmr-checkpoint",
            format!("size={actual_size}"),
            format!("established root {actual_root}"),
            true,
        ),
        Err(e) => report
            .problems
            .push(format!("failed to write MMR checkpoint: {e}")),
    }
    crate::failpoint::fail_point("recover.after_checkpoint");
}

/// Recreate any `refs/heads/<name>` file the store knows a head for but that is
/// missing on disk. Existing ref files are never touched or removed.
fn recover_timeline_refs(
    store: &Store,
    ivaldi_dir: &Path,
    report: &mut RecoverReport,
    dry_run: bool,
) {
    let heads = match store.list_timeline_heads() {
        Ok(h) => h,
        Err(e) => {
            report.problems.push(format!("cannot list timelines: {e}"));
            return;
        }
    };
    for (name, _idx) in heads {
        let ref_path = match timeline_ref_path(ivaldi_dir, &name) {
            Ok(path) => path,
            Err(e) => {
                report.problems.push(format!(
                    "unsafe timeline ref name '{name}'; refusing to recreate: {e}"
                ));
                continue;
            }
        };
        if ref_path.exists() {
            continue;
        }
        if dry_run {
            report.act(
                "timeline-ref",
                name,
                "would recreate missing ref file",
                false,
            );
            continue;
        }
        // Match the rest of the codebase: ref files are empty markers, the head
        // index lives in the store. Create the immediate parent as timeline
        // names may contain path components such as `feature/parser`.
        let parent = ref_path.parent().unwrap_or(ivaldi_dir);
        crate::failpoint::fail_point("recover.before_ref_write");
        let written = std::fs::create_dir_all(parent).and_then(|_| atomic_write(&ref_path, b""));
        match written {
            Ok(()) => report.act("timeline-ref", name, "recreated missing ref file", true),
            Err(e) => report
                .problems
                .push(format!("failed to recreate ref '{name}': {e}")),
        }
        crate::failpoint::fail_point("recover.after_ref_write");
    }
}

/// Walk `objects/<2hex>/<62hex>`; any object whose content does not hash to its
/// address is *moved* into `.ivaldi/quarantine/<same-path>`. Nothing is ever
/// deleted or overwritten — a corrupt object is preserved for later forensics.
fn quarantine_corrupt_objects(ivaldi_dir: &Path, report: &mut RecoverReport, dry_run: bool) {
    let objects_dir = ivaldi_dir.join("objects");
    let quarantine_dir = ivaldi_dir.join("quarantine");

    let shards = match std::fs::read_dir(&objects_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            report
                .problems
                .push(format!("cannot read {}: {e}", objects_dir.display()));
            return;
        }
    };

    for shard in shards.flatten() {
        let shard_path = shard.path();
        if !shard_path.is_dir() {
            continue;
        }
        let prefix = shard.file_name().to_string_lossy().into_owned();
        let entries = match std::fs::read_dir(&shard_path) {
            Ok(rd) => rd,
            Err(e) => {
                report
                    .problems
                    .push(format!("cannot read shard {prefix}: {e}"));
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A name that isn't a full hash is a tmp leftover / stray, not a CAS
            // object — leave it alone.
            let Some(expected) = B3Hash::from_hex(&format!("{prefix}{name}")) else {
                continue;
            };
            let src = entry.path();
            let data = match std::fs::read(&src) {
                Ok(d) => d,
                Err(e) => {
                    report
                        .problems
                        .push(format!("cannot read object {expected}: {e}"));
                    continue;
                }
            };
            if B3Hash::digest(&data) == expected {
                continue; // intact
            }

            let dest = quarantine_dir.join(&prefix).join(&name);
            if dry_run {
                report.act(
                    "quarantine",
                    expected.to_hex(),
                    "would move corrupt object",
                    false,
                );
                continue;
            }
            // Never clobber a prior quarantine copy: preserving both the object
            // in place and the earlier copy still loses nothing.
            if dest.exists() {
                report.problems.push(format!(
                    "quarantine target for {expected} exists; left object in place"
                ));
                continue;
            }
            crate::failpoint::fail_point("recover.before_quarantine");
            let moved = std::fs::create_dir_all(quarantine_dir.join(&prefix))
                .and_then(|_| std::fs::rename(&src, &dest));
            match moved {
                Ok(()) => report.act(
                    "quarantine",
                    expected.to_hex(),
                    "moved corrupt object",
                    true,
                ),
                Err(e) => report.problems.push(format!(
                    "failed to quarantine {expected}: {e} (left in place)"
                )),
            }
            crate::failpoint::fail_point("recover.after_quarantine");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{self, FileCas};
    use crate::repo::Repo;

    fn setup_repo(dir: &Path) {
        crate::forge::forge(dir).unwrap();
        let mut cfg = crate::config::Config::new();
        cfg.set("user.name", "T");
        cfg.set("user.email", "t@t");
        cfg.save(&dir.join(".ivaldi/config")).unwrap();
    }

    #[test]
    fn rebuilds_deleted_timeline_ref() {
        let dir = tempfile::tempdir().unwrap();
        setup_repo(dir.path());
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"t"), "A", "c").unwrap();
        }
        let ref_path = dir.path().join(".ivaldi/refs/heads/main");
        // Simulate loss of the marker after a normal commit materialized it.
        std::fs::remove_file(&ref_path).ok();
        assert!(!ref_path.exists());

        let report = recover(dir.path(), false);
        assert!(ref_path.exists(), "recover should recreate the missing ref");
        assert!(
            report
                .actions
                .iter()
                .any(|a| a.kind == "timeline-ref" && a.target == "main" && a.done)
        );
    }

    #[test]
    fn rebuilds_nested_timeline_ref_and_parents() {
        let dir = tempfile::tempdir().unwrap();
        setup_repo(dir.path());
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"t"), "A", "c").unwrap();
            repo.create_timeline("feature/parser", None).unwrap();
        }
        let refs = dir.path().join(".ivaldi/refs");
        std::fs::remove_dir_all(&refs).unwrap();

        let report = recover(dir.path(), false);
        let nested = refs.join("heads/feature/parser");
        assert!(
            nested.exists(),
            "recover should recreate nested ref parents"
        );
        assert!(
            report
                .actions
                .iter()
                .any(|a| { a.kind == "timeline-ref" && a.target == "feature/parser" && a.done })
        );
    }

    #[test]
    fn quarantines_tampered_object_without_deleting() {
        let dir = tempfile::tempdir().unwrap();
        setup_repo(dir.path());
        let objects = dir.path().join(".ivaldi/objects");

        let cas = FileCas::new(&objects).unwrap();
        let hash = cas::put_and_hash(&cas, b"hello").unwrap();
        let hex = hash.to_hex();
        let (d, f) = hex.split_at(2);
        let obj_path = objects.join(d).join(f);
        // Tamper: content no longer hashes to its address.
        std::fs::write(&obj_path, b"tampered").unwrap();

        let report = recover(dir.path(), false);

        // Original gone from objects/, preserved under quarantine/ verbatim.
        assert!(!obj_path.exists(), "corrupt object must leave objects/");
        let quarantined = dir.path().join(".ivaldi/quarantine").join(d).join(f);
        assert!(
            quarantined.exists(),
            "corrupt object must be preserved in quarantine/"
        );
        assert_eq!(std::fs::read(&quarantined).unwrap(), b"tampered");
        assert!(
            report
                .actions
                .iter()
                .any(|a| a.kind == "quarantine" && a.done)
        );
    }

    #[test]
    fn dry_run_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        setup_repo(dir.path());
        let objects = dir.path().join(".ivaldi/objects");
        let cas = FileCas::new(&objects).unwrap();
        let hash = cas::put_and_hash(&cas, b"hi").unwrap();
        let hex = hash.to_hex();
        let (d, f) = hex.split_at(2);
        let obj_path = objects.join(d).join(f);
        std::fs::write(&obj_path, b"bad").unwrap();

        let report = recover(dir.path(), true);
        // Planned but not performed.
        assert!(
            report
                .actions
                .iter()
                .any(|a| a.kind == "quarantine" && !a.done)
        );
        assert!(obj_path.exists(), "dry-run must not move anything");
        assert!(
            !dir.path()
                .join(".ivaldi/quarantine")
                .join(d)
                .join(f)
                .exists()
        );
    }

    #[test]
    fn refuses_to_overwrite_mismatched_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        setup_repo(dir.path());
        {
            let mut repo = Repo::open(dir.path()).unwrap();
            repo.commit(B3Hash::digest(b"t"), "A", "c").unwrap();
            // Corrupt the durable root checkpoint out of band.
            repo.store
                .set_meta(MMR_ROOT_KEY, &B3Hash::digest(b"wrong").to_hex())
                .unwrap();
        }
        let report = recover(dir.path(), false);
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("refusing to overwrite")),
            "must surface, not silently repair, a mismatched checkpoint"
        );
        // And it must not have rewritten it.
        assert!(!report.actions.iter().any(|a| a.kind == "mmr-checkpoint"));
    }
}
