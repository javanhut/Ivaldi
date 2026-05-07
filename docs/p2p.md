# Peer-to-Peer Transport — `ivaldi://`

Direct user-to-user code sharing over TCP, with **no third-party host** in
the loop. Two users (or two machines) exchange Ivaldi seals, trees, and
blobs through an encrypted, mutually-authenticated channel.

This is Ivaldi's *native* transport — there is no git pack format involved
(unlike SSH, which speaks git's wire protocol). The protocol talks
directly in Ivaldi's own data structures (MMR leaves, BLAKE3-keyed CAS
objects), so there's no translation cost or impedance mismatch.

## When to use it

- Pair-coding with a teammate without provisioning a Git host.
- Sharing a private repo on a LAN, VPN, or Tailscale/WireGuard mesh.
- Air-gapped environments where GitHub/GitLab can't be reached.
- Quick "send me your branch" exchanges without push permissions.

NAT traversal is **not** built in. Both peers need to be reachable from
each other — same LAN, port-forwarded, behind a VPN, or via a tunnel.

## Identity

Every Ivaldi install has a long-lived ed25519/X25519 keypair at
`~/.ivaldi/identity` (created on first need; mode 0600 on Unix). The
public half is what other users add to their authorized-peers list.

```bash
ivaldi peer whoami
# 51c2277e9d51a47680c11d1e7a9b74f9fce3f1a4207c5e8a92e0e384f358044c
```

See [`docs/identity.md`](identity.md) for the on-disk format and rotation.

## Trust models

Two distinct allowlists, both human-editable plain-text files:

| File | Direction | What it controls |
|---|---|---|
| `<repo>/.ivaldi/authorized_peers` | **inbound** | Who can connect to *our* `ivaldi serve` |
| `~/.ivaldi/known_peers` | **outbound** | Servers *we* have already trusted on first connect (TOFU, like `~/.ssh/known_hosts`) |

The inbound list is strict: connections from pubkeys not on the list are
rejected after the Noise handshake completes. There is no "auto-trust"
on the server side. The outbound list uses TOFU by default (see below).

## Wire protocol

```text
                     TCP
   client  <─ Noise XX handshake ──>  server
            │ (mutual ed25519 auth) │
            │   ChaCha20-Poly1305    │
            ▼                        ▼
       length-prefixed JSON Message frames
```

After the handshake, frames are 4-byte big-endian length + AEAD ciphertext.
Each "logical" message can span multiple Noise transport chunks (the high
bit of the length prefix marks the last chunk). Payloads are JSON-encoded
`Message` enums:

| Direction | Variant | Purpose |
|---|---|---|
| C→S | `ListTimelines` | Ask the server which timelines it serves |
| S→C | `Timelines { names }` | Reply with timeline names |
| C→S | `WantTimeline { timeline, have }` | "Send me everything reachable from <timeline>; I already have these blake3s" |
| S→C | `Bundle { leaves, blobs }` | Chunk of leaves + tree-nodes + blobs (server may send several) |
| S→C | `Done { head_b3_hex }` | End of stream; carries the tip's BLAKE3 |
| C→S | `PushStart { timeline }` | Begin a push under the named timeline |
| C→S | `PushBundle { leaves, blobs }` | Chunk of objects to land |
| C→S | `PushDone { head_b3_hex }` | End of push |
| S→C | `PushAccepted { landed_as }` | Push wrote to `peers/<sender>/<timeline>` |
| S→C | `PushRejected { reason }` | Verification failure |
| S→C | `Error { message }` | Logical (not transport) error |

The Message enum is `#[serde(tag = "kind")]` and additive — new variants
can be added without breaking existing peers.

## Server side: `ivaldi serve`

```bash
# Inside an Ivaldi repo
ivaldi peer trust <client-pubkey> alice
ivaldi peer trust <client-pubkey> bob
ivaldi serve --bind 0.0.0.0:9418
# ivaldi serve listening on 0.0.0.0:9418 as <my-pubkey>
# press Ctrl-C to stop.
```

- One worker thread per accepted connection, capped at 16 concurrent
  (excess connections drop immediately with a warning).
- Workers share a single `Arc<Mutex<Repo>>` because redb is
  single-handle-per-file. Per-message lock granularity is a follow-up.
- The default port is **9418** (Ivaldi-specific; not the same as git's 9418).

### Receive-only push semantics

Inbound pushes never advance your working timelines. They land at
`peers/<sender>/<timeline>` instead — a quarantine namespace. You
fuse manually when ready:

```bash
ivaldi timeline list                    # peers/bob/main visible
ivaldi timeline switch peers/bob/main   # inspect bob's tree directly
ivaldi fuse peers/bob/main              # integrate into your current timeline
```

Sender label resolution: if the connecting peer's pubkey has a friendly
name in `authorized_peers` (the second column), that's used; otherwise
the first 8 hex chars of the pubkey. Names are sanitized to
`[A-Za-z0-9_-]` so they can't escape the `peers/` prefix.

Server-side dedup: a leaf whose BLAKE3 is already in the MMR (under any
timeline) is skipped on push. Repeating a push is cheap.

## Client side: `download` and `upload`

```bash
# Clone
ivaldi download ivaldi://alice.local:9418/main alice-clone
# First connection to alice.local:9418.
#   pubkey fingerprint: 02e77dff…
# Trust this peer? [y/N] y
# Saved.

# Push (after `ivaldi portal add ivaldi://alice.local:9418`)
ivaldi upload                           # interactive TOFU on first connect
ivaldi upload --branch feat-x           # specific timeline
```

URL forms:

- `ivaldi://host` — defaults to port 9418, server's default timeline
- `ivaldi://host:9999` — custom port
- `ivaldi://host:9999/main` — pin a specific timeline

### TOFU policy

Client behavior on connection (controlled by `~/.ivaldi/known_peers`):

| Policy | Flag | Behavior on unknown pubkey |
|---|---|---|
| Prompt (default) | (none) | Print fingerprint, ask y/N on stdin, save on yes |
| Accept-all | `--accept-new-peer` | Auto-save and proceed (CI, scripts) |
| Strict | `--strict-peer` | Refuse to connect |

Mismatch is **always** fatal — if a host's pubkey changes, Ivaldi
refuses to connect and tells you to run
`ivaldi peer known forget <host[:port]>` if intentional.

```bash
ivaldi peer known list                 # see all known servers
ivaldi peer known forget alice.local   # forget one (port defaults to 9418)
```

## End-to-end example

```bash
# === Alice's machine ===
mkdir alice-repo && cd alice-repo
ivaldi forge
ivaldi config --set user.name "Alice"
echo hello > greeting.txt
ivaldi gather . && ivaldi seal -m "first"

# Get bob's pubkey out of band (Slack, signal, whatever)
ivaldi peer trust 3a2bfe2a8d233055314e0ee47711b1563c232b248f5b4ce0f25187559520a02c bob
ivaldi serve --bind 0.0.0.0:9418

# === Bob's machine ===
ivaldi peer whoami                     # paste this into alice's `peer trust`
ivaldi download ivaldi://alice.local:9418/main alice-clone
# First connection… Trust this peer? [y/N] y
cd alice-clone
echo "bob's edit" >> greeting.txt
ivaldi gather . && ivaldi seal -m "bob edit"
ivaldi portal add ivaldi://alice.local:9418
ivaldi upload
# Pushed timeline 'main' to ivaldi://alice.local:9418
#   landed as: peers/bob/main (1 leaves, 2 objects)

# === Alice's machine ===
# (stop serve to inspect — redb is single-handle-per-file)
ivaldi timeline list
#   peers/bob/main
# * main
ivaldi fuse peers/bob/main
# [OK] Merge completed successfully!
```

## Limitations / not yet built

- **NAT traversal**: explicit URLs only. No discovery, no relay.
- **Push acks aren't persisted per-peer**: the client sends every leaf
  reachable from head on each push. The server dedupes by BLAKE3, so
  repeated content is cheap, but the wire payload doesn't shrink.
- **Connection lifecycle**: `serve` blocks forever; stop with Ctrl-C.
  No graceful shutdown signal yet.
- **No browser/web bridge**: pure TCP between Ivaldi installs.

## Files / modules

- `src/p2p.rs` — Channel (Noise framing) + Message enum + serve/fetch/push
- `src/identity.rs` — long-lived X25519 identity at `~/.ivaldi/identity`
- `src/peers.rs` — repo-local `authorized_peers` (inbound allowlist)
- `src/known_peers.rs` — global `~/.ivaldi/known_peers` (outbound TOFU)

Dependencies added: [`snow`](https://crates.io/crates/snow) (Noise
framework — the only crypto dep; no separate ed25519/x25519 crate is
needed).
