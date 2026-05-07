//! Trust-on-first-use (TOFU) store for peers we *connect to* over
//! `ivaldi://`. Mirrors the spirit of `~/.ssh/known_hosts`: the first time
//! a client connects to a given `host:port`, we record the server's
//! pubkey; subsequent connects refuse if the pubkey changes.
//!
//! The "inbound" side — i.e. who is allowed to talk to *our* `ivaldi serve`
//! — uses [`crate::peers::PeerStore`] (per-repo `authorized_peers`), which
//! is strictly explicit and never auto-accepts. TOFU only relaxes the
//! *outbound* side, where the user is consciously choosing to trust a
//! server they're connecting to.
//!
//! On-disk format (line-oriented, plain text, comments allowed):
//!
//! ```text
//! <host>:<port> <pubkey-hex>
//! # comments allowed
//! ```
//!
//! Default location: `~/.ivaldi/known_peers`. Override with
//! `IVALDI_KNOWN_PEERS` for tests / multi-account workflows.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::identity::KEY_LEN;

/// Result of looking up a `(host, port)` in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Known {
    /// We've never seen this peer — caller decides (prompt / auto-accept / strict).
    Unknown,
    /// Stored pubkey matches what we expect.
    Match,
    /// Stored pubkey differs from what we expect — REFUSE.
    Mismatch { stored: [u8; KEY_LEN] },
}

/// Policy for what to do on `Known::Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TofuPolicy {
    /// Print the fingerprint, ask y/N, save on yes. (Default for interactive use.)
    Prompt,
    /// Auto-accept and save. (For scripts / CI.)
    AcceptAll,
    /// Refuse to connect to any unknown peer. (Belt-and-suspenders.)
    StrictKnown,
}

/// On-disk store of known peer pubkeys, keyed by `host:port`.
pub struct KnownPeers {
    path: PathBuf,
}

impl KnownPeers {
    /// Open the store at the given path. Reads happen lazily; nothing on
    /// disk is created until [`KnownPeers::record`] is called.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Open the default store at `~/.ivaldi/known_peers` (overridable via
    /// `IVALDI_KNOWN_PEERS`).
    pub fn default_for_user() -> Option<Self> {
        if let Ok(p) = std::env::var("IVALDI_KNOWN_PEERS") {
            if !p.is_empty() {
                return Some(Self::new(PathBuf::from(p)));
            }
        }
        let base = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)?;
        Some(Self::new(base.join(".ivaldi").join("known_peers")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Look up `(host, port)`; returns one of `Known::{Unknown,Match,Mismatch}`.
    pub fn lookup(
        &self,
        host: &str,
        port: u16,
        expected: &[u8; KEY_LEN],
    ) -> Result<Known, KnownPeersError> {
        let map = self.load_map()?;
        match map.get(&entry_key(host, port)) {
            None => Ok(Known::Unknown),
            Some(stored) if stored == expected => Ok(Known::Match),
            Some(stored) => Ok(Known::Mismatch { stored: *stored }),
        }
    }

    /// Record `(host, port) → pubkey`. Idempotent: re-recording the same
    /// pubkey is a no-op; replacing a different pubkey overwrites (but the
    /// caller should normally refuse to do that — `lookup` returns
    /// `Mismatch` precisely so the caller can choose).
    pub fn record(
        &self,
        host: &str,
        port: u16,
        pubkey: &[u8; KEY_LEN],
    ) -> Result<(), KnownPeersError> {
        let mut map = self.load_map()?;
        map.insert(entry_key(host, port), *pubkey);
        self.write_map(&map)
    }

    /// Forget `(host, port)`. Returns true if removed.
    pub fn forget(&self, host: &str, port: u16) -> Result<bool, KnownPeersError> {
        let mut map = self.load_map()?;
        let removed = map.remove(&entry_key(host, port)).is_some();
        if removed {
            self.write_map(&map)?;
        }
        Ok(removed)
    }

    /// Return all known peers, sorted by `host:port`.
    pub fn list(&self) -> Result<Vec<(String, [u8; KEY_LEN])>, KnownPeersError> {
        let map = self.load_map()?;
        Ok(map.into_iter().collect())
    }

    fn load_map(&self) -> Result<BTreeMap<String, [u8; KEY_LEN]>, KnownPeersError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = fs::read_to_string(&self.path).map_err(KnownPeersError::Io)?;
        let mut out = BTreeMap::new();
        for (lineno, raw) in text.lines().enumerate() {
            let trimmed = raw.split('#').next().unwrap_or("").trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let key = parts.next().ok_or_else(|| {
                KnownPeersError::Other(format!("line {}: missing host:port", lineno + 1))
            })?;
            let pk = parts.next().ok_or_else(|| {
                KnownPeersError::Other(format!("line {}: missing pubkey", lineno + 1))
            })?;
            let pubkey = decode_pubkey(pk).map_err(|e| {
                KnownPeersError::Other(format!("line {}: {}", lineno + 1, e))
            })?;
            out.insert(key.to_string(), pubkey);
        }
        Ok(out)
    }

    fn write_map(
        &self,
        map: &BTreeMap<String, [u8; KEY_LEN]>,
    ) -> Result<(), KnownPeersError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(KnownPeersError::Io)?;
        }
        let mut text = String::new();
        text.push_str("# Ivaldi known peers (TOFU). One line per host:port.\n");
        text.push_str("# Format: host:port pubkey-hex\n\n");
        for (k, v) in map {
            text.push_str(k);
            text.push(' ');
            text.push_str(&hex::encode(v));
            text.push('\n');
        }
        fs::write(&self.path, text).map_err(KnownPeersError::Io)?;
        Ok(())
    }
}

fn entry_key(host: &str, port: u16) -> String {
    format!("{}:{}", host, port)
}

fn decode_pubkey(s: &str) -> Result<[u8; KEY_LEN], String> {
    let raw = hex::decode(s).map_err(|e| format!("not valid hex: {}", e))?;
    if raw.len() != KEY_LEN {
        return Err(format!("expected {} bytes, got {}", KEY_LEN, raw.len()));
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&raw);
    Ok(out)
}

/// Resolve a fingerprint shown to a user — for now just hex, but factored
/// out so future short-form fingerprints (BLAKE3 hashes, base58, …) are a
/// one-line change.
pub fn fingerprint(pubkey: &[u8; KEY_LEN]) -> String {
    hex::encode(pubkey)
}

#[derive(Debug, thiserror::Error)]
pub enum KnownPeersError {
    #[error("known_peers I/O: {0}")]
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
    fn lookup_unknown_when_file_missing() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        let r = kp.lookup("h", 9999, &key(0x11)).unwrap();
        assert_eq!(r, Known::Unknown);
    }

    #[test]
    fn record_then_lookup_match() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        kp.record("h", 9999, &key(0x11)).unwrap();
        assert_eq!(kp.lookup("h", 9999, &key(0x11)).unwrap(), Known::Match);
    }

    #[test]
    fn lookup_mismatch_when_pubkey_changes() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        kp.record("h", 9999, &key(0x11)).unwrap();
        match kp.lookup("h", 9999, &key(0x22)).unwrap() {
            Known::Mismatch { stored } => assert_eq!(stored, key(0x11)),
            other => panic!("expected mismatch, got {:?}", other),
        }
    }

    #[test]
    fn forget_removes_entry() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        kp.record("h", 9999, &key(0x11)).unwrap();
        assert!(kp.forget("h", 9999).unwrap());
        assert_eq!(kp.lookup("h", 9999, &key(0x11)).unwrap(), Known::Unknown);
    }

    #[test]
    fn forget_no_match_returns_false() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        assert!(!kp.forget("h", 9999).unwrap());
    }

    #[test]
    fn list_sorted_by_key() {
        let dir = tempdir().unwrap();
        let kp = KnownPeers::new(dir.path().join("known_peers"));
        kp.record("b.example", 9999, &key(0x22)).unwrap();
        kp.record("a.example", 9999, &key(0x11)).unwrap();
        let entries = kp.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].0.starts_with("a.example:"));
        assert!(entries[1].0.starts_with("b.example:"));
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("known_peers");
        let pubkey_hex = hex::encode(key(0x77));
        let content = format!(
            "# top comment\n\nhost:9999 {} # tail\n\n",
            pubkey_hex
        );
        std::fs::write(&path, content).unwrap();
        let kp = KnownPeers::new(path);
        assert_eq!(kp.lookup("host", 9999, &key(0x77)).unwrap(), Known::Match);
    }
}
