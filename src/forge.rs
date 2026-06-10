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
