//! Repository initialization (forge) for Ivaldi VCS.
//!
//! Creates the `.ivaldi/` directory structure and initializes
//! all required components for a new Ivaldi repository.
//!
//! Directory structure created:
//! ```text
//! .ivaldi/
//! ├── objects/        # Content-addressable storage
//! ├── refs/
//! │   ├── heads/      # Timeline references
//! │   ├── remotes/    # Remote timeline references
//! │   └── seals/      # Seal name → hash mappings
//! ├── shelves/        # Auto-shelving storage
//! ├── butterflies/    # Butterfly metadata
//! ├── stage/          # Staging area
//! ├── config          # Repository configuration
//! ├── FORMAT          # On-disk format version + compatibility
//! └── HEAD            # Current timeline pointer
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Result of a forge operation.
#[derive(Debug)]
pub struct ForgeResult {
    /// Path to the created .ivaldi directory.
    pub ivaldi_dir: PathBuf,
    /// Name of the default timeline.
    pub default_timeline: String,
    /// Whether this was a new initialization or already existed.
    pub already_existed: bool,
    /// Number of Git branches imported (0 if no .git/).
    pub git_imported: usize,
}

/// Initialize a new Ivaldi repository in the given directory.
///
/// Creates the `.ivaldi/` directory structure with all required subdirectories.
/// If `.ivaldi/` already exists, returns without modifying it.
pub fn forge(work_dir: &Path) -> Result<ForgeResult, ForgeError> {
    let ivaldi_dir = work_dir.join(".ivaldi");

    if ivaldi_dir.exists() {
        return Ok(ForgeResult {
            ivaldi_dir,
            default_timeline: "main".to_string(),
            already_existed: true,
            git_imported: 0,
        });
    }

    // Create directory structure
    let dirs = [
        "",             // .ivaldi/
        "objects",      // CAS
        "refs",         // References root
        "refs/heads",   // Timeline heads
        "refs/remotes", // Remote refs
        "refs/seals",   // Seal name mappings
        "shelves",      // Auto-shelving
        "butterflies",  // Butterfly metadata
        "stage",        // Staging area
        "reviews",      // Local code reviews
    ];

    for dir in &dirs {
        let path = if dir.is_empty() {
            ivaldi_dir.clone()
        } else {
            ivaldi_dir.join(dir)
        };
        fs::create_dir_all(&path).map_err(ForgeError::Io)?;
    }

    // Create HEAD pointing to main
    let head_path = ivaldi_dir.join("HEAD");
    fs::write(&head_path, "ref: refs/heads/main\n").map_err(ForgeError::Io)?;

    // Stamp the on-disk format so newer repos can be refused by older binaries.
    write_format(&ivaldi_dir).map_err(ForgeError::Io)?;

    // Create default config
    let config = Config::new();
    config
        .save(&ivaldi_dir.join("config"))
        .map_err(|e| ForgeError::Io(std::io::Error::other(e.to_string())))?;

    // Detect and import from existing .git/ if present
    let git_imported = detect_and_import_git(work_dir, &ivaldi_dir);

    Ok(ForgeResult {
        ivaldi_dir,
        default_timeline: "main".to_string(),
        already_existed: false,
        git_imported,
    })
}

/// On-disk repository format this binary writes and can open. Bump on any
/// breaking change to a persisted encoding; a repository stamped higher than
/// this is refused. See plan.md Phase 1.
pub const CURRENT_FORMAT: u32 = 1;

/// Oldest Ivaldi version that understands `CURRENT_FORMAT`. Written into
/// `.ivaldi/FORMAT` purely so the "too new" error can name a version to
/// install; the actual gate is the format number.
const MIN_IVALDI: &str = "0.1.1";

/// Parsed `.ivaldi/FORMAT`. A missing file means format 0 (repositories
/// created before FORMAT existed) and is always openable.
#[derive(Debug, Clone)]
pub struct RepoFormat {
    pub version: u32,
    pub min_ivaldi: String,
}

/// Write `.ivaldi/FORMAT` as plain `key = value` lines. The line format is
/// deliberate: an older or newer binary can read the keys it knows and ignore
/// the rest, which a strict struct decode could not.
fn write_format(ivaldi_dir: &Path) -> std::io::Result<()> {
    // ponytail: `features` is empty until a real optional feature exists;
    // add feature-gating on open when one does.
    let body = format!("format = {CURRENT_FORMAT}\nmin_ivaldi = {MIN_IVALDI}\nfeatures =\n");
    crate::atomic_io::atomic_write(&ivaldi_dir.join("FORMAT"), body.as_bytes())
}

/// Read `.ivaldi/FORMAT`. A missing file is format 0, not an error.
pub fn read_format(ivaldi_dir: &Path) -> Result<RepoFormat, ForgeError> {
    let path = ivaldi_dir.join("FORMAT");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RepoFormat {
                version: 0,
                min_ivaldi: String::new(),
            });
        }
        Err(e) => return Err(ForgeError::Io(e)),
    };

    let mut version = None;
    let mut min_ivaldi = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "format" => {
                version = Some(value.trim().parse::<u32>().map_err(|_| {
                    ForgeError::Io(std::io::Error::other(format!(
                        "{}: invalid format version {:?}",
                        path.display(),
                        value.trim()
                    )))
                })?);
            }
            "min_ivaldi" => min_ivaldi = value.trim().to_string(),
            _ => {} // unknown key: forward-compatible, ignore
        }
    }

    match version {
        Some(version) => Ok(RepoFormat {
            version,
            min_ivaldi,
        }),
        None => Err(ForgeError::Io(std::io::Error::other(format!(
            "{}: missing 'format' field",
            path.display()
        )))),
    }
}

/// Refuse to open a repository whose format is newer than this binary supports.
pub fn check_format(ivaldi_dir: &Path) -> Result<(), ForgeError> {
    let fmt = read_format(ivaldi_dir)?;
    if fmt.version > CURRENT_FORMAT {
        return Err(ForgeError::FormatTooNew {
            found: fmt.version,
            supported: CURRENT_FORMAT,
            min_ivaldi: fmt.min_ivaldi,
        });
    }
    Ok(())
}

/// Detect a .git/ directory and import basic refs info.
/// Returns number of branches found, or 0 if no .git/.
fn detect_and_import_git(work_dir: &Path, ivaldi_dir: &Path) -> usize {
    let git_dir = work_dir.join(".git");
    if !git_dir.exists() {
        return 0;
    }

    let mut imported = 0;

    // Import HEAD
    if let Ok(head_content) = fs::read_to_string(git_dir.join("HEAD")) {
        let head = head_content.trim();
        if let Some(ref_path) = head.strip_prefix("ref: refs/heads/") {
            // Write Ivaldi HEAD pointing to same branch
            let _ = fs::write(
                ivaldi_dir.join("HEAD"),
                format!("ref: refs/heads/{}\n", ref_path),
            );
        }
    }

    // Import branch names from .git/refs/heads/
    let git_heads = git_dir.join("refs").join("heads");
    if let Ok(entries) = fs::read_dir(&git_heads) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                let ivaldi_ref = ivaldi_dir.join("refs/heads").join(&name);
                // Create empty ref file (will be populated on first commit)
                let _ = fs::write(&ivaldi_ref, "");
                imported += 1;
            }
        }
    }

    imported
}

/// Read the current HEAD reference.
/// Returns the timeline name if HEAD is a ref, or the raw hash if detached.
pub fn read_head(ivaldi_dir: &Path) -> Result<HeadRef, ForgeError> {
    let head_path = ivaldi_dir.join("HEAD");
    let content = fs::read_to_string(&head_path).map_err(ForgeError::Io)?;
    let content = content.trim();

    if let Some(ref_path) = content.strip_prefix("ref: refs/heads/") {
        Ok(HeadRef::Timeline(ref_path.to_string()))
    } else {
        Ok(HeadRef::Detached(content.to_string()))
    }
}

/// Write the HEAD reference.
pub fn write_head(ivaldi_dir: &Path, head: &HeadRef) -> Result<(), ForgeError> {
    let head_path = ivaldi_dir.join("HEAD");
    let content = match head {
        HeadRef::Timeline(name) => format!("ref: refs/heads/{}\n", name),
        HeadRef::Detached(hash) => format!("{}\n", hash),
    };
    crate::atomic_io::atomic_write(&head_path, content.as_bytes()).map_err(ForgeError::Io)
}

/// What HEAD points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadRef {
    /// Normal state: points to a timeline name.
    Timeline(String),
    /// Detached state: points to a raw hash.
    Detached(String),
}

/// Check if a directory is an Ivaldi repository.
pub fn is_ivaldi_repo(work_dir: &Path) -> bool {
    work_dir.join(".ivaldi").join("HEAD").exists()
}

/// Find the Ivaldi repository root by searching upward from the given directory.
pub fn find_repo_root(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();
    loop {
        if is_ivaldi_repo(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("repository already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("not an Ivaldi repository")]
    NotARepo,
    #[error(
        "repository format v{found} is newer than this Ivaldi supports (up to v{supported}); upgrade to Ivaldi >= {min_ivaldi}"
    )]
    FormatTooNew {
        found: u32,
        supported: u32,
        min_ivaldi: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_creates_structure() {
        let dir = tempfile::tempdir().unwrap();
        let result = forge(dir.path()).unwrap();

        assert!(!result.already_existed);
        assert_eq!(result.default_timeline, "main");

        // Verify directory structure
        let ivaldi = dir.path().join(".ivaldi");
        assert!(ivaldi.join("objects").is_dir());
        assert!(ivaldi.join("refs/heads").is_dir());
        assert!(ivaldi.join("refs/remotes").is_dir());
        assert!(ivaldi.join("refs/seals").is_dir());
        assert!(ivaldi.join("shelves").is_dir());
        assert!(ivaldi.join("butterflies").is_dir());
        assert!(ivaldi.join("stage").is_dir());
        assert!(ivaldi.join("HEAD").is_file());
        assert!(ivaldi.join("config").is_file());
        assert!(ivaldi.join("FORMAT").is_file());
    }

    #[test]
    fn format_roundtrips_current() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let fmt = read_format(&ivaldi).unwrap();
        assert_eq!(fmt.version, CURRENT_FORMAT);
        assert_eq!(fmt.min_ivaldi, MIN_IVALDI);
        check_format(&ivaldi).unwrap(); // current format opens fine
    }

    #[test]
    fn missing_format_is_version_zero() {
        // Pre-FORMAT repositories have no FORMAT file and must still open.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        assert_eq!(read_format(dir.path()).unwrap().version, 0);
        check_format(dir.path()).unwrap();
    }

    #[test]
    fn newer_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("FORMAT"),
            format!("format = {}\nmin_ivaldi = 9.9.9\n", CURRENT_FORMAT + 1),
        )
        .unwrap();

        match check_format(dir.path()) {
            Err(ForgeError::FormatTooNew {
                found, min_ivaldi, ..
            }) => {
                assert_eq!(found, CURRENT_FORMAT + 1);
                assert_eq!(min_ivaldi, "9.9.9");
            }
            other => panic!("expected FormatTooNew, got {other:?}"),
        }
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // Forward compatibility: a future key must not break an older reader.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("FORMAT"),
            format!("format = {CURRENT_FORMAT}\nfuture_thing = enabled\n"),
        )
        .unwrap();
        assert_eq!(read_format(dir.path()).unwrap().version, CURRENT_FORMAT);
    }

    #[test]
    fn forge_head_points_to_main() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();

        let head = read_head(&dir.path().join(".ivaldi")).unwrap();
        assert_eq!(head, HeadRef::Timeline("main".to_string()));
    }

    #[test]
    fn forge_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let result1 = forge(dir.path()).unwrap();
        assert!(!result1.already_existed);

        let result2 = forge(dir.path()).unwrap();
        assert!(result2.already_existed);
    }

    #[test]
    fn forge_creates_config() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();

        let config = Config::load(&dir.path().join(".ivaldi/config")).unwrap();
        assert_eq!(config.get("color.ui"), Some("true"));
        assert_eq!(config.get("core.autoshelf"), Some("true"));
    }

    #[test]
    fn head_read_write_timeline() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();

        let ivaldi_dir = dir.path().join(".ivaldi");
        write_head(&ivaldi_dir, &HeadRef::Timeline("feature".to_string())).unwrap();

        let head = read_head(&ivaldi_dir).unwrap();
        assert_eq!(head, HeadRef::Timeline("feature".to_string()));
    }

    #[test]
    fn head_read_write_detached() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();

        let ivaldi_dir = dir.path().join(".ivaldi");
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        write_head(&ivaldi_dir, &HeadRef::Detached(hash.to_string())).unwrap();

        let head = read_head(&ivaldi_dir).unwrap();
        assert_eq!(head, HeadRef::Detached(hash.to_string()));
    }

    #[test]
    fn is_ivaldi_repo_check() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_ivaldi_repo(dir.path()));

        forge(dir.path()).unwrap();
        assert!(is_ivaldi_repo(dir.path()));
    }

    #[test]
    fn find_repo_root_from_subdir() {
        let dir = tempfile::tempdir().unwrap();
        forge(dir.path()).unwrap();

        let subdir = dir.path().join("src/deep/nested");
        fs::create_dir_all(&subdir).unwrap();

        let found = find_repo_root(&subdir).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn find_repo_root_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_repo_root(dir.path()).is_none());
    }
}
