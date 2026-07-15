//! Repository integrity verification (`ivaldi verify`).
//!
//! The fast (default) check reuses `Repo::open`, which already validates the
//! MMR leaf-index sequence, the size/root checkpoints, parses every leaf, and
//! compares the rebuilt root. `--full` adds a content pass that re-hashes every
//! CAS object and confirms it matches its address — the one integrity property
//! `FileCas::get` does not check on read.

use std::collections::HashSet;
use std::path::Path;

use crate::hash::B3Hash;
use crate::repo::Repo;
use crate::store::Store;

/// One named integrity check and its outcome.
#[derive(serde::Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Result of verifying a repository. `ok` is true only if every check passed.
#[derive(serde::Serialize)]
pub struct Report {
    pub ok: bool,
    pub full: bool,
    pub checks: Vec<Check>,
}

impl Report {
    fn from_checks(full: bool, checks: Vec<Check>) -> Self {
        let ok = checks.iter().all(|c| c.ok);
        Self { ok, full, checks }
    }

    /// Actionable recovery guidance derived from which checks failed. Used by
    /// `ivaldi doctor` to turn a diagnosis into next steps.
    pub fn guidance(&self) -> Vec<String> {
        if self.ok {
            return vec!["Repository is healthy. No action needed.".into()];
        }
        let mut out = Vec::new();
        for c in self.checks.iter().filter(|c| !c.ok) {
            match c.name.as_str() {
                "format" => out.push(
                    "Format problem: this repository was written by a newer Ivaldi. \
                     Upgrade Ivaldi to the version named in the error above."
                        .into(),
                ),
                "structure" => out.push(
                    "History is damaged (MMR/leaf/checkpoint inconsistency). Your file \
                     content is likely still intact — recover it with:\n    \
                     ivaldi rescue --out ./ivaldi-rescue"
                        .into(),
                ),
                "cas-objects" => out.push(
                    "Some stored objects are corrupt on disk. Recover the intact files with:\n    \
                     ivaldi rescue --out ./ivaldi-rescue\n  \
                     (corrupt objects cannot be reconstructed and will be reported as missing)."
                        .into(),
                ),
                other => out.push(format!("{other}: {}", c.detail)),
            }
        }
        out
    }

    /// Human-readable rendering for the terminal.
    pub fn print_human(&self) {
        for c in &self.checks {
            let mark = if c.ok {
                crate::color::green("ok")
            } else {
                crate::color::bold_red("FAIL")
            };
            println!("[{mark}] {}: {}", c.name, c.detail);
        }
        println!();
        if self.ok {
            println!("{}", crate::color::bold_green("repository OK"));
        } else {
            println!("{}", crate::color::bold_red("repository has problems"));
        }
    }
}

/// Verify the repository rooted at `work_dir`. Never errors: a broken
/// repository is reported as failed checks, not a returned error, so callers
/// can always print a diagnosis.
pub fn verify(work_dir: &Path, full: bool) -> Report {
    let ivaldi_dir = work_dir.join(".ivaldi");
    let mut checks = Vec::new();

    // Format: readable and not newer than this binary supports.
    match crate::forge::read_format(&ivaldi_dir) {
        Ok(fmt) => checks.push(Check {
            name: "format".into(),
            ok: fmt.version <= crate::forge::CURRENT_FORMAT,
            detail: format!(
                "on-disk v{}, this binary supports up to v{}",
                fmt.version,
                crate::forge::CURRENT_FORMAT
            ),
        }),
        Err(e) => checks.push(Check {
            name: "format".into(),
            ok: false,
            detail: e.to_string(),
        }),
    }

    // Structure: reuse Repo::open, which validates the MMR index sequence,
    // size/root checkpoints, leaf parsing, and the rebuilt root.
    match Repo::open(work_dir) {
        Ok(_) => checks.push(Check {
            name: "structure".into(),
            ok: true,
            detail: "MMR, leaves, and root checkpoint are consistent".into(),
        }),
        Err(e) => checks.push(Check {
            name: "structure".into(),
            ok: false,
            detail: e.to_string(),
        }),
    }

    // Content: re-hash every CAS object (opt-in; O(total bytes)).
    if full {
        checks.push(verify_cas_objects(&ivaldi_dir.join("objects")));

        // Deeper structural checks need direct store access. Never create a
        // store if store.db is absent (redb would materialize an empty one).
        let store_path = ivaldi_dir.join("store.db");
        let store = if store_path.exists() {
            Store::open(&store_path).ok()
        } else {
            None
        };
        checks.push(verify_refs(&ivaldi_dir, store.as_ref()));
        checks.push(verify_seal_mappings(store.as_ref()));
    }

    Report::from_checks(full, checks)
}

/// Every `refs/heads/<name>` must resolve: either the store has a timeline head
/// for that name, or (for the rare ref file that records a value) that value is
/// a leaf index the store actually holds. Empty ref files are legitimate "no
/// seals yet" markers. Anything else is a dangling ref.
fn verify_refs(ivaldi_dir: &Path, store: Option<&Store>) -> Check {
    let heads_dir = ivaldi_dir.join("refs/heads");
    let entries = match std::fs::read_dir(&heads_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Check {
                name: "refs".into(),
                ok: true,
                detail: "no refs/heads directory (nothing to check)".into(),
            };
        }
        Err(e) => {
            return Check {
                name: "refs".into(),
                ok: false,
                detail: format!("cannot read {}: {e}", heads_dir.display()),
            };
        }
    };

    let head_names: HashSet<String> = store
        .and_then(|s| s.list_timeline_heads().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let indices: HashSet<u64> = store
        .and_then(|s| s.all_leaf_indices().ok())
        .unwrap_or_default()
        .into_iter()
        .collect();

    let mut count = 0u64;
    let mut problems = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                problems.push(format!("cannot read ref directory entry: {e}"));
                continue;
            }
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        count += 1;
        let name = entry.file_name().to_string_lossy().into_owned();
        if head_names.contains(&name) {
            continue; // resolves via the store's timeline head
        }
        // No store head: an empty file is an uncommitted-timeline marker; a
        // recorded leaf index resolves only if the store holds that leaf.
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                problems.push(format!("ref '{name}' is unreadable: {e}"));
                continue;
            }
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(idx) = trimmed.parse::<u64>()
            && indices.contains(&idx)
        {
            continue;
        }
        problems.push(format!("dangling ref '{name}'"));
    }

    if problems.is_empty() {
        Check {
            name: "refs".into(),
            ok: true,
            detail: format!("{count} refs resolve"),
        }
    } else {
        Check {
            name: "refs".into(),
            ok: false,
            detail: problems.join("; "),
        }
    }
}

/// Every seal name must map to a hash that belongs to a leaf that exists. A
/// seal pointing at a hash with no matching leaf is a broken mapping.
fn verify_seal_mappings(store: Option<&Store>) -> Check {
    let Some(store) = store else {
        return Check {
            name: "seal-mappings".into(),
            ok: true,
            detail: "no store (nothing to check)".into(),
        };
    };

    let indices = match store.all_leaf_indices() {
        Ok(i) => i,
        Err(e) => {
            return Check {
                name: "seal-mappings".into(),
                ok: false,
                detail: format!("cannot list leaves: {e}"),
            };
        }
    };
    let mut leaf_hashes = HashSet::new();
    let mut problems = Vec::new();
    for idx in &indices {
        match store.get_leaf(*idx) {
            Ok(Some(bytes)) => match crate::leaf::parse_leaf(&bytes) {
                Ok(l) => {
                    leaf_hashes.insert(l.hash());
                }
                Err(e) => problems.push(format!("leaf {idx} corrupt: {e}")),
            },
            Ok(None) => {}
            Err(e) => problems.push(format!("leaf {idx} unreadable: {e}")),
        }
    }

    let names = store.find_seal_names_by_prefix("").unwrap_or_default();
    let mut count = 0u64;
    for name in &names {
        count += 1;
        match store.get_hash_by_seal_name(name) {
            Ok(Some(h)) if leaf_hashes.contains(&h) => {}
            Ok(Some(h)) => problems.push(format!("seal '{name}' -> {h} has no matching leaf")),
            Ok(None) => problems.push(format!("seal '{name}' has no hash mapping")),
            Err(e) => problems.push(format!("seal '{name}': {e}")),
        }
    }

    if problems.is_empty() {
        Check {
            name: "seal-mappings".into(),
            ok: true,
            detail: format!("{count} seal mappings resolve"),
        }
    } else {
        Check {
            name: "seal-mappings".into(),
            ok: false,
            detail: format!("{count} checked; {}", problems.join("; ")),
        }
    }
}

/// Re-hash every object under a `FileCas` root (`<objects>/<2hex>/<62hex>`) and
/// confirm its content matches its address.
fn verify_cas_objects(objects_dir: &Path) -> Check {
    let mut count: u64 = 0;
    let mut problems = Vec::new();

    let shards = match std::fs::read_dir(objects_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Check {
                name: "cas-objects".into(),
                ok: true,
                detail: "no objects directory (empty repository)".into(),
            };
        }
        Err(e) => {
            return Check {
                name: "cas-objects".into(),
                ok: false,
                detail: format!("cannot read {}: {e}", objects_dir.display()),
            };
        }
    };

    for shard in shards.flatten() {
        let shard_path = shard.path();
        if !shard_path.is_dir() {
            continue; // stray file at the shard level; not an object
        }
        let prefix = shard.file_name().to_string_lossy().into_owned();
        let entries = match std::fs::read_dir(&shard_path) {
            Ok(rd) => rd,
            Err(e) => {
                problems.push(format!("cannot read shard {prefix}: {e}"));
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // ponytail: skip crashed-write leftovers (`*.tmp.<pid>.<n>`) and
            // anything whose name isn't a full hash — not corruption.
            let Some(expected) = B3Hash::from_hex(&format!("{prefix}{name}")) else {
                continue;
            };
            match std::fs::read(entry.path()) {
                Ok(data) => {
                    count += 1;
                    if B3Hash::digest(&data) != expected {
                        problems.push(format!("object {expected} content does not match its hash"));
                    }
                }
                Err(e) => problems.push(format!("cannot read object {expected}: {e}")),
            }
        }
    }

    if problems.is_empty() {
        Check {
            name: "cas-objects".into(),
            ok: true,
            detail: format!("{count} objects re-hashed, all match"),
        }
    } else {
        Check {
            name: "cas-objects".into(),
            ok: false,
            detail: format!("{count} objects checked; {}", problems.join("; ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::FileCas;

    #[test]
    fn guidance_points_damaged_repo_at_rescue() {
        let report = Report::from_checks(
            true,
            vec![Check {
                name: "structure".into(),
                ok: false,
                detail: "MMR root mismatch".into(),
            }],
        );
        let guidance = report.guidance();
        assert!(guidance.iter().any(|g| g.contains("ivaldi rescue")));
    }

    #[test]
    fn guidance_is_clean_when_healthy() {
        let report = Report::from_checks(
            false,
            vec![Check {
                name: "format".into(),
                ok: true,
                detail: "v1".into(),
            }],
        );
        assert_eq!(
            report.guidance(),
            vec!["Repository is healthy. No action needed."]
        );
    }

    #[test]
    fn clean_repo_verifies() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();

        let report = verify(dir.path(), true);
        assert!(
            report.ok,
            "checks: {:?}",
            report
                .checks
                .iter()
                .map(|c| (&c.name, c.ok))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn corrupted_object_fails_full_check() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let objects = dir.path().join(".ivaldi/objects");

        // Store an object, then overwrite its file with different bytes so the
        // content no longer matches its address.
        let cas = FileCas::new(&objects).unwrap();
        let hash = crate::cas::put_and_hash(&cas, b"hello").unwrap();
        let hex = hash.to_hex();
        let (d, f) = hex.split_at(2);
        std::fs::write(objects.join(d).join(f), b"tampered").unwrap();

        let full = verify(dir.path(), true);
        assert!(!full.ok);
        assert!(full.checks.iter().any(|c| c.name == "cas-objects" && !c.ok));

        // The fast check does not re-hash content, so it still passes.
        let fast = verify(dir.path(), false);
        assert!(fast.checks.iter().all(|c| c.name == "cas-objects" || c.ok));
        assert!(!fast.checks.iter().any(|c| c.name == "cas-objects"));
    }

    #[test]
    fn dangling_ref_fails_full_check() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();

        // A ref file for a timeline the store has no head for, recording a leaf
        // index the store does not hold: dangling.
        let heads = dir.path().join(".ivaldi/refs/heads");
        std::fs::create_dir_all(&heads).unwrap();
        std::fs::write(heads.join("ghost"), "999").unwrap();

        let report = verify(dir.path(), true);
        assert!(report.checks.iter().any(|c| c.name == "refs" && !c.ok));
        assert!(!report.ok);

        // The fast check does not run the deeper refs pass.
        let fast = verify(dir.path(), false);
        assert!(!fast.checks.iter().any(|c| c.name == "refs"));
    }

    #[test]
    fn malformed_ref_fails_full_check() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let heads = dir.path().join(".ivaldi/refs/heads");
        std::fs::write(heads.join("broken"), [0xff]).unwrap();

        let report = verify(dir.path(), true);
        let refs = report
            .checks
            .iter()
            .find(|c| c.name == "refs")
            .expect("full verification should include refs");
        assert!(!refs.ok);
        assert!(refs.detail.contains("unreadable"));
    }
}
