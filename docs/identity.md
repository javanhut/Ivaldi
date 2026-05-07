# Identity (`identity.rs`)

Long-lived peer identity used by the `ivaldi://` peer-to-peer transport.
A single X25519 keypair per Ivaldi installation, stored at
`~/.ivaldi/identity` and auto-generated on first need.

## On-disk format

`~/.ivaldi/identity` is a small JSON document (mode `0600` on Unix) that
round-trips a 32-byte X25519 keypair:

```json
{
  "version": 1,
  "secret_hex": "<64 hex chars>",
  "public_hex": "<64 hex chars>"
}
```

Override the path for tests / multi-account workflows by passing an
explicit path to `Identity::load_or_create`. The file is created with
`fs::write` to a `.tmp` then atomically renamed.

## Why X25519, not ed25519?

The `snow` Noise framework (used by `src/p2p.rs` for the encrypted
transport) keys its handshake on X25519. We could carry an ed25519 key
and convert at runtime, but a single curve keeps the on-disk schema
simpler. The curve is irrelevant for end-user concerns — the public key
is just an opaque 64-char hex fingerprint.

## Lifecycle

```rust
let path = identity::default_path()
    .ok_or("could not resolve $HOME for ~/.ivaldi/identity")?;
let id = identity::Identity::load_or_create(&path)?;
let pub_hex = id.pubkey_hex();   // 64-char lowercase hex
```

- `Identity::generate()` mints a fresh keypair via `snow::Builder`'s DH
  (no separate `rand` plumbing).
- `Identity::load(path)` returns `None` if the file doesn't exist (so the
  caller can decide to mint).
- `Identity::load_or_create(path)` is the convenience: load-or-mint-and-save.

## CLI

```bash
ivaldi peer whoami
# 51c2277e9d51a47680c11d1e7a9b74f9fce3f1a4207c5e8a92e0e384f358044c
```

That hex is the **only** thing other users need to add you to their
inbound allowlist (`ivaldi peer trust <pubkey>`). Anything else they
care about (display name) is annotated locally on each side.

## Rotation

Not supported as a built-in command yet. Manual rotation:

```bash
mv ~/.ivaldi/identity ~/.ivaldi/identity.old
ivaldi peer whoami       # mints a fresh identity, prints the new pubkey
# Tell every peer to update their `authorized_peers`; tell every server
# you push to to remove the old entry from their `known_peers`.
```

## Security notes

- The file is mode 0600 on Unix. **Don't sync `~/.ivaldi/identity` via
  Dropbox / iCloud / etc.** — it's a private signing key, not credential
  metadata.
- There is no recovery if the file is lost. Future work: derive from a
  passphrase / restore via export.
- The Noise XX handshake gives mutual authentication and forward secrecy;
  pasting a pubkey to a peer over a public channel is fine (it's the
  *public* key).

## Tests

8 unit tests in `src/identity.rs` cover:

- generate produces distinct keys
- save/load round-trip
- load returns None for missing file
- `load_or_create` mints when missing, returns same on second call
- pubkey hex is 64 lowercase hex chars
- load rejects unknown version numbers
- decode_key validates length and hex
