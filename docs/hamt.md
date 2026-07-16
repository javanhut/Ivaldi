# HAMT Directory Index (`hamt.rs`)

## Status

The HAMT is a production-quality, CAS-backed persistent directory index,
**integrated into repository storage** behind the format-2 gate:

- `FsStore::put_tree` stores any directory with more than
  `HAMT_DIR_THRESHOLD` (256) entries as a HAMT root — when the repository
  format allows it (`Cas::hamt_dirs`, true for format >= 2). The rule is
  deterministic on content, so the same directory always produces the same
  hash within a repository. The threshold is part of the on-disk format.
- `FsStore::load_tree` transparently flattens a HAMT root back to a
  `TreeNode`, so every reader (workspace, status, materialize, git export,
  sync, pick, review, TUI) handles both encodings without knowing.
- `diff_trees` takes a structural fast path when both sides of a directory
  are HAMT roots — cost proportional to the change, not the directory size.
- Encoding-aware walkers: the p2p transfer set ships HAMT interior nodes
  (`collect_objects_from_tree`), `verify --full` walks and validates them as
  a distinct object kind, and `rescue` recovers HAMT directories
  best-effort (a bad interior node loses only its own subtrie).
- Format gate: new repositories are stamped format 2 (`forge.rs`,
  `HAMT_DIRS_FORMAT`). Format-1 repositories remain fully supported and
  never receive HAMT objects; older binaries refuse format-2 repositories
  with a clear error. `FileCas` reads `.ivaldi/FORMAT` next to its objects
  directory at open, so every `FsStore` inherits the gate automatically.

A directory is represented by the BLAKE3 hash of its root node. Every node is
stored in the CAS under the hash of its canonical encoding, so:

- unchanged subtrees keep their hashes across seals and synchronized
  repositories (storage-level structural sharing, across processes);
- inserting, overwriting, or removing one entry rewrites only the
  ~log32(n) nodes on the path to it, where `fsmerkle` re-encodes the whole
  directory;
- old roots stay readable forever — persistence falls out of content
  addressing.

## Encoding

Canonical and versioned (`HAMT_VERSION = 1`). Every node starts
`'H' <version> <tag>`; leaves carry one directory entry with the same field
order as an fsmerkle tree entry (mode, name, kind, child hash); branches
carry a 32-bit occupancy bitmap and the child node hashes in ascending bit
order. Exactly one byte string exists per logical node: the parser re-encodes
after decoding and rejects any input that is not byte-identical, so
non-minimal varints cannot smuggle in a second encoding of the same node.

Slot indexing consumes the BLAKE3 digest of the entry name 5 bits per level,
MSB-first, crossing byte boundaries (max depth 52). The structure is
canonical: the same entry set always produces the same root hash regardless
of the insert/remove order that built it — removals collapse lone surviving
leaves back up the trie.

## Validation

Nodes arrive verbatim from untrusted peers, so `load` re-hashes every node's
bytes (the CAS `get` does not verify), bounds traversal depth against hostile
branch chains, rejects empty branches below the root, and full walks verify
each leaf sits on the slot path its name's digest dictates. Name and mode
rules are shared with `fsmerkle` (`validate_name` / `validate_mode`).
`fuzz/fuzz_targets/parse_hamt_node.rs` asserts arbitrary bytes never panic
the parser.

## Tests and benchmarks

- Unit tests in `src/hamt.rs` include byte-exact golden encodings — changing
  the format breaks them and requires a `HAMT_VERSION` bump.
- `tests/hamt_props.rs`: BTreeMap-mirror random-operation tests,
  shuffled-order determinism (insert and remove), encode/decode round-trips,
  and a corruption matrix covering every malformed-node case.
- `tests/hamt_bench.rs` compares HAMT vs `fsmerkle` at 1K/10K/100K/1M
  entries (construction, lookup, single-entry update, diff, stored bytes and
  object counts):

  ```
  cargo test --release --test hamt_bench -- --ignored --nocapture     # 1K-100K
  cargo test --release --test hamt_bench bench_1m -- --ignored --nocapture
  ```

  Measured (Apple Silicon, release, 100K entries): single-entry modify
  0.015 ms writing ~3.4 KB across 5 nodes, vs 10.4 ms writing 5.2 MB for
  fsmerkle's full re-encode; 100 successive updates write ~315 KB vs ~508 MB.
  fsmerkle remains faster at one-shot bulk build (one large object vs many
  small ones) and that is the trade-off integration must weigh.

## Remaining opportunities

The integration is correctness-first; two optimizations are deliberately
deferred:

- The seal path still rebuilds directory indexes from the full entry list
  (`build_tree_from_hash_map` → `put_root`); unchanged nodes dedup in the
  CAS so only changed-path nodes hit disk, but the CPU cost is O(n) per
  seal. Incremental `HamtStore::insert`/`remove` against the parent root
  would make it O(changed).
- `load_tree` flattening of a large HAMT reads every node; hot read paths
  that only need one entry could use `HamtStore::get` directly.

Note for mixed-format fleets: a directory's hash depends on its encoding,
so the same content hashes differently in a format-1 and a format-2
repository when it exceeds the threshold. Objects transfer verbatim over
sync (no re-encode), so mixed histories interoperate; only independently
re-imported content diverges.
