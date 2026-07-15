//! Ivaldi peer-to-peer transport — `ivaldi://` URLs.
//!
//! Lets two users share an Ivaldi repo directly over TCP, with no third-party
//! Git host in the loop. Encryption + mutual authentication ride on the
//! Noise XX handshake (`Noise_XX_25519_ChaChaPoly_BLAKE2s`); each peer is
//! identified by a long-lived X25519 static key (see `identity.rs`).
//!
//! Trust model: the serving peer maintains an `authorized_peers` allowlist
//! (see `peers.rs`); only handshakes whose remote static key appears in
//! that list are honoured. Pure pubkey allowlist — no TOFU prompt yet
//! (the client just rejects/accepts based on the URL it was given).
//!
//! Wire format (after handshake):
//!
//! ```text
//!   <4-byte BE u32 length><payload bytes>
//! ```
//!
//! Payload bytes are JSON-encoded `Message` values. JSON keeps the v1
//! debuggable; binary framing can replace it later without changing the
//! handshake. Each `WriteFrame` is also passed through Noise's
//! `write_message`, which adds the AEAD tag — see `Channel::write_frame`.
//!
//! Protocol shape (v1):
//!
//! - Client opens TCP, performs Noise XX as initiator using its static key.
//! - Server accepts, performs Noise XX as responder, looks up the remote
//!   static key in its `PeerStore`, drops the connection if absent.
//! - Fetch: client sends `ListTimelines` or `WantTimeline`; server replies
//!   `Timelines { … }` or `Bundle { leaves, blobs }` chunks then `Done`.
//! - Push: client sends `PushStart`, `PushBundle` chunks, `PushDone`; the
//!   server lands the chain at `peers/<sender>/<timeline>` and replies
//!   `PushAccepted` or `PushRejected`.
//!
//! Index-space translation: leaf parent links (`prev_idx`/`merge_idxs`)
//! cross the wire as *sender-local* MMR indices. Both landing sites
//! translate them into the recipient's index space via [`LeafLander`]
//! (explicit `sender_idx → local idx` map, hard error on any unmapped
//! parent), and the `Done`/`PushDone` tip claim is checked against the
//! leaves that actually arrived — a truncated transfer fails loudly.
//!
//! Crash model: each landed leaf is one atomic store transaction
//! (`commit_raw`), so an interrupted transfer leaves a valid chain prefix
//! under the target timeline; a fetch additionally removes its half-forged
//! target directory on any failure, and a retried push deduplicates
//! already-landed leaves by hash.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hash::B3Hash;
use crate::identity::Identity;
use crate::peers::PeerStore;

/// Default port for `ivaldi serve` / `ivaldi://host` URLs.
pub const DEFAULT_PORT: u16 = 9418;

const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_FRAME: usize = 16 * 1024 * 1024; // 16 MiB per logical message
const NOISE_MSG_MAX: usize = 65535; // Noise transport limit

/// Parsed `ivaldi://` URL.
///
/// Forms:
/// - `ivaldi://host`                  → port=default, timeline=None
/// - `ivaldi://host:9999`             → custom port
/// - `ivaldi://host:9999/main`        → request a specific timeline
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerUrl {
    pub host: String,
    pub port: u16,
    pub timeline: Option<String>,
}

impl PeerUrl {
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("ivaldi://")?;
        let (hostport, timeline) = match rest.split_once('/') {
            Some((hp, t)) => {
                let t = t.trim_start_matches('/');
                if t.is_empty() {
                    (hp, None)
                } else {
                    (hp, Some(t.to_string()))
                }
            }
            None => (rest, None),
        };
        if hostport.is_empty() {
            return None;
        }
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => {
                let port = p.parse::<u16>().ok()?;
                (h.to_string(), port)
            }
            None => (hostport.to_string(), DEFAULT_PORT),
        };
        if host.is_empty() {
            return None;
        }
        Some(Self {
            host,
            port,
            timeline,
        })
    }

    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// One serialized leaf: its canonical bytes plus the sender-local MMR index
/// it lives at. `prev_idx`/`merge_idxs` inside the canonical bytes are
/// *sender-local* indices; the receiving side MUST translate them through an
/// explicit sender→local index map (see [`LeafLander`]) before persisting —
/// committing them raw would silently graft the chain onto whatever leaves
/// happen to occupy those indices in the recipient's repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLeaf {
    /// Canonical-encoded bytes of the leaf (same shape as on disk).
    pub canonical: Vec<u8>,
    /// The sender-local MMR index of this leaf; the key the sender's
    /// `prev_idx`/`merge_idxs` refer to.
    #[serde(default = "wire_leaf_missing_idx")]
    pub sender_idx: u64,
}

/// Sentinel for a `WireLeaf` sent by an old peer that predates `sender_idx`.
/// Landing such a leaf fails loudly instead of guessing at its lineage.
fn wire_leaf_missing_idx() -> u64 {
    u64::MAX
}

/// One serialized blob, addressed by its BLAKE3 (so the receiver can verify
/// before writing into its CAS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireBlob {
    pub hash_hex: String,
    pub data: Vec<u8>,
}

/// Top-level protocol message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Message {
    /// Client → server: list local timelines.
    ListTimelines,
    /// Server → client: response to `ListTimelines`.
    Timelines { names: Vec<String> },

    /// Client → server: send everything reachable from `timeline`. The
    /// optional `have` list lets the client say "I already have these
    /// leaf-hashes" so the server can prune.
    WantTimeline { timeline: String, have: Vec<String> },

    /// Server → client: data payload(s). Multiple `Bundle` messages may be
    /// sent in sequence before a final `Done`.
    Bundle {
        leaves: Vec<WireLeaf>,
        blobs: Vec<WireBlob>,
    },
    /// Server → client: end of stream.
    Done {
        /// The branch tip's leaf hash (BLAKE3 hex), so the client can wire
        /// up `refs/heads/<timeline>` correctly.
        head_b3_hex: String,
    },

    /// Client → server: about to push the named timeline. Subsequent
    /// `PushBundle` / `PushDone` messages target this timeline.
    PushStart { timeline: String },

    /// Client → server: a chunk of leaves + objects to land. Multiple
    /// `PushBundle` messages may follow `PushStart` before `PushDone`.
    PushBundle {
        leaves: Vec<WireLeaf>,
        blobs: Vec<WireBlob>,
    },

    /// Client → server: end of push. `head_b3_hex` is the BLAKE3 of the
    /// timeline's tip leaf; the server uses it to wire up
    /// `peers/<sender>/<timeline>`.
    PushDone { head_b3_hex: String },

    /// Server → client: push landed at the given local timeline.
    PushAccepted { landed_as: String },

    /// Server → client: push rejected (verification, missing parents,
    /// unknown timeline tip, etc.).
    PushRejected { reason: String },

    /// Server → client: error (logical, not transport).
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum P2pError {
    #[error("p2p I/O: {0}")]
    Io(String),
    #[error("noise handshake: {0}")]
    Handshake(String),
    #[error("peer not authorized")]
    PeerNotAuthorized,
    #[error("protocol: {0}")]
    Protocol(String),
}

impl From<std::io::Error> for P2pError {
    fn from(e: std::io::Error) -> Self {
        P2pError::Io(e.to_string())
    }
}

/// Encrypted, framed channel over a TCP stream after a successful Noise
/// handshake.
pub struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    /// Static public key of the peer on the other end, for authorization
    /// checks and display.
    pub remote_static: [u8; crate::identity::KEY_LEN],
}

impl Channel {
    /// Initiator side. Performs Noise XX with the supplied static keypair.
    pub fn connect(addr: impl ToSocketAddrs, identity: &Identity) -> Result<Self, P2pError> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(Duration::from_secs(60)))?;
        let noise = handshake_initiator(&stream, identity)?;
        let remote_static = extract_remote_static(&noise)?;
        Ok(Self {
            stream,
            noise,
            remote_static,
        })
    }

    /// Responder side. Caller has already accepted a TCP connection.
    pub fn accept(stream: TcpStream, identity: &Identity) -> Result<Self, P2pError> {
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(Duration::from_secs(60)))?;
        let noise = handshake_responder(&stream, identity)?;
        let remote_static = extract_remote_static(&noise)?;
        Ok(Self {
            stream,
            noise,
            remote_static,
        })
    }

    /// Send one logical message. Encrypts via Noise, frames with a 4-byte
    /// big-endian length prefix.
    pub fn send(&mut self, msg: &Message) -> Result<(), P2pError> {
        let payload =
            serde_json::to_vec(msg).map_err(|e| P2pError::Protocol(format!("encode: {}", e)))?;
        if payload.len() > MAX_FRAME {
            return Err(P2pError::Protocol(format!(
                "outbound message too large ({} > {}) — a single file larger than \
                 ~4 MiB cannot yet transit ivaldi:// (JSON encoding overhead); \
                 use an HTTPS or SSH portal for this repo",
                payload.len(),
                MAX_FRAME
            )));
        }

        // Encrypt in NOISE_MSG_MAX-sized chunks (snow's transport limit).
        // Each chunk is its own length-prefixed frame; the receiver glues
        // them back together in `recv` until the outer logical message is
        // exhausted (we mark the last chunk by setting the high bit of the
        // length prefix).
        let mut offset = 0;
        while offset < payload.len() {
            let take = (payload.len() - offset).min(NOISE_MSG_MAX - 16); // leave room for AEAD tag
            let last = offset + take == payload.len();
            let mut buf = vec![0u8; take + 16];
            let n = self
                .noise
                .write_message(&payload[offset..offset + take], &mut buf)
                .map_err(|e| P2pError::Protocol(format!("encrypt: {}", e)))?;
            buf.truncate(n);
            let mut hdr = (n as u32).to_be_bytes();
            if last {
                hdr[0] |= 0x80; // high bit = "end of logical message"
            }
            self.stream.write_all(&hdr)?;
            self.stream.write_all(&buf)?;
            offset += take;
        }
        self.stream.flush()?;
        Ok(())
    }

    /// Receive one logical message (possibly reassembled from multiple
    /// Noise-transport chunks).
    pub fn recv(&mut self) -> Result<Message, P2pError> {
        let mut payload = Vec::new();
        loop {
            let mut hdr = [0u8; 4];
            self.stream.read_exact(&mut hdr)?;
            let last = hdr[0] & 0x80 != 0;
            hdr[0] &= 0x7f;
            let len = u32::from_be_bytes(hdr) as usize;
            if len > NOISE_MSG_MAX {
                return Err(P2pError::Protocol(format!(
                    "inbound chunk too large ({} > {})",
                    len, NOISE_MSG_MAX
                )));
            }
            // Enforce the logical-message cap BEFORE reading the chunk, so the
            // reassembled payload can never exceed MAX_FRAME even transiently.
            if payload.len() + len > MAX_FRAME {
                return Err(P2pError::Protocol(format!(
                    "inbound message too large ({} > {})",
                    payload.len() + len,
                    MAX_FRAME
                )));
            }
            let mut ctxt = vec![0u8; len];
            self.stream.read_exact(&mut ctxt)?;
            let mut ptxt = vec![0u8; len];
            let n = self
                .noise
                .read_message(&ctxt, &mut ptxt)
                .map_err(|e| P2pError::Protocol(format!("decrypt: {}", e)))?;
            payload.extend_from_slice(&ptxt[..n]);
            if last {
                break;
            }
        }
        let msg: Message = serde_json::from_slice(&payload)
            .map_err(|e| P2pError::Protocol(format!("decode: {}", e)))?;
        Ok(msg)
    }

    pub fn shutdown(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

fn handshake_initiator(
    mut stream: &TcpStream,
    identity: &Identity,
) -> Result<snow::TransportState, P2pError> {
    let params: snow::params::NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|e: snow::Error| P2pError::Handshake(e.to_string()))?;
    let mut h = snow::Builder::new(params)
        .local_private_key(&identity.secret)
        .build_initiator()
        .map_err(|e| P2pError::Handshake(e.to_string()))?;

    let mut buf = vec![0u8; NOISE_MSG_MAX];

    // -> e
    let n = h
        .write_message(&[], &mut buf)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;
    write_handshake(&mut stream, &buf[..n])?;

    // <- e, ee, s, es
    let msg = read_handshake(&mut stream)?;
    let mut tmp = vec![0u8; NOISE_MSG_MAX];
    h.read_message(&msg, &mut tmp)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;

    // -> s, se
    let n = h
        .write_message(&[], &mut buf)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;
    write_handshake(&mut stream, &buf[..n])?;

    h.into_transport_mode()
        .map_err(|e| P2pError::Handshake(e.to_string()))
}

fn handshake_responder(
    mut stream: &TcpStream,
    identity: &Identity,
) -> Result<snow::TransportState, P2pError> {
    let params: snow::params::NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|e: snow::Error| P2pError::Handshake(e.to_string()))?;
    let mut h = snow::Builder::new(params)
        .local_private_key(&identity.secret)
        .build_responder()
        .map_err(|e| P2pError::Handshake(e.to_string()))?;

    let mut buf = vec![0u8; NOISE_MSG_MAX];
    let mut tmp = vec![0u8; NOISE_MSG_MAX];

    // <- e
    let msg = read_handshake(&mut stream)?;
    h.read_message(&msg, &mut tmp)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;

    // -> e, ee, s, es
    let n = h
        .write_message(&[], &mut buf)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;
    write_handshake(&mut stream, &buf[..n])?;

    // <- s, se
    let msg = read_handshake(&mut stream)?;
    h.read_message(&msg, &mut tmp)
        .map_err(|e| P2pError::Handshake(e.to_string()))?;

    h.into_transport_mode()
        .map_err(|e| P2pError::Handshake(e.to_string()))
}

fn write_handshake(stream: &mut &TcpStream, msg: &[u8]) -> Result<(), P2pError> {
    let len = (msg.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(msg)?;
    stream.flush()?;
    Ok(())
}

fn read_handshake(stream: &mut &TcpStream) -> Result<Vec<u8>, P2pError> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > NOISE_MSG_MAX {
        return Err(P2pError::Handshake(format!(
            "handshake message too large ({})",
            n
        )));
    }
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

fn extract_remote_static(
    state: &snow::TransportState,
) -> Result<[u8; crate::identity::KEY_LEN], P2pError> {
    let raw = state
        .get_remote_static()
        .ok_or_else(|| P2pError::Handshake("missing remote static key".into()))?;
    if raw.len() != crate::identity::KEY_LEN {
        return Err(P2pError::Handshake(format!(
            "remote static key wrong length: {}",
            raw.len()
        )));
    }
    let mut out = [0u8; crate::identity::KEY_LEN];
    out.copy_from_slice(raw);
    Ok(out)
}

// =====================================================================
// Server side
// =====================================================================

/// Maximum number of concurrent client connections served at once. Hard cap
/// so a flood of opens can't spawn unbounded threads.
const SERVE_MAX_CONCURRENT: usize = 16;

/// `ivaldi serve` loop. Listens, then for each accepted connection spawns
/// a worker thread that authenticates against `authorized_peers` and serves
/// the request. Concurrency is capped at [`SERVE_MAX_CONCURRENT`] in-flight
/// handlers; new connections beyond that are dropped (TCP closed) so we
/// don't queue work invisibly to the operator.
///
/// **Locking note**: redb allows only one open `Database` per file (even
/// within one process), so all workers share a single `Arc<Mutex<Repo>>`.
/// This means repo-touching operations effectively serialize at the
/// per-connection granularity — fine for v1 (the connection lifecycle is
/// short and dominated by network), but a finer-grained read/write split
/// is the obvious next step if a server gets traffic.
pub fn serve(
    bind: &str,
    repo_root: std::path::PathBuf,
    identity: &Identity,
    peer_store_path: std::path::PathBuf,
) -> Result<(), P2pError> {
    use std::sync::Arc;
    use std::sync::Mutex;

    let repo = crate::repo::Repo::open(&repo_root)
        .map_err(|e| P2pError::Io(format!("open repo: {}", e)))?;
    let repo = Arc::new(Mutex::new(repo));
    serve_with_repo(bind, repo, identity, peer_store_path)
}

/// Like [`serve`], but with a caller-supplied repo handle. Lets tests
/// share the same `Arc<Mutex<Repo>>` between the server thread and the
/// test thread (redb is single-handle per file, so two opens in one
/// process collide).
pub fn serve_with_repo(
    bind: &str,
    repo: std::sync::Arc<std::sync::Mutex<crate::repo::Repo>>,
    identity: &Identity,
    peer_store_path: std::path::PathBuf,
) -> Result<(), P2pError> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let listener = TcpListener::bind(bind)?;
    eprintln!(
        "ivaldi serve listening on {} as {}",
        listener.local_addr()?,
        identity.pubkey_hex()
    );
    eprintln!("press Ctrl-C to stop.");

    let inflight = Arc::new(AtomicUsize::new(0));

    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(e) => {
                crate::logging::warn(&format!("accept failed: {}", e));
                continue;
            }
        };

        if inflight.load(Ordering::Acquire) >= SERVE_MAX_CONCURRENT {
            crate::logging::warn(&format!(
                "rejecting connection — at concurrency cap ({})",
                SERVE_MAX_CONCURRENT
            ));
            continue;
        }
        inflight.fetch_add(1, Ordering::AcqRel);

        let identity = identity.clone();
        let peer_store_path = peer_store_path.clone();
        let counter = inflight.clone();
        let repo = repo.clone();
        std::thread::spawn(move || {
            struct Guard<'a>(&'a Arc<AtomicUsize>);
            impl<'a> Drop for Guard<'a> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _g = Guard(&counter);

            let peer_store = PeerStore::new(peer_store_path);
            let mut guard = match repo.lock() {
                Ok(g) => g,
                Err(_) => {
                    crate::logging::warn("worker: repo mutex poisoned, skipping");
                    return;
                }
            };
            if let Err(e) = handle_connection(&mut guard, stream, &identity, &peer_store) {
                crate::logging::warn(&format!("connection error: {}", e));
            }
        });
    }
    Ok(())
}

fn handle_connection(
    repo: &mut crate::repo::Repo,
    stream: TcpStream,
    identity: &Identity,
    peer_store: &PeerStore,
) -> Result<(), P2pError> {
    let peer_addr = stream.peer_addr().ok();
    let mut chan = Channel::accept(stream, identity)?;
    if !peer_store
        .is_trusted(&chan.remote_static)
        .map_err(|e| P2pError::Protocol(e.to_string()))?
    {
        eprintln!(
            "rejecting connection from {:?}: pubkey {} not in authorized_peers",
            peer_addr,
            hex::encode(chan.remote_static)
        );
        let _ = chan.send(&Message::Error {
            message: "peer not authorized".into(),
        });
        chan.shutdown();
        return Err(P2pError::PeerNotAuthorized);
    }
    eprintln!(
        "peer {} ({:?}) connected",
        hex::encode(chan.remote_static),
        peer_addr
    );

    loop {
        let req = match chan.recv() {
            Ok(m) => m,
            Err(P2pError::Io(_)) => break, // peer disconnected
            Err(e) => return Err(e),
        };
        match req {
            Message::ListTimelines => {
                let names = repo
                    .list_timelines()
                    .map_err(|e| P2pError::Protocol(e.to_string()))?
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect();
                chan.send(&Message::Timelines { names })?;
            }
            Message::WantTimeline { timeline, have } => {
                serve_want(repo, &mut chan, &timeline, &have)?;
            }
            Message::PushStart { timeline } => {
                serve_push(repo, &mut chan, &timeline, peer_store)?;
            }
            other => {
                chan.send(&Message::Error {
                    message: format!("unsupported request: {:?}", other),
                })?;
            }
        }
    }
    Ok(())
}

/// Receive-only push. Lands inbound seals at `peers/<sender>/<timeline>`
/// rather than advancing any of the recipient's working timelines. The
/// recipient runs `ivaldi fuse peers/<sender>/<timeline>` to integrate
/// the push manually — this matches the explicit "user fuses manually"
/// model picked at design time.
///
/// Sender label resolution: if the connecting peer's pubkey has a
/// friendly name in `authorized_peers`, use that; otherwise use the first
/// 8 hex chars of the pubkey (sufficiently unique in practice for two
/// users sharing one repo).
fn serve_push(
    repo: &mut crate::repo::Repo,
    chan: &mut Channel,
    timeline: &str,
    peer_store: &PeerStore,
) -> Result<(), P2pError> {
    use crate::cas::FileCas;

    // Resolve sender label.
    let entries = peer_store
        .list()
        .map_err(|e| P2pError::Protocol(e.to_string()))?;
    let sender = entries
        .iter()
        .find(|e| e.pubkey == chan.remote_static)
        .and_then(|e| e.name.clone())
        .unwrap_or_else(|| hex::encode(&chan.remote_static[..4]));

    // Sanitize the sender label so it can't escape the `peers/` prefix.
    let sender_clean: String = sender
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let landed_as = format!("peers/{}/{}", sender_clean, timeline);

    let cas =
        FileCas::new(repo.ivaldi_dir.join("objects")).map_err(|e| P2pError::Io(e.to_string()))?;

    let mut lander = LeafLander::default();
    let claimed_head = loop {
        match chan.recv()? {
            Message::PushBundle { leaves, blobs } => {
                // Write objects first (they're prerequisites for any
                // leaf's tree walk later). Bytes are content-addressed,
                // so duplicates are no-ops.
                for wb in blobs {
                    apply_blob(&cas, &wb)?;
                }
                // This bundle's objects must be durable (directory entries
                // included) before any leaf transaction references them.
                cas.flush().map_err(|e| P2pError::Io(e.to_string()))?;
                for wl in leaves {
                    if let Err(e) = lander.land(repo, &landed_as, &wl) {
                        // A bad parent reference or malformed leaf must not
                        // poison the recipient's history: reject explicitly.
                        // Everything landed so far is a valid prefix under
                        // peers/<sender>/ and a retry re-lands idempotently.
                        chan.send(&Message::PushRejected {
                            reason: e.to_string(),
                        })?;
                        return Ok(());
                    }
                }
            }
            Message::PushDone { head_b3_hex } => break head_b3_hex,
            Message::Error { message } => return Err(P2pError::Protocol(message)),
            other => {
                chan.send(&Message::PushRejected {
                    reason: format!("unexpected message during push: {:?}", other),
                })?;
                return Ok(());
            }
        }
    };

    // The sender's claimed tip must be a leaf that actually arrived —
    // otherwise a truncated push would be indistinguishable from a
    // complete one.
    let tip_idx = match lander.local_idx_for_wire_hash(&claimed_head) {
        Some(idx) => idx,
        None => {
            chan.send(&Message::PushRejected {
                reason: format!(
                    "push tip {} never arrived — transfer truncated or peer misbehaving",
                    claimed_head
                ),
            })?;
            return Ok(());
        }
    };
    // Re-pushing an already-known chain lands zero new leaves, so point the
    // peers/ timeline at the (deduplicated) tip explicitly.
    if repo
        .get_timeline_head(&landed_as)
        .map_err(|e| P2pError::Protocol(e.to_string()))?
        != Some(tip_idx)
    {
        repo.set_timeline_head(&landed_as, tip_idx)
            .map_err(|e| P2pError::Io(e.to_string()))?;
    }

    chan.send(&Message::PushAccepted {
        landed_as: landed_as.clone(),
    })?;
    eprintln!(
        "received push: {} new leaf(s) landed at {}",
        lander.leaves_landed, landed_as
    );
    Ok(())
}

/// Translates wire leaves (whose `prev_idx`/`merge_idxs` are *sender-local*
/// MMR indices) into the recipient's index space before persisting.
///
/// Every parent must resolve through the explicit sender→local map built
/// from the leaves that arrived earlier in the same transfer; an unmapped
/// parent is a hard error, never a clamp — committing a guessed index would
/// silently graft the pushed chain onto unrelated local history.
#[derive(Default)]
struct LeafLander {
    /// sender-local index → recipient-local index.
    sender_to_local: std::collections::BTreeMap<u64, u64>,
    /// BLAKE3(wire canonical bytes) hex → recipient-local index. Used to
    /// check the sender's `Done`/`PushDone` tip claim against what actually
    /// arrived (the *landed* leaf hash differs once indices are rewritten).
    local_by_wire_hash: std::collections::BTreeMap<String, u64>,
    leaves_landed: usize,
}

impl LeafLander {
    /// Land one wire leaf on `timeline`: parse, rewrite parents through the
    /// sender→local map, deduplicate against leaves already in the MMR, and
    /// commit. Old-or-new per leaf (single store transaction inside
    /// `commit_raw`); an error leaves the repo at a valid prefix.
    fn land(
        &mut self,
        repo: &mut crate::repo::Repo,
        timeline: &str,
        wl: &WireLeaf,
    ) -> Result<(), P2pError> {
        use crate::leaf::NO_PARENT;

        if wl.sender_idx == u64::MAX {
            return Err(P2pError::Protocol(
                "peer sent a leaf without its sender index (old protocol?) — refusing to guess \
                 its lineage; both peers must run a current ivaldi"
                    .into(),
            ));
        }
        let mut leaf = crate::leaf::parse_leaf(&wl.canonical)
            .map_err(|e| P2pError::Protocol(format!("parse leaf: {}", e)))?;
        let wire_hash = hex::encode(B3Hash::digest(&wl.canonical).as_bytes());

        if leaf.prev_idx != NO_PARENT {
            leaf.prev_idx = self.resolve_parent(leaf.prev_idx)?;
        }
        leaf.merge_idxs = leaf
            .merge_idxs
            .iter()
            .map(|&m| self.resolve_parent(m))
            .collect::<Result<Vec<_>, _>>()?;

        let hash = leaf.hash();
        let already = repo
            .get_seal_name(hash)
            .map_err(|e| P2pError::Protocol(e.to_string()))?
            .is_some();
        let local_idx = if already {
            // ponytail: O(n) scan per duplicate leaf; index hash→idx in the
            // store if re-pushing huge histories ever gets hot.
            repo.resolve_seal(&hex::encode(hash.as_bytes()))
                .map_err(|e| P2pError::Protocol(e.to_string()))?
                .map(|(idx, _)| idx)
                .ok_or_else(|| P2pError::Protocol("seal name exists but leaf not found".into()))?
        } else {
            let result = repo
                .commit_raw(leaf, timeline)
                .map_err(|e| P2pError::Io(e.to_string()))?;
            self.leaves_landed += 1;
            result.index
        };
        self.sender_to_local.insert(wl.sender_idx, local_idx);
        self.local_by_wire_hash.insert(wire_hash, local_idx);
        Ok(())
    }

    fn resolve_parent(&self, sender_idx: u64) -> Result<u64, P2pError> {
        self.sender_to_local
            .get(&sender_idx)
            .copied()
            .ok_or_else(|| {
                P2pError::Protocol(format!(
                    "leaf references parent at sender index {} which was never transferred — \
                     refusing to graft onto unrelated local history",
                    sender_idx
                ))
            })
    }

    fn local_idx_for_wire_hash(&self, wire_hash_hex: &str) -> Option<u64> {
        self.local_by_wire_hash.get(wire_hash_hex).copied()
    }
}

fn serve_want(
    repo: &mut crate::repo::Repo,
    chan: &mut Channel,
    timeline: &str,
    have: &[String],
) -> Result<(), P2pError> {
    let head = repo
        .get_timeline_head(timeline)
        .map_err(|e| P2pError::Protocol(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol(format!("unknown timeline '{}'", timeline)))?;

    // Walk the linear chain (prev_idx + merge parents) from head back, stopping
    // at any leaf whose blake3 the client already has.
    let have_set: BTreeSet<String> = have.iter().cloned().collect();
    let mut to_send_idx: Vec<u64> = Vec::new();
    let mut visited: BTreeSet<u64> = BTreeSet::new();
    let mut q: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
    q.push_back(head);
    while let Some(idx) = q.pop_front() {
        if !visited.insert(idx) {
            continue;
        }
        let leaf = match repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Protocol(e.to_string()))?
        {
            Some(l) => l,
            None => continue,
        };
        if have_set.contains(&hex::encode(leaf.hash().as_bytes())) {
            continue;
        }
        to_send_idx.push(idx);
        for p in leaf.all_parents() {
            q.push_back(p);
        }
    }

    // Walk the head leaf's tree to collect every reachable blob hash.
    use crate::cas::FileCas;
    use crate::fsmerkle::FsStore;
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| P2pError::Protocol(e.to_string()))?;
    let store = FsStore::new(&cas);

    // We need to ship every CAS object the receiver will need to
    // materialize: blobs *and* tree-node bytes (addressed by their own
    // hash). Both go through the same `WireBlob` channel — they're just
    // hash → bytes pairs from CAS's point of view.
    let mut object_hashes: BTreeSet<B3Hash> = BTreeSet::new();
    let mut tree_visited: BTreeSet<B3Hash> = BTreeSet::new();
    for &idx in &to_send_idx {
        if let Some(leaf) = repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Protocol(e.to_string()))?
        {
            collect_objects_from_tree(
                &store,
                leaf.tree_root,
                &mut tree_visited,
                &mut object_hashes,
            )
            .map_err(|e| P2pError::Protocol(e.to_string()))?;
        }
    }

    // Send leaves oldest → newest. Ascending MMR index is a valid
    // topological order (commit_raw enforces parent < child index), so every
    // parent is on the wire before any leaf that references it — the
    // receiver's sender→local remap can then resolve parents as they arrive.
    to_send_idx.sort_unstable();
    let mut leaves: Vec<WireLeaf> = Vec::with_capacity(to_send_idx.len());
    for &idx in to_send_idx.iter() {
        let leaf = repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Protocol(e.to_string()))?
            .ok_or_else(|| P2pError::Protocol(format!("leaf {} vanished", idx)))?;
        leaves.push(WireLeaf {
            canonical: leaf.canonical_bytes(),
            sender_idx: idx,
        });
    }

    let head_leaf = repo
        .get_leaf(head)
        .map_err(|e| P2pError::Protocol(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol("head leaf vanished".into()))?;
    let head_hex = hex::encode(head_leaf.hash().as_bytes());

    // Chunk: ship leaves first, then blobs.
    chan.send(&Message::Bundle {
        leaves,
        blobs: Vec::new(),
    })?;

    let mut chunk: Vec<WireBlob> = Vec::new();
    for hash in object_hashes {
        use crate::cas::Cas;
        let bytes = cas
            .get(hash)
            .map_err(|e| P2pError::Protocol(format!("cas read: {}", e)))?;
        chunk.push(WireBlob {
            hash_hex: hex::encode(hash.as_bytes()),
            data: bytes,
        });
        if chunk.len() >= 64 {
            let take = std::mem::take(&mut chunk);
            chan.send(&Message::Bundle {
                leaves: Vec::new(),
                blobs: take,
            })?;
        }
    }
    if !chunk.is_empty() {
        chan.send(&Message::Bundle {
            leaves: Vec::new(),
            blobs: chunk,
        })?;
    }

    chan.send(&Message::Done {
        head_b3_hex: head_hex,
    })?;
    Ok(())
}

/// Collect every CAS object hash reachable from `tree_hash` — both blob
/// hashes (`NodeKind::Blob` entries) and the tree-node hashes themselves.
/// The receiver needs both to be able to load and materialize the tree.
fn collect_objects_from_tree(
    store: &crate::fsmerkle::FsStore<'_>,
    tree_hash: B3Hash,
    seen_trees: &mut BTreeSet<B3Hash>,
    out: &mut BTreeSet<B3Hash>,
) -> Result<(), crate::fsmerkle::FsMerkleError> {
    if !seen_trees.insert(tree_hash) {
        return Ok(());
    }
    out.insert(tree_hash); // ship the tree-node bytes too
    let tree = store.load_tree(tree_hash)?;
    for entry in &tree.entries {
        match entry.kind {
            crate::fsmerkle::NodeKind::Blob => {
                out.insert(entry.hash);
            }
            crate::fsmerkle::NodeKind::Tree => {
                collect_objects_from_tree(store, entry.hash, seen_trees, out)?;
            }
        }
    }
    Ok(())
}

// =====================================================================
// Client side
// =====================================================================

/// `ivaldi download ivaldi://host[:port][/timeline]`.
pub fn fetch_into(
    url: &PeerUrl,
    target_dir: &Path,
    identity: &Identity,
) -> Result<FetchSummary, P2pError> {
    fetch_into_with_policy(
        url,
        target_dir,
        identity,
        crate::known_peers::TofuPolicy::Prompt,
    )
}

/// Fetch with explicit TOFU policy. Used by `cmd_download` to honour
/// `--accept-new-peer` / `--strict-peer` flags.
pub fn fetch_into_with_policy(
    url: &PeerUrl,
    target_dir: &Path,
    identity: &Identity,
    tofu: crate::known_peers::TofuPolicy,
) -> Result<FetchSummary, P2pError> {
    if target_dir.exists()
        && target_dir
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        return Err(P2pError::Io(format!(
            "directory '{}' already exists and is not empty",
            target_dir.display()
        )));
    }
    let created_target = !target_dir.exists();
    std::fs::create_dir_all(target_dir).map_err(P2pError::from)?;

    // Run the whole fetch in a closure so any failure (truncated stream,
    // bad parent reference, missing objects) removes the half-forged target
    // instead of stranding a repo that trips the "already exists and is not
    // empty" guard on retry. Mirrors sync::download_with_fetch.
    let result = fetch_into_created_target(url, target_dir, identity, tofu);
    if result.is_err() && created_target {
        let _ = std::fs::remove_dir_all(target_dir);
    }
    result
}

fn fetch_into_created_target(
    url: &PeerUrl,
    target_dir: &Path,
    identity: &Identity,
    tofu: crate::known_peers::TofuPolicy,
) -> Result<FetchSummary, P2pError> {
    let mut chan = Channel::connect(url.socket_addr(), identity)?;
    enforce_tofu(url, &chan.remote_static, tofu)?;
    eprintln!("connected to {}", hex::encode(chan.remote_static));

    // Decide which timeline to fetch. If the URL pinned one, use it; else
    // ask the server and pick the first.
    let timeline = match url.timeline.clone() {
        Some(t) => t,
        None => {
            chan.send(&Message::ListTimelines)?;
            match chan.recv()? {
                Message::Timelines { names } => names
                    .into_iter()
                    .next()
                    .ok_or_else(|| P2pError::Protocol("server has no timelines".into()))?,
                Message::Error { message } => return Err(P2pError::Protocol(message)),
                other => {
                    return Err(P2pError::Protocol(format!(
                        "unexpected response to ListTimelines: {:?}",
                        other
                    )));
                }
            }
        }
    };

    chan.send(&Message::WantTimeline {
        timeline: timeline.clone(),
        have: Vec::new(),
    })?;

    crate::forge::forge(target_dir).map_err(|e| P2pError::Io(e.to_string()))?;
    let mut repo = crate::repo::Repo::open(target_dir).map_err(|e| P2pError::Io(e.to_string()))?;

    let cas = crate::cas::FileCas::new(target_dir.join(".ivaldi/objects"))
        .map_err(|e| P2pError::Io(e.to_string()))?;

    let mut lander = LeafLander::default();
    let mut blobs_imported = 0usize;
    let head_b3_hex: String = loop {
        match chan.recv()? {
            Message::Bundle { leaves, blobs } => {
                for wl in leaves {
                    lander.land(&mut repo, &timeline, &wl)?;
                }
                for wb in blobs {
                    apply_blob(&cas, &wb)?;
                    blobs_imported += 1;
                }
            }
            Message::Done { head_b3_hex: head } => {
                break head;
            }
            Message::Error { message } => return Err(P2pError::Protocol(message)),
            other => {
                return Err(P2pError::Protocol(format!(
                    "unexpected message: {:?}",
                    other
                )));
            }
        }
    };
    chan.shutdown();
    let leaves_imported = lander.leaves_landed;

    // The server's claimed tip must correspond to a leaf that actually
    // arrived AND be the timeline head we landed — otherwise the stream was
    // truncated or reordered and the clone would silently be missing seals.
    let claimed_idx = lander
        .local_idx_for_wire_hash(&head_b3_hex)
        .ok_or_else(|| {
            P2pError::Protocol(format!(
                "server reported tip {} but that leaf never arrived — transfer truncated",
                head_b3_hex
            ))
        })?;
    let head_idx = repo
        .get_timeline_head(&timeline)
        .map_err(|e| P2pError::Io(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol("no leaves imported".into()))?;
    if head_idx != claimed_idx {
        return Err(P2pError::Protocol(format!(
            "imported head (leaf {}) does not match the tip the server reported (leaf {})",
            head_idx, claimed_idx
        )));
    }

    // Materialize the working tree from the imported head.
    let head_leaf = repo
        .get_leaf(head_idx)
        .map_err(|e| P2pError::Io(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol("imported head leaf missing".into()))?;
    let workspace = crate::workspace::Workspace::new(&cas, target_dir, target_dir.join(".ivaldi"));
    workspace
        .materialize(head_leaf.tree_root)
        .map_err(|e| P2pError::Io(e.to_string()))?;
    crate::forge::write_head(
        &target_dir.join(".ivaldi"),
        &crate::forge::HeadRef::Timeline(timeline.clone()),
    )
    .map_err(|e| P2pError::Io(e.to_string()))?;

    Ok(FetchSummary {
        timeline,
        leaves_imported,
        blobs_imported,
        head_b3_hex,
    })
}

/// Apply the TOFU policy after a successful Noise handshake. On `Match`
/// returns Ok. On `Mismatch` always errors (never silently overwrites a
/// known pubkey). On `Unknown` the policy decides:
///
/// - `Prompt`: print fingerprint, ask y/N on stdin, save on yes.
/// - `AcceptAll`: save and proceed silently.
/// - `StrictKnown`: refuse with a clear error.
fn enforce_tofu(
    url: &PeerUrl,
    remote: &[u8; crate::identity::KEY_LEN],
    policy: crate::known_peers::TofuPolicy,
) -> Result<(), P2pError> {
    use crate::known_peers::{Known, KnownPeers, TofuPolicy, fingerprint};

    let Some(store) = KnownPeers::default_for_user() else {
        // No HOME — fall through without TOFU. Identity files won't have
        // worked either; treat as "skipped" rather than fatal.
        crate::logging::warn("no $HOME — skipping TOFU check; consider setting IVALDI_KNOWN_PEERS");
        return Ok(());
    };

    match store
        .lookup(&url.host, url.port, remote)
        .map_err(|e| P2pError::Protocol(format!("known_peers: {}", e)))?
    {
        Known::Match => Ok(()),
        Known::Mismatch { stored } => Err(P2pError::Protocol(format!(
            "REFUSING TO CONNECT: server pubkey for {}:{} changed.\n  expected (known): {}\n  remote sent:      {}\nIf this is intentional, run `ivaldi peer known forget {}:{}` first.",
            url.host,
            url.port,
            fingerprint(&stored),
            fingerprint(remote),
            url.host,
            url.port,
        ))),
        Known::Unknown => match policy {
            TofuPolicy::AcceptAll => {
                store
                    .record(&url.host, url.port, remote)
                    .map_err(|e| P2pError::Protocol(format!("known_peers: {}", e)))?;
                Ok(())
            }
            TofuPolicy::StrictKnown => Err(P2pError::Protocol(format!(
                "unknown peer {}:{} (--strict-peer): pubkey {} not in known_peers",
                url.host,
                url.port,
                fingerprint(remote),
            ))),
            TofuPolicy::Prompt => {
                eprintln!("First connection to {}:{}.", url.host, url.port);
                eprintln!("  pubkey fingerprint: {}", fingerprint(remote));
                eprint!("Trust this peer? [y/N] ");
                use std::io::Write;
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| P2pError::Io(e.to_string()))?;
                if line.trim().eq_ignore_ascii_case("y") || line.trim().eq_ignore_ascii_case("yes")
                {
                    store
                        .record(&url.host, url.port, remote)
                        .map_err(|e| P2pError::Protocol(format!("known_peers: {}", e)))?;
                    eprintln!("Saved.");
                    Ok(())
                } else {
                    Err(P2pError::Protocol("user declined to trust peer".into()))
                }
            }
        },
    }
}

fn apply_blob(cas: &crate::cas::FileCas, wb: &WireBlob) -> Result<(), P2pError> {
    use crate::cas::Cas;
    let raw = hex::decode(&wb.hash_hex)
        .map_err(|e| P2pError::Protocol(format!("blob hash hex: {}", e)))?;
    let hash = B3Hash::from_slice(&raw)
        .ok_or_else(|| P2pError::Protocol("blob hash wrong length".into()))?;
    cas.put(hash, &wb.data)
        .map_err(|e| P2pError::Io(format!("cas put: {}", e)))?;
    Ok(())
}

/// Result of a successful fetch.
#[derive(Debug, Clone)]
pub struct FetchSummary {
    pub timeline: String,
    pub leaves_imported: usize,
    pub blobs_imported: usize,
    pub head_b3_hex: String,
}

/// Result of a successful push.
#[derive(Debug, Clone)]
pub struct PushSummary {
    pub landed_as: String,
    pub leaves_sent: usize,
    pub objects_sent: usize,
}

/// `ivaldi upload ivaldi://host[:port]` — push the given timeline of `repo`
/// to a peer's `serve`. The peer lands the push at
/// `peers/<sender>/<timeline>`; the peer fuses manually from there.
pub fn push_to(
    url: &PeerUrl,
    repo: &mut crate::repo::Repo,
    identity: &Identity,
    timeline: &str,
    tofu: crate::known_peers::TofuPolicy,
) -> Result<PushSummary, P2pError> {
    use std::collections::BTreeSet;

    let head_idx = repo
        .get_timeline_head(timeline)
        .map_err(|e| P2pError::Io(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol(format!("local timeline '{}' has no head", timeline)))?;

    let mut chan = Channel::connect(url.socket_addr(), identity)?;
    enforce_tofu(url, &chan.remote_static, tofu)?;

    chan.send(&Message::PushStart {
        timeline: timeline.to_string(),
    })?;

    // Walk back along prev_idx + merge_idxs to collect the leaves to send,
    // then walk each leaf's tree for the object set.
    let mut leaf_indices: Vec<u64> = Vec::new();
    let mut visited: BTreeSet<u64> = BTreeSet::new();
    let mut q: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
    q.push_back(head_idx);
    while let Some(idx) = q.pop_front() {
        if !visited.insert(idx) {
            continue;
        }
        let leaf = match repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Io(e.to_string()))?
        {
            Some(l) => l,
            None => continue,
        };
        leaf_indices.push(idx);
        for p in leaf.all_parents() {
            q.push_back(p);
        }
    }

    use crate::cas::{Cas, FileCas};
    use crate::fsmerkle::FsStore;
    let cas =
        FileCas::new(repo.ivaldi_dir.join("objects")).map_err(|e| P2pError::Io(e.to_string()))?;
    let store = FsStore::new(&cas);

    let mut object_hashes: BTreeSet<crate::hash::B3Hash> = BTreeSet::new();
    let mut tree_visited: BTreeSet<crate::hash::B3Hash> = BTreeSet::new();
    for &idx in &leaf_indices {
        if let Some(leaf) = repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Io(e.to_string()))?
        {
            collect_objects_from_tree(
                &store,
                leaf.tree_root,
                &mut tree_visited,
                &mut object_hashes,
            )
            .map_err(|e| P2pError::Protocol(e.to_string()))?;
        }
    }

    // Materialize leaves oldest-first (ascending MMR index is a valid
    // topological order) so the receiver can remap parent indices as leaves
    // arrive.
    leaf_indices.sort_unstable();
    let mut wire_leaves: Vec<WireLeaf> = Vec::new();
    for &idx in leaf_indices.iter() {
        let leaf = repo
            .get_leaf(idx)
            .map_err(|e| P2pError::Io(e.to_string()))?
            .ok_or_else(|| P2pError::Protocol(format!("leaf {} vanished", idx)))?;
        wire_leaves.push(WireLeaf {
            canonical: leaf.canonical_bytes(),
            sender_idx: idx,
        });
    }

    let head_leaf = repo
        .get_leaf(head_idx)
        .map_err(|e| P2pError::Io(e.to_string()))?
        .ok_or_else(|| P2pError::Protocol("head leaf vanished".into()))?;
    let head_b3_hex = hex::encode(head_leaf.hash().as_bytes());

    // Ship leaves first (one bundle), then objects in 64-entry chunks.
    let leaves_sent = wire_leaves.len();
    chan.send(&Message::PushBundle {
        leaves: wire_leaves,
        blobs: Vec::new(),
    })?;

    let mut objects_sent = 0usize;
    let mut chunk: Vec<WireBlob> = Vec::new();
    for hash in object_hashes {
        let bytes = cas
            .get(hash)
            .map_err(|e| P2pError::Io(format!("cas read: {}", e)))?;
        chunk.push(WireBlob {
            hash_hex: hex::encode(hash.as_bytes()),
            data: bytes,
        });
        objects_sent += 1;
        if chunk.len() >= 64 {
            let take = std::mem::take(&mut chunk);
            chan.send(&Message::PushBundle {
                leaves: Vec::new(),
                blobs: take,
            })?;
        }
    }
    if !chunk.is_empty() {
        chan.send(&Message::PushBundle {
            leaves: Vec::new(),
            blobs: chunk,
        })?;
    }

    chan.send(&Message::PushDone { head_b3_hex })?;

    match chan.recv()? {
        Message::PushAccepted { landed_as } => Ok(PushSummary {
            landed_as,
            leaves_sent,
            objects_sent,
        }),
        Message::PushRejected { reason } => Err(P2pError::Protocol(reason)),
        Message::Error { message } => Err(P2pError::Protocol(message)),
        other => Err(P2pError::Protocol(format!(
            "unexpected reply to push: {:?}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests in this module touch the global `IVALDI_KNOWN_PEERS` env var
    /// so the TOFU enforcer doesn't write to the developer's real
    /// `~/.ivaldi/known_peers`. Cargo runs tests in parallel by default,
    /// so guard the env mutation + the TOFU-enforcing call with a
    /// process-wide mutex.
    fn tofu_guard() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static GATE: OnceLock<Mutex<()>> = OnceLock::new();
        GATE.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` with `IVALDI_KNOWN_PEERS` pointing at a fresh tempfile.
    /// `set_var` / `remove_var` are `unsafe` on edition 2024 because env
    /// is process-global; we serialize via `tofu_guard` so concurrent
    /// tests don't race, which is the documented soundness requirement.
    fn with_isolated_known_peers<R>(f: impl FnOnce() -> R) -> R {
        let _guard = tofu_guard();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let prev = std::env::var_os("IVALDI_KNOWN_PEERS");
        // SAFETY: serialized by `tofu_guard`; no other thread mutates
        // this env var concurrently for the duration of `f`.
        unsafe {
            std::env::set_var("IVALDI_KNOWN_PEERS", tmp.path());
        }
        let result = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("IVALDI_KNOWN_PEERS", v),
                None => std::env::remove_var("IVALDI_KNOWN_PEERS"),
            }
        }
        result
    }

    #[test]
    fn parse_url_with_default_port_and_no_timeline() {
        let u = PeerUrl::parse("ivaldi://example.com").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, DEFAULT_PORT);
        assert_eq!(u.timeline, None);
    }

    #[test]
    fn parse_url_with_port() {
        let u = PeerUrl::parse("ivaldi://10.0.0.1:9999").unwrap();
        assert_eq!(u.host, "10.0.0.1");
        assert_eq!(u.port, 9999);
        assert_eq!(u.timeline, None);
    }

    #[test]
    fn parse_url_with_timeline() {
        let u = PeerUrl::parse("ivaldi://example.com:9999/main").unwrap();
        assert_eq!(u.timeline.as_deref(), Some("main"));
    }

    #[test]
    fn parse_url_rejects_non_ivaldi_scheme() {
        assert!(PeerUrl::parse("https://example.com").is_none());
        assert!(PeerUrl::parse("example.com").is_none());
    }

    #[test]
    fn parse_url_rejects_empty_host() {
        assert!(PeerUrl::parse("ivaldi://").is_none());
        assert!(PeerUrl::parse("ivaldi:///main").is_none());
    }

    #[test]
    fn parse_url_rejects_bad_port() {
        assert!(PeerUrl::parse("ivaldi://h:notaport").is_none());
        assert!(PeerUrl::parse("ivaldi://h:99999").is_none()); // > u16
    }

    #[test]
    fn message_roundtrips_via_serde_json() {
        let m = Message::WantTimeline {
            timeline: "main".into(),
            have: vec!["abc".into(), "def".into()],
        };
        let j = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&j).unwrap();
        match back {
            Message::WantTimeline { timeline, have } => {
                assert_eq!(timeline, "main");
                assert_eq!(have, vec!["abc", "def"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn end_to_end_fetch_over_localhost() {
        // Set up a serving repo with one commit + one blob, then drop the
        // handle so the spawned server thread can open redb fresh.
        let server_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(server_dir.path()).unwrap();
        {
            let mut server_repo = crate::repo::Repo::open(server_dir.path()).unwrap();
            let cas = crate::cas::FileCas::new(server_dir.path().join(".ivaldi/objects")).unwrap();
            let store = crate::fsmerkle::FsStore::new(&cas);
            let (blob_hash, _) = store.put_blob(b"hello p2p").unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree_hash = store
                .put_tree(vec![Entry {
                    name: "greet.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob_hash,
                }])
                .unwrap();
            server_repo
                .commit(tree_hash, "tester <t@x>", "first p2p commit")
                .unwrap();
        }

        let server_id = Identity::generate().unwrap();
        let client_id = Identity::generate().unwrap();

        let peers_path = server_dir.path().join(".ivaldi/authorized_peers");
        let peer_store = PeerStore::new(peers_path);
        peer_store.trust(client_id.public, Some("client")).unwrap();

        // Bind to an ephemeral port.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Server thread: accept exactly one connection.
        let server_id_clone = server_id.clone();
        let server_root = server_dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut srv_repo = crate::repo::Repo::open(&server_root).unwrap();
            let store = PeerStore::new(server_root.join(".ivaldi/authorized_peers"));
            handle_connection(&mut srv_repo, stream, &server_id_clone, &store).unwrap();
        });

        // Client.
        let client_dir = tempfile::tempdir().unwrap();
        let target = client_dir.path().join("clone");
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}/main", port)).unwrap();
        let summary = with_isolated_known_peers(|| {
            fetch_into_with_policy(
                &url,
                &target,
                &client_id,
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        })
        .expect("fetch should succeed");
        assert_eq!(summary.timeline, "main");
        assert_eq!(summary.leaves_imported, 1);
        // Two objects ship: the file blob + the tree-node containing it.
        assert_eq!(summary.blobs_imported, 2);

        // Working tree contains the file with the right content.
        let materialized = std::fs::read_to_string(target.join("greet.txt")).unwrap();
        assert_eq!(materialized, "hello p2p");

        handle.join().unwrap();
    }

    #[test]
    fn untrusted_peer_is_rejected() {
        let server_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(server_dir.path()).unwrap();
        let server_id = Identity::generate().unwrap();
        let client_id = Identity::generate().unwrap();
        // Note: we deliberately do NOT trust the client.
        let peer_store = PeerStore::new(server_dir.path().join(".ivaldi/authorized_peers"));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_id_clone = server_id.clone();
        let server_root = server_dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut srv_repo = crate::repo::Repo::open(&server_root).unwrap();
            let store = PeerStore::new(server_root.join(".ivaldi/authorized_peers"));
            // Expect this to err with PeerNotAuthorized.
            let _ = handle_connection(&mut srv_repo, stream, &server_id_clone, &store);
        });

        let _ = peer_store; // keep alive to prove we never trusted

        let client_dir = tempfile::tempdir().unwrap();
        let target = client_dir.path().join("clone");
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}/main", port)).unwrap();
        let res = with_isolated_known_peers(|| {
            fetch_into_with_policy(
                &url,
                &target,
                &client_id,
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        });
        assert!(res.is_err(), "fetch should fail for untrusted peer");

        handle.join().unwrap();
    }

    /// Bob pushes his timeline to Alice; Alice's repo gains a
    /// `peers/bob/main` timeline whose tree matches Bob's tip.
    #[test]
    fn end_to_end_push_lands_under_peers_namespace() {
        // Alice's serve repo (empty).
        let alice_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(alice_dir.path()).unwrap();

        let alice_id = Identity::generate().unwrap();
        let bob_id = Identity::generate().unwrap();

        let peer_store = PeerStore::new(alice_dir.path().join(".ivaldi/authorized_peers"));
        peer_store.trust(bob_id.public, Some("bob")).unwrap();

        // Bob's source repo with a real commit.
        let bob_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(bob_dir.path()).unwrap();
        let bob_head_blake3;
        {
            let mut bob_repo = crate::repo::Repo::open(bob_dir.path()).unwrap();
            let cas = crate::cas::FileCas::new(bob_dir.path().join(".ivaldi/objects")).unwrap();
            let store = crate::fsmerkle::FsStore::new(&cas);
            let (blob_hash, _) = store.put_blob(b"bob's contribution").unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree_hash = store
                .put_tree(vec![Entry {
                    name: "bob.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob_hash,
                }])
                .unwrap();
            let r = bob_repo
                .commit(tree_hash, "bob <bob@x>", "from bob")
                .unwrap();
            bob_head_blake3 = r.hash;
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        // Share Alice's repo between the server thread and this test
        // thread (redb is single-handle per file).
        let alice_repo_arc = std::sync::Arc::new(std::sync::Mutex::new(
            crate::repo::Repo::open(alice_dir.path()).unwrap(),
        ));
        let alice_id_clone = alice_id.clone();
        let alice_root = alice_dir.path().to_path_buf();
        let alice_peer_path = alice_root.join(".ivaldi/authorized_peers");
        let alice_repo_for_server = alice_repo_arc.clone();
        let server_handle = std::thread::spawn(move || {
            let _ = serve_with_repo(
                &format!("127.0.0.1:{}", port),
                alice_repo_for_server,
                &alice_id_clone,
                alice_peer_path,
            );
        });
        // Wait for the listener to bind.
        for _ in 0..50 {
            if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Bob pushes.
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}", port)).unwrap();
        let summary = with_isolated_known_peers(|| {
            let mut bob_repo = crate::repo::Repo::open(bob_dir.path()).unwrap();
            push_to(
                &url,
                &mut bob_repo,
                &bob_id,
                "main",
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        })
        .expect("push should succeed");
        assert_eq!(summary.landed_as, "peers/bob/main");
        assert!(summary.leaves_sent >= 1);

        // Inspect Alice's shared repo handle. Wait briefly for the
        // server worker to release the mutex after replying PushAccepted.
        for _ in 0..50 {
            if let Ok(g) = alice_repo_arc.try_lock()
                && let Ok(Some(idx)) = g.get_timeline_head("peers/bob/main")
            {
                let leaf = g.get_leaf(idx).unwrap().unwrap();
                assert_eq!(leaf.hash(), bob_head_blake3);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        drop(server_handle);
        panic!("alice never observed peers/bob/main");
    }

    /// Helper: forge a repo and seal one commit containing `name` → `body`.
    /// Returns the commit's tree hash.
    fn seal_file(dir: &Path, name: &str, body: &[u8]) -> crate::hash::B3Hash {
        let mut repo = crate::repo::Repo::open(dir).unwrap();
        let cas = crate::cas::FileCas::new(dir.join(".ivaldi/objects")).unwrap();
        let store = crate::fsmerkle::FsStore::new(&cas);
        let (blob_hash, _) = store.put_blob(body).unwrap();
        use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
        let tree_hash = store
            .put_tree(vec![Entry {
                name: name.into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob_hash,
            }])
            .unwrap();
        repo.commit(tree_hash, "tester <t@x>", &format!("seal {}", name))
            .unwrap();
        tree_hash
    }

    /// The core remap case: the served timeline does NOT start at MMR index
    /// 0 on the server (other timelines occupy the low indices). The client
    /// must land the chain with parents rewritten into its own index space.
    #[test]
    fn fetch_remaps_parent_indices_when_server_indices_are_shifted() {
        let server_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(server_dir.path()).unwrap();
        {
            let mut repo = crate::repo::Repo::open(server_dir.path()).unwrap();
            let cas = crate::cas::FileCas::new(server_dir.path().join(".ivaldi/objects")).unwrap();
            let store = crate::fsmerkle::FsStore::new(&cas);
            let (blob, _) = store.put_blob(b"scratch").unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let scratch_tree = store
                .put_tree(vec![Entry {
                    name: "s.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob,
                }])
                .unwrap();
            // Two throwaway leaves on an unrelated timeline occupy indices 0-1.
            let mut l0 = crate::leaf::Leaf::new(scratch_tree, "scratch", "t <t@x>", 1, "s0");
            l0.prev_idx = crate::leaf::NO_PARENT;
            let r0 = repo.commit_raw(l0, "scratch").unwrap();
            assert_eq!(r0.index, 0);
            let mut l1 = crate::leaf::Leaf::new(scratch_tree, "scratch", "t <t@x>", 2, "s1");
            l1.prev_idx = 0;
            repo.commit_raw(l1, "scratch").unwrap();
        }
        // "main" now starts at server index 2.
        seal_file(server_dir.path(), "one.txt", b"first");
        seal_file(server_dir.path(), "two.txt", b"second");
        {
            let repo = crate::repo::Repo::open(server_dir.path()).unwrap();
            assert_eq!(repo.get_timeline_head("main").unwrap(), Some(3));
        }

        let server_id = Identity::generate().unwrap();
        let client_id = Identity::generate().unwrap();
        let peer_store = PeerStore::new(server_dir.path().join(".ivaldi/authorized_peers"));
        peer_store.trust(client_id.public, Some("client")).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_id_clone = server_id.clone();
        let server_root = server_dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut srv_repo = crate::repo::Repo::open(&server_root).unwrap();
            let store = PeerStore::new(server_root.join(".ivaldi/authorized_peers"));
            handle_connection(&mut srv_repo, stream, &server_id_clone, &store).unwrap();
        });

        let client_dir = tempfile::tempdir().unwrap();
        let target = client_dir.path().join("clone");
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}/main", port)).unwrap();
        let summary = with_isolated_known_peers(|| {
            fetch_into_with_policy(
                &url,
                &target,
                &client_id,
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        })
        .expect("fetch should succeed");
        assert_eq!(summary.leaves_imported, 2);

        // Client chain must be self-consistent: head at local index 1 with
        // prev pointing at local index 0, which is a root.
        let client_repo = crate::repo::Repo::open(&target).unwrap();
        let head = client_repo.get_timeline_head("main").unwrap().unwrap();
        assert_eq!(
            head, 1,
            "client indices must start at 0, not mirror the server's"
        );
        let head_leaf = client_repo.get_leaf(head).unwrap().unwrap();
        assert_eq!(head_leaf.prev_idx, 0);
        let root_leaf = client_repo.get_leaf(0).unwrap().unwrap();
        assert_eq!(root_leaf.prev_idx, crate::leaf::NO_PARENT);
        assert_eq!(head_leaf.message, "seal two.txt");

        // Workspace materialized from the true head.
        assert_eq!(
            std::fs::read_to_string(target.join("two.txt")).unwrap(),
            "second"
        );

        // Full integrity check on the clone (drop the handle first — redb is
        // single-open per file).
        drop(client_repo);
        let report = crate::verify::verify(&target, true);
        assert!(
            report.ok,
            "verify --full failed on clone: {:?}",
            report.checks
        );

        handle.join().unwrap();
    }

    /// Pushing into a recipient that already has history must remap the
    /// pushed chain's parents, never graft it onto the recipient's leaves.
    /// A second identical push must be an idempotent no-op.
    #[test]
    fn push_into_repo_with_existing_history_remaps_and_is_idempotent() {
        // Alice already has two seals on main (indices 0-1).
        let alice_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(alice_dir.path()).unwrap();
        seal_file(alice_dir.path(), "alice1.txt", b"a1");
        seal_file(alice_dir.path(), "alice2.txt", b"a2");

        let alice_id = Identity::generate().unwrap();
        let bob_id = Identity::generate().unwrap();
        let peer_store = PeerStore::new(alice_dir.path().join(".ivaldi/authorized_peers"));
        peer_store.trust(bob_id.public, Some("bob")).unwrap();

        // Bob has a two-seal chain at indices 0-1 in HIS repo.
        let bob_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(bob_dir.path()).unwrap();
        seal_file(bob_dir.path(), "bob1.txt", b"b1");
        let bob_tip_tree = seal_file(bob_dir.path(), "bob2.txt", b"b2");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let alice_repo_arc = std::sync::Arc::new(std::sync::Mutex::new(
            crate::repo::Repo::open(alice_dir.path()).unwrap(),
        ));
        let alice_id_clone = alice_id.clone();
        let alice_peer_path = alice_dir.path().join(".ivaldi/authorized_peers");
        let alice_repo_for_server = alice_repo_arc.clone();
        let _server = std::thread::spawn(move || {
            let _ = serve_with_repo(
                &format!("127.0.0.1:{}", port),
                alice_repo_for_server,
                &alice_id_clone,
                alice_peer_path,
            );
        });
        for _ in 0..50 {
            if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}", port)).unwrap();
        let push_once = |ids: &Identity| {
            let mut bob_repo = crate::repo::Repo::open(bob_dir.path()).unwrap();
            push_to(
                &url,
                &mut bob_repo,
                ids,
                "main",
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        };
        let summary =
            with_isolated_known_peers(|| push_once(&bob_id)).expect("first push should succeed");
        assert_eq!(summary.landed_as, "peers/bob/main");

        // Wait for the worker to land it, then check the remapped chain.
        let check = |expected_total: u64| {
            for _ in 0..100 {
                if let Ok(g) = alice_repo_arc.try_lock()
                    && let Ok(Some(head)) = g.get_timeline_head("peers/bob/main")
                {
                    // Bob's leaves must land at indices 2-3 (after
                    // Alice's), with parents remapped accordingly.
                    assert_eq!(head, 3);
                    let head_leaf = g.get_leaf(head).unwrap().unwrap();
                    assert_eq!(head_leaf.prev_idx, 2);
                    assert_eq!(head_leaf.tree_root, bob_tip_tree);
                    let first = g.get_leaf(2).unwrap().unwrap();
                    assert_eq!(first.prev_idx, crate::leaf::NO_PARENT);
                    assert_eq!(g.commit_count(), expected_total);
                    // Alice's own history is untouched.
                    assert_eq!(g.get_timeline_head("main").unwrap(), Some(1));
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            panic!("alice never observed peers/bob/main");
        };
        check(4);

        // Idempotent re-push: no new leaves, still accepted.
        let summary2 =
            with_isolated_known_peers(|| push_once(&bob_id)).expect("re-push should succeed");
        assert_eq!(summary2.landed_as, "peers/bob/main");
        check(4);
        // (No verify --full here: the serve thread keeps Alice's redb handle
        // open for the process lifetime, and redb is single-open per file.)
    }

    /// A wire leaf whose parent index was never transferred must be rejected
    /// outright — landing it would graft onto unrelated local history.
    #[test]
    fn lander_rejects_parent_index_that_was_never_transferred() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        // Recipient has one unrelated seal at index 0 — the graft target.
        seal_file(dir.path(), "local.txt", b"local");
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let before = repo.commit_count();

        // Attacker/sender leaf claiming parent at sender index 0, but leaf 0
        // was never part of the transfer.
        let mut evil = crate::leaf::Leaf::new(
            crate::hash::B3Hash::digest(b"tree"),
            "main",
            "evil <e@x>",
            0,
            "graft me",
        );
        evil.prev_idx = 0;
        let wl = WireLeaf {
            canonical: evil.canonical_bytes(),
            sender_idx: 5,
        };

        let mut lander = LeafLander::default();
        let err = lander
            .land(&mut repo, "peers/evil/main", &wl)
            .expect_err("unmapped parent must be rejected");
        assert!(err.to_string().contains("never transferred"), "{}", err);
        // Old state intact: nothing landed.
        assert_eq!(repo.commit_count(), before);
        assert!(repo.get_timeline_head("peers/evil/main").unwrap().is_none());
    }

    /// A leaf sent without a sender index (old peer) is refused, not guessed.
    #[test]
    fn lander_rejects_leaf_without_sender_idx() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let leaf =
            crate::leaf::Leaf::new(crate::hash::B3Hash::digest(b"t"), "main", "a <a@x>", 0, "m");
        let wl = WireLeaf {
            canonical: leaf.canonical_bytes(),
            sender_idx: u64::MAX,
        };
        let mut lander = LeafLander::default();
        assert!(lander.land(&mut repo, "main", &wl).is_err());
        assert_eq!(repo.commit_count(), 0);
    }

    /// A scripted "server" that reports a tip which never arrived: the fetch
    /// must fail and the half-forged target directory must be removed so an
    /// immediate retry is possible.
    #[test]
    fn fetch_rejects_wrong_done_head_and_cleans_up_target() {
        let server_id = Identity::generate().unwrap();
        let client_id = Identity::generate().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_id_clone = server_id.clone();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut chan = Channel::accept(stream, &server_id_clone).unwrap();
            // Expect WantTimeline (the URL pins /main).
            match chan.recv().unwrap() {
                Message::WantTimeline { .. } => {}
                other => panic!("unexpected: {:?}", other),
            }
            let leaf = crate::leaf::Leaf::new(
                crate::hash::B3Hash::digest(b"tree"),
                "main",
                "srv <s@x>",
                0,
                "only leaf",
            );
            chan.send(&Message::Bundle {
                leaves: vec![WireLeaf {
                    canonical: leaf.canonical_bytes(),
                    sender_idx: 0,
                }],
                blobs: Vec::new(),
            })
            .unwrap();
            // Claim a tip that was never sent.
            chan.send(&Message::Done {
                head_b3_hex: "00".repeat(32),
            })
            .unwrap();
            // Keep the connection open until the client is done.
            let _ = chan.recv();
        });

        let client_dir = tempfile::tempdir().unwrap();
        let target = client_dir.path().join("clone");
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}/main", port)).unwrap();
        let res = with_isolated_known_peers(|| {
            fetch_into_with_policy(
                &url,
                &target,
                &client_id,
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        });
        let err = res.expect_err("fetch must fail on tip mismatch");
        assert!(err.to_string().contains("never arrived"), "{}", err);
        assert!(
            !target.exists(),
            "half-forged target must be removed on failure"
        );
        handle.join().unwrap();
    }

    /// A stream that dies before `Done` must fail the fetch and remove the
    /// target directory (no debris tripping the retry guard).
    #[test]
    fn fetch_cleans_up_target_on_truncated_stream() {
        let server_id = Identity::generate().unwrap();
        let client_id = Identity::generate().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_id_clone = server_id.clone();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut chan = Channel::accept(stream, &server_id_clone).unwrap();
            match chan.recv().unwrap() {
                Message::WantTimeline { .. } => {}
                other => panic!("unexpected: {:?}", other),
            }
            let leaf = crate::leaf::Leaf::new(
                crate::hash::B3Hash::digest(b"tree"),
                "main",
                "srv <s@x>",
                0,
                "only leaf",
            );
            chan.send(&Message::Bundle {
                leaves: vec![WireLeaf {
                    canonical: leaf.canonical_bytes(),
                    sender_idx: 0,
                }],
                blobs: Vec::new(),
            })
            .unwrap();
            // Drop the connection before Done.
            chan.shutdown();
        });

        let client_dir = tempfile::tempdir().unwrap();
        let target = client_dir.path().join("clone");
        let url = PeerUrl::parse(&format!("ivaldi://127.0.0.1:{}/main", port)).unwrap();
        let res = with_isolated_known_peers(|| {
            fetch_into_with_policy(
                &url,
                &target,
                &client_id,
                crate::known_peers::TofuPolicy::AcceptAll,
            )
        });
        assert!(res.is_err(), "fetch must fail on truncated stream");
        assert!(
            !target.exists(),
            "half-forged target must be removed on failure"
        );
        handle.join().unwrap();
    }

    /// Two clients hit the threaded `serve` simultaneously and both must
    /// receive correct data. Validates Slice B's thread-per-connection
    /// model (fresh `Repo` per worker, no cross-thread state sharing).
    #[test]
    fn serve_handles_concurrent_clients() {
        let server_dir = tempfile::tempdir().unwrap();
        crate::forge::forge(server_dir.path()).unwrap();
        {
            let mut server_repo = crate::repo::Repo::open(server_dir.path()).unwrap();
            let cas = crate::cas::FileCas::new(server_dir.path().join(".ivaldi/objects")).unwrap();
            let store = crate::fsmerkle::FsStore::new(&cas);
            let (blob_hash, _) = store.put_blob(b"concurrent body").unwrap();
            use crate::fsmerkle::{Entry, MODE_FILE, NodeKind};
            let tree_hash = store
                .put_tree(vec![Entry {
                    name: "shared.txt".into(),
                    mode: MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: blob_hash,
                }])
                .unwrap();
            server_repo
                .commit(tree_hash, "tester <t@x>", "shared commit")
                .unwrap();
        }

        let server_id = Identity::generate().unwrap();
        let alice_id = Identity::generate().unwrap();
        let bob_id = Identity::generate().unwrap();

        let peer_store = PeerStore::new(server_dir.path().join(".ivaldi/authorized_peers"));
        peer_store.trust(alice_id.public, Some("alice")).unwrap();
        peer_store.trust(bob_id.public, Some("bob")).unwrap();

        // Spin up the real threaded serve loop on an ephemeral port.
        // We fire it on a background thread and tear it down by killing
        // the process when the test exits — `serve` blocks forever, so
        // we can't join it cleanly. The test passes as soon as both
        // client fetches complete.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // release port; serve() rebinds it

        let server_root = server_dir.path().to_path_buf();
        let peer_path = server_root.join(".ivaldi/authorized_peers");
        let shared_repo = std::sync::Arc::new(std::sync::Mutex::new(
            crate::repo::Repo::open(&server_root).unwrap(),
        ));
        let server_id_clone = server_id.clone();
        let shared_repo_for_server = shared_repo.clone();
        let server_handle = std::thread::spawn(move || {
            let _ = serve_with_repo(
                &format!("127.0.0.1:{}", port),
                shared_repo_for_server,
                &server_id_clone,
                peer_path,
            );
        });

        // Give the listener a beat to bind. (Race-free alternative would
        // be to wire a "ready" channel in serve, but that's bigger churn
        // than this test warrants.)
        for _ in 0..50 {
            if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let url_str = format!("ivaldi://127.0.0.1:{}/main", port);
        let alice_dir = tempfile::tempdir().unwrap();
        let bob_dir = tempfile::tempdir().unwrap();
        let alice_target = alice_dir.path().join("clone-a");
        let bob_target = bob_dir.path().join("clone-b");

        let alice_url = url_str.clone();
        let bob_url = url_str.clone();
        let (alice_res, bob_res) = with_isolated_known_peers(|| {
            let policy = crate::known_peers::TofuPolicy::AcceptAll;
            let alice_t = std::thread::spawn(move || {
                let url = PeerUrl::parse(&alice_url).unwrap();
                fetch_into_with_policy(&url, &alice_target, &alice_id, policy)
            });
            let bob_t = std::thread::spawn(move || {
                let url = PeerUrl::parse(&bob_url).unwrap();
                fetch_into_with_policy(&url, &bob_target, &bob_id, policy)
            });
            (alice_t.join().unwrap(), bob_t.join().unwrap())
        });
        assert!(alice_res.is_ok(), "alice fetch failed: {:?}", alice_res);
        assert!(bob_res.is_ok(), "bob fetch failed: {:?}", bob_res);

        let alice_content =
            std::fs::read_to_string(alice_dir.path().join("clone-a/shared.txt")).unwrap();
        let bob_content =
            std::fs::read_to_string(bob_dir.path().join("clone-b/shared.txt")).unwrap();
        assert_eq!(alice_content, "concurrent body");
        assert_eq!(bob_content, "concurrent body");

        // Leave the serve thread running — it will be torn down when the
        // test process exits. Cleaner shutdown is a follow-up.
        drop(server_handle);
    }
}
