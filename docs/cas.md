# CAS Module (`cas.rs`)

Content-Addressable Storage for Ivaldi VCS.

## Overview

Every piece of content in Ivaldi (files, directories, seals) is identified by its BLAKE3 hash. The CAS stores and retrieves content by hash, providing automatic deduplication and data integrity verification.

## Trait: `Cas`

The core storage interface.

```rust
pub trait Cas {
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError>;
    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError>;
    fn has(&self, hash: B3Hash) -> Result<bool, CasError>;
}
```

**Hash Verification:** `put` always verifies that the provided hash matches `BLAKE3(data)`. Mismatches are rejected with `CasError::HashMismatch`.

### Helper

```rust
use ivaldi::cas::put_and_hash;

// Hash data and store it in one call
let hash = put_and_hash(&cas, b"content")?;
```

## Implementations

### `MemoryCas`

Thread-safe in-memory CAS for testing. Uses `RwLock<HashMap>`.

```rust
use ivaldi::cas::MemoryCas;

let cas = MemoryCas::new();
assert!(cas.is_empty());
assert_eq!(cas.len(), 0);
```

### `FileCas`

File-based CAS for production use. Uses 2-character directory sharding to avoid filesystem limits.

```rust
use ivaldi::cas::FileCas;

let cas = FileCas::new(".ivaldi/objects")?;
```

**Storage layout:**
```
.ivaldi/objects/
├── ab/
│   └── cdef1234567890...   # hash: abcdef1234567890...
├── de/
│   └── f4567890abcdef...
└── ...
```

**Atomic writes:** Uses temp file + rename to prevent corruption from interrupted writes.

**Idempotent:** Writing the same content twice is a no-op (skips if file exists).

## Error Types

```rust
pub enum CasError {
    NotFound(B3Hash),                           // Hash doesn't exist in store
    HashMismatch { expected: B3Hash, actual: B3Hash }, // Data doesn't match hash
    Io(std::io::Error),                         // Filesystem error
}
```

## Design Decisions

- **Hash verification on write**: Prevents data corruption and ensures integrity
- **2-char sharding**: Keeps directory sizes manageable for large repositories
- **Atomic writes**: Temp file + rename prevents partial writes from corrupting the store
- **Copy semantics**: `MemoryCas` copies data on put/get to prevent external mutation
