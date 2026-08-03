//! The movement section and the single move record inside it.
//!
//! This is the innermost layer of the RPC: everything above it is framing that
//! locates these bits, and everything below it (in [`crate::primitives`]) is
//! the numeric vocabulary they are written in.
//!
//! # The marker sequence
//!
//! Moves are separated by a 3-bit marker that counts 1, 2, 3, 4, 5, 6, 7, 2, 3,
//! ... -- it wraps at 7 back to 2, so 1 appears only once, at the start. A
//! marker that does not match the expected next value means the cursor has
//! drifted, and is reported rather than skipped: continuing from a desynced
//! position yields well-formed nonsense.

use vrf_bitio::BitReader;

use crate::error::MovementError;
use crate::primitives::{ANGLE_SCALE, read_fixed_vector, read_quantized_vector, read_vlq};
use crate::types::{MovementMove, RpcDecodeResult};

/// Magic byte at the start of a movement section.
pub(crate) const MOVEMENT_MAGIC: u8 = 0x52;

/// Maximum padding bits at the end of a movement section before we stop
/// looking for another marker.
const MAX_MOVEMENT_PADDING_BITS: u64 = 31;

/// Parse the movement section: magic byte, then a sequence of moves.
pub(crate) fn parse_movement_section(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    if reader.bits_remaining() < 8 {
        return Ok(());
    }

    let magic = reader.read_u8()?;
    if magic != MOVEMENT_MAGIC {
        return Err(MovementError::InvalidMagic(magic));
    }

    if reader.bits_remaining() < 3 {
        return Ok(());
    }

    let mut expected_marker: u8 = 1;
    let mut marker = reader.read_bits(3)? as u8;

    while marker != 0 {
        if marker != expected_marker {
            return Err(MovementError::MarkerMismatch {
                expected: expected_marker,
                actual: marker,
            });
        }

        let mv = parse_single_move(reader, shooter_guid)?;
        emit(mv);
        result.total_moves += 1;

        // Check if we're in trailing padding territory.
        if reader.bits_remaining() <= MAX_MOVEMENT_PADDING_BITS {
            return Ok(());
        }

        expected_marker = next_marker(expected_marker);
        marker = reader.read_bits(3)? as u8;
    }

    Ok(())
}

/// Compute the next expected marker in the sequence 1->2->3->4->5->6->7->2->3->...
///
/// The sequence wraps at 7 and skips 0 and 1 (except the initial 1).
#[inline]
pub(crate) fn next_marker(marker: u8) -> u8 {
    let next = (marker + 1) & 7;
    if next < 2 { 1 } else { next }
}

/// Parse one MovementMove from the stream.
fn parse_single_move(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
) -> Result<MovementMove, MovementError> {
    // -- 25-bit header ----------------------------------------------------
    let header = reader.read_bits(25)?;
    let move_type_flag = (header & 1) != 0; // bit 0
    let _rotation_yaw_multiplier = ((header >> 1) & 0xFF) as u8; // bits [1..9]
    let movement_state = ((header >> 9) & 0xFF) as u8; // bits [9..17]
    let _unused_byte = ((header >> 17) & 0xFF) as u8; // bits [17..25]

    // -- FixedVector: rotationInput (48 bits) -----------------------------
    let _rotation_input = read_fixed_vector(reader)?;

    // -- Timestamp (VLQ) --------------------------------------------------
    let timestamp = read_vlq(reader)?;

    // -- Position: QuantizedVector (scaleFactor=100) ----------------------
    let (pos_x, pos_y, pos_z) = read_quantized_vector(reader, 100)?;

    // -- Optional byte ----------------------------------------------------
    let has_optional = reader.read_bit()?;
    if has_optional {
        let _optional_byte = reader.read_u8()?;
    }

    // -- 33-bit flag + packed angles --------------------------------------
    let flag_and_angles = reader.read_bits(33)?;
    let _flag48 = (flag_and_angles & 1) != 0;
    let packed_angles = (flag_and_angles >> 1) as u32;
    let raw_pitch = (packed_angles & 0xFFFF) as u16;
    let raw_yaw = (packed_angles >> 16) as u16;

    let yaw = f64::from(raw_yaw) * ANGLE_SCALE;
    let pitch = f64::from(raw_pitch) * ANGLE_SCALE;

    // -- Variant-specific data --------------------------------------------
    let (vel_x, vel_y, vel_z) = if move_type_flag {
        // Variant 1: has velocity.
        let _variant1_flag = reader.read_bit()?;
        read_quantized_vector(reader, 10)?
    } else {
        // Variant 0: 33-bit packed angles (no velocity).
        let variant0_data = reader.read_bits(33)?;
        let has_external_ref = (variant0_data & 1) != 0;
        if has_external_ref {
            return Err(MovementError::Variant0ExternalCharRef);
        }
        (0.0, 0.0, 0.0)
    };

    // -- Error sentinel ---------------------------------------------------
    let error_sentinel = reader.read_bit()?;
    if error_sentinel {
        return Err(MovementError::ErrorSentinel);
    }

    Ok(MovementMove {
        shooter_character_net_guid: shooter_guid,
        pos_x,
        pos_y,
        pos_z,
        yaw,
        pitch,
        vel_x,
        vel_y,
        vel_z,
        timestamp,
        movement_state,
        mode_flags: movement_state, // same field in wire format
        move_type: if move_type_flag { 1 } else { 0 },
    })
}
