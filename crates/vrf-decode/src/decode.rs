//! Field type enumeration, decoded value representation, and the dispatch decoder.
//!
//! This module is the core of the type overlay: given a [`FieldType`] and raw
//! bits, it produces a [`DecodedValue`] or a [`DecodeError`].

use vrf_bitio::BitReader;

use crate::types::{FQuat, FRepMovement, FRotator, FTransform, FVector, RotatorQuantization};

/// Every primitive type the overlay can decode.
///
/// Parametric variants carry their configuration inline so the overlay table
/// needs no side data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldType {
    Bool,
    Byte,
    EnumByte,
    Int32,
    UInt32,
    UInt64,
    Float,
    Double,
    FString,
    FName,
    ObjectNetGuid,
    Guid,
    SerializedInt {
        max: u32,
    },
    EnumRemainingBits,
    GameplayTag,
    ByteArray {
        max_bytes: u32,
    },
    VectorFloat,
    VectorDouble,
    VectorNetQuantize {
        scale: u32,
    },
    VectorNetQuantizeNormal,
    RotationShort,
    RotationByte,
    Transform,
    RepMovement {
        rotation: RotatorQuantization,
    },
    /// Dynamic arrays and custom decoders — not decoded, raw_bits suffices.
    Raw,
    /// Explicitly skipped fields (`.Ignore()` in descriptors).
    Skip,
}

/// The result of a successful decode. Exactly one variant is populated;
/// the caller maps it to the appropriate `value_*` column.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
}

/// Decode failure. Non-fatal: the field stays as raw_bits only.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DecodeError {
    #[error("bit read error: {0}")]
    BitIo(#[from] vrf_bitio::BitError),
    #[error("not fully consumed: {remaining} bits left after decode")]
    NotFullyConsumed { remaining: u64 },
    #[error("field type is Raw/Skip -- no decode attempted")]
    RawOrSkip,
}

/// Decode raw bits according to the given [`FieldType`].
///
/// Returns `Err(DecodeError::RawOrSkip)` for `Raw` and `Skip` types — these
/// are never decoded and the caller should leave `value_*` as null.
///
/// On success the reader is fully consumed. If bits remain after decoding,
/// `Err(DecodeError::NotFullyConsumed)` is returned (indicates a layout
/// mismatch, e.g. game-version drift).
pub fn decode_field(
    field_type: FieldType,
    data: &[u8],
    bit_count: u32,
) -> Result<DecodedValue, DecodeError> {
    if matches!(field_type, FieldType::Raw | FieldType::Skip) {
        return Err(DecodeError::RawOrSkip);
    }
    let mut reader = BitReader::with_bit_len(data, u64::from(bit_count));
    let value = dispatch_decode(field_type, &mut reader, bit_count)?;
    let remaining = reader.bits_remaining();
    if remaining != 0 && !matches!(field_type, FieldType::EnumRemainingBits) {
        return Err(DecodeError::NotFullyConsumed { remaining });
    }
    Ok(value)
}

fn dispatch_decode(
    ft: FieldType,
    r: &mut BitReader<'_>,
    bit_count: u32,
) -> Result<DecodedValue, DecodeError> {
    match ft {
        FieldType::Bool => decode_bool(r),
        FieldType::Byte | FieldType::EnumByte => decode_byte(r),
        FieldType::Int32 => decode_i32(r),
        FieldType::UInt32 => decode_u32(r),
        FieldType::UInt64 => decode_u64(r),
        FieldType::Float => decode_float(r),
        FieldType::Double => decode_double(r),
        FieldType::FString => decode_fstring(r),
        FieldType::FName => decode_fname(r),
        FieldType::ObjectNetGuid => decode_object_net_guid(r),
        FieldType::Guid => decode_guid(r),
        FieldType::SerializedInt { max } => decode_serialized_int(r, max),
        FieldType::EnumRemainingBits => decode_enum_remaining_bits(r, bit_count),
        FieldType::GameplayTag => decode_gameplay_tag(r),
        FieldType::ByteArray { max_bytes } => decode_byte_array(r, max_bytes),
        FieldType::VectorFloat => decode_vector_float(r),
        FieldType::VectorDouble => decode_vector_double(r),
        FieldType::VectorNetQuantize { scale } => decode_vector_net_quantize(r, scale),
        FieldType::VectorNetQuantizeNormal => decode_vector_normal(r),
        FieldType::RotationShort => decode_rotation_short(r),
        FieldType::RotationByte => decode_rotation_byte(r),
        FieldType::Transform => decode_transform(r),
        FieldType::RepMovement { rotation } => decode_rep_movement(r, rotation),
        FieldType::Raw | FieldType::Skip => Err(DecodeError::RawOrSkip),
    }
}

// ── Scalar decoders ──────────────────────────────────────────────────────────

fn decode_bool(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
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
fn decode_byte(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let width = r.bits_remaining();
    if width == 0 || width > 8 {
        // Fall back to the nominal width so the error names a concrete read.
        return Ok(DecodedValue::I64(i64::from(r.read_u8()?)));
    }
    let raw = r.read_bits(width as u32)?;
    Ok(DecodedValue::I64(raw as i64))
}

fn decode_i32(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_i32()?)))
}

fn decode_u32(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_u32()?)))
}

fn decode_u64(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    // Store as i64 (reinterpret); values > i64::MAX are rare in practice.
    Ok(DecodedValue::I64(r.read_u64()? as i64))
}

fn decode_float(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::F64(f64::from(r.read_f32()?)))
}

fn decode_double(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::F64(r.read_f64()?))
}

fn decode_fstring(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
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
fn decode_fname(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    if r.read_bit()? {
        let index = r.read_int_packed()?;
        return Ok(DecodedValue::Str(index.to_string()));
    }
    let name = r.read_fstring(64 * 1024)?;
    let _suffix = r.read_i32()?;
    Ok(DecodedValue::Str(name))
}

fn decode_object_net_guid(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_int_packed()?)))
}

/// 128-bit GUID: 4 × u32 LE → formatted as standard hex GUID.
fn decode_guid(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let a = r.read_u32()?;
    let b = r.read_u32()?;
    let c = r.read_u32()?;
    let d = r.read_u32()?;
    let s = format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a,
        (b >> 16) & 0xFFFF,
        b & 0xFFFF,
        (c >> 16) & 0xFFFF,
        (u64::from(c & 0xFFFF) << 32) | u64::from(d)
    );
    Ok(DecodedValue::Str(s))
}

fn decode_serialized_int(r: &mut BitReader<'_>, max: u32) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_serialized_int(max)?)))
}

fn decode_enum_remaining_bits(
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

fn decode_gameplay_tag(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(DecodedValue::I64(i64::from(r.read_int_packed()?)))
}

fn decode_byte_array(r: &mut BitReader<'_>, max_bytes: u32) -> Result<DecodedValue, DecodeError> {
    let count = r.read_int_packed()?;
    if count > max_bytes {
        return Err(DecodeError::NotFullyConsumed {
            remaining: r.bits_remaining(),
        });
    }
    let mut buf = Vec::with_capacity(count as usize);
    for _ in 0..count {
        buf.push(r.read_u8()?);
    }
    // Hex-encode for value_str
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    Ok(DecodedValue::Str(hex))
}

// ── Vector decoders ──────────────────────────────────────────────────────────

fn decode_vector_float(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let v = read_float_vector(r)?;
    Ok(DecodedValue::Str(v.to_string()))
}

fn decode_vector_double(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let v = read_double_vector(r)?;
    Ok(DecodedValue::Str(v.to_string()))
}

fn decode_vector_net_quantize(
    r: &mut BitReader<'_>,
    scale: u32,
) -> Result<DecodedValue, DecodeError> {
    let v = read_quantized_vector(r, scale)?;
    Ok(DecodedValue::Str(v.to_string()))
}

fn decode_vector_normal(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let v = read_fixed_vector_normal(r)?;
    Ok(DecodedValue::Str(v.to_string()))
}

fn decode_rotation_short(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let rot = read_rotation_short(r)?;
    Ok(DecodedValue::Str(rot.to_string()))
}

fn decode_rotation_byte(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let rot = read_rotation_byte(r)?;
    Ok(DecodedValue::Str(rot.to_string()))
}

fn decode_transform(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    let t = read_transform(r)?;
    Ok(DecodedValue::Str(t.to_string()))
}

fn decode_rep_movement(
    r: &mut BitReader<'_>,
    rotation: RotatorQuantization,
) -> Result<DecodedValue, DecodeError> {
    let m = read_rep_movement(r, rotation)?;
    Ok(DecodedValue::Str(m.to_string()))
}

// ── Shared vector/rotation reading functions (public for tests) ──────────────

pub(crate) fn read_float_vector(r: &mut BitReader<'_>) -> Result<FVector, vrf_bitio::BitError> {
    Ok(FVector {
        x: f64::from(r.read_f32()?),
        y: f64::from(r.read_f32()?),
        z: f64::from(r.read_f32()?),
    })
}

pub(crate) fn read_double_vector(r: &mut BitReader<'_>) -> Result<FVector, vrf_bitio::BitError> {
    Ok(FVector {
        x: r.read_f64()?,
        y: r.read_f64()?,
        z: r.read_f64()?,
    })
}

/// Read a quantized vector: 7-bit header encodes component bit count + extra info.
///
/// ```text
/// header = SerializedInt(128)
///   bits [5:0] = componentBitCount
///   bit  [6]   = extraInfo (1 = scaled integer, 0 = float/double fallback)
///
/// if componentBitCount > 0:
///   3 × componentBitCount bits, sign-magnitude with sign at MSB
///   if extraInfo: divide by scaleFactor
/// else:
///   if extraInfo == 0: 3 × f32 (float vector)
///   else:              3 × f64 (double vector)
/// ```
pub(crate) fn read_quantized_vector(
    r: &mut BitReader<'_>,
    scale_factor: u32,
) -> Result<FVector, vrf_bitio::BitError> {
    let header = r.read_serialized_int(1 << 7)?;
    let component_bit_count = header & 63;
    let extra_info = header >> 6;

    if component_bit_count > 0 {
        read_packed_quantized_vector(r, component_bit_count, extra_info, scale_factor)
    } else if extra_info == 0 {
        read_float_vector(r)
    } else {
        read_double_vector(r)
    }
}

fn read_packed_quantized_vector(
    r: &mut BitReader<'_>,
    component_bit_count: u32,
    extra_info: u32,
    scale_factor: u32,
) -> Result<FVector, vrf_bitio::BitError> {
    let x_raw = r.read_bits(component_bit_count)?;
    let y_raw = r.read_bits(component_bit_count)?;
    let z_raw = r.read_bits(component_bit_count)?;
    let sign_bit = 1u64 << (component_bit_count - 1);

    let fx = (x_raw ^ sign_bit) as i64 - sign_bit as i64;
    let fy = (y_raw ^ sign_bit) as i64 - sign_bit as i64;
    let fz = (z_raw ^ sign_bit) as i64 - sign_bit as i64;

    let (x, y, z) = if extra_info > 0 {
        let sf = f64::from(scale_factor);
        (fx as f64 / sf, fy as f64 / sf, fz as f64 / sf)
    } else {
        (fx as f64, fy as f64, fz as f64)
    };

    Ok(FVector { x, y, z })
}

/// Fixed-point normal vector: 3 × SerializedInt(65536), bias = 32768, scale = 32767.
pub(crate) fn read_fixed_vector_normal(
    r: &mut BitReader<'_>,
) -> Result<FVector, vrf_bitio::BitError> {
    const BIAS: i32 = 1 << 15;
    const SCALE: f64 = (BIAS - 1) as f64;
    const MAX: u32 = 1 << 16;

    let dx = r.read_serialized_int(MAX)?;
    let dy = r.read_serialized_int(MAX)?;
    let dz = r.read_serialized_int(MAX)?;

    Ok(FVector {
        x: (dx as i32 - BIAS) as f64 / SCALE,
        y: (dy as i32 - BIAS) as f64 / SCALE,
        z: (dz as i32 - BIAS) as f64 / SCALE,
    })
}

pub(crate) fn read_rotation_short(r: &mut BitReader<'_>) -> Result<FRotator, vrf_bitio::BitError> {
    let pitch = read_compressed_short_component(r)?;
    let yaw = read_compressed_short_component(r)?;
    let roll = read_compressed_short_component(r)?;
    Ok(FRotator { pitch, yaw, roll })
}

pub(crate) fn read_rotation_byte(r: &mut BitReader<'_>) -> Result<FRotator, vrf_bitio::BitError> {
    let pitch = read_compressed_byte_component(r)?;
    let yaw = read_compressed_byte_component(r)?;
    let roll = read_compressed_byte_component(r)?;
    Ok(FRotator { pitch, yaw, roll })
}

fn read_compressed_short_component(r: &mut BitReader<'_>) -> Result<f32, vrf_bitio::BitError> {
    if r.read_bit()? {
        let v = r.read_u16()?;
        Ok(f32::from(v) * (360.0 / 65536.0))
    } else {
        Ok(0.0)
    }
}

fn read_compressed_byte_component(r: &mut BitReader<'_>) -> Result<f32, vrf_bitio::BitError> {
    if r.read_bit()? {
        let v = r.read_u8()?;
        Ok(f32::from(v) * (360.0 / 256.0))
    } else {
        Ok(0.0)
    }
}

fn read_quaternion(r: &mut BitReader<'_>) -> Result<FQuat, vrf_bitio::BitError> {
    Ok(FQuat {
        x: r.read_f32()?,
        y: r.read_f32()?,
        z: r.read_f32()?,
        w: r.read_f32()?,
    })
}

pub(crate) fn read_transform(r: &mut BitReader<'_>) -> Result<FTransform, vrf_bitio::BitError> {
    let rotation = read_quaternion(r)?;
    let translation = read_float_vector(r)?;
    let scale = read_float_vector(r)?;
    Ok(FTransform {
        rotation,
        translation,
        scale,
    })
}

pub(crate) fn read_rep_movement(
    r: &mut BitReader<'_>,
    rotation_quant: RotatorQuantization,
) -> Result<FRepMovement, vrf_bitio::BitError> {
    let simulated_physics_sleep = r.read_bit()?;
    let rep_physics = r.read_bit()?;
    let rep_server_frame = r.read_bit()?;
    let rep_server_handle = r.read_bit()?;

    let location = read_quantized_vector(r, 100)?;
    let rotation = match rotation_quant {
        RotatorQuantization::ByteComponents => read_rotation_byte(r)?,
        RotatorQuantization::ShortComponents => read_rotation_short(r)?,
    };
    let linear_velocity = read_quantized_vector(r, 1)?;

    let angular_velocity = if rep_physics {
        Some(read_quantized_vector(r, 1)?)
    } else {
        None
    };

    let server_frame = if rep_server_frame {
        r.read_int_packed()?
    } else {
        0
    };

    let server_physics_handle = if rep_server_handle {
        r.read_int_packed()?
    } else {
        0
    };

    Ok(FRepMovement {
        location,
        rotation,
        linear_velocity,
        angular_velocity,
        simulated_physics_sleep,
        rep_physics,
        server_frame,
        server_physics_handle,
    })
}
