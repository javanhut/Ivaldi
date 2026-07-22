# Reader Module (`reader.rs`)

`ByteReader`: a bounded, `Result`-based cursor for decoding untrusted bytes.

## Overview

Every on-disk object and network message is untrusted input. `ByteReader` is
the single place decoders do bounds checking: a truncated or malformed buffer
yields a typed `ReadError` instead of a slice-index panic, a silent partial
decode, or an out-of-memory abort. Parsers built on it never index a raw slice.

## API

```rust
let mut r = ByteReader::new(bytes);
let n     = r.uvarint()?;          // LEB128, bounded to 64 bits (max 10 bytes)
let t     = r.varint()?;           // signed (zigzag)
let hash  = r.array::<32>()?;      // fixed-size, bounds-checked
let byte  = r.u8()?;
let s      = r.string("field")?;   // uvarint length prefix + UTF-8 bytes
let raw   = r.take(n)?;            // n bytes, overflow-checked
r.finish()?;                       // reject trailing data
```

## Guarantees

- **No panics.** `take` uses `checked_add` and a length check; it can never
  index out of bounds or overflow the offset.
- **Bounded varints.** A varint cannot exceed 64 bits (10 bytes), so a hostile
  stream cannot spin or overflow.
- **No pre-allocation from length prefixes.** A decoder loops on a count and
  errors when the input runs out, so a bogus count cannot exhaust memory.

## Users

`leaf::parse_leaf`, `fsmerkle::parse_tree`, and the `filechunk` chunk-node
readers decode through `ByteReader`. The `pack` decoder uses the same
principles with checked arithmetic for its random-access index (see
[pack.md](pack.md)).

Round-trip and malformed-input tests live in `tests/roundtrip.rs`: valid values
survive `decode(encode(x)) == x`, and thousands of truncated and garbage
buffers are asserted to error rather than panic.
