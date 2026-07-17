//! Stable, black-box repository-format migration contract.
//!
//! These tests intentionally assert externally observable state and exact file
//! digests. They must not be weakened when the migration implementation
//! changes: new migration code has to preserve this contract.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

use ivaldi::forge;
use ivaldi::migration::{self, MigrationError};
use ivaldi::repo::Repo;

fn ivaldi(dir: &Path, failpoint: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ivaldi"));
    command.current_dir(dir).env("NO_COLOR", "1").args(args);
    if let Some(failpoint) = failpoint {
        command.env("IVALDI_FAILPOINT", failpoint);
    }
    command.output().expect("run ivaldi")
}

fn ok(dir: &Path, args: &[&str]) -> Output {
    let output = ivaldi(dir, None, args);
    assert!(
        output.status.success(),
        "ivaldi {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A contentful format-1 repository with divergent history, executable data,
/// nested paths, staging state, and an unknown metadata file. This catches
/// migrations that preserve only the happy-path head or only database state.
fn format1_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["forge"]);
    ok(
        dir.path(),
        &["config", "--set", "user.name", "Migration Test"],
    );
    ok(
        dir.path(),
        &["config", "--set", "user.email", "migration@example.com"],
    );
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/nested/data.txt"), b"one\n").unwrap();
    std::fs::write(dir.path().join("script.sh"), b"#!/bin/sh\necho stable\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("script.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    ok(dir.path(), &["gather", "."]);
    ok(dir.path(), &["seal", "base"]);
    ok(dir.path(), &["timeline", "create", "feature/nested"]);
    std::fs::write(dir.path().join("src/nested/data.txt"), b"feature\n").unwrap();
    ok(dir.path(), &["gather", "."]);
    ok(dir.path(), &["seal", "feature change"]);
    ok(dir.path(), &["timeline", "switch", "main"]);
    std::fs::write(dir.path().join("uncommitted.txt"), b"staged but unsealed\n").unwrap();
    ok(dir.path(), &["gather", "uncommitted.txt"]);
    std::fs::write(
        dir.path().join(".ivaldi/FUTURE-OPAQUE"),
        b"must survive byte-for-byte\0\xff",
    )
    .unwrap();
    std::fs::write(
        dir.path().join(".ivaldi/FORMAT"),
        include_bytes!("fixtures/repository-format/v1/FORMAT"),
    )
    .unwrap();
    dir
}

fn repo_files(root: &Path) -> BTreeMap<String, (u32, Vec<u8>)> {
    fn walk(root: &Path, current: &Path, out: &mut BTreeMap<String, (u32, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(current)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap();
            if rel
                .components()
                .next()
                .is_some_and(|c| c.as_os_str() == "migrations")
                || rel == Path::new("repo.lock")
                || entry.file_name().to_string_lossy().contains(".tmp.")
            {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777
                };
                #[cfg(not(unix))]
                let mode = 0;
                out.insert(
                    rel.to_string_lossy().replace('\\', "/"),
                    (mode, std::fs::read(path).unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[derive(Debug, PartialEq, Eq)]
struct ObservableHistory {
    commit_count: u64,
    timelines: Vec<(String, Option<u64>)>,
    seals: Vec<String>,
}

fn observable_history(dir: &Path) -> ObservableHistory {
    let repo = Repo::open(dir).unwrap();
    let mut timelines = repo
        .list_timelines()
        .unwrap()
        .into_iter()
        .map(|(name, head)| (name, Some(head)))
        .collect::<Vec<_>>();
    timelines.sort();
    let mut seals = repo
        .walk_history("feature/nested")
        .unwrap()
        .into_iter()
        .map(|entry| format!("{}:{}:{}", entry.timeline, entry.message, entry.hash))
        .collect::<Vec<_>>();
    seals.sort();
    ObservableHistory {
        commit_count: repo.commit_count(),
        timelines,
        seals,
    }
}

#[test]
fn immutable_v1_format_fixture_has_the_promised_bytes() {
    let fixture = include_str!("fixtures/repository-format/v1/FORMAT");
    assert_eq!(
        fixture, "format = 1\nmin_ivaldi = 0.1.1\nfeatures =\n",
        "historical fixtures are immutable contract inputs, not generated by current code"
    );
}

#[test]
fn migration_preserves_history_staging_permissions_and_unknown_files() {
    let dir = format1_repo();
    let history_before = observable_history(dir.path());
    let opaque_before = std::fs::read(dir.path().join(".ivaldi/FUTURE-OPAQUE")).unwrap();

    let report = migration::migrate_to_current(dir.path()).unwrap();
    assert!(report.changed);
    assert_eq!((report.from, report.to), (1, forge::CURRENT_FORMAT));
    assert_eq!(observable_history(dir.path()), history_before);
    assert_eq!(
        std::fs::read(dir.path().join(".ivaldi/FUTURE-OPAQUE")).unwrap(),
        opaque_before
    );
    let status = ok(dir.path(), &["status"]);
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("uncommitted.txt"),
        "staging state disappeared during migration"
    );
    ok(dir.path(), &["verify", "--full"]);
}

#[test]
fn successful_rollback_restores_every_original_repository_byte_and_mode() {
    let dir = format1_repo();
    let before = repo_files(&dir.path().join(".ivaldi"));
    migration::migrate_to_current(dir.path()).unwrap();
    migration::rollback(dir.path()).unwrap();
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), before);
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        1
    );
    ok(dir.path(), &["verify", "--full"]);
}

#[test]
fn rollback_refuses_to_destroy_work_created_after_migration() {
    let dir = format1_repo();
    migration::migrate_to_current(dir.path()).unwrap();
    std::fs::write(dir.path().join("after.txt"), b"new work\n").unwrap();
    ok(dir.path(), &["gather", "after.txt"]);
    ok(dir.path(), &["seal", "after migration"]);
    let count = Repo::open(dir.path()).unwrap().commit_count();

    assert!(matches!(
        migration::rollback(dir.path()),
        Err(MigrationError::ChangedAfterMigration)
    ));
    assert_eq!(Repo::open(dir.path()).unwrap().commit_count(), count);
    assert_eq!(
        std::fs::read(dir.path().join("after.txt")).unwrap(),
        b"new work\n"
    );
}

#[test]
fn current_format_migration_is_a_byte_exact_noop() {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["forge"]);
    let before = repo_files(&dir.path().join(".ivaldi"));
    let report = migration::migrate_to_current(dir.path()).unwrap();
    assert!(!report.changed);
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), before);
    assert!(!dir.path().join(".ivaldi/migrations").exists());
}

#[test]
fn too_new_repository_is_refused_without_creating_even_a_backup_directory() {
    let dir = format1_repo();
    std::fs::write(
        dir.path().join(".ivaldi/FORMAT"),
        include_bytes!("fixtures/repository-format/future/FORMAT"),
    )
    .unwrap();
    let before = repo_files(&dir.path().join(".ivaldi"));
    assert!(matches!(
        migration::migrate_to_current(dir.path()),
        Err(MigrationError::TooNew(999))
    ));
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), before);
    assert!(!dir.path().join(".ivaldi/migrations").exists());
}

#[test]
fn corrupt_source_fails_verification_and_is_restored_byte_exactly() {
    let dir = format1_repo();
    let object = std::fs::read_dir(dir.path().join(".ivaldi/objects"))
        .unwrap()
        .flatten()
        .flat_map(|shard| std::fs::read_dir(shard.path()).unwrap().flatten())
        .map(|entry| entry.path())
        .next()
        .expect("stored object");
    std::fs::write(&object, b"tampered historical object").unwrap();
    let before = repo_files(&dir.path().join(".ivaldi"));

    assert!(matches!(
        migration::migrate_to_current(dir.path()),
        Err(MigrationError::Verification(_))
    ));
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), before);
    assert!(!dir.path().join(".ivaldi/migrations").exists());
}

#[test]
fn corrupt_backup_is_never_used_for_rollback() {
    let dir = format1_repo();
    migration::migrate_to_current(dir.path()).unwrap();
    let format = dir.path().join(".ivaldi/migrations/backup/FORMAT");
    std::fs::write(format, b"corrupt backup").unwrap();
    let current_before = repo_files(&dir.path().join(".ivaldi"));
    assert!(matches!(
        migration::rollback(dir.path()),
        Err(MigrationError::Invalid(_))
    ));
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), current_before);
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        2
    );
}

#[test]
fn altered_manifest_is_rejected_by_independent_seal_before_restore() {
    let dir = format1_repo();
    migration::migrate_to_current(dir.path()).unwrap();
    let manifest = dir.path().join(".ivaldi/migrations/backup/MANIFEST.json");
    let mut bytes = std::fs::read(&manifest).unwrap();
    bytes.extend_from_slice(b"\n");
    std::fs::write(manifest, bytes).unwrap();
    let current_before = repo_files(&dir.path().join(".ivaldi"));

    let error = migration::rollback(dir.path()).unwrap_err();
    assert!(error.to_string().contains("manifest seal does not match"));
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), current_before);
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        2
    );
}

#[cfg(unix)]
#[test]
fn symlink_inside_repository_is_refused_without_touching_source_state() {
    use std::os::unix::fs::symlink;
    let dir = format1_repo();
    symlink("HEAD", dir.path().join(".ivaldi/suspicious-link")).unwrap();
    let format_before = std::fs::read(dir.path().join(".ivaldi/FORMAT")).unwrap();
    assert!(matches!(
        migration::migrate_to_current(dir.path()),
        Err(MigrationError::Invalid(_))
    ));
    assert_eq!(
        std::fs::read(dir.path().join(".ivaldi/FORMAT")).unwrap(),
        format_before
    );
    assert!(dir.path().join(".ivaldi/suspicious-link").is_symlink());
    assert!(!dir.path().join(".ivaldi/migrations").exists());
}

#[cfg(feature = "failpoints")]
#[test]
fn every_migration_publication_crash_blocks_open_then_restores_and_retries() {
    for failpoint in [
        "migration.after_backup",
        "migration.before_format",
        "migration.after_format",
        "migration.after_receipt",
    ] {
        let dir = format1_repo();
        let old_bytes = repo_files(&dir.path().join(".ivaldi"));
        let output = ivaldi(dir.path(), Some(failpoint), &["migrate"]);
        assert!(!output.status.success(), "{failpoint} did not abort");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(&format!("failpoint hit: {failpoint}"))
        );
        let open_error = match Repo::open(dir.path()) {
            Ok(_) => panic!("repository opened while migration was pending"),
            Err(error) => error,
        };
        assert!(
            open_error
                .to_string()
                .contains("interrupted format migration")
        );

        // Retrying is defined to restore the verified old snapshot first, then
        // perform a fresh migration. It cannot continue from guessed state.
        ok(dir.path(), &["migrate"]);
        ok(dir.path(), &["verify", "--full"]);
        ok(dir.path(), &["migrate", "--rollback"]);
        assert_eq!(repo_files(&dir.path().join(".ivaldi")), old_bytes);
    }
}

#[cfg(feature = "failpoints")]
#[test]
fn crash_before_backup_publication_leaves_old_repo_openable_and_retryable() {
    let dir = format1_repo();
    let old_bytes = repo_files(&dir.path().join(".ivaldi"));
    let output = ivaldi(
        dir.path(),
        Some("migration.before_backup_publish"),
        &["migrate"],
    );
    assert!(!output.status.success());
    assert_eq!(repo_files(&dir.path().join(".ivaldi")), old_bytes);
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        1
    );
    assert_eq!(observable_history(dir.path()).commit_count, 2);

    ok(dir.path(), &["migrate"]);
    ok(dir.path(), &["verify", "--full"]);
}

#[cfg(feature = "failpoints")]
#[test]
fn crash_after_pending_clear_is_a_complete_verified_migration() {
    let dir = format1_repo();
    let output = ivaldi(
        dir.path(),
        Some("migration.after_pending_clear"),
        &["migrate"],
    );
    assert!(!output.status.success());
    assert!(!dir.path().join(".ivaldi/migrations/PENDING").exists());
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        2
    );
    ok(dir.path(), &["verify", "--full"]);
    ok(dir.path(), &["migrate", "--rollback"]);
    assert_eq!(
        forge::read_format(&dir.path().join(".ivaldi"))
            .unwrap()
            .version,
        1
    );
}

#[cfg(feature = "failpoints")]
#[test]
fn rollback_is_idempotently_recoverable_at_every_destructive_boundary() {
    for failpoint in [
        "migration.rollback.after_marker",
        "migration.rollback.after_clear",
        "migration.rollback.after_restore",
    ] {
        let dir = format1_repo();
        let old_bytes = repo_files(&dir.path().join(".ivaldi"));
        ok(dir.path(), &["migrate"]);

        let output = ivaldi(dir.path(), Some(failpoint), &["migrate", "--rollback"]);
        assert!(!output.status.success(), "{failpoint} did not abort");
        assert!(dir.path().join(".ivaldi/HEAD").exists());
        assert!(dir.path().join(".ivaldi/migrations/PENDING").exists());
        let open_error = match Repo::open(dir.path()) {
            Ok(_) => panic!("repository opened during interrupted rollback"),
            Err(error) => error,
        };
        assert!(
            open_error
                .to_string()
                .contains("interrupted format migration")
        );

        ok(dir.path(), &["migrate", "--rollback"]);
        assert_eq!(repo_files(&dir.path().join(".ivaldi")), old_bytes);
        assert!(!dir.path().join(".ivaldi/migrations").exists());
        ok(dir.path(), &["verify", "--full"]);
    }
}
