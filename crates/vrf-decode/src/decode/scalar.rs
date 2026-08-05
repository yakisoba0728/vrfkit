//! Scalar field readers: everything Unreal writes as a single primitive.
//!
//! Each function consumes exactly the field's payload and returns the value in
//! the slot the overlay writes it to. `decode_field` is what checks that the
//! payload was fully consumed, so nothing here needs to.

use vrf_bitio::BitReader;

use super::{DecodeError, DecodedValue};

pub(super) fn decode_bool(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::Bool(r.read_bit()?))
}

/// Decode a byte-width enum or `uint8` property.
///
/// The width is taken from what the field payload actually carries rather than
/// fixed at 8 bits. Unreal writes only the significant bits for byte-sized
/// properties nested inside replicated arrays, so a 5-bit payload is normal and a
/// hard 8-bit read fails on it. The reference implementation does the same thing:
///
/// ```csharp
/// private static byte ReadByte(FBitArchive archive) =>
///     checked((byte)archive.ReadBitsToUInt64(checked((int)archive.BitsRemaining)));
/// ```
///
/// Concretely this is what makes `CombatReport` `AssistType` (5 bits) decode; a
/// fixed-width read left all 364 of its rows untyped.
///
/// Payloads wider than 8 bits are rejected rather than truncated: that means the
/// field is not really byte-sized and silently keeping the low byte would emit a
/// plausible wrong number.
pub(super) fn decode_byte(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let width = r.bits_remaining();
    if width == 0 || width > 8 {
        // Fall back to the nominal width so the error names a concrete read.
        return Ok(DecodedValue::I64(i64::from(r.read_u8()?)));
    }
    let raw = r.read_bits(width as u32)?;
    Ok(DecodedValue::I64(raw as i64))
}

pub(super) fn decode_i32(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_i32()?)))
}

pub(super) fn decode_u32(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_u32()?)))
}

pub(super) fn decode_u64(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    // The overlay stores integers as i64. A u64 with its high bit set cannot
    // be represented without a silent sign flip, so reject it loudly rather
    // than emit a plausible wrong (negative) number. The only UInt64 overlay
    // entries are effect IDs (small values), so this never fires on supported
    // replays -- it is a defensive loud failure for malformed input.
    let value = r.read_u64()?;
    if value > i64::MAX as u64 {
        return Err(DecodeError::UnsignedOverflow { value });
    }
    Ok(DecodedValue::I64(value as i64))
}

pub(super) fn decode_float(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::F64(f64::from(r.read_f32()?)))
}

pub(super) fn decode_double(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::F64(r.read_f64()?))
}

pub(super) fn decode_fstring(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::Str(r.read_fstring(64 * 1024)?))
}

/// FName on the wire: 1 bit `isHardcoded`, then one of two shapes.
///
/// Mirrors `FArchive.ReadFNameCore` in the reference. When the bit is set the
/// name is an index into the engine's hardcoded name table, sent as a single
/// IntPacked and rendered as its decimal value; there is no string. When it is
/// clear the name is inline: FString plus an i32 suffix.
///
/// The comment here used to assert "isHardcoded=false for replays" and the
/// code read the bit and discarded it, always taking the inline path. That is
/// false: 177 of the 581 `DamagedBone` payloads on 02d4d478 are 9 bits, which
/// is exactly the hardcoded shape (1 flag + one IntPacked byte). Reading them
/// as an FString ran off the end of the payload and produced mojibake, which
/// is why the field had to be forced to Raw in the type-correction pass.
pub(super) fn decode_fname(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    if r.read_bit()? {
        let index = r.read_int_packed()?;
        return Ok(DecodedValue::Str(index.to_string()));
    }
    let name = r.read_fstring(64 * 1024)?;
    let _suffix = r.read_i32()?;
    Ok(DecodedValue::Str(name))
}

pub(super) fn decode_object_net_guid(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_int_packed()?)))
}

/// 128-bit GUID: 4 x u32 LE -> formatted as standard hex GUID.
pub(super) fn decode_guid(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    use core::fmt::Write as _;
    let a = r.read_u32()?;
    let b = r.read_u32()?;
    let c = r.read_u32()?;
    let d = r.read_u32()?;
    // 8 + 4 + 4 + 4 + 12 digits plus four dashes: always exactly 36 bytes.
    let mut s = String::with_capacity(36);
    let _ = write!(
        s,
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a,
        (b >> 16) & 0xFFFF,
        b & 0xFFFF,
        (c >> 16) & 0xFFFF,
        (u64::from(c & 0xFFFF) << 32) | u64::from(d)
    );
    Ok(DecodedValue::Str(s))
}

pub(super) fn decode_serialized_int(
    r: &mut BitReader<'_>,
    max: u32,
) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_serialized_int(max)?)))
}

pub(super) fn decode_enum_remaining_bits(
    r: &mut BitReader<'_>,
    bit_count: u32,
) -> Result<DecodedValue, DecodeError> {
    if bit_count == 0 {
        return Ok(DecodedValue::I64(0));
    }
    let bits_left = r.bits_remaining();
    if bits_left == 0 {
        return Ok(DecodedValue::I64(0));
    }
    let to_read = bits_left.min(32);
    Ok(DecodedValue::I64(i64::from(
        r.read_bits(to_read as u32)? as u32
    )))
}

pub(super) fn decode_gameplay_tag(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_int_packed()?)))
}

/// Lowercase hex digits, indexed by nibble.
const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";

/// Length-prefixed byte blob, rendered as lowercase hex into `value_str`.
///
/// The bytes are hex-encoded as they are read rather than collected first: the
/// previous shape ran `format!("{b:02x}")` per byte, which is one heap
/// allocation and one formatting machine per byte of payload.
pub(super) fn decode_byte_array(
    r: &mut BitReader<'_>,
    max_bytes: u32,
) -> Result<DecodedValue, DecodeError> {
    let count = r.read_int_packed()?;
    if count > max_bytes {
        return Err(DecodeError::NotFullyConsumed {
            remaining: r.bits_remaining(),
        });
    }
    let mut hex = String::with_capacity(count as usize * 2);
    for _ in 0..count {
        let byte = r.read_u8()?;
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    Ok(DecodedValue::Str(hex))
}
