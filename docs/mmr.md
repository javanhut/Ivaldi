# MMR Module (`mmr.rs`)

Merkle Mountain Range accumulator for Ivaldi VCS.

## Overview

The MMR is an append-only data structure that tracks commit history. It provides:
- **Tamper-proof history**: Changing any past commit changes all subsequent hashes
- **Efficient root computation**: O(log n)
- **Inclusion proofs**: Prove any commit exists with O(log n) hashes
- **Append-only semantics**: History is never modified, only extended

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
- **No mutation**: Leaves are append-only, peaks only grow or merge
- **Deterministic root**: Same sequence of leaves always produces the same root
