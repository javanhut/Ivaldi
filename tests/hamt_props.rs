//! Property-style tests for the CAS-backed HAMT: a BTreeMap mirror under
//! random operation sequences, canonical-form determinism across shuffled
//! insert/remove orders, encode/decode round-trips, and a corruption matrix
//! proving every malformed node type is rejected with an error — never a
//! panic, never a loop.
//!
//! A deterministic splitmix64 generator drives the cases (same pattern as
//! tests/roundtrip.rs) — reproducible with no dependency.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use ivaldi::cas::{Cas, CasError, MemoryCas};
use ivaldi::fsmerkle::{Entry, MODE_DIR, MODE_EXEC, MODE_FILE, MODE_SYMLINK, NodeKind};
use ivaldi::hamt::{HamtNode, HamtStore, parse_node};
use ivaldi::hash::B3Hash;

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
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
    fn hash(&mut self) -> B3Hash {
        let mut b = [0u8; 32];
        for x in b.iter_mut() {
            *x = self.next_u64() as u8;
        }
        B3Hash::from_bytes(b)
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i as u64 + 1) as usize);
        }
    }
}

/// A valid entry name: non-empty, not "." / "..", no path separator.
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

fn random_entry(rng: &mut Rng, name: String) -> Entry {
    let (mode, kind) = match rng.below(5) {
        0 => (MODE_DIR, NodeKind::Tree),
        1 => (MODE_EXEC, NodeKind::Blob),
        2 => (MODE_SYMLINK, NodeKind::Blob),
        _ => (MODE_FILE, NodeKind::Blob),
    };
    Entry {
        name,
        mode,
        kind,
        hash: rng.hash(),
    }
}

/// `n` entries with unique names.
fn unique_entries(rng: &mut Rng, n: usize) -> Vec<Entry> {
    let mut names = std::collections::HashSet::new();
    let mut out = Vec::new();
    while out.len() < n {
        let name = random_name(rng);
        if names.insert(name.clone()) {
            out.push(random_entry(rng, name));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// BTreeMap mirror
// ---------------------------------------------------------------------------

#[test]
fn random_ops_mirror_btreemap() {
    for seed in [1u64, 2, 3, 4] {
        let mut rng = Rng::new(seed);
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);

        // Small name pool so inserts, overwrites, and removes hit the same
        // keys often enough to exercise every structural transition.
        let pool: Vec<String> = unique_entries(&mut rng, 80)
            .into_iter()
            .map(|e| e.name)
            .collect();

        let mut root = store.put_root(Vec::new()).unwrap();
        let mut mirror: BTreeMap<String, Entry> = BTreeMap::new();

        for _ in 0..500 {
            let name = pool[rng.below(pool.len() as u64) as usize].clone();
            match rng.below(10) {
                0..=5 => {
                    let e = random_entry(&mut rng, name.clone());
                    root = store.insert(root, e.clone()).unwrap();
                    mirror.insert(name.clone(), e);
                }
                6..=8 => {
                    root = store.remove(root, &name).unwrap();
                    mirror.remove(&name);
                }
                _ => {}
            }
            assert_eq!(
                store.get(root, &name).unwrap().as_ref(),
                mirror.get(&name),
                "seed {}",
                seed
            );
        }

        let expected: Vec<Entry> = mirror.values().cloned().collect();
        assert_eq!(store.entries(root).unwrap(), expected, "seed {}", seed);

        // The incrementally-reached root must equal a direct bulk build of
        // the surviving set — canonical form across the whole op sequence.
        let direct = store.put_root(expected).unwrap();
        assert_eq!(root, direct, "seed {}", seed);
    }
}

// ---------------------------------------------------------------------------
// Canonical form / determinism
// ---------------------------------------------------------------------------

#[test]
fn insertion_order_is_irrelevant() {
    for seed in [10u64, 11, 12] {
        let mut rng = Rng::new(seed);
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let mut entries = unique_entries(&mut rng, 64);

        let bulk = store.put_root(entries.clone()).unwrap();
        for _ in 0..4 {
            rng.shuffle(&mut entries);
            let mut root = store.put_root(Vec::new()).unwrap();
            for e in &entries {
                root = store.insert(root, e.clone()).unwrap();
            }
            assert_eq!(root, bulk, "seed {}", seed);
        }
    }
}

#[test]
fn removal_order_is_irrelevant() {
    for seed in [20u64, 21, 22] {
        let mut rng = Rng::new(seed);
        let cas = MemoryCas::new();
        let store = HamtStore::new(&cas);
        let entries = unique_entries(&mut rng, 80);
        let (keep, drop) = entries.split_at(50);

        let full = store.put_root(entries.clone()).unwrap();
        let direct_subset = store.put_root(keep.to_vec()).unwrap();

        let mut victims: Vec<&Entry> = drop.iter().collect();
        for _ in 0..4 {
            rng.shuffle(&mut victims);
            let mut root = full;
            for v in &victims {
                root = store.remove(root, &v.name).unwrap();
            }
            assert_eq!(root, direct_subset, "seed {}", seed);
        }
    }
}

// ---------------------------------------------------------------------------
// Encode/decode round-trip
// ---------------------------------------------------------------------------

#[test]
fn node_roundtrip_randomized() {
    let mut rng = Rng::new(42);
    for _ in 0..200 {
        let node = if rng.below(2) == 0 {
            let name = random_name(&mut rng);
            HamtNode::Leaf(random_entry(&mut rng, name))
        } else {
            let bitmap = (rng.next_u64() as u32).max(1);
            let children = (0..bitmap.count_ones()).map(|_| rng.hash()).collect();
            HamtNode::Branch { bitmap, children }
        };
        let bytes = node.canonical_bytes().unwrap();
        assert_eq!(parse_node(&bytes).unwrap(), node);
    }
}

// ---------------------------------------------------------------------------
// Corruption matrix
// ---------------------------------------------------------------------------

/// CAS that serves attacker-controlled bytes for chosen hashes. Needed
/// because MemoryCas::put verifies content against hash, so honest puts
/// cannot inject corruption.
struct TamperCas {
    inner: MemoryCas,
    overrides: Mutex<HashMap<B3Hash, Vec<u8>>>,
}

impl TamperCas {
    fn new() -> Self {
        Self {
            inner: MemoryCas::new(),
            overrides: Mutex::new(HashMap::new()),
        }
    }
    fn plant(&self, hash: B3Hash, data: Vec<u8>) {
        self.overrides.lock().unwrap().insert(hash, data);
    }
}

impl Cas for TamperCas {
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError> {
        self.inner.put(hash, data)
    }
    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError> {
        if let Some(data) = self.overrides.lock().unwrap().get(&hash) {
            return Ok(data.clone());
        }
        self.inner.get(hash)
    }
    fn has(&self, hash: B3Hash) -> Result<bool, CasError> {
        if self.overrides.lock().unwrap().contains_key(&hash) {
            return Ok(true);
        }
        self.inner.has(hash)
    }
}

/// Hand-encode a leaf without canonical_bytes' validation, so invalid
/// names/modes/kinds can be crafted.
fn raw_leaf(mode: u64, name: &[u8], kind: u8, hash: &[u8; 32]) -> Vec<u8> {
    let mut buf = vec![b'H', 0x01, 0x01];
    write_uvarint(&mut buf, mode);
    write_uvarint(&mut buf, name.len() as u64);
    buf.extend_from_slice(name);
    buf.push(kind);
    buf.extend_from_slice(hash);
    buf
}

fn raw_branch(bitmap: u64, children: &[[u8; 32]]) -> Vec<u8> {
    let mut buf = vec![b'H', 0x01, 0x02];
    write_uvarint(&mut buf, bitmap);
    for c in children {
        buf.extend_from_slice(c);
    }
    buf
}

fn write_uvarint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// The 5-bit slot index math, mirrored from hamt.rs so tests can compute
/// where a leaf legitimately belongs (and plant it elsewhere).
fn slot_at_level(hash: &B3Hash, level: u32) -> usize {
    let start = level as usize * 5;
    let b = hash.as_bytes();
    let hi = b[start / 8] as u16;
    let lo = *b.get(start / 8 + 1).unwrap_or(&0) as u16;
    (((hi << 8 | lo) >> (11 - start % 8)) & 0x1F) as usize
}

#[test]
fn parse_rejects_every_malformed_node() {
    let zeros = [0u8; 32];
    let good_leaf = raw_leaf(MODE_FILE as u64, b"a", 1, &zeros);
    assert!(parse_node(&good_leaf).is_ok());

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("magic only", vec![b'H']),
        (
            "bad magic",
            raw_leaf(MODE_FILE as u64, b"a", 1, &zeros)[..]
                .iter()
                .enumerate()
                .map(|(i, &b)| if i == 0 { b'X' } else { b })
                .collect(),
        ),
        ("bad version", {
            let mut v = good_leaf.clone();
            v[1] = 2;
            v
        }),
        ("bad tag", vec![b'H', 0x01, 0x03, 0x00]),
        ("truncated leaf name", {
            let mut v = raw_leaf(MODE_FILE as u64, b"abcdef", 1, &zeros);
            v.truncate(8);
            v
        }),
        ("truncated leaf hash", {
            let mut v = good_leaf.clone();
            v.truncate(v.len() - 1);
            v
        }),
        ("trailing bytes", {
            let mut v = good_leaf.clone();
            v.push(0);
            v
        }),
        ("empty name", raw_leaf(MODE_FILE as u64, b"", 1, &zeros)),
        ("dot name", raw_leaf(MODE_FILE as u64, b".", 1, &zeros)),
        ("dotdot name", raw_leaf(MODE_FILE as u64, b"..", 1, &zeros)),
        (
            "slash in name",
            raw_leaf(MODE_FILE as u64, b"a/b", 1, &zeros),
        ),
        (
            "non-utf8 name",
            raw_leaf(MODE_FILE as u64, &[0xFF, 0xFE], 1, &zeros),
        ),
        ("bad kind byte", raw_leaf(MODE_FILE as u64, b"a", 3, &zeros)),
        (
            "mode/kind mismatch: dir mode on blob",
            raw_leaf(MODE_DIR as u64, b"a", 1, &zeros),
        ),
        (
            "mode/kind mismatch: file mode on tree",
            raw_leaf(MODE_FILE as u64, b"a", 2, &zeros),
        ),
        ("mode overflows u32", raw_leaf(1u64 << 33, b"a", 1, &zeros)),
        ("bitmap overflows u32", raw_branch(1u64 << 33, &[])),
        ("branch missing child", raw_branch(0b11, &[zeros])),
        ("branch extra child", raw_branch(0b1, &[zeros, zeros])),
        ("non-minimal mode varint", {
            // 0o100644 re-encoded with a redundant continuation byte: decodes
            // to the same value but is not the canonical byte string.
            let mut v = vec![b'H', 0x01, 0x01, 0xA4, 0x83, 0x82, 0x00];
            write_uvarint(&mut v, 1);
            v.push(b'a');
            v.push(1);
            v.extend_from_slice(&zeros);
            v
        }),
    ];

    for (label, bytes) in cases {
        assert!(
            parse_node(&bytes).is_err(),
            "case {:?} must be rejected",
            label
        );
    }
}

#[test]
fn load_rejects_bytes_not_matching_hash() {
    let cas = TamperCas::new();
    let store = HamtStore::new(&cas);
    // Serve a valid-looking leaf under a hash it does not hash to.
    let lie = B3Hash::digest(b"some other object");
    cas.plant(lie, raw_leaf(MODE_FILE as u64, b"a", 1, &[0u8; 32]));
    assert!(store.get(lie, "a").is_err());
    assert!(store.entries(lie).is_err());
}

#[test]
fn load_rejects_empty_branch_below_root() {
    let cas = MemoryCas::new();
    let store = HamtStore::new(&cas);
    // Craft an empty branch as the child of a root branch — both are
    // well-formed CAS objects, only the position is illegal.
    let empty = raw_branch(0, &[]);
    let empty_hash = B3Hash::digest(&empty);
    cas.put(empty_hash, &empty).unwrap();
    let root = raw_branch(0b1, &[*empty_hash.as_bytes()]);
    let root_hash = B3Hash::digest(&root);
    cas.put(root_hash, &root).unwrap();

    assert!(store.entries(root_hash).is_err());
}

#[test]
fn walk_rejects_leaf_under_wrong_slot() {
    let cas = MemoryCas::new();
    let store = HamtStore::new(&cas);
    let leaf = raw_leaf(MODE_FILE as u64, b"victim", 1, &[7u8; 32]);
    let leaf_hash = B3Hash::digest(&leaf);
    cas.put(leaf_hash, &leaf).unwrap();

    // Park the leaf under a slot its name's digest does not map to.
    let right = slot_at_level(&B3Hash::digest(b"victim"), 0);
    let wrong = (right + 1) % 32;
    let root = raw_branch(1u64 << wrong, &[*leaf_hash.as_bytes()]);
    let root_hash = B3Hash::digest(&root);
    cas.put(root_hash, &root).unwrap();

    assert!(store.entries(root_hash).is_err());
    // Lookup by name simply misses (the slot bit for the real path is unset).
    assert_eq!(store.get(root_hash, "victim").unwrap(), None);
}

#[test]
fn traversal_rejects_overdeep_chains() {
    // An attacker can craft arbitrarily deep chains of well-formed
    // single-child branches (every node honestly hashed). Traversal must
    // stop with an error, not recurse without bound.
    let cas = MemoryCas::new();
    let store = HamtStore::new(&cas);

    let leaf = raw_leaf(MODE_FILE as u64, b"deep", 1, &[0u8; 32]);
    let mut hash = B3Hash::digest(&leaf);
    cas.put(hash, &leaf).unwrap();
    // Lay each branch's single bit along the digest path of "deep" so a
    // lookup is forced to descend rather than bail on an unset slot bit
    // (levels past 51 never load — the depth check fires first).
    let digest = B3Hash::digest(b"deep");
    for level in (0..60u32).rev() {
        let slot = slot_at_level(&digest, level.min(51));
        let branch = raw_branch(1u64 << slot, &[*hash.as_bytes()]);
        hash = B3Hash::digest(&branch);
        cas.put(hash, &branch).unwrap();
    }

    assert!(store.entries(hash).is_err());
    assert!(store.get(hash, "deep").is_err());
    assert!(store.diff(hash, B3Hash::digest(b"x")).is_err());
}
