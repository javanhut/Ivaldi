//! HAMT vs fsmerkle benchmarks — the integration gate from docs/hamt.md.
//!
//! All tests are #[ignore]d so `cargo test` never pays for them. Run with:
//!
//!     cargo test --release --test hamt_bench -- --ignored --nocapture
//!
//! The 1M case is heavyweight and only runs when named explicitly:
//!
//!     cargo test --release --test hamt_bench bench_1m -- --ignored --nocapture
//!
//! Both structures run against MemoryCas so the numbers measure the data
//! structure, not FileCas's per-object fsync.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ivaldi::cas::{Cas, CasError, MemoryCas};
use ivaldi::fsmerkle::{diff_trees, Entry, FsStore, NodeKind, MODE_FILE};
use ivaldi::hamt::HamtStore;
use ivaldi::hash::B3Hash;

/// CAS wrapper that tallies traffic: object counts and bytes written, reads
/// served. This is how "stored bytes / CAS object count" is measured.
struct CountingCas {
    inner: MemoryCas,
    puts: AtomicU64,
    put_bytes: AtomicU64,
    gets: AtomicU64,
}

impl CountingCas {
    fn new() -> Self {
        Self {
            inner: MemoryCas::new(),
            puts: AtomicU64::new(0),
            put_bytes: AtomicU64::new(0),
            gets: AtomicU64::new(0),
        }
    }
    fn reset(&self) {
        self.puts.store(0, Ordering::Relaxed);
        self.put_bytes.store(0, Ordering::Relaxed);
        self.gets.store(0, Ordering::Relaxed);
    }
    fn puts(&self) -> u64 {
        self.puts.load(Ordering::Relaxed)
    }
    fn put_bytes(&self) -> u64 {
        self.put_bytes.load(Ordering::Relaxed)
    }
    fn gets(&self) -> u64 {
        self.gets.load(Ordering::Relaxed)
    }
}

impl Cas for CountingCas {
    fn put(&self, hash: B3Hash, data: &[u8]) -> Result<(), CasError> {
        self.puts.fetch_add(1, Ordering::Relaxed);
        self.put_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
        self.inner.put(hash, data)
    }
    fn get(&self, hash: B3Hash) -> Result<Vec<u8>, CasError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get(hash)
    }
    fn has(&self, hash: B3Hash) -> Result<bool, CasError> {
        self.inner.has(hash)
    }
}

fn entries(n: usize) -> Vec<Entry> {
    (0..n)
        .map(|i| Entry {
            name: format!("file_{:07}.rs", i),
            mode: MODE_FILE,
            kind: NodeKind::Blob,
            hash: B3Hash::digest(format!("content {}", i).as_bytes()),
        })
        .collect()
}

fn modified(mut e: Entry) -> Entry {
    e.hash = B3Hash::digest(b"edited content");
    e
}

struct Row {
    label: &'static str,
    hamt: String,
    fsm: String,
}

fn ms(t: Instant) -> String {
    format!("{:>10.3}ms", t.elapsed().as_secs_f64() * 1000.0)
}

fn run_size(n: usize) {
    let set = entries(n);
    let mut rows = Vec::new();

    // --- HAMT ---------------------------------------------------------
    let cas = CountingCas::new();
    let hamt = HamtStore::new(&cas);

    let t = Instant::now();
    let hamt_root = hamt.put_root(set.clone()).unwrap();
    let build_h = ms(t);
    let (h_objs, h_bytes) = (cas.puts(), cas.put_bytes());

    let target = &set[n / 2];
    let t = Instant::now();
    let found = hamt.get(hamt_root, &target.name).unwrap();
    let lookup_h = format!("{} ({} reads)", ms(t), cas.gets());
    assert_eq!(found.as_ref(), Some(target));

    cas.reset();
    let t = Instant::now();
    let hamt_edit = hamt.insert(hamt_root, modified(target.clone())).unwrap();
    let modify_h = format!("{} ({} objs, {} B)", ms(t), cas.puts(), cas.put_bytes());

    cas.reset();
    let t = Instant::now();
    let hamt_add = hamt
        .insert(hamt_edit, modified(Entry {
            name: "zz_new_file.rs".into(),
            ..target.clone()
        }))
        .unwrap();
    let add_h = format!("{} ({} objs)", ms(t), cas.puts());

    cas.reset();
    let t = Instant::now();
    hamt.remove(hamt_add, &target.name).unwrap();
    let remove_h = format!("{} ({} objs)", ms(t), cas.puts());

    let t = Instant::now();
    let changes = hamt.diff(hamt_root, hamt_edit).unwrap();
    let diff_h = ms(t);
    assert_eq!(changes.len(), 1);

    // 100 successive single-entry updates: write amplification over time.
    cas.reset();
    let mut r = hamt_root;
    let t = Instant::now();
    for e in set.iter().take(100) {
        r = hamt.insert(r, modified(e.clone())).unwrap();
    }
    let churn_h = format!("{} ({} objs, {} KB)", ms(t), cas.puts(), cas.put_bytes() / 1024);

    // --- fsmerkle -------------------------------------------------------
    // fsmerkle has no incremental update: every edit re-encodes the whole
    // directory via put_tree. That IS the honest comparison — it's what the
    // repository actually does today.
    let cas = CountingCas::new();
    let fsm = FsStore::new(&cas);

    let t = Instant::now();
    let fsm_root = fsm.put_tree(set.clone()).unwrap();
    let build_f = ms(t);
    let (f_objs, f_bytes) = (cas.puts(), cas.put_bytes());

    let t = Instant::now();
    let tree = fsm.load_tree(fsm_root).unwrap();
    let found = tree.find_entry(&target.name);
    let lookup_f = format!("{} ({} reads)", ms(t), cas.gets());
    assert_eq!(found, Some(target));

    cas.reset();
    let edit_set: Vec<Entry> = set
        .iter()
        .map(|e| if e.name == target.name { modified(e.clone()) } else { e.clone() })
        .collect();
    let t = Instant::now();
    let fsm_edit = fsm.put_tree(edit_set.clone()).unwrap();
    let modify_f = format!("{} ({} objs, {} B)", ms(t), cas.puts(), cas.put_bytes());

    cas.reset();
    let mut add_set = edit_set.clone();
    add_set.push(Entry {
        name: "zz_new_file.rs".into(),
        ..modified(target.clone())
    });
    let t = Instant::now();
    let fsm_add = fsm.put_tree(add_set).unwrap();
    let add_f = format!("{} ({} objs)", ms(t), cas.puts());

    cas.reset();
    let remove_set: Vec<Entry> = edit_set.iter().filter(|e| e.name != target.name).cloned().collect();
    let t = Instant::now();
    fsm.put_tree(remove_set).unwrap();
    let remove_f = format!("{} ({} objs)", ms(t), cas.puts());
    let _ = fsm_add;

    let t = Instant::now();
    let changes = diff_trees(fsm_root, fsm_edit, &fsm).unwrap();
    let diff_f = ms(t);
    assert_eq!(changes.len(), 1);

    cas.reset();
    let mut cur = set.clone();
    let t = Instant::now();
    for i in 0..100 {
        cur[i] = modified(cur[i].clone());
        fsm.put_tree(cur.clone()).unwrap();
    }
    let churn_f = format!("{} ({} objs, {} KB)", ms(t), cas.puts(), cas.put_bytes() / 1024);

    rows.push(Row { label: "build", hamt: format!("{} ({} objs, {} KB)", build_h, h_objs, h_bytes / 1024), fsm: format!("{} ({} objs, {} KB)", build_f, f_objs, f_bytes / 1024) });
    rows.push(Row { label: "lookup (cold)", hamt: lookup_h, fsm: lookup_f });
    rows.push(Row { label: "modify 1 entry", hamt: modify_h, fsm: modify_f });
    rows.push(Row { label: "add 1 entry", hamt: add_h, fsm: add_f });
    rows.push(Row { label: "remove 1 entry", hamt: remove_h, fsm: remove_f });
    rows.push(Row { label: "diff (1 change)", hamt: diff_h, fsm: diff_f });
    rows.push(Row { label: "100 updates", hamt: churn_h, fsm: churn_f });

    println!("\n=== {} entries ===", n);
    println!("{:<16} | {:<40} | {:<40}", "operation", "HAMT", "fsmerkle");
    println!("{}", "-".repeat(102));
    for row in rows {
        println!("{:<16} | {:<40} | {:<40}", row.label, row.hamt, row.fsm);
    }
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn bench_1k_10k_100k() {
    for n in [1_000, 10_000, 100_000] {
        run_size(n);
    }
}

#[test]
#[ignore = "benchmark: heavyweight, run by name with --release --ignored --nocapture"]
fn bench_1m() {
    run_size(1_000_000);
}
