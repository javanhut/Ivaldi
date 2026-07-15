//! Protobuf wire encoding for the `ivaldi://` protocol (v2).
//!
//! Payloads on the Noise channel are protobuf-encoded [`Envelope`] values,
//! defined with prost derives — pure Rust, no protoc or build.rs. The field
//! numbers below ARE the wire contract: never renumber or reuse a tag, only
//! append. A `.proto` schema for cross-language peers can be transcribed
//! from these definitions when one is needed.
//!
//! Version negotiation: the Noise prologue is `b"ivaldi/2"`, so a peer
//! speaking protocol v1 (JSON frames) cannot even complete the handshake —
//! there is no way to confuse the two encodings. Within the protobuf era,
//! both sides exchange `Hello { version }` immediately after the handshake
//! and refuse a mismatch explicitly; unknown protobuf fields are ignored by
//! prost, so additive v2.x changes stay compatible.

use crate::p2p::{Message, WireBlob, WireLeaf};
use prost::Message as _;

/// Wire protocol version carried in `Hello`. Bump on breaking changes
/// (also bump the Noise prologue in `p2p.rs` when the framing itself changes).
pub const PROTOCOL_VERSION: u32 = 2;

/// Sentinel for a leaf that arrived without its sender index. Landing such
/// a leaf fails loudly instead of guessing at its lineage.
pub const MISSING_SENDER_IDX: u64 = u64::MAX;

#[derive(Clone, PartialEq, prost::Message)]
pub struct Envelope {
    #[prost(
        oneof = "envelope::Msg",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13"
    )]
    pub msg: Option<envelope::Msg>,
}

pub mod envelope {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Msg {
        #[prost(message, tag = "1")]
        Hello(super::Hello),
        #[prost(message, tag = "2")]
        ListTimelines(super::ListTimelines),
        #[prost(message, tag = "3")]
        Timelines(super::Timelines),
        #[prost(message, tag = "4")]
        WantTimeline(super::WantTimeline),
        #[prost(message, tag = "5")]
        Bundle(super::Bundle),
        #[prost(message, tag = "6")]
        Done(super::Done),
        #[prost(message, tag = "7")]
        PushStart(super::PushStart),
        #[prost(message, tag = "8")]
        PushBundle(super::Bundle),
        #[prost(message, tag = "9")]
        PushDone(super::Done),
        #[prost(message, tag = "10")]
        PushAccepted(super::PushAccepted),
        #[prost(message, tag = "11")]
        PushRejected(super::PushRejected),
        #[prost(message, tag = "12")]
        Error(super::ProtocolError),
        #[prost(message, tag = "13")]
        BlobChunk(super::BlobChunk),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Hello {
    #[prost(uint32, tag = "1")]
    pub version: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ListTimelines {}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Timelines {
    #[prost(string, repeated, tag = "1")]
    pub names: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WantTimeline {
    #[prost(string, tag = "1")]
    pub timeline: String,
    #[prost(string, repeated, tag = "2")]
    pub have: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PbWireLeaf {
    #[prost(bytes = "vec", tag = "1")]
    pub canonical: Vec<u8>,
    /// Sender-local MMR index. `optional` so "absent" is distinguishable
    /// from index 0 — an absent index is refused at landing, never guessed.
    #[prost(uint64, optional, tag = "2")]
    pub sender_idx: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PbWireBlob {
    #[prost(string, tag = "1")]
    pub hash_hex: String,
    #[prost(bytes = "vec", tag = "2")]
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Bundle {
    #[prost(message, repeated, tag = "1")]
    pub leaves: Vec<PbWireLeaf>,
    #[prost(message, repeated, tag = "2")]
    pub blobs: Vec<PbWireBlob>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Done {
    #[prost(string, tag = "1")]
    pub head_b3_hex: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PushStart {
    #[prost(string, tag = "1")]
    pub timeline: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PushAccepted {
    #[prost(string, tag = "1")]
    pub landed_as: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PushRejected {
    #[prost(string, tag = "1")]
    pub reason: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtocolError {
    #[prost(string, tag = "1")]
    pub message: String,
}

/// One slice of a large object streamed outside the inline-bundle path.
/// Chunks for one object are contiguous and never interleaved with chunks
/// of another object; the receiver enforces both (see `BlobAssembler`).
#[derive(Clone, PartialEq, prost::Message)]
pub struct BlobChunk {
    #[prost(string, tag = "1")]
    pub hash_hex: String,
    #[prost(uint64, tag = "2")]
    pub total_len: u64,
    #[prost(uint64, tag = "3")]
    pub offset: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub data: Vec<u8>,
}

fn pb_leaf(l: &WireLeaf) -> PbWireLeaf {
    PbWireLeaf {
        canonical: l.canonical.clone(),
        sender_idx: (l.sender_idx != MISSING_SENDER_IDX).then_some(l.sender_idx),
    }
}

fn leaf_from_pb(l: PbWireLeaf) -> WireLeaf {
    WireLeaf {
        canonical: l.canonical,
        sender_idx: l.sender_idx.unwrap_or(MISSING_SENDER_IDX),
    }
}

fn pb_blob(b: &WireBlob) -> PbWireBlob {
    PbWireBlob {
        hash_hex: b.hash_hex.clone(),
        data: b.data.clone(),
    }
}

fn blob_from_pb(b: PbWireBlob) -> WireBlob {
    WireBlob {
        hash_hex: b.hash_hex,
        data: b.data,
    }
}

fn pb_bundle(leaves: &[WireLeaf], blobs: &[WireBlob]) -> Bundle {
    Bundle {
        leaves: leaves.iter().map(pb_leaf).collect(),
        blobs: blobs.iter().map(pb_blob).collect(),
    }
}

/// Encode a protocol message to protobuf bytes.
pub fn encode(msg: &Message) -> Vec<u8> {
    use envelope::Msg as M;
    let m = match msg {
        Message::Hello { version } => M::Hello(Hello { version: *version }),
        Message::ListTimelines => M::ListTimelines(ListTimelines {}),
        Message::Timelines { names } => M::Timelines(Timelines {
            names: names.clone(),
        }),
        Message::WantTimeline { timeline, have } => M::WantTimeline(WantTimeline {
            timeline: timeline.clone(),
            have: have.clone(),
        }),
        Message::Bundle { leaves, blobs } => M::Bundle(pb_bundle(leaves, blobs)),
        Message::Done { head_b3_hex } => M::Done(Done {
            head_b3_hex: head_b3_hex.clone(),
        }),
        Message::PushStart { timeline } => M::PushStart(PushStart {
            timeline: timeline.clone(),
        }),
        Message::PushBundle { leaves, blobs } => M::PushBundle(pb_bundle(leaves, blobs)),
        Message::PushDone { head_b3_hex } => M::PushDone(Done {
            head_b3_hex: head_b3_hex.clone(),
        }),
        Message::PushAccepted { landed_as } => M::PushAccepted(PushAccepted {
            landed_as: landed_as.clone(),
        }),
        Message::PushRejected { reason } => M::PushRejected(PushRejected {
            reason: reason.clone(),
        }),
        Message::Error { message } => M::Error(ProtocolError {
            message: message.clone(),
        }),
        Message::BlobChunk {
            hash_hex,
            total_len,
            offset,
            data,
        } => M::BlobChunk(BlobChunk {
            hash_hex: hash_hex.clone(),
            total_len: *total_len,
            offset: *offset,
            data: data.clone(),
        }),
    };
    Envelope { msg: Some(m) }.encode_to_vec()
}

/// Decode protobuf bytes into a protocol message. Fails on malformed bytes
/// or an empty envelope (e.g. a message kind newer than this binary).
pub fn decode(bytes: &[u8]) -> Result<Message, String> {
    use envelope::Msg as M;
    let env = Envelope::decode(bytes).map_err(|e| e.to_string())?;
    let m = env
        .msg
        .ok_or("empty envelope — peer sent a message kind this ivaldi does not know")?;
    Ok(match m {
        M::Hello(v) => Message::Hello { version: v.version },
        M::ListTimelines(_) => Message::ListTimelines,
        M::Timelines(v) => Message::Timelines { names: v.names },
        M::WantTimeline(v) => Message::WantTimeline {
            timeline: v.timeline,
            have: v.have,
        },
        M::Bundle(v) => Message::Bundle {
            leaves: v.leaves.into_iter().map(leaf_from_pb).collect(),
            blobs: v.blobs.into_iter().map(blob_from_pb).collect(),
        },
        M::Done(v) => Message::Done {
            head_b3_hex: v.head_b3_hex,
        },
        M::PushStart(v) => Message::PushStart {
            timeline: v.timeline,
        },
        M::PushBundle(v) => Message::PushBundle {
            leaves: v.leaves.into_iter().map(leaf_from_pb).collect(),
            blobs: v.blobs.into_iter().map(blob_from_pb).collect(),
        },
        M::PushDone(v) => Message::PushDone {
            head_b3_hex: v.head_b3_hex,
        },
        M::PushAccepted(v) => Message::PushAccepted {
            landed_as: v.landed_as,
        },
        M::PushRejected(v) => Message::PushRejected { reason: v.reason },
        M::Error(v) => Message::Error { message: v.message },
        M::BlobChunk(v) => Message::BlobChunk {
            hash_hex: v.hash_hex,
            total_len: v.total_len,
            offset: v.offset,
            data: v.data,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_kind_roundtrips() {
        let leaf = WireLeaf {
            canonical: vec![1, 2, 3],
            sender_idx: 0, // index 0 must survive (optional field, not default-elision)
        };
        let blob = WireBlob {
            hash_hex: "ab".repeat(32),
            data: vec![9; 100],
        };
        let messages = vec![
            Message::Hello { version: 2 },
            Message::ListTimelines,
            Message::Timelines {
                names: vec!["main".into(), "feature/x".into()],
            },
            Message::WantTimeline {
                timeline: "main".into(),
                have: vec!["aa".repeat(32)],
            },
            Message::Bundle {
                leaves: vec![leaf.clone()],
                blobs: vec![blob.clone()],
            },
            Message::Done {
                head_b3_hex: "cd".repeat(32),
            },
            Message::PushStart {
                timeline: "main".into(),
            },
            Message::PushBundle {
                leaves: vec![leaf],
                blobs: vec![blob],
            },
            Message::PushDone {
                head_b3_hex: "cd".repeat(32),
            },
            Message::PushAccepted {
                landed_as: "peers/x/main".into(),
            },
            Message::PushRejected {
                reason: "nope".into(),
            },
            Message::Error {
                message: "boom".into(),
            },
            Message::BlobChunk {
                hash_hex: "ef".repeat(32),
                total_len: 10,
                offset: 4,
                data: vec![0; 6],
            },
        ];
        for m in messages {
            let bytes = encode(&m);
            let back = decode(&bytes).unwrap();
            assert_eq!(format!("{:?}", m), format!("{:?}", back));
        }
    }

    #[test]
    fn missing_sender_idx_maps_to_sentinel() {
        let bytes = encode(&Message::Bundle {
            leaves: vec![WireLeaf {
                canonical: vec![1],
                sender_idx: MISSING_SENDER_IDX,
            }],
            blobs: vec![],
        });
        match decode(&bytes).unwrap() {
            Message::Bundle { leaves, .. } => {
                assert_eq!(leaves[0].sender_idx, MISSING_SENDER_IDX)
            }
            other => panic!("wrong kind: {:?}", other),
        }
    }

    #[test]
    fn malformed_bytes_error_without_panic() {
        assert!(decode(&[0xff; 64]).is_err());
        assert!(decode(b"not protobuf at all").is_err());
    }

    #[test]
    fn unknown_message_kind_is_an_explicit_error() {
        // An envelope whose oneof tag is unknown to us decodes to msg=None.
        // Tag 200, wire type 2 (length-delimited), empty payload.
        let mut bytes = Vec::new();
        prost::encoding::encode_key(200, prost::encoding::WireType::LengthDelimited, &mut bytes);
        prost::encoding::encode_varint(0, &mut bytes);
        let err = decode(&bytes).unwrap_err();
        assert!(err.contains("does not know"), "{err}");
    }
}
