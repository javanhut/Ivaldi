# Pack Module (`pack.rs`)

Combines many small CAS objects into larger pack files for efficient storage
and transfer, with optional delta compression.

## Format

```
<magic "IVPK"><version:u8><entry_count:u64 LE><index_entries><data>
```

- **v1** index entry: `hash:32 | offset:u64 | size:u64`
- **v2** index entry: `hash:32 | offset:u64 | size:u64 | entry_type:u8`
  (type 0 = full object, 1 = delta; a delta's data is `base_hash:32 || delta`)

Deltas use `COPY(offset, len)` / `INSERT(len, data)` instructions: COPY
references bytes in the resolved base object, INSERT supplies new bytes.

## Usage

`PackWriter::write` / `write_delta` produce packs; `PackReader::get_object` and
`extract_to_cas` read them. Packs are used by garbage collection (packing loose
objects) and by network transfer.

## Hardening

Pack data arrives from disk and from network peers, so decoding treats it as
hostile. All of the following are bounded, returning `PackError::Corrupt`
rather than panicking, allocating unboundedly, or overflowing the stack:

- **Index/offset arithmetic** is overflow-checked (`checked_add`/`checked_mul`)
  and every slice goes through a bounds-checked helper — no out-of-bounds panic
  from a hostile offset or size.
- **Entry count** is validated against the buffer size before use, so a bogus
  count cannot pre-allocate or outrun the data.
- **Delta chains** have a depth limit, so a cyclic or absurdly deep chain
  cannot overflow the stack.
- **Delta output** is capped, so a tiny delta cannot expand into a
  memory-exhausting "delta bomb".

Adversarial tests cover each vector (huge entry count, delta cycle, delta bomb,
malformed delta ops). See also [reader.md](reader.md) for the shared
bounded-decoding approach.
