//! Build a git-compatible packfile (v2, no deltas) for `git-receive-pack`.
//!
//! Format (per git's pack-format docs):
//!
//! ```text
//!   "PACK"            4 bytes  magic
//!   0x0000_0002       4 bytes  version (BE)
//!   0xNN_NN_NN_NN     4 bytes  object count (BE)
//!   {  per object:
//!        var-len header   (type:3 + size, with continuation)
//!        zlib-deflated body
//!   }
//!   <SHA-1 of all preceding bytes>     20 bytes
//! ```
//!
//! V1 ships every object as a base (no deltas). Receivers happily index a
//! delta-free pack — it's just larger over the wire. Adding deltas later
//! is a perf-only follow-up; correctness doesn't change.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use sha1::{Digest, Sha1};

use crate::git_export::GitObject;
use crate::git_remote::GitObjectKind;

/// Encode an object set into a git packfile. Caller streams the resulting
/// bytes into `git-receive-pack`'s stdin after the command pkt-lines + flush.
pub fn write_pack(objects: &[&GitObject]) -> Result<Vec<u8>, PackWriteError> {
    let mut hasher = Sha1::new();
    let mut buf: Vec<u8> = Vec::new();

    // Header.
    write_all(&mut buf, &mut hasher, b"PACK");
    write_all(&mut buf, &mut hasher, &2u32.to_be_bytes());
    write_all(&mut buf, &mut hasher, &(objects.len() as u32).to_be_bytes());

    for obj in objects {
        let header = encode_object_header(obj.kind, obj.body.len());
        write_all(&mut buf, &mut hasher, &header);

        // Each body is zlib-compressed independently.
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&obj.body)
            .map_err(|e| PackWriteError::Other(e.to_string()))?;
        let compressed = enc
            .finish()
            .map_err(|e| PackWriteError::Other(e.to_string()))?;
        write_all(&mut buf, &mut hasher, &compressed);
    }

    let trailer = hasher.finalize();
    buf.extend_from_slice(&trailer);
    Ok(buf)
}

/// Write `data` both to `out` and to `hasher` (the trailer SHA-1 covers
/// everything up to but not including itself).
fn write_all(out: &mut Vec<u8>, hasher: &mut Sha1, data: &[u8]) {
    hasher.update(data);
    out.extend_from_slice(data);
}

/// Encode the per-object header. First byte: bit 7 = continuation, bits
/// 6-4 = type, bits 3-0 = low 4 size bits. Subsequent bytes: bit 7 =
/// continuation, bits 6-0 = next 7 size bits.
fn encode_object_header(kind: GitObjectKind, size: usize) -> Vec<u8> {
    let type_code: u8 = match kind {
        GitObjectKind::Commit => 1,
        GitObjectKind::Tree => 2,
        GitObjectKind::Blob => 3,
        GitObjectKind::Tag => 4,
    };
    let mut size = size;
    let mut first = (type_code << 4) | ((size & 0x0f) as u8);
    size >>= 4;
    let mut out = Vec::new();
    if size > 0 {
        first |= 0x80;
    }
    out.push(first);
    while size > 0 {
        let mut byte = (size & 0x7f) as u8;
        size >>= 7;
        if size > 0 {
            byte |= 0x80;
        }
        out.push(byte);
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum PackWriteError {
    #[error("git pack write: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tiny pack and feed it back through Ivaldi's existing git
    /// pack *parser* (`crate::git_remote::parse_packfile`) to confirm we
    /// emit a structurally valid v2 pack. parse_packfile already handles
    /// header, per-object framing, and zlib decoding.
    #[test]
    fn round_trip_through_parser() {
        // Build a single blob "hello".
        let body = b"hello".to_vec();
        // Compute its git SHA-1 via the existing helper.
        let sha1_hex = crate::git_remote::git_object_id(GitObjectKind::Blob, &body);
        let mut sha1 = [0u8; 20];
        sha1.copy_from_slice(&hex::decode(&sha1_hex).unwrap());

        let obj = GitObject {
            sha1,
            kind: GitObjectKind::Blob,
            body: body.clone(),
        };
        let pack = write_pack(&[&obj]).unwrap();

        // Sanity: starts with PACK magic, has trailer of right length.
        assert_eq!(&pack[..4], b"PACK");
        assert!(pack.len() > 12 + 20);

        // Parser should round-trip.
        let parsed = crate::git_remote::parse_packfile(&pack).unwrap();
        assert!(parsed.contains_key(&sha1_hex));
        assert_eq!(parsed[&sha1_hex].data, body);
    }

    #[test]
    fn header_encoding_round_trips_for_small_size() {
        // Size fits in 4 bits → single-byte header.
        let h = encode_object_header(GitObjectKind::Blob, 5);
        assert_eq!(h.len(), 1);
        // Type=3 (blob), size=5: 0b0011_0101 = 0x35
        assert_eq!(h[0], 0x35);
    }

    #[test]
    fn header_encoding_for_larger_size_uses_continuation() {
        let h = encode_object_header(GitObjectKind::Tree, 200);
        // First byte type=2 << 4 = 0x20; size lo nibble = 200 & 0x0f = 8
        // Continuation bit set because size >> 4 = 12 > 0.
        assert_eq!(h[0], 0xa8);
        // Second byte: 12 (no continuation, no high bit)
        assert_eq!(h[1], 0x0c);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn empty_pack_has_just_header_plus_trailer() {
        let pack = write_pack(&[]).unwrap();
        assert_eq!(pack.len(), 12 + 20);
        assert_eq!(&pack[..4], b"PACK");
    }
}
