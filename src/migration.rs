//! Transactional repository-format upgrades with verified rollback snapshots.
//!
//! A migration never edits the only copy of a repository. Before any source
//! mutation, it creates a complete snapshot under `.ivaldi/migrations/`, writes
//! a manifest containing a BLAKE3 for every file, and verifies that snapshot.
//! A pending marker blocks normal repository opens. Retrying after interruption
//! first restores the old snapshot, then starts the migration from scratch.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::forge::{self, CURRENT_FORMAT};

const MIGRATIONS: &str = "migrations";
const PENDING: &str = "PENDING";
const RECEIPT: &str = "RECEIPT.json";
const CHANGED: &str = "CHANGED";
const BACKUP: &str = "backup";
const MANIFEST: &str = "MANIFEST.json";
const MANIFEST_SEAL: &str = "MANIFEST.blake3";

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid migration state: {0}")]
    Invalid(String),
    #[error("repository verification failed: {0}")]
    Verification(String),
    #[error("repository format v{0} is newer than this binary supports")]
    TooNew(u32),
    #[error(
        "rollback refused: repository changed after migration; preserve new work and restore the backup manually"
    )]
    ChangedAfterMigration,
    #[error("no completed migration is available to roll back")]
    NoRollback,
}

#[derive(Debug)]
pub struct MigrationReport {
    pub from: u32,
    pub to: u32,
    pub changed: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FileRecord {
    path: String,
    len: u64,
    blake3: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    schema: u32,
    source_format: u32,
    target_format: u32,
    directories: Vec<String>,
    files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    schema: u32,
    source_format: u32,
    target_format: u32,
    post_migration_digest: String,
}

pub fn migrate_to_current(work_dir: &Path) -> Result<MigrationReport, MigrationError> {
    let ivaldi = work_dir.join(".ivaldi");
    if !ivaldi.join("HEAD").is_file() {
        return Err(MigrationError::Invalid("not an Ivaldi repository".into()));
    }
    let migration_dir = ivaldi.join(MIGRATIONS);

    // An abort may leave FORMAT old or new. Never guess which mutation landed:
    // restore the verified snapshot and restart from the exact old bytes.
    if migration_dir.join(PENDING).exists() {
        restore_verified_backup(&ivaldi)?;
        remove_if_exists(&migration_dir.join(PENDING))?;
        remove_if_exists(&migration_dir.join(RECEIPT))?;
    }

    let source = forge::read_format(&ivaldi).map_err(|e| MigrationError::Invalid(e.to_string()))?;
    if source.version > CURRENT_FORMAT {
        return Err(MigrationError::TooNew(source.version));
    }
    if source.version == CURRENT_FORMAT {
        return Ok(MigrationReport {
            from: source.version,
            to: CURRENT_FORMAT,
            changed: false,
            message: format!("repository is already at format v{CURRENT_FORMAT}"),
        });
    }

    fs::create_dir_all(&migration_dir)?;
    if let Err(error) = create_verified_backup(&ivaldi, source.version) {
        let _ = cleanup_migrations(&migration_dir);
        return Err(error);
    }
    crate::atomic_io::atomic_write(
        &migration_dir.join(PENDING),
        format!("from = {}\nto = {}\n", source.version, CURRENT_FORMAT).as_bytes(),
    )?;
    crate::failpoint::fail_point("migration.after_backup");

    let result = (|| {
        // Opening may perform an old-format compatibility repair (for example,
        // establishing a missing legacy MMR checkpoint). It is now protected
        // by the verified snapshot and is included in rollback semantics.
        let repo = crate::repo::Repo::open_for_migration(work_dir)
            .map_err(|e| MigrationError::Verification(e.to_string()))?;
        drop(repo);

        crate::failpoint::fail_point("migration.before_format");
        let body = format!(
            "format = {CURRENT_FORMAT}\nmin_ivaldi = {}\nfeatures =\nmigrated_from = {}\n",
            env!("CARGO_PKG_VERSION"),
            source.version
        );
        crate::atomic_io::atomic_write(&ivaldi.join("FORMAT"), body.as_bytes())?;
        crate::failpoint::fail_point("migration.after_format");

        let repo = crate::repo::Repo::open_for_migration(work_dir)
            .map_err(|e| MigrationError::Verification(e.to_string()))?;
        drop(repo);
        let report = crate::verify::verify_while_migrating(work_dir, true);
        if !report.ok {
            return Err(MigrationError::Verification(
                report
                    .checks
                    .iter()
                    .filter(|c| !c.ok)
                    .map(|c| format!("{}: {}", c.name, c.detail))
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }

        let receipt = Receipt {
            schema: 1,
            source_format: source.version,
            target_format: CURRENT_FORMAT,
            post_migration_digest: repository_digest(&ivaldi)?,
        };
        let bytes = serde_json::to_vec_pretty(&receipt)
            .map_err(|e| MigrationError::Invalid(e.to_string()))?;
        crate::atomic_io::atomic_write(&migration_dir.join(RECEIPT), &bytes)?;
        crate::failpoint::fail_point("migration.after_receipt");
        remove_if_exists(&migration_dir.join(PENDING))?;
        crate::failpoint::fail_point("migration.after_pending_clear");
        Ok(())
    })();

    if let Err(error) = result {
        restore_verified_backup(&ivaldi)?;
        cleanup_migrations(&migration_dir)?;
        return Err(error);
    }

    Ok(MigrationReport {
        from: source.version,
        to: CURRENT_FORMAT,
        changed: true,
        message: format!(
            "migrated repository format v{} -> v{}; verified rollback snapshot retained",
            source.version, CURRENT_FORMAT
        ),
    })
}

pub fn rollback(work_dir: &Path) -> Result<MigrationReport, MigrationError> {
    let ivaldi = work_dir.join(".ivaldi");
    let migration_dir = ivaldi.join(MIGRATIONS);
    if migration_dir.join(PENDING).exists() {
        let manifest = read_manifest(&migration_dir.join(BACKUP))?;
        restore_verified_backup(&ivaldi)?;
        cleanup_migrations(&migration_dir)?;
        return Ok(MigrationReport {
            from: manifest.target_format,
            to: manifest.source_format,
            changed: true,
            message: format!(
                "restored interrupted migration to format v{}",
                manifest.source_format
            ),
        });
    }

    let receipt_bytes = fs::read(migration_dir.join(RECEIPT)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MigrationError::NoRollback
        } else {
            MigrationError::Io(e)
        }
    })?;
    let receipt: Receipt = serde_json::from_slice(&receipt_bytes)
        .map_err(|e| MigrationError::Invalid(format!("invalid migration receipt: {e}")))?;
    if migration_dir.join(CHANGED).exists() {
        return Err(MigrationError::ChangedAfterMigration);
    }
    if repository_digest(&ivaldi)? != receipt.post_migration_digest {
        return Err(MigrationError::ChangedAfterMigration);
    }
    crate::atomic_io::atomic_write(
        &migration_dir.join(PENDING),
        format!(
            "rollback_from = {}\nrollback_to = {}\n",
            receipt.target_format, receipt.source_format
        )
        .as_bytes(),
    )?;
    crate::failpoint::fail_point("migration.rollback.after_marker");
    restore_verified_backup(&ivaldi)?;
    crate::failpoint::fail_point("migration.rollback.after_restore");
    cleanup_migrations(&migration_dir)?;
    Ok(MigrationReport {
        from: receipt.target_format,
        to: receipt.source_format,
        changed: true,
        message: format!(
            "rolled repository back to format v{}",
            receipt.source_format
        ),
    })
}

/// Conservatively invalidate automatic rollback before a normal mutating CLI
/// command runs. Marking before execution means even a partially failed or
/// interrupted command can never be overwritten by automatic rollback.
pub(crate) fn mark_changed_after_migration(ivaldi: &Path) -> Result<(), MigrationError> {
    let migrations = ivaldi.join(MIGRATIONS);
    if migrations.join(RECEIPT).exists() {
        crate::atomic_io::atomic_write(
            &migrations.join(CHANGED),
            b"repository mutation attempted after migration\n",
        )?;
    }
    Ok(())
}

fn create_verified_backup(ivaldi: &Path, source_format: u32) -> Result<(), MigrationError> {
    let migrations = ivaldi.join(MIGRATIONS);
    let staging = migrations.join("backup.staging");
    let backup = migrations.join(BACKUP);
    remove_dir_if_exists(&staging)?;
    remove_dir_if_exists(&backup)?;
    fs::create_dir_all(&staging)?;

    let mut directories = Vec::new();
    let mut files = Vec::new();
    snapshot_dir(ivaldi, ivaldi, &staging, &mut directories, &mut files)?;
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest {
        schema: 1,
        source_format,
        target_format: CURRENT_FORMAT,
        directories,
        files,
    };
    let bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| MigrationError::Invalid(e.to_string()))?;
    crate::atomic_io::atomic_write(&staging.join(MANIFEST), &bytes)?;
    verify_backup(&staging, &manifest)?;
    crate::failpoint::fail_point("migration.before_backup_publish");
    fs::rename(&staging, &backup)?;
    crate::atomic_io::atomic_write(
        &migrations.join(MANIFEST_SEAL),
        format!("{}\n", blake3::hash(&bytes).to_hex()).as_bytes(),
    )?;
    sync_dir(&migrations);
    verify_backup(&backup, &manifest)
}

fn snapshot_dir(
    root: &Path,
    current: &Path,
    destination: &Path,
    directories: &mut Vec<String>,
    files: &mut Vec<FileRecord>,
) -> Result<(), MigrationError> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|e| MigrationError::Invalid(e.to_string()))?;
        if rel
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == MIGRATIONS)
            || rel == Path::new("repo.lock")
            || entry.file_name().to_string_lossy().contains(".tmp.")
        {
            continue;
        }
        let rel_string = portable_path(rel)?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(MigrationError::Invalid(format!(
                "refusing to snapshot symlink inside repository: {rel_string}"
            )));
        }
        let target = destination.join(rel);
        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
            directories.push(rel_string);
            snapshot_dir(root, &path, destination, directories, files)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let (len, hash) = copy_and_hash(&path, &target)?;
            fs::set_permissions(&target, fs::metadata(&path)?.permissions())?;
            files.push(FileRecord {
                path: rel_string,
                len,
                blake3: hash,
            });
        } else {
            return Err(MigrationError::Invalid(format!(
                "unsupported filesystem entry in repository: {rel_string}"
            )));
        }
    }
    Ok(())
}

fn restore_verified_backup(ivaldi: &Path) -> Result<(), MigrationError> {
    let backup = ivaldi.join(MIGRATIONS).join(BACKUP);
    let manifest = read_manifest(&backup)?;
    verify_backup(&backup, &manifest)?;

    // Keep HEAD until the verified snapshot is copied back. Repository
    // discovery depends on it, so an interrupted rollback remains retryable.
    // The snapshot copy atomically overwrites HEAD near the end.
    let keep = BTreeSet::from([
        MIGRATIONS.to_string(),
        "repo.lock".to_string(),
        "HEAD".to_string(),
    ]);
    for entry in fs::read_dir(ivaldi)?.collect::<Result<Vec<_>, _>>()? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name) {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    crate::failpoint::fail_point("migration.rollback.after_clear");
    for dir in &manifest.directories {
        fs::create_dir_all(ivaldi.join(dir))?;
    }
    for file in &manifest.files {
        let source = backup.join(&file.path);
        let target = ivaldi.join(&file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_and_hash(&source, &target)?;
        fs::set_permissions(&target, fs::metadata(source)?.permissions())?;
    }
    sync_dir(ivaldi);
    Ok(())
}

fn verify_backup(backup: &Path, manifest: &Manifest) -> Result<(), MigrationError> {
    let mut actual = BTreeMap::new();
    for file in &manifest.files {
        let path = backup.join(&file.path);
        let (len, hash) = hash_file(&path)?;
        actual.insert(file.path.clone(), (len, hash));
    }
    for expected in &manifest.files {
        let Some((len, hash)) = actual.get(&expected.path) else {
            return Err(MigrationError::Invalid(format!(
                "backup missing {}",
                expected.path
            )));
        };
        if *len != expected.len || hash != &expected.blake3 {
            return Err(MigrationError::Invalid(format!(
                "backup checksum mismatch for {}",
                expected.path
            )));
        }
    }
    Ok(())
}

fn read_manifest(backup: &Path) -> Result<Manifest, MigrationError> {
    let bytes = fs::read(backup.join(MANIFEST))?;
    let migrations = backup
        .parent()
        .ok_or_else(|| MigrationError::Invalid("backup has no migration directory".into()))?;
    let expected = fs::read_to_string(migrations.join(MANIFEST_SEAL))?;
    let actual = blake3::hash(&bytes).to_hex().to_string();
    if expected.trim() != actual {
        return Err(MigrationError::Invalid(
            "backup manifest seal does not match MANIFEST.json".into(),
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| MigrationError::Invalid(format!("invalid backup manifest: {e}")))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &Manifest) -> Result<(), MigrationError> {
    if manifest.schema != 1
        || manifest.source_format >= manifest.target_format
        || manifest.target_format > CURRENT_FORMAT
    {
        return Err(MigrationError::Invalid(
            "unsupported or inconsistent backup manifest version".into(),
        ));
    }
    let mut paths = BTreeSet::new();
    for path in manifest
        .directories
        .iter()
        .chain(manifest.files.iter().map(|file| &file.path))
    {
        let candidate = Path::new(path);
        if candidate.is_absolute()
            || path.contains('\\')
            || candidate
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || !paths.insert(path)
        {
            return Err(MigrationError::Invalid(format!(
                "unsafe or duplicate path in backup manifest: {path:?}"
            )));
        }
    }
    Ok(())
}

fn repository_digest(ivaldi: &Path) -> Result<String, MigrationError> {
    let temp = tempfile_manifest(ivaldi)?;
    let mut hasher = blake3::Hasher::new();
    for file in temp {
        hasher.update(&(file.path.len() as u64).to_le_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update(&file.len.to_le_bytes());
        hasher.update(file.blake3.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn tempfile_manifest(root: &Path) -> Result<Vec<FileRecord>, MigrationError> {
    fn walk(root: &Path, current: &Path, out: &mut Vec<FileRecord>) -> Result<(), MigrationError> {
        let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .map_err(|e| MigrationError::Invalid(e.to_string()))?;
            if rel
                .components()
                .next()
                .is_some_and(|c| c.as_os_str() == MIGRATIONS)
                || rel == Path::new("repo.lock")
                || rel == Path::new("store.db")
                || entry.file_name().to_string_lossy().contains(".tmp.")
            {
                continue;
            }
            let ty = entry.file_type()?;
            if ty.is_dir() {
                walk(root, &path, out)?;
            } else if ty.is_file() {
                let (len, hash) = hash_file(&path)?;
                out.push(FileRecord {
                    path: portable_path(rel)?,
                    len,
                    blake3: hash,
                });
            } else {
                return Err(MigrationError::Invalid(
                    "unsupported repository entry".into(),
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn copy_and_hash(source: &Path, target: &Path) -> Result<(u64, String), MigrationError> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(target)?;
    let mut hasher = blake3::Hasher::new();
    let mut len = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        len += read as u64;
    }
    output.sync_all()?;
    Ok((len, hasher.finalize().to_hex().to_string()))
}

fn hash_file(path: &Path) -> Result<(u64, String), MigrationError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut len = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        len += read as u64;
    }
    Ok((len, hasher.finalize().to_hex().to_string()))
}

fn portable_path(path: &Path) -> Result<String, MigrationError> {
    let parts = path
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| MigrationError::Invalid("repository path is not UTF-8".into()))?;
    Ok(parts.join("/"))
}

fn cleanup_migrations(path: &Path) -> Result<(), MigrationError> {
    remove_dir_if_exists(path)
}

fn remove_if_exists(path: &Path) -> Result<(), MigrationError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<(), MigrationError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn sync_dir(path: &Path) {
    if let Ok(dir) = fs::File::open(path) {
        let _ = dir.sync_all();
    }
}
