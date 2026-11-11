//! Type-aware field decoders for Unreal Engine replay primitives: every field
//! update arrives as raw bits, and this crate turns them into typed values.
//!
//! # Design: additive overlay
//!
//! Raw bits are always preserved. A successful decode fills one `value_*`
//! slot; a failure leaves them all `None` and increments an error counter, so
//! an unknown field or a layout change loses nothing, it only lacks a typed
//! value. Nothing here ever substitutes a plausible value for one it could not
//! read.
//!
//! [`decode_field`] reads one declared [`FieldType`] (the variants and their
//! readers say what each consumes). Three decoders key off the field's name or
//! array shape instead: [`decode_struct_array`] for RepLayout arrays,
//! [`structs`] for named struct blobs and [`effect`] for the `EffectContainer`
//! arrays (a JSON array in `value_str`). They are additive the same way.
//!
//! # Module map
//!
//! | Module | What it owns |
//! |--------|--------------|
//! | `types` | The model structs and their `Display` forms |
//! | `decode` | `FieldType`, `DecodedValue` and the dispatch; `scalar` and `geometry` hold the readers |
//! | `ftext` | The strict `FText` history-tree reader |
//! | `overlay` | The `(group_path, field_name)` table, its hash index and the resolution order |
//! | `table` | The overlay table, maintained here; `tools/apply_type_corrections.py` rewrites its counts |
//! | `checksum_table`, `scoped_types` | GENERATED -- never hand-edited |
//! | `array` | The RepLayout dynamic-array walker and the nesting schema |
//! | `structs` | The three named struct blobs, one module each |
//! | `effect` | The `EffectContainer` arrays: framing, elements, JSON |
//! | `cnc` | ClassNetCache RPC framing for groups the replay never declared |
//! | `fastarray` | FastArray custom-delta framing, with no in-tree caller |
//!
//! # Feature flags
//!
//! [`decode_field`] and the model types are the always-compiled core. The four
//! decoders on top are independent of each other and all on by default:
//!
//! | Feature | Brings in |
//! |---------|-----------|
//! | `overlay` | [`OverlayTable`], [`apply_overlay_with_handle`], `OVERLAY_TABLE` and the checksum and scoped tables |
//! | `array` | [`decode_struct_array`] and [`COMBAT_ROUNDS_SCHEMA`] |
//! | `structs` | [`structs`] |
//! | `effect` | [`effect`] |
//!
//! `overlay` is the one worth dropping if the primitive decoders are all you
//! want: it is what pulls in the overlay tables.

#![forbid(unsafe_code)]

mod decode;
mod ftext;
mod types;

pub mod cnc;

/// Numeric FastArray headers, item IDs and raw property windows. No in-tree
/// caller: `tools/extract_fastarray_observations.py` reads the same grammar in
/// Python, kept in step (docs/GAS_AND_PATCHVOLUME_INVESTIGATION.md).
pub mod fastarray;

#[cfg(feature = "array")]
mod array;
#[cfg(feature = "overlay")]
mod checksum_table;
/// Decoder for the `EffectContainer` arrays RPCs carry as `FloatValues` /
/// `ObjectValues` / `VectorValues`, wired into the export through
/// [`effect::decode_effect_blob_json`]. See the module docs.
#[cfg(feature = "effect")]
pub mod effect;
#[cfg(any(feature = "effect", feature = "structs"))]
mod framing;
#[cfg(feature = "overlay")]
mod overlay;
#[cfg(feature = "overlay")]
mod scoped_types;
#[cfg(feature = "structs")]
pub mod structs;
#[cfg(feature = "overlay")]
mod table;
#[cfg(test)]
mod tests;

pub use decode::{DecodeError, DecodedValue, FieldType, decode_field};
pub use ftext::{
    FTextArgument, FTextArgumentValue, FTextName, FTextNumberFormat, FTextTree, FTextTreeError,
    decode_ftext_tree,
};
pub use types::{
    FQuat, FRepMovement, FRotator, FTransform, FVector, RotatorQuantization, VectorQuantization,
};

#[cfg(feature = "array")]
pub use array::{
    ABILITY_CASTS_SCHEMA, ABILITY_EFFECTS_SCHEMA, ArrayDecodeStats, ArrayFieldSchema,
    COMBAT_ROUNDS_SCHEMA, COMBAT_ROUNDS_SCHEMA_1310, FlattenedField, LIFE_CHANGE_BY_SECTION_SCHEMA,
    LIFE_CHANGE_DAMAGE_SCHEMA, LIFE_CHANGE_DAMAGE_SCHEMA_1310, LIFE_CHANGE_SECTION_SCHEMA,
    MAX_ELEMENTS, MAX_FIELDS_PER_ELEMENT, MAX_RECURSION_DEPTH, decode_object_ref_array,
    decode_object_ref_array_with_stats, decode_struct_array, decode_struct_array_exact,
};
#[cfg(feature = "overlay")]
pub use checksum_table::CHECKSUM_TYPES;
#[cfg(feature = "effect")]
pub use effect::{EffectArrayKind, EffectBlobError, decode_effect_blob_json};
#[cfg(feature = "overlay")]
pub use overlay::{
    DecodeErrorKind, GroupHashState, OverlayEntry, OverlayErrorReport, OverlayErrorRow,
    OverlayHandleEntry, OverlayStats, OverlayTable, apply_overlay, apply_overlay_with_checksum,
    apply_overlay_with_handle, canonical_group, group_hash_state, lookup_checksum,
    resolve_field_type, resolve_field_type_with_checksum,
};
#[cfg(feature = "overlay")]
pub use table::{OVERLAY_HANDLE_TABLE, OVERLAY_TABLE};

/// Hex fixtures for the effect and struct-blob tests; the bit encoders are
/// `vrf_testkit`'s.
#[cfg(all(test, any(feature = "effect", feature = "structs")))]
pub(crate) mod test_bits {
    /// Bytes from hex digits; whitespace between them is ignored.
    pub(crate) fn hex(digits: &str) -> Vec<u8> {
        let clean: Vec<u8> = digits
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        assert!(
            clean.len() % 2 == 0,
            "odd number of hex digits in {digits:?}"
        );
        clean
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
}
