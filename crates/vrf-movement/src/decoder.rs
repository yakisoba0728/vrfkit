//! Core movement decoder -- translates the binary RPC payload into structured
//! movement records.
//!
//! The decoder is streaming: it yields moves via a callback rather than
//! collecting them, so callers can push directly to the Parquet writer without
//! buffering millions of rows.

use vrf_bitio::BitReader;

use crate::error::MovementError;

// -- Constants ----------------------------------------------------------------

/// Magic byte at the start of a movement section.
const MOVEMENT_MAGIC: u8 = 0x52;

/// Scale for FixedVector: each u16 component maps to signed range via (raw - 0x8000) / 65536.
const FIXED_VECTOR_SCALE: f64 = 1.0 / 65536.0;

/// Angle conversion: raw u16 -> degrees.
const ANGLE_SCALE: f64 = 360.0 / 65536.0;

/// Maximum padding bits at the end of a movement section before we stop
/// looking for another marker.
const MAX_MOVEMENT_PADDING_BITS: u64 = 31;

/// Maximum number of character updates in a single RPC batch.
const MAX_REMOTE_CHARACTER_UPDATES: u32 = 256;

/// Handle constants for the property-style framing inside the RPC.
const REMOTE_CHARACTER_UPDATES_HANDLE: u32 = 1;
const SHOOTER_CHARACTER_NET_GUID_HANDLE: u32 = 2;
const COMPONENT_DATA_STREAM_HANDLE: u32 = 3;

// -- Public types -------------------------------------------------------------

/// A single decoded movement sample (one "move" from one character update).
#[derive(Debug, Clone, Copy)]
pub struct MovementMove {
    /// The character's network GUID (identifies which player/character).
    pub shooter_character_net_guid: u32,
    /// Position in Unreal world coordinates (cm).
    pub pos_x: f64,
    pub pos_y: f64,
    pub pos_z: f64,
    /// Yaw in degrees [0, 360).
    pub yaw: f64,
    /// Pitch in degrees [0, 360).
    pub pitch: f64,
    /// Velocity (cm/s). Only present for variant-1 moves; zero for variant-0.
    pub vel_x: f64,
    pub vel_y: f64,
    pub vel_z: f64,
    /// Server-assigned timestamp (VLQ-encoded tick).
    pub timestamp: u32,
    /// Movement state byte.
    pub movement_state: u8,
    /// Mode flags byte (same as movement_state in the wire format).
    pub mode_flags: u8,
    /// 0 = variant0, 1 = variant1.
    pub move_type: u8,
}

/// A single character update descriptor (carries moves).
#[derive(Debug, Clone)]
pub struct MovementUpdate {
    /// Index within the batch.
    pub index: u32,
    /// The character GUID this update belongs to.
    pub shooter_character_net_guid: Option<u32>,
    /// Number of moves decoded for this update.
    pub move_count: u32,
}

/// Result of decoding the full RPC payload.
#[derive(Debug, Clone, Copy)]
pub struct RpcDecodeResult {
    /// Total moves decoded across all updates.
    pub total_moves: u32,
    /// Number of character updates in the batch.
    pub update_count: u32,
    /// Number of updates that had parse errors.
    pub error_count: u32,
}

// -- Top-level RPC decoder ----------------------------------------------------

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

// -- Array-level decoder ------------------------------------------------------

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
            if end_bit.saturating_sub(reader.position()) == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        }

        let index = encoded_index - 1;
        if index >= update_count {
            reader.skip_remaining();
            break;
        }

        match decode_single_update(reader, result, emit) {
            Ok(()) => {}
            Err(_) => {
                result.error_count += 1;
                // After a parse error we cannot reliably continue (bit position
                // is indeterminate). Skip remaining bits in this array.
                reader.skip_remaining();
                break;
            }
        }
    }

    Ok(())
}

// -- Single character update --------------------------------------------------

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
            reader.skip_remaining();
            break;
        }

        match handle {
            SHOOTER_CHARACTER_NET_GUID_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if payload_bits >= 32 {
                    shooter_guid = Some(sub.read_u32()?);
                }
            }
            COMPONENT_DATA_STREAM_HANDLE => {
                let mut sub = reader.sub_reader(u64::from(payload_bits))?;
                if let Some(guid) = shooter_guid {
                    decode_component_data_stream(&mut sub, guid, result, emit)?;
                }
            }
            _ => {
                reader.skip_bits(u64::from(payload_bits))?;
            }
        }
    }

    Ok(())
}

// -- ComponentDataStream ------------------------------------------------------

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
    let end_bit = reader.len_bits();

    if reader.bits_remaining() < 16 {
        return Ok(());
    }

    let first_u16 = reader.read_u16()?;

    // Check if this could be a byte-wrapped envelope:
    // The byte count must be > 0 and byte_count * 8 must fit in remaining bits.
    let byte_count = first_u16 as u64;
    if byte_count > 0 && reader.bits_remaining() >= byte_count * 8 {
        // Looks like a byte-wrapped envelope. Parse inner component payload.
        let mut inner = reader.sub_reader(byte_count * 8)?;
        parse_component_payload(&mut inner, shooter_guid, result, emit)?;
        return Ok(());
    }

    // Not byte-wrapped: first_u16 is the movementBitCount.
    parse_movement_with_bit_count(reader, first_u16, end_bit, shooter_guid, result, emit)
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
    let end_bit = reader.len_bits();

    if reader.bits_remaining() < 16 {
        return Ok(());
    }

    let movement_bit_count = reader.read_u16()?;
    parse_movement_with_bit_count(
        reader,
        movement_bit_count,
        end_bit,
        shooter_guid,
        result,
        emit,
    )
}

/// Common logic after reading movementBitCount.
fn parse_movement_with_bit_count(
    reader: &mut BitReader<'_>,
    movement_bit_count: u16,
    _end_bit: u64,
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

// -- Movement section ---------------------------------------------------------

/// Parse the movement section: magic byte, then a sequence of moves.
fn parse_movement_section(
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
fn next_marker(marker: u8) -> u8 {
    let next = (marker + 1) & 7;
    if next < 2 { 1 } else { next }
}

// -- Single move record -------------------------------------------------------

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
        let (vx, vy, vz) = read_quantized_vector(reader, 10)?;
        (vx, vy, vz)
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

// -- Vector primitives --------------------------------------------------------

/// Read a FixedVector: 3 x u16 packed into 48 bits.
///
/// Each component is sign-offset: `(raw - 0x8000) * (1/65536)`.
/// Result is in range approximately [-0.5, +0.5).
fn read_fixed_vector(reader: &mut BitReader<'_>) -> Result<(f64, f64, f64), MovementError> {
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
fn read_quantized_vector(
    reader: &mut BitReader<'_>,
    scale_factor: i32,
) -> Result<(f64, f64, f64), MovementError> {
    // SerializedInt(128) -- uses up to 7 bits.
    let info = reader.read_serialized_int(128)? as u64;
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

// -- VLQ reader ---------------------------------------------------------------

/// Read a VLQ-encoded u32.
///
/// This is a custom VLQ (not Unreal's IntPacked!). Each byte:
/// - bits [1..8] (7 bits): payload, shifted left by `7 * byte_index`
/// - bit [0]: continuation flag (1 = more bytes follow)
///
/// This is the same encoding as IntPacked but with the bit order reversed
/// within each byte (IntPacked: high 7 are payload, low 1 is continue;
/// VLQ here: low 1 is continue, high 7 shifted right by 1 are payload).
///
/// Wait -- looking at C# more carefully, it's the same as IntPacked:
/// `value |= (uint)(((b >> 1) & 0x7F) << shift)` and `(b & 1) == 0` means stop.
/// That IS IntPacked. Let me just use IntPacked.
fn read_vlq(reader: &mut BitReader<'_>) -> Result<u32, MovementError> {
    // This is identical to IntPacked encoding.
    Ok(reader.read_int_packed()?)
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a bit vector from individual bit values, then convert to bytes.
    struct BitWriter {
        bits: Vec<bool>,
    }

    impl BitWriter {
        fn new() -> Self {
            Self { bits: Vec::new() }
        }

        fn write_bit(&mut self, v: bool) {
            self.bits.push(v);
        }

        fn write_bits_u64(&mut self, value: u64, count: u32) {
            for i in 0..count {
                self.bits.push((value >> i) & 1 != 0);
            }
        }

        fn write_u8(&mut self, v: u8) {
            self.write_bits_u64(u64::from(v), 8);
        }

        fn write_u16(&mut self, v: u16) {
            self.write_bits_u64(u64::from(v), 16);
        }

        fn write_u32(&mut self, v: u32) {
            self.write_bits_u64(u64::from(v), 32);
        }

        fn write_f32(&mut self, v: f32) {
            self.write_u32(v.to_bits());
        }

        fn write_int_packed(&mut self, mut value: u32) {
            loop {
                let mut next_byte = ((value & 0x7F) << 1) as u8;
                value >>= 7;
                if value != 0 {
                    next_byte |= 1;
                }
                self.write_u8(next_byte);
                if value == 0 {
                    break;
                }
            }
        }

        fn write_serialized_int(&mut self, value: u32, max_value: u32) {
            let mut written_value = 0u32;
            let mut mask = 1u32;
            while written_value.saturating_add(mask) < max_value {
                let bit = (value & mask) != 0;
                self.write_bit(bit);
                if bit {
                    written_value |= mask;
                }
                mask <<= 1;
            }
        }

        fn write_other(&mut self, other: &BitWriter) {
            self.bits.extend_from_slice(&other.bits);
        }

        fn bit_count(&self) -> u32 {
            self.bits.len() as u32
        }

        fn to_bytes(&self) -> Vec<u8> {
            let byte_count = self.bits.len().div_ceil(8);
            let mut bytes = vec![0u8; byte_count];
            for (i, &bit) in self.bits.iter().enumerate() {
                if bit {
                    bytes[i >> 3] |= 1 << (i & 7);
                }
            }
            bytes
        }
    }

    /// Build a single move payload (variant 0 or variant 1).
    fn build_move(variant1: bool, timestamp: u32, x: f32, y: f32, z: f32) -> BitWriter {
        let mut w = BitWriter::new();

        // 25-bit header: moveType(1) + rotationYawMultiplier(8) + movementState(8) + unused(8)
        w.write_bit(variant1); // moveType
        w.write_u8(2); // rotationYawMultiplier
        w.write_u8(3); // movementState
        w.write_u8(0); // unused

        // FixedVector rotationInput: 3 x u16 = 48 bits (all zero = center)
        w.write_serialized_int(0x8000, 0x10000);
        w.write_serialized_int(0x8000, 0x10000);
        w.write_serialized_int(0x8000, 0x10000);

        // Timestamp VLQ (= IntPacked)
        w.write_int_packed(timestamp);

        // Position: QuantizedVector(scaleFactor=100)
        // Use componentBits=0, extraInfo=0 -> 3 x f32
        w.write_serialized_int(0, 128); // info = 0 -> componentBits=0, extraInfo=0
        w.write_f32(x);
        w.write_f32(y);
        w.write_f32(z);

        // hasOptionalByte = false
        w.write_bit(false);

        // 33-bit flag+packedAngles: flag48(1) + packedAngles(32)
        w.write_bit(false); // flag48
        w.write_u32(0); // packedAngles (pitch=0, yaw=0)

        if variant1 {
            // variant1Flag + quantized velocity
            w.write_bit(true);
            // QuantizedVector(scaleFactor=10): componentBits=10, extraInfo=1
            let info = 10u32 | (1 << 6); // componentBits=10, extraInfo=1
            w.write_serialized_int(info, 128);
            // 3 signed components of 10 bits each = 30 bits total
            // velocity = (4.0, 5.0, 6.0) -> scaled by 10 = (40, 50, 60)
            let vx = 40i64 as u64 & 0x3FF;
            let vy = 50i64 as u64 & 0x3FF;
            let vz = 60i64 as u64 & 0x3FF;
            let packed = vx | (vy << 10) | (vz << 20);
            w.write_bits_u64(packed, 30);
        } else {
            // variant0: 33-bit flag+angles
            w.write_bit(false); // hasExternalCharacterRef = false
            w.write_u32(0); // variant0PackedAngles
        }

        // errorSentinel = false
        w.write_bit(false);

        w
    }

    /// Build a ComponentDataStream payload (direct, not byte-wrapped).
    fn build_component_data_stream(moves: &[BitWriter]) -> BitWriter {
        let mut movement = BitWriter::new();
        movement.write_u8(MOVEMENT_MAGIC);

        let mut marker: u8 = 1;
        for (i, mv) in moves.iter().enumerate() {
            movement.write_bits_u64(u64::from(marker), 3);
            movement.write_other(mv);
            if i + 1 < moves.len() {
                marker = next_marker(marker);
            }
        }
        // Terminal marker = 0 (only if we haven't hit padding)
        if !moves.is_empty() {
            movement.write_bits_u64(0, 3);
        }

        let mut payload = BitWriter::new();
        payload.write_u16(movement.bit_count() as u16); // movementBitCount
        payload.write_other(&movement);
        payload
    }

    /// Build a full RPC payload with one character update.
    fn build_rpc_payload(shooter_guid: u32, component_stream: &BitWriter) -> BitWriter {
        // Build the single update's property stream
        let mut update = BitWriter::new();
        // handle 2 (ShooterCharacterNetGuidValue): encodedHandle=3, payload=32 bits
        update.write_int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1);
        update.write_int_packed(32);
        update.write_u32(shooter_guid);
        // handle 3 (ComponentDataStream): encodedHandle=4, payload=stream bits
        update.write_int_packed(COMPONENT_DATA_STREAM_HANDLE + 1);
        update.write_int_packed(component_stream.bit_count());
        update.write_other(component_stream);
        // terminator
        update.write_int_packed(0);

        // Build the updates array
        let mut array = BitWriter::new();
        array.write_int_packed(1); // updateCount = 1
        array.write_int_packed(1); // encodedIndex = 1 -> index 0
        array.write_other(&update);
        array.write_int_packed(0); // array terminator

        // Build the RPC wrapper
        let mut rpc = BitWriter::new();
        rpc.write_bit(false); // first bit (consumed, value discarded per C# TryReadBit(out _))
        // Property-style: handle 1 (RemoteCharacterUpdates)
        rpc.write_int_packed(REMOTE_CHARACTER_UPDATES_HANDLE + 1); // encodedHandle = 2
        rpc.write_int_packed(array.bit_count()); // payload bits
        rpc.write_other(&array);
        rpc.write_int_packed(0); // terminator

        rpc
    }

    #[test]
    fn decodes_single_variant0_move() {
        let mv = build_move(false, 42, 1.25, 2.5, 3.75);
        let stream = build_component_data_stream(&[mv]);
        let rpc = build_rpc_payload(1234, &stream);
        let bytes = rpc.to_bytes();
        let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

        let mut moves = Vec::new();
        let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

        assert_eq!(result.total_moves, 1);
        assert_eq!(result.update_count, 1);
        assert_eq!(result.error_count, 0);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].shooter_character_net_guid, 1234);
        assert_eq!(moves[0].move_type, 0);
        assert_eq!(moves[0].timestamp, 42);
        assert!((moves[0].pos_x - 1.25).abs() < 0.001);
        assert!((moves[0].pos_y - 2.5).abs() < 0.001);
        assert!((moves[0].pos_z - 3.75).abs() < 0.001);
        assert_eq!(moves[0].vel_x, 0.0);
        assert_eq!(moves[0].vel_y, 0.0);
        assert_eq!(moves[0].vel_z, 0.0);
    }

    #[test]
    fn decodes_single_variant1_move_with_velocity() {
        let mv = build_move(true, 42, 1.25, 2.5, 3.75);
        let stream = build_component_data_stream(&[mv]);
        let rpc = build_rpc_payload(5678, &stream);
        let bytes = rpc.to_bytes();
        let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

        let mut moves = Vec::new();
        let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

        assert_eq!(result.total_moves, 1);
        assert_eq!(moves[0].move_type, 1);
        assert!((moves[0].vel_x - 4.0).abs() < 0.001);
        assert!((moves[0].vel_y - 5.0).abs() < 0.001);
        assert!((moves[0].vel_z - 6.0).abs() < 0.001);
    }

    #[test]
    fn decodes_two_moves_in_one_update() {
        let mv1 = build_move(false, 42, 1.0, 2.0, 3.0);
        let mv2 = build_move(false, 84, 10.0, 11.0, 12.0);
        let stream = build_component_data_stream(&[mv1, mv2]);
        let rpc = build_rpc_payload(9999, &stream);
        let bytes = rpc.to_bytes();
        let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

        let mut moves = Vec::new();
        let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

        assert_eq!(result.total_moves, 2);
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].timestamp, 42);
        assert!((moves[0].pos_x - 1.0).abs() < 0.001);
        assert_eq!(moves[1].timestamp, 84);
        assert!((moves[1].pos_x - 10.0).abs() < 0.001);
    }

    #[test]
    fn empty_rpc_returns_zero() {
        // Zero bits -> empty
        let data = [0u8; 0];
        let mut reader = BitReader::with_bit_len(&data, 0);

        let mut moves = Vec::new();
        let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();
        assert_eq!(result.total_moves, 0);
        assert!(moves.is_empty());
    }

    #[test]
    fn invalid_magic_returns_error() {
        // Build a stream with wrong magic
        let mut movement = BitWriter::new();
        movement.write_u8(0x00); // wrong magic

        let mut payload = BitWriter::new();
        payload.write_u16(movement.bit_count() as u16);
        payload.write_other(&movement);

        let rpc = build_rpc_payload(1234, &payload);
        let bytes = rpc.to_bytes();
        let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

        let mut moves = Vec::new();
        let result = decode_movement_rpc(&mut reader, |m| moves.push(m));
        // The error should be caught at the update level, incrementing error_count.
        // Since we catch errors in decode_single_update, it returns Ok with error_count > 0.
        match result {
            Ok(r) => assert_eq!(r.error_count, 1),
            Err(MovementError::InvalidMagic(0x00)) => {} // also acceptable
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
