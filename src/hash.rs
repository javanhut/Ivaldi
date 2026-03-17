//! BLAKE3-based hashing for Ivaldi VCS.
//!
//! All internal operations use BLAKE3-256. SHA1 is only used as a
//! compatibility mapping for GitHub/GitLab sync and is never part
//! of the internal pipeline.

use std::fmt;

/// A BLAKE3-256 hash value (32 bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct B3Hash([u8; 32]);

impl B3Hash {
    /// The zero hash (all bytes zero).
    pub const ZERO: Self = Self([0u8; 32]);

    /// Create a hash from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Create a hash from a byte slice. Returns `None` if slice length != 32.
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(slice);
        Some(Self(bytes))
    }

    /// Parse a hash from a hex string. Returns `None` if invalid.
    pub fn from_hex(s: &str) -> Option<Self> {
        let decoded = hex::decode(s).ok()?;
        Self::from_slice(&decoded)
    }

    /// Return the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Return the full hex string (64 characters).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Return a short hex prefix (first `n` characters, default 8).
    pub fn short(&self, n: usize) -> String {
        let full = self.to_hex();
        full[..n.min(full.len())].to_string()
    }

    /// Return the first 8 hex characters.
    pub fn short8(&self) -> String {
        self.short(8)
    }

    /// Check if a hex prefix matches this hash.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        self.to_hex().starts_with(prefix)
    }

    /// Compute the BLAKE3 hash of the given data.
    pub fn digest(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(*hash.as_bytes())
    }
}

impl fmt::Debug for B3Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B3Hash({})", self.short8())
    }
}

impl fmt::Display for B3Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl AsRef<[u8]> for B3Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// SHA1 hash used ONLY for GitHub/GitLab compatibility mapping.
/// Never used in internal operations.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Sha1Hash([u8; 20]);

impl Sha1Hash {
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let decoded = hex::decode(s).ok()?;
        if decoded.len() != 20 {
            return None;
        }
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&decoded);
        Some(Self(bytes))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Sha1Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// A dual hash mapping: seal name → (BLAKE3, optional SHA1).
/// SHA1 is only populated during remote sync operations.
#[derive(Clone, Debug)]
pub struct DualHash {
    pub blake3: B3Hash,
    pub sha1: Option<Sha1Hash>,
}

impl DualHash {
    pub fn new(blake3: B3Hash) -> Self {
        Self { blake3, sha1: None }
    }

    pub fn with_sha1(blake3: B3Hash, sha1: Sha1Hash) -> Self {
        Self {
            blake3,
            sha1: Some(sha1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_deterministic() {
        let data = b"hello ivaldi";
        let h1 = B3Hash::digest(data);
        let h2 = B3Hash::digest(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn digest_different_data_different_hash() {
        let h1 = B3Hash::digest(b"hello");
        let h2 = B3Hash::digest(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_data_has_valid_hash() {
        let h = B3Hash::digest(b"");
        assert_ne!(h, B3Hash::ZERO);
    }

    #[test]
    fn hex_roundtrip() {
        let h = B3Hash::digest(b"test data");
        let hex_str = h.to_hex();
        assert_eq!(hex_str.len(), 64);
        let parsed = B3Hash::from_hex(&hex_str).unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn from_hex_invalid() {
        assert!(B3Hash::from_hex("not_hex").is_none());
        assert!(B3Hash::from_hex("abcd").is_none()); // too short
        assert!(B3Hash::from_hex("").is_none());
    }

    #[test]
    fn short_prefix() {
        let h = B3Hash::digest(b"test");
        let s = h.short8();
        assert_eq!(s.len(), 8);
        assert!(h.to_hex().starts_with(&s));
    }

    #[test]
    fn matches_prefix() {
        let h = B3Hash::digest(b"test");
        let hex = h.to_hex();
        assert!(h.matches_prefix(&hex[..4]));
        assert!(h.matches_prefix(&hex[..8]));
        assert!(h.matches_prefix(&hex));
        assert!(!h.matches_prefix("0000000000"));
    }

    #[test]
    fn from_slice_valid() {
        let h = B3Hash::digest(b"data");
        let from_slice = B3Hash::from_slice(h.as_bytes()).unwrap();
        assert_eq!(h, from_slice);
    }

    #[test]
    fn from_slice_wrong_length() {
        assert!(B3Hash::from_slice(&[0u8; 31]).is_none());
        assert!(B3Hash::from_slice(&[0u8; 33]).is_none());
    }

    #[test]
    fn display_and_debug() {
        let h = B3Hash::digest(b"test");
        let display = format!("{}", h);
        assert_eq!(display.len(), 64);
        let debug = format!("{:?}", h);
        assert!(debug.starts_with("B3Hash("));
    }

    #[test]
    fn hash_ordering() {
        let h1 = B3Hash::from_bytes([0u8; 32]);
        let h2 = B3Hash::from_bytes([1u8; 32]);
        assert!(h1 < h2);
    }

    #[test]
    fn sha1_hex_roundtrip() {
        let hex_str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let h = Sha1Hash::from_hex(hex_str).unwrap();
        assert_eq!(h.to_hex(), hex_str);
    }

    #[test]
    fn sha1_from_hex_invalid() {
        assert!(Sha1Hash::from_hex("tooshort").is_none());
        assert!(Sha1Hash::from_hex("").is_none());
    }

    #[test]
    fn dual_hash_without_sha1() {
        let b3 = B3Hash::digest(b"test");
        let dh = DualHash::new(b3);
        assert_eq!(dh.blake3, b3);
        assert!(dh.sha1.is_none());
    }

    #[test]
    fn dual_hash_with_sha1() {
        let b3 = B3Hash::digest(b"test");
        let sha1 = Sha1Hash::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
        let dh = DualHash::with_sha1(b3, sha1);
        assert_eq!(dh.blake3, b3);
        assert_eq!(dh.sha1.unwrap(), sha1);
    }
}
