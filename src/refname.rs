//! Canonical validation and path construction for timeline references.
//!
//! Timeline names cross several trust boundaries: they are database keys,
//! filesystem paths under `refs/heads`, Git branch names, and network input.
//! Keeping the rules here prevents one subsystem from accepting a name another
//! subsystem cannot safely or losslessly represent.

use std::path::{Path, PathBuf};

const MAX_REF_BYTES: usize = 255;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RefNameError {
    #[error("invalid timeline name {name:?}: {reason}")]
    Invalid { name: String, reason: &'static str },
}

fn invalid(name: &str, reason: &'static str) -> RefNameError {
    RefNameError::Invalid {
        name: name.to_string(),
        reason,
    }
}

/// Validate a timeline name as a safe relative ref path and Git branch name.
/// Nested names such as `feature/parser` are supported.
pub fn validate_timeline_name(name: &str) -> Result<(), RefNameError> {
    if name.is_empty() {
        return Err(invalid(name, "name is empty"));
    }
    if name.len() > MAX_REF_BYTES {
        return Err(invalid(name, "name exceeds 255 UTF-8 bytes"));
    }
    if name == "@" {
        return Err(invalid(name, "'@' is reserved by Git"));
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err(invalid(name, "empty path components are not allowed"));
    }
    if name.contains("..") {
        return Err(invalid(name, "'..' is not allowed"));
    }
    if name.contains("@{") {
        return Err(invalid(name, "'@{' is reserved by Git"));
    }
    if name.chars().any(|c| {
        c.is_control() || c == ' ' || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return Err(invalid(
            name,
            "contains a control character or Git-incompatible character",
        ));
    }
    for component in name.split('/') {
        if component == "." || component == ".." {
            return Err(invalid(name, "dot path components are not allowed"));
        }
        if component.starts_with('.') {
            return Err(invalid(name, "path components may not start with '.'"));
        }
        if component.ends_with('.') {
            return Err(invalid(name, "path components may not end with '.'"));
        }
        if component.ends_with(".lock") {
            return Err(invalid(name, "path components may not end with '.lock'"));
        }
        let device_stem = component.split('.').next().unwrap_or(component);
        let upper = device_stem.to_ascii_uppercase();
        if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || upper
                .strip_prefix("COM")
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
            || upper
                .strip_prefix("LPT")
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        {
            return Err(invalid(name, "contains a Windows reserved device name"));
        }
    }
    Ok(())
}

/// Return the filesystem marker path for a validated timeline name.
pub fn timeline_ref_path(ivaldi_dir: &Path, name: &str) -> Result<PathBuf, RefNameError> {
    validate_timeline_name(name)?;
    Ok(ivaldi_dir.join("refs/heads").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_and_nested_names() {
        for name in ["main", "feature/parser", "release-1.0", "café"] {
            assert_eq!(validate_timeline_name(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn rejects_path_escape_and_git_incompatible_names() {
        for name in [
            "",
            "../escape",
            "/main",
            "main/",
            "a//b",
            ".hidden",
            "a/../b",
            "a\\b",
            "bad name",
            "bad.lock",
            "topic@{1}",
            "bad~name",
            "@",
            "CON",
            "aux.txt",
            "COM1",
        ] {
            assert!(validate_timeline_name(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn path_is_always_below_heads() {
        let path = timeline_ref_path(Path::new("/repo/.ivaldi"), "feature/parser").unwrap();
        assert_eq!(path, Path::new("/repo/.ivaldi/refs/heads/feature/parser"));
    }
}
