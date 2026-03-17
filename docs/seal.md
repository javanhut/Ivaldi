# Seal Module (`seal.rs`)

Seal name generation for Ivaldi VCS.

## Overview

Every seal (commit) gets a deterministic, memorable name derived from its BLAKE3 hash. The seal name is the **key** in Ivaldi's key-value hash system.

## Format

```
adjective-noun-verb-adverb-shortHash
```

Example: `swift-eagle-flies-high-447abe9b`

```
swift    ← adjective (from 64-word list)
eagle    ← noun (from 64-word list)
flies    ← verb (from 64-word list)
high     ← adverb (from 64-word list)
447abe9b ← first 8 hex chars of BLAKE3 hash
```

## Usage

```rust
use ivaldi::seal::generate_seal_name;
use ivaldi::hash::B3Hash;

let hash = B3Hash::digest(b"commit content");
let name = generate_seal_name(hash);
// e.g., "bold-mountain-soars-deep-a1b2c3d4"

// Deterministic: same hash → same name
assert_eq!(generate_seal_name(hash), generate_seal_name(hash));
```

## Matching

Users can reference seals by partial name or hash:

```rust
use ivaldi::seal::matches_seal_name;

let full = "swift-eagle-flies-high-447abe9b";

assert!(matches_seal_name(full, full));                // exact match
assert!(matches_seal_name(full, "swift-eagle"));       // partial prefix
assert!(matches_seal_name(full, "swift"));             // single word
assert!(!matches_seal_name(full, "bold-wolf"));        // no match
```

## Properties

- **Deterministic**: Same BLAKE3 hash always produces the same name
- **Unique**: The 8-character hash suffix ensures uniqueness even if words collide
- **Memorable**: Four English words are easier to remember than hex strings
- **Searchable**: Prefix matching enables quick lookups

## Dual Hash Mapping

Each seal name maps to two hash values:
- **BLAKE3 hash**: Used for all internal operations
- **SHA1 hash** (optional): Populated only during GitHub/GitLab sync

The SHA1 mapping is never used internally — it exists solely for remote compatibility.

## Word Lists

Each category has 64 words, giving `64^4 = 16,777,216` possible word combinations. Combined with the 8-character hex suffix, collisions are effectively impossible.

## Design Decisions

- **Seeded PRNG from hash**: First 8 bytes of BLAKE3 hash used as seed for word selection
- **xorshift64 RNG**: Fast, deterministic, good distribution
- **64 words per list**: Balances memorability with uniqueness
- **Hash suffix last**: Makes the name readable while preserving uniqueness
