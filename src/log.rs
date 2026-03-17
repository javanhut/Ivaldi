//! History log and display for Ivaldi VCS.
//!
//! Provides:
//! - Walking commit history from timeline head backwards
//! - Display-ready commit entries with seal names
//! - Relative time formatting
//! - Filtering options (limit, all timelines)

use crate::hash::B3Hash;
use crate::seal::generate_seal_name;
use crate::timeline::HistoryManager;

/// A display-ready commit entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Leaf index in the MMR.
    pub index: u64,
    /// Full BLAKE3 hash of the leaf.
    pub hash: B3Hash,
    /// Short hex hash (first 8 chars).
    pub short_hash: String,
    /// Deterministic seal name.
    pub seal_name: String,
    /// Commit author.
    pub author: String,
    /// Commit message.
    pub message: String,
    /// Unix timestamp.
    pub time_unix: i64,
    /// Timeline this commit belongs to.
    pub timeline: String,
    /// Whether this is a merge commit.
    pub is_merge: bool,
    /// Parent indices.
    pub parents: Vec<u64>,
}

/// Options for retrieving commit history.
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    /// Maximum number of entries to return (0 = unlimited).
    pub limit: usize,
    /// Show commits from all timelines.
    pub all_timelines: bool,
}

/// Walk the commit history from a timeline head backwards.
pub fn walk_timeline(mgr: &HistoryManager, timeline: &str) -> Vec<LogEntry> {
    let head_idx = match mgr.get_timeline_head(timeline) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut current = Some(head_idx);

    while let Some(idx) = current {
        let leaf = match mgr.mmr.get_leaf(idx) {
            Some(l) => l,
            None => break,
        };

        let hash = leaf.hash();
        entries.push(LogEntry {
            index: idx,
            hash,
            short_hash: hash.short8(),
            seal_name: generate_seal_name(hash),
            author: leaf.author.clone(),
            message: leaf.message.clone(),
            time_unix: leaf.time_unix,
            timeline: leaf.timeline_id.clone(),
            is_merge: leaf.is_merge(),
            parents: leaf.all_parents(),
        });

        current = if leaf.has_parent() {
            Some(leaf.prev_idx)
        } else {
            None
        };
    }

    entries
}

/// Get commit history with options.
pub fn get_log(mgr: &HistoryManager, opts: &LogOptions) -> Vec<LogEntry> {
    let mut entries = if opts.all_timelines {
        let mut all = Vec::new();
        for timeline in mgr.list_timelines() {
            all.extend(walk_timeline(mgr, &timeline));
        }
        // Deduplicate by index (same commit may appear on multiple timelines)
        all.sort_by_key(|e| std::cmp::Reverse(e.time_unix));
        all.dedup_by_key(|e| e.index);
        all
    } else {
        walk_timeline(mgr, mgr.current_timeline())
    };

    if opts.limit > 0 && entries.len() > opts.limit {
        entries.truncate(opts.limit);
    }

    entries
}

/// Format a Unix timestamp as a relative time string.
pub fn relative_time(timestamp: i64, now: i64) -> String {
    let diff = now - timestamp;

    if diff < 0 {
        return "in the future".to_string();
    }

    let seconds = diff;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", minutes)
        }
    } else if hours < 24 {
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else if days < 7 {
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        }
    } else if days < 30 {
        if weeks == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", weeks)
        }
    } else if days < 365 {
        if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        }
    } else if years == 1 {
        "1 year ago".to_string()
    } else {
        format!("{} years ago", years)
    }
}

/// Format a log entry as a full multi-line display.
pub fn format_entry_full(entry: &LogEntry) -> String {
    format!(
        "Seal: {} ({})\nTimeline: {}\nAuthor: {}\nDate: {}\n\n    {}\n",
        entry.seal_name,
        entry.short_hash,
        entry.timeline,
        entry.author,
        entry.time_unix,
        entry.message,
    )
}

/// Format a log entry as a single oneline.
pub fn format_entry_oneline(entry: &LogEntry) -> String {
    format!("{} {} {}", entry.short_hash, entry.seal_name, entry.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaf::Leaf;

    fn setup_mgr() -> HistoryManager {
        let mut mgr = HistoryManager::new();
        for i in 0..5 {
            let leaf = Leaf::new(
                B3Hash::digest(format!("tree {}", i).as_bytes()),
                "",
                "Alice <alice@test.com>",
                1700000000 + i * 3600,
                format!("Commit {}", i),
            );
            mgr.commit("main", leaf).unwrap();
        }
        mgr
    }

    #[test]
    fn walk_timeline_basic() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "main");

        assert_eq!(entries.len(), 5);
        // Most recent first
        assert_eq!(entries[0].message, "Commit 4");
        assert_eq!(entries[4].message, "Commit 0");
    }

    #[test]
    fn walk_timeline_has_seal_names() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "main");

        for entry in &entries {
            assert!(!entry.seal_name.is_empty());
            assert_eq!(entry.short_hash.len(), 8);
            // Seal name should end with the short hash
            assert!(entry.seal_name.ends_with(&entry.short_hash));
        }
    }

    #[test]
    fn walk_timeline_empty() {
        let mgr = HistoryManager::new();
        let entries = walk_timeline(&mgr, "main");
        assert!(entries.is_empty());
    }

    #[test]
    fn walk_nonexistent_timeline() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "nonexistent");
        assert!(entries.is_empty());
    }

    #[test]
    fn get_log_with_limit() {
        let mgr = setup_mgr();
        let opts = LogOptions {
            limit: 3,
            all_timelines: false,
        };
        let entries = get_log(&mgr, &opts);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn get_log_all_timelines() {
        let mut mgr = setup_mgr();
        mgr.create_timeline("feature", None).unwrap();
        let leaf = Leaf::new(
            B3Hash::digest(b"feature tree"),
            "",
            "Bob <bob@test.com>",
            1700100000,
            "Feature commit",
        );
        mgr.commit("feature", leaf).unwrap();

        let opts = LogOptions {
            limit: 0,
            all_timelines: true,
        };
        let entries = get_log(&mgr, &opts);
        // Should include commits from both timelines
        assert!(entries.len() >= 6);
    }

    #[test]
    fn log_entries_have_parents() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "main");

        // First commit (oldest, last in list) has no parents
        assert!(entries.last().unwrap().parents.is_empty());

        // All other commits have one parent
        for entry in &entries[..entries.len() - 1] {
            assert_eq!(entry.parents.len(), 1);
        }
    }

    // ---- Relative time tests ----

    #[test]
    fn relative_time_just_now() {
        assert_eq!(relative_time(1000, 1030), "just now");
    }

    #[test]
    fn relative_time_minutes() {
        assert_eq!(relative_time(1000, 1060), "1 minute ago");
        assert_eq!(relative_time(1000, 1300), "5 minutes ago");
    }

    #[test]
    fn relative_time_hours() {
        assert_eq!(relative_time(1000, 4600), "1 hour ago");
        assert_eq!(relative_time(1000, 11800), "3 hours ago");
    }

    #[test]
    fn relative_time_days() {
        assert_eq!(relative_time(0, 86400), "1 day ago");
        assert_eq!(relative_time(0, 86400 * 3), "3 days ago");
    }

    #[test]
    fn relative_time_weeks() {
        assert_eq!(relative_time(0, 86400 * 7), "1 week ago");
        assert_eq!(relative_time(0, 86400 * 14), "2 weeks ago");
    }

    #[test]
    fn relative_time_months() {
        assert_eq!(relative_time(0, 86400 * 31), "1 month ago");
        assert_eq!(relative_time(0, 86400 * 90), "3 months ago");
    }

    #[test]
    fn relative_time_years() {
        assert_eq!(relative_time(0, 86400 * 366), "1 year ago");
        assert_eq!(relative_time(0, 86400 * 730), "2 years ago");
    }

    // ---- Format tests ----

    #[test]
    fn format_oneline() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "main");
        let line = format_entry_oneline(&entries[0]);

        assert!(line.contains(&entries[0].short_hash));
        assert!(line.contains(&entries[0].seal_name));
        assert!(line.contains("Commit 4"));
    }

    #[test]
    fn format_full() {
        let mgr = setup_mgr();
        let entries = walk_timeline(&mgr, "main");
        let output = format_entry_full(&entries[0]);

        assert!(output.contains("Seal:"));
        assert!(output.contains("Timeline: main"));
        assert!(output.contains("Author: Alice"));
        assert!(output.contains("Commit 4"));
    }
}
