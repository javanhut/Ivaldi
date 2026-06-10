//! Trusted peer store for Ivaldi's user-to-user transport.
//!
//! Each repo (or, when not in a repo, the user's global config dir) keeps a
//! plain-text `.ivaldi/authorized_peers` file. Lines are
//!
//! ```text
//! <pubkey-hex> [name]   # optional comment
//! ```
//!
//! `ivaldi serve` accepts incoming connections only from peers whose Noise
//! handshake remote-static-key appears in this list. `ivaldi peer trust
//! <pubkey> [name]` adds an entry; `ivaldi peer list` shows them; `ivaldi
//! peer forget <pubkey-prefix>` removes one.
//!
//! The file format is line-oriented and human-editable on purpose — modeled
//! after `~/.ssh/authorized_keys`. Comments start with `#`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::identity::KEY_LEN;

/// One entry in the trusted-peers file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerEntry {
    pub pubkey: [u8; KEY_LEN],
    pub name: Option<String>,
}

impl PeerEntry {
    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.pubkey)
    }
}

/// On-disk store for authorized peers.
pub struct PeerStore {
    path: PathBuf,
}

impl PeerStore {
    /// Open (or implicitly create on first save) the peer store at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Standard repo-local location: `<ivaldi_dir>/authorized_peers`.
    pub fn repo_local(ivaldi_dir: &Path) -> Self {
        Self {
            path: ivaldi_dir.join("authorized_peers"),
        }
    }

    /// Read all entries. Returns an empty map if the file doesn't exist —
    /// not an error, just "no peers trusted yet."
    pub fn list(&self) -> Result<Vec<PeerEntry>, PeerError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path).map_err(PeerError::Io)?;
        let mut out = Vec::new();
        let mut by_key: BTreeMap<[u8; KEY_LEN], PeerEntry> = BTreeMap::new();
        for (lineno, raw) in text.lines().enumerate() {
            let trimmed = raw.split('#').next().unwrap_or("").trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let key_hex = parts
                .next()
                .ok_or_else(|| PeerError::Other(format!("line {}: missing pubkey", lineno + 1)))?;
            let pubkey = decode_pubkey(key_hex)
                .map_err(|e| PeerError::Other(format!("line {}: {}", lineno + 1, e)))?;
            let name = parts.next().map(str::to_string);
            // Last entry for a given key wins, but preserve insertion order
            // for display.
            by_key.insert(
                pubkey,
                PeerEntry {
                    pubkey,
                    name: name.clone(),
                },
            );
            out.push(PeerEntry { pubkey, name });
        }
        // Dedup keeping the latest entry for each pubkey.
        let mut seen = std::collections::BTreeSet::new();
        let mut deduped: Vec<PeerEntry> = Vec::new();
        for entry in out.into_iter().rev() {
            if seen.insert(entry.pubkey) {
                deduped.push(entry);
            }
        }
        deduped.reverse();
        Ok(deduped)
    }

    /// True iff `pubkey` is in the store.
    pub fn is_trusted(&self, pubkey: &[u8; KEY_LEN]) -> Result<bool, PeerError> {
        Ok(self.list()?.iter().any(|e| &e.pubkey == pubkey))
    }

    /// Add a peer. Idempotent: a duplicate pubkey replaces the existing
    /// entry's optional name.
    pub fn trust(&self, pubkey: [u8; KEY_LEN], name: Option<&str>) -> Result<(), PeerError> {
        let mut entries = self.list()?;
        entries.retain(|e| e.pubkey != pubkey);
        entries.push(PeerEntry {
            pubkey,
            name: name.map(str::to_string),
        });
        self.write(&entries)
    }

    /// Remove the first peer whose pubkey-hex starts with `prefix`. Returns
    /// the removed entry on success, `None` if no match.
    pub fn forget(&self, prefix: &str) -> Result<Option<PeerEntry>, PeerError> {
        let prefix = prefix.to_lowercase();
        let mut entries = self.list()?;
        let pos = entries
            .iter()
            .position(|e| e.pubkey_hex().starts_with(&prefix));
        match pos {
            Some(idx) => {
                let removed = entries.remove(idx);
                self.write(&entries)?;
                Ok(Some(removed))
            }
            None => Ok(None),
        }
    }

    fn write(&self, entries: &[PeerEntry]) -> Result<(), PeerError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(PeerError::Io)?;
        }
        let mut text = String::new();
        text.push_str("# Ivaldi authorized peers — one per line.\n");
        text.push_str("# Format: <pubkey-hex> [optional-name]\n");
        text.push_str("# Add via `ivaldi peer trust <pubkey> [name]`.\n\n");
        for e in entries {
            text.push_str(&e.pubkey_hex());
            if let Some(name) = &e.name {
                text.push(' ');
                text.push_str(name);
            }
            text.push('\n');
        }
        fs::write(&self.path, text).map_err(PeerError::Io)?;
        Ok(())
    }
}

/// Decode a 64-char hex pubkey (case-insensitive) into 32 bytes.
pub fn decode_pubkey(s: &str) -> Result<[u8; KEY_LEN], String> {
    let raw = hex::decode(s).map_err(|e| format!("not valid hex: {}", e))?;
    if raw.len() != KEY_LEN {
        return Err(format!("expected {} bytes, got {}", KEY_LEN, raw.len()));
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&raw);
    Ok(out)
}

#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    #[error("peers I/O: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn key(b: u8) -> [u8; KEY_LEN] {
        [b; KEY_LEN]
    }

    #[test]
    fn list_empty_when_file_missing() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn trust_then_list() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        store.trust(key(0x11), Some("alice")).unwrap();
        store.trust(key(0x22), None).unwrap();
        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].pubkey, key(0x11));
        assert_eq!(entries[0].name.as_deref(), Some("alice"));
        assert_eq!(entries[1].pubkey, key(0x22));
        assert_eq!(entries[1].name, None);
    }

    #[test]
    fn trust_is_idempotent_and_replaces_name() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        store.trust(key(0x33), Some("first")).unwrap();
        store.trust(key(0x33), Some("second")).unwrap();
        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name.as_deref(), Some("second"));
    }

    #[test]
    fn is_trusted_matches() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        store.trust(key(0xAA), None).unwrap();
        assert!(store.is_trusted(&key(0xAA)).unwrap());
        assert!(!store.is_trusted(&key(0xBB)).unwrap());
    }

    #[test]
    fn forget_by_prefix() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        store.trust(key(0xCD), Some("c")).unwrap();
        store.trust(key(0xEF), Some("e")).unwrap();
        // Hex of [0xCD; 32] starts with "cdcd"; [0xEF; 32] with "efef".
        let removed = store.forget("cdcd").unwrap().unwrap();
        assert_eq!(removed.pubkey, key(0xCD));
        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pubkey, key(0xEF));
    }

    #[test]
    fn forget_no_match_returns_none() {
        let dir = tempdir().unwrap();
        let store = PeerStore::new(dir.path().join("authorized_peers"));
        store.trust(key(0xCD), None).unwrap();
        assert!(store.forget("ffff").unwrap().is_none());
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("authorized_peers");
        let pubkey_hex = hex::encode(key(0x77));
        let content = format!(
            "# header comment\n\n{} bob # trailing comment\n\n# trailing\n",
            pubkey_hex
        );
        fs::write(&path, content).unwrap();
        let store = PeerStore::new(path);
        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name.as_deref(), Some("bob"));
    }

    #[test]
    fn decode_pubkey_validates_length_and_hex() {
        assert!(decode_pubkey("not-hex").is_err());
        assert!(decode_pubkey(&"ab".repeat(31)).is_err()); // 31 bytes
        assert!(decode_pubkey(&"ab".repeat(KEY_LEN)).is_ok());
    }
}
