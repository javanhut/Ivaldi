//! Content-Addressable Storage (CAS) for Ivaldi VCS.
//!
//! Every piece of content is identified by its BLAKE3 hash.
//! Provides both in-memory (testing) and file-based (production) implementations.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::hash::B3Hash;

/// Errors that can occur during CAS operations.
#[derive(Debug, thiserror::Error)]
pub enum CasError {
    #[error("hash not found: {0}")]
    NotFound(B3Hash),

    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: B3Hash, actual: B3Hash },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Content-Addressable Storage trait.
pub trait Cas {
    /// Store data keyed by its hash. Verifies hash matches content.
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError>;

    /// Retrieve data by its hash.
    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError>;

    /// Check if data exists for the given hash.
    fn has(&self, hash: B3Hash) -> Result<bool, CasError>;
}

/// Convenience method: hash data and store it, returning the hash.
pub fn put_and_hash(cas: &dyn Cas, data: &[u8]) -> Result<B3Hash, CasError> {
    let hash = B3Hash::digest(data);
    cas.put(hash, data)?;
    Ok(hash)
}

// ---------------------------------------------------------------------------
// In-memory CAS (for testing)
// ---------------------------------------------------------------------------

/// Thread-safe in-memory CAS implementation.
pub struct MemoryCas {
    data: RwLock<HashMap<B3Hash, Vec<u8>>>,
}

impl MemoryCas {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<B3Hash, Vec<u8>>> {
        self.data.read().expect("lock poisoned")
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<B3Hash, Vec<u8>>> {
        self.data.write().expect("lock poisoned")
    }
}

impl Default for MemoryCas {
    fn default() -> Self {
        Self::new()
    }
}

impl Cas for MemoryCas {
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError> {
        let computed = B3Hash::digest(data);
        if computed != hash {
            return Err(CasError::HashMismatch {
                expected: hash,
                actual: computed,
            });
        }
        self.write().insert(hash, data.to_vec());
        Ok(())
    }

    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError> {
        self.read()
            .get(&hash)
            .cloned()
            .ok_or(CasError::NotFound(hash))
    }

    fn has(&self, hash: B3Hash) -> Result<bool, CasError> {
        Ok(self.read().contains_key(&hash))
    }
}

// ---------------------------------------------------------------------------
// File-based CAS (production)
// ---------------------------------------------------------------------------

/// File-based CAS with 2-character directory sharding.
///
/// Storage layout: `<root>/<first2hex>/<remaining_hex>`
pub struct FileCas {
    root: PathBuf,
}

impl FileCas {
    /// Create a new file-based CAS rooted at the given directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, CasError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Get the file path for a given hash.
    fn object_path(&self, hash: B3Hash) -> PathBuf {
        let hex = hash.to_hex();
        let (dir, file) = hex.split_at(2);
        self.root.join(dir).join(file)
    }
}

impl Cas for FileCas {
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError> {
        // Verify hash matches content
        let computed = B3Hash::digest(data);
        if computed != hash {
            return Err(CasError::HashMismatch {
                expected: hash,
                actual: computed,
            });
        }

        let path = self.object_path(hash);

        // Already exists — content-addressed, no need to rewrite
        if path.exists() {
            return Ok(());
        }

        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write to temp file then rename (atomic on most filesystems)
        let tmp_path = path.with_extension("tmp");
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Err(CasError::NotFound(hash));
        }
        Ok(fs::read(&path)?)
    }

    fn has(&self, hash: B3Hash) -> Result<bool, CasError> {
        Ok(self.object_path(hash).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- MemoryCas tests ----

    #[test]
    fn memory_put_get_roundtrip() {
        let cas = MemoryCas::new();
        let data = b"hello ivaldi";
        let hash = B3Hash::digest(data);

        cas.put(hash, data).unwrap();
        let retrieved = cas.get(hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn memory_put_rejects_mismatch() {
        let cas = MemoryCas::new();
        let data = b"hello";
        let wrong_hash = B3Hash::digest(b"wrong");

        let result = cas.put(wrong_hash, data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CasError::HashMismatch { .. }));
    }

    #[test]
    fn memory_get_not_found() {
        let cas = MemoryCas::new();
        let hash = B3Hash::digest(b"nonexistent");

        let result = cas.get(hash);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CasError::NotFound(_)));
    }

    #[test]
    fn memory_has() {
        let cas = MemoryCas::new();
        let data = b"test";
        let hash = B3Hash::digest(data);

        assert!(!cas.has(hash).unwrap());
        cas.put(hash, data).unwrap();
        assert!(cas.has(hash).unwrap());
    }

    #[test]
    fn memory_len() {
        let cas = MemoryCas::new();
        assert_eq!(cas.len(), 0);
        assert!(cas.is_empty());

        let data = b"data";
        let hash = B3Hash::digest(data);
        cas.put(hash, data).unwrap();
        assert_eq!(cas.len(), 1);
        assert!(!cas.is_empty());
    }

    #[test]
    fn memory_put_idempotent() {
        let cas = MemoryCas::new();
        let data = b"same data";
        let hash = B3Hash::digest(data);

        cas.put(hash, data).unwrap();
        cas.put(hash, data).unwrap();
        assert_eq!(cas.len(), 1);
    }

    #[test]
    fn put_and_hash_helper() {
        let cas = MemoryCas::new();
        let data = b"content";
        let hash = put_and_hash(&cas, data).unwrap();
        assert_eq!(hash, B3Hash::digest(data));
        assert_eq!(cas.get(hash).unwrap(), data);
    }

    // ---- FileCas tests ----

    #[test]
    fn file_put_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let data = b"hello file cas";
        let hash = B3Hash::digest(data);

        cas.put(hash, data).unwrap();
        let retrieved = cas.get(hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn file_sharding_layout() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let data = b"test sharding";
        let hash = B3Hash::digest(data);
        cas.put(hash, data).unwrap();

        let hex = hash.to_hex();
        let shard_dir = dir.path().join(&hex[..2]);
        assert!(shard_dir.exists());
        let object_file = shard_dir.join(&hex[2..]);
        assert!(object_file.exists());
    }

    #[test]
    fn file_put_rejects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let data = b"hello";
        let wrong_hash = B3Hash::digest(b"wrong");

        let result = cas.put(wrong_hash, data);
        assert!(matches!(result.unwrap_err(), CasError::HashMismatch { .. }));
    }

    #[test]
    fn file_get_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let hash = B3Hash::digest(b"nonexistent");

        let result = cas.get(hash);
        assert!(matches!(result.unwrap_err(), CasError::NotFound(_)));
    }

    #[test]
    fn file_has() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let data = b"exists";
        let hash = B3Hash::digest(data);

        assert!(!cas.has(hash).unwrap());
        cas.put(hash, data).unwrap();
        assert!(cas.has(hash).unwrap());
    }

    #[test]
    fn file_put_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();
        let data = b"idempotent";
        let hash = B3Hash::digest(data);

        cas.put(hash, data).unwrap();
        cas.put(hash, data).unwrap();
        // Should succeed without error
        assert_eq!(cas.get(hash).unwrap(), data);
    }

    #[test]
    fn file_multiple_objects() {
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path()).unwrap();

        for i in 0..10 {
            let data = format!("object {}", i);
            let hash = B3Hash::digest(data.as_bytes());
            cas.put(hash, data.as_bytes()).unwrap();
        }

        for i in 0..10 {
            let data = format!("object {}", i);
            let hash = B3Hash::digest(data.as_bytes());
            assert_eq!(cas.get(hash).unwrap(), data.as_bytes());
        }
    }
}
