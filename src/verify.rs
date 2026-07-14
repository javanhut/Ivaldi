//! Repository integrity verification (`ivaldi verify`).
//!
//! The fast (default) check reuses `Repo::open`, which already validates the
//! MMR leaf-index sequence, the size/root checkpoints, parses every leaf, and
//! compares the rebuilt root. `--full` adds a content pass that re-hashes every
//! CAS object and confirms it matches its address — the one integrity property
//! `FileCas::get` does not check on read.

use std::path::Path;

use crate::hash::B3Hash;
use crate::repo::Repo;

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
    }

    Report::from_checks(full, checks)
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
}
