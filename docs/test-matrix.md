# Correctness and Test-Evidence Matrix

Ivaldi's tests are executable evidence for the guarantees of its native VCS.
This document maps those guarantees to test layers so test counts are never
used as a substitute for examining what the suite proves.

Run every normal Rust target, integration test, feature, and deterministic
failpoint with:

```bash
cargo test --locked --all-targets --all-features
```

The two HAMT scale benchmarks are intentionally ignored by the normal suite;
their source files document the release-mode commands used to run them.

| Guarantee | Executable evidence |
|---|---|
| CAS identity and durability | File and memory CAS round trips, hash-mismatch rejection, idempotent and concurrent puts, shard durability tracking, and atomic-I/O cleanup tests |
| Append-only verifiable history | MMR roots, peaks, inclusion proofs, invalid-proof rejection, persisted checkpoints, parent validation, leaf-gap detection, and tampered-history rejection |
| Atomic metadata publication | `redb` batch validation, commit rollback tests, process-level failpoints, and reopen verification |
| Writer isolation | Two-process seal races, same-timeline creation races, lock contention, and lock release after process abort |
| Crash-safe local operations | Kill/reopen matrices for seal, gather, reseal, fuse, weld, undo, rewind, reverse, timeline removal and rename, and journaled timeline switching |
| Old-or-new visibility | Failpoints before, during, and after publication assert that reopen exposes complete old state, complete new state, or a documented recovery state—never silently accepted partial state |
| Idempotent recovery | Repeated crash, retry, resume, rollback, and reverse tests verify convergence without losing prior history or shelved work |
| Automatic per-timeline shelving | CLI process tests and switch-journal tests verify dirty work is shelved, isolated, restored, and preserved through interruption |
| Filesystem integrity | Canonical blob/tree encodings, structural sharing, additions, modifications, deletions, executable bits, symlinks, ignored files, dotfile controls, and materialization tests |
| HAMT directory correctness | Golden encodings, randomized `BTreeMap` mirroring, insertion/removal-order independence, malformed-node rejection, structural diff, format-1/format-2 lifecycle, rescue, and native-transfer traversal |
| Fusion semantics | Three-way additions, changes, deletions, clean and conflicted results, every resolution strategy, fast-forward detection, large merges, and crash-interrupted fusion |
| Native authenticated transfer | Real localhost fetch and push, Noise trust rejection, concurrent clients, protobuf round trips, parent-index remapping, received-tip verification, peer-namespace landing, and idempotent repeat transfer |
| Bounded large-object transfer | Chunked multi-megabyte blob transfer, contiguous assembly, declared-length and BLAKE3 verification, hostile sequence rejection, and truncated-stream cleanup |
| Import landing integrity | Empty and unrelated repositories, merge-parent preservation, local-index remapping, identical content at distinct indices, failed-landing authority, and idempotent retry |
| Fail-closed repository verification | Corrupt objects, malformed and dangling refs, missing reachable trees/blobs, invalid parents, unsafe persisted names, and mismatched checkpoints |
| Evidence-preserving recovery | Destroyed refs, corrupt stores, orphan sweeping, tampered-object quarantine, path-traversal rejection, nested timelines, shared subtrees, excessive depth, and HAMT recovery |
| Hostile-input resistance | Bounded readers, overflowing/truncated varints, forged sizes and entry counts, delta bombs and cycles, excessive nesting, malformed packs, invalid tree/HAMT nodes, and malformed protocol messages |
| Command-level behavior | Multi-process CLI smoke tests cover forge, status, timelines, automatic shelving, divergent fusion, error exit status, and malformed-state reporting without repository damage |

In addition to the deterministic suite, `fuzz/` contains targets for native
readers and encodings, HAMT nodes, and compatibility-boundary pack and delta
formats. CI runs formatting, Clippy with warnings denied, the all-features test
suite on Linux/macOS/Windows, the minimum supported Rust version, dependency
policy checks, and scheduled fuzzing.
