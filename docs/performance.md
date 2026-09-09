# Workspace and upload performance

Status and gather share a disposable, checksummed `.ivaldi/workspace-cache-v1`.
On Unix, an entry records device, inode, size, mode, mtime and ctime. Hashes
are reused only after a stable read outside a conservative timestamp race
window. Missing or damaged caches fall back to content reads. Platforms
without these metadata checks currently always read content. Removing the
cache is safe; it is not authoritative storage or proof of CAS presence.

Status hashes uncached files in 64 KiB buffers. Gather checks CAS presence
before constructing canonical blob bytes, and hashes/stores new content in
batches of at most eight files with a 16 MiB input-size budget. Larger files
run alone; files growing during the operation can exceed the size estimate.
Gather reuses one directory walk for candidates, deletions and dotfile
reporting, and explicit paths limit parent-tree traversal. Staging remains
atomically published; CAS synchronization has not been weakened.

Repository open reads history through one database snapshot. It still
validates every leaf and rebuilds the MMR to check the saved root. This
remains linear in history size: the optimization removes per-leaf database
transactions, not corruption checks.

Git upload resolves export identities only for required roots and parents
and reuses already validated in-memory history for its advertised-tip
index. Remote ancestry and snapshot reachability planning still require
traversal; this is not a persistent reachability index.

Native protocol v3 negotiates missing leaves and object hashes before
transferring payloads. Both peers must upgrade. Repeat pushes send no
duplicate object bodies or leaf bundles; inventories still scale with
reachable history and object counts. Receiver duplicate-leaf resolution
uses one in-memory hash index rather than repeated full-history searches.

Run the opt-in filesystem benchmark with:

```sh
cargo test --release --locked --test workspace_bench -- --ignored --nocapture
```

It creates 5,000 files totaling approximately 80 MiB and times initial
gather, status without and with the file cache, unchanged gather, one-file
status and explicit gathering. The operating-system page cache is not
cleared. Numbers depend on storage and filesystem; use this alongside the
full correctness suite, not as a durability or storage-throughput test.

Filesystem watchers, constant-time history opening, durable batch/pack
ingestion and path-local seal-tree updates are separate future work.
