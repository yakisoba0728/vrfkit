//! The RepLayout dynamic-array framing every effect blob shares, and the
//! structural scan that derives an array's element handle pair from it.

use vrf_bitio::BitReader;

use super::{EffectBlobError, EffectHandles, Result};

/// Maximum array element count. In practice, FloatValues has at most ~6
/// elements, ObjectValues ~4, and VectorValues up to ~15 (shotgun pellets).
/// 256 provides generous headroom without risking runaway allocation.
pub(super) const MAX_ARRAY_COUNT: u32 = 256;

/// Maximum fields per element. EffectData* structs have 2 handles each
/// (tag + value), so 8 is very generous.
pub(super) const MAX_FIELDS_PER_ELEMENT: u32 = 8;

/// Maximum bits in a single field payload. Prevents runaway on corrupt data.
const MAX_FIELD_PAYLOAD_BITS: u32 = 64 * 1024;

/// Build a reader over an exact bit window, without the panic.
pub(super) fn new_blob_reader(raw: &[u8], bit_count: u32) -> Result<BitReader<'_>> {
    let available = (raw.len() as u64) * 8;
    if u64::from(bit_count) > available {
        // `BitReader::with_bit_len` asserts on this, and a panic in the export
        // path would take the whole run down over one malformed row.
        return Err(EffectBlobError::BitLengthExceedsBuffer {
            bits: bit_count,
            available,
        });
    }
    Ok(BitReader::with_bit_len(raw, u64::from(bit_count)))
}

/// Read the declared element count from the stream.
pub(super) fn read_array_count(reader: &mut BitReader<'_>) -> Result<u32> {
    let count = reader.read_int_packed()?;
    if count > MAX_ARRAY_COUNT {
        return Err(EffectBlobError::ArrayCountTooLarge {
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
        return Err(EffectBlobError::IndexOutOfBounds {
            index,
            count: declared_count,
        });
    }
    Ok(Some(index))
}

/// Read the next field handle + payload bit count. Returns `None` on terminator.
pub(super) fn read_field_header(reader: &mut BitReader<'_>) -> Result<Option<(u32, u32)>> {
    let encoded_handle = reader.read_int_packed()?;
    if encoded_handle == 0 {
        return Ok(None);
    }
    let handle = encoded_handle - 1;
    let payload_bits = reader.read_int_packed()?;
    if payload_bits > MAX_FIELD_PAYLOAD_BITS || u64::from(payload_bits) > reader.bits_remaining() {
        return Err(EffectBlobError::PayloadTooLarge {
            bits: payload_bits,
            remaining: reader.bits_remaining(),
        });
    }
    Ok(Some((handle, payload_bits)))
}

/// The C# parser checks whether exactly 8 bits remain after the zero terminator
/// and reads one more IntPacked if so. Replicate that.
pub(super) fn consume_trailing_terminator(reader: &mut BitReader<'_>) {
    if reader.bits_remaining() == 8 {
        let _ = reader.read_int_packed();
    }
}

/// Reject a value field whose declared width its type cannot occupy.
pub(super) fn expect_width(context: &'static str, expected: u32, found: u32) -> Result<()> {
    if found == expected {
        Ok(())
    } else {
        Err(EffectBlobError::UnexpectedPayloadWidth {
            context,
            expected,
            found,
        })
    }
}

/// Advance the reader to the end of a field whose header declared
/// `payload_bits`, having already read `reader.position() - start_pos` of them.
///
/// Reading *past* the declared width is the interesting case: it means the
/// field's type is wider than the field, so the decode has already consumed
/// part of the next field and every value after it is suspect.
pub(super) fn settle_field(
    reader: &mut BitReader<'_>,
    start_pos: u64,
    payload_bits: u32,
) -> Result<()> {
    let consumed = reader.position() - start_pos;
    let declared = u64::from(payload_bits);
    if consumed > declared {
        return Err(EffectBlobError::PayloadOverread {
            declared: payload_bits,
            consumed,
        });
    }
    reader.skip_bits(declared - consumed)?;
    Ok(())
}

/// Derive an array's element handle pair by walking its framing.
///
/// Reads structure only -- every field payload is skipped, no handle is
/// interpreted -- so the result does not depend on knowing which function the
/// blob came from. Returns `None` when the array populates no element, which
/// leaves nothing to derive the pair from and nothing for it to decode.
///
/// The derivation is structural on purpose, so that it assumes nothing about
/// how Unreal numbers handles. That independence is what makes the following
/// a real check rather than a tautology: Unreal is documented to number a
/// dynamic array's element handles from the array's own handle plus one, and
/// on `02d4d478` the derived base equals the RPC parameter's own handle plus
/// one for all 53,908 blobs, with no exception. Two unrelated routes to the
/// same number.
///
/// # Errors
/// Rejects any array whose elements do not each carry exactly two fields at
/// adjacent handles, all elements agreeing on the lower one. That is the shape
/// of all 128,000 elements on `02d4d478`, and it is what makes the pair
/// derivable at all; a blob outside it is reported rather than guessed at.
pub fn scan_element_handles(raw: &[u8], bit_count: u32) -> Result<Option<EffectHandles>> {
    let mut reader = new_blob_reader(raw, bit_count)?;
    let count = read_array_count(&mut reader)?;
    let mut base: Option<u32> = None;

    while !reader.at_end() {
        let Some(_index) = read_element_index(&mut reader, count)? else {
            consume_trailing_terminator(&mut reader);
            break;
        };

        let mut seen = [0u32; 2];
        for slot in &mut seen {
            let Some((handle, payload_bits)) = read_field_header(&mut reader)? else {
                return Err(EffectBlobError::ElementFieldCount { found: 0 });
            };
            reader.skip_bits(u64::from(payload_bits))?;
            *slot = handle;
        }
        if let Some((_, payload_bits)) = read_field_header(&mut reader)? {
            // Consume it so the error message's position is not misleading if
            // a caller ever reports one; the blob is rejected either way.
            let _ = reader.skip_bits(u64::from(payload_bits));
            return Err(EffectBlobError::ElementFieldCount { found: 3 });
        }

        // Order within the element is not assumed; the tag is the lower handle.
        let (lo, hi) = (seen[0].min(seen[1]), seen[0].max(seen[1]));
        if hi != lo + 1 {
            return Err(EffectBlobError::NonAdjacentHandles {
                first: seen[0],
                second: seen[1],
            });
        }
        match base {
            None => base = Some(lo),
            Some(known) if known != lo => {
                return Err(EffectBlobError::InconsistentHandleBase {
                    expected: known,
                    found: lo,
                });
            }
            Some(_) => {}
        }
    }

    Ok(base.map(EffectHandles::from_base))
}
