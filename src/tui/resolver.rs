//! Shared types for per-file conflict resolution during fuse.
//!
//! The interactive flow itself lives in the Fuse tab view
//! (`crate::tui::views::fuse`), which drives selection through the app's
//! event loop as a modal rather than a separate blocking sub-loop. These
//! types are the contract between that view and the apply logic.

/// A conflict to resolve.
#[derive(Debug, Clone)]
pub struct ConflictItem {
    pub path: String,
    pub description: String,
}

/// Resolution choice for a single conflicted file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolution {
    /// Keep the target timeline's version.
    Ours,
    /// Keep the source timeline's version.
    Theirs,
    /// Concatenate both versions (ours then theirs) into one blob.
    Both,
    /// Leave this file unresolved (the merge will not commit).
    Skip,
}

/// The selectable resolutions, in display order. The index+1 is the hotkey.
pub const CHOICES: &[(&str, Resolution)] = &[
    ("Keep OURS (target timeline)", Resolution::Ours),
    ("Keep THEIRS (source timeline)", Resolution::Theirs),
    ("Keep BOTH (concatenate)", Resolution::Both),
    ("Skip this file", Resolution::Skip),
];
