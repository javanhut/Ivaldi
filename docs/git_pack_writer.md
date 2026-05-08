# Git Pack Writer (`git_pack_writer.rs`)

Encodes a set of git objects into a v2 packfile, as accepted by
`git-receive-pack`. Used by SSH push (see [`docs/ssh.md`](ssh.md)) to
serialize the output of [`git_export.rs`](git_export.md).

This is **not** Ivaldi's native pack format (that lives in `src/pack.rs`
and is BLAKE3-keyed). This module is git-compatible only.

## Format

```text
"PACK"               4 bytes  magic
0x0000_0002          4 bytes  version (BE)
0xNN_NN_NN_NN        4 bytes  object count (BE)
{   per object:
    <var-len header>            type:3 + size + continuation bits
    <zlib-deflated body>
}
<SHA-1 of all preceding bytes>   20 bytes
```

### Object header

First byte:

```
bit 7   = continuation
bits 6-4 = type (1=commit, 2=tree, 3=blob, 4=tag, 6=ofs-delta, 7=ref-delta)
bits 3-0 = low 4 bits of size
```

Subsequent bytes (if continuation):

```
bit 7   = continuation
bits 6-0 = next 7 bits of size
```

We only emit base types (1/2/3/4); the writer never produces deltas.

### SHA-1 trailer

The trailer covers everything from the magic through the last byte of
the last zlib stream. The hasher is updated as bytes are appended to
the buffer.

## What we don't do (yet)

- **Deltas**. Every object is shipped as a full base. Wire size is
  larger than what `git push` would send, but the receiving side
  doesn't care — `git-receive-pack` indexes the pack normally.
- **Pack idx files** (`.idx`). Not needed for push; the server builds
  its own index after receiving.
- **Multi-pass / streaming**. The whole pack is built in memory and
  shipped at once. For repos in the millions of commits this would need
  rework; for typical use it's fine.

## Round-trip verified by Ivaldi's own parser

Confidence that the format is right comes from feeding our output back
through `crate::git_remote::parse_packfile` (the parser used during
`download` to import packs from GitHub). If parser → writer → parser
preserves the object set, the wire format is sound.

```rust
let pack = git_pack_writer::write_pack(&objects)?;
let parsed = git_remote::parse_packfile(&pack)?;
// parsed[&sha1].data == original body
```

That is exactly what the unit test
`round_trip_through_parser` asserts in `src/git_pack_writer.rs`.

## Tests

4 unit tests:

- `round_trip_through_parser` — write a single blob, parse it back,
  confirm body bytes match.
- `header_encoding_round_trips_for_small_size` — single-byte header
  for `size=5`, type=blob.
- `header_encoding_for_larger_size_uses_continuation` — multi-byte
  header for `size=200`, type=tree.
- `empty_pack_has_just_header_plus_trailer` — 12 + 20 = 32 bytes for an
  object-free pack.

## Files

- `src/git_pack_writer.rs` — `write_pack(objects)` plus
  `encode_object_header`.
- Reuses:
  - `crate::git_export::GitObject` — the input type.
  - `crate::git_remote::GitObjectKind` — the type-code enum.
  - `crate::git_remote::git_object_id` — SHA-1 of `<type> <len>\0<body>`,
    used by `git_export` (not the writer itself).
- Crates: `flate2` (zlib), `sha1` (RustCrypto, hardware-accelerated).
