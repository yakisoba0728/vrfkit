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
    if component_bits == 0 || component_bits > 62 {
        // Out of sane range -- treat as zero vector.
        return Ok((0, 0, 0));
    }

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
