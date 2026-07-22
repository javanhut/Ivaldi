//! Bounded, `Result`-based reader for decoding untrusted bytes.
//!
//! Every read is length-checked, so a truncated or malformed buffer yields a
//! typed [`ReadError`] instead of a slice-index panic or a silent partial
//! decode. Varints are bounded to their 64-bit maximum, so a hostile stream
//! cannot spin or overflow. Length prefixes are never used to pre-allocate,
//! so a bogus count cannot trigger an out-of-memory abort — the reader simply
//! runs out of input and errors.
//!
//! This is the single place on-disk and network decoders do bounds checking;
//! parsers built on it never index a raw slice.

/// Error from decoding a byte buffer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadError {
    #[error("unexpected end of input at offset {offset}: needed {needed} more byte(s)")]
    UnexpectedEof { offset: usize, needed: usize },

    #[error("varint at offset {offset} overflows 64 bits")]
    VarintOverflow { offset: usize },

    #[error("invalid UTF-8 in {field}")]
    Utf8 { field: &'static str },

    #[error("{0} trailing byte(s) after decode")]
    TrailingData(usize),
}

/// A forward-only cursor over a byte slice with bounds-checked reads.
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Take exactly `n` bytes, advancing the cursor. Errors (never panics) if
    /// fewer than `n` remain, and cannot overflow on the offset arithmetic.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
        let end = self.pos.checked_add(n).ok_or(ReadError::UnexpectedEof {
            offset: self.pos,
            needed: n,
        })?;
        if end > self.data.len() {
            return Err(ReadError::UnexpectedEof {
                offset: self.pos,
                needed: end - self.data.len(),
            });
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Read a single byte.
    pub fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.take(1)?[0])
    }

    /// Read a fixed-size array (e.g. a 32-byte hash).
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let slice = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    /// Read an unsigned LEB128 varint, bounded to 64 bits (max 10 bytes).
    pub fn uvarint(&mut self) -> Result<u64, ReadError> {
        let start = self.pos;
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            // Reject a continuation past 64 bits, or a final byte whose high
            // bits would not fit — both are malformed, not just truncated.
            if shift >= 64 || (shift == 63 && byte > 0x01) {
                return Err(ReadError::VarintOverflow { offset: start });
            }
            value |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    /// Read a signed LEB128 varint (zigzag-decoded).
    pub fn varint(&mut self) -> Result<i64, ReadError> {
        let encoded = self.uvarint()?;
        Ok(((encoded >> 1) as i64) ^ (-((encoded & 1) as i64)))
    }

    /// Read a uvarint length prefix followed by that many bytes, as a UTF-8
    /// string. `field` names the field for the error message.
    pub fn string(&mut self, field: &'static str) -> Result<String, ReadError> {
        let len = self.uvarint()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ReadError::Utf8 { field })
    }

    /// Assert the whole buffer was consumed. Trailing bytes indicate corruption
    /// in a fixed-size canonical encoding.
    pub fn finish(self) -> Result<(), ReadError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(ReadError::TrailingData(self.data.len() - self.pos))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_is_bounded() {
        let mut r = ByteReader::new(&[1, 2, 3]);
        assert_eq!(r.take(2).unwrap(), &[1, 2]);
        assert_eq!(
            r.take(2),
            Err(ReadError::UnexpectedEof {
                offset: 2,
                needed: 1
            })
        );
    }

    #[test]
    fn take_does_not_overflow() {
        // A huge length must error, never wrap and panic.
        let mut r = ByteReader::new(&[0u8; 4]);
        assert!(matches!(
            r.take(usize::MAX),
            Err(ReadError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn uvarint_roundtrip_and_bounds() {
        for v in [0u64, 1, 127, 128, 300, u64::MAX, u64::MAX / 2] {
            let mut buf = Vec::new();
            crate::filechunk::write_uvarint(&mut buf, v);
            let mut r = ByteReader::new(&buf);
            assert_eq!(r.uvarint().unwrap(), v);
            assert!(r.finish().is_ok());
        }
    }

    #[test]
    fn uvarint_rejects_overflow() {
        // 11 continuation bytes: never terminates within 64 bits.
        let bytes = [0x80u8; 11];
        let mut r = ByteReader::new(&bytes);
        assert!(matches!(r.uvarint(), Err(ReadError::VarintOverflow { .. })));
    }

    #[test]
    fn uvarint_rejects_truncation() {
        // Continuation bit set but no following byte.
        let mut r = ByteReader::new(&[0x80]);
        assert!(matches!(r.uvarint(), Err(ReadError::UnexpectedEof { .. })));
    }

    #[test]
    fn string_length_is_bounded() {
        // Claims length 200 but only a few bytes follow.
        let mut buf = Vec::new();
        crate::filechunk::write_uvarint(&mut buf, 200);
        buf.extend_from_slice(b"short");
        let mut r = ByteReader::new(&buf);
        assert!(matches!(
            r.string("x"),
            Err(ReadError::UnexpectedEof { .. })
        ));
    }
}
