//! LSB-first bit reader for Unreal Engine replay streams.
//!
//! # Bit order
//!
//! Unreal's `FBitWriter` packs bits **least-significant-first within each byte**
//! and lets values straddle byte boundaries, so a payload is one continuous bit
//! stream: global bit `i` lives at `data[i >> 3] >> (i & 7) & 1`. Reading a
//! multi-bit value therefore means "shift the window right by the bit offset and
//! mask", never "shift left and OR" as an MSB-first reader would.
//!
//! # Why a bespoke reader
//!
//! Three of the wire primitives are Unreal-specific and consume a *variable*
//! number of bits, so the exact implementation is part of the format:
//!
//! * [`BitReader::read_int_packed`] -- 7 bits of payload per byte, low bit is the
//!   continuation flag, capped at five bytes.
//! * [`BitReader::read_serialized_int`] -- `floor(log2(max))` bits, plus one more
//!   bit only when the value could still reach `max`.
//! * [`BitReader::read_fstring`] -- a signed length that selects UTF-8 (positive)
//!   or UTF-16 (negative), null-terminated either way.
//!
//! Getting the consumed bit count wrong on any of these desynchronises the rest
//! of the stream rather than failing loudly, which is why each one is pinned by
//! tests.
//!
//! # Errors
//!
//! Every read is bounds-checked and returns [`BitError`] instead of panicking or
//! silently yielding zeros: a truncated payload must be distinguishable from a
//! payload whose value happens to be zero.

#![forbid(unsafe_code)]

use core::fmt;

/// Largest number of bytes an [`Int packed`](BitReader::read_int_packed) value
/// may occupy. Unreal encodes 7 payload bits per byte, so five bytes cover the
/// full 32-bit range (35 bits) and a sixth would mean a malformed stream.
const MAX_INT_PACKED_BYTES: u32 = 5;

/// Failure modes of a bit read. All of them mean "the stream is not what the
/// caller assumed", never "the value was empty".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitError {
    /// Fewer bits remain than the read requires.
    Eof {
        /// Bit position at which the read was attempted.
        position: u64,
        /// Total length of the archive, in bits.
        length: u64,
        /// Bits the read needed.
        requested: u64,
    },
    /// An `IntPacked` value did not terminate within [`MAX_INT_PACKED_BYTES`].
    MalformedIntPacked {
        /// Bit position at which the value started.
        position: u64,
    },
    /// `read_serialized_int` was given a non-positive maximum.
    InvalidSerializedIntMax {
        /// The rejected maximum.
        max: u32,
    },
    /// A length-prefixed value declared a size beyond a sane limit or beyond the
    /// remaining stream.
    InvalidLength {
        /// Bit position at which the length prefix started.
        position: u64,
        /// The rejected length.
        length: i64,
    },
    /// A string's bytes were not valid UTF-8 / UTF-16.
    InvalidString {
        /// Bit position at which the string started.
        position: u64,
    },
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof {
                position,
                length,
                requested,
            } => write!(
                f,
                "unexpected end of archive: needed {requested} bit(s) at position {position} of {length}"
            ),
            Self::MalformedIntPacked { position } => write!(
                f,
                "packed integer at position {position} did not terminate within {MAX_INT_PACKED_BYTES} bytes"
            ),
            Self::InvalidSerializedIntMax { max } => {
                write!(f, "serialized int maximum must be positive, got {max}")
            }
            Self::InvalidLength { position, length } => {
                write!(f, "invalid length {length} at position {position}")
            }
            Self::InvalidString { position } => {
                write!(f, "malformed string at position {position}")
            }
        }
    }
}

impl core::error::Error for BitError {}

/// Result alias for bit reads.
pub type Result<T> = core::result::Result<T, BitError>;

/// A cursor over a bit stream.
///
/// The reader borrows its backing bytes, so sub-readers carved out of a payload
/// are views rather than copies -- framing a bunch into content blocks and a
/// content block into fields allocates nothing.
///
/// `start_bit` lets a sub-reader address a window that does not begin on a byte
/// boundary, which is the normal case: Unreal field payloads are bit-aligned.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit index (into `data`) where this reader's window begins.
    start_bit: u64,
    /// Bits consumed so far, relative to `start_bit`.
    pos: u64,
    /// Window length in bits.
    len: u64,
}

impl<'a> BitReader<'a> {
    /// Create a reader over every bit of `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            start_bit: 0,
            pos: 0,
            len: (data.len() as u64) * 8,
        }
    }

    /// Create a reader over the first `bit_len` bits of `data`.
    ///
    /// Used for payloads whose exact bit length is known from the wire (a bunch
    /// header, a content block header) and whose final byte therefore carries
    /// padding that must not be read as data.
    ///
    /// # Panics
    ///
    /// Panics if `bit_len` exceeds the bits available in `data`; that is a
    /// programming error at the call site, not malformed input.
    #[must_use]
    pub fn with_bit_len(data: &'a [u8], bit_len: u64) -> Self {
        assert!(bit_len <= (data.len() as u64) * 8, "bit_len exceeds data");
        Self {
            data,
            start_bit: 0,
            pos: 0,
            len: bit_len,
        }
    }

    /// Bits consumed so far.
    #[must_use]
    #[inline]
    pub const fn position(&self) -> u64 {
        self.pos
    }

    /// Total window length in bits.
    #[must_use]
    #[inline]
    pub const fn len_bits(&self) -> u64 {
        self.len
    }

    /// Bits left to read.
    #[must_use]
    #[inline]
    pub const fn bits_remaining(&self) -> u64 {
        self.len - self.pos
    }

    /// Whether the window is fully consumed.
    #[must_use]
    #[inline]
    pub const fn at_end(&self) -> bool {
        self.pos >= self.len
    }

    #[inline]
    fn need(&self, bits: u64) -> Result<()> {
        if self.bits_remaining() < bits {
            return Err(BitError::Eof {
                position: self.pos,
                length: self.len,
                requested: bits,
            });
        }
        Ok(())
    }

    /// Load 64 bits starting at absolute byte `byte`, zero-padding past the end.
    ///
    /// Padding is safe because callers have already checked that the *bits* they
    /// want are in range; the padding only ever covers bits that get masked off.
    #[inline]
    fn load_u64(&self, byte: usize) -> u64 {
        let mut buf = [0u8; 8];
        let end = (byte + 8).min(self.data.len());
        if byte < end {
            let n = end - byte;
            buf[..n].copy_from_slice(&self.data[byte..end]);
        }
        u64::from_le_bytes(buf)
    }

    /// Read a single bit.
    #[inline]
    pub fn read_bit(&mut self) -> Result<bool> {
        self.need(1)?;
        let abs = self.start_bit + self.pos;
        // Bounds: `need` guarantees this bit is inside the window, and the
        // window is inside `data`, so the index cannot be out of range.
        let bit = (self.data[(abs >> 3) as usize] >> (abs & 7)) & 1;
        self.pos += 1;
        Ok(bit != 0)
    }

    /// Read `count` bits (0..=64) LSB-first into the low bits of a `u64`.
    #[inline]
    pub fn read_bits(&mut self, count: u32) -> Result<u64> {
        debug_assert!(count <= 64, "read_bits supports at most 64 bits");
        if count == 0 {
            return Ok(0);
        }
        self.need(u64::from(count))?;
        let abs = self.start_bit + self.pos;
        let byte = (abs >> 3) as usize;
        let off = (abs & 7) as u32;

        // A single 8-byte window holds `64 - off` usable bits. When the request
        // straddles past that, a second window supplies the remainder. Two loads
        // always suffice: off <= 7 and count <= 64, so off + count <= 71 < 128.
        let low = self.load_u64(byte) >> off;
        let got = 64 - off;
        let value = if count <= got {
            low & mask_u64(count)
        } else {
            let high = self.load_u64(byte + 8);
            (low | (high << got)) & mask_u64(count)
        };
        self.pos += u64::from(count);
        Ok(value)
    }

    /// Read 8 bits as a byte.
    #[inline]
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bits(8)? as u8)
    }

    /// Read 16 bits little-endian.
    #[inline]
    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(self.read_bits(16)? as u16)
    }

    /// Read 32 bits little-endian.
    #[inline]
    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(self.read_bits(32)? as u32)
    }

    /// Read 32 bits little-endian as a signed integer.
    #[inline]
    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read_u32()? as i32)
    }

    /// Read 64 bits little-endian.
    #[inline]
    pub fn read_u64(&mut self) -> Result<u64> {
        self.read_bits(64)
    }

    /// Read an IEEE-754 single.
    #[inline]
    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    /// Read an IEEE-754 double.
    #[inline]
    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    /// Read Unreal's `SerializeIntPacked` variable-length integer.
    ///
    /// Each byte carries 7 payload bits in its high bits; the low bit is set
    /// when another byte follows. Chunks are little-endian, so byte `i`
    /// contributes at shift `7 * i`.
    pub fn read_int_packed(&mut self) -> Result<u32> {
        let start = self.pos;
        let mut value: u32 = 0;
        let mut shift: u32 = 0;
        for _ in 0..MAX_INT_PACKED_BYTES {
            let next = self.read_u8()?;
            value |= u32::from(next >> 1) << shift;
            if next & 1 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
        Err(BitError::MalformedIntPacked { position: start })
    }

    /// Read Unreal's bounded integer encoding (`FBitReader::SerializeInt`).
    ///
    /// Spends `floor(log2(max))` bits unconditionally, then one extra bit only
    /// when the value read so far could still be raised to reach `max`. The
    /// consumed width therefore depends on the *value*, not just on `max`.
    pub fn read_serialized_int(&mut self, max: u32) -> Result<u32> {
        if max == 0 {
            return Err(BitError::InvalidSerializedIntMax { max });
        }
        let value_bits = max.ilog2();
        let mut value = if value_bits > 0 {
            self.read_bits(value_bits)? as u32
        } else {
            0
        };
        let bit_mask = 1u32 << value_bits;
        // `value + bit_mask >= max` means raising the top bit would overshoot,
        // so the encoder never wrote it and there is nothing more to read.
        if value.saturating_add(bit_mask) >= max {
            return Ok(value);
        }
        if self.read_bit()? {
            value |= bit_mask;
        }
        Ok(value)
    }

    /// Read a length-prefixed Unreal string.
    ///
    /// A positive length counts UTF-8 bytes, a negative one counts UTF-16 code
    /// units; both include a trailing null which is stripped. `max_bytes` caps
    /// the serialized size so a corrupt prefix cannot trigger a huge allocation.
    pub fn read_fstring(&mut self, max_bytes: i64) -> Result<String> {
        let start = self.pos;
        let raw = i64::from(self.read_i32()?);
        if raw == 0 {
            return Ok(String::new());
        }
        let utf16 = raw < 0;
        let units = raw.unsigned_abs();
        let byte_len = if utf16 {
            units.checked_mul(2).ok_or(BitError::InvalidLength {
                position: start,
                length: raw,
            })?
        } else {
            units
        };
        if byte_len > max_bytes.unsigned_abs() {
            return Err(BitError::InvalidLength {
                position: start,
                length: raw,
            });
        }
        let byte_len = usize::try_from(byte_len).map_err(|_| BitError::InvalidLength {
            position: start,
            length: raw,
        })?;

        if utf16 {
            let mut units16 = Vec::with_capacity(byte_len / 2);
            for _ in 0..byte_len / 2 {
                units16.push(self.read_u16()?);
            }
            // Drop the trailing null before decoding so it never lands in the
            // returned string.
            if units16.last() == Some(&0) {
                units16.pop();
            }
            String::from_utf16(&units16).map_err(|_| BitError::InvalidString { position: start })
        } else {
            let mut bytes = Vec::with_capacity(byte_len);
            for _ in 0..byte_len {
                bytes.push(self.read_u8()?);
            }
            if bytes.last() == Some(&0) {
                bytes.pop();
            }
            String::from_utf8(bytes).map_err(|_| BitError::InvalidString { position: start })
        }
    }

    /// Copy `count` bits into `dst`, LSB-first, zero-filling the final byte's
    /// padding, and advance past them.
    ///
    /// Zero-filling matters: the payload transform runs over whole bytes, so any
    /// stale bits above `count` would be folded into the result and corrupt the
    /// tail byte.
    pub fn copy_bits_to(&mut self, dst: &mut [u8], count: u64) -> Result<()> {
        let byte_count =
            usize::try_from(count.div_ceil(8)).map_err(|_| BitError::InvalidLength {
                position: self.pos,
                length: count as i64,
            })?;
        assert!(
            dst.len() >= byte_count,
            "destination too small for {count} bits"
        );
        self.need(count)?;
        if count == 0 {
            return Ok(());
        }

        let mut written = 0usize;
        let mut left = count;
        while left >= 64 {
            let chunk = self.read_bits(64)?;
            dst[written..written + 8].copy_from_slice(&chunk.to_le_bytes());
            written += 8;
            left -= 64;
        }
        if left > 0 {
            let chunk = self.read_bits(left as u32)?;
            let bytes = chunk.to_le_bytes();
            let n = left.div_ceil(8) as usize;
            dst[written..written + n].copy_from_slice(&bytes[..n]);
            // The high bits of the last byte are padding; `chunk` already has
            // them as zero because read_bits masked to `left` bits.
        }
        Ok(())
    }

    /// Carve out a view over the next `count` bits and advance past them.
    ///
    /// The child shares the parent's buffer, so framing is allocation-free.
    pub fn sub_reader(&mut self, count: u64) -> Result<BitReader<'a>> {
        self.need(count)?;
        let child = BitReader {
            data: self.data,
            start_bit: self.start_bit + self.pos,
            pos: 0,
            len: count,
        };
        self.pos += count;
        Ok(child)
    }

    /// Skip `count` bits.
    pub fn skip_bits(&mut self, count: u64) -> Result<()> {
        self.need(count)?;
        self.pos += count;
        Ok(())
    }

    /// Skip the rest of the window.
    pub fn skip_remaining(&mut self) {
        self.pos = self.len;
    }
}

/// Mask with the low `count` bits set. `count == 64` would overflow `1 << 64`,
/// so it is special-cased.
#[inline]
const fn mask_u64(count: u32) -> u64 {
    if count >= 64 {
        u64::MAX
    } else {
        (1u64 << count) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_least_significant_bit_first() {
        // 0b1010_0101 -> bit0 = 1, bit1 = 0, bit2 = 1, bit3 = 0 ...
        let data = [0b1010_0101u8];
        let mut r = BitReader::new(&data);
        let bits: Vec<bool> = (0..8).map(|_| r.read_bit().unwrap()).collect();
        assert_eq!(
            bits,
            vec![true, false, true, false, false, true, false, true]
        );
        assert!(r.at_end());
    }

    #[test]
    fn read_bits_spans_byte_boundaries() {
        // Two bytes little-endian = 0xBEEF; reading 16 bits must yield it whole.
        let data = [0xEFu8, 0xBE];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_bits(16).unwrap(), 0xBEEF);

        // Starting 4 bits in, the next 8 bits straddle the boundary.
        let mut r = BitReader::new(&data);
        r.skip_bits(4).unwrap();
        assert_eq!(r.read_bits(8).unwrap(), 0xEE);
    }

    #[test]
    fn read_bits_handles_full_width_at_offset() {
        let data = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99];
        let mut r = BitReader::new(&data);
        r.skip_bits(4).unwrap();
        let v = r.read_bits(64).unwrap();
        // Expected: the 64 bits starting at bit 4 of the little-endian stream.
        let lo = u64::from_le_bytes([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let expected = (lo >> 4) | (u64::from(0x99u8) << 60);
        assert_eq!(v, expected);
    }

    #[test]
    fn read_bits_zero_is_noop() {
        let data = [0xFFu8];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_bits(0).unwrap(), 0);
        assert_eq!(r.position(), 0);
    }

    #[test]
    fn eof_is_reported_not_padded() {
        let data = [0xFFu8];
        let mut r = BitReader::new(&data);
        r.skip_bits(6).unwrap();
        let err = r.read_bits(8).unwrap_err();
        assert_eq!(
            err,
            BitError::Eof {
                position: 6,
                length: 8,
                requested: 8
            }
        );
    }

    #[test]
    fn int_packed_single_byte() {
        // 7 payload bits, continuation clear: value 0x3F.
        let data = [0x3F << 1];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_int_packed().unwrap(), 0x3F);
        assert_eq!(r.position(), 8);
    }

    #[test]
    fn int_packed_multi_byte_is_little_endian_in_chunks() {
        // value 300 = 0b1_0010_1100 -> chunk0 = 0b010_1100 (44), chunk1 = 0b10 (2)
        // byte0 = 44 << 1 | 1 (more), byte1 = 2 << 1 (last)
        let data = [(44u8 << 1) | 1, 2u8 << 1];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_int_packed().unwrap(), 300);
        assert_eq!(r.position(), 16);
    }

    #[test]
    fn int_packed_rejects_runaway() {
        let data = [0xFFu8; 8];
        let mut r = BitReader::new(&data);
        assert_eq!(
            r.read_int_packed().unwrap_err(),
            BitError::MalformedIntPacked { position: 0 }
        );
    }

    #[test]
    fn int_packed_is_bit_aligned_not_byte_aligned() {
        // Same value, but the stream starts one bit in: the reader must still
        // consume 8 bits per chunk from the *bit* position.
        let value = 0x3Fu8 << 1;
        let shifted = [(value as u16) << 1].map(|v| v);
        let data = [(shifted[0] & 0xFF) as u8, (shifted[0] >> 8) as u8];
        let mut r = BitReader::new(&data);
        r.skip_bits(1).unwrap();
        assert_eq!(r.read_int_packed().unwrap(), 0x3F);
    }

    #[test]
    fn serialized_int_spends_log2_bits_then_maybe_one_more() {
        // max = 4 -> value_bits = 2, bit_mask = 4. value 0..=3 all satisfy
        // value + 4 >= 4, so exactly 2 bits are consumed and no extra bit.
        let data = [0b11u8];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_serialized_int(4).unwrap(), 3);
        assert_eq!(r.position(), 2);

        // max = 5 -> value_bits = 2, bit_mask = 4. value 0 gives 0 + 4 < 5, so a
        // third bit is read and can raise the value to 4.
        let data = [0b100u8];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_serialized_int(5).unwrap(), 4);
        assert_eq!(r.position(), 3);
    }

    #[test]
    fn serialized_int_max_one_consumes_nothing() {
        let data = [0xFFu8];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_serialized_int(1).unwrap(), 0);
        assert_eq!(r.position(), 0);
    }

    #[test]
    fn serialized_int_rejects_zero_max() {
        let data = [0xFFu8];
        let mut r = BitReader::new(&data);
        assert_eq!(
            r.read_serialized_int(0).unwrap_err(),
            BitError::InvalidSerializedIntMax { max: 0 }
        );
    }

    #[test]
    fn fstring_utf8_strips_null() {
        let mut data = Vec::new();
        data.extend_from_slice(&4i32.to_le_bytes());
        data.extend_from_slice(b"abc\0");
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_fstring(1024).unwrap(), "abc");
    }

    #[test]
    fn fstring_utf16_uses_negative_length() {
        let mut data = Vec::new();
        data.extend_from_slice(&(-3i32).to_le_bytes());
        for u in ['h' as u16, 'i' as u16, 0u16] {
            data.extend_from_slice(&u.to_le_bytes());
        }
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_fstring(1024).unwrap(), "hi");
    }

    #[test]
    fn fstring_empty() {
        let data = 0i32.to_le_bytes();
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_fstring(1024).unwrap(), "");
    }

    #[test]
    fn fstring_rejects_oversized_length() {
        let data = 1_000_000i32.to_le_bytes();
        let mut r = BitReader::new(&data);
        assert!(matches!(
            r.read_fstring(64).unwrap_err(),
            BitError::InvalidLength { .. }
        ));
    }

    #[test]
    fn copy_bits_masks_padding() {
        // 0xBF = 0b1011_1111. Copying 1 bit must yield 0x01, not 0xBF: the
        // transform runs over bytes, so padding has to be zero.
        let data = [0xBFu8];
        let mut r = BitReader::new(&data);
        let mut dst = [0xAAu8; 1];
        r.copy_bits_to(&mut dst, 1).unwrap();
        assert_eq!(dst[0], 0x01);
    }

    #[test]
    fn copy_bits_across_many_words() {
        let data: Vec<u8> = (0..=20u8).collect();
        let mut r = BitReader::new(&data);
        let mut dst = vec![0u8; 21];
        r.copy_bits_to(&mut dst, 21 * 8).unwrap();
        assert_eq!(dst, data);
    }

    #[test]
    fn copy_bits_from_unaligned_start() {
        let data = [0b1111_0000u8, 0b0000_1111];
        let mut r = BitReader::new(&data);
        r.skip_bits(4).unwrap();
        let mut dst = [0u8; 1];
        r.copy_bits_to(&mut dst, 8).unwrap();
        assert_eq!(dst[0], 0b1111_1111);
    }

    #[test]
    fn sub_reader_is_a_window_that_advances_the_parent() {
        let data = [0xFFu8, 0x00, 0xFF];
        let mut parent = BitReader::new(&data);
        let mut child = parent.sub_reader(12).unwrap();
        assert_eq!(child.len_bits(), 12);
        assert_eq!(parent.position(), 12);
        assert_eq!(child.read_bits(12).unwrap(), 0x0FF);
        assert!(child.at_end());
        // The child cannot see past its window even though the buffer continues.
        assert!(child.read_bit().is_err());
    }

    #[test]
    fn sub_reader_of_sub_reader_keeps_absolute_offset() {
        let data = [0x00u8, 0xFF, 0x00];
        let mut parent = BitReader::new(&data);
        parent.skip_bits(8).unwrap();
        let mut child = parent.sub_reader(8).unwrap();
        let mut grandchild = child.sub_reader(4).unwrap();
        assert_eq!(grandchild.read_bits(4).unwrap(), 0xF);
    }

    #[test]
    fn with_bit_len_hides_trailing_padding() {
        let data = [0xFFu8, 0xFF];
        let mut r = BitReader::with_bit_len(&data, 12);
        assert_eq!(r.bits_remaining(), 12);
        r.skip_bits(12).unwrap();
        assert!(r.read_bit().is_err());
    }
}
