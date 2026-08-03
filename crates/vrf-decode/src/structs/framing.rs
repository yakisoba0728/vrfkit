//! The RepLayout dynamic-array framing the three struct blobs share.
//!
//! Kept apart from the effect decoder's near-identical framing on purpose: the
//! two disagree on the element-count ceiling (128 here, 256 there) and on what
//! they do with a malformed element, and the acceptance bar for this crate is
//! byte-identical output. One shared abstraction that quietly changed either
//! would be a worse trade than two small honest copies.

use vrf_bitio::BitReader;

use super::{Result, StructBlobError};

const MAX_ARRAY_COUNT: u32 = 128;
pub(super) const MAX_FIELDS_PER_ELEMENT: u32 = 8;
const MAX_FIELD_PAYLOAD_BITS: u32 = 64 * 1024;

/// Read the declared element count from the stream.
pub(super) fn read_array_count(reader: &mut BitReader<'_>) -> Result<u32> {
    let count = reader.read_int_packed()?;
    if count > MAX_ARRAY_COUNT {
        return Err(StructBlobError::ArrayCountTooLarge {
            count,
            max: MAX_ARRAY_COUNT,
        });
    }
    Ok(count)
}

/// Read the next element index. Returns `None` if the terminator (0) is read.
pub(super) fn read_element_index(
    reader: &mut BitReader<'_>,
    declared_count: u32,
) -> Result<Option<u32>> {
    let encoded = reader.read_int_packed()?;
    if encoded == 0 {
        return Ok(None);
    }
    let index = encoded - 1;
    if index >= declared_count {
        return Err(StructBlobError::IndexOutOfBounds {
            index,
            count: declared_count,
        });
    }
    Ok(Some(index))
}

/// Read the next field handle. Returns `None` if the terminator (0) is read.
/// Also reads the bit_count of the field payload.
pub(super) fn read_field_header(reader: &mut BitReader<'_>) -> Result<Option<(u32, u32)>> {
    let encoded = reader.read_int_packed()?;
    if encoded == 0 {
        return Ok(None);
    }
    let handle = encoded - 1;
    let bit_count = reader.read_int_packed()?;
    if bit_count > MAX_FIELD_PAYLOAD_BITS || u64::from(bit_count) > reader.bits_remaining() {
        return Err(StructBlobError::PayloadTooLarge {
            bits: bit_count,
            remaining: reader.bits_remaining(),
        });
    }
    Ok(Some((handle, bit_count)))
}

/// Read an FName from a sub-reader (1 bit hardcoded flag, then either IntPacked
/// or FString + Int32).
pub(super) fn read_fname(reader: &mut BitReader<'_>) -> Result<String> {
    let is_hardcoded = reader.read_bit()?;
    if is_hardcoded {
        let index = reader.read_int_packed()?;
        Ok(index.to_string())
    } else {
        let name = reader.read_fstring(1024)?;
        let _number = reader.read_i32()?;
        Ok(name)
    }
}

/// Read a byte-width enum whose payload carries only its significant bits.
///
/// Returns `None` for a zero-width or over-wide payload rather than a value:
/// those are not byte enums, and a truncated low byte would be a plausible
/// wrong answer where no answer is the honest one.
pub(super) fn read_narrow_byte(reader: &mut BitReader<'_>) -> Result<Option<u8>> {
    let bits = reader.bits_remaining();
    if bits == 0 || bits > 8 {
        return Ok(None);
    }
    Ok(Some(reader.read_bits(bits as u32)? as u8))
}

/// Ensure the reader is fully consumed.
pub(super) fn ensure_consumed(reader: &BitReader<'_>) -> Result<()> {
    if reader.bits_remaining() > 0 {
        return Err(StructBlobError::NotFullyConsumed {
            remaining: reader.bits_remaining(),
        });
    }
    Ok(())
}
