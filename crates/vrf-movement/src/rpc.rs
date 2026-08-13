//! The RPC framing layers: batch -> updates array -> one update -> the
//! component data stream that finally holds the movement section.
//!
//! Four nested property-style loops, each of the shape
//! `encodedHandle : IntPacked` (0 terminates), `payloadBits : IntPacked`,
//! then that many bits. Handles this crate does not decode are skipped by
//! their declared length rather than guessed at, which is what lets an unknown
//! future field pass through without desynchronising the ones around it.

use vrf_bitio::BitReader;

use crate::error::MovementError;
use crate::moves::parse_movement_section;
use crate::types::{MovementMove, RpcDecodeResult};

/// Maximum number of character updates in a single RPC batch.
const MAX_REMOTE_CHARACTER_UPDATES: u32 = 256;

/// Handle constants for the property-style framing inside the RPC.
///
/// `pub(crate)` so the round-trip tests can build a payload using the same
/// numbers the decoder matches on, rather than restating them.
pub(crate) const REMOTE_CHARACTER_UPDATES_HANDLE: u32 = 1;
pub(crate) const SHOOTER_CHARACTER_NET_GUID_HANDLE: u32 = 2;
pub(crate) const COMPONENT_DATA_STREAM_HANDLE: u32 = 3;

/// Decode the full movement RPC payload, calling `emit` for each decoded move.
///
/// The `reader` should be bounded to the exact bit length of the RPC payload.
///
/// # Streaming design
///
/// Calls `emit` for each move rather than collecting into a Vec.
/// This allows the caller to push directly to the Parquet writer.
pub fn decode_movement_rpc(
    reader: &mut BitReader<'_>,
    mut emit: impl FnMut(MovementMove),
) -> Result<RpcDecodeResult, MovementError> {
    let end_bit = reader.len_bits();

    // First bit: consumed but value ignored (C# discards via `TryReadBit(out _)`).
    // If no bits remain, the payload is empty.
    if reader.bits_remaining() == 0 {
        return Ok(RpcDecodeResult {
            total_moves: 0,
            update_count: 0,
            error_count: 0,
        });
    }
    let _ = reader.read_bit()?; // consume and discard

    let mut result = RpcDecodeResult {
        total_moves: 0,
        update_count: 0,
        error_count: 0,
    };

    // Property-style framing: loop over handles.
    while reader.position() < end_bit {
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
        decode_updates_array(&mut sub, &mut result, &mut emit)?;
    }

    Ok(result)
}

/// Decode the RemoteCharacterUpdates array.
fn decode_updates_array(
    reader: &mut BitReader<'_>,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let end_bit = reader.len_bits();
    let update_count = reader.read_int_packed()?;

    if update_count > MAX_REMOTE_CHARACTER_UPDATES {
        return Err(MovementError::TooManyUpdates(update_count));
    }

    result.update_count = update_count;

    while reader.position() < end_bit {
        let encoded_index = reader.read_int_packed()?;
        if encoded_index == 0 {
            // Trailing padding: if exactly 8 bits remain, consume IntPacked.
            //
            // The result used to be dropped with `let _ =`. A single `0x01`
            // here sets the continuation bit and demands a byte the window does
            // not have, so the read fails -- and the RPC reported success with
            // `error_count == 0` anyway. The read is still allowed to fail
            // (these are padding bits; nothing downstream depends on them) but
            // a tail that does not parse is evidence the grammar has drifted,
            // so it is counted rather than swallowed.
            if end_bit.saturating_sub(reader.position()) == 8 && reader.read_int_packed().is_err() {
                result.error_count += 1;
            }
            break;
        }

        let index = encoded_index - 1;
        if index >= update_count {
            // The index addresses an update the array never declared, so the
            // position of the next handle is unknown and the tail has to go.
            // That part is unchanged; what was missing is any trace of it.
            // `Ok(update_count: n, total_moves: 0, error_count: 0)` is
            // indistinguishable from a batch of well-formed empty updates.
            result.error_count += 1;
            reader.skip_remaining();
            break;
        }

        if decode_single_update(reader, result, emit).is_err() {
            result.error_count += 1;
            // After a parse error we cannot reliably continue (bit position
            // is indeterminate). Skip remaining bits in this array.
            reader.skip_remaining();
            break;
        }
    }

    Ok(())
}

/// Decode one RemoteCharacterUpdate.
fn decode_single_update(
    reader: &mut BitReader<'_>,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let end_bit = reader.len_bits();
    let mut shooter_guid: Option<u32> = None;

    while reader.position() < end_bit {
        let encoded_handle = reader.read_int_packed()?;
        if encoded_handle == 0 {
            break;
        }
        let handle = encoded_handle - 1;
        let payload_bits = reader.read_int_packed()?;

        if u64::from(payload_bits) > reader.bits_remaining() {
            // The field claims more bits than the whole updates window still
            // holds, so this framing no longer describes the payload and the
            // next handle cannot be located. Abandoning the window is right;
            // reporting it as a clean end-of-update was not -- every update
            // still queued behind this one goes with it.
            result.error_count += 1;
            reader.skip_remaining();
            break;
        }

        match handle {
            SHOOTER_CHARACTER_NET_GUID_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if payload_bits >= 32 {
                    shooter_guid = Some(sub.read_u32()?);
                } else {
                    // Too narrow to hold the u32 it must carry. The field is
                    // consumed either way, so the framing survives -- but the
                    // update now has no character to attribute moves to, which
                    // is a loss and not a shape of "no moves present".
                    result.error_count += 1;
                }
            }
            COMPONENT_DATA_STREAM_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if let Some(guid) = shooter_guid {
                    decode_component_data_stream(&mut sub, guid, result, emit)?;
                } else {
                    // A stream with no GUID: either handle 2 was undersized
                    // (counted just above) or it has not arrived yet. The
                    // decoder is single-pass and cannot rewind to it, so the
                    // moves in this stream are dropped. Counted per occurrence,
                    // so an update that hits both paths contributes two.
                    result.error_count += 1;
                }
            }
            _ => {
                reader.skip_bits(u64::from(payload_bits))?;
            }
        }
    }

    Ok(())
}

/// Decode a ComponentDataStream.
///
/// The C# parser uses a checkpoint to try byte-wrapped parsing first, then
/// falls back. Since our BitReader cannot rewind, we implement this by
/// peeking at the structure: read the first u16 and check if it looks like
/// a valid byte-count wrapper. If so, parse inner. Otherwise, treat the u16
/// as the movementBitCount for direct parsing.
///
/// Key insight: both paths start by reading a u16. In byte-wrapped mode, it's
/// the byte count of the outer envelope. In direct mode, it's the
/// movementBitCount. The C# checkpoint-rollback pattern is equivalent to:
/// "if the first u16 passes the byte-wrapped validity check, use it as byte
/// count; otherwise reinterpret it as movementBitCount."
fn decode_component_data_stream(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    if reader.bits_remaining() < 16 {
        return Err(MovementError::TruncatedComponentHeader {
            available_bits: reader.bits_remaining(),
        });
    }

    let first_u16 = reader.read_u16()?;

    // Check if this could be a byte-wrapped envelope:
    // The byte count must be > 0 and byte_count * 8 must fit in remaining bits.
    let byte_count = u64::from(first_u16);
    if byte_count > 0 && reader.bits_remaining() >= byte_count * 8 {
        // Looks like a byte-wrapped envelope. Parse inner component payload.
        let mut inner = reader.sub_reader(byte_count * 8)?;
        return parse_component_payload(&mut inner, shooter_guid, result, emit);
    }

    // Not byte-wrapped: first_u16 is the movementBitCount.
    parse_movement_with_bit_count(reader, first_u16, shooter_guid, result, emit)
}

/// Parse the component payload (inside byte-wrapper or at top level).
///
/// Reads a u16 movementBitCount, then the movement section.
fn parse_component_payload(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    if reader.bits_remaining() < 16 {
        return Err(MovementError::TruncatedComponentHeader {
            available_bits: reader.bits_remaining(),
        });
    }

    let movement_bit_count = reader.read_u16()?;
    parse_movement_with_bit_count(reader, movement_bit_count, shooter_guid, result, emit)
}

/// Common logic after reading movementBitCount.
fn parse_movement_with_bit_count(
    reader: &mut BitReader<'_>,
    movement_bit_count: u16,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let remaining = reader.bits_remaining();

    if movement_bit_count == 0 || u64::from(movement_bit_count) > remaining {
        // Movement uses all remaining bits.
        let mut movement_reader = reader.sub_reader(remaining)?;
        parse_movement_section(&mut movement_reader, shooter_guid, result, emit)?;
    } else {
        // Movement uses exactly movementBitCount bits.
        let mut movement_reader = reader.sub_reader(u64::from(movement_bit_count))?;
        parse_movement_section(&mut movement_reader, shooter_guid, result, emit)?;
        // Skip any remaining bits after movement section.
        reader.skip_remaining();
    }

    Ok(())
}
