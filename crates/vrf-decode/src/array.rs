//! RepLayout DynamicArray decoder — parses nested struct arrays from raw bits.
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
//!       skip remaining bits → break
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
//!           → recurse if this field is itself an array
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

use vrf_bitio::BitReader;

/// Maximum number of elements per array level.
pub const MAX_ELEMENTS: u32 = 4096;
/// Maximum number of fields per struct element.
pub const MAX_FIELDS_PER_ELEMENT: u32 = 128;
/// Maximum recursion depth for nested arrays.
pub const MAX_RECURSION_DEPTH: u32 = 12;

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
}

/// Decode a DynamicArray (struct elements) from raw bits and emit flattened fields.
///
/// `element_fields` is an optional lookup that maps `(depth, handle)` to a
/// sub-array indicator. For now we recurse on ALL fields that contain nested
/// handle/payloadBits framing (detected heuristically by checking if the
/// payload starts with a valid IntPacked sequence that looks like handles).
///
/// However, for correctness we use the **known schema**: the `ArrayFieldSchema`
/// tells us which handles at each depth are themselves arrays (so we recurse)
/// vs primitives (so we emit as-is).
///
/// If `schema` is None, we treat all elements as opaque structs and emit each
/// field without attempting to recurse into sub-arrays.
pub fn decode_struct_array(
    data: &[u8],
    bit_count: u32,
    schema: Option<&ArrayFieldSchema>,
    stats: &mut ArrayDecodeStats,
) -> Vec<FlattenedField> {
    let mut output = Vec::new();
    let mut reader = BitReader::with_bit_len(data, u64::from(bit_count));
    let mut path_buf = String::with_capacity(64);
    decode_array_level(&mut reader, schema, 0, &mut path_buf, &mut output, stats);
    output
}

/// Schema for array fields: maps handle → sub-array schema (if the field is
/// itself a nested array) or None (leaf/primitive field).
///
/// Built from the C# descriptor knowledge. The handle numbers are from the
/// `CombatRoundReportsDecoder` and related descriptors.
#[derive(Debug, Clone)]
pub struct ArrayFieldSchema {
    /// For each handle in this struct level, whether it's a sub-array.
    /// Key = handle, Value = schema for the sub-array's element struct.
    pub sub_arrays: &'static [(u32, &'static ArrayFieldSchema)],
    /// Optional handle → field name mapping for human-readable output.
    /// Only leaf fields need names here; sub-array fields get their name from
    /// the path segment they introduce.
    pub field_names: &'static [(u32, &'static str)],
}

/// Recursively decode one array level.
fn decode_array_level(
    reader: &mut BitReader<'_>,
    schema: Option<&ArrayFieldSchema>,
    depth: u32,
    path_buf: &mut String,
    output: &mut Vec<FlattenedField>,
    stats: &mut ArrayDecodeStats,
) {
    if reader.at_end() {
        return;
    }

    // Read element count.
    let element_count = match reader.read_int_packed() {
        Ok(c) => c,
        Err(_) => return,
    };

    if element_count > MAX_ELEMENTS {
        stats.truncations += 1;
        // Emit remaining as a single raw field at this level.
        emit_remaining_raw(reader, path_buf, output);
        return;
    }

    // Read elements.
    loop {
        if reader.at_end() {
            break;
        }

        let encoded_index = match reader.read_int_packed() {
            Ok(v) => v,
            Err(_) => break,
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
            // Index exceeds declared count — skip remaining.
            reader.skip_remaining();
            break;
        }

        stats.elements_decoded += 1;
        let path_prefix_len = path_buf.len();
        // Append "[index]" to path.
        use std::fmt::Write;
        let _ = write!(path_buf, "[{index}]");

        // Decode struct fields within this element.
        decode_struct_fields(reader, schema, depth, path_buf, output, stats);

        // Restore path buffer.
        path_buf.truncate(path_prefix_len);
    }
}

/// Decode the struct fields of one array element (handle/payloadBits loop).
fn decode_struct_fields(
    reader: &mut BitReader<'_>,
    schema: Option<&ArrayFieldSchema>,
    depth: u32,
    path_buf: &mut String,
    output: &mut Vec<FlattenedField>,
    stats: &mut ArrayDecodeStats,
) {
    for _field_idx in 0..MAX_FIELDS_PER_ELEMENT {
        if reader.at_end() {
            return;
        }

        let encoded_handle = match reader.read_int_packed() {
            Ok(v) => v,
            Err(_) => return,
        };

        if encoded_handle == 0 {
            return;
        }

        let handle = encoded_handle - 1;
        let payload_bits = match reader.read_int_packed() {
            Ok(v) => v,
            Err(_) => return,
        };

        if payload_bits == 0 {
            continue;
        }

        if (payload_bits as u64) > reader.bits_remaining() {
            // Malformed: skip remaining.
            reader.skip_remaining();
            return;
        }

        // Check if this handle is a sub-array.
        let sub_schema = schema.and_then(|s| {
            s.sub_arrays
                .iter()
                .find(|(h, _)| *h == handle)
                .map(|(_, sub)| *sub)
        });

        if let Some(sub) = sub_schema {
            if depth + 1 >= MAX_RECURSION_DEPTH {
                // Hit recursion limit — emit raw.
                stats.truncations += 1;
                let byte_count = (payload_bits as usize).div_ceil(8);
                let mut raw = vec![0u8; byte_count];
                let mut sub_reader = match reader.sub_reader(u64::from(payload_bits)) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                let _ = sub_reader.copy_bits_to(&mut raw, u64::from(payload_bits));
                let path_prefix_len = path_buf.len();
                use std::fmt::Write;
                let field_label = resolve_field_label(schema, handle);
                let _ = write!(path_buf, ".{field_label}");
                output.push(FlattenedField {
                    path: path_buf.clone(),
                    handle,
                    bit_count: payload_bits,
                    raw_bits: raw,
                });
                stats.fields_emitted += 1;
                path_buf.truncate(path_prefix_len);
            } else {
                // Recurse into sub-array.
                let mut sub_reader = match reader.sub_reader(u64::from(payload_bits)) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                let path_prefix_len = path_buf.len();
                use std::fmt::Write;
                let field_label = resolve_field_label(schema, handle);
                let _ = write!(path_buf, ".{field_label}");
                decode_array_level(
                    &mut sub_reader,
                    Some(sub),
                    depth + 1,
                    path_buf,
                    output,
                    stats,
                );
                // If sub_reader has remaining bits, skip them (tolerance).
                path_buf.truncate(path_prefix_len);
            }
        } else {
            // Leaf field — emit as-is.
            let byte_count = (payload_bits as usize).div_ceil(8);
            let mut raw = vec![0u8; byte_count];
            let mut sub_reader = match reader.sub_reader(u64::from(payload_bits)) {
                Ok(r) => r,
                Err(_) => return,
            };
            let _ = sub_reader.copy_bits_to(&mut raw, u64::from(payload_bits));

            let path_prefix_len = path_buf.len();
            use std::fmt::Write;
            let field_label = resolve_field_label(schema, handle);
            let _ = write!(path_buf, ".{field_label}");
            output.push(FlattenedField {
                path: path_buf.clone(),
                handle,
                bit_count: payload_bits,
                raw_bits: raw,
            });
            stats.fields_emitted += 1;
            path_buf.truncate(path_prefix_len);
        }
    }

    // Hit MAX_FIELDS_PER_ELEMENT — truncate.
    stats.truncations += 1;
}

/// Emit all remaining bits as a single raw field.
fn emit_remaining_raw(
    reader: &mut BitReader<'_>,
    path_buf: &str,
    output: &mut Vec<FlattenedField>,
) {
    let remaining = reader.bits_remaining();
    if remaining == 0 {
        return;
    }
    let byte_count = (remaining as usize).div_ceil(8);
    let mut raw = vec![0u8; byte_count];
    let _ = reader.copy_bits_to(&mut raw, remaining);
    output.push(FlattenedField {
        path: format!("{path_buf}._raw"),
        handle: u32::MAX,
        bit_count: remaining as u32,
        raw_bits: raw,
    });
}

/// Resolve a handle to a human-readable field label using the schema's name map.
/// Falls back to `_h{handle}` if no name is known.
fn resolve_field_label(schema: Option<&ArrayFieldSchema>, handle: u32) -> String {
    if let Some(s) = schema {
        // Check field_names first.
        if let Some((_, name)) = s.field_names.iter().find(|(h, _)| *h == handle) {
            return (*name).to_owned();
        }
        // Check sub_arrays (use handle-derived name for sub-arrays too).
        if let Some((_, _)) = s.sub_arrays.iter().find(|(h, _)| *h == handle) {
            // Sub-arrays don't have explicit names in field_names — use handle.
        }
    }
    format!("_h{handle}")
}

// ── CombatRoundReports schema ────────────────────────────────────────────────
//
// Derived from CombatRoundReports.cs (handles confirmed against the manifest):
//
// Rounds[] (top array)
//   handle 3: RoundNumber (Int32)
//   handle 4: Reports[] (sub-array)
//     handle 5: RoundNumber (Int32)
//     handle 10: Interactions[] (sub-array)
//       handle 11: Subject (FString)
//       handle 12: Team (FName)
//       handle 13: CharacterIcon (ObjectNetGuid)
//       handle 18: DamageDealt (Float)
//       handle 19: HitsDealt (Int32)
//       handle 20: DamageReceived (Float)
//       handle 21: HitsReceived (Int32)
//       handle 22: DidKill (Bool)
//       handle 23: AssistType (EnumByte)
//       handle 24: KillerPlayerState (ObjectNetGuid)
//       handle 25: WasKiller (Bool)
//       handle 26: DealtInteractions[] (sub-array)
//         handle 44: Regions[] (sub-array)
//           handle 45: Region (EnumByte)
//           handle 46: Hits (Int32)
//           handle 47: Damage (Float)
//           handle 48: IsWallPen (Bool)
//           handle 49: IsKill (Bool)
//           handle 50: DestroyedArmor (ObjectNetGuid)
//       handle 61: ReceivedInteractions[] (sub-array)
//         handle 79: Regions[] (sub-array)
//           handle 80: Region (EnumByte)
//           handle 81: Hits (Int32)
//           handle 82: Damage (Float)
//           handle 83: IsWallPen (Bool)
//           handle 84: IsKill (Bool)
//           handle 85: DestroyedArmor (ObjectNetGuid)
//       handle 96: CombatReportIndex (Int32)
//       handle 98: ResurrectorPlayerState (ObjectNetGuid)
//       handle 103: Died (Bool)

/// Regional damage interaction — leaf level (no sub-arrays).
static REGION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[],
    field_names: &[
        (45, "Region"),
        (46, "Hits"),
        (47, "Damage"),
        (48, "IsWallPen"),
        (49, "IsKill"),
        (50, "DestroyedArmor"),
        // ReceivedInteractions uses different handles for the same fields:
        (80, "Region"),
        (81, "Hits"),
        (82, "Damage"),
        (83, "IsWallPen"),
        (84, "IsKill"),
        (85, "DestroyedArmor"),
    ],
};

/// Dealt interaction regions: handle 44 → Regions sub-array.
static DEALT_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(44, &REGION_SCHEMA)],
    field_names: &[(44, "Regions")],
};

/// Received interaction regions: handle 79 → Regions sub-array.
static RECEIVED_INTERACTION_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(79, &REGION_SCHEMA)],
    field_names: &[(79, "Regions")],
};

/// Participant interaction: handles 26, 61 are sub-arrays.
static PARTICIPANT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[
        (26, &DEALT_INTERACTION_SCHEMA),
        (61, &RECEIVED_INTERACTION_SCHEMA),
    ],
    field_names: &[
        (11, "Subject"),
        (12, "Team"),
        (13, "CharacterIcon"),
        (18, "DamageDealt"),
        (19, "HitsDealt"),
        (20, "DamageReceived"),
        (21, "HitsReceived"),
        (22, "DidKill"),
        (23, "AssistType"),
        (24, "KillerPlayerState"),
        (25, "WasKiller"),
        (26, "DealtInteractions"),
        (61, "ReceivedInteractions"),
        (96, "CombatReportIndex"),
        (98, "ResurrectorPlayerState"),
        (103, "Died"),
    ],
};

/// Character combat report: handle 10 → Interactions sub-array.
static CHARACTER_REPORT_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(10, &PARTICIPANT_SCHEMA)],
    field_names: &[
        (5, "RoundNumber"),
        (10, "Interactions"),
        (98, "ResurrectorPlayerState"),
        (103, "Died"),
    ],
};

/// Round-level schema: handle 4 → Reports sub-array.
pub static COMBAT_ROUNDS_SCHEMA: ArrayFieldSchema = ArrayFieldSchema {
    sub_arrays: &[(4, &CHARACTER_REPORT_SCHEMA)],
    field_names: &[(3, "RoundNumber"), (4, "Reports")],
};

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
        let fields = decode_struct_array(&data, bit_count, None, &mut stats);

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
        write_int_packed(&mut inner_bits, 8); // encodedHandle=8 → handle=7
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
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &mut stats);

        assert_eq!(stats.elements_decoded, 2); // 1 outer + 1 inner
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0]._h7");
        assert_eq!(fields[0].handle, 7);
        assert_eq!(fields[0].bit_count, 16);
    }

    #[test]
    fn empty_array_emits_nothing() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 0); // elementCount = 0
        write_int_packed(&mut bits, 0); // immediate terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &mut stats);

        assert!(fields.is_empty());
        assert_eq!(stats.elements_decoded, 0);
    }
}
