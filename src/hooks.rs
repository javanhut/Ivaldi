//! Hook system for Ivaldi VCS.
//!
//! Hooks are executable scripts in `.ivaldi/hooks/` that run before/after
//! specific operations. If a pre-hook exits non-zero, the operation is aborted.
//!
//! Supported hooks:
//! - `pre-seal` / `post-seal` — before/after creating a commit
//! - `pre-fuse` / `post-fuse` — before/after merging
//! - `pre-upload` / `post-upload` — before/after pushing
//! - `pre-switch` / `post-switch` — before/after timeline switch

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Hook execution points.
#[derive(Debug, Clone, Copy)]
pub enum Hook {
    PreSeal,
    PostSeal,
    PreFuse,
    PostFuse,
    PreUpload,
    PostUpload,
    PreSwitch,
    PostSwitch,
}

impl Hook {
    pub fn filename(&self) -> &str {
        match self {
            Hook::PreSeal => "pre-seal",
            Hook::PostSeal => "post-seal",
            Hook::PreFuse => "pre-fuse",
            Hook::PostFuse => "post-fuse",
            Hook::PreUpload => "pre-upload",
            Hook::PostUpload => "post-upload",
            Hook::PreSwitch => "pre-switch",
            Hook::PostSwitch => "post-switch",
        }
    }

    pub fn is_pre(&self) -> bool {
        matches!(self, Hook::PreSeal | Hook::PreFuse | Hook::PreUpload | Hook::PreSwitch)
    }
}

/// Hook manager.
pub struct HookManager {
    hooks_dir: PathBuf,
}

impl HookManager {
    pub fn new(ivaldi_dir: &Path) -> Self {
        let hooks_dir = ivaldi_dir.join("hooks");
        let _ = fs::create_dir_all(&hooks_dir);
        Self { hooks_dir }
    }

    /// Run a hook. Returns Ok(true) if hook ran successfully or didn't exist.
    /// Returns Ok(false) if a pre-hook failed (operation should abort).
    /// Returns Err on execution errors.
    pub fn run(&self, hook: Hook, args: &[&str]) -> Result<bool, HookError> {
        let path = self.hooks_dir.join(hook.filename());
        if !path.exists() {
            return Ok(true); // No hook = success
        }

        // Check if executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&path).map_err(HookError::Io)?;
            if meta.permissions().mode() & 0o111 == 0 {
                return Err(HookError::NotExecutable(hook.filename().to_string()));
            }
        }

        crate::logging::debug(&format!("Running hook: {}", hook.filename()));

        let output = Command::new(&path)
            .args(args)
            .current_dir(self.hooks_dir.parent().unwrap_or(Path::new(".")))
            .output()
            .map_err(HookError::Io)?;

        if output.status.success() {
            Ok(true)
        } else {
            if hook.is_pre() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    eprintln!("Hook {} failed:\n{}", hook.filename(), stderr.trim());
                }
                Ok(false) // Pre-hook failed — abort
            } else {
                // Post-hooks don't abort, just warn
                crate::logging::warn(&format!("Post-hook {} exited with error", hook.filename()));
                Ok(true)
            }
        }
    }

    /// Install a hook script.
    pub fn install(&self, hook: Hook, script: &str) -> Result<(), HookError> {
        let path = self.hooks_dir.join(hook.filename());
        fs::write(&path, script).map_err(HookError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(HookError::Io)?;
        }
        Ok(())
    }

    /// List installed hooks.
    pub fn list(&self) -> Vec<String> {
        let mut hooks = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.hooks_dir) {
            for entry in entries.flatten() {
                hooks.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        hooks.sort();
        hooks
    }

    /// Check if a hook exists.
    pub fn exists(&self, hook: Hook) -> bool {
        self.hooks_dir.join(hook.filename()).exists()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("hook not executable: {0}")]
    NotExecutable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_hook_returns_success() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = HookManager::new(dir.path());
        assert!(mgr.run(Hook::PreSeal, &[]).unwrap());
    }

    #[test]
    fn list_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = HookManager::new(dir.path());
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn install_and_exists() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();
        let mgr = HookManager::new(&ivaldi_dir);
        mgr.install(Hook::PreSeal, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(mgr.exists(Hook::PreSeal));
        assert!(!mgr.exists(Hook::PostSeal));
    }

    #[test]
    fn hook_filenames() {
        assert_eq!(Hook::PreSeal.filename(), "pre-seal");
        assert_eq!(Hook::PostUpload.filename(), "post-upload");
        assert!(Hook::PreSeal.is_pre());
        assert!(!Hook::PostSeal.is_pre());
    }
}
