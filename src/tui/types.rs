//! Shared types for the TUI dashboard.

use std::path::PathBuf;

use crate::repo::Repo;

/// Identifies which tab is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabId {
    Status,
    Log,
    Diff,
    Timelines,
    Remote,
    Fuse,
    Review,
}

impl TabId {
    pub const ALL: [TabId; 7] = [
        TabId::Status,
        TabId::Log,
        TabId::Diff,
        TabId::Timelines,
        TabId::Remote,
        TabId::Fuse,
        TabId::Review,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TabId::Status => "Status",
            TabId::Log => "Log",
            TabId::Diff => "Diff",
            TabId::Timelines => "Timelines",
            TabId::Remote => "Remote",
            TabId::Fuse => "Fuse",
            TabId::Review => "Review",
        }
    }

    pub fn index(self) -> usize {
        match self {
            TabId::Status => 0,
            TabId::Log => 1,
            TabId::Diff => 2,
            TabId::Timelines => 3,
            TabId::Remote => 4,
            TabId::Fuse => 5,
            TabId::Review => 6,
        }
    }

    pub fn from_index(i: usize) -> Option<TabId> {
        match i {
            0 => Some(TabId::Status),
            1 => Some(TabId::Log),
            2 => Some(TabId::Diff),
            3 => Some(TabId::Timelines),
            4 => Some(TabId::Remote),
            5 => Some(TabId::Fuse),
            6 => Some(TabId::Review),
            _ => None,
        }
    }
}

/// Action returned from event handlers to tell the app loop what to do.
pub enum Action {
    /// Event was not handled.
    None,
    /// Event was consumed, no further action.
    Consumed,
    /// Refresh current tab data.
    Refresh,
    /// Update status bar data.
    StatusUpdate(StatusData),
    /// Show an error message.
    Error(String),
    /// Show a success message.
    Success(String),
    /// Quit the application.
    Quit,
}

/// Data displayed in the bottom status bar.
#[derive(Debug, Clone, Default)]
pub struct StatusData {
    pub timeline: String,
    pub seal_name: String,
    pub staged: usize,
    pub modified: usize,
    pub untracked: usize,
    pub deleted: usize,
}

/// Context shared with all tab views.
pub struct AppContext {
    pub repo: Repo,
    pub work_dir: PathBuf,
    pub ivaldi_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_roundtrip() {
        for tab in TabId::ALL {
            assert_eq!(TabId::from_index(tab.index()), Some(tab));
        }
    }

    #[test]
    fn tab_id_from_index_out_of_range() {
        assert_eq!(TabId::from_index(7), None);
        assert_eq!(TabId::from_index(99), None);
    }

    #[test]
    fn tab_labels_non_empty() {
        for tab in TabId::ALL {
            assert!(!tab.label().is_empty());
        }
    }

    #[test]
    fn status_data_default() {
        let sd = StatusData::default();
        assert!(sd.timeline.is_empty());
        assert_eq!(sd.staged, 0);
    }
}
