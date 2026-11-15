//! RepLayout DynamicArray decoder -- parses nested struct arrays from raw bits.
//!
//! ```text
//! elementCount = IntPacked
//! loop: encodedIndex = IntPacked       // 0 ends the array; index = encoded - 1
//!   loop: encodedHandle = IntPacked    // 0 ends the element; handle = encoded - 1
//!         payloadBits = IntPacked, then that many bits (a nested array when
//!         the schema says so)
//! ```
//!
//! Limits: [`MAX_ELEMENTS`] 4096 (real data peaks ~50), [`MAX_FIELDS_PER_ELEMENT`]
//! 128 and [`MAX_RECURSION_DEPTH`] 12 (real data is 4-5 deep). A limit hit keeps
//! the remaining bits and counts a truncation; nothing panics.
//!
//! One [`FlattenedField`] per leaf, with a path such as
//! `Rounds[3].Reports[1].DamageDealt`, or `Rounds[0]._h18` where no name is known.

mod schema;

use std::fmt::Write as _;

use vrf_bitio::BitReader;

pub use schema::{
    ABILITY_CASTS_SCHEMA, ABILITY_EFFECTS_SCHEMA, ArrayFieldSchema, COMBAT_ROUNDS_SCHEMA,
    COMBAT_ROUNDS_SCHEMA_1310, LIFE_CHANGE_BY_SECTION_SCHEMA, LIFE_CHANGE_DAMAGE_SCHEMA,
    LIFE_CHANGE_DAMAGE_SCHEMA_1310, LIFE_CHANGE_SECTION_SCHEMA,
};

/// Maximum number of elements per array level.
pub const MAX_ELEMENTS: u32 = 4096;
/// Maximum number of fields per struct element.
pub const MAX_FIELDS_PER_ELEMENT: u32 = 128;
/// Maximum recursion depth for nested arrays.
pub const MAX_RECURSION_DEPTH: u32 = 12;

/// A single flattened field emitted from array decoding.
#[derive(Debug, Clone)]
pub struct FlattenedField {
    /// Dotted path from the array root, e.g. `"[2].Reports[1].DamageDealt"`; the
    /// caller prepends the parent field name.
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
    /// Read failures or declared-vs-available overruns; the walk abandons the
    /// rest of the stream, so this is the only sign leaves were lost.
    pub errors: u64,
    /// Declared bits inside a nested window the walk stepped past: what a nested
    /// array left in its window, and every object-reference field after an
    /// element's first. A tally, not an error.
    pub unconsumed_nested_bits: u64,
    /// Bits left in the root window after the index terminator (a nonzero
    /// trailer included). A stop that moves `errors` or `truncations` consumes
    /// the rest of the stream, so it is not counted again here.
    pub unconsumed_root_bits: u64,
    /// Times an element or array level ended at EOF instead of its explicit `0`
    /// terminator: the only sign a payload cut short is not a complete one.
    pub implicit_terminations: u64,
}

impl ArrayDecodeStats {
    /// Add every counter of `other` into `self`. No `..` in the destructure, so
    /// a new field does not compile until it is summed here.
    pub fn merge_from(&mut self, other: &Self) {
        let Self {
            elements_decoded,
            fields_emitted,
            truncations,
            errors,
            unconsumed_nested_bits,
            unconsumed_root_bits,
            implicit_terminations,
        } = other;
        self.elements_decoded += elements_decoded;
        self.fields_emitted += fields_emitted;
        self.truncations += truncations;
        self.errors += errors;
        self.unconsumed_nested_bits += unconsumed_nested_bits;
        self.unconsumed_root_bits += unconsumed_root_bits;
        self.implicit_terminations += implicit_terminations;
    }

    /// Whether a walk recorded none of the five failure shapes: measured routes
    /// emit typed children only from a clean walk. No `..` in the destructure,
    /// so a new field must be classified as work (`_`) or failure first.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        let Self {
            elements_decoded: _,
            fields_emitted: _,
            truncations,
            errors,
            unconsumed_nested_bits,
            unconsumed_root_bits,
            implicit_terminations,
        } = self;
        *truncations == 0
            && *errors == 0
            && *unconsumed_nested_bits == 0
            && *unconsumed_root_bits == 0
            && *implicit_terminations == 0
    }
}

/// Decode a DynamicArray of structs from raw bits into flattened leaves.
///
/// `schema` says which handles at each depth are themselves arrays; `None`
/// emits every field as a leaf. `declared[h]` is the name the REPLAY declares
/// for handle `h` (`&[]` when there is none). A leaf is labelled declaration ->
/// schema -> `_h{handle}`, a container by the schema alone.
pub fn decode_struct_array(
    data: &[u8],
    bit_count: u32,
    schema: Option<&ArrayFieldSchema>,
    declared: &[Option<&str>],
    stats: &mut ArrayDecodeStats,
) -> Vec<FlattenedField> {
    decode_struct_array_window(data, bit_count, schema, declared, stats, true)
}

/// Walk a flat struct array without accepting the optional trailer. Remaining
/// bits are reported in `unconsumed_root_bits`; callers must check every
/// counter before accepting the leaves.
pub fn decode_struct_array_exact(
    data: &[u8],
    bit_count: u32,
    declared: &[Option<&str>],
    stats: &mut ArrayDecodeStats,
) -> Vec<FlattenedField> {
    decode_struct_array_window(data, bit_count, None, declared, stats, false)
}

fn decode_struct_array_window(
    data: &[u8],
    bit_count: u32,
    schema: Option<&ArrayFieldSchema>,
    declared: &[Option<&str>],
    stats: &mut ArrayDecodeStats,
    trailer: bool,
) -> Vec<FlattenedField> {
    let mut sink = StructSink {
        trailer,
        declared,
        schema,
        depth: 0,
        path: String::with_capacity(64),
        output: Vec::with_capacity(32),
    };
    // An empty window is no array and no anomaly.
    if bit_count != 0 {
        walk_root(data, bit_count, &mut sink, stats);
    }
    sink.output
}

/// Decode a RepLayout dynamic array of object references (`TArray<UObject*>`)
/// into `(wire index, NetGUID)` pairs, without diagnostics.
///
/// `MultiItemSlot.MultiContents` (`TArray<AAresItem*>`) is this shape: one
/// field per element holding the item actor's NetGUID as one IntPacked.
/// Malformed input returns what was decoded so far.
pub fn decode_object_ref_array(data: &[u8], bit_count: u32) -> Vec<(u32, u32)> {
    decode_object_ref_array_with_stats(data, bit_count, &mut ArrayDecodeStats::default())
}

/// [`decode_object_ref_array`] with every partial or malformed path counted in
/// `stats`. Pairs, not a dense `Vec`: the array is delta-replicated per element,
/// so the wire index is the only thing that says which slot a GUID belongs in.
///
/// An element's first populated field is its NetGUID, whether or not it
/// decodes; later fields go to `unconsumed_nested_bits` (none in 273,386 real
/// elements), and a zero-width field is skipped untallied.
pub fn decode_object_ref_array_with_stats(
    data: &[u8],
    bit_count: u32,
    stats: &mut ArrayDecodeStats,
) -> Vec<(u32, u32)> {
    let mut sink = ObjectSink {
        item: None,
        output: Vec::new(),
    };
    walk_root(data, bit_count, &mut sink, stats);
    sink.output
}

/// What each walker does where the two differ; the framing loops are shared.
trait Sink {
    /// Whether an index terminator may carry the optional trailer.
    fn trailer(&self) -> bool;
    /// The rest of the stream after a limit hit: kept or skipped.
    fn keep_tail(&mut self, reader: &mut BitReader<'_>, stats: &mut ArrayDecodeStats);
    /// One element, around `walk_fields`; returns whether it closed.
    fn element(
        &mut self,
        index: u32,
        reader: &mut BitReader<'_>,
        stats: &mut ArrayDecodeStats,
    ) -> bool;
    /// One present field, `sub` being exactly its possibly empty payload window.
    fn field(&mut self, handle: u32, sub: BitReader<'_>, stats: &mut ArrayDecodeStats);
}

fn walk_root(data: &[u8], bit_count: u32, sink: &mut impl Sink, stats: &mut ArrayDecodeStats) {
    let Ok(mut reader) = BitReader::with_bit_len(data, u64::from(bit_count)) else {
        stats.errors += 1;
        return;
    };
    walk_array(&mut reader, sink, stats);
    stats.unconsumed_root_bits += reader.bits_remaining();
}

/// One array level. Every stop that moves `errors` or `truncations` consumes
/// the rest of the stream (skipped, or kept by `keep_tail`), so the caller's
/// residual tally never counts the same stop twice.
fn walk_array(reader: &mut BitReader<'_>, sink: &mut impl Sink, stats: &mut ArrayDecodeStats) {
    let Ok(element_count) = reader.read_int_packed() else {
        stats.errors += 1;
        reader.skip_remaining();
        return;
    };
    if element_count > MAX_ELEMENTS {
        stats.truncations += 1;
        sink.keep_tail(reader, stats);
        return;
    }
    for elements_seen in 0..=MAX_ELEMENTS {
        if reader.at_end() {
            stats.implicit_terminations += 1;
            return;
        }
        // At the limit only the index terminator is still read.
        let encoded_index = if elements_seen < MAX_ELEMENTS {
            reader.read_int_packed()
        } else {
            match take_zero_int_packed(reader) {
                Ok(true) => Ok(0),
                Ok(false) => {
                    stats.truncations += 1;
                    sink.keep_tail(reader, stats);
                    return;
                }
                Err(err) => Err(err),
            }
        };
        let Ok(encoded_index) = encoded_index else {
            stats.errors += 1;
            reader.skip_remaining();
            return;
        };
        if encoded_index == 0 {
            if sink.trailer() {
                consume_optional_trailing_int_packed(reader, stats);
            }
            return;
        }
        let index = encoded_index - 1;
        if index >= element_count {
            stats.errors += 1;
            reader.skip_remaining();
            return;
        }
        stats.elements_decoded += 1;
        // An element that did not close has tallied why; reading on would count
        // the same bits again.
        if !sink.element(index, reader, stats) {
            reader.skip_remaining();
            return;
        }
    }
}

/// One element's fields. Returns whether it closed on its zero handle; every
/// other exit has moved a counter first.
fn walk_fields(
    reader: &mut BitReader<'_>,
    sink: &mut impl Sink,
    stats: &mut ArrayDecodeStats,
) -> bool {
    for field_idx in 0..=MAX_FIELDS_PER_ELEMENT {
        if reader.at_end() {
            stats.implicit_terminations += 1;
            return false;
        }
        if field_idx == MAX_FIELDS_PER_ELEMENT {
            match take_zero_int_packed(reader) {
                Ok(true) => return true,
                Ok(false) => {
                    stats.truncations += 1;
                    sink.keep_tail(reader, stats);
                }
                Err(_) => stats.errors += 1,
            }
            return false;
        }
        let Ok(encoded_handle) = reader.read_int_packed() else {
            stats.errors += 1;
            return false;
        };
        if encoded_handle == 0 {
            return true;
        }
        let Ok(payload_bits) = reader.read_int_packed() else {
            stats.errors += 1;
            return false;
        };
        // Fails only on a payload past the stream: the lost leaves are an
        // error. `sub_reader` advances the parent past the whole window.
        let Ok(sub) = reader.sub_reader(u64::from(payload_bits)) else {
            stats.errors += 1;
            reader.skip_remaining();
            return false;
        };
        sink.field(encoded_handle - 1, sub, stats);
    }
    false
}

/// Consume the optional one-IntPacked trailer after an index terminator -- a
/// ZERO one only, and only when exactly eight bits remain. Another value is
/// left for the caller's residual tally; a failed read is `errors`. All 240
/// trailers in the 1,018-replay audit corpus are zero, in
/// `AbilityCastsThisRound`'s depth-2 `AffectedTargetsArray`.
fn consume_optional_trailing_int_packed(reader: &mut BitReader<'_>, stats: &mut ArrayDecodeStats) {
    if reader.bits_remaining() == 8 && take_zero_int_packed(reader).is_err() {
        stats.errors += 1;
        reader.skip_remaining();
    }
}

/// Consume the next IntPacked only if it is `0`: `Ok(true)` when it was, and
/// `Ok(false)` or `Err` with the reader untouched.
fn take_zero_int_packed(reader: &mut BitReader<'_>) -> vrf_bitio::Result<bool> {
    let mut probe = reader.clone();
    let zero = probe.read_int_packed()? == 0;
    if zero {
        *reader = probe;
    }
    Ok(zero)
}

/// The struct walker: one leaf per field, recursing into the schema's nested
/// arrays, with the path built in one reused buffer.
struct StructSink<'a, 'd> {
    trailer: bool,
    /// Names the REPLAY declares for this group's handles, indexed by handle.
    declared: &'a [Option<&'d str>],
    schema: Option<&'a ArrayFieldSchema>,
    depth: u32,
    path: String,
    output: Vec<FlattenedField>,
}

impl StructSink<'_, '_> {
    /// Push `sub`'s bits as one leaf at `.<label>`.
    fn emit(
        &mut self,
        stats: &mut ArrayDecodeStats,
        handle: u32,
        mut sub: BitReader<'_>,
        label: impl FnOnce(&mut String),
    ) {
        let bit_count = sub.bits_remaining();
        let mut raw_bits = vec![0u8; (bit_count as usize).div_ceil(8)];
        let _ = sub.copy_bits_to(&mut raw_bits, bit_count);
        let prefix_len = self.path.len();
        self.path.push('.');
        label(&mut self.path);
        self.output.push(FlattenedField {
            path: self.path.clone(),
            handle,
            bit_count: bit_count as u32,
            raw_bits,
        });
        stats.fields_emitted += 1;
        self.path.truncate(prefix_len);
    }
}

impl Sink for StructSink<'_, '_> {
    fn trailer(&self) -> bool {
        self.trailer
    }

    /// Everything left, as one `._raw` leaf.
    fn keep_tail(&mut self, reader: &mut BitReader<'_>, stats: &mut ArrayDecodeStats) {
        let remaining = reader.bits_remaining();
        if remaining != 0 {
            if let Ok(tail) = reader.sub_reader(remaining) {
                self.emit(stats, u32::MAX, tail, |path| path.push_str("_raw"));
            }
        }
    }

    fn element(
        &mut self,
        index: u32,
        reader: &mut BitReader<'_>,
        stats: &mut ArrayDecodeStats,
    ) -> bool {
        let prefix_len = self.path.len();
        let _ = write!(self.path, "[{index}]");
        let closed = walk_fields(reader, self, stats);
        self.path.truncate(prefix_len);
        closed
    }

    fn field(&mut self, handle: u32, mut sub: BitReader<'_>, stats: &mut ArrayDecodeStats) {
        let schema = self.schema;
        let nested = schema.and_then(|s| s.sub_array(handle));
        // An empty container has no array to walk; an empty leaf still exists.
        if nested.is_some() && sub.at_end() {
            return;
        }
        if nested.is_some() && self.depth + 1 < MAX_RECURSION_DEPTH {
            let prefix_len = self.path.len();
            self.path.push('.');
            push_label(&mut self.path, &[], schema, handle);
            self.schema = nested;
            self.depth += 1;
            walk_array(&mut sub, self, stats);
            self.depth -= 1;
            self.schema = schema;
            self.path.truncate(prefix_len);
            // The parent is aligned past the window whatever the child read.
            stats.unconsumed_nested_bits += sub.bits_remaining();
            return;
        }
        // A container past the depth limit is kept as one leaf.
        stats.truncations += u64::from(nested.is_some());
        let declared = if nested.is_some() { &[] } else { self.declared };
        self.emit(stats, handle, sub, |path| {
            push_label(path, declared, schema, handle)
        });
    }
}

/// The object-reference walker. `item` is `None` until an element's first
/// populated field, then that field's NetGUID if it decoded.
struct ObjectSink {
    item: Option<Option<u32>>,
    output: Vec<(u32, u32)>,
}

impl Sink for ObjectSink {
    fn trailer(&self) -> bool {
        true
    }

    fn keep_tail(&mut self, reader: &mut BitReader<'_>, _: &mut ArrayDecodeStats) {
        reader.skip_remaining();
    }

    fn element(
        &mut self,
        index: u32,
        reader: &mut BitReader<'_>,
        stats: &mut ArrayDecodeStats,
    ) -> bool {
        self.item = None;
        let closed = walk_fields(reader, self, stats);
        // Kept even when the element did not close.
        if let Some(Some(guid)) = self.item {
            self.output.push((index, guid));
            stats.fields_emitted += 1;
        }
        closed
    }

    fn field(&mut self, _: u32, mut sub: BitReader<'_>, stats: &mut ArrayDecodeStats) {
        // Empty fields do not encode a NetGUID or claim the item's slot.
        if sub.at_end() {
            return;
        }
        if self.item.is_some() {
            stats.unconsumed_nested_bits += sub.bits_remaining();
            return;
        }
        let guid = sub.read_int_packed().ok().filter(|_| sub.at_end());
        stats.errors += u64::from(guid.is_none());
        self.item = Some(guid);
    }
}

/// Append a handle's label: the replay's declared name, then the schema's,
/// then `_h{handle}`. Containers pass `&[]`: their path spelling is an output
/// contract, and the replay declares both 44 and 79 `RegionalDamageInteractions`.
/// `declared` applies at every depth: Unreal flattens a `TArray` of structs
/// onto the ENCLOSING group's handles, so one handle space spans the tree.
fn push_label(
    path: &mut String,
    declared: &[Option<&str>],
    schema: Option<&ArrayFieldSchema>,
    handle: u32,
) {
    let declared = declared.get(handle as usize).copied().flatten();
    match declared.or_else(|| schema.and_then(|s| s.field_name(handle))) {
        Some(name) => path.push_str(name),
        None => {
            let _ = write!(path, "_h{handle}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_testkit::{BitWrite, BitWriter};

    static INNER: ArrayFieldSchema = ArrayFieldSchema {
        sub_arrays: &[],
        field_names: &[],
    };
    /// Handle 4 is a sub-array, `Reports`, with no further nesting; handle 3 is
    /// a leaf the schema names `RoundNumber`.
    static OUTER: ArrayFieldSchema = ArrayFieldSchema {
        sub_arrays: &[(4, &INNER)],
        field_names: &[(3, "RoundNumber"), (4, "Reports")],
    };

    /// One element carrying one field per `(handle, payload width)`, each
    /// payload all ones, then the element and array terminators.
    fn one_element_with_handles(handles: &[(u32, u32)]) -> BitWriter {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex=1 -> index 0
        for &(handle, payload) in handles {
            bits.int_packed(handle + 1);
            bits.int_packed(payload);
            bits.repeat(true, payload as usize);
        }
        bits.int_packed(0); // end of element
        bits.int_packed(0); // array terminator
        bits
    }

    /// One outer element whose handle 4 (`Reports` under [`OUTER`]) is a
    /// nested window: `inner`, then `tail` one-bits the inner array never
    /// reads, inside the window's declared width.
    fn nested(inner: &BitWriter, tail: usize) -> (Vec<u8>, u32) {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // outer elementCount
        bits.int_packed(1); // encodedIndex=1
        bits.int_packed(5); // encodedHandle=5 -> handle 4
        bits.int_packed(inner.bit_len() + tail as u32);
        bits.extend_bits(inner).repeat(true, tail);
        bits.int_packed(0); // end outer element
        bits.int_packed(0); // outer array terminator
        bits.finish()
    }

    #[test]
    fn a_zero_width_leaf_preserves_presence_and_decodes_as_enum_zero() {
        // Given: capacity one, element zero, handle zero, an empty payload,
        // then both terminators.
        let data = [2, 2, 2, 0, 0, 0];
        let mut stats = ArrayDecodeStats::default();

        // When: walking exactly the recorded 48-bit array window.
        let fields = decode_struct_array_exact(&data, 48, &[], &mut stats);

        // Then: presence survives without borrowing any terminator bits.
        assert_eq!(fields.len(), 1, "{stats:?}: {fields:?}");
        let leaf = &fields[0];
        assert_eq!((leaf.path.as_str(), leaf.handle), ("[0]._h0", 0));
        assert_eq!(leaf.bit_count, 0);
        assert!(leaf.raw_bits.is_empty());
        assert_eq!(
            crate::decode_field(
                crate::FieldType::EnumRemainingBits,
                &leaf.raw_bits,
                leaf.bit_count,
            )
            .unwrap(),
            crate::DecodedValue::I64(0)
        );
        assert_eq!(stats.elements_decoded, 1);
        assert_eq!(stats.fields_emitted, 1);
        assert!(stats.is_clean(), "{stats:?}");
        assert_eq!(data, [2, 2, 2, 0, 0, 0]);
    }

    #[test]
    fn a_nested_zero_width_leaf_preserves_its_declared_path() {
        // Given: a schema container with a present, empty member.
        let (data, bit_count) = nested(&one_element_with_handles(&[(7, 0)]), 0);
        let names = [None, None, None, None, None, None, None, Some("State")];
        let mut stats = ArrayDecodeStats::default();

        // When: flattening the nested array.
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &names, &mut stats);

        // Then: only the leaf's own empty window is emitted at its full path.
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0].State");
        assert_eq!(fields[0].handle, 7);
        assert_eq!(fields[0].bit_count, 0);
        assert!(fields[0].raw_bits.is_empty());
        assert_eq!(stats.fields_emitted, 1);
        assert!(stats.is_clean(), "{stats:?}");
    }

    #[test]
    fn a_zero_width_container_still_has_no_array_to_walk() {
        // Given: an empty payload at a schema's container handle.
        let (data, bit_count) = one_element_with_handles(&[(4, 0)]).finish();
        let mut stats = ArrayDecodeStats::default();

        // When: walking the schema-guided array.
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);

        // Then: no nested array or leaf is invented.
        assert!(fields.is_empty());
        assert_eq!(stats.elements_decoded, 1);
        assert_eq!(stats.fields_emitted, 0);
        assert!(stats.is_clean(), "{stats:?}");
    }

    #[test]
    fn a_zero_width_object_field_still_leaves_the_guid_slot_available() {
        // Given: an empty field before the first populated NetGUID field.
        let mut bits = BitWriter::new();
        bits.int_packed(1).int_packed(1);
        bits.int_packed(1).int_packed(0);
        bits.int_packed(3).int_packed(8).int_packed(5);
        bits.int_packed(0).int_packed(0);
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        // When: walking the same framing as an object-reference array.
        let items = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        // Then: the empty field has not displaced the real GUID or moved errors.
        assert_eq!(items, [(0, 5)]);
        assert_eq!(stats.fields_emitted, 1);
        assert!(stats.is_clean(), "{stats:?}");
    }

    #[test]
    fn decode_simple_struct_array() {
        // elementCount=2, element[0] has handle=3 with 32 bits, element[1]
        // has handle=5 with 8 bits, then terminators.
        let mut bits = BitWriter::new();
        bits.int_packed(2); // elementCount
        // Element 0 (encodedIndex=1): encodedHandle=4 (handle=3), payloadBits=32
        bits.int_packed(1);
        bits.int_packed(4);
        bits.int_packed(32);
        bits.repeat(true, 32); // payload
        bits.int_packed(0); // end of element 0
        // Element 1 (encodedIndex=2): encodedHandle=6 (handle=5), payloadBits=8
        bits.int_packed(2);
        bits.int_packed(6);
        bits.int_packed(8);
        bits.repeat(false, 8); // payload
        bits.int_packed(0); // end of element 1
        // Array terminator
        bits.int_packed(0);

        let (data, bit_count) = bits.finish();
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
        // One outer element whose handle 4 is a nested array of one element
        // carrying handle 7 with 16 bits.
        let (data, bit_count) = nested(&one_element_with_handles(&[(7, 16)]), 0);
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);

        assert_eq!(stats.elements_decoded, 2); // 1 outer + 1 inner
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0]._h7");
        assert_eq!(fields[0].handle, 7);
        assert_eq!(fields[0].bit_count, 16);
    }

    /// The replay's declaration outranks the schema on a leaf.
    #[test]
    fn a_declared_leaf_name_beats_the_schema_name() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32)]).finish();
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[3] = Some("RoundNum");

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &declared, &mut stats);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].RoundNum");
    }

    /// A handle the schema cannot name is labelled by the replay, not `_hN`.
    #[test]
    fn a_declared_leaf_name_replaces_the_handle_placeholder() {
        let (data, bit_count) = one_element_with_handles(&[(7, 32)]).finish();
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[7] = Some("StateRemainingTime");

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &declared, &mut stats);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].StateRemainingTime");
    }

    /// With no declaration the schema still names what it can, and an
    /// undeclared, unschematised handle still falls through to `_hN`.
    #[test]
    fn an_undeclared_leaf_falls_back_to_schema_then_placeholder() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32), (7, 32)]).finish();

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);

        assert_eq!(fields.len(), 2);
        // Schema names handle 3; nothing names handle 7.
        assert_eq!(fields[0].path, "[0].RoundNumber");
        assert_eq!(fields[1].path, "[0]._h7");
    }

    /// A declaration shorter than the handle it is asked about must not panic
    /// and must fall through, not silently mislabel.
    #[test]
    fn a_handle_past_the_end_of_the_declaration_falls_back() {
        let (data, bit_count) = one_element_with_handles(&[(7, 32)]).finish();
        // Only handles 0..=3 declared; handle 7 is past the end.
        let declared: Vec<Option<&str>> = vec![None, None, None, Some("RoundNum")];

        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &declared, &mut stats);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0]._h7");
    }

    /// A container segment keeps the production schema's name over a declared one.
    #[test]
    fn a_container_segment_keeps_its_schema_name() {
        // Outer element 0 carries handle 4 (Reports, a sub-array) whose single
        // element carries handle 5.
        let (data, bit_count) = nested(&one_element_with_handles(&[(5, 32)]), 0);

        // Declare a DIFFERENT name for the container handle 4, and the real
        // declared name for the leaf handle 5.
        let mut declared: Vec<Option<&str>> = vec![None; 8];
        declared[4] = Some("NotReports");
        declared[5] = Some("RoundNumber");

        let mut stats = ArrayDecodeStats::default();
        let schema = Some(&COMBAT_ROUNDS_SCHEMA);
        let fields = decode_struct_array(&data, bit_count, schema, &declared, &mut stats);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, "[0].Reports[0].RoundNumber");
    }

    #[test]
    fn empty_array_emits_nothing() {
        let mut bits = BitWriter::new();
        bits.int_packed(0); // elementCount = 0
        bits.int_packed(0); // immediate terminator

        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert!(fields.is_empty());
        assert_eq!(stats.elements_decoded, 0);
        // A legitimately empty array must read as zero errors so a malformed
        // run cannot hide behind the same number.
        assert_eq!(stats.errors, 0);
        // Nor is an empty window an anomaly: there is no array to walk.
        assert!(decode_struct_array(&[], 0, None, &[], &mut stats).is_empty());
        assert_eq!(anomalies(&stats), [0; 5], "{stats:?}");
    }

    /// A payload that declares an element but EOFs mid-field is an error, not
    /// an empty `Vec` that reads like a clean empty array.
    #[test]
    fn truncated_payload_mid_element_counts_error() {
        // elementCount=2, element 0 starts, its first field declares 32 bits
        // of payload but only 8 remain -> overrun.
        let mut bits = BitWriter::new();
        bits.int_packed(2); // elementCount = 2
        bits.int_packed(1); // encodedIndex = 1 -> element 0
        bits.int_packed(4); // encodedHandle = 4 -> handle 3
        bits.int_packed(32); // payloadBits = 32 (overruns)
        bits.repeat(true, 8); // only 8 bits of payload

        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        // One overrun, one counter: not also an implicit termination from the
        // array loop meeting the EOF the element already reported.
        assert_eq!(anomalies(&stats), [1, 0, 0, 0, 0], "{stats:?}");
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
        let mut bits = BitWriter::new();
        bits.int_packed(2); // elementCount
        // Element 0: NetGUID 812 in a 16-bit payload window.
        bits.int_packed(1); // encodedIndex -> index 0
        bits.int_packed(3); // encodedHandle -> handle 2
        bits.int_packed(16); // payloadBits
        bits.int_packed(812); // ObjectNetGuid payload
        bits.int_packed(0); // element terminator
        // Element 1: NetGUID 25492 in a 24-bit payload window.
        bits.int_packed(2); // encodedIndex -> index 1
        bits.int_packed(3); // handle 2
        bits.int_packed(24); // payloadBits
        bits.int_packed(25492); // ObjectNetGuid payload
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator

        let (data, bit_count) = bits.finish();
        let guids = decode_object_ref_array(&data, bit_count);

        assert_eq!(guids, vec![(0, 812), (1, 25492)]);
    }

    /// A sparse re-send -- only wire index 1 of a 3-slot array present -- must
    /// keep that index, not relabel the lone GUID onto slot 0.
    #[test]
    fn decode_object_ref_array_preserves_a_sparse_wire_index() {
        let mut bits = BitWriter::new();
        bits.int_packed(3); // elementCount (3-slot array)
        bits.int_packed(2); // encodedIndex -> index 1 (only element sent)
        bits.int_packed(3); // encodedHandle -> handle 2
        bits.int_packed(16); // payloadBits
        bits.int_packed(5150); // ObjectNetGuid payload
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator

        let (data, bit_count) = bits.finish();
        let guids = decode_object_ref_array(&data, bit_count);

        assert_eq!(guids, vec![(1, 5150)]);
    }

    /// An empty array (elementCount = 0, immediate terminator) decodes to nothing.
    #[test]
    fn decode_object_ref_array_empty() {
        let mut bits = BitWriter::new();
        bits.int_packed(0);
        bits.int_packed(0);

        let (data, bit_count) = bits.finish();
        let guids = decode_object_ref_array(&data, bit_count);

        assert!(guids.is_empty());
    }

    #[test]
    fn malformed_object_ref_array_exposes_its_failure_stats() {
        let mut bits = BitWriter::new();
        bits.int_packed(1);
        bits.int_packed(1);
        bits.int_packed(3);
        bits.int_packed(32); // only eight payload bits follow
        bits.repeat(false, 8);
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        assert!(guids.is_empty());
        assert_eq!(stats.errors, 1, "{stats:?}");
    }

    /// Every counter a clean walk must leave at zero.
    fn anomalies(stats: &ArrayDecodeStats) -> [u64; 5] {
        [
            stats.errors,
            stats.truncations,
            stats.implicit_terminations,
            stats.unconsumed_root_bits,
            stats.unconsumed_nested_bits,
        ]
    }

    /// One `MultiContents` element at wire index 0 carrying NetGUID 5 in an
    /// 8-bit window, closed by its zero handle.
    fn push_one_closed_item(bits: &mut BitWriter) {
        bits.int_packed(1); // encodedIndex -> index 0
        bits.int_packed(3); // encodedHandle -> handle 2
        bits.int_packed(8); // payloadBits
        bits.int_packed(5); // ObjectNetGuid payload
        bits.int_packed(0); // element terminator
    }

    /// A payload cut short after a complete element, before the index
    /// terminator, must not read as the whole array: in a delta array the
    /// missing elements would pass for ones not re-sent.
    #[test]
    fn an_object_ref_array_ending_at_eof_is_not_a_clean_terminator() {
        let mut bits = BitWriter::new();
        bits.int_packed(2); // elementCount: two slots
        push_one_closed_item(&mut bits);
        // No second element and no array terminator: the payload just ends.
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        assert_eq!(guids, vec![(0, 5)], "the complete element is still decoded");
        assert_eq!(stats.implicit_terminations, 1, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_root_bits, 0, "{stats:?}");
    }

    /// A window holding only the element count ends the same way.
    #[test]
    fn a_count_only_object_ref_window_is_an_implicit_termination() {
        let mut stats = ArrayDecodeStats::default();
        let guids = decode_object_ref_array_with_stats(&[0x02], 8, &mut stats);

        assert!(guids.is_empty());
        assert_eq!(stats.implicit_terminations, 1, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    /// An element that runs out of bits is ONE implicit termination: the
    /// element's own tally already fires, and the array loop must not add a
    /// second for the same missing bits.
    #[test]
    fn an_object_ref_element_ending_at_eof_counts_once() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        bits.int_packed(3); // handle 2
        bits.int_packed(8); // payloadBits
        bits.int_packed(5); // NetGUID -- then EOF, no terminators
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        assert_eq!(guids, vec![(0, 5)]);
        assert_eq!(stats.implicit_terminations, 1, "{stats:?}");
    }

    /// A second populated field (`TArray<AAresItem*>` elements have one) is
    /// stepped over and its payload bits tallied; the 16-bit window shows the
    /// tally counts bits, not fields.
    #[test]
    fn an_extra_field_in_an_object_ref_element_is_tallied() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        bits.int_packed(3); // handle 2
        bits.int_packed(8); // payloadBits
        bits.int_packed(5); // NetGUID 5
        bits.int_packed(4); // handle 3: a field the type does not have
        bits.int_packed(16); // payloadBits
        bits.int_packed(300); // two IntPacked bytes
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        assert_eq!(guids, vec![(0, 5)], "the first field is still the item");
        assert_eq!(stats.unconsumed_nested_bits, 16, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");
    }

    /// The first populated field is the item whether or not it decodes, so a
    /// first field that fails yields no item: the field after it is one the
    /// walker calls foreign, and must not be promoted into the item's place.
    #[test]
    fn an_object_ref_element_whose_first_field_fails_yields_no_item() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        bits.int_packed(3); // handle 2
        bits.int_packed(16); // payloadBits
        bits.int_packed(5); // NetGUID 5, leaving eight bits unread
        bits.repeat(false, 8);
        bits.int_packed(4); // handle 3: a field the type does not have
        bits.int_packed(16); // payloadBits
        bits.int_packed(777); // two IntPacked bytes
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);

        assert!(guids.is_empty(), "second field promoted: {guids:?}");
        assert_eq!(anomalies(&stats), [1, 0, 0, 0, 16], "{stats:?}");
    }

    /// The counters above must stay silent on well-formed arrays, or they
    /// would flag every `MultiContents` blob in the corpus.
    #[test]
    fn terminated_object_ref_arrays_report_no_anomaly() {
        // Two populated slots.
        let mut two = BitWriter::new();
        two.int_packed(2);
        push_one_closed_item(&mut two);
        two.int_packed(2); // encodedIndex -> index 1
        two.int_packed(3);
        two.int_packed(24);
        two.int_packed(25492); // three IntPacked bytes
        two.int_packed(0);
        two.int_packed(0); // array terminator
        // An empty array: count, then the terminator.
        let mut empty = BitWriter::new();
        empty.int_packed(0);
        empty.int_packed(0);

        for (bits, want) in [(two, vec![(0, 5), (1, 25492)]), (empty, vec![])] {
            let (data, bit_count) = bits.finish();
            let mut stats = ArrayDecodeStats::default();
            let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);
            assert_eq!(guids, want);
            assert_eq!(anomalies(&stats), [0; 5], "{stats:?}");
        }
    }

    /// A BitIo read failure (fewer than 8 bits left for an IntPacked read) must
    /// also count as an error rather than returning silently.
    #[test]
    fn read_failure_mid_stream_counts_error() {
        // elementCount=1, element 0 starts, its field header declares a handle,
        // then the stream ends with too few bits for the payloadBits IntPacked.
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount = 1
        bits.int_packed(1); // encodedIndex = 1 -> element 0
        bits.int_packed(4); // encodedHandle = 4 -> handle 3
        // Three stray bits: not enough for an IntPacked payloadBits read.
        bits.repeat(false, 3);

        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();
        let _fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        // One failed read, one counter: the array loop must not read the same
        // three bits as an index and fail again, nor leave them as residual.
        assert_eq!(anomalies(&stats), [1, 0, 0, 0, 0], "{stats:?}");
    }

    /// A nested array that leaves bits inside its own window must say so: the
    /// parent's `sub_reader` keeps the walk aligned either way.
    #[test]
    fn a_nested_array_that_leaves_bits_reports_them() {
        // Sixteen bits the inner array will never look at. Not 8: exactly
        // eight bits after a terminator are read as the optional trailer,
        // which is a different path with its own tests.
        let (data, bit_count) = nested(&one_element_with_handles(&[(7, 16)]), 16);
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
        let mut bits = BitWriter::new();
        bits.int_packed(1);
        bits.int_packed(1);
        bits.int_packed(4); // handle 3
        bits.int_packed(32);
        bits.repeat(true, 32);

        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        // The field itself is complete and is still emitted.
        assert_eq!(fields.len(), 1);
        // One EOF, counted once: by the element, not again by the array loop.
        assert_eq!(anomalies(&stats), [0, 0, 1, 0, 0], "{stats:?}");
    }

    /// The counter must NOT fire on a well-formed array that closes with both
    /// its terminators, or it would flag every clean blob in the corpus.
    #[test]
    fn a_cleanly_terminated_array_reports_no_implicit_termination() {
        let (data, bit_count) = one_element_with_handles(&[(3, 32)]).finish();
        let mut stats = ArrayDecodeStats::default();
        let _ = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    #[test]
    fn bits_after_the_root_array_terminator_are_tallied() {
        let mut bits = BitWriter::new();
        bits.int_packed(0);
        bits.int_packed(0);
        bits.repeat(true, 16);
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.unconsumed_root_bits, 16, "{stats:?}");
    }

    #[test]
    fn an_out_of_range_element_index_is_malformed_not_a_clean_empty_array() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // one declared element
        bits.int_packed(2); // index 1 is outside [0, 1)
        bits.repeat(true, 16);
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert!(fields.is_empty());
        assert_eq!(stats.errors, 1, "{stats:?}");
    }

    #[test]
    fn a_truncated_trailing_int_packed_is_not_accepted() {
        let mut bits = BitWriter::new();
        bits.int_packed(0); // zero elements
        bits.int_packed(0); // array terminator
        // Exactly eight trailing bits activates the optional IntPacked read,
        // but the continuation flag asks for a byte that is not present.
        bits.bits(0x01, 8);
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.errors, 1, "{stats:?}");
    }

    /// Every byte that can follow the root terminator as the optional trailer
    /// moves exactly one counter, or none for the one byte that IS a
    /// terminator (discarding any of them unread would hide an appended byte).
    #[test]
    fn every_root_trailer_byte_is_accounted_for() {
        for byte in 0..=u8::MAX {
            // Zero elements, the index terminator, then the one trailer byte.
            let data = [0x00, 0x00, byte];
            let mut stats = ArrayDecodeStats::default();

            let fields = decode_struct_array(&data, 24, None, &[], &mut stats);

            assert!(fields.is_empty(), "byte {byte:#04x}");
            let (root_bits, errors) = match byte {
                // The terminator: consumed, nothing to report.
                0 => (0, 0),
                // A continuation bit asking for a byte past the window.
                b if b & 1 == 1 => (0, 1),
                // A complete, nonzero IntPacked: not a terminator.
                _ => (8, 0),
            };
            assert_eq!(
                (stats.unconsumed_root_bits, stats.errors),
                (root_bits, errors),
                "byte {byte:#04x}: {stats:?}"
            );
            assert_eq!(stats.unconsumed_nested_bits, 0, "byte {byte:#04x}");
            assert_eq!(stats.implicit_terminations, 0, "byte {byte:#04x}");
        }
    }

    /// One nested window: a one-element inner array, its terminator, then one
    /// trailing byte holding IntPacked `trailer`.
    fn nested_window_with_trailer(trailer: u32) -> (Vec<u8>, u32) {
        nested(one_element_with_handles(&[(7, 16)]).int_packed(trailer), 0)
    }

    /// A nonzero trailer inside a nested array's window is the same anomaly as
    /// at the root, and the nested residual tally is what must report it: the
    /// parent's window advanced past it, so nothing else would.
    #[test]
    fn a_nonzero_trailer_inside_a_nested_window_is_tallied() {
        let (data, bit_count) = nested_window_with_trailer(1);
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);

        // The leaf before the trailer is still emitted -- only a counter moves.
        assert_eq!(fields.len(), 1, "{fields:?}");
        assert_eq!(fields[0].path, "[0].Reports[0]._h7");
        assert_eq!(stats.unconsumed_nested_bits, 8, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_root_bits, 0, "{stats:?}");

        // The same window closed by a ZERO trailer is the shape real data
        // carries (`AffectedTargetsArray`, depth 2) and must stay silent.
        let (data, bit_count) = nested_window_with_trailer(0);
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);
        assert_eq!(fields.len(), 1, "{fields:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    /// The zero trailer is real: an `Effects[]` window off the wire, pinned so a
    /// tighter trailer rule has to face it. Carved from an
    /// `AbilityCastsThisRound` parent of 13.01 replay `b9d2fac4`: 168 bits, one
    /// effect element (`Value`, `Time`, then an `AffectedTargetsArray` whose
    /// 24-bit window is `02 00 00` -- capacity one, no changed element, the zero
    /// trailer). All 240 trailers in the 1,018-replay audit corpus are this.
    #[test]
    fn a_real_zero_trailer_is_consumed_and_a_flipped_one_is_not() {
        let mut raw = [
            0x02, 0x02, 0x22, 0x40, 0x00, 0x00, 0x80, 0x3f, 0x24, 0x40, 0x20, 0xee, 0x8d, 0x41,
            0x26, 0x30, 0x02, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut stats = ArrayDecodeStats::default();
        let out = decode_struct_array(&raw, 168, Some(&ABILITY_EFFECTS_SCHEMA), &[], &mut stats);

        let paths: Vec<&str> = out.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["[0].Value", "[0].Time"], "{paths:?}");
        // Positive payload control: the real Value window is float 1.0.
        assert_eq!(out[0].bit_count, 32);
        assert_eq!(out[0].raw_bits, [0x00, 0x00, 0x80, 0x3f]);
        assert_eq!(
            crate::decode_field(crate::FieldType::Float, &out[0].raw_bits, out[0].bit_count)
                .unwrap(),
            crate::DecodedValue::F64(1.0)
        );
        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_root_bits, 0, "{stats:?}");
        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");

        // Byte 18 is that trailer. As IntPacked 1 it is no terminator, and
        // the eight bits must surface in the nested window they sit in.
        raw[18] = 0x02;
        let mut stats = ArrayDecodeStats::default();
        let out = decode_struct_array(&raw, 168, Some(&ABILITY_EFFECTS_SCHEMA), &[], &mut stats);
        assert_eq!(out.len(), 2, "the leaves before it are still emitted");
        assert_eq!(stats.unconsumed_nested_bits, 8, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    /// `MultiContents` goes through the same trailer helper, unconditionally.
    #[test]
    fn a_nonzero_trailer_after_an_object_ref_array_is_tallied() {
        let mut stats = ArrayDecodeStats::default();
        let guids = decode_object_ref_array_with_stats(&[0x00, 0x00, 0x02], 24, &mut stats);
        assert!(guids.is_empty());
        assert_eq!(stats.unconsumed_root_bits, 8, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");

        let mut stats = ArrayDecodeStats::default();
        let guids = decode_object_ref_array_with_stats(&[0x00, 0x00, 0x00], 24, &mut stats);
        assert!(guids.is_empty());
        assert_eq!(stats.unconsumed_root_bits, 0, "{stats:?}");
        assert_eq!(stats.errors, 0, "{stats:?}");
    }

    /// `n` fields at handle 0 with an empty payload: headers, no payload bits.
    fn zero_width_fields(bits: &mut BitWriter, n: u32) {
        for _ in 0..n {
            bits.int_packed(1).int_packed(0);
        }
    }

    /// The zero handle arriving exactly at the field limit closes the element,
    /// so the next element is still read: reported as unclosed, it would be
    /// dropped with every counter still at zero.
    #[test]
    fn exactly_max_fields_followed_by_a_terminator_is_not_truncated() {
        let mut bits = BitWriter::new();
        bits.int_packed(2); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        zero_width_fields(&mut bits, MAX_FIELDS_PER_ELEMENT);
        bits.int_packed(0); // element terminator, read by the limit check
        bits.int_packed(2); // encodedIndex -> index 1
        bits.int_packed(4).int_packed(32).repeat(true, 32); // handle 3, 32 bits
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.elements_decoded, 2, "{stats:?}");
        let paths: Vec<&str> = fields.iter().map(|f| f.path.as_str()).collect();
        let mut expected = vec!["[0]._h0"; usize::try_from(MAX_FIELDS_PER_ELEMENT).unwrap()];
        expected.push("[1]._h3");
        assert_eq!(paths, expected);
        assert_eq!(stats.fields_emitted, u64::from(MAX_FIELDS_PER_ELEMENT) + 1);
        assert_eq!(anomalies(&stats), [0; 5], "{stats:?}");
    }

    /// A 129th field is one truncation: the rest of the window, from that
    /// field's handle on, is kept as one raw leaf, and the array stops there.
    #[test]
    fn a_field_past_the_field_limit_is_one_truncation() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        zero_width_fields(&mut bits, MAX_FIELDS_PER_ELEMENT + 1);
        bits.int_packed(0); // element terminator
        bits.int_packed(0); // array terminator
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(anomalies(&stats), [0, 1, 0, 0, 0], "{stats:?}");
        // The 129th field's 16-bit header and both terminators: the limit
        // check read that header without consuming it.
        let raw: Vec<(&str, u32)> = fields
            .iter()
            .map(|f| (f.path.as_str(), f.bit_count))
            .collect();
        let mut expected = vec![("[0]._h0", 0); usize::try_from(MAX_FIELDS_PER_ELEMENT).unwrap()];
        expected.push(("[0]._raw", 32));
        assert_eq!(raw, expected);
    }

    /// A read that fails at the field limit is one error: the array loop must
    /// not read the same bits again as an index, nor leave them as residual.
    #[test]
    fn a_failed_read_at_the_field_limit_is_one_error() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // elementCount
        bits.int_packed(1); // encodedIndex -> index 0
        zero_width_fields(&mut bits, MAX_FIELDS_PER_ELEMENT);
        bits.repeat(true, 3); // too few bits for the next handle
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(
            fields.len(),
            usize::try_from(MAX_FIELDS_PER_ELEMENT).unwrap()
        );
        assert!(fields.iter().all(|field| field.path == "[0]._h0"
            && field.bit_count == 0
            && field.raw_bits.is_empty()));
        assert_eq!(anomalies(&stats), [1, 0, 0, 0, 0], "{stats:?}");
    }

    /// The object-reference walker at the same limit: a zero handle closes the
    /// element and the next one is still read; a 129th field is one
    /// truncation and a failed read one error, and either stops the array.
    #[test]
    fn the_object_ref_walker_reads_the_next_int_packed_at_the_field_limit() {
        let element_at_the_limit = || {
            let mut bits = BitWriter::new();
            bits.int_packed(2); // elementCount
            bits.int_packed(1); // encodedIndex -> index 0
            zero_width_fields(&mut bits, MAX_FIELDS_PER_ELEMENT);
            bits
        };
        let mut closed = element_at_the_limit();
        closed.int_packed(0); // element terminator, read by the limit check
        closed.int_packed(2); // encodedIndex -> index 1
        closed.int_packed(3).int_packed(8).int_packed(5); // handle 2: NetGUID 5
        closed.int_packed(0).int_packed(0); // element and array terminators
        let mut over = element_at_the_limit();
        zero_width_fields(&mut over, 1); // a 129th field
        over.int_packed(0).int_packed(0);
        let mut failed = element_at_the_limit();
        failed.repeat(true, 3); // too few bits for the next handle

        for (name, bits, want, counters) in [
            ("closed", closed, vec![(1, 5)], [0; 5]),
            ("129th field", over, vec![], [0, 1, 0, 0, 0]),
            ("failed read", failed, vec![], [1, 0, 0, 0, 0]),
        ] {
            let (data, bit_count) = bits.finish();
            let mut stats = ArrayDecodeStats::default();
            let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);
            assert_eq!(guids, want, "{name}");
            assert_eq!(anomalies(&stats), counters, "{name}: {stats:?}");
        }
    }

    #[test]
    fn repeated_element_indices_cannot_bypass_the_element_work_limit() {
        let mut bits = BitWriter::new();
        bits.int_packed(1); // one declared slot
        for _ in 0..=MAX_ELEMENTS {
            bits.int_packed(1); // repeat index 0
            bits.int_packed(0); // empty element
        }
        bits.int_packed(0); // array terminator
        let (data, bit_count) = bits.finish();
        let mut stats = ArrayDecodeStats::default();

        let _ = decode_struct_array(&data, bit_count, None, &[], &mut stats);

        assert_eq!(stats.elements_decoded, u64::from(MAX_ELEMENTS));
        assert_eq!(stats.truncations, 1, "{stats:?}");
    }

    /// After `MAX_ELEMENTS` elements both walkers read the next IntPacked in
    /// the limit check: the index terminator, then the optional trailer as
    /// after any terminator; another index, one truncation; or a failed read,
    /// one error. Only bits after the terminator are root residual: the other
    /// two are one anomaly each, the struct walker keeping a truncated tail as
    /// a `._raw` leaf.
    #[test]
    fn both_walkers_read_the_next_int_packed_at_the_element_limit() {
        // (the tail after the last element as `bits(value, width)`, the struct
        // walker's counters and `._raw` leaf width, the object-ref walker's
        // counters)
        let cases = [
            (0x0000, 16, [0; 5], None, [0; 5]), // terminator, zero trailer
            (0x0200, 16, [0, 0, 0, 8, 0], None, [0, 0, 0, 8, 0]), // terminator, trailer 1
            (0x0100, 16, [1, 0, 0, 0, 0], None, [1, 0, 0, 0, 0]), // terminator, cut trailer
            (0x0002, 16, [0, 1, 0, 0, 0], Some(16), [0, 1, 0, 0, 0]), // one element too many
            (0b111, 3, [1, 0, 0, 0, 0], None, [1, 0, 0, 0, 0]), // too few bits to read
        ];
        for (tail, width, struct_counters, raw_width, object_ref_counters) in cases {
            let case = format!("tail {tail:#06x}/{width}");
            let mut bits = BitWriter::new();
            bits.int_packed(1); // one declared slot
            for _ in 0..MAX_ELEMENTS {
                bits.int_packed(1).int_packed(0); // index 0 again, empty element
            }
            bits.bits(tail, width);
            let (data, bit_count) = bits.finish();

            let mut stats = ArrayDecodeStats::default();
            let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);
            let raw: Vec<(&str, u32)> = fields
                .iter()
                .map(|f| (f.path.as_str(), f.bit_count))
                .collect();
            let want: Vec<(&str, u32)> = raw_width.map(|n| ("._raw", n)).into_iter().collect();
            assert_eq!(raw, want, "{case}");
            assert_eq!(stats.elements_decoded, u64::from(MAX_ELEMENTS), "{case}");
            assert_eq!(anomalies(&stats), struct_counters, "{case}: {stats:?}");

            let mut stats = ArrayDecodeStats::default();
            let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);
            assert!(guids.is_empty(), "{case}: {guids:?}");
            assert_eq!(stats.elements_decoded, u64::from(MAX_ELEMENTS), "{case}");
            assert_eq!(anomalies(&stats), object_ref_counters, "{case}: {stats:?}");
        }
    }

    /// The other array-level stops that move `errors` or `truncations` are one
    /// anomaly each in both walkers as well: a failed element-count read, a
    /// declared count over `MAX_ELEMENTS` (the struct walker keeps the rest as
    /// a `._raw` leaf), a failed index read and an index out of range. The
    /// tail is neither root residual nor, in a nested window, nested residual,
    /// and what was decoded before the stop is kept.
    #[test]
    fn an_array_level_stop_counts_once_in_both_walkers() {
        let mut count_cut = BitWriter::new();
        count_cut.repeat(true, 3); // too few bits for the element count
        let mut over_limit = BitWriter::new();
        over_limit.int_packed(MAX_ELEMENTS + 1).repeat(true, 16);
        let mut index_cut = BitWriter::new();
        index_cut.int_packed(2); // elementCount
        push_one_closed_item(&mut index_cut);
        index_cut.repeat(true, 3); // too few bits for the next index
        let mut out_of_range = BitWriter::new();
        out_of_range.int_packed(1).int_packed(2).repeat(true, 16); // index 1 of 1

        // (case, bits, both walkers' counters, the struct walker's leaves, the
        // object-ref walker's items)
        let cases = [
            ("count read", count_cut, [1, 0, 0, 0, 0], vec![], vec![]),
            (
                "count over the limit",
                over_limit,
                [0, 1, 0, 0, 0],
                vec![("._raw", 16)],
                vec![],
            ),
            (
                "index read",
                index_cut,
                [1, 0, 0, 0, 0],
                vec![("[0]._h2", 8)],
                vec![(0, 5)],
            ),
            (
                "index out of range",
                out_of_range,
                [1, 0, 0, 0, 0],
                vec![],
                vec![],
            ),
        ];
        for (case, bits, counters, leaves, items) in cases {
            let (data, bit_count) = bits.finish();
            let mut stats = ArrayDecodeStats::default();
            let fields = decode_struct_array(&data, bit_count, None, &[], &mut stats);
            let got: Vec<(&str, u32)> = fields
                .iter()
                .map(|f| (f.path.as_str(), f.bit_count))
                .collect();
            assert_eq!(got, leaves, "{case}");
            assert_eq!(anomalies(&stats), counters, "{case}: {stats:?}");

            let mut stats = ArrayDecodeStats::default();
            let guids = decode_object_ref_array_with_stats(&data, bit_count, &mut stats);
            assert_eq!(guids, items, "{case}");
            assert_eq!(anomalies(&stats), counters, "{case}: {stats:?}");
        }

        // A nested array whose index read fails: one error, and the outer walk
        // still reads its own element and array terminators.
        let (data, bit_count) = nested(BitWriter::new().int_packed(1), 3);
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(&data, bit_count, Some(&OUTER), &[], &mut stats);
        assert!(fields.is_empty(), "{fields:?}");
        assert_eq!(anomalies(&stats), [1, 0, 0, 0, 0], "{stats:?}");
    }

    /// `AbilityCastsThisRound[].Effects[]` is a nested array only its schema
    /// reveals. The payload is real, the smallest carrying an
    /// `AffectedTargetsArray`: 128 bits, one effect element whose only field is
    /// handle 18, itself an 80-bit array. It also pins the residual tallies at
    /// zero on REAL data, not only on hand-built fixtures.
    #[test]
    fn the_ability_effects_array_descends_into_its_targets() {
        let raw = [
            0x02, 0x02, 0x26, 0xa0, 0x04, 0x04, 0x2a, 0x40, 0x7e, 0x16, 0x12, 0x3f, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut stats = ArrayDecodeStats::default();
        let out = decode_struct_array(&raw, 128, Some(&ABILITY_EFFECTS_SCHEMA), &[], &mut stats);
        assert_eq!(stats.errors, 0, "{stats:?}");
        assert_eq!(stats.unconsumed_nested_bits, 0, "{stats:?}");
        assert_eq!(stats.implicit_terminations, 0, "{stats:?}");

        // Only `Value` appears, not `AffectedPlayer`: replication is per
        // property, so an element re-sent because one member changed carries
        // only that member (carry the last value forward per element index).
        let paths: Vec<&str> = out.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["[0].AffectedTargetsArray[1].Value"], "{paths:?}");
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

    /// Every field, a distinct value, so a field summed into the wrong target
    /// or not summed at all fails here instead of cancelling out.
    fn distinct_stats(base: u64) -> ArrayDecodeStats {
        ArrayDecodeStats {
            elements_decoded: base + 1,
            fields_emitted: base + 2,
            truncations: base + 3,
            errors: base + 4,
            unconsumed_nested_bits: base + 5,
            unconsumed_root_bits: base + 6,
            implicit_terminations: base + 7,
        }
    }

    /// `merge_from` sums each counter into its own total.
    #[test]
    fn merge_from_sums_every_field_into_its_own_target() {
        let mut total = distinct_stats(0);
        total.merge_from(&distinct_stats(100));
        assert_eq!(total.elements_decoded, 1 + 101);
        assert_eq!(total.fields_emitted, 2 + 102);
        assert_eq!(total.truncations, 3 + 103);
        assert_eq!(total.errors, 4 + 104);
        assert_eq!(total.unconsumed_nested_bits, 5 + 105);
        assert_eq!(total.unconsumed_root_bits, 6 + 106);
        assert_eq!(total.implicit_terminations, 7 + 107);
    }

    /// `is_clean` gates whether a measured route may emit typed children from
    /// a walk. Each of the five failure counters must refuse on its own; the
    /// two work counters must not.
    #[test]
    fn is_clean_refuses_each_failure_counter_alone_and_ignores_work_counters() {
        let work_only = ArrayDecodeStats {
            elements_decoded: 3,
            fields_emitted: 9,
            ..ArrayDecodeStats::default()
        };
        assert!(work_only.is_clean(), "work counters are not failures");
        assert!(ArrayDecodeStats::default().is_clean());

        let one_failure_each = [
            (
                "truncations",
                ArrayDecodeStats {
                    truncations: 1,
                    ..work_only.clone()
                },
            ),
            (
                "errors",
                ArrayDecodeStats {
                    errors: 1,
                    ..work_only.clone()
                },
            ),
            (
                "unconsumed_nested_bits",
                ArrayDecodeStats {
                    unconsumed_nested_bits: 1,
                    ..work_only.clone()
                },
            ),
            (
                "unconsumed_root_bits",
                ArrayDecodeStats {
                    unconsumed_root_bits: 1,
                    ..work_only.clone()
                },
            ),
            (
                "implicit_terminations",
                ArrayDecodeStats {
                    implicit_terminations: 1,
                    ..work_only.clone()
                },
            ),
        ];
        for (name, stats) in one_failure_each {
            assert!(!stats.is_clean(), "{name} alone must make a walk unclean");
        }
    }

    /// The life-change array walks into its four members, on actual payloads
    /// from a 13.02 replay. The local handles differ per RPC (three schemas),
    /// and `MulticastNotifyHeal` names its parameter `LifeChangeBySection`, so a
    /// dispatch on the array's name alone would miss two of the five functions.
    #[test]
    fn the_life_change_array_walks_into_its_members() {
        // MulticastNotifyDamage_Point.LifeChangeEvents, one element.
        let damage: Vec<u8> = vec![
            0x02, 0x02, 0x16, 0x20, 0xE5, 0x3E, 0x18, 0x40, 0x00, 0x80, 0x0E, 0x44, 0x1A, 0x40,
            0x00, 0x00, 0xF0, 0xC1, 0x1C, 0x02, 0x01, 0x00, 0x00,
        ];
        // MulticastNotifyHeal.LifeChangeBySection, one element.
        let heal: Vec<u8> = vec![
            0x02, 0x02, 0x06, 0x20, 0x29, 0x14, 0x08, 0x40, 0xFF, 0x0F, 0x08, 0x42, 0x0A, 0x40,
            0x00, 0x00, 0x80, 0x3F, 0x0C, 0x02, 0x01, 0x00, 0x00,
        ];

        for (label, raw, bits, schema, want) in [
            (
                "damage",
                &damage,
                177u32,
                &LIFE_CHANGE_DAMAGE_SCHEMA,
                [
                    "ChangedComponent",
                    "LifeResult",
                    "DeltaLife",
                    "bAliveAfterChange",
                ],
            ),
            (
                "heal",
                &heal,
                177,
                &LIFE_CHANGE_BY_SECTION_SCHEMA,
                [
                    "ChangedComponent",
                    "LifeResult",
                    "DeltaLife",
                    "bAliveAfterChange",
                ],
            ),
        ] {
            let mut stats = ArrayDecodeStats::default();
            let fields = decode_struct_array(raw, bits, Some(schema), &[], &mut stats);
            assert_eq!(stats.errors, 0, "{label} decoded with errors");
            assert_eq!(fields.len(), 4, "{label}: {fields:?}");
            for (field, name) in fields.iter().zip(want) {
                assert!(
                    field.path.ends_with(name),
                    "{label}: {} vs {name}",
                    field.path
                );
            }
        }

        // The section schema numbers from 1, so it must not be interchangeable.
        let mut stats = ArrayDecodeStats::default();
        let wrong = decode_struct_array(
            &damage,
            177,
            Some(&LIFE_CHANGE_SECTION_SCHEMA),
            &[],
            &mut stats,
        );
        assert!(
            wrong.iter().all(|f| !f.path.ends_with("LifeResult")),
            "the wrong schema must not happen to name the members: {wrong:?}"
        );
    }

    #[test]
    fn release_1310_array_layouts_walk_observed_payloads() {
        // Captured from the 13.10 replay. This round contains a dealt
        // RegionalDamageInteractions element with 32-bit Hits and Damage.
        let round = crate::test_bits::hex(
            "04040a310802021040a8159c4216610606062040a8159c4226400000c04228400400000036d10202024240a8159c425c0102020260400400000062400000c04200000000000000000000",
        );
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &round,
            592,
            Some(&COMBAT_ROUNDS_SCHEMA_1310),
            &[],
            &mut stats,
        );
        assert!(stats.is_clean(), "{stats:?}");
        assert!(
            fields.iter().any(
                |f| f.path.ends_with("RegionalDamageInteractions[0]._h47") && f.bit_count == 32
            ),
            "{fields:?}"
        );
        assert!(
            fields.iter().any(
                |f| f.path.ends_with("RegionalDamageInteractions[0]._h48") && f.bit_count == 32
            )
        );

        // A damage event contains the shifted 16-bit ChangedComponent
        // NetGUID, followed by the float, float and bool members.
        let life = crate::test_bits::hex("0202182029481a40000000001c40000080bf1e02000000");
        let mut stats = ArrayDecodeStats::default();
        let fields = decode_struct_array(
            &life,
            177,
            Some(&LIFE_CHANGE_DAMAGE_SCHEMA_1310),
            &[],
            &mut stats,
        );
        assert!(stats.is_clean(), "{stats:?}");
        assert_eq!(
            crate::decode_field(crate::FieldType::ObjectNetGuid, &fields[0].raw_bits, 16)
                .unwrap()
                .into_columns()
                .0,
            Some(4628),
        );
        assert_eq!(
            fields
                .iter()
                .map(|f| (f.path.as_str(), f.bit_count))
                .collect::<Vec<_>>(),
            vec![
                ("[0].ChangedComponent", 16),
                ("[0].LifeResult", 32),
                ("[0].DeltaLife", 32),
                ("[0].bAliveAfterChange", 1),
            ]
        );
    }
}
