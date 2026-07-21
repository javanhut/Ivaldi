//! Plain serde view structs for `--json` / `--format json` output.
//!
//! These mirror what the human-readable commands print. They are kept as
//! flat string-based views so domain types (`HistoryEntry`, `Leaf`, …)
//! never need to derive `Serialize` themselves.

use crate::repo::HistoryEntry;

/// JSON view of `ivaldi status`.
#[derive(serde::Serialize)]
pub struct StatusJson {
    pub timeline: String,
    pub head: Option<SealRefJson>,
    pub files: Vec<FileJson>,
    pub staged_deletions: Vec<String>,
}

/// JSON reference to a seal (commit).
#[derive(serde::Serialize)]
pub struct SealRefJson {
    pub seal_name: String,
    pub hash: String,
    pub short_hash: String,
}

/// JSON view of a single workspace file and its state.
#[derive(serde::Serialize)]
pub struct FileJson {
    pub path: String,
    pub state: String,
    pub hash: Option<String>,
}

/// JSON view of one `ivaldi log` entry.
#[derive(serde::Serialize)]
pub struct LogEntryJson {
    pub index: u64,
    pub hash: String,
    pub short_hash: String,
    pub seal_name: String,
    pub author: String,
    pub message: String,
    pub time_unix: i64,
    pub timeline: String,
    pub is_merge: bool,
}

impl From<&HistoryEntry> for LogEntryJson {
    fn from(entry: &HistoryEntry) -> Self {
        Self {
            index: entry.index,
            hash: entry.hash.to_hex(),
            short_hash: entry.short_hash.clone(),
            seal_name: entry.seal_name.clone(),
            author: entry.author.clone(),
            message: entry.message.clone(),
            time_unix: entry.time_unix,
            timeline: entry.timeline.clone(),
            is_merge: entry.is_merge,
        }
    }
}

/// JSON view of one `ivaldi timeline list` row.
#[derive(serde::Serialize)]
pub struct TimelineJson {
    pub name: String,
    pub current: bool,
}

/// JSON view of one `ivaldi portal list` row.
#[derive(serde::Serialize)]
pub struct PortalJson {
    /// `owner/repo` as printed by `portal list`.
    pub repo: String,
    /// Platform name (`github` or `gitlab`).
    pub platform: String,
    /// Custom instance URL, if configured.
    pub url: Option<String>,
    /// True for the default portal (the one `upload`/`sync` target).
    pub default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::B3Hash;

    #[test]
    fn log_entry_json_has_expected_keys() {
        let hash = B3Hash::digest(b"seal content");
        let entry = HistoryEntry {
            index: 7,
            hash,
            seal_name: "swift-eagle".into(),
            short_hash: hash.short8(),
            author: "Jane Doe <jane@example.com>".into(),
            message: "add login".into(),
            time_unix: 1_700_000_000,
            timeline: "main".into(),
            is_merge: true,
        };

        let value = serde_json::to_value(LogEntryJson::from(&entry)).unwrap();
        let obj = value.as_object().unwrap();
        for key in [
            "index",
            "hash",
            "short_hash",
            "seal_name",
            "author",
            "message",
            "time_unix",
            "timeline",
            "is_merge",
        ] {
            assert!(obj.contains_key(key), "missing key: {}", key);
        }

        assert_eq!(value["index"], 7);
        assert_eq!(value["hash"], hash.to_hex());
        assert_eq!(value["short_hash"], hash.short8());
        assert_eq!(value["seal_name"], "swift-eagle");
        assert_eq!(value["author"], "Jane Doe <jane@example.com>");
        assert_eq!(value["message"], "add login");
        assert_eq!(value["time_unix"], 1_700_000_000_i64);
        assert_eq!(value["timeline"], "main");
        assert_eq!(value["is_merge"], true);
    }
}
