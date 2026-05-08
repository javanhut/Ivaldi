//! Long-lived peer identity for Ivaldi's user-to-user (P2P) transport.
//!
//! Each user has a single static X25519 keypair stored at
//! `~/.ivaldi/identity` (file mode 0600 on Unix). The public half is what
//! other users add to their `.ivaldi/authorized_peers` file via
//! `ivaldi peer trust`. The private half never leaves the machine and is
//! used as the static key in the Noise XX handshake (see `p2p.rs`).
//!
//! On-disk format is a small JSON document so it's easy to inspect and
//! to evolve:
//!
//! ```json
//! { "version": 1, "secret_hex": "<64 hex chars>", "public_hex": "<64 hex chars>" }
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Length of an X25519 key (public or secret) in bytes.
pub const KEY_LEN: usize = 32;

/// A fully-resolved Ivaldi peer identity.
#[derive(Debug, Clone)]
pub struct Identity {
    pub secret: [u8; KEY_LEN],
    pub public: [u8; KEY_LEN],
}

impl Identity {
    /// Generate a fresh keypair. Uses `snow`'s built-in DH so we don't pull
    /// in `x25519-dalek` separately.
    pub fn generate() -> Result<Self, IdentityError> {
        // `snow::Builder::new(...).generate_keypair()` is the canonical way
        // to mint an X25519 keypair without depending on a specific RNG.
        let params: snow::params::NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s"
            .parse()
            .map_err(|e: snow::Error| IdentityError::Other(e.to_string()))?;
        let kp = snow::Builder::new(params)
            .generate_keypair()
            .map_err(|e| IdentityError::Other(e.to_string()))?;
        let mut secret = [0u8; KEY_LEN];
        let mut public = [0u8; KEY_LEN];
        if kp.private.len() != KEY_LEN || kp.public.len() != KEY_LEN {
            return Err(IdentityError::Other(format!(
                "unexpected key length from snow: secret={}, public={}",
                kp.private.len(),
                kp.public.len()
            )));
        }
        secret.copy_from_slice(&kp.private);
        public.copy_from_slice(&kp.public);
        Ok(Self { secret, public })
    }

    /// Load the identity from disk, or generate-and-save if missing.
    /// `path` is typically `~/.ivaldi/identity` resolved by `default_path`.
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        if let Some(id) = Self::load(path)? {
            return Ok(id);
        }
        let id = Self::generate()?;
        id.save(path)?;
        Ok(id)
    }

    /// Load the identity from disk, returning `None` if the file doesn't
    /// exist (so the caller can decide whether to mint a fresh one).
    pub fn load(path: &Path) -> Result<Option<Self>, IdentityError> {
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(IdentityError::Io)?;
        let stored: StoredIdentity = serde_json::from_slice(&bytes)
            .map_err(|e| IdentityError::Other(format!("identity file corrupt: {}", e)))?;
        if stored.version != 1 {
            return Err(IdentityError::Other(format!(
                "identity file version {} not supported (expected 1)",
                stored.version
            )));
        }
        let secret = decode_key(&stored.secret_hex, "secret")?;
        let public = decode_key(&stored.public_hex, "public")?;
        Ok(Some(Self { secret, public }))
    }

    /// Persist the identity to disk with restrictive permissions.
    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(IdentityError::Io)?;
        }
        let stored = StoredIdentity {
            version: 1,
            secret_hex: hex::encode(self.secret),
            public_hex: hex::encode(self.public),
        };
        let json = serde_json::to_vec_pretty(&stored)
            .map_err(|e| IdentityError::Other(e.to_string()))?;
        // Atomic-ish write: temp file + rename, then chmod 0600 on Unix.
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &json).map_err(IdentityError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, path).map_err(IdentityError::Io)?;
        Ok(())
    }

    /// Display fingerprint of the public key — full 64-char hex. We don't
    /// shorten (yet) so users can copy/paste the exact value into a peer's
    /// `peer trust` command without ambiguity.
    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.public)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    secret_hex: String,
    public_hex: String,
}

fn decode_key(hex_str: &str, label: &str) -> Result<[u8; KEY_LEN], IdentityError> {
    let raw = hex::decode(hex_str)
        .map_err(|e| IdentityError::Other(format!("{} key not valid hex: {}", label, e)))?;
    if raw.len() != KEY_LEN {
        return Err(IdentityError::Other(format!(
            "{} key wrong length: {} (expected {})",
            label,
            raw.len(),
            KEY_LEN
        )));
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&raw);
    Ok(out)
}

/// Default location of the identity file. Returns `None` if `$HOME` (and
/// `USERPROFILE` on Windows) are both unset.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(base.join(".ivaldi").join("identity"))
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("identity I/O: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generate_produces_distinct_keys() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.public, b.public);
        assert_ne!(a.secret, b.secret);
        assert_eq!(a.public.len(), KEY_LEN);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity");
        let id = Identity::generate().unwrap();
        id.save(&path).unwrap();
        let loaded = Identity::load(&path).unwrap().unwrap();
        assert_eq!(id.public, loaded.public);
        assert_eq!(id.secret, loaded.secret);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope");
        assert!(Identity::load(&path).unwrap().is_none());
    }

    #[test]
    fn load_or_create_mints_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity");
        assert!(!path.exists());
        let id = Identity::load_or_create(&path).unwrap();
        assert!(path.exists());
        // Calling again returns the same identity (no re-mint).
        let again = Identity::load_or_create(&path).unwrap();
        assert_eq!(id.public, again.public);
    }

    #[test]
    fn pubkey_hex_is_64_chars() {
        let id = Identity::generate().unwrap();
        assert_eq!(id.pubkey_hex().len(), 64);
        assert!(id.pubkey_hex().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn load_rejects_wrong_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity");
        let bad = r#"{"version":99,"secret_hex":"00","public_hex":"00"}"#;
        std::fs::write(&path, bad).unwrap();
        let err = Identity::load(&path).unwrap_err();
        assert!(format!("{}", err).contains("version 99"));
    }
}
