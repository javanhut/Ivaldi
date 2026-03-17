# Hash Module (`hash.rs`)

BLAKE3-based hashing system for Ivaldi VCS.

## Overview

All internal Ivaldi operations use BLAKE3-256 hashing exclusively. SHA1 exists only as a compatibility mapping for GitHub/GitLab remote sync and is never used in the internal pipeline.

## Types

### `B3Hash`

A 32-byte BLAKE3-256 hash value. This is the fundamental identifier for all content in Ivaldi.

```rust
use ivaldi::hash::B3Hash;

// Compute hash from data
let hash = B3Hash::digest(b"file content");

// Display
println!("{}", hash);           // full 64-char hex
println!("{:?}", hash);         // B3Hash(abcd1234)
println!("{}", hash.short8());  // first 8 hex chars

// Hex roundtrip
let hex_str = hash.to_hex();
let parsed = B3Hash::from_hex(&hex_str).unwrap();
assert_eq!(hash, parsed);

// Prefix matching (for partial hash lookups)
assert!(hash.matches_prefix("abcd"));

// From raw bytes
let from_bytes = B3Hash::from_bytes([0u8; 32]);
let from_slice = B3Hash::from_slice(&[0u8; 32]).unwrap();
```

**Properties:**
- Deterministic: same input always produces the same hash
- Implements `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, `PartialOrd`, `Ord`
- `B3Hash::ZERO` constant for the all-zeros hash

### `Sha1Hash`

A 20-byte SHA1 hash used **only** for GitHub/GitLab compatibility mapping.

```rust
use ivaldi::hash::Sha1Hash;

let sha1 = Sha1Hash::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
println!("{}", sha1.to_hex());
```

### `DualHash`

Maps a seal to both its BLAKE3 (internal) and optional SHA1 (remote) hashes.

```rust
use ivaldi::hash::{B3Hash, DualHash, Sha1Hash};

// Internal-only (no remote sync yet)
let dual = DualHash::new(B3Hash::digest(b"content"));

// After remote sync, SHA1 is populated
let sha1 = Sha1Hash::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
let dual = DualHash::with_sha1(B3Hash::digest(b"content"), sha1);
```

## Design Decisions

- **BLAKE3 over SHA-256/SHA-1**: ~10x faster, parallelizable, cryptographically secure
- **SHA1 as optional**: Only populated during `upload`/`download` operations with GitHub/GitLab
- **Prefix matching**: Supports partial hash lookups (minimum 4 characters) for user-facing commands like `ivaldi diff 447a`
