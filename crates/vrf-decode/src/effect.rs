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

use crate::types::FVector;
use vrf_bitio::BitReader;

// ---- Error type ----

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
}

/// Convenience alias.
pub type Result<T> = core::result::Result<T, EffectBlobError>;

// ---- Constants ----

/// Maximum array element count. In practice, FloatValues has at most ~6
/// elements, ObjectValues ~4, and VectorValues up to ~15 (shotgun pellets).
/// 256 provides generous headroom without risking runaway allocation.
const MAX_ARRAY_COUNT: u32 = 256;

/// Maximum fields per element. EffectData* structs have 2 handles each
/// (tag + value), so 8 is very generous.
const MAX_FIELDS_PER_ELEMENT: u32 = 8;

/// Maximum bits in a single field payload. Prevents runaway on corrupt data.
const MAX_FIELD_PAYLOAD_BITS: u32 = 64 * 1024;

// ---- Element field handles ----

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

// ---- Output types ----

/// A single decoded `FEffectDataFloat` element: a gameplay-tag index plus a
/// float value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectDataFloat {
    /// Gameplay tag index (resolved to a name like `FiringState.AmmoRemaining`
    /// via the replay's tag table). `None` if the tag field was absent.
    pub tag_index: Option<u32>,
    /// The float value. `None` if the value field was absent.
    pub value: Option<f32>,
}

/// A single decoded `FEffectDataObject` element: a gameplay-tag index plus a
/// net GUID (object reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectDataObject {
    /// Gameplay tag index. `None` if the tag field was absent.
    pub tag_index: Option<u32>,
    /// Object net GUID. `None` if the value field was absent.
    pub value: Option<u32>,
}

/// A single decoded `FEffectDataVector` element: a gameplay-tag index plus a
/// 3D vector (f64 components, matching the wire format).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectDataVector {
    /// Gameplay tag index. `None` if the tag field was absent.
    pub tag_index: Option<u32>,
    /// The vector value. `None` if the value field was absent.
    pub value: Option<FVector>,
}

// ---- Internal helpers ----

/// Read the declared element count from the stream.
fn read_array_count(reader: &mut BitReader<'_>) -> Result<u32> {
    let count = reader.read_int_packed()?;
    if count > MAX_ARRAY_COUNT {
        return Err(EffectBlobError::ArrayCountTooLarge {
            count,
            max: MAX_ARRAY_COUNT,
        });
    }
    Ok(count)
}

/// Read the next element index. Returns `None` if the terminator (0) is read.
fn read_element_index(reader: &mut BitReader<'_>, declared_count: u32) -> Result<Option<u32>> {
    let encoded = reader.read_int_packed()?;
    if encoded == 0 {
        return Ok(None);
    }
    let index = encoded - 1;
    if index >= declared_count {
        return Err(EffectBlobError::IndexOutOfBounds {
            index,
            count: declared_count,
        });
    }
    Ok(Some(index))
}

/// Reject a value field whose declared width its type cannot occupy.
fn expect_width(context: &'static str, expected: u32, found: u32) -> Result<()> {
    if found == expected {
        Ok(())
    } else {
        Err(EffectBlobError::UnexpectedPayloadWidth {
            context,
            expected,
            found,
        })
    }
}

/// Advance the reader to the end of a field whose header declared
/// `payload_bits`, having already read `reader.position() - start_pos` of them.
///
/// Reading *past* the declared width is the interesting case: it means the
/// field's type is wider than the field, so the decode has already consumed
/// part of the next field and every value after it is suspect.
fn settle_field(reader: &mut BitReader<'_>, start_pos: u64, payload_bits: u32) -> Result<()> {
    let consumed = reader.position() - start_pos;
    let declared = u64::from(payload_bits);
    if consumed > declared {
        return Err(EffectBlobError::PayloadOverread {
            declared: payload_bits,
            consumed,
        });
    }
    reader.skip_bits(declared - consumed)?;
    Ok(())
}

/// Read the next field handle + payload bit count. Returns `None` on terminator.
fn read_field_header(reader: &mut BitReader<'_>) -> Result<Option<(u32, u32)>> {
    let encoded_handle = reader.read_int_packed()?;
    if encoded_handle == 0 {
        return Ok(None);
    }
    let handle = encoded_handle - 1;
    let payload_bits = reader.read_int_packed()?;
    if payload_bits > MAX_FIELD_PAYLOAD_BITS {
        return Err(EffectBlobError::PayloadTooLarge {
            bits: payload_bits,
            remaining: reader.bits_remaining(),
        });
    }
    if u64::from(payload_bits) > reader.bits_remaining() {
        return Err(EffectBlobError::PayloadTooLarge {
            bits: payload_bits,
            remaining: reader.bits_remaining(),
        });
    }
    Ok(Some((handle, payload_bits)))
}

// ---- Public decoders ----

/// Decode a `TArray<FEffectDataFloat>` RepLayout dynamic array.
///
/// # Arguments
/// - `reader`: bit reader positioned at the start of the FloatValues blob.
///
/// # Returns
/// A vector of decoded float data elements. Elements that were not populated
/// in the stream (sparse array) are filled with `tag_index: None, value: None`.
///
/// # Wire layout
/// ```text
/// [IntPacked: count]
/// elements (sparse, terminated by index=0):
///   [IntPacked: index+1]
///   fields (terminated by handle=0):
///     handle 7: [IntPacked: bits] [IntPacked: tag_index]
///     handle 8: [IntPacked: bits] [f32: value]
/// ```
pub fn decode_effect_floats(reader: &mut BitReader<'_>) -> Result<Vec<EffectDataFloat>> {
    decode_effect_floats_at(reader, FLOAT_HANDLES)
}

/// [`decode_effect_floats`] with the element's handle pair supplied.
///
/// See [`EffectHandles`] for why the pair is not a constant.
pub fn decode_effect_floats_at(
    reader: &mut BitReader<'_>,
    handles: EffectHandles,
) -> Result<Vec<EffectDataFloat>> {
    let count = read_array_count(reader)?;
    let mut elements = vec![
        EffectDataFloat {
            tag_index: None,
            value: None,
        };
        count as usize
    ];

    while !reader.at_end() {
        let Some(index) = read_element_index(reader, count)? else {
            // Terminator: the C# parser checks if exactly 8 bits remain after
            // the zero terminator and reads one more IntPacked. Replicate that.
            if reader.bits_remaining() == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        };

        let elem = &mut elements[index as usize];
        let mut field_count = 0u32;

        while !reader.at_end() {
            let Some((handle, payload_bits)) = read_field_header(reader)? else {
                break;
            };
            field_count += 1;
            if field_count > MAX_FIELDS_PER_ELEMENT {
                return Err(EffectBlobError::TooManyFields {
                    context: "EffectDataFloat",
                });
            }

            let start_pos = reader.position();
            if handle == handles.tag {
                // FGameplayTag: IntPacked tag index
                elem.tag_index = Some(reader.read_int_packed()?);
            } else if handle == handles.value {
                // Float32
                expect_width("EffectDataFloat value", 32, payload_bits)?;
                elem.value = Some(reader.read_f32()?);
            } else {
                // Unknown handle: skip the payload
                reader.skip_bits(u64::from(payload_bits))?;
            }

            // Ensure we consumed exactly payload_bits
            settle_field(reader, start_pos, payload_bits)?;
        }
    }

    Ok(elements)
}

/// Decode a `TArray<FEffectDataObject>` RepLayout dynamic array.
///
/// # Arguments
/// - `reader`: bit reader positioned at the start of the ObjectValues blob.
///
/// # Returns
/// A vector of decoded object-reference data elements.
///
/// # Wire layout
/// ```text
/// [IntPacked: count]
/// elements (sparse, terminated by index=0):
///   [IntPacked: index+1]
///   fields (terminated by handle=0):
///     handle 15: [IntPacked: bits] [IntPacked: tag_index]
///     handle 16: [IntPacked: bits] [IntPacked: net_guid]
/// ```
pub fn decode_effect_objects(reader: &mut BitReader<'_>) -> Result<Vec<EffectDataObject>> {
    decode_effect_objects_at(reader, OBJECT_HANDLES)
}

/// [`decode_effect_objects`] with the element's handle pair supplied.
///
/// See [`EffectHandles`] for why the pair is not a constant.
pub fn decode_effect_objects_at(
    reader: &mut BitReader<'_>,
    handles: EffectHandles,
) -> Result<Vec<EffectDataObject>> {
    let count = read_array_count(reader)?;
    let mut elements = vec![
        EffectDataObject {
            tag_index: None,
            value: None,
        };
        count as usize
    ];

    while !reader.at_end() {
        let Some(index) = read_element_index(reader, count)? else {
            if reader.bits_remaining() == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        };

        let elem = &mut elements[index as usize];
        let mut field_count = 0u32;

        while !reader.at_end() {
            let Some((handle, payload_bits)) = read_field_header(reader)? else {
                break;
            };
            field_count += 1;
            if field_count > MAX_FIELDS_PER_ELEMENT {
                return Err(EffectBlobError::TooManyFields {
                    context: "EffectDataObject",
                });
            }

            let start_pos = reader.position();
            if handle == handles.tag {
                // FGameplayTag: IntPacked tag index
                elem.tag_index = Some(reader.read_int_packed()?);
            } else if handle == handles.value {
                // ObjectNetGuid: IntPacked. Both members of this element type
                // are IntPacked, so width cannot tell them apart -- the tag is
                // identified by being the lower handle. Verified per function
                // on `02d4d478`: the lower handle takes 1 to 5 distinct values
                // drawn from the gameplay-tag space (282, 283, 298, 306,
                // 65535), the upper takes 209 to 580 distinct values spanning
                // the dynamic net-GUID range.
                elem.value = Some(reader.read_int_packed()?);
            } else {
                reader.skip_bits(u64::from(payload_bits))?;
            }

            settle_field(reader, start_pos, payload_bits)?;
        }
    }

    Ok(elements)
}

/// Decode a `TArray<FEffectDataVector>` RepLayout dynamic array.
///
/// # Arguments
/// - `reader`: bit reader positioned at the start of the VectorValues blob.
///
/// # Returns
/// A vector of decoded vector data elements. Each vector has f64 components
/// matching the 192-bit FVector(double) wire format.
///
/// # Wire layout
/// ```text
/// [IntPacked: count]
/// elements (sparse, terminated by index=0):
///   [IntPacked: index+1]
///   fields (terminated by handle=0):
///     handle 11: [IntPacked: bits] [IntPacked: tag_index]
///     handle 12: [IntPacked: bits] [f64: x] [f64: y] [f64: z]
/// ```
pub fn decode_effect_vectors(reader: &mut BitReader<'_>) -> Result<Vec<EffectDataVector>> {
    decode_effect_vectors_at(reader, VECTOR_HANDLES)
}

/// [`decode_effect_vectors`] with the element's handle pair supplied.
///
/// See [`EffectHandles`] for why the pair is not a constant.
pub fn decode_effect_vectors_at(
    reader: &mut BitReader<'_>,
    handles: EffectHandles,
) -> Result<Vec<EffectDataVector>> {
    let count = read_array_count(reader)?;
    let mut elements = vec![
        EffectDataVector {
            tag_index: None,
            value: None,
        };
        count as usize
    ];

    while !reader.at_end() {
        let Some(index) = read_element_index(reader, count)? else {
            if reader.bits_remaining() == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        };

        let elem = &mut elements[index as usize];
        let mut field_count = 0u32;

        while !reader.at_end() {
            let Some((handle, payload_bits)) = read_field_header(reader)? else {
                break;
            };
            field_count += 1;
            if field_count > MAX_FIELDS_PER_ELEMENT {
                return Err(EffectBlobError::TooManyFields {
                    context: "EffectDataVector",
                });
            }

            let start_pos = reader.position();
            if handle == handles.tag {
                // FGameplayTag: IntPacked tag index
                elem.tag_index = Some(reader.read_int_packed()?);
            } else if handle == handles.value {
                // FVector(double): 3 x f64
                expect_width("EffectDataVector value", 192, payload_bits)?;
                let x = reader.read_f64()?;
                let y = reader.read_f64()?;
                let z = reader.read_f64()?;
                elem.value = Some(FVector { x, y, z });
            } else {
                reader.skip_bits(u64::from(payload_bits))?;
            }

            settle_field(reader, start_pos, payload_bits)?;
        }
    }

    Ok(elements)
}

// ---- Export wiring: blob -> JSON ----

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

/// Append a JSON number for a finite `f64`.
///
/// Rust's `Display` for floats is the shortest representation that round-trips,
/// which is always a valid JSON number for a finite value. `1.0` renders as
/// `1`; that is a JSON number too, so no consumer sees a type it cannot read.
fn push_json_f64(out: &mut String, v: f64) {
    use core::fmt::Write as _;
    // Writing into a String is infallible; the Result exists only to satisfy
    // the `fmt::Write` signature.
    let _ = write!(out, "{v}");
}

/// Derive an array's element handle pair by walking its framing.
///
/// Reads structure only -- every field payload is skipped, no handle is
/// interpreted -- so the result does not depend on knowing which function the
/// blob came from. Returns `None` when the array populates no element, which
/// leaves nothing to derive the pair from and nothing for it to decode.
///
/// The derivation is structural on purpose, so that it assumes nothing about
/// how Unreal numbers handles. That independence is what makes the following
/// a real check rather than a tautology: Unreal is documented to number a
/// dynamic array's element handles from the array's own handle plus one, and
/// on `02d4d478` the derived base equals the RPC parameter's own handle plus
/// one for all 53,908 blobs, with no exception. Two unrelated routes to the
/// same number.
///
/// # Errors
/// Rejects any array whose elements do not each carry exactly two fields at
/// adjacent handles, all elements agreeing on the lower one. That is the shape
/// of all 128,000 elements on `02d4d478`, and it is what makes the pair
/// derivable at all; a blob outside it is reported rather than guessed at.
pub fn scan_element_handles(raw: &[u8], bit_count: u32) -> Result<Option<EffectHandles>> {
    let mut reader = new_blob_reader(raw, bit_count)?;
    let count = read_array_count(&mut reader)?;
    let mut base: Option<u32> = None;

    while !reader.at_end() {
        let Some(_index) = read_element_index(&mut reader, count)? else {
            if reader.bits_remaining() == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        };

        let mut seen = [0u32; 2];
        for slot in &mut seen {
            let Some((handle, payload_bits)) = read_field_header(&mut reader)? else {
                return Err(EffectBlobError::ElementFieldCount { found: 0 });
            };
            reader.skip_bits(u64::from(payload_bits))?;
            *slot = handle;
        }
        if let Some((_, payload_bits)) = read_field_header(&mut reader)? {
            // Consume it so the error message's position is not misleading if
            // a caller ever reports one; the blob is rejected either way.
            let _ = reader.skip_bits(u64::from(payload_bits));
            return Err(EffectBlobError::ElementFieldCount { found: 3 });
        }

        // Order within the element is not assumed; the tag is the lower handle.
        let (lo, hi) = (seen[0].min(seen[1]), seen[0].max(seen[1]));
        if hi != lo + 1 {
            return Err(EffectBlobError::NonAdjacentHandles {
                first: seen[0],
                second: seen[1],
            });
        }
        match base {
            None => base = Some(lo),
            Some(known) if known != lo => {
                return Err(EffectBlobError::InconsistentHandleBase {
                    expected: known,
                    found: lo,
                });
            }
            Some(_) => {}
        }
    }

    Ok(base.map(EffectHandles::from_base))
}

/// Build a reader over an exact bit window, without the panic.
fn new_blob_reader(raw: &[u8], bit_count: u32) -> Result<BitReader<'_>> {
    let available = (raw.len() as u64) * 8;
    if u64::from(bit_count) > available {
        // `BitReader::with_bit_len` asserts on this, and a panic in the export
        // path would take the whole run down over one malformed row.
        return Err(EffectBlobError::BitLengthExceedsBuffer {
            bits: bit_count,
            available,
        });
    }
    Ok(BitReader::with_bit_len(raw, u64::from(bit_count)))
}

/// Decode one effect-array blob and render it as a JSON array.
///
/// Each element becomes `{"tag":<u32|null>,"value":<value|null>}`, where the
/// value is a number for [`EffectArrayKind::Float`] and
/// [`EffectArrayKind::Object`] and `{"x":..,"y":..,"z":..}` for
/// [`EffectArrayKind::Vector`]. A sparse array's unpopulated slots keep their
/// position and render both members as `null`, so an element's index in the
/// JSON is its index on the wire.
///
/// # Arguments
/// - `raw`: the parameter payload, as `fields.parquet` stores it.
/// - `bit_count`: the payload's exact bit length. This is the RPC parameter's
///   declared `payload_bits`, **not** `raw.len() * 8` -- the last byte is
///   padded, and feeding the padding in as data is a latent bug the audit in
///   `PROJECT_STATUS.md` 12-D calls out on the Python side.
///
/// # Errors
/// Returns [`EffectBlobError`] if the payload is not a well-formed array of
/// this kind, does not consume its window, or contains a float that JSON
/// cannot represent. The caller keeps the raw bits and counts the failure; it
/// must not substitute a partial or plausible-looking structure.
pub fn decode_effect_blob_json(
    kind: EffectArrayKind,
    raw: &[u8],
    bit_count: u32,
) -> Result<String> {
    // First pass: which handles does *this* function put the element's members
    // under. `None` means no element carries a field, so the pair is both
    // underivable and unused -- any pair decodes such a blob identically.
    let handles = scan_element_handles(raw, bit_count)?.unwrap_or(EffectHandles::from_base(0));
    let mut reader = new_blob_reader(raw, bit_count)?;

    let mut out = String::new();
    out.push('[');
    match kind {
        EffectArrayKind::Float => {
            for (index, elem) in decode_effect_floats_at(&mut reader, handles)?
                .iter()
                .enumerate()
            {
                if index > 0 {
                    out.push(',');
                }
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) if v.is_finite() => push_json_f64(&mut out, f64::from(v)),
                    Some(_) => return Err(EffectBlobError::NonFiniteFloat { index }),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
        EffectArrayKind::Object => {
            for (index, elem) in decode_effect_objects_at(&mut reader, handles)?
                .iter()
                .enumerate()
            {
                if index > 0 {
                    out.push(',');
                }
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) => {
                        use core::fmt::Write as _;
                        let _ = write!(out, "{v}");
                    }
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
        EffectArrayKind::Vector => {
            for (index, elem) in decode_effect_vectors_at(&mut reader, handles)?
                .iter()
                .enumerate()
            {
                if index > 0 {
                    out.push(',');
                }
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) => {
                        if !(v.x.is_finite() && v.y.is_finite() && v.z.is_finite()) {
                            return Err(EffectBlobError::NonFiniteFloat { index });
                        }
                        out.push_str("{\"x\":");
                        push_json_f64(&mut out, v.x);
                        out.push_str(",\"y\":");
                        push_json_f64(&mut out, v.y);
                        out.push_str(",\"z\":");
                        push_json_f64(&mut out, v.z);
                        out.push('}');
                    }
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
    }
    out.push(']');

    // Checked after the decode, not during: the decoders stop at the array
    // terminator by design, so "did it consume the window" is only answerable
    // once they have returned.
    let remaining = reader.bits_remaining();
    if remaining >= 8 {
        return Err(EffectBlobError::ResidualBits { remaining });
    }

    Ok(out)
}

/// Open one element object and write its `tag` member.
fn push_tag(out: &mut String, tag_index: Option<u32>) {
    use core::fmt::Write as _;
    match tag_index {
        Some(t) => {
            let _ = write!(out, "{{\"tag\":{t},\"value\":");
        }
        None => out.push_str("{\"tag\":null,\"value\":"),
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode hex string to bytes (no external crate needed).
    fn decode_hex(hex: &str) -> Vec<u8> {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(clean.len() % 2 == 0, "hex string must have even length");
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Helper to create a BitReader from a hex string, using only the specified
    /// number of bits.
    fn reader_from_hex(hex: &str, bit_count: u64) -> BitReader<'static> {
        let data = decode_hex(hex);
        let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
        BitReader::with_bit_len(leaked, bit_count)
    }

    // ---- FloatValues tests ----

    /// Pin: packet 4368, Sheriff shot. 4 elements:
    /// tag 284=NumProjectiles(1.0), tag 263=AmmoRemaining(5.0),
    /// tag 286=TracerOption(1.0), tag 285=RandomSeed(-1509722752.0)
    #[test]
    fn decode_float_values_sheriff_basic() {
        let hex = "08021020390412400000803f000410200f0412400000a040\
                   000610203d0412400000803f000810203b04124015f9b3ce0000";
        let mut reader = reader_from_hex(hex, 400);
        let result = decode_effect_floats(&mut reader).unwrap();

        assert_eq!(result.len(), 4);
        // Element 0: tag 284, float 1.0 (NumProjectiles)
        assert_eq!(result[0].tag_index, Some(284));
        assert_eq!(result[0].value, Some(1.0));
        // Element 1: tag 263, float 5.0 (AmmoRemaining)
        assert_eq!(result[1].tag_index, Some(263));
        assert_eq!(result[1].value, Some(5.0));
        // Element 2: tag 286, float 1.0 (TracerOption)
        assert_eq!(result[2].tag_index, Some(286));
        assert_eq!(result[2].value, Some(1.0));
        // Element 3: tag 285, float -1509722752.0 (RandomSeed)
        assert_eq!(result[3].tag_index, Some(285));
        assert_eq!(result[3].value, Some(-1509722752.0));
    }

    /// Pin: packet 17421, Classic shot with YawSwitch. 5 elements including
    /// tag 287 = 16.0 (YawSwitch).
    #[test]
    fn decode_float_values_with_yaw_switch() {
        let hex = "0a021020390412400000803f000410200f0412400000e040\
                   000610203d0412400000803f000810203b04124032cb82cc\
                   000a10203f041240000080410000";
        let mut reader = reader_from_hex(hex, 496);
        let result = decode_effect_floats(&mut reader).unwrap();

        assert_eq!(result.len(), 5);
        // Element 4: tag 287, float 16.0 (YawSwitch)
        assert_eq!(result[4].tag_index, Some(287));
        assert_eq!(result[4].value, Some(16.0));
        // Element 1: tag 263, float 7.0 (AmmoRemaining)
        assert_eq!(result[1].tag_index, Some(263));
        assert_eq!(result[1].value, Some(7.0));
    }

    /// Pin: packet 30968, Judge shotgun. 3 elements:
    /// NumProjectiles=12, AmmoRemaining=4, RandomSeed=480247136.0
    /// (no TracerOption for shotguns)
    #[test]
    fn decode_float_values_shotgun() {
        let hex = "060210203904124000004041000410200f04124000008040\
                   000610203b041240ebffe44d0000";
        let mut reader = reader_from_hex(hex, 304);
        let result = decode_effect_floats(&mut reader).unwrap();

        assert_eq!(result.len(), 3);
        // Element 0: tag 284, float 12.0 (NumProjectiles)
        assert_eq!(result[0].tag_index, Some(284));
        assert_eq!(result[0].value, Some(12.0));
        // Element 1: tag 263, float 4.0 (AmmoRemaining)
        assert_eq!(result[1].tag_index, Some(263));
        assert_eq!(result[1].value, Some(4.0));
        // Element 2: tag 285, float 480247136.0 (RandomSeed)
        assert_eq!(result[2].tag_index, Some(285));
        assert_eq!(result[2].value, Some(480247136.0));
    }

    // ---- ObjectValues tests ----

    /// Pin: packet 4368. 4 elements:
    /// tag 283=FiringState(3086), tag 282=FiringPlayerState(268),
    /// tag 65535=unknown(2731), tag 306=unknown(1466)
    #[test]
    fn decode_object_values_basic() {
        let hex = "08022020370422201d300004202035042220190400\
                   062030ffff062220572a000820206504222075160000";
        let mut reader = reader_from_hex(hex, 344);
        let result = decode_effect_objects(&mut reader).unwrap();

        assert_eq!(result.len(), 4);
        // Element 0: tag 283, object 3086 (FiringState)
        assert_eq!(result[0].tag_index, Some(283));
        assert_eq!(result[0].value, Some(3086));
        // Element 1: tag 282, object 268 (FiringPlayerState)
        assert_eq!(result[1].tag_index, Some(282));
        assert_eq!(result[1].value, Some(268));
        // Element 2: tag 65535
        assert_eq!(result[2].tag_index, Some(65535));
        assert_eq!(result[2].value, Some(2731));
        // Element 3: tag 306
        assert_eq!(result[3].tag_index, Some(306));
        assert_eq!(result[3].value, Some(1466));
    }

    // ---- VectorValues tests ----

    /// Pin: packet 4368, single attack vector for a Sheriff shot.
    /// Expected: (-0.7793076561609785, 0.6228944653768754, -0.06842559500463913)
    #[test]
    fn decode_vector_values_single_pellet() {
        let hex = "0202182013041a81026b7b179c16f0e8bf11e6b45fc0eee33f\
                   9417c1fc5684b1bf0000";
        let mut reader = reader_from_hex(hex, 280);
        let result = decode_effect_vectors(&mut reader).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tag_index, Some(265));
        let v = result[0].value.unwrap();
        assert!((v.x - (-0.7793076561609785)).abs() < 1e-15);
        assert!((v.y - 0.6228944653768754).abs() < 1e-15);
        assert!((v.z - (-0.06842559500463913)).abs() < 1e-15);
    }

    /// Pin: packet 30968, Judge shotgun with 12 attack vectors.
    /// Verifies element count and first/last vectors.
    #[test]
    fn decode_vector_values_shotgun_12_pellets() {
        let hex = "1802182013041a8102dbc17bd5d196e5bfe0a5c85f789ee73f\
                   2c6acea060d67cbf0004182023041a8102d928d6ec3252e4bf\
                   db684947f09ae83f8ff46bf204feb23f0006182025041a8102\
                   cab091674d89e3bfca826e887d57e93f329135a84dc986bf00\
                   08182027041a81021feb5fbfc5c9e4bfbe2eacda9150e83fd4\
                   e0ab8708b8993f000a182029041a8102eb1e013403a6e5bf70\
                   586cb99487e73f211db57f12dda43f000c18202b041a8102cf\
                   8ff307062fe5bfbc8f8d8712eae73f9ea909fb1d4fadbf000e\
                   18202d041a8102b2d91d6d819be4bf868429ea797ae83f82a1\
                   7b5833f487bf001018202f041a81024c18cf0e9622e6bfebed\
                   6acd4518e73f1643e10a3d219a3f0012182031041a81021e4a\
                   512e352ee5bf896bbcfad9fae73fa65e98e42cf892bf001418\
                   2015041a81023f37da4a7fa9e3bff6ce548e533ae93f0ec90b\
                   be86529f3f0016182017041a8102246b3d964f35e3bf7de713\
                   59cc90e93f0801a7f26937a33f0018182019041a81028e462c\
                   93744fe3bf46adf442357ae93f7f3bcd1e38b3a6bf0000";
        let mut reader = reader_from_hex(hex, 3184);
        let result = decode_effect_vectors(&mut reader).unwrap();

        assert_eq!(result.len(), 12);
        // First vector: x=-0.6746606034849337, y=0.7380945082451795, z=-0.0070403837716193456
        let v0 = result[0].value.unwrap();
        assert!((v0.x - (-0.6746606034849337)).abs() < 1e-12);
        assert!((v0.y - 0.7380945082451795).abs() < 1e-12);
        assert!((v0.z - (-0.0070403837716193456)).abs() < 1e-12);

        // Last vector (index 11): x=-0.6034491419288359, y=0.796167975209223, z=-0.04433608413693601
        let v11 = result[11].value.unwrap();
        assert!((v11.x - (-0.6034491419288359)).abs() < 1e-12);
        assert!((v11.y - 0.796167975209223).abs() < 1e-12);
        assert!((v11.z - (-0.04433608413693601)).abs() < 1e-12);
    }

    /// Empty blob (0 elements).
    #[test]
    fn decode_empty_float_array() {
        // IntPacked 0 = byte 0x00
        let data = [0u8; 1];
        let mut reader = BitReader::with_bit_len(&data, 8);
        let result = decode_effect_floats(&mut reader).unwrap();
        assert!(result.is_empty());
    }

    /// Empty blob (0 elements) for objects.
    #[test]
    fn decode_empty_object_array() {
        let data = [0u8; 1];
        let mut reader = BitReader::with_bit_len(&data, 8);
        let result = decode_effect_objects(&mut reader).unwrap();
        assert!(result.is_empty());
    }

    /// Empty blob (0 elements) for vectors.
    #[test]
    fn decode_empty_vector_array() {
        let data = [0u8; 1];
        let mut reader = BitReader::with_bit_len(&data, 8);
        let result = decode_effect_vectors(&mut reader).unwrap();
        assert!(result.is_empty());
    }

    // ---- JSON wiring tests ----

    #[test]
    fn param_names_select_the_element_type() {
        assert_eq!(
            EffectArrayKind::from_param_name("FloatValues"),
            Some(EffectArrayKind::Float)
        );
        assert_eq!(
            EffectArrayKind::from_param_name("ObjectValues"),
            Some(EffectArrayKind::Object)
        );
        assert_eq!(
            EffectArrayKind::from_param_name("VectorValues"),
            Some(EffectArrayKind::Vector)
        );
        // Every other RPC parameter, including ones from the same functions.
        assert_eq!(EffectArrayKind::from_param_name("EffectID"), None);
        assert_eq!(EffectArrayKind::from_param_name("SourceID"), None);
        assert_eq!(EffectArrayKind::from_param_name("Translation"), None);
        assert_eq!(EffectArrayKind::from_param_name("248"), None);
        assert_eq!(EffectArrayKind::from_param_name("floatvalues"), None);
    }

    /// The whole string is asserted, not a substring: a member carrying no
    /// data is exactly where a serialization bug hides (see 13-B).
    #[test]
    fn float_blob_renders_as_json() {
        let hex = "08021020390412400000803f000410200f0412400000a040\
                   000610203d0412400000803f000810203b04124015f9b3ce0000";
        let raw = decode_hex(hex);
        let json = decode_effect_blob_json(EffectArrayKind::Float, &raw, 400).unwrap();
        assert_eq!(
            json,
            "[{\"tag\":284,\"value\":1},\
              {\"tag\":263,\"value\":5},\
              {\"tag\":286,\"value\":1},\
              {\"tag\":285,\"value\":-1509722752}]"
        );
    }

    #[test]
    fn object_blob_renders_as_json() {
        let hex = "08022020370422201d300004202035042220190400\
                   062030ffff062220572a000820206504222075160000";
        let raw = decode_hex(hex);
        let json = decode_effect_blob_json(EffectArrayKind::Object, &raw, 344).unwrap();
        assert_eq!(
            json,
            "[{\"tag\":283,\"value\":3086},\
              {\"tag\":282,\"value\":268},\
              {\"tag\":65535,\"value\":2731},\
              {\"tag\":306,\"value\":1466}]"
        );
    }

    #[test]
    fn vector_blob_renders_as_json() {
        let hex = "0202182013041a81026b7b179c16f0e8bf11e6b45fc0eee33f\
                   9417c1fc5684b1bf0000";
        let raw = decode_hex(hex);
        let json = decode_effect_blob_json(EffectArrayKind::Vector, &raw, 280).unwrap();
        assert_eq!(
            json,
            "[{\"tag\":265,\"value\":{\"x\":-0.7793076561609785,\
              \"y\":0.6228944653768754,\"z\":-0.06842559500463913}}]"
        );
    }

    #[test]
    fn an_empty_blob_renders_as_an_empty_array() {
        let json = decode_effect_blob_json(EffectArrayKind::Float, &[0u8], 8).unwrap();
        assert_eq!(json, "[]");
    }

    /// A payload that is not this format usually parses into something and
    /// leaves a tail. That tail is the only thing separating a decode from a
    /// plausible-looking fabrication, so it must be an error, not a warning.
    #[test]
    fn a_tail_after_the_terminator_is_an_error() {
        // A well-formed 1-element float array followed by 6 spare bytes.
        let hex = "0202102039041240000080 3f0000 00000000000000";
        let raw = decode_hex(&hex.replace(' ', ""));
        let bits = (raw.len() as u32) * 8;
        let err = decode_effect_blob_json(EffectArrayKind::Float, &raw, bits).unwrap_err();
        assert!(
            matches!(err, EffectBlobError::ResidualBits { .. }),
            "expected ResidualBits, got {err:?}"
        );
    }

    /// `BitReader::with_bit_len` asserts rather than returning an error, and a
    /// panic in the export path would take the whole run down over one row.
    #[test]
    fn a_bit_length_past_the_buffer_is_an_error_not_a_panic() {
        let err = decode_effect_blob_json(EffectArrayKind::Float, &[0u8], 4096).unwrap_err();
        assert!(
            matches!(
                err,
                EffectBlobError::BitLengthExceedsBuffer {
                    bits: 4096,
                    available: 8
                }
            ),
            "expected BitLengthExceedsBuffer, got {err:?}"
        );
    }

    /// JSON has no literal for NaN or an infinity. Rendering one bare would
    /// produce a document no strict reader accepts; coercing it to `null` or
    /// `0` would fabricate. The blob is rejected so the caller counts it.
    #[test]
    fn a_non_finite_float_is_rejected_rather_than_rendered() {
        // Element 0, tag 284, value = f32::NAN (0x7fc00000).
        let hex = "020210203904124000 00c07f 0000";
        let raw = decode_hex(&hex.replace(' ', ""));
        let err = decode_effect_blob_json(EffectArrayKind::Float, &raw, 112).unwrap_err();
        assert!(
            matches!(err, EffectBlobError::NonFiniteFloat { index: 0 }),
            "expected NonFiniteFloat, got {err:?}"
        );
    }

    // ---- Handle derivation ----

    /// The pinned vectors come from `ReplayPlayContinuousEffectAtLocation`, so
    /// the scan must recover exactly the constants the fixed-handle decoders
    /// use -- derivation and declaration agreeing on the one function where
    /// both are known.
    #[test]
    fn scanning_recovers_the_declared_handles() {
        let floats = decode_hex(
            "08021020390412400000803f000410200f0412400000a040\
             000610203d0412400000803f000810203b04124015f9b3ce0000",
        );
        assert_eq!(
            scan_element_handles(&floats, 400).unwrap(),
            Some(FLOAT_HANDLES)
        );

        let objects = decode_hex(
            "08022020370422201d300004202035042220190400\
             062030ffff062220572a000820206504222075160000",
        );
        assert_eq!(
            scan_element_handles(&objects, 344).unwrap(),
            Some(OBJECT_HANDLES)
        );

        let vectors = decode_hex(
            "0202182013041a81026b7b179c16f0e8bf11e6b45fc0eee33f\
             9417c1fc5684b1bf0000",
        );
        assert_eq!(
            scan_element_handles(&vectors, 280).unwrap(),
            Some(VECTOR_HANDLES)
        );
    }

    /// An array that populates no element has no pair to derive.
    #[test]
    fn an_empty_array_has_no_handles_to_derive() {
        assert_eq!(scan_element_handles(&[0u8], 8).unwrap(), None);
    }

    /// The whole point of the scan: the same struct under a different function
    /// arrives at a different handle pair, and the fixed constants would read
    /// it as all-null (or worse -- see `EffectHandles`). This is the pinned
    /// Sheriff FloatValues blob with its handles rewritten from 7/8 to 3/4,
    /// which is where `ClientPlayOneShotEffectAtLocation` puts them.
    #[test]
    fn a_rebased_blob_decodes_through_derivation_and_not_through_the_constants() {
        // Handle bytes are IntPacked(handle + 1): 0x10 -> 7 and 0x12 -> 8
        // become 0x08 -> 3 and 0x0a -> 4.
        let hex = "0802082039040a400000803f000408200f040a400000a040\
                   000608203d040a400000803f000808203b040a4015f9b3ce0000";
        let raw = decode_hex(hex);
        assert_eq!(
            scan_element_handles(&raw, 400).unwrap(),
            Some(EffectHandles::from_base(3))
        );

        // Through the constants: every field is an unknown handle, so the
        // elements come back empty. This is what shipped before derivation.
        let mut reader = BitReader::with_bit_len(&raw, 400);
        let blind = decode_effect_floats(&mut reader).unwrap();
        assert_eq!(blind.len(), 4);
        assert!(
            blind
                .iter()
                .all(|e| e.tag_index.is_none() && e.value.is_none())
        );

        // Through derivation: the same values as the 7/8 original.
        let json = decode_effect_blob_json(EffectArrayKind::Float, &raw, 400).unwrap();
        assert_eq!(
            json,
            "[{\"tag\":284,\"value\":1},\
              {\"tag\":263,\"value\":5},\
              {\"tag\":286,\"value\":1},\
              {\"tag\":285,\"value\":-1509722752}]"
        );
    }

    /// A float value field must declare 32 bits. Without the check the decoder
    /// reads 32 bits regardless and runs off the end of its own field.
    #[test]
    fn a_value_field_of_the_wrong_width_is_rejected() {
        // Element 0: tag at handle 3 (16 bits), value at handle 4 declaring
        // 16 bits where a float needs 32.
        let hex = "0202082039040a2000000000";
        let raw = decode_hex(hex);
        let err = decode_effect_blob_json(EffectArrayKind::Float, &raw, 96).unwrap_err();
        assert!(
            matches!(
                err,
                EffectBlobError::UnexpectedPayloadWidth {
                    expected: 32,
                    found: 16,
                    ..
                }
            ),
            "expected UnexpectedPayloadWidth, got {err:?}"
        );
    }
}
