//! Round-trip property tests for the persisted on-disk encodings.
//!
//! The property under test is `decode(encode(x)) == x` for every persisted
//! type. These are the formats `ivaldi verify` and `ivaldi rescue` decode by
//! hand, so a round-trip bug here is silent data corruption, not a crash.
//!
//! No property-test framework is used (the crate has none): a deterministic
//! splitmix64 generator drives thousands of structured cases per format, plus
//! explicit boundary cases. Deterministic seeds mean a failure always
//! reproduces from the printed input.

use std::collections::{BTreeMap, BTreeSet};

use ivaldi::fsmerkle::{
    BlobNode, Entry, MODE_DIR, MODE_EXEC, MODE_FILE, MODE_SYMLINK, NodeKind, TreeNode, parse_blob,
    parse_tree,
};
use ivaldi::hash::B3Hash;
use ivaldi::leaf::{Leaf, NO_PARENT, parse_leaf};

/// Deterministic splitmix64 PRNG. Not for cryptography — just reproducible,
/// well-distributed test inputs with no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform-ish value in `0..n` (bias negligible for the small n used here).
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }
    fn hash(&mut self) -> B3Hash {
        let mut b = [0u8; 32];
        for x in b.iter_mut() {
            *x = self.next_u64() as u8;
        }
        B3Hash::from_bytes(b)
    }
}

/// Arbitrary string: newlines, nulls, unicode, quotes, path chars — anything a
/// length-prefixed field must survive.
fn random_string(rng: &mut Rng, max_len: usize) -> String {
    const POOL: &[char] = &[
        'a', 'B', 'z', '0', '9', ' ', '\t', '\n', '\0', 'é', 'ü', '日', '本', '🦀', '"', '\\', '/',
        '.', ':', '=', ',', '-', '_', '{', '}',
    ];
    let len = rng.below(max_len as u64 + 1) as usize;
    (0..len)
        .map(|_| POOL[rng.below(POOL.len() as u64) as usize])
        .collect()
}

/// A valid tree entry name: non-empty, not "." / "..", no path separator.
fn random_name(rng: &mut Rng) -> String {
    const POOL: &[char] = &['a', 'B', 'z', '0', '9', ' ', 'é', '日', '🦀', '-', '_', '.'];
    loop {
        let len = 1 + rng.below(12) as usize;
        let s: String = (0..len)
            .map(|_| POOL[rng.below(POOL.len() as u64) as usize])
            .collect();
        if s != "." && s != ".." && !s.contains('/') {
            return s;
        }
    }
}

fn gen_leaf(rng: &mut Rng) -> Leaf {
    let n_merge = rng.below(5) as usize;
    let merge_idxs = (0..n_merge).map(|_| rng.next_u64()).collect();

    let n_meta = rng.below(5) as usize;
    let mut meta = BTreeMap::new();
    for _ in 0..n_meta {
        meta.insert(random_string(rng, 12), random_string(rng, 20));
    }

    Leaf {
        tree_root: rng.hash(),
        timeline_id: random_string(rng, 20),
        // NO_PARENT is u64::MAX — exercise the sentinel and ordinary indices.
        prev_idx: if rng.below(4) == 0 {
            NO_PARENT
        } else {
            rng.next_u64()
        },
        merge_idxs,
        author: random_string(rng, 30),
        time_unix: rng.next_u64() as i64, // full i64 range, including negatives
        message: random_string(rng, 60),
        meta,
    }
}

fn gen_tree(rng: &mut Rng) -> TreeNode {
    let k = rng.below(8) as usize;
    let mut names = BTreeSet::new();
    let mut entries = Vec::new();
    while entries.len() < k {
        let name = random_name(rng);
        if !names.insert(name.clone()) {
            continue; // entry names must be unique
        }
        let (kind, mode) = if rng.below(2) == 0 {
            (NodeKind::Tree, MODE_DIR)
        } else {
            let mode = match rng.below(3) {
                0 => MODE_FILE,
                1 => MODE_EXEC,
                _ => MODE_SYMLINK,
            };
            (NodeKind::Blob, mode)
        };
        entries.push(Entry {
            name,
            mode,
            kind,
            hash: rng.hash(),
        });
    }
    TreeNode::new(entries) // canonical order (sorted)
}

#[test]
fn leaf_roundtrips() {
    let mut rng = Rng::new(0x1EAF_5EED);
    for _ in 0..5000 {
        let leaf = gen_leaf(&mut rng);
        let parsed = parse_leaf(&leaf.canonical_bytes())
            .unwrap_or_else(|e| panic!("parse failed for {leaf:?}: {e}"));
        assert_eq!(parsed, leaf, "round-trip mismatch");
    }

    // Boundary cases the generator is unlikely to hit exactly.
    let edges = [
        Leaf {
            tree_root: B3Hash::ZERO,
            timeline_id: String::new(),
            prev_idx: NO_PARENT,
            merge_idxs: vec![],
            author: String::new(),
            time_unix: 0,
            message: String::new(),
            meta: BTreeMap::new(),
        },
        Leaf {
            tree_root: B3Hash::from_bytes([0xFF; 32]),
            timeline_id: "🦀\n\0".into(),
            prev_idx: u64::MAX - 1,
            merge_idxs: vec![0, u64::MAX, 42],
            author: "a\tb".into(),
            time_unix: i64::MIN,
            message: "multi\nline\0msg".into(),
            meta: BTreeMap::from([(String::new(), String::new()), ("k".into(), "v".into())]),
        },
        Leaf {
            tree_root: B3Hash::ZERO,
            timeline_id: "main".into(),
            prev_idx: 0,
            merge_idxs: vec![],
            author: "x".into(),
            time_unix: i64::MAX,
            message: "y".into(),
            meta: BTreeMap::new(),
        },
    ];
    for leaf in edges {
        assert_eq!(parse_leaf(&leaf.canonical_bytes()).unwrap(), leaf);
    }
}

#[test]
fn tree_roundtrips() {
    let mut rng = Rng::new(0x79EE_5EED);
    for _ in 0..5000 {
        let tree = gen_tree(&mut rng);
        let bytes = tree.canonical_bytes().expect("encode");
        let parsed = parse_tree(&bytes).expect("decode");
        assert_eq!(parsed.entries, tree.entries, "tree round-trip mismatch");
    }

    // Empty tree and one-of-each-kind.
    let empty = TreeNode::new(vec![]);
    assert_eq!(
        parse_tree(&empty.canonical_bytes().unwrap())
            .unwrap()
            .entries,
        empty.entries
    );

    let mixed = TreeNode::new(vec![
        Entry {
            name: "dir".into(),
            mode: MODE_DIR,
            kind: NodeKind::Tree,
            hash: B3Hash::ZERO,
        },
        Entry {
            name: "exe".into(),
            mode: MODE_EXEC,
            kind: NodeKind::Blob,
            hash: B3Hash::from_bytes([1; 32]),
        },
        Entry {
            name: "file 日".into(),
            mode: MODE_FILE,
            kind: NodeKind::Blob,
            hash: B3Hash::from_bytes([2; 32]),
        },
        Entry {
            name: "link".into(),
            mode: MODE_SYMLINK,
            kind: NodeKind::Blob,
            hash: B3Hash::from_bytes([3; 32]),
        },
    ]);
    assert_eq!(
        parse_tree(&mixed.canonical_bytes().unwrap())
            .unwrap()
            .entries,
        mixed.entries
    );
}

/// The bounded parsers must return a typed error on any malformed input, never
/// panic, OOM, or spin. This is the oracle for the parser-hardening work: the
/// old hand-rolled decoders panicked on truncated buffers and could allocate
/// from an attacker-controlled length prefix.
#[test]
fn malformed_inputs_never_panic() {
    let mut rng = Rng::new(0xBAD_5EED);

    // Every truncation of a valid encoding must decode-or-error, never panic.
    for _ in 0..500 {
        let leaf = gen_leaf(&mut rng);
        let bytes = leaf.canonical_bytes();
        for k in 0..bytes.len() {
            let _ = parse_leaf(&bytes[..k]);
        }
        let tree = gen_tree(&mut rng);
        let tb = tree.canonical_bytes().unwrap();
        for k in 0..tb.len() {
            let _ = parse_tree(&tb[..k]);
        }
    }

    // Pure garbage of assorted lengths.
    for _ in 0..10_000 {
        let len = rng.below(64) as usize;
        let g = rng.bytes(len);
        let _ = parse_leaf(&g);
        let _ = parse_tree(&g);
        let _ = parse_blob(&g);
    }

    // Resource attacks the old code was vulnerable to: a length prefix claiming
    // a gigantic count with no data behind it must error, not pre-allocate.
    let mut tree_bomb = Vec::new();
    ivaldi::filechunk::write_uvarint(&mut tree_bomb, u64::MAX); // claims u64::MAX entries
    assert!(parse_tree(&tree_bomb).is_err());

    let mut leaf_bomb = vec![0x01]; // version 1
    leaf_bomb.extend_from_slice(&[0u8; 32]); // tree root
    ivaldi::filechunk::write_uvarint(&mut leaf_bomb, u64::MAX); // huge timeline-ID length
    assert!(parse_leaf(&leaf_bomb).is_err());
}

#[test]
fn blob_content_roundtrips() {
    let mut rng = Rng::new(0xB10B_5EED);
    for _ in 0..5000 {
        let len = rng.below(400) as usize;
        let content = rng.bytes(len);
        let (_, parsed) = parse_blob(&BlobNode::canonical_bytes(&content)).expect("decode");
        assert_eq!(parsed, content, "blob round-trip mismatch");
    }

    // Empty and all-zero contents.
    for content in [vec![], vec![0u8; 100], vec![0xFFu8; 100]] {
        let (_, parsed) = parse_blob(&BlobNode::canonical_bytes(&content)).unwrap();
        assert_eq!(parsed, content);
    }
}
