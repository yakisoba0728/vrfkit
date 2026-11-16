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
                    // moves to: a loss, not "no moves".
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
                    if decode_component_data_stream(&mut sub, guid, result, &mut component_emit)
                        .is_err()
                    {
                        result.error_count += 1;
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

/// The leading u16 is an envelope byte count iff it is non-zero and the
/// envelope fits, else movementBitCount. The choice is final: an inner failure
/// never rolls back to the other reading.
fn decode_component_data_stream(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let first_u16 = read_u16_checked(reader)?;

    let byte_count = u64::from(first_u16);
    if byte_count > 0 && reader.bits_remaining() >= byte_count * 8 {
        // Wrapped: the envelope's payload starts with its own movementBitCount.
        let mut inner = reader.sub_reader(byte_count * 8)?;
        // What follows the envelope is never read; tallied here, before the
        // section can fail, with the stream counted even when nothing follows.
        result.envelope_trailer_streams += 1;
        result.envelope_trailer_bits += reader.bits_remaining();
        let bit_count = read_u16_checked(&mut inner)?;
        parse_movement_with_bit_count(&mut inner, bit_count, shooter_guid, result, emit)
    } else {
        parse_movement_with_bit_count(reader, first_u16, shooter_guid, result, emit)
    }
}

/// Read a u16, failing with `TruncatedComponentHeader` rather than a bare Eof.
fn read_u16_checked(reader: &mut BitReader<'_>) -> Result<u16, MovementError> {
    if reader.bits_remaining() < 16 {
        return Err(MovementError::TruncatedComponentHeader {
            available_bits: reader.bits_remaining(),
        });
    }
    Ok(reader.read_u16()?)
}

/// Parse the movement section in a window of `movement_bit_count` bits, or of
/// all that remain when that is 0 or larger (an open window), tallying a tail.
fn parse_movement_with_bit_count(
    reader: &mut BitReader<'_>,
    movement_bit_count: u16,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let remaining = reader.bits_remaining();

    let uses_all_remaining = movement_bit_count == 0 || u64::from(movement_bit_count) > remaining;
    let bits = if uses_all_remaining {
        remaining
    } else {
        u64::from(movement_bit_count)
    };

    let mut movement_reader = reader.sub_reader(bits)?;
    // Outside the sized window, but still inside the component or envelope.
    // Count before parsing so a failed section cannot hide its trailer.
    let trailer_bits = reader.bits_remaining();
    if trailer_bits > 0 {
        result.sized_section_trailers += 1;
        result.sized_section_trailer_bits += trailer_bits;
    }
    let tail_bits = parse_movement_section(&mut movement_reader, shooter_guid, result, emit)?;
    if tail_bits > 0 {
        if uses_all_remaining {
            result.open_section_tails += 1;
            result.open_section_tail_bits += tail_bits;
        } else {
            result.sized_section_tails += 1;
            result.sized_section_tail_bits += tail_bits;
        }
    }
    Ok(())
}
