//! The numeric primitives a move record is built from.
//!
//! # These are format, not style
//!
//! Every constant, width and rounding step below is validated against the C#
//! reference to **zero** error on yaw, pitch and velocity and a maximum of
//! 0.0005 on position. The 48-bit FixedVector, the `SerializedInt(128)` header
//! on a QuantizedVector, and the sign-extension of arbitrary-width components
//! are all wire layout. Rewriting any of the arithmetic here -- even into a
//! form that looks equivalent -- changes decoded output, so it is left exactly
//! as validated.

use vrf_bitio::BitReader;

use crate::error::MovementError;

/// Scale for FixedVector: each u16 component maps to signed range via (raw - 0x8000) / 65536.
pub(crate) const FIXED_VECTOR_SCALE: f64 = 1.0 / 65536.0;

/// Angle conversion: raw u16 -> degrees.
pub(crate) const ANGLE_SCALE: f64 = 360.0 / 65536.0;

/// Read a FixedVector: 3 x u16 packed into 48 bits.
///
/// Each component is sign-offset: `(raw - 0x8000) * (1/65536)`.
/// Result is in range approximately [-0.5, +0.5).
pub(crate) fn read_fixed_vector(
    reader: &mut BitReader<'_>,
) -> Result<(f64, f64, f64), MovementError> {
    let bits = reader.read_bits(48)?;
    let rx = (bits & 0xFFFF) as u32;
    let ry = ((bits >> 16) & 0xFFFF) as u32;
    let rz = ((bits >> 32) & 0xFFFF) as u32;

    let x = (rx as i32 - 0x8000) as f64 * FIXED_VECTOR_SCALE;
    let y = (ry as i32 - 0x8000) as f64 * FIXED_VECTOR_SCALE;
    let z = (rz as i32 - 0x8000) as f64 * FIXED_VECTOR_SCALE;

    Ok((x, y, z))
}

/// Read a QuantizedVector with the given scale factor.
///
/// ## Wire format
///
/// ```text
/// componentBitCountAndExtraInfo : SerializedInt(128)  -- 7 bits via serialized_int
///   componentBits = value & 63
///   extraInfo = value >> 6
///
/// IF componentBits > 0:
///   3 signed components of `componentBits` bits each
///   IF extraInfo > 0: divide each by scaleFactor
/// ELIF extraInfo == 0:
///   3 x f32 (raw IEEE-754)
/// ELSE:
///   3 x f64 (raw IEEE-754)
/// ```
pub(crate) fn read_quantized_vector(
    reader: &mut BitReader<'_>,
    scale_factor: i32,
) -> Result<(f64, f64, f64), MovementError> {
    // SerializedInt(128) -- uses up to 7 bits.
    let info = u64::from(reader.read_serialized_int(128)?);
    let component_bits = (info & 63) as u32;
    let extra_info = info >> 6;

    if component_bits > 0 {
        let (x, y, z) = read_signed_quantized_components(reader, component_bits)?;
        if extra_info > 0 {
            let sf = f64::from(scale_factor);
            Ok((x as f64 / sf, y as f64 / sf, z as f64 / sf))
        } else {
            Ok((x as f64, y as f64, z as f64))
        }
    } else if extra_info == 0 {
        // 3 x f32
        let x = f64::from(reader.read_f32()?);
        let y = f64::from(reader.read_f32()?);
        let z = f64::from(reader.read_f32()?);
        Ok((x, y, z))
    } else {
        // 3 x f64
        let x = reader.read_f64()?;
        let y = reader.read_f64()?;
        let z = reader.read_f64()?;
        Ok((x, y, z))
    }
}

/// Read 3 signed components of `component_bits` bits each.
///
/// When the total (3 x component_bits) fits in 64 bits, all three are packed
/// into a single read. Otherwise they are read individually.
fn read_signed_quantized_components(
    reader: &mut BitReader<'_>,
    component_bits: u32,
) -> Result<(i64, i64, i64), MovementError> {
    // `componentBits` is `info & 63`, so 63 is the largest width the header can
    // express -- and it is fully readable: `read_bits` takes up to 64 and
    // `sign_extend`'s sign bit lands at `1 << 62`. The bound used to be 62,
    // which made a declared width of 63 return `(0, 0, 0)` *without consuming
    // the 189 bits it declared*. That is the worst available answer: a
    // world-origin position that looks like a real sample, a move that reports
    // no error, and a cursor left 189 bits behind so everything after it
    // decodes from the wrong offset. Zero is the only width that legitimately
    // reads nothing, and the caller already handles it.
    if component_bits == 0 {
        return Ok((0, 0, 0));
    }
    // A real assert, not a `debug_assert`: `[profile.release]` does not enable
    // debug assertions, so a debug-only check would be absent from exactly the
    // binary that exports the corpus. Above 64 this is not a wrong answer but
    // an out-of-range shift -- `mask_u64(65)` is `u64::MAX >> (64 - 65)`,
    // which release masks into a nonsense mask instead of trapping.
    //
    // Panicking rather than returning an error is deliberate and follows
    // `copy_bits_to`'s precedent for the same shape: the only caller derives
    // this width as `info & 63`, so a value above 63 is a bug at the call site
    // and not malformed input, and a recoverable error would imply the wire
    // can produce it. One compare against a constant, once per vector header.
    assert!(
        component_bits <= 63,
        "component_bits must be 1..=63, got {component_bits}"
    );

    let total_bits = component_bits * 3;

    if total_bits <= 64 {
        let raw = reader.read_bits(total_bits)?;
        let mask = (1u64 << component_bits) - 1;
        let x = sign_extend(raw & mask, component_bits);
        let y = sign_extend((raw >> component_bits) & mask, component_bits);
        let z = sign_extend((raw >> (component_bits * 2)) & mask, component_bits);
        Ok((x, y, z))
    } else {
        let x = read_signed_component(reader, component_bits)?;
        let y = read_signed_component(reader, component_bits)?;
        let z = read_signed_component(reader, component_bits)?;
        Ok((x, y, z))
    }
}

/// Read a single signed component of `bits` width.
fn read_signed_component(reader: &mut BitReader<'_>, bits: u32) -> Result<i64, MovementError> {
    let raw = reader.read_bits(bits)?;
    Ok(sign_extend(raw, bits))
}

/// Sign-extend a `bit_count`-wide unsigned value to i64.
#[inline]
fn sign_extend(raw: u64, bit_count: u32) -> i64 {
    let sign_bit = 1u64 << (bit_count - 1);
    (raw ^ sign_bit).wrapping_sub(sign_bit) as i64
}

/// Read a VLQ-encoded u32.
///
/// This is byte-for-byte Unreal's `IntPacked`: each byte carries 7 payload bits
/// in its high bits and the continuation flag in bit 0
/// (`value |= ((b >> 1) & 0x7F) << shift`, stopping when `(b & 1) == 0`). The
/// name "VLQ" comes from the C# reference, which spells the loop out inline
/// rather than reusing its own IntPacked reader; the encodings are identical,
/// so this delegates.
#[inline]
pub(crate) fn read_vlq(reader: &mut BitReader<'_>) -> Result<u32, MovementError> {
    Ok(reader.read_int_packed()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LSB-first bit writer -- the inverse of the reader under test, spelled
    /// out here rather than shared with `vrf-bitio` so that a bug mirrored in
    /// both cannot cancel itself out.
    #[derive(Default)]
    struct Bits {
        bits: Vec<bool>,
    }

    impl Bits {
        fn write(&mut self, value: u64, count: u32) {
            for i in 0..count {
                self.bits.push((value >> i) & 1 != 0);
            }
        }

        fn bit_len(&self) -> u64 {
            self.bits.len() as u64
        }

        fn to_bytes(&self) -> Vec<u8> {
            let mut bytes = vec![0u8; self.bits.len().div_ceil(8)];
            for (i, &bit) in self.bits.iter().enumerate() {
                if bit {
                    bytes[i >> 3] |= 1 << (i & 7);
                }
            }
            bytes
        }
    }

    /// Build a QuantizedVector payload: the `SerializedInt(128)` header
    /// followed by three components of `component_bits` each.
    fn quantized(component_bits: u32, extra_info: u64, comps: [u64; 3]) -> Bits {
        let mut w = Bits::default();
        // `read_serialized_int(128)` spends `128.ilog2() == 7` bits and never
        // the extra one, because `value + 128 >= 128` holds for every value.
        w.write((extra_info << 6) | u64::from(component_bits), 7);
        for c in comps {
            w.write(c, component_bits);
        }
        w
    }

    #[test]
    fn component_bits_of_63_reads_all_189_declared_bits() {
        // 63 is the largest value `info & 63` can produce, and every part of
        // reading it is in range: `read_bits(63)` is legal and `sign_extend`'s
        // sign bit lands at `1 << 62`. The old bound of 62 fabricated a
        // world-origin `(0, 0, 0)` here *without consuming the 189 bits the
        // header declared*, so the move still decoded, `error_count` stayed 0,
        // and every field after it came from the wrong bit offset.
        let most_negative = 1u64 << 62; // -2^62 in 63-bit two's complement
        let minus_one = (1u64 << 63) - 1; // all 63 bits set
        let w = quantized(63, 0, [1, minus_one, most_negative]);
        let bytes = w.to_bytes();
        let mut r = BitReader::with_bit_len(&bytes, w.bit_len());

        let (x, y, z) = read_quantized_vector(&mut r, 100).unwrap();

        assert_eq!(x, 1.0);
        assert_eq!(y, -1.0);
        assert_eq!(z, -(2f64.powi(62)));
        assert_eq!(r.position(), 7 + 189, "all three components must be read");
        assert!(r.at_end());
    }

    #[test]
    fn component_bits_of_62_still_reads_its_186_bits() {
        // Regression guard, not TDD credit: 62 already worked. It pins the
        // boundary that used to separate "read" from "fabricated" so a future
        // bound change has to break a test rather than a corpus.
        let w = quantized(62, 0, [7, (1u64 << 62) - 1, 1u64 << 61]);
        let bytes = w.to_bytes();
        let mut r = BitReader::with_bit_len(&bytes, w.bit_len());

        let (x, y, z) = read_quantized_vector(&mut r, 100).unwrap();

        assert_eq!(x, 7.0);
        assert_eq!(y, -1.0);
        assert_eq!(z, -(2f64.powi(61)));
        assert_eq!(r.position(), 7 + 186);
    }

    #[test]
    #[should_panic(expected = "component_bits must be 1..=63")]
    fn a_width_the_header_cannot_express_is_refused_even_without_debug_assertions() {
        // Replacing the old `> 62` bound removed a *total* guard, and a
        // `debug_assert` does not restore it: `[profile.release]` in the
        // workspace manifest does not enable debug assertions, so the binary
        // that exports the corpus compiles it away. Above 64 the arithmetic
        // does not merely mis-answer, it goes out of range --
        // `mask_u64(65)` is `u64::MAX >> (64 - 65)`, an over-wide shift that
        // release masks into a nonsense mask rather than trapping.
        //
        // This is the call-site-bug shape `copy_bits_to` already treats as a
        // hard assert rather than a recoverable error, for the same reason:
        // the only caller masks the width to six bits, so reaching here means
        // a programming error and not malformed input. Run this file with
        // `-C debug-assertions=off` to see the guard actually hold.
        let data = [0xFFu8; 32];
        let mut r = BitReader::with_bit_len(&data, 256);
        let _ = read_signed_quantized_components(&mut r, 64);
    }

    #[test]
    fn a_truncated_63_bit_vector_reports_eof_rather_than_a_zero_vector() {
        // The other half of the same fix: refusing to fabricate means a short
        // payload has to fail, not quietly return the origin.
        let mut w = quantized(63, 0, [1, 1, 1]);
        w.bits.truncate(7 + 100);
        let bytes = w.to_bytes();
        let mut r = BitReader::with_bit_len(&bytes, w.bit_len());

        assert!(matches!(
            read_quantized_vector(&mut r, 100),
            Err(MovementError::Bit(_))
        ));
    }
}
