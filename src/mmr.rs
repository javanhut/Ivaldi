//! Merkle Mountain Range (MMR) accumulator for Ivaldi VCS.
//!
//! An append-only structure that tracks commit history with:
//! - Efficient root computation: O(log n)
//! - Inclusion proofs: O(log n)
//! - Tamper-evident history when checked against a trusted root
//!
//! Hashing rules:
//! - Leaf hash:     `BLAKE3(0x00 || LeafHash)`
//! - Internal hash: `BLAKE3(0x01 || LeftChildHash || RightChildHash)`

use crate::hash::B3Hash;
use crate::leaf::Leaf;

const LEAF_PREFIX: u8 = 0x00;
const INTERNAL_PREFIX: u8 = 0x01;

/// Inclusion proof for a leaf in the MMR.
#[derive(Debug, Clone)]
pub struct Proof {
    pub leaf_index: u64,
    pub siblings: Vec<B3Hash>,
    pub peaks: Vec<B3Hash>,
}

/// A peak in the MMR: a root of a complete binary tree of a given height.
#[derive(Debug, Clone)]
struct Peak {
    height: u32,
    hash: B3Hash,
}

/// In-memory MMR accumulator.
///
/// Internally tracks leaves and a stack of peaks. When two peaks of the
/// same height exist, they merge into one peak of height+1.
#[derive(Clone)]
pub struct Mmr {
    leaves: Vec<Leaf>,
    /// Stack of peaks, each representing a complete binary subtree.
    peaks: Vec<Peak>,
}

impl Mmr {
    pub fn new() -> Self {
        Self {
            leaves: Vec::new(),
            peaks: Vec::new(),
        }
    }

    /// Append a leaf and return (leaf_index, new_root).
    pub fn append_leaf(&mut self, leaf: Leaf) -> (u64, B3Hash) {
        let leaf_idx = self.leaves.len() as u64;
        let leaf_hash = compute_leaf_hash(leaf.hash());
        self.leaves.push(leaf);

        // Start as a height-0 peak
        let mut new_peak = Peak {
            height: 0,
            hash: leaf_hash,
        };

        // Merge with existing peaks of the same height
        while let Some(last) = self.peaks.last() {
            if last.height == new_peak.height {
                let left = self.peaks.pop().unwrap();
                new_peak = Peak {
                    height: left.height + 1,
                    hash: compute_internal_hash(left.hash, new_peak.hash),
                };
            } else {
                break;
            }
        }

        self.peaks.push(new_peak);

        let root = self.compute_root();
        (leaf_idx, root)
    }

    /// Current MMR root hash.
    pub fn root(&self) -> B3Hash {
        self.compute_root()
    }

    /// Get a leaf by index.
    pub fn get_leaf(&self, idx: u64) -> Option<&Leaf> {
        self.leaves.get(idx as usize)
    }

    /// Number of leaves in the MMR.
    pub fn size(&self) -> u64 {
        self.leaves.len() as u64
    }

    /// Generate an inclusion proof for a leaf.
    pub fn proof(&self, idx: u64) -> Option<Proof> {
        if idx >= self.size() {
            return None;
        }

        let peak_hashes: Vec<B3Hash> = self.peaks.iter().map(|p| p.hash).collect();

        // To build siblings, we replay the MMR construction up to this leaf
        // and collect the sibling at each merge step.
        let siblings = self.collect_siblings(idx as usize);

        Some(Proof {
            leaf_index: idx,
            siblings,
            peaks: peak_hashes,
        })
    }

    /// Verify an inclusion proof against a root hash.
    pub fn verify(&self, leaf_hash: B3Hash, proof: &Proof, root: B3Hash) -> bool {
        let mut current = compute_leaf_hash(leaf_hash);

        for sibling in &proof.siblings {
            // We need to know if current is left or right child.
            // During proof collection we always store the sibling such that
            // current is left when it was added first (lower index).
            // We'll use an approach: track the position within each subtree.
            // For simplicity, we use the leaf index bits to determine left/right.
            // Bit k of leaf_index tells us if the leaf is in the left (0) or right (1)
            // subtree at level k.
            current = compute_internal_hash(current, *sibling);
        }

        // Hmm, this naive approach doesn't handle left/right correctly.
        // Let's use a different verification strategy.
        // Instead, verify by checking that current matches a peak, then peaks make root.
        // But we need correct left/right ordering.
        // Let me redo this properly.

        // Re-verify: rebuild from leaf
        let mut current = compute_leaf_hash(leaf_hash);
        let leaf_idx = proof.leaf_index as usize;
        let mut pos_in_tree = leaf_idx;

        for sibling in &proof.siblings {
            if pos_in_tree.is_multiple_of(2) {
                // current is left child
                current = compute_internal_hash(current, *sibling);
            } else {
                // current is right child
                current = compute_internal_hash(*sibling, current);
            }
            pos_in_tree /= 2;
        }

        // current should now be one of the peaks
        if !proof.peaks.contains(&current) {
            return false;
        }

        compute_root_from_peaks(&proof.peaks) == root
    }

    /// Like [`Self::verify`], but additionally binds the claimed leaf index:
    /// the replayed path must land on the specific peak whose leaf range
    /// covers `proof.leaf_index` (derived from `size`), not on just any peak.
    /// Prefer this when the index comes from an untrusted source — a
    /// siblingless proof (the odd tail leaf) otherwise verifies under any
    /// claimed index.
    pub fn verify_for_size(
        &self,
        leaf_hash: B3Hash,
        proof: &Proof,
        root: B3Hash,
        size: u64,
    ) -> bool {
        // The index must exist, and an MMR of `size` leaves has exactly
        // popcount(size) peaks.
        if proof.leaf_index >= size || proof.peaks.len() != size.count_ones() as usize {
            return false;
        }

        let mut current = compute_leaf_hash(leaf_hash);
        let mut pos_in_tree = proof.leaf_index;
        for sibling in &proof.siblings {
            current = if pos_in_tree.is_multiple_of(2) {
                compute_internal_hash(current, *sibling)
            } else {
                compute_internal_hash(*sibling, current)
            };
            pos_in_tree /= 2;
        }

        match peak_covering(size, proof.leaf_index) {
            Some(k) if proof.peaks.get(k) == Some(&current) => {}
            _ => return false,
        }

        compute_root_from_peaks(&proof.peaks) == root
    }

    /// Collect sibling hashes for an inclusion proof by replaying construction.
    fn collect_siblings(&self, target_leaf: usize) -> Vec<B3Hash> {
        // Replay the MMR and track which subtree the target leaf falls into.
        // At each merge, if the target is in the left subtree, the sibling is the right peak,
        // and vice versa.

        let mut siblings = Vec::new();
        let mut temp_peaks: Vec<(u32, B3Hash, usize, usize)> = Vec::new();
        // Each peak tracks: (height, hash, start_leaf_idx, end_leaf_idx_exclusive)

        for i in 0..self.leaves.len() {
            let leaf_hash = compute_leaf_hash(self.leaves[i].hash());
            let mut new_peak = (0u32, leaf_hash, i, i + 1);

            while let Some(last) = temp_peaks.last() {
                if last.0 == new_peak.0 {
                    let left = temp_peaks.pop().unwrap();
                    let merged_hash = compute_internal_hash(left.1, new_peak.1);

                    // If target is in left subtree, sibling is right (new_peak)
                    // If target is in right subtree, sibling is left
                    if target_leaf >= left.2 && target_leaf < left.3 {
                        // Target in left subtree — sibling is right
                        siblings.push(new_peak.1);
                    } else if target_leaf >= new_peak.2 && target_leaf < new_peak.3 {
                        // Target in right subtree — sibling is left
                        siblings.push(left.1);
                    }

                    new_peak = (left.0 + 1, merged_hash, left.2, new_peak.3);
                } else {
                    break;
                }
            }

            temp_peaks.push(new_peak);
        }

        siblings
    }

    fn compute_root(&self) -> B3Hash {
        if self.peaks.is_empty() {
            return B3Hash::ZERO;
        }
        let hashes: Vec<B3Hash> = self.peaks.iter().map(|p| p.hash).collect();
        compute_root_from_peaks(&hashes)
    }
}

impl Default for Mmr {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Hash computation
// ---------------------------------------------------------------------------

fn compute_leaf_hash(leaf_hash: B3Hash) -> B3Hash {
    let mut data = [0u8; 33];
    data[0] = LEAF_PREFIX;
    data[1..].copy_from_slice(leaf_hash.as_bytes());
    B3Hash::digest(&data)
}

fn compute_internal_hash(left: B3Hash, right: B3Hash) -> B3Hash {
    let mut data = [0u8; 65];
    data[0] = INTERNAL_PREFIX;
    data[1..33].copy_from_slice(left.as_bytes());
    data[33..65].copy_from_slice(right.as_bytes());
    B3Hash::digest(&data)
}

fn compute_root_from_peaks(peaks: &[B3Hash]) -> B3Hash {
    match peaks.len() {
        0 => B3Hash::ZERO,
        1 => peaks[0],
        _ => {
            let mut result = peaks[peaks.len() - 1];
            for i in (0..peaks.len() - 1).rev() {
                result = compute_internal_hash(peaks[i], result);
            }
            result
        }
    }
}

/// Index of the peak whose leaf range covers `leaf_idx`, for an MMR of
/// `size` leaves. Peaks are stored left-to-right by leaf range (decreasing
/// height), and the ranges follow the binary decomposition of `size`: each
/// set bit contributes one peak spanning 2^bit leaves.
fn peak_covering(size: u64, leaf_idx: u64) -> Option<usize> {
    if leaf_idx >= size {
        return None;
    }
    let mut pos = 0u64;
    let mut peak_k = 0usize;
    for h in (0..64).rev() {
        let span = 1u64 << h;
        if size & span != 0 {
            if leaf_idx < pos + span {
                return Some(peak_k);
            }
            pos += span;
            peak_k += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_leaf(msg: &str) -> Leaf {
        Leaf::new(B3Hash::digest(msg.as_bytes()), "main", "Author", 1000, msg)
    }

    #[test]
    fn verify_for_size_binds_index_to_its_peak() {
        for n in 1..=8u64 {
            let mut mmr = Mmr::new();
            let mut hashes = Vec::new();
            for i in 0..n {
                let leaf = make_leaf(&format!("c{i}"));
                hashes.push(leaf.hash());
                mmr.append_leaf(leaf);
            }
            for idx in 0..n {
                let proof = mmr.proof(idx).unwrap();
                assert!(
                    mmr.verify_for_size(hashes[idx as usize], &proof, mmr.root(), n),
                    "honest proof rejected (n={n}, idx={idx})"
                );
                // Claiming any other index must fail — including for the odd
                // tail leaf, whose proof has no siblings to bind it.
                for wrong in 0..n {
                    if wrong == idx {
                        continue;
                    }
                    let mut p = proof.clone();
                    p.leaf_index = wrong;
                    assert!(
                        !mmr.verify_for_size(hashes[idx as usize], &p, mmr.root(), n),
                        "index slide accepted (n={n}, {idx}->{wrong})"
                    );
                }
            }
        }
    }

    #[test]
    fn empty_mmr() {
        let mmr = Mmr::new();
        assert_eq!(mmr.size(), 0);
        assert_eq!(mmr.root(), B3Hash::ZERO);
    }

    #[test]
    fn single_leaf() {
        let mut mmr = Mmr::new();
        let (idx, root) = mmr.append_leaf(make_leaf("first"));
        assert_eq!(idx, 0);
        assert_ne!(root, B3Hash::ZERO);
        assert_eq!(mmr.size(), 1);
        assert_eq!(mmr.peaks.len(), 1);
    }

    #[test]
    fn two_leaves_merge() {
        let mut mmr = Mmr::new();
        mmr.append_leaf(make_leaf("first"));
        let (idx, _) = mmr.append_leaf(make_leaf("second"));
        assert_eq!(idx, 1);
        assert_eq!(mmr.size(), 2);
        // Two height-0 peaks merge into one height-1 peak
        assert_eq!(mmr.peaks.len(), 1);
        assert_eq!(mmr.peaks[0].height, 1);
    }

    #[test]
    fn three_leaves() {
        let mut mmr = Mmr::new();
        mmr.append_leaf(make_leaf("1"));
        mmr.append_leaf(make_leaf("2"));
        mmr.append_leaf(make_leaf("3"));
        assert_eq!(mmr.size(), 3);
        // 2 merge → height-1, then height-1 + height-0 = 2 peaks
        assert_eq!(mmr.peaks.len(), 2);
    }

    #[test]
    fn four_leaves_single_peak() {
        let mut mmr = Mmr::new();
        for i in 0..4 {
            mmr.append_leaf(make_leaf(&format!("leaf {}", i)));
        }
        assert_eq!(mmr.size(), 4);
        // 4 = 2^2, perfect binary tree → 1 peak
        assert_eq!(mmr.peaks.len(), 1);
        assert_eq!(mmr.peaks[0].height, 2);
    }

    #[test]
    fn peak_counts() {
        // n leaves → number of peaks = popcount(n)
        for n in 1..=32u64 {
            let mut mmr = Mmr::new();
            for i in 0..n {
                mmr.append_leaf(make_leaf(&format!("leaf {}", i)));
            }
            assert_eq!(
                mmr.peaks.len(),
                n.count_ones() as usize,
                "n={} should have {} peaks",
                n,
                n.count_ones()
            );
        }
    }

    #[test]
    fn root_deterministic() {
        let mut mmr1 = Mmr::new();
        let mut mmr2 = Mmr::new();

        for i in 0..5 {
            let msg = format!("leaf {}", i);
            mmr1.append_leaf(make_leaf(&msg));
            mmr2.append_leaf(make_leaf(&msg));
        }

        assert_eq!(mmr1.root(), mmr2.root());
    }

    #[test]
    fn root_changes_on_append() {
        let mut mmr = Mmr::new();
        let (_, root1) = mmr.append_leaf(make_leaf("first"));
        let (_, root2) = mmr.append_leaf(make_leaf("second"));
        assert_ne!(root1, root2);
    }

    #[test]
    fn get_leaf() {
        let mut mmr = Mmr::new();
        mmr.append_leaf(make_leaf("first"));
        mmr.append_leaf(make_leaf("second"));

        assert_eq!(mmr.get_leaf(0).unwrap().message, "first");
        assert_eq!(mmr.get_leaf(1).unwrap().message, "second");
        assert!(mmr.get_leaf(2).is_none());
    }

    #[test]
    fn proof_single_leaf() {
        let mut mmr = Mmr::new();
        let leaf = make_leaf("only");
        let leaf_hash = leaf.hash();
        mmr.append_leaf(leaf);

        let proof = mmr.proof(0).unwrap();
        assert!(proof.siblings.is_empty());
        assert!(mmr.verify(leaf_hash, &proof, mmr.root()));
    }

    #[test]
    fn proof_two_leaves() {
        let mut mmr = Mmr::new();
        let leaf0 = make_leaf("first");
        let leaf1 = make_leaf("second");
        let hash0 = leaf0.hash();
        let hash1 = leaf1.hash();

        mmr.append_leaf(leaf0);
        mmr.append_leaf(leaf1);

        let root = mmr.root();

        let proof0 = mmr.proof(0).unwrap();
        assert!(mmr.verify(hash0, &proof0, root));

        let proof1 = mmr.proof(1).unwrap();
        assert!(mmr.verify(hash1, &proof1, root));
    }

    #[test]
    fn proof_four_leaves() {
        let mut mmr = Mmr::new();
        let mut hashes = Vec::new();
        for i in 0..4 {
            let leaf = make_leaf(&format!("leaf {}", i));
            hashes.push(leaf.hash());
            mmr.append_leaf(leaf);
        }

        let root = mmr.root();
        for i in 0..4u64 {
            let proof = mmr.proof(i).unwrap();
            assert!(
                mmr.verify(hashes[i as usize], &proof, root),
                "proof failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn proof_invalid_hash_fails() {
        let mut mmr = Mmr::new();
        let leaf = make_leaf("real");
        mmr.append_leaf(leaf);

        let proof = mmr.proof(0).unwrap();
        let fake_hash = B3Hash::digest(b"fake");
        assert!(!mmr.verify(fake_hash, &proof, mmr.root()));
    }

    #[test]
    fn proof_out_of_range() {
        let mmr = Mmr::new();
        assert!(mmr.proof(0).is_none());
    }

    #[test]
    fn many_leaves() {
        let mut mmr = Mmr::new();
        let mut hashes = Vec::new();
        for i in 0..100 {
            let leaf = make_leaf(&format!("leaf {}", i));
            hashes.push(leaf.hash());
            mmr.append_leaf(leaf);
        }

        let root = mmr.root();
        for i in 0..100u64 {
            let proof = mmr.proof(i).unwrap();
            assert!(
                mmr.verify(hashes[i as usize], &proof, root),
                "proof failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn append_only_root_integrity() {
        let mut mmr = Mmr::new();
        let mut roots = Vec::new();

        for i in 0..10 {
            let (_, root) = mmr.append_leaf(make_leaf(&format!("leaf {}", i)));
            roots.push(root);
        }

        // Each root should be unique
        for i in 0..roots.len() {
            for j in i + 1..roots.len() {
                assert_ne!(roots[i], roots[j]);
            }
        }
    }
}
