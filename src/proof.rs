//! MMR inclusion receipts: portable proof that a seal belongs to a
//! repository's append-only history.
//!
//! A receipt pins the MMR root at generation time and carries the O(log n)
//! inclusion proof for one leaf. Anyone who trusts that root — obtained from
//! `ivaldi prove`, shared out of band, or pinned in CI — can verify the
//! receipt without fetching the history. Git has no equivalent: there is no
//! compact proof that a commit sits inside a given history.

use crate::hash::B3Hash;
use crate::mmr::{Mmr, Proof};
use crate::repo::Repo;

/// Receipt format version, embedded as `"version"` in the JSON.
pub const RECEIPT_VERSION: u32 = 1;

/// A portable MMR inclusion receipt for one seal.
///
/// Hashes are hex strings: receipts are a presentation-layer artifact, so the
/// core types stay free of serde derives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InclusionReceipt {
    pub version: u32,
    /// Registered seal name, when the repository has one for the leaf.
    pub seal: Option<String>,
    pub leaf_index: u64,
    /// BLAKE3 hash of the leaf (hex).
    pub leaf_hash: String,
    /// MMR root the proof was generated against (hex).
    pub root: String,
    /// Number of leaves in the MMR at generation time.
    pub size: u64,
    /// Sibling hashes on the path from the leaf to its peak (hex).
    pub siblings: Vec<String>,
    /// MMR peak hashes at generation time (hex).
    pub peaks: Vec<String>,
}

/// Errors building, parsing, or checking a receipt.
#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("repo error: {0}")]
    Repo(#[from] crate::repo::RepoError),
    #[error("seal index {0} is out of range")]
    OutOfRange(u64),
    #[error("invalid receipt: {0}")]
    InvalidReceipt(String),
}

impl InclusionReceipt {
    /// Build a receipt for the leaf at `idx` in `repo`, pinning the current
    /// MMR root.
    pub fn build(repo: &Repo, idx: u64) -> Result<Self, ProofError> {
        let leaf = repo.get_leaf(idx)?.ok_or(ProofError::OutOfRange(idx))?;
        let proof = repo
            .inclusion_proof(idx)
            .ok_or(ProofError::OutOfRange(idx))?;
        let leaf_hash = leaf.hash();
        Ok(Self {
            version: RECEIPT_VERSION,
            seal: repo.get_seal_name(leaf_hash).ok().flatten(),
            leaf_index: proof.leaf_index,
            leaf_hash: leaf_hash.to_hex(),
            root: repo.root().to_hex(),
            size: repo.commit_count(),
            siblings: proof.siblings.iter().map(|h| h.to_hex()).collect(),
            peaks: proof.peaks.iter().map(|h| h.to_hex()).collect(),
        })
    }

    /// Serialize as pretty JSON (what `ivaldi prove` prints).
    pub fn to_json(&self) -> Result<String, ProofError> {
        serde_json::to_string_pretty(self).map_err(|e| ProofError::InvalidReceipt(e.to_string()))
    }

    /// Parse a receipt from JSON.
    pub fn from_json(text: &str) -> Result<Self, ProofError> {
        serde_json::from_str(text)
            .map_err(|e| ProofError::InvalidReceipt(format!("not a receipt JSON: {e}")))
    }
}

/// Result of checking a receipt.
#[derive(Debug)]
pub struct ReceiptCheck {
    /// The receipt is structurally consistent and its inclusion proof
    /// verifies against the receipt's own root.
    pub proof_valid: bool,
    /// Whether the receipt's root equals the trusted pinned root — `None`
    /// when no pin was supplied.
    pub root_matches_pin: Option<bool>,
}

/// Verify a receipt's inclusion proof against its embedded root, and
/// optionally compare that root against a trusted one pinned out of band.
pub fn verify_receipt(
    receipt: &InclusionReceipt,
    pinned_root: Option<B3Hash>,
) -> Result<ReceiptCheck, ProofError> {
    let parse_hash = |label: &str, s: &str| -> Result<B3Hash, ProofError> {
        B3Hash::from_hex(s).ok_or_else(|| {
            ProofError::InvalidReceipt(format!("{label} is not a valid BLAKE3 hex hash: {s:?}"))
        })
    };
    let leaf_hash = parse_hash("leaf_hash", &receipt.leaf_hash)?;
    let root = parse_hash("root", &receipt.root)?;
    let siblings = receipt
        .siblings
        .iter()
        .map(|s| parse_hash("sibling", s))
        .collect::<Result<Vec<_>, _>>()?;
    let peaks = receipt
        .peaks
        .iter()
        .map(|s| parse_hash("peak", s))
        .collect::<Result<Vec<_>, _>>()?;

    let proof = Proof {
        leaf_index: receipt.leaf_index,
        siblings,
        peaks,
    };
    // Mmr::verify_for_size is stateless. Beyond replaying the proof, it binds
    // the claimed (untrusted) index to the peak whose range covers it and
    // checks the peak count against the claimed size.
    let proof_valid = Mmr::new().verify_for_size(leaf_hash, &proof, root, receipt.size);

    Ok(ReceiptCheck {
        proof_valid,
        root_matches_pin: pinned_root.map(|pin| pin == root),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::FileCas;
    use crate::fsmerkle::FsStore;

    /// Forge a repo with `n` seals on main.
    fn repo_with_leaves(n: u8) -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = Repo::open(dir.path()).unwrap();
        let cas = FileCas::new(dir.path().join(".ivaldi/objects")).unwrap();
        let store = FsStore::new(&cas);
        for i in 0..n {
            let (blob, _) = store.put_blob(&[i]).unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree = store
                .put_tree(vec![Entry {
                    name: "f".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob,
                }])
                .unwrap();
            repo.commit(tree, "t <t@x>", &format!("c{i}")).unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn receipt_roundtrip_verifies_against_its_root() {
        let (_dir, repo) = repo_with_leaves(5);
        let receipt = InclusionReceipt::build(&repo, 2).unwrap();
        assert_eq!(receipt.version, RECEIPT_VERSION);
        assert_eq!(receipt.leaf_index, 2);
        assert_eq!(receipt.size, 5);
        assert!(!receipt.siblings.is_empty());
        // Every committed seal gets a generated name, and it lands in the receipt.
        assert!(receipt.seal.is_some());

        let parsed = InclusionReceipt::from_json(&receipt.to_json().unwrap()).unwrap();
        let check = verify_receipt(&parsed, None).unwrap();
        assert!(check.proof_valid);
        assert_eq!(check.root_matches_pin, None);

        // A correct pin confirms; a wrong one rejects.
        let pin = B3Hash::from_hex(&receipt.root).unwrap();
        assert_eq!(
            verify_receipt(&parsed, Some(pin)).unwrap().root_matches_pin,
            Some(true)
        );
        assert_eq!(
            verify_receipt(&parsed, Some(B3Hash::ZERO))
                .unwrap()
                .root_matches_pin,
            Some(false)
        );
    }

    #[test]
    fn tampered_receipt_fails_verification() {
        let (_dir, repo) = repo_with_leaves(5);
        let receipt = InclusionReceipt::build(&repo, 3).unwrap();

        let mut bad = receipt.clone();
        bad.siblings[0] = B3Hash::ZERO.to_hex();
        assert!(!verify_receipt(&bad, None).unwrap().proof_valid);

        let mut bad = receipt.clone();
        bad.root = B3Hash::ZERO.to_hex();
        assert!(!verify_receipt(&bad, None).unwrap().proof_valid);

        let mut bad = receipt;
        bad.leaf_hash = B3Hash::ZERO.to_hex();
        assert!(!verify_receipt(&bad, None).unwrap().proof_valid);
    }

    #[test]
    fn structurally_inconsistent_receipt_is_rejected() {
        let (_dir, repo) = repo_with_leaves(5);
        let receipt = InclusionReceipt::build(&repo, 2).unwrap();

        // Index outside the claimed history size.
        let mut bad = receipt.clone();
        bad.leaf_index = bad.size;
        assert!(!verify_receipt(&bad, None).unwrap().proof_valid);

        // An MMR of `size` leaves has popcount(size) peaks — no more.
        let mut bad = receipt;
        bad.peaks.push(B3Hash::ZERO.to_hex());
        assert!(!verify_receipt(&bad, None).unwrap().proof_valid);

        // Degenerate case: a 1-leaf (siblingless) receipt must name leaf 0 —
        // the cryptographic path alone cannot bind the index there.
        let (_dir2, repo2) = repo_with_leaves(1);
        let mut one = InclusionReceipt::build(&repo2, 0).unwrap();
        one.leaf_index = 1;
        assert!(!verify_receipt(&one, None).unwrap().proof_valid);

        // Tail-leaf slide: the odd leaf's proof has no siblings either, so
        // its claimed index must still land on its own peak.
        let (_dir3, repo3) = repo_with_leaves(3);
        let mut tail = InclusionReceipt::build(&repo3, 2).unwrap();
        assert!(tail.siblings.is_empty());
        tail.leaf_index = 1;
        assert!(!verify_receipt(&tail, None).unwrap().proof_valid);
    }

    #[test]
    fn build_rejects_out_of_range_index() {
        let (_dir, repo) = repo_with_leaves(2);
        assert!(matches!(
            InclusionReceipt::build(&repo, 99),
            Err(ProofError::OutOfRange(99))
        ));
    }

    #[test]
    fn from_json_rejects_garbage_and_bad_hex() {
        assert!(InclusionReceipt::from_json("not json").is_err());

        let (_dir, repo) = repo_with_leaves(2);
        let mut receipt = InclusionReceipt::build(&repo, 0).unwrap();
        receipt.root = "zz".into();
        assert!(matches!(
            verify_receipt(&receipt, None),
            Err(ProofError::InvalidReceipt(_))
        ));
    }
}
