//! The three `FEffectData*` element types and their array decoders.
//!
//! All three arrays share one framing and differ only in how the value member
//! is read, so the element loop is written once over the [`EffectElement`]
//! trait. The three public decoders differ from each other in exactly the four
//! lines the trait declares -- which is the point: an element loop copied three
//! times is three places for the `settle_field` accounting to drift.

use vrf_bitio::BitReader;

use super::framing::{
    MAX_FIELDS_PER_ELEMENT, consume_trailing_terminator, expect_width, read_array_count,
    read_element_index, read_field_header, settle_field,
};
use super::{
    EffectBlobError, EffectHandles, FLOAT_HANDLES, OBJECT_HANDLES, Result, VECTOR_HANDLES,
};
use crate::types::FVector;

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

/// What an element type has to say for itself so the shared loop can decode it.
trait EffectElement: Copy {
    /// Names this element type in a [`EffectBlobError::TooManyFields`].
    const CONTEXT: &'static str;
    /// A slot the wire never populated. A sparse array keeps these.
    const ABSENT: Self;

    fn set_tag(&mut self, tag_index: u32);

    /// Read the value member, having already checked the handle. `payload_bits`
    /// is the field's declared width -- the implementations that know their own
    /// width check it here, because reading a wider type than the field
    /// declared would run into the next field.
    fn read_value(&mut self, reader: &mut BitReader<'_>, payload_bits: u32) -> Result<()>;
}

impl EffectElement for EffectDataFloat {
    const CONTEXT: &'static str = "EffectDataFloat";
    const ABSENT: Self = Self {
        tag_index: None,
        value: None,
    };

    fn set_tag(&mut self, tag_index: u32) {
        self.tag_index = Some(tag_index);
    }

    fn read_value(&mut self, reader: &mut BitReader<'_>, payload_bits: u32) -> Result<()> {
        expect_width("EffectDataFloat value", 32, payload_bits)?;
        self.value = Some(reader.read_f32()?);
        Ok(())
    }
}

impl EffectElement for EffectDataObject {
    const CONTEXT: &'static str = "EffectDataObject";
    const ABSENT: Self = Self {
        tag_index: None,
        value: None,
    };

    fn set_tag(&mut self, tag_index: u32) {
        self.tag_index = Some(tag_index);
    }

    /// ObjectNetGuid: IntPacked. Both members of this element type are
    /// IntPacked, so width cannot tell them apart -- the tag is identified by
    /// being the lower handle. Verified per function on `02d4d478`: the lower
    /// handle takes 1 to 5 distinct values drawn from the gameplay-tag space
    /// (282, 283, 298, 306, 65535), the upper takes 209 to 580 distinct values
    /// spanning the dynamic net-GUID range.
    fn read_value(&mut self, reader: &mut BitReader<'_>, _payload_bits: u32) -> Result<()> {
        self.value = Some(reader.read_int_packed()?);
        Ok(())
    }
}

impl EffectElement for EffectDataVector {
    const CONTEXT: &'static str = "EffectDataVector";
    const ABSENT: Self = Self {
        tag_index: None,
        value: None,
    };

    fn set_tag(&mut self, tag_index: u32) {
        self.tag_index = Some(tag_index);
    }

    fn read_value(&mut self, reader: &mut BitReader<'_>, payload_bits: u32) -> Result<()> {
        expect_width("EffectDataVector value", 192, payload_bits)?;
        let x = reader.read_f64()?;
        let y = reader.read_f64()?;
        let z = reader.read_f64()?;
        self.value = Some(FVector { x, y, z });
        Ok(())
    }
}

/// Decode one `TArray<FEffectData*>` given the handle pair its function uses.
///
/// The returned vector always has the array's declared length: slots the wire
/// left unpopulated stay [`EffectElement::ABSENT`] so an element's index in the
/// output is its index on the wire.
fn decode_elements<T: EffectElement>(
    reader: &mut BitReader<'_>,
    handles: EffectHandles,
) -> Result<Vec<T>> {
    let count = read_array_count(reader)?;
    let mut elements = vec![T::ABSENT; count as usize];

    while !reader.at_end() {
        let Some(index) = read_element_index(reader, count)? else {
            consume_trailing_terminator(reader);
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
                    context: T::CONTEXT,
                });
            }

            let start_pos = reader.position();
            if handle == handles.tag {
                // FGameplayTag: IntPacked tag index
                elem.set_tag(reader.read_int_packed()?);
            } else if handle == handles.value {
                elem.read_value(reader, payload_bits)?;
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
    decode_elements(reader, handles)
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
    decode_elements(reader, handles)
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
    decode_elements(reader, handles)
}
