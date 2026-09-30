# Chunked Large Files (`filechunk.rs`)

## Status

Integrated into repository storage behind the format-3 gate:

- `FsStore::put_blob` stores any file larger than `CHUNKED_FILE_THRESHOLD`
  (4 MiB) as a chunk tree when the repository format allows it
  (`Cas::chunked_files`, true for format >= 3). Smaller files, and every file
  in a format-1 or format-2 repository, stay one whole `blob` object. The rule
  is deterministic on size and format, so the same content always gets the
  same hash within a repository.
- A directory entry of kind `Blob` names either encoding. `FsStore::load_blob`
  reads both, so diff, fuse, pick, review, inspect, Git export and sync need no
  changes. `FsStore::write_blob_to` streams a chunked file without holding it in
  memory.
- The workspace streams large files. `gather` stores chunks as it reads them,
  `status` hashes the working file into the chunked encoding without storing
  anything, and `materialize` (timeline switch, rewind, travel) writes chunked
  files straight from the store. It skips the write when the file on disk
  already hashes to the same tree. Peak memory is one chunk regardless of file
  size.
- Encoding-aware walkers: native transfer ships every chunk
  (`collect_objects_from_tree` → `FsStore::blob_objects`), `verify --full`
  walks and validates each chunked file, and `rescue` reassembles chunked files
  from raw objects.
- Format gate: new repositories are stamped format 3 (`forge.rs`,
  `CHUNKED_FILES_FORMAT`). `ivaldi migrate` upgrades format-2 repositories.
  Older binaries refuse format-3 repositories with a clear error.

## Encoding

Canonical and versioned (`CHUNK_VERSION = 1`). Every node starts
`'C' <version> <tag>`:

```
Leaf:     'C' 1 0 | chunk bytes                      (1..=CHUNK_SIZE bytes)
Interior: 'C' 1 1 | height u8 | uvarint(total_size) | uvarint(count) | child_hash[32] * count
Hash:     BLAKE3(canonical_bytes)
```

A whole blob starts with `blob `, so the first three bytes are enough to tell
the two file encodings apart (`FsStore::is_chunked_blob` reads only those). A
chunk node can never parse as a directory: read as a tree, it would carry mode
1, which directory validation rejects.

### Shape

The leaves are the file's consecutive 1 MiB chunks, all full except the last.
Each level groups the one below it into runs of `FANOUT` (64), left to right,
and the root is the first level with a single node:

```
6 MiB + 1 byte, 7 leaves:          65 leaves (65 MiB):

      root (h=1, 7 children)                root (h=2, 2 children)
   /  /  /  |  \  \  \                    /                    \
  L0 L1 L2 L3 L4 L5 L6            n (h=1, 64 children)   n' (h=1, 1 child)
                                   L0 … L63                     L64
```

So every leaf sits at the same depth, every node except the rightmost on its
level is full, and the root has at least two children and more than the
threshold in bytes. Readers enforce all of it, together with each node's hash,
height, and recorded size. That makes content → root hash a bijection: the
same bytes produce the same root in every repository, and a hostile peer
cannot introduce a second encoding of a file.

### Parameters

The chunk size, fanout and threshold are part of the on-disk format. Never
change them without a format bump.

- **1 MiB chunks.** `FileCas::put` fsyncs each object, so a 64 KiB chunk
  would mean ~16,000 fsyncs per GiB stored. 1 MiB keeps that to ~1,000 while
  an in-place edit or append still rewrites little.
- **Fanout 64.** Interior nodes add ~1.6% more objects than there are leaves,
  where a binary tree would double the object count. An edit rewrites one node
  per level (two levels up to 4 GiB).
- **4 MiB threshold.** Below it a file would have four chunks or fewer, and
  the extra objects buy little.
- **Fixed-size chunking.** In-place edits and appends dedup. An insertion
  shifts every later chunk, so those chunks are stored again.

## Migration from format 2

A large file sealed before migration stays one whole blob. Its hash differs
from the chunk tree of the same bytes. So that migrating is not a change:

- `status` and change capture compare a mismatching file against the whole
  blob's hash once, then cache the match (`Workspace::matches_unchunked`).
- Sealing keeps the parent's whole blob when the staged chunk tree holds the
  same bytes (`FsStore::same_content_as_unchunked`).

The first real edit stores the file chunked.

## API

```rust
use ivaldi::filechunk::{ChunkWriter, chunked_hash, put_chunked, read_all, read_to};

// Store (or, with `None`, only hash) a stream of bytes.
let mut w = ChunkWriter::new(Some(&cas));
w.write(&bytes[..])?;
let (root, size) = w.finish()?;          // needs > CHUNK_SIZE bytes

// Read back, validated.
let content = read_all(&cas, root)?;
read_to(&cas, root, &mut |chunk| { /* stream */ Ok(()) })?;
```

Most callers go through `FsStore` (`put_blob`, `blob_hash`, `load_blob`,
`write_blob_to`, `blob_objects`), which picks the encoding.

## Variable-Length Integer Encoding

The module also provides the LEB128 helpers used by Ivaldi's other canonical
encodings:

```rust
use ivaldi::filechunk::{write_uvarint, read_uvarint, write_varint, read_varint};

let mut buf = Vec::new();
write_uvarint(&mut buf, 300);
let (value, bytes_read) = read_uvarint(&buf);
assert_eq!(value, 300);
```

## Tests

- Unit tests in `src/filechunk.rs` run the writer and reader at tiny parameters
  (4-byte chunks, fanout 3). They cover hand-built canonical shapes, every size
  across level boundaries, streaming in arbitrary pieces, dedup and
  edit-cost object counts, and rejection of every non-canonical or hostile
  shape.
- `fuzz/fuzz_targets/parse_chunk_node.rs` asserts arbitrary bytes never panic
  the parser.
- `tests/chunked_files.rs` runs the real binary through seal, status, a
  one-byte edit, timeline switching, verify, rescue, and a format 2 → 3
  migration.

## Not covered

- LFS: Git LFS pointer files pass through the Git bridge as ordinary small
  files. Their content is not fetched from an LFS server.
- Content-defined chunking (dedup across insertions) is not implemented.
