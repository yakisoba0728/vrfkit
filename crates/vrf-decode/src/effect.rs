//! Decoder for the Valorant shot-effect RepLayoutDynamicArray blobs.
//!
//! # Where this runs
//!
//! [`decode_effect_blob_json`] is wired into the export path: `sink.rs`'s RPC
//! parameter loop calls it for every RPC parameter named `FloatValues`,
//! `ObjectValues` or `VectorValues`, and puts the JSON it returns into
//! `value_str` **in addition to** `raw_bits`, never instead of it. The decode
//! is additive, exactly like the type overlay: a failure leaves `value_str`
//! null, keeps the bits, and increments a counter.
//!
//! ## One RPC is deliberately excluded, and it is not excluded here
//!
//! `ReplayPlayContinuousEffectAtLocation` -- the shot RPC -- is skipped by the
//! caller. The reason is a property of the downstream Python consumer, not of
//! this wire format, so the exclusion lives at the call site in `sink.rs`
//! rather than in this module. See the comment on
//! `effect_array_kind_for_param` there.
//!
//! That RPC's blobs are still decoded, by the Python port in
//! `tools/to_valplay_bundle.py` (`_decode_effect_blob` and friends), which
//! reads the raw bits back out of `fields.parquet` after export.
//!
//! ## How this module's contract differs from the Python port
//!
//! The two agree on every well-formed blob but their failure contracts differ:
//! on a malformed blob this module returns `Err` and discards the whole array,
//! while the Python port breaks out of its loop and returns the elements it had
//! already decoded. A direct differential over 100,997 real blobs from 11
//! replays, comparing floats as IEEE-754 bit patterns, found 0 disagreements,
//! and a corpus census over 2,045,428 blobs found no input that reaches a
//! branch where they could differ. The two therefore coexist without a
//! reconciliation: they are never asked about the same rows.
//!
//! **That measurement predates four added rejections and has not been re-run.**
//! This module now also refuses an `IntPacked` member that underfills its
//! declared window, an array or element that ends without its terminator, a
//! non-zero trailing terminator byte, and a residual of 1-7 bits (previously
//! all four were accepted). Each is a branch where this module fails and the
//! Python port still returns elements, so the "no input reaches a branch where
//! they could differ" half of the claim is exactly what the changes put back in
//! question. The reasoning says these fire on nothing well-formed -- an
//! `IntPacked` is self-delimiting, and every pinned wire vector in
//! [`tests`] still passes -- but that is an argument and a fixture set, not the
//! census. Re-run `tools/check_effect_decoder.py` over the corpus to restore
//! the claim to a measurement.
//!
//! This module's own tests are the repo's only executable specification of this
//! wire format: eight pinned hex vectors lifted from real packets, with values
//! checked against the C# reference. `tools/` contains no test files, so
//! deleting this would leave the format specified by prose alone.
//!
//! # Purpose
//!
//! Four metric sections in the valplay analytics pipeline (`weapons`,
//! `shot_rays`, `spray_control`, `posture`) depend on the
//! `valorant_shot_received` event. In the replay, each shot arrives as a
//! `ReplayPlayContinuousEffectAtLocation` RPC whose payload contains three
//! RepLayoutDynamicArray blobs:
//!
//! | Field | Element type | Contains |
//! |-------|-------------|----------|
//! | `FloatValues` | `EffectDataFloat` | ammo, projectile count, random seed, tracer option, burst, yaw switch |
//! | `ObjectValues` | `EffectDataObject` | firing player state, firing state, equippable |
//! | `VectorValues` | `EffectDataVector` | attack direction vectors (1 per projectile) |
//!
//! Each blob uses the standard UE RepLayout dynamic-array wire format.
//!
//! # Wire layout (established via C# reference + corpus validation)
//!
//! All three use identical framing:
//!
//! ```text
//! [IntPacked: element_count]
//! repeat {
//!     [IntPacked: encoded_index]   // 0 = terminator, else index = encoded - 1
//!     // per element: repeat field handles until 0 terminator
//!     repeat {
//!         [IntPacked: encoded_handle]  // 0 = end, else handle = encoded - 1
//!         [IntPacked: payload_bits]    // field payload bit length
//!         [bits: payload]              // field-specific decode
//!     }
//! }
//! ```
//!
//! ## EffectDataFloat (handles from C# `EffectDataFloat.cs`)
//! - handle 7: tag name (IntPacked gameplay tag index)
//! - handle 8: float value (32-bit IEEE-754)
//!
//! ## EffectDataObject (handles from C# `EffectDataObject.cs`)
//! - handle 15: tag name (IntPacked gameplay tag index)
//! - handle 16: object value (IntPacked net GUID)
//!
//! ## EffectDataVector (handles from C# `EffectDataVector.cs`)
//! - handle 11: tag name (IntPacked gameplay tag index)
//! - handle 12: vector value (3 x f64 = 192 bits)
//!
//! # Validation
//!
//! Decoded output was verified against the C# parser's `events.ndjson` for
//! replay `02d4d478` (2,647 shot invocations). Float values match at f32
//! precision, object net GUIDs match exactly, and vector components match at
//! f64 bit-exact precision. Tag indices are replay-specific (resolved from
//! the `NetworkGameplayTagNodeIndex` table); the decoder outputs raw indices
//! which the wiring layer resolves to field names like
//! `FiringState.AmmoRemaining`.
//!
//! # Derivation sources
//!
//! - `src/Replay.Valorant/Descriptors/Effects/Replay/EffectDataFloat.cs`
//! - `src/Replay.Valorant/Descriptors/Effects/Replay/EffectDataObject.cs`
//! - `src/Replay.Valorant/Descriptors/Effects/Replay/EffectDataVector.cs`
//! - `src/Replay.Valorant/Descriptors/Effects/Replay/ReplayPlayContinuousEffectAtLocationParameters.cs`
//! - `src/Replay.Unreal/Parsing/RepLayoutArrayDecoders.cs`
//!
//! # Module layout
//!
//! - [`framing`]: the shared array framing and [`scan_element_handles`], which
//!   is what makes the element handle pair a measurement rather than a guess.
//! - [`elements`]: the three element types and their decoders.
//! - [`json`]: the export path's blob-to-JSON rendering.

mod elements;
mod framing;
mod json;
#[cfg(test)]
mod tests;

pub use elements::{
    EffectDataFloat, EffectDataObject, EffectDataVector, decode_effect_floats,
    decode_effect_floats_at, decode_effect_objects, decode_effect_objects_at,
    decode_effect_vectors, decode_effect_vectors_at,
};
pub use framing::scan_element_handles;
pub use json::decode_effect_blob_json;

/// Errors that can occur while decoding an effect array blob.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EffectBlobError {
    /// The underlying bit reader hit EOF or produced a malformed primitive.
    #[error("bit read: {0}")]
    BitIo(#[from] vrf_bitio::BitError),

    /// The declared array element count exceeds a sane maximum.
    #[error("array count {count} exceeds maximum {max}")]
    ArrayCountTooLarge { count: u32, max: u32 },

    /// An element index is out of bounds relative to the declared count.
    #[error("element index {index} >= declared count {count}")]
    IndexOutOfBounds { index: u32, count: u32 },

    /// A field payload declared more bits than remain in the stream.
    #[error("field payload {bits} bits exceeds remaining {remaining}")]
    PayloadTooLarge { bits: u32, remaining: u64 },

    /// Too many fields in a single element (guard against infinite loops).
    #[error("too many fields in element ({context})")]
    TooManyFields { context: &'static str },

    /// The declared bit length is longer than the buffer that carries it.
    #[error("declared {bits} bits but buffer holds {available}")]
    BitLengthExceedsBuffer { bits: u32, available: u64 },

    /// The array terminated with more than byte padding left over.
    ///
    /// A blob in this format consumes its window exactly: all 61,617 blobs on
    /// `02d4d478` finish with 0 bits remaining. A payload that is *not* this
    /// format usually still parses into something plausible and then leaves a
    /// large tail, so this is the guard that separates "decoded" from "decoded
    /// into a plausible-looking structure". Sub-byte padding is tolerated
    /// because it cannot carry an element.
    #[error("{remaining} bits left after terminator")]
    ResidualBits { remaining: u64 },

    /// A float element decoded to NaN or an infinity.
    ///
    /// JSON has no literal for either, and this repo has been burned three
    /// times by values that looked right, so the blob is rejected rather than
    /// coerced to `null` or `0`. Zero occurrences across the 24,000 float
    /// blobs on `02d4d478`; the guard exists so that the first one is loud.
    #[error("element {index} is a non-finite float")]
    NonFiniteFloat { index: usize },

    /// A value field declared a width its type cannot occupy.
    ///
    /// The decoders do not sub-window per field: they read the type, then skip
    /// whatever the declared width had left over. A field declaring *fewer*
    /// bits than its type needs would therefore read past its own payload and
    /// into the next field. Checking the width first turns that silent
    /// corruption into a rejected blob.
    #[error("{context}: expected a {expected}-bit payload, found {found}")]
    UnexpectedPayloadWidth {
        context: &'static str,
        expected: u32,
        found: u32,
    },

    /// An element carried a number of fields other than two.
    ///
    /// Every one of the 128,000 elements on `02d4d478` carries exactly two: a
    /// gameplay tag and a value. Two is what makes the handle pair derivable,
    /// so anything else is rejected rather than guessed at.
    #[error("element has {found} field(s), expected 2")]
    ElementFieldCount { found: u32 },

    /// An element's two field handles were not adjacent.
    #[error("field handles {first} and {second} are not adjacent")]
    NonAdjacentHandles { first: u32, second: u32 },

    /// Two elements of one array disagreed about the handle base.
    #[error("handle base {found} contradicts {expected} seen earlier")]
    InconsistentHandleBase { expected: u32, found: u32 },

    /// A field's type consumed more bits than the field declared.
    #[error("field declared {declared} bits but its type read {consumed}")]
    PayloadOverread { declared: u32, consumed: u64 },

    /// A field's type consumed FEWER bits than the field declared.
    ///
    /// The mirror of [`Self::PayloadOverread`], and it used to be silent: the
    /// leftover was skipped so the next field still started in the right place,
    /// and nothing recorded that part of a field had gone uninterpreted. The
    /// fixed-width members already refused this shape via
    /// [`Self::UnexpectedPayloadWidth`]; the `IntPacked` ones (the gameplay tag
    /// and the object GUID) did not, which made the accounting depend on which
    /// member happened to be reading.
    ///
    /// An `IntPacked` is self-delimiting -- it spends `ceil(bits/7)` whole
    /// bytes and a writer-measured `payload_bits` matches it exactly -- so a
    /// short read means the window was not what this decoder thinks it was.
    #[error("field declared {declared} bits but its type read only {consumed}")]
    PayloadUnderread { declared: u32, consumed: u64 },

    /// The array ran out of bits without reading its terminator.
    ///
    /// An element index of `0` ends the array and a handle of `0` ends an
    /// element. Reaching EOF instead was accepted, which made a payload cut
    /// short indistinguishable from a complete one: the array is sparse by
    /// design, so the elements that never arrived simply render as absent.
    #[error("{context} ended without its terminator")]
    MissingTerminator { context: &'static str },

    /// The trailing byte after the array terminator was not zero.
    ///
    /// The C# parser reads that byte and discards both its value and any error.
    /// Copying that made any appended byte a valid "terminator", so a payload
    /// with one spare byte of anything passed as well-formed. This crate
    /// already declines to mirror a reference that is silently permissive; see
    /// the note on `decode_field` in `decode.rs`.
    #[error("trailing terminator byte is {value}, expected 0")]
    NonZeroTerminator { value: u32 },
}

/// Convenience alias.
pub type Result<T> = core::result::Result<T, EffectBlobError>;

/// The pair of RepLayout field handles an `FEffectData*` element uses: one for
/// the gameplay tag, one for the value.
///
/// # Why this is not a constant
///
/// It reads like a property of the struct -- `FEffectDataFloat` has two members
/// and they should have two fixed handles -- but it is a property of the
/// *containing function*. Unreal numbers a dynamic array's element handles from
/// the array's own handle in the parent layout, and each RPC declares these
/// arrays at a different position, so the same struct arrives under a different
/// pair in every function. Measured on `02d4d478` -- ten functions, and this
/// table is a measurement, not an enumeration: nothing stops another build
/// declaring an eleventh at a base none of these use, which is the reason the
/// pair is derived rather than tabulated.
///
/// | Function | FloatValues | ObjectValues | VectorValues |
/// |----------|-------------|--------------|--------------|
/// | `ReplayPlayContinuousEffectAtLocation` | 7/8 | 15/16 | 11/12 |
/// | `ClientPlayOneShotEffectAtLocation` | 3/4 | 11/12 | -- |
/// | `MulticastPlayContinuousEffect` | 3/4 | 11/12 | 7/8 |
/// | `MulticastPlayContinuousEffectFromClient` | 4/5 | 12/13 | -- |
/// | `MulticastPlayOneShotEffect` | 3/4 | 11/12 | -- |
/// | `MulticastPlayOneShotEffectFromClient` | 4/5 | 12/13 | -- |
/// | `MulticastUpdateContinuousEffect` | 6/7 | -- | -- |
/// | `ReplayPlayOneShotEffectAtLocation` | 3/4 | 11/12 | 7/8 |
/// | `ReplayRecordOneShotEffect` | -- | 11/12 | -- |
/// | `ReplayRecordContinuousEffect` | 3/4 | 11/12 | -- |
///
/// Treating the first row's numbers as universal is not a harmless
/// approximation. It makes every other function's elements decode to
/// all-`None` -- and worse, `MulticastUpdateContinuousEffect`'s float value
/// sits at handle 7, the slot the first row calls the tag, so 298 elements
/// decoded a 32-bit float payload as a tag index and produced confident,
/// wrong numbers. [`scan_element_handles`] derives the pair from the blob
/// instead of assuming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectHandles {
    /// Handle of the `FGameplayTag` member. Always the lower of the two.
    pub tag: u32,
    /// Handle of the value member. Always `tag + 1`.
    pub value: u32,
}

impl EffectHandles {
    /// The pair whose tag member is at `base`.
    ///
    /// Saturates rather than overflowing. `scan_element_handles` rejects a
    /// non-adjacent pair before it gets here so `u32::MAX` should be
    /// unreachable, but the export path is not a place to leave a debug-build
    /// panic on unreachable-in-principle input.
    #[must_use]
    pub const fn from_base(base: u32) -> Self {
        Self {
            tag: base,
            value: base.saturating_add(1),
        }
    }
}

/// `FEffectDataFloat` handles in `ReplayPlayContinuousEffectAtLocation`.
///
/// These are the numbers the C# reference descriptors state and the numbers
/// this module's pinned vectors were captured under. They are the default only
/// because that is the function those vectors come from.
const FLOAT_HANDLES: EffectHandles = EffectHandles::from_base(7);

/// `FEffectDataObject` handles in `ReplayPlayContinuousEffectAtLocation`.
const OBJECT_HANDLES: EffectHandles = EffectHandles::from_base(15);

/// `FEffectDataVector` handles in `ReplayPlayContinuousEffectAtLocation`.
const VECTOR_HANDLES: EffectHandles = EffectHandles::from_base(11);

/// Which `FEffectData*` element type a blob carries.
///
/// The three arrays share one framing and differ only in their field handles
/// and value decode, so the export path selects between them by the RPC
/// parameter's declared name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectArrayKind {
    /// `TArray<FEffectDataFloat>`, declared as `FloatValues`.
    Float,
    /// `TArray<FEffectDataObject>`, declared as `ObjectValues`.
    Object,
    /// `TArray<FEffectDataVector>`, declared as `VectorValues`.
    Vector,
}

impl EffectArrayKind {
    /// Map an RPC parameter's declared name to its element type.
    ///
    /// Name-driven rather than handle-driven: the handle is the parameter's
    /// index within its own function, so it differs between the eleven
    /// functions that carry these arrays, while the name is stable across all
    /// of them. Returns `None` for every other parameter name.
    #[must_use]
    pub fn from_param_name(name: &str) -> Option<Self> {
        match name {
            "FloatValues" => Some(Self::Float),
            "ObjectValues" => Some(Self::Object),
            "VectorValues" => Some(Self::Vector),
            _ => None,
        }
    }
}
