//! The RPC framing layers: batch -> updates array -> one update -> component
//! data stream (diagrams in the crate docs). A handle not decoded here is
//! skipped by its declared length, so an unknown field desynchronises nothing.

use vrf_bitio::BitReader;

use crate::error::MovementError;
use crate::moves::parse_movement_section;
use crate::types::{MovementContext, MovementMove, RpcDecodeResult};

/// Maximum number of character updates in a single RPC batch.
const MAX_REMOTE_CHARACTER_UPDATES: u32 = 256;

/// Property handles, `pub(crate)` so the tests build payloads from the same numbers.
pub(crate) const REMOTE_CHARACTER_UPDATES_HANDLE: u32 = 1;
pub(crate) const SHOOTER_CHARACTER_NET_GUID_HANDLE: u32 = 2;
pub(crate) const COMPONENT_DATA_STREAM_HANDLE: u32 = 3;

/// Decode the full movement RPC payload, calling `emit` for each decoded move
/// rather than collecting a Vec, so the caller can push straight to the
/// Parquet writer. `reader` must be bounded to the RPC payload's exact length.
pub fn decode_movement_rpc(
    reader: &mut BitReader<'_>,
    mut emit: impl FnMut(MovementMove),
) -> Result<RpcDecodeResult, MovementError> {
    decode_movement_rpc_with_context(reader, |movement, _| emit(movement))
}

/// Decode the same moves as [`decode_movement_rpc`], retaining their exact
/// array, update and component occurrence context. No move is merged or
/// reordered. Repeated indices remain separate arrivals; callers must not
/// infer final replacement semantics from emitted moves alone, since a later
/// empty or failed component need not emit a row.
///
/// `reader` must be bounded to the RPC payload's exact declared bit length.
pub fn decode_movement_rpc_with_context(
    reader: &mut BitReader<'_>,
    mut emit: impl FnMut(MovementMove, MovementContext),
) -> Result<RpcDecodeResult, MovementError> {
    let mut result = RpcDecodeResult::default();
    // The discarded first bit; without it the payload is empty.
    if reader.at_end() {
        return Ok(result);
    }
    let _ = reader.read_bit()?;

    let mut array_ordinal = 0;
    while !reader.at_end() {
        let encoded_handle = reader.read_int_packed()?;
        if encoded_handle == 0 {
            break;
        }
        let handle = encoded_handle - 1;
        let payload_bits = reader.read_int_packed()?;

        if handle != REMOTE_CHARACTER_UPDATES_HANDLE {
            reader.skip_bits(u64::from(payload_bits))?;
            continue;
        }

        let mut sub = reader.sub_reader(u64::from(payload_bits))?;
        decode_updates_array(&mut sub, array_ordinal, &mut result, &mut emit)?;
        array_ordinal += 1;
    }

    // A 0 handle before the end leaves the rest unread. A drift can make the
    // very first read return 0, which uncounted is exactly an empty RPC.
    if !reader.at_end() {
        result.error_count += 1;
    }

    Ok(result)
}

fn decode_updates_array(
    reader: &mut BitReader<'_>,
    array_ordinal: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove, MovementContext),
) -> Result<(), MovementError> {
    let update_count = reader.read_int_packed()?;
    if update_count > MAX_REMOTE_CHARACTER_UPDATES {
        return Err(MovementError::TooManyUpdates(update_count));
    }
    result.update_count = update_count;

    let mut update_ordinal = 0;
    loop {
        // Capacity is not the number of changed elements: only a zero index
        // ends a sparse array cleanly, even when no elements changed.
        if reader.at_end() {
            result.error_count += 1;
            break;
        }
        let encoded_index = reader.read_int_packed()?;
        if encoded_index == 0 {
            // Only a trailing 8-bit IntPacked may follow (never seen); any
            // other remainder is lost updates or a drifted cursor.
            if (reader.bits_remaining() == 8 && reader.read_int_packed().is_err())
                || !reader.at_end()
            {
                result.error_count += 1;
            }
            break;
        }

        // An index (`encoded_index - 1`) past the declared count, or a failed
        // framing read (a handle, a length, or a field longer than the window),
        // after which the next index cannot be located: the rest is lost, counted.
        let context = MovementContext {
            array_ordinal,
            update_ordinal,
            update_index: encoded_index - 1,
            component_ordinal: 0,
            move_index: 0,
        };
        if encoded_index > update_count
            || decode_single_update(reader, context, result, emit).is_err()
        {
            result.error_count += 1;
            break;
        }
        update_ordinal += 1;
    }

    Ok(())
}

fn decode_single_update(
    reader: &mut BitReader<'_>,
    mut context: MovementContext,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove, MovementContext),
) -> Result<(), MovementError> {
    let mut shooter_guid: Option<u32> = None;
    while !reader.at_end() {
        let encoded_handle = reader.read_int_packed()?;
        if encoded_handle == 0 {
            break;
        }
        let handle = encoded_handle - 1;
        let payload_bits = reader.read_int_packed()?;

        match handle {
            SHOOTER_CHARACTER_NET_GUID_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if payload_bits == 32 {
                    shooter_guid = Some(sub.read_u32()?);
                } else {
                    // Not a u32. The field is consumed, so the framing
                    // survives, but the update has no character to attribute
                    // later moves to, even if an earlier occurrence was
                    // valid: a loss, not "no moves".
                    shooter_guid = None;
                    result.error_count += 1;
                }
            }
            COMPONENT_DATA_STREAM_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if let Some(guid) = shooter_guid {
                    // `sub_reader` has already moved `reader` past the whole
                    // stream, so a failure inside it cannot misplace the next
                    // handle: count it and go on.
                    context.move_index = 0;
                    let mut component_emit = |movement| {
                        emit(movement, context);
                        context.move_index += 1;
                    };
                    result.component_streams += 1;
                    if decode_component_data_stream(&mut sub, guid, result, &mut component_emit)
                        .is_err()
                    {
                        result.error_count += 1;
                        result.component_errors += 1;
                    } else {
                        result.component_complete += 1;
                    }
                } else {
                    // No GUID: handle 2 was not 32 bits (counted above) or has
                    // not arrived, and a single pass cannot rewind to it, so
                    // the moves are dropped. An update hitting both adds two.
                    result.error_count += 1;
                }
                context.component_ordinal += 1;
            }
            _ => {
                reader.skip_bits(u64::from(payload_bits))?;
            }
        }
    }

    Ok(())
}

/// Native byte array: a signed 32-bit byte count, exactly that many payload
/// bytes and one unused-bit-count byte. The logical payload excludes the high
/// unused bits, then contains movement, an absolute input end bit and headers.
fn decode_component_data_stream(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    if reader.bits_remaining() < 32 {
        return Err(MovementError::TruncatedComponentHeader {
            available_bits: reader.bits_remaining(),
        });
    }
    let count = reader.read_i32()?;
    if count < 0 {
        return Err(MovementError::NegativeComponentByteCount { count });
    }
    let byte_count = count as u32;
    let payload_bits = u64::from(byte_count) * 8;
    if reader.bits_remaining() != payload_bits + 8 {
        return Err(MovementError::ComponentLengthMismatch {
            byte_count,
            available_bits: reader.bits_remaining(),
        });
    }
    let mut payload = reader.sub_reader(payload_bits)?;
    let unused_bits = reader.read_u8()?;
    if unused_bits > 7 || (byte_count == 0 && unused_bits != 0) {
        return Err(MovementError::InvalidUnusedBitCount {
            byte_count,
            unused_bits,
        });
    }
    result.component_envelopes += 1;
    result.component_payload_bytes += u64::from(byte_count);
    result.component_unused_bits += u64::from(unused_bits);
    if byte_count == 0 {
        result.component_empty += 1;
        return Ok(());
    }

    let mut components = payload.sub_reader(payload_bits - u64::from(unused_bits))?;
    parse_movement_section(&mut components, shooter_guid, result, emit)?;
    let end_bit = components.read_u16()?;
    let minimum = components.position() + 8;
    if u64::from(end_bit) < minimum || u64::from(end_bit) > components.len_bits() {
        return Err(MovementError::InvalidInputBoundary {
            end_bit,
            minimum,
            payload_bits: components.len_bits(),
        });
    }
    let mut input = components.sub_reader(u64::from(end_bit) - components.position())?;
    read_component_header(&mut input, 0xA3, "input")?;
    result.component_input_headers += 1;
    require_component_end(&input, "input")?;
    if !components.at_end() {
        read_component_header(&mut components, 0xA4, "discrepancy")?;
        result.component_discrepancy_headers += 1;
    }
    require_component_end(&components, "discrepancy")?;
    Ok(())
}

fn read_component_header(
    reader: &mut BitReader<'_>,
    expected: u8,
    component: &'static str,
) -> Result<(), MovementError> {
    let actual = reader.read_u8()?;
    if actual != expected {
        return Err(MovementError::UnknownComponentMagic { component, actual });
    }
    Ok(())
}

fn require_component_end(
    reader: &BitReader<'_>,
    component: &'static str,
) -> Result<(), MovementError> {
    if !reader.at_end() {
        return Err(MovementError::UnparsedComponentBody {
            component,
            remaining_bits: reader.bits_remaining(),
        });
    }
    Ok(())
}
