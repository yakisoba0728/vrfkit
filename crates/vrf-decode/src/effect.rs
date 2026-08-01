//! Decoder for the Valorant shot-effect RepLayoutDynamicArray blobs.
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
            match handle {
                7 => {
                    // FGameplayTag: IntPacked tag index
                    elem.tag_index = Some(reader.read_int_packed()?);
                }
                8 => {
                    // Float32
                    elem.value = Some(reader.read_f32()?);
                }
                _ => {
                    // Unknown handle: skip the payload
                    reader.skip_bits(u64::from(payload_bits))?;
                }
            }

            // Ensure we consumed exactly payload_bits
            let consumed = reader.position() - start_pos;
            if consumed < u64::from(payload_bits) {
                reader.skip_bits(u64::from(payload_bits) - consumed)?;
            }
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
            match handle {
                15 => {
                    // FGameplayTag: IntPacked tag index
                    elem.tag_index = Some(reader.read_int_packed()?);
                }
                16 => {
                    // ObjectNetGuid: IntPacked
                    elem.value = Some(reader.read_int_packed()?);
                }
                _ => {
                    reader.skip_bits(u64::from(payload_bits))?;
                }
            }

            let consumed = reader.position() - start_pos;
            if consumed < u64::from(payload_bits) {
                reader.skip_bits(u64::from(payload_bits) - consumed)?;
            }
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
            match handle {
                11 => {
                    // FGameplayTag: IntPacked tag index
                    elem.tag_index = Some(reader.read_int_packed()?);
                }
                12 => {
                    // FVector(double): 3 x f64
                    let x = reader.read_f64()?;
                    let y = reader.read_f64()?;
                    let z = reader.read_f64()?;
                    elem.value = Some(FVector { x, y, z });
                }
                _ => {
                    reader.skip_bits(u64::from(payload_bits))?;
                }
            }

            let consumed = reader.position() - start_pos;
            if consumed < u64::from(payload_bits) {
                reader.skip_bits(u64::from(payload_bits) - consumed)?;
            }
        }
    }

    Ok(elements)
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
}
