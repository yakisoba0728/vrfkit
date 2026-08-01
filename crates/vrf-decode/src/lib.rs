//! Type-aware field decoders for Unreal Engine replay primitives.
//!
//! # Purpose
//!
//! The replay pipeline records every field update as raw bits. This crate
//! converts those bits into typed values by applying the same decoding logic
//! that the game client uses (ported from the C# reference parser).
//!
//! # Design: additive overlay
//!
//! Decoding is *additive*: raw bits are always preserved. A successful decode
//! populates one of the `value_*` slots; a failure leaves them all `None` and
//! increments an error counter. This guarantees forward compatibility: unknown
//! fields or layout changes never lose data, they just lack typed overlays
//! until the decoder is updated.
//!
//! # Supported types
//!
//! | Type | Bits consumed | Value slot |
//! |------|---------------|------------|
//! | Bool | 1 | `value_bool` |
//! | Byte | 8 | `value_i64` |
//! | EnumByte | 8 | `value_i64` |
//! | Int32 | 32 | `value_i64` |
//! | UInt32 | 32 | `value_i64` |
//! | UInt64 | 64 | `value_i64` |
//! | Float | 32 | `value_f64` |
//! | Double | 64 | `value_f64` |
//! | FString | variable | `value_str` |
//! | FName | variable | `value_str` |
//! | ObjectNetGuid | variable (IntPacked) | `value_i64` |
//! | Guid | 128 | `value_str` (hex) |
//! | SerializedInt(max) | variable | `value_i64` |
//! | EnumRemainingBits | all remaining | `value_i64` |
//! | GameplayTag | variable (IntPacked) | `value_i64` (tag index) |
//! | ByteArray(max) | variable | `value_str` (hex) |
//! | FVector(double) | 192 | `value_str` (compact) |
//! | FVector(float) | 96 | `value_str` (compact) |
//! | VectorNetQuantize(1/10/100) | variable | `value_str` (compact) |
//! | VectorNetQuantizeNormal | ~48 (3×16 serialized) | `value_str` (compact) |
//! | RotationShort | 3×(1+16) max | `value_str` (compact) |
//! | RotationByte | 3×(1+8) max | `value_str` (compact) |
//! | Transform | 4×32 + 3×32 + 3×32 = 320 | `value_str` (compact) |
//! | RepMovement | variable | `value_str` (compact) |
//! | RepLayoutDynamicArray | variable | *Raw* (not decoded) |

#![forbid(unsafe_code)]

mod array;
mod decode;
/// Shot-effect blob decoder. **Not wired into the pipeline** -- the live
/// decoder is a Python port in `tools/to_valplay_bundle.py` with a different
/// failure contract. See the module docs before calling any of it.
pub mod effect;
mod overlay;
pub mod structs;
mod table;
#[cfg(test)]
mod tests;
mod types;

pub use array::{
    ArrayDecodeStats, ArrayFieldSchema, COMBAT_ROUNDS_SCHEMA, FlattenedField, MAX_ELEMENTS,
    MAX_FIELDS_PER_ELEMENT, MAX_RECURSION_DEPTH, decode_struct_array,
};
pub use decode::{DecodeError, DecodedValue, FieldType, decode_field};
pub use overlay::{
    DecodeErrorKind, OverlayEntry, OverlayErrorReport, OverlayErrorRow, OverlayStats, OverlayTable,
    apply_overlay,
};
pub use table::OVERLAY_TABLE;
pub use types::{FQuat, FRepMovement, FRotator, FTransform, FVector, RotatorQuantization};
