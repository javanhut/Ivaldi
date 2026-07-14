# MMR Module (`mmr.rs`)

Merkle Mountain Range accumulator for Ivaldi VCS.

## Overview

The MMR is an append-only data structure that tracks commit history. It provides:
- **Tamper-evident history**: Changing any past seal changes the MMR root
- **Efficient root computation**: O(log n)
- **Inclusion proofs**: Prove any commit exists with O(log n) hashes
- **Append-only semantics**: History is never modified, only extended

## Durable checkpoints

Every seal transaction persists both `mmr.size` and `mmr.root` alongside the
new leaf, timeline head, and seal mappings. Repository open performs the
following checks before making history available:

1. Leaf indices must be contiguous from zero with no gaps.
2. The stored size must equal the number of leaves.
3. Every leaf must parse successfully.
4. Rebuilding the MMR from those leaves must reproduce the stored root.

Any mismatch is a repository integrity error. Normal store APIs also reject an
attempt to overwrite an existing leaf index.

Repositories created before root checkpoints are migrated on their first open:
Ivaldi validates their index sequence and size, rebuilds the MMR, and records
the resulting root. Later opens enforce that checkpoint.

### Trust boundary

The checkpoint detects accidental corruption and out-of-band modification of
the leaf table. Because the root and leaves currently live in the same redb
database, it does not protect against an attacker who can deliberately rewrite
both. Protection against that threat requires a root anchored outside the
mutable repository, such as a signed identity checkpoint, a trusted remote
root, or a published transparency log.

Inclusion proofs are implemented at the data-structure level but are not yet
exchanged by synchronization transports. Until roots are authenticated across
machines, they should not be described as remote history authentication.

## Hashing Rules

- **Leaf hash**: `BLAKE3(0x00 || LeafHash)` — the `0x00` prefix prevents collision attacks
- **Internal hash**: `BLAKE3(0x01 || LeftChildHash || RightChildHash)`

The `0x00`/`0x01` prefixes ensure leaf nodes can never be confused with internal nodes.

## Structure

The MMR maintains a stack of **peaks** — roots of complete binary subtrees. When two peaks of the same height exist, they automatically merge:

```
After 7 leaves:    peaks = [height-2, height-1, height-0]

        H₂
       /  \             H₁
      /    \           / \
     /      \         /   \
    H₁      H₁      L₅   L₆    L₇
   / \     / \
  L₁  L₂ L₃  L₄
```

**Key property**: For `n` leaves, the number of peaks equals `popcount(n)` (number of set bits).

## Usage

```rust
use ivaldi::mmr::Mmr;
use ivaldi::leaf::Leaf;

let mut mmr = Mmr::new();

// Append leaves
let (idx, root) = mmr.append_leaf(leaf);

// Query
let leaf = mmr.get_leaf(0).unwrap();
let size = mmr.size();
let root = mmr.root();

// Inclusion proofs
let proof = mmr.proof(0).unwrap();
let valid = mmr.verify(leaf_hash, &proof, root);
```

## Inclusion Proofs

A proof consists of:
- **Siblings**: Hashes needed to climb from the leaf to a peak
- **Peaks**: All current peak hashes

Verification:
1. Start with the leaf hash (with `0x00` prefix)
2. Combine with each sibling (left/right determined by leaf position)
3. Result should match one of the peaks
4. Peaks should produce the claimed root

```rust
let proof = mmr.proof(leaf_idx).unwrap();
assert!(mmr.verify(leaf_hash, &proof, mmr.root()));
```

## Performance

| Operation | Complexity |
|-----------|-----------|
| Append    | O(log n)  |
| Root      | O(log n)  |
| Proof     | O(n) replay (to be optimized with persistent storage) |
| Verify    | O(log n)  |

## Design Decisions

- **Peak stack approach**: Simpler and more correct than position-based MMR formulas
- **Replay-based proofs**: Current implementation replays construction to collect siblings. Will be optimized with persistent storage in Stage 1.6
- **No mutation through repository APIs**: Existing leaf indices are rejected;
  peaks only grow or merge
- **Deterministic root**: Same sequence of leaves always produces the same root
