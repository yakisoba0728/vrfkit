//! Field type enumeration, decoded value representation, and the dispatch decoder.
//!
//! This module is the core of the type overlay: given a [`FieldType`] and raw
//! bits, it produces a [`DecodedValue`] or a [`DecodeError`]. The per-type
//! readers live next door -- [`scalar`] for the primitives Unreal writes
//! directly, [`geometry`] for the vector, rotator and transform shapes that
//! render as a compact string.
//!
//! `table.rs` is generated against `crate::decode::FieldType`, so this module
//! keeps its path even though its body is split.

mod geometry;
mod scalar;

use vrf_bitio::BitReader;

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
        rotation: crate::types::RotatorQuantization,
    },
    /// Dynamic arrays and custom decoders -- not decoded, raw_bits suffices.
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
    /// A `UInt64` value exceeded `i64::MAX`. The overlay stores integers as
    /// `i64`, so a value with its high bit set cannot be represented without a
    /// silent sign flip; error loudly rather than emit a plausible wrong
    /// (negative) number.
    #[error("unsigned value {value} (0x{value:016X}) exceeds i64::MAX")]
    UnsignedOverflow { value: u64 },
}

/// Decode raw bits according to the given [`FieldType`].
///
/// Returns `Err(DecodeError::RawOrSkip)` for `Raw` and `Skip` types -- these
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
        FieldType::Bool => scalar::decode_bool(r),
        FieldType::Byte | FieldType::EnumByte => scalar::decode_byte(r),
        FieldType::Int32 => scalar::decode_i32(r),
        FieldType::UInt32 => scalar::decode_u32(r),
        FieldType::UInt64 => scalar::decode_u64(r),
        FieldType::Float => scalar::decode_float(r),
        FieldType::Double => scalar::decode_double(r),
        FieldType::FString => scalar::decode_fstring(r),
        FieldType::FName => scalar::decode_fname(r),
        FieldType::ObjectNetGuid => scalar::decode_object_net_guid(r),
        FieldType::Guid => scalar::decode_guid(r),
        FieldType::SerializedInt { max } => scalar::decode_serialized_int(r, max),
        FieldType::EnumRemainingBits => scalar::decode_enum_remaining_bits(r, bit_count),
        FieldType::GameplayTag => scalar::decode_gameplay_tag(r),
        FieldType::ByteArray { max_bytes } => scalar::decode_byte_array(r, max_bytes),
        FieldType::VectorFloat => geometry::decode_vector_float(r),
        FieldType::VectorDouble => geometry::decode_vector_double(r),
        FieldType::VectorNetQuantize { scale } => geometry::decode_vector_net_quantize(r, scale),
        FieldType::VectorNetQuantizeNormal => geometry::decode_vector_normal(r),
        FieldType::RotationShort => geometry::decode_rotation_short(r),
        FieldType::RotationByte => geometry::decode_rotation_byte(r),
        FieldType::Transform => geometry::decode_transform(r),
        FieldType::RepMovement { rotation } => geometry::decode_rep_movement(r, rotation),
        FieldType::Raw | FieldType::Skip => Err(DecodeError::RawOrSkip),
    }
}

/// Render a `Display` value into [`DecodedValue::Str`].
///
/// Deliberately plain `to_string()`. Pre-reserving a typical rendered length
/// here was tried -- 32 bytes for a vector, 256 for a `ReplicatedMovement` --
/// on the reasoning that the string otherwise doubles its way up from zero.
/// It measured no faster end to end AND raised peak RSS by ~3 MB, because
/// these strings live in the export's row buffer until the Parquet write and
/// the reservation is dead space for every value that renders short (`(0,0,0)`
/// is seven characters). Growing on demand is both the simpler code and the
/// smaller footprint.
fn render(value: impl core::fmt::Display) -> DecodedValue {
    DecodedValue::Str(value.to_string())
}
