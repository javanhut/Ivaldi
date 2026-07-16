# HAMT Directory Index (`hamt.rs`)

## Status

The HAMT is a production-quality, CAS-backed persistent directory index. It
is fully implemented and tested, but **not yet wired into** the repository,
workspace, synchronization, or Git interoperability paths — Ivaldi still
stores directories as `fsmerkle` tree nodes. Integration is a separate,
repo-format-gated step (see below).

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

## Remaining work: integration

Switching repository directory storage to the HAMT requires:

1. A repository-format gate: bump `CURRENT_FORMAT` (see `forge.rs`) or wire
   the reserved `features` key in `.ivaldi/FORMAT`, with a forward migration
   per `VERSIONING.md`.
2. Rewiring workspace/status/diff paths and Git export (reusing
   `git_tree_entry_order` in `git_export.rs` for canonical Git ordering —
   `entries()` already returns plain name order).
3. `FileCas::flush()` before any durable record references new HAMT nodes,
   same as every other object write.

Until then, `fsmerkle` remains Ivaldi's sole directory storage
implementation.
