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

pub use schema::{
    ABILITY_CASTS_SCHEMA, ABILITY_EFFECTS_SCHEMA, ArrayFieldSchema, COMBAT_ROUNDS_SCHEMA,
    LIFE_CHANGE_BY_SECTION_SCHEMA, LIFE_CHANGE_DAMAGE_SCHEMA, LIFE_CHANGE_SECTION_SCHEMA,
};

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
    /// Bits left inside a nested array's window after that array stopped
    /// decoding.
    ///
    /// The parent carves a sub-reader of the declared `payloadBits` and
    /// advances past the whole window, so the parent stays aligned whatever the
    /// child did with it -- and the leftover used to be dropped on exactly that
    /// reasoning. But parent ALIGNMENT and child COMPLETENESS are two claims,
    /// and the sub-reader establishes only the first. A nested array that
    /// stopped early still hands its parent a correctly positioned reader, so
    /// the walk continues, no counter moves, and the leaves inside those bits
    /// are gone with nothing saying so.
    ///
    /// A tally rather than an error, deliberately: this is the call
    /// [`Self::truncations`] already makes, and the bits themselves survive in
    /// the parent row's `raw_bits`. Zero on a corpus that decodes cleanly,
    /// which is what makes a non-zero value worth looking at.
    pub unconsumed_nested_bits: u64,
    /// Bits left after the root array's explicit terminator or an early stop.
    ///
    /// The caller retains the whole parent payload, so these bits remain
    /// recoverable. This counter distinguishes that raw fallback from a fully
    /// walked root array; the nested equivalent is
    /// [`Self::unconsumed_nested_bits`].
    pub unconsumed_root_bits: u64,
    /// Times an element's field loop or an array level ended because the reader
    /// ran out, rather than because it read an explicit `0` terminator.
    ///
    /// The framing ends an element with a zero handle and an array with a zero
    /// index. Accepting EOF in their place makes a payload truncated mid-stream
    /// decode to the same shape a complete one does -- the last element simply
    /// looks finished. Nothing else in the walk distinguishes them, so this
    /// counter is the only signal.
    ///
    /// Not an error: this walker is also handed windows whose final terminator
    /// is absorbed by byte padding, so raising it would turn well-formed data
    /// loud.
    pub implicit_terminations: u64,
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
    let Ok(mut reader) = BitReader::with_bit_len(data, u64::from(bit_count)) else {
        stats.errors += 1;
        return Vec::new();
    };
    let mut walk = Walk {
        declared,
        path: String::with_capacity(64),
        output: Vec::with_capacity(INITIAL_FIELD_CAPACITY),
    };
    decode_array_level(&mut reader, &mut walk, schema, 0, stats);
    stats.unconsumed_root_bits += reader.bits_remaining();
    walk.output
}

/// Decode a RepLayout dynamic array of object references (`TArray<UObject*>`)
/// into the actor NetGUIDs it carries.
///
/// `MultiItemSlot.MultiContents` is this shape: a dynamic array whose every
/// element is a single object-reference property. The framing is the same one
/// [`decode_struct_array`] walks for struct arrays -- an `elementCount`, then a
/// run of `(encodedIndex, handle, payloadBits, payload, element-terminator)`
/// tuples closed by an index terminator -- confirmed on 245/245 wire payloads,
/// where each element carries exactly one field at handle 2 whose payload is
/// the item actor's NetGUID as one IntPacked value. The C# reference types the
/// property as `TArray<AAresItem*>`.
///
/// Returns one NetGUID per populated element, in the order the stream sends
/// them (element-index order on every payload seen). Malformed input returns
/// whatever was decoded so far: the caller still emits the parent row from its
/// own `raw_bits`, so a short `Vec` costs the typed leaves, not the bits.
pub fn decode_object_ref_array(data: &[u8], bit_count: u32) -> Vec<u32> {
    let mut ignored = ArrayDecodeStats::default();
    decode_object_ref_array_with_stats(data, bit_count, &mut ignored)
}

/// Decode an object-reference dynamic array while exposing every partial or
/// malformed path through `stats`.
///
/// This is the production entry point for `MultiContents`: the parent raw row
/// is preserved by the caller, while these diagnostics state whether all typed
/// item rows were recovered. [`decode_object_ref_array`] remains as the
/// compatibility wrapper for callers that do not need diagnostics.
pub fn decode_object_ref_array_with_stats(
    data: &[u8],
    bit_count: u32,
    stats: &mut ArrayDecodeStats,
) -> Vec<u32> {
    let Ok(mut reader) = BitReader::with_bit_len(data, u64::from(bit_count)) else {
        stats.errors += 1;
        return Vec::new();
    };
    let out = decode_object_ref_array_reader(&mut reader, stats);
    stats.unconsumed_root_bits += reader.bits_remaining();
    out
}

fn decode_object_ref_array_reader(
    reader: &mut BitReader<'_>,
    stats: &mut ArrayDecodeStats,
) -> Vec<u32> {
    let mut out = Vec::new();

    let Ok(element_count) = reader.read_int_packed() else {
        stats.errors += 1;
        return out;
    };
    if element_count > MAX_ELEMENTS {
        stats.truncations += 1;
        return out;
    }

    let mut elements_seen = 0u32;
    while !reader.at_end() {
        if elements_seen == MAX_ELEMENTS {
            let mut probe = reader.clone();
            match probe.read_int_packed() {
                Ok(0) => {
                    *reader = probe;
                    consume_optional_trailing_int_packed(reader, stats);
                }
                Ok(_) => stats.truncations += 1,
                Err(_) => stats.errors += 1,
            }
            return out;
        }
        let Ok(encoded_index) = reader.read_int_packed() else {
            stats.errors += 1;
            break;
        };
        if encoded_index == 0 {
            consume_optional_trailing_int_packed(reader, stats);
            break;
        }
        let index = encoded_index - 1;
        if index >= element_count {
            stats.errors += 1;
            reader.skip_remaining();
            break;
        }
        elements_seen += 1;
        stats.elements_decoded += 1;

        // Each element carries one object-reference field. Walk its handle loop
        // -- real payloads run it once -- and decode the first payload as the
        // item's IntPacked NetGUID. The bounded loop keeps a multi-field element
        // (none seen on the wire, but the framing permits it) from
        // desynchronising the rest of the array.
        let mut guid = None;
        let mut element_complete = false;
        for field_idx in 0..=MAX_FIELDS_PER_ELEMENT {
            if reader.at_end() {
                stats.implicit_terminations += 1;
                break;
            }
            if field_idx == MAX_FIELDS_PER_ELEMENT {
                let mut probe = reader.clone();
                match probe.read_int_packed() {
                    Ok(0) => {
                        *reader = probe;
                        element_complete = true;
                    }
                    Ok(_) => stats.truncations += 1,
                    Err(_) => stats.errors += 1,
                }
                break;
            }
            let Ok(encoded_handle) = reader.read_int_packed() else {
                stats.errors += 1;
                break;
            };
            if encoded_handle == 0 {
                element_complete = true;
                break;
            }
            let Ok(payload_bits) = reader.read_int_packed() else {
                stats.errors += 1;
                break;
            };
            if payload_bits == 0 {
                continue;
            }
            if u64::from(payload_bits) > reader.bits_remaining() {
                stats.errors += 1;
                reader.skip_remaining();
                break;
            }
            // `sub_reader` carves out the field's declared window and advances
            // the parent past it, so a NetGUID that spends fewer bits than the
            // window still leaves the reader aligned on the next handle.
            let Ok(mut sub) = reader.sub_reader(u64::from(payload_bits)) else {
                stats.errors += 1;
                break;
            };
            if guid.is_none() {
                match sub.read_int_packed() {
                    Ok(v) if sub.at_end() => guid = Some(v),
                    Ok(_) | Err(_) => stats.errors += 1,
                }
            }
        }

        if let Some(g) = guid {
            out.push(g);
            stats.fields_emitted += 1;
        }
        if !element_complete {
            reader.skip_remaining();
            break;
        }
    }

    out
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

    // Read elements. An explicit `0` index is what ends the array; running out
    // of bits instead is accepted (a padded window legitimately does that) but
    // tallied, because it is also what a truncated payload looks like.
    let mut elements_seen = 0u32;
    loop {
        if reader.at_end() {
            stats.implicit_terminations += 1;
            break;
        }
        if elements_seen == MAX_ELEMENTS {
            let mut probe = reader.clone();
            match probe.read_int_packed() {
                Ok(0) => {
                    *reader = probe;
                    consume_optional_trailing_int_packed(reader, stats);
                }
                Ok(_) => {
                    stats.truncations += 1;
                    emit_remaining_raw(reader, walk);
                }
                Err(_) => stats.errors += 1,
            }
            break;
        }
        let Ok(encoded_index) = reader.read_int_packed() else {
            stats.errors += 1;
            break;
        };

        if encoded_index == 0 {
            consume_optional_trailing_int_packed(reader, stats);
            break;
        }

        let index = encoded_index - 1;
        if index >= element_count {
            stats.errors += 1;
            reader.skip_remaining();
            break;
        }

        elements_seen += 1;
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
    for field_idx in 0..=MAX_FIELDS_PER_ELEMENT {
        if reader.at_end() {
            // An element ends on a zero handle. Reaching EOF instead means the
            // element was never closed -- harmless for alignment, since there
            // is nothing after it, but indistinguishable from a complete
            // element without this tally.
            stats.implicit_terminations += 1;
            return;
        }

        if field_idx == MAX_FIELDS_PER_ELEMENT {
            let mut probe = reader.clone();
            match probe.read_int_packed() {
                Ok(0) => *reader = probe,
                Ok(_) => {
                    stats.truncations += 1;
                    emit_remaining_raw(reader, walk);
                }
                Err(_) => stats.errors += 1,
            }
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
                    // Unreachable: the width was bounds-checked just above.
                    // Counted anyway, so that "unreachable" stays a claim the
                    // stats can contradict rather than an assumption.
                    stats.errors += 1;
                    return;
                };
                let prefix_len = walk.path.len();
                walk.path.push('.');
                push_field_label(&mut walk.path, schema, handle);
                decode_array_level(&mut sub_reader, walk, Some(sub), depth + 1, stats);
                // The parent's window already advanced past all of these bits,
                // so dropping them keeps the parent ALIGNED -- but alignment is
                // not the same claim as the child having CONSUMED them, and
                // only the first was ever established here. Leaves inside an
                // abandoned tail are lost silently otherwise.
                stats.unconsumed_nested_bits += sub_reader.bits_remaining();
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
}

/// Consume the format's optional one-IntPacked trailer without hiding a
/// continuation byte that runs past the declared array window.
fn consume_optional_trailing_int_packed(reader: &mut BitReader<'_>, stats: &mut ArrayDecodeStats) {
    if reader.bits_remaining() == 8 && reader.read_int_packed().is_err() {
        stats.errors += 1;
    }
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
///
/// # `declared` applies at every depth, on purpose
///
/// It looks wrong that ONE root-group table is consulted for a leaf at any
/// nesting level, as if a nested element had its own handle space that could
/// collide with the root's. It does not. Unreal flattens a `TArray` of structs
/// onto CONSECUTIVE handles of the ENCLOSING group, so one flat handle space
/// spans the whole tree and the group's own net field exports name every leaf
/// in it. That is why `COMBAT_ROUNDS_SCHEMA`'s handles are disjoint by depth
/// (3-4, then 5/10, then 11-26, then 44-50, then 79-85) rather than restarting
/// per level -- they are all handles of one group.
///
/// The typo example above is itself the proof: `DamageRecieved` and
/// `HitsRecieved` are handles 20 and 21 of `PARTICIPANT_SCHEMA`, which sits at
/// **depth 2** (`Rounds` -> `Reports` -> `Interactions`). So declaration-beats-
/// schema was always a statement about nested leaves, not only root ones, and
/// restricting it to depth 0 would rename those two exported columns --
/// `tools/to_valplay_bundle.py` maps handle 20 from the wire's `DamageRecieved`
/// and its test pins the path `Rounds[0].Reports[0].Interactions[0].DamageRecieved`.
///
/// The schemas that DO use a private, restarting handle space --
/// `LIFE_CHANGE_DAMAGE_SCHEMA` (10-13), `LIFE_CHANGE_SECTION_SCHEMA` (1-4) and
/// `LIFE_CHANGE_BY_SECTION_SCHEMA` (2-5), whose numbering is per RPC parameter
/// rather than per group -- cannot reach this code with a populated table:
/// each declares `sub_arrays: &[]`, so no recursion happens, and the export
/// path passes `declared` as `&[]` for them.
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

    /// Build the wire shape of `MultiItemSlot.MultiContents` and check the
    /// decoded NetGUIDs. The payload of every element is a single object
    /// reference at handle 2; IntPacked 812 occupies a 16-bit window, 25492 a
    /// 24-bit one, mirroring the two payload widths seen on real replays.
    #[test]
    fn decode_object_ref_array_extracts_item_netguids() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 2); // elementCount
        // Element 0: NetGUID 812 in a 16-bit payload window.
        write_int_packed(&mut bits, 1); // encodedIndex -> index 0
        write_int_packed(&mut bits, 3); // encodedHandle -> handle 2
        write_int_packed(&mut bits, 16); // payloadBits
        write_int_packed(&mut bits, 812); // ObjectNetGuid payload
        write_int_packed(&mut bits, 0); // element terminator
        // Element 1: NetGUID 25492 in a 24-bit payload window.
        write_int_packed(&mut bits, 2); // encodedIndex -> index 1
        write_int_packed(&mut bits, 3); // handle 2
        write_int_packed(&mut bits, 24); // payloadBits
        write_int_packed(&mut bits, 25492); // ObjectNetGuid payload
        write_int_packed(&mut bits, 0); // element terminator
        write_int_packed(&mut bits, 0); // array terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let guids = decode_object_ref_array(&data, bit_count);

        assert_eq!(guids, vec![812, 25492]);
    }

    /// An empty array (elementCount = 0, immediate terminator) decodes to nothing.
    #[test]
    fn decode_object_ref_array_empty() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 0);
        write_int_packed(&mut bits, 0);

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let guids = decode_object_ref_array(&data, bit_count);

        assert!(guids.is_empty());
    }

    #[test]
    fn malformed_object_ref_array_exposes_its_failure_stats() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 3);
        write_int_packed(&mut bits, 32); // only eight payload bits follow
        bits.extend(std::iter::repeat_n(false, 8));
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bits.len() as u32, &mut stats);

        assert!(guids.is_empty());
        assert_eq!(stats.errors, 1, "{stats:?}");
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

    /// A nested array that leaves bits inside its own window must say so.
    ///
    /// The parent's `sub_reader` advances past the whole declared window, so
    /// the walk stays aligned and every later element still decodes -- which is
    /// exactly why this was invisible. Alignment is not completeness: the
    /// leaves inside the abandoned bits are lost either way, and without a
    /// tally nothing distinguishes this from a nested array that consumed its
    /// window exactly.
    #[test]
    fn a_nested_array_that_leaves_bits_reports_them() {
        let mut inner = Vec::new();
        write_int_packed(&mut inner, 1); // inner elementCount
        write_int_packed(&mut inner, 1); // encodedIndex=1
        write_int_packed(&mut inner, 8); // encodedHandle=8 -> handle 7
        write_int_packed(&mut inner, 16);
        inner.extend(std::iter::repeat_n(true, 16));
        write_int_packed(&mut inner, 0); // end inner element
        write_int_packed(&mut inner, 0); // inner array terminator
        // Sixteen bits the inner array will never look at. Not 8: an 8-bit
        // tail is the trailing terminator the framing legitimately consumes.
        let declared_inner_bits = inner.len() as u32 + 16;

        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // outer elementCount
        write_int_packed(&mut bits, 1); // encodedIndex=1
        write_int_packed(&mut bits, 5); // encodedHandle=5 -> handle 4
        write_int_packed(&mut bits, declared_inner_bits);
        bits.extend(inner);
        bits.extend(std::iter::repeat_n(true, 16)); // the abandoned tail
        write_int_packed(&mut bits, 0); // end outer element
        write_int_packed(&mut bits, 0); // outer array terminator

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;

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

        // The walk still succeeds and stays aligned -- that is the point.
        assert_eq!(fields.len(), 1, "{fields:?}");
        assert_eq!(fields[0].path, "[0].Reports[0]._h7");
        assert_eq!(stats.errors, 0, "alignment held, so this is not an error");
        assert_eq!(
            stats.unconsumed_nested_bits, 16,
            "the 16 abandoned bits must be tallied"
        );
    }

    /// An element that runs out of bits instead of reading its zero handle is
    /// a truncated element, and must be distinguishable from a complete one.
    #[test]
    fn an_element_ending_at_eof_is_not_a_clean_terminator() {
        // elementCount=1, element 0, one complete 32-bit field, then nothing:
        // no element terminator and no array terminator.
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 4); // handle 3
        write_int_packed(&mut bits, 32);
        bits.extend(std::iter::repeat_n(true, 32));

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        // The field itself is complete and is still emitted.
        assert_eq!(fields.len(), 1);
        assert!(
            stats.implicit_terminations >= 1,
            "EOF stood in for a terminator and nothing counted it: {stats:?}"
        );
    }

    /// The counter must NOT fire on a well-formed array that closes with both
    /// its terminators, or it would flag every clean blob in the corpus.
    #[test]
    fn a_cleanly_terminated_array_reports_no_implicit_termination() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32)]);
        let mut stats = ArrayDecodeStats::default();
        let _ = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    #[test]
    fn bits_after_the_root_array_terminator_are_tallied() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 0);
        write_int_packed(&mut bits, 0);
        bits.extend(std::iter::repeat_n(true, 16));
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bits.len() as u32, None, &[], &mut stats);

        assert_eq!(stats.unconsumed_root_bits, 16, "{stats:?}");
    }

    #[test]
    fn an_out_of_range_element_index_is_malformed_not_a_clean_empty_array() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // one declared element
        write_int_packed(&mut bits, 2); // index 1 is outside [0, 1)
        bits.extend(std::iter::repeat_n(true, 16));
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let fields = decode_struct_array(&data, bits.len() as u32, None, &[], &mut stats);

        assert!(fields.is_empty());
        assert_eq!(stats.errors, 1, "{stats:?}");
    }

    #[test]
    fn a_truncated_trailing_int_packed_is_not_accepted() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 0); // zero elements
        write_int_packed(&mut bits, 0); // array terminator
        // Exactly eight trailing bits activates the optional IntPacked read,
        // but the continuation flag asks for a byte that is not present.
        for bit in 0..8 {
            bits.push((0x01 & (1 << bit)) != 0);
        }
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bits.len() as u32, None, &[], &mut stats);

        assert_eq!(stats.errors, 1, "{stats:?}");
    }

    #[test]
    fn exactly_max_fields_followed_by_a_terminator_is_not_truncated() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 1);
        for _ in 0..MAX_FIELDS_PER_ELEMENT {
            write_int_packed(&mut bits, 1); // handle 0
            write_int_packed(&mut bits, 0); // valid empty payload
        }
        write_int_packed(&mut bits, 0); // element terminator
        write_int_packed(&mut bits, 0); // array terminator
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bits.len() as u32, None, &[], &mut stats);

        assert_eq!(stats.truncations, 0, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    #[test]
    fn repeated_element_indices_cannot_bypass_the_element_work_limit() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 1); // one declared slot
        for _ in 0..=MAX_ELEMENTS {
            write_int_packed(&mut bits, 1); // repeat index 0
            write_int_packed(&mut bits, 0); // empty element
        }
        write_int_packed(&mut bits, 0); // array terminator
        let data = bits_to_bytes(&bits);
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bits.len() as u32, None, &[], &mut stats);

        assert_eq!(stats.elements_decoded, u64::from(MAX_ELEMENTS));
        assert_eq!(stats.truncations, 1, "{stats:?}");
    }

    /// `AbilityCastsThisRound[].Effects[]` is a nested array, and until it had
    /// a schema the walker could not know that: a sub-array and an opaque leaf
    /// both arrive as `handle + payloadBits + bits`, so `Effects` was emitted
    /// whole and the per-cast statistics inside it stayed raw.
    ///
    /// The payload below is a real one off the wire, the smallest that carries
    /// an `AffectedTargetsArray`: 128 bits holding one effect element whose
    /// only field is handle 18, itself an 80-bit array.
    #[test]
    fn the_ability_effects_array_descends_into_its_targets() {
        let raw = [
            0x02, 0x02, 0x26, 0xa0, 0x04, 0x04, 0x2a, 0x40, 0x7e, 0x16, 0x12, 0x3f, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut stats = ArrayDecodeStats::default();
        let out = decode_struct_array(&raw, 128, Some(&ABILITY_EFFECTS_SCHEMA), &[], &mut stats);
        assert_eq!(stats.errors, 0, "{stats:?}");

        // Without the schema this is one opaque leaf at handle 18. With it, the
        // walker descends and the target's own members come out with an index.
        //
        // Only `Value` appears here, not `AffectedPlayer`: replication is per
        // property, so an element re-sent because one member changed carries
        // only that member. Reconstructing a target means carrying the last
        // seen value forward per (element index), the same as any delta stream.
        let paths: Vec<&str> = out.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["[0].AffectedTargetsArray[1].Value"], "{paths:?}");
    }

    /// What the two new tallies read on a REAL wire payload, pinned so that a
    /// later change to either counter has to face real data and not only
    /// hand-built fixtures.
    ///
    /// This is the same 128-bit `AbilityCastsThisRound[].Effects[]` blob the
    /// test above walks. Its nested `AffectedTargetsArray` consumes its window
    /// exactly, and the outer level closes on its own terminator with only byte
    /// padding after it -- so both counters are zero, which is the evidence
    /// that neither fires on well-formed data.
    #[test]
    fn the_real_effects_payload_leaves_no_unconsumed_bits() {
        let raw = [
            0x02, 0x02, 0x26, 0xa0, 0x04, 0x04, 0x2a, 0x40, 0x7e, 0x16, 0x12, 0x3f, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut stats = ArrayDecodeStats::default();
        let _ = decode_struct_array(&raw, 128, Some(&ABILITY_EFFECTS_SCHEMA), &[], &mut stats);

        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");
    }

    /// The nesting the schema declares, checked against the replay's own
    /// declaration rather than against itself: `Effects` is handle 13 on a cast,
    /// its members are 14..18, and `AffectedTargetsArray` (18) holds 19 and 20.
    #[test]
    fn the_ability_effects_schema_matches_the_declared_handles() {
        assert!(ABILITY_CASTS_SCHEMA.sub_array(13).is_some(), "Effects");
        let effects = ABILITY_CASTS_SCHEMA.sub_array(13).unwrap();
        for (handle, name) in [
            (14, "Statistic"),
            (15, "LocalizedStat"),
            (16, "Value"),
            (17, "Time"),
            (18, "AffectedTargetsArray"),
        ] {
            assert_eq!(effects.field_name(handle), Some(name), "handle {handle}");
        }
        let targets = effects.sub_array(18).expect("AffectedTargetsArray");
        assert_eq!(targets.field_name(19), Some("AffectedPlayer"));
        assert_eq!(targets.field_name(20), Some("Value"));
        assert!(targets.sub_array(19).is_none(), "leaves stay leaves");
    }
}
