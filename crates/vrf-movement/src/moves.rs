//! The movement section and the move record inside it. A 3-bit marker out of
//! sequence means the cursor has drifted: an error, not skipped, since reading
//! on from a desynced position yields well-formed nonsense.

use vrf_bitio::BitReader;

use crate::error::MovementError;
use crate::types::{MovementMove, RpcDecodeResult};

/// Raw u16 angle to degrees; yaw and pitch match an independent parser exactly.
const ANGLE_SCALE: f64 = 360.0 / 65536.0;

/// Magic byte at the start of a movement section.
pub(crate) const MOVEMENT_MAGIC: u8 = 0x52;

/// Parse through the explicit zero marker. The caller then reads the input
/// component boundary and headers from the same logical payload window.
pub(crate) fn parse_movement_section(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
    result: &mut RpcDecodeResult,
    emit: &mut impl FnMut(MovementMove),
) -> Result<(), MovementError> {
    let magic = reader.read_u8()?;
    if magic != MOVEMENT_MAGIC {
        return Err(MovementError::InvalidMagic(magic));
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

        expected_marker = next_marker(expected_marker);
        marker = reader.read_bits(3)? as u8;
    }

    Ok(())
}

/// The marker expected after `marker`: 1 to 7, wrapping to 1 and never 0.
#[inline]
pub(crate) fn next_marker(marker: u8) -> u8 {
    let next = (marker + 1) & 7;
    if next < 2 { 1 } else { next }
}

/// Parse one MovementMove. The fields it drops all vary on real data; their
/// meaning is unknown.
#[inline(always)]
fn parse_single_move(
    reader: &mut BitReader<'_>,
    shooter_guid: u32,
) -> Result<MovementMove, MovementError> {
    let header = reader.read_bits(25)?;
    let move_type_flag = (header & 1) != 0; // bit 0
    let _rotation_yaw_multiplier = ((header >> 1) & 0xFF) as u8; // bits [1..9]
    let movement_state = ((header >> 9) & 0xFF) as u8; // bits [9..17]
    let _unused_byte = ((header >> 17) & 0xFF) as u8; // bits [17..25]

    reader.skip_bits(48)?; // rotationInput, not exported
    let timestamp = reader.read_int_packed()?;

    let [pos_x, pos_y, pos_z] = reader.read_quantized_vector(100)?;

    let has_optional = reader.read_bit()?;
    if has_optional {
        let _optional_byte = reader.read_u8()?;
    }

    let flag_and_angles = reader.read_bits(33)?;
    let _flag48 = (flag_and_angles & 1) != 0;
    let packed_angles = (flag_and_angles >> 1) as u32;
    let raw_pitch = (packed_angles & 0xFFFF) as u16;
    let raw_yaw = (packed_angles >> 16) as u16;

    let yaw = f64::from(raw_yaw) * ANGLE_SCALE;
    let pitch = f64::from(raw_pitch) * ANGLE_SCALE;

    let [vel_x, vel_y, vel_z] = if move_type_flag {
        let _variant1_flag = reader.read_bit()?;
        reader.read_quantized_vector(10)?
    } else {
        // Variant 0: 33-bit packed angles, no velocity.
        let variant0_data = reader.read_bits(33)?;
        let has_external_ref = (variant0_data & 1) != 0;
        if has_external_ref {
            return Err(MovementError::Variant0ExternalCharRef);
        }
        [0.0; 3]
    };

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
        move_type: u8::from(move_type_flag),
    })
}
