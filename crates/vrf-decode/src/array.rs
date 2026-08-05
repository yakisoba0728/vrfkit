//! RepLayout DynamicArray decoder -- parses nested struct arrays from raw bits.
//!
//! # Wire format (confirmed against C# `RepLayoutArrayDecoders.cs`)
//!
//! ```text
//! DynamicArray:
//!   elementCount = IntPacked         // declared capacity
//!   loop:
//!     encodedIndex = IntPacked       // 0 = terminator
//!     index = encodedIndex - 1
//!     if index >= elementCount:
//!       skip remaining bits -> break
//!     [element payload]:
//!       if primitive array:
//!         single-value decode (e.g. ObjectNetGuid per element)
//!       if struct array:
//!         loop:                      // same handle/payloadBits framing as RepLayout
//!           encodedHandle = IntPacked  // 0 = end of element
//!           handle = encodedHandle - 1
//!           payloadBits = IntPacked
//!           if payloadBits == 0: continue
//!           field data = payloadBits bits
//!           -> recurse if this field is itself an array
//! ```
//!
//! # Recursion limits
//!
//! The C# parser uses `MaxItems = 256` for array element count and `MaxFields = 128`
//! for fields per struct element (see `CombatRoundReportsDecoder`). The generic
//! `DynamicArrayDecoder` has no explicit limit but fails on malformed data via
//! `BitsRemaining` checks.
//!
//! We use:
//! - **MAX_ELEMENTS = 4096** per array (generous; real data peaks ~50)
//! - **MAX_FIELDS_PER_ELEMENT = 128** struct fields per element
//! - **MAX_RECURSION_DEPTH = 12** levels of nesting (real data is 4-5 deep)
//!
//! When a limit is hit we stop decoding that branch, preserve remaining raw bits
//! in the output, and increment a truncation counter (never panic/error).
//!
//! # Output: flattened field records
//!
//! Each leaf value produces a `FlattenedField` with:
//! - `path`: e.g. `"Rounds[3].Reports[1].DamageDealt"` or `"Rounds[0]._h18"`
//!   (for fields with unknown names, the handle is used as suffix)
//! - `handle`: the leaf field's handle within its struct
//! - `bit_count`: payload bits
//! - `raw_bits`: the raw bytes of this field's payload
//!
//! The path string format was chosen because:
//! 1. DuckDB/pandas can filter with `LIKE 'Rounds[%].Reports[%].DamageDealt'`
//! 2. Parquet dictionary encoding collapses repeated path prefixes efficiently
//! 3. The upstream Python pipeline (compute_metrics.py) can parse it trivially
//!
//! # Allocation
//!
//! One `String` and one `Vec<u8>` per emitted leaf are unavoidable --
//! [`FlattenedField`] owns both. Everything else is written into a single
//! reused path buffer: labels in particular are appended in place rather than
//! resolved into a temporary `String` that is copied and dropped, which was one
//! extra allocation per leaf for nothing.

mod schema;

use std::fmt::Write as _;

use vrf_bitio::BitReader;

pub use schema::{ArrayFieldSchema, COMBAT_ROUNDS_SCHEMA};

/// Maximum number of elements per array level.
pub const MAX_ELEMENTS: u32 = 4096;
/// Maximum number of fields per struct element.
pub const MAX_FIELDS_PER_ELEMENT: u32 = 128;
/// Maximum recursion depth for nested arrays.
pub const MAX_RECURSION_DEPTH: u32 = 12;

/// Starting capacity for the emitted-field vector. The reference replay's
/// CombatReport rounds run to a few hundred leaves per blob; this skips the
/// first handful of doublings without over-reserving for a small array.
const INITIAL_FIELD_CAPACITY: usize = 32;

/// A single flattened field emitted from array decoding.
#[derive(Debug, Clone)]
pub struct FlattenedField {
    /// Dotted path from the array root, e.g. `"[0].RoundNumber"` or
    /// `"[2].Reports[1].DamageDealt"`. The caller prepends the parent field
    /// name (e.g. `"Rounds"`) to form the final `field_name`.
    pub path: String,
    /// Handle of the leaf field within its containing struct.
    pub handle: u32,
    /// Number of payload bits.
    pub bit_count: u32,
    /// Raw payload bytes (ceil(bit_count/8) bytes).
    pub raw_bits: Vec<u8>,
}

/// Statistics from one array decode pass.
#[derive(Debug, Clone, Default)]
pub struct ArrayDecodeStats {
    /// Total elements decoded across all array levels.
    pub elements_decoded: u64,
    /// Total leaf fields emitted.
    pub fields_emitted: u64,
    /// Number of times a limit was hit (element count, field count, or depth).
    pub truncations: u64,
    /// BitIo read failures or declared-vs-available overruns the walker
    /// recovered from by abandoning the rest of the stream.
    ///
    /// Without this, a truncated element-count read or a payload that EOFs
    /// mid-element returned an empty/partial `Vec` with `truncations == 0`,
    /// indistinguishable from a legitimately empty array. The parent row's
    /// `raw_bits` are still emitted by the caller (`emit_flattened_array`
    /// always emits the parent), so this counter is the only signal that
    /// flattened leaves were lost. Mirrors `struct_blobs_failed`: counted and
    /// surfaced, never silently dropped.
    pub errors: u64,
}

/// Everything one walk carries down through the recursion.
///
/// `declared` and `output`/`stats` are the same for every level; bundling them
/// keeps the recursive calls to three arguments instead of seven.
struct Walk<'a, 'd> {
    /// Names the REPLAY declares for this group's handles, indexed by handle.
    declared: &'a [Option<&'d str>],
    /// Path under construction, reused across every leaf.
    path: String,
    output: Vec<FlattenedField>,
}

/// Decode a DynamicArray (struct elements) from raw bits and emit flattened fields.
///
/// `schema` tells us which handles at each depth are themselves arrays (so we
/// recurse) vs primitives (so we emit as-is). If `schema` is None, we treat all
/// elements as opaque structs and emit each field without attempting to recurse
/// into sub-arrays.
///
/// `declared` carries the names the REPLAY itself declares for this group's
/// handles, indexed by handle: slot `h` holds the declared name for handle `h`,
/// or `None` where the replay declares nothing there. Pass `&[]` when no
/// declaration is available; the schema's own names are then the only source.
/// A LEAF label is resolved declaration -> schema -> `_h{handle}`; container
/// segments come from the schema alone. See [`push_leaf_label`].
pub fn decode_struct_array(
    data: &[u8],
    bit_count: u32,
    schema: Option<&ArrayFieldSchema>,
    declared: &[Option<&str>],
    stats: &mut ArrayDecodeStats,
) -> Vec<FlattenedField> {
    let mut reader = BitReader::with_bit_len(data, u64::from(bit_count));
    let mut walk = Walk {
        declared,
        path: String::with_capacity(64),
        output: Vec::with_capacity(INITIAL_FIELD_CAPACITY),
    };
    decode_array_level(&mut reader, &mut walk, schema, 0, stats);
    walk.output
}

/// Recursively decode one array level.
fn decode_array_level(
    reader: &mut BitReader<'_>,
    walk: &mut Walk<'_, '_>,
    schema: Option<&ArrayFieldSchema>,
    depth: u32,
    stats: &mut ArrayDecodeStats,
) {
    if reader.at_end() {
        return;
    }

    // Read element count.
    let Ok(element_count) = reader.read_int_packed() else {
        stats.errors += 1;
        return;
    };

    if element_count > MAX_ELEMENTS {
        stats.truncations += 1;
        // Emit remaining as a single raw field at this level.
        emit_remaining_raw(reader, walk);
        return;
    }

    // Read elements.
    while !reader.at_end() {
        let Ok(encoded_index) = reader.read_int_packed() else {
            stats.errors += 1;
            break;
        };

        if encoded_index == 0 {
            // Check for the trailing 8-bit terminator that the C# parser handles:
            // "if (archive.BitsRemaining == 8) { _ = archive.ReadIntPacked(); }"
            // This occurs in struct arrays when exactly 8 bits remain after the
            // zero terminator. We consume it silently.
            if reader.bits_remaining() == 8 {
                let _ = reader.read_int_packed();
            }
            break;
        }

        let index = encoded_index - 1;
        if index >= element_count {
            // Index exceeds declared count -- skip remaining.
            reader.skip_remaining();
            break;
        }

        stats.elements_decoded += 1;
        let prefix_len = walk.path.len();
        let _ = write!(walk.path, "[{index}]");
        decode_struct_fields(reader, walk, schema, depth, stats);
        walk.path.truncate(prefix_len);
    }
}

/// Decode the struct fields of one array element (handle/payloadBits loop).
fn decode_struct_fields(
    reader: &mut BitReader<'_>,
    walk: &mut Walk<'_, '_>,
    schema: Option<&ArrayFieldSchema>,
    depth: u32,
    stats: &mut ArrayDecodeStats,
) {
    for _field_idx in 0..MAX_FIELDS_PER_ELEMENT {
        if reader.at_end() {
            return;
        }

        let Ok(encoded_handle) = reader.read_int_packed() else {
            stats.errors += 1;
            return;
        };
        if encoded_handle == 0 {
            return;
        }
        let handle = encoded_handle - 1;

        let Ok(payload_bits) = reader.read_int_packed() else {
            stats.errors += 1;
            return;
        };
        if payload_bits == 0 {
            continue;
        }
        if u64::from(payload_bits) > reader.bits_remaining() {
            // Malformed: declared more bits than available. The abandoned bits
            // are counted as an error so the loss of flattened leaves is
            // visible, never a silent empty Vec.
            stats.errors += 1;
            reader.skip_remaining();
            return;
        }

        match schema.and_then(|s| s.sub_array(handle)) {
            // A nested array we are still allowed to descend into.
            Some(sub) if depth + 1 < MAX_RECURSION_DEPTH => {
                let Ok(mut sub_reader) = reader.sub_reader(u64::from(payload_bits)) else {
                    return;
                };
                let prefix_len = walk.path.len();
                walk.path.push('.');
                push_field_label(&mut walk.path, schema, handle);
                decode_array_level(&mut sub_reader, walk, Some(sub), depth + 1, stats);
                // Any bits the sub-reader left are dropped on purpose: the
                // parent's window already accounted for them.
                walk.path.truncate(prefix_len);
            }
            // Same, but the nesting limit stops us -- keep the bits instead.
            Some(_) => {
                stats.truncations += 1;
                let Some(raw) = copy_payload(reader, payload_bits) else {
                    return;
                };
                emit(walk, stats, handle, payload_bits, raw, |path| {
                    push_field_label(path, schema, handle);
                });
            }
            // Leaf field -- emit as-is.
            None => {
                let Some(raw) = copy_payload(reader, payload_bits) else {
                    return;
                };
                let declared = walk.declared;
                emit(walk, stats, handle, payload_bits, raw, |path| {
                    push_leaf_label(path, declared, schema, handle);
                });
            }
        }
    }

    // Hit MAX_FIELDS_PER_ELEMENT -- truncate.
    stats.truncations += 1;
}

/// Take `payload_bits` bits out of `reader` as owned bytes.
///
/// Returns `None` when the window cannot be opened, which is the caller's
/// signal to stop this element rather than emit a partial record.
fn copy_payload(reader: &mut BitReader<'_>, payload_bits: u32) -> Option<Vec<u8>> {
    let mut raw = vec![0u8; (payload_bits as usize).div_ceil(8)];
    let mut sub_reader = reader.sub_reader(u64::from(payload_bits)).ok()?;
    let _ = sub_reader.copy_bits_to(&mut raw, u64::from(payload_bits));
    Some(raw)
}

/// Append `.<label>` to the walk's path, push one record, and restore the path.
fn emit(
    walk: &mut Walk<'_, '_>,
    stats: &mut ArrayDecodeStats,
    handle: u32,
    bit_count: u32,
    raw_bits: Vec<u8>,
    label: impl FnOnce(&mut String),
) {
    let prefix_len = walk.path.len();
    walk.path.push('.');
    label(&mut walk.path);
    walk.output.push(FlattenedField {
        path: walk.path.clone(),
        handle,
        bit_count,
        raw_bits,
    });
    stats.fields_emitted += 1;
    walk.path.truncate(prefix_len);
}

/// Emit all remaining bits as a single raw field.
fn emit_remaining_raw(reader: &mut BitReader<'_>, walk: &mut Walk<'_, '_>) {
    let remaining = reader.bits_remaining();
    if remaining == 0 {
        return;
    }
    let mut raw = vec![0u8; (remaining as usize).div_ceil(8)];
    let _ = reader.copy_bits_to(&mut raw, remaining);
    let mut path = walk.path.clone();
    path.push_str("._raw");
    walk.output.push(FlattenedField {
        path,
        handle: u32::MAX,
        bit_count: remaining as u32,
        raw_bits: raw,
    });
}

/// Append a LEAF handle's label, preferring the name the replay declares.
///
/// Order: the replay's own net field export, then the hardcoded schema, then
/// `_h{handle}`.
///
/// The replay wins because it is the wire's own statement about the field, and
/// the schema is a transcription of the C# reference that can disagree with it:
/// handle 3 is `RoundNum` on the wire and `RoundNumber` in the schema, and
/// Riot's spellings carry typos (`DamageRecieved`, `HitsRecieved`) that the
/// schema silently corrected. The schema stays as the floor for handles a
/// replay does not declare.
///
/// Only LEAF labels use this. Container segments -- the path components a
/// sub-array introduces -- keep [`push_field_label`], because the schema is
/// what decides the nesting in the first place and the declaration adds nothing
/// there: handles 44 and 79 both declare `RegionalDamageInteractions`, so the
/// wire cannot tell the two apart where the schema can.
fn push_leaf_label(
    path: &mut String,
    declared: &[Option<&str>],
    schema: Option<&ArrayFieldSchema>,
    handle: u32,
) {
    if let Some(name) = declared.get(handle as usize).copied().flatten() {
        path.push_str(name);
        return;
    }
    push_field_label(path, schema, handle);
}

/// Append a handle's label from the schema's name map, falling back to
/// `_h{handle}` when the schema cannot name it.
fn push_field_label(path: &mut String, schema: Option<&ArrayFieldSchema>, handle: u32) {
    match schema.and_then(|s| s.field_name(handle)) {
        Some(name) => path.push_str(name),
        None => {
            let _ = write!(path, "_h{handle}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: write IntPacked into a bit buffer.
    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }

    #[test]
    fn decode_simple_struct_array() {
        // Build: elementCount=2, element[0] has handle=3 with 32 bits,
        //        element[1] has handle=5 with 8 bits, then terminators.
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 2); // elementCount
        // Element 0 (encodedIndex=1)
        write_int_packed(&mut bits, 1);
        // Field: encodedHandle=4 (handle=3), payloadBits=32
        write_int_packed(&mut bits, 4);
        write_int_packed(&mut bits, 32);
        bits.extend(std::iter::repeat_n(true, 32)); // payload
        write_int_packed(&mut bits, 0); // end of element 0
        // Element 1 (encodedIndex=2)
        write_int_packed(&mut bits, 2);
        // Field: encodedHandle=6 (handle=5), payloadBits=8
        write_int_packed(&mut bits, 6);
        write_int_packed(&mut bits, 8);
        bits.extend(std::iter::repeat_n(false, 8)); // payload
        write_int_packed(&mut bits, 0); // end of element 1
        // Array terminator
        write_int_packed(&mut bits, 0);

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.elements_decoded, 2);
        assert_eq!(stats.fields_emitted, 2);
        assert_eq!(stats.truncations, 0);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].path, "[0]._h3");
        assert_eq!(fields[0].handle, 3);
        assert_eq!(fields[0].bit_count, 32);
        assert_eq!(fields[1].path, "[1]._h5");
        assert_eq!(fields[1].handle, 5);
        assert_eq!(fields[1].bit_count, 8);
    }

    #[test]
    fn decode_nested_struct_array() {
        // Build: elementCount=1, element[0] has handle=4 (sub-array) with a
        // nested array of 1 element containing handle=7.
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // outer elementCount

        // Outer element 0
        write_int_packed(&mut bits, 1); // encodedIndex=1

        // Inner array field: handle=4, we need to build its payload separately
        let mut inner_bits = Vec::new();
        write_int_packed(&mut inner_bits, 1); // inner elementCount
        write_int_packed(&mut inner_bits, 1); // encodedIndex=1
        // Inner field: handle=7, 16 bits
        write_int_packed(&mut inner_bits, 8); // encodedHandle=8 -> handle=7
        write_int_packed(&mut inner_bits, 16);
        inner_bits.extend(std::iter::repeat_n(true, 16));
        write_int_packed(&mut inner_bits, 0); // end inner element
        write_int_packed(&mut inner_bits, 0); // inner array terminator

        let inner_payload_bits = inner_bits.len() as u32;
        // Outer field header: encodedHandle=5 (handle=4), payloadBits=inner_payload_bits
        write_int_packed(&mut bits, 5);
        write_int_packed(&mut bits, inner_payload_bits);
        bits.extend(inner_bits);

        write_int_packed(&mut bits, 0); // end outer element
        write_int_packed(&mut bits, 0); // outer array terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;

        // Schema: handle 4 at depth 0 is a sub-array with no further nesting.
        static INNER: ArrayFieldSchema = ArrayFieldSchema {
            sub_arrays: &[],
            field_names: &[],
        };
        static OUTER: ArrayFieldSchema = ArrayFieldSchema {
            sub_arrays: &[(4, &INNER)],
            field_names: &[(4, "Reports")],
        };

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);

        assert_eq!(stats.elements_decoded, 2); // 1 outer + 1 inner
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0]._h7");
        assert_eq!(fields[0].handle, 7);
        assert_eq!(fields[0].bit_count, 16);
    }

    /// One element carrying handles 3 (schema-named), 7 (schema-unnamed) and
    /// 40 (declared by neither).
    fn one_element_with_handles(handles: &[(u32, u32)]) -> (Vec<u8>, u32) {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // elementCount
        write_int_packed(&mut bits, 1); // encodedIndex=1 -> index 0
        for &(handle, payload) in handles {
            write_int_packed(&mut bits, handle + 1);
            write_int_packed(&mut bits, payload);
            bits.extend(std::iter::repeat_n(true, payload as usize));
        }
        write_int_packed(&mut bits, 0); // end of element
        write_int_packed(&mut bits, 0); // array terminator
        let bit_count = bits.len() as u32;
        (bits_to_bytes(&bits), bit_count)
    }

    /// The replay's declaration outranks the hardcoded schema on a leaf.
    ///
    /// Handle 3 is the live case: `COMBAT_ROUNDS_SCHEMA` calls it `RoundNumber`
    /// and every replay declares it `RoundNum`.
    #[test]
    fn a_declared_leaf_name_beats_the_schema_name() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32)]);
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[3] = Some("RoundNum");

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &data,
            bit_count,
            Some(&COMBAT_ROUNDS_SCHEMA),
            &declared,
            &mut stats,
        );

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].RoundNum");
    }

    /// A handle the schema cannot name is labelled by the replay, not `_hN`.
    #[test]
    fn a_declared_leaf_name_replaces_the_handle_placeholder() {
        let (data, bit_count) = one_element_with_handles(&[(7, 32)]);
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[7] = Some("StateRemainingTime");

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &data,
            bit_count,
            Some(&COMBAT_ROUNDS_SCHEMA),
            &declared,
            &mut stats,
        );

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].StateRemainingTime");
    }

    /// With no declaration the schema still names what it can, and an
    /// undeclared, unschematised handle still falls through to `_hN`.
    #[test]
    fn an_undeclared_leaf_falls_back_to_schema_then_placeholder() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32), (7, 32)]);

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &data,
            bit_count,
            Some(&COMBAT_ROUNDS_SCHEMA),
            &[],
            &mut stats,
        );

        assert_eq!(fields.len(), 2);
        // Schema names handle 3; nothing names handle 7.
        assert_eq!(fields[0].path, "[0].RoundNumber");
        assert_eq!(fields[1].path, "[0]._h7");
    }

    /// A declaration shorter than the handle it is asked about must not panic
    /// and must fall through, not silently mislabel.
    #[test]
    fn a_handle_past_the_end_of_the_declaration_falls_back() {
        let (data, bit_count) = one_element_with_handles(&[(7, 32)]);
        // Only handles 0..=3 declared; handle 7 is past the end.
        let declared: Vec<Option<&str>> = vec![None, None, None, Some("RoundNum")];

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &data,
            bit_count,
            Some(&COMBAT_ROUNDS_SCHEMA),
            &declared,
            &mut stats,
        );

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0]._h7");
    }

    /// Container segments keep the schema's name even when the replay declares
    /// a different one, because the schema is what decides the nesting.
    #[test]
    fn a_container_segment_keeps_its_schema_name() {
        // Outer element 0 carries handle 4 (Reports, a sub-array) whose single
        // element carries handle 5.
        let mut inner = Vec::new();
        write_int_packed(&mut inner, 1); // inner elementCount
        write_int_packed(&mut inner, 1); // encodedIndex=1
        write_int_packed(&mut inner, 6); // encodedHandle=6 -> handle 5
        write_int_packed(&mut inner, 32);
        inner.extend(std::iter::repeat_n(true, 32));
        write_int_packed(&mut inner, 0); // end inner element
        write_int_packed(&mut inner, 0); // inner terminator

        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // outer elementCount
        write_int_packed(&mut bits, 1); // encodedIndex=1
        write_int_packed(&mut bits, 5); // encodedHandle=5 -> handle 4
        write_int_packed(&mut bits, inner.len() as u32);
        bits.extend(inner);
        write_int_packed(&mut bits, 0); // end outer element
        write_int_packed(&mut bits, 0); // outer terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;

        // Declare a DIFFERENT name for the container handle 4, and the real
        // declared name for the leaf handle 5.
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[4] = Some("NotReports");
        declared[5] = Some("RoundNumber");

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &data,
            bit_count,
            Some(&COMBAT_ROUNDS_SCHEMA),
            &declared,
            &mut stats,
        );

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0].RoundNumber");
    }

    #[test]
    fn empty_array_emits_nothing() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 0); // elementCount = 0
        write_int_packed(&mut bits, 0); // immediate terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert!(fields.is_empty());
        assert_eq!(stats.elements_decoded, 0);
        // A legitimately empty array must read as zero errors so a malformed
        // run cannot hide behind the same number.
        assert_eq!(stats.errors, 0);
    }

    /// A payload that declares an element but EOFs mid-field must surface an
    /// error, not return an empty `Vec` indistinguishable from a clean empty
    /// array. This is the core silent-drop bug: the parent row's raw_bits are
    /// emitted by the caller regardless, but flattened leaves are lost and,
    /// without `errors`, the loss was invisible.
    #[test]
    fn truncated_payload_mid_element_counts_error() {
        // elementCount=2, element 0 starts, its first field declares 32 bits
        // of payload but only 8 remain -> overrun.
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 2); // elementCount = 2
        write_int_packed(&mut bits, 1); // encodedIndex = 1 -> element 0
        write_int_packed(&mut bits, 4); // encodedHandle = 4 -> handle 3
        write_int_packed(&mut bits, 32); // payloadBits = 32 (overruns)
        bits.extend(std::iter::repeat_n(true, 8)); // only 8 bits of payload

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert!(
            stats.errors >= 1,
            "truncated array must count errors, got {}",
            stats.errors
        );
        // No complete leaf was emitted; the caller still emits the parent row
        // from its own raw_bits, independent of this Vec.
        assert!(fields.is_empty());
    }

    /// A BitIo read failure (fewer than 8 bits left for an IntPacked read) must
    /// also count as an error rather than returning silently.
    #[test]
    fn read_failure_mid_stream_counts_error() {
        // elementCount=1, element 0 starts, its field header declares a handle,
        // then the stream ends with too few bits for the payloadBits IntPacked.
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // elementCount = 1
        write_int_packed(&mut bits, 1); // encodedIndex = 1 -> element 0
        write_int_packed(&mut bits, 4); // encodedHandle = 4 -> handle 3
        // Three stray bits: not enough for an IntPacked payloadBits read.
        bits.extend(std::iter::repeat_n(false, 3));

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let _fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert!(
            stats.errors >= 1,
            "mid-stream read failure must count errors, got {}",
            stats.errors
        );
    }
}
