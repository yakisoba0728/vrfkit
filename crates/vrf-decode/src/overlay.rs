//! Overlay table: `(group_path, field_name)` -> [`FieldType`], plus decoding.
//! The entries live in `table.rs` as a sorted slice (`table_is_sorted` keeps
//! the reference binary search valid); this module adds the hash index
//! ([`index`], for the ~2M probes a replay makes) and the resolution order
//! ([`apply_overlay_with_handle`]).

mod index;
mod stats;

use std::sync::OnceLock;

use crate::decode::{DecodedValue, FieldType, decode_field};
use index::{OverlayIndex, handle_hash, handle_hash_from_group, name_hash, name_hash_from_group};

pub use index::{GroupHashState, group_hash_state};
pub use stats::{DecodeErrorKind, OverlayErrorReport, OverlayErrorRow, OverlayStats};

/// A single entry in the overlay table, which is sorted by
/// `(group_path, field_name)`.
#[derive(Debug, Clone, Copy)]
pub struct OverlayEntry {
    pub group_path: &'static str,
    pub field_name: &'static str,
    pub field_type: FieldType,
}

/// Maps a descriptor's explicit property handle back to its field name.
/// The type stays in [`OverlayEntry`], so type corrections apply to both
/// lookups alike.
#[derive(Debug, Clone, Copy)]
pub struct OverlayHandleEntry {
    pub group_path: &'static str,
    pub handle: u32,
    pub field_name: &'static str,
}

/// The overlay table: a static slice of entries plus a hash index built on the
/// first lookup, so construction stays a `const fn` usable in a `static` and
/// an unqueried table costs nothing.
#[derive(Debug, Clone)]
pub struct OverlayTable {
    entries: &'static [OverlayEntry],
    handle_entries: &'static [OverlayHandleEntry],
    index: OnceLock<OverlayIndex>,
}

impl OverlayTable {
    /// Create a table from a static slice.
    #[must_use]
    pub const fn new(entries: &'static [OverlayEntry]) -> Self {
        Self::with_handles(entries, &[])
    }

    /// Create a table with explicit-handle fallback metadata.
    #[must_use]
    pub const fn with_handles(
        entries: &'static [OverlayEntry],
        handle_entries: &'static [OverlayHandleEntry],
    ) -> Self {
        Self {
            entries,
            handle_entries,
            index: OnceLock::new(),
        }
    }

    #[inline]
    fn index(&self) -> &OverlayIndex {
        self.index
            .get_or_init(|| OverlayIndex::build(self.entries, self.handle_entries))
    }

    /// Look up the field type for a `(group_path, field_name)` pair.
    #[must_use]
    pub fn lookup(&self, group_path: &str, field_name: &str) -> Option<FieldType> {
        self.lookup_hashed(name_hash(group_path, field_name), group_path, field_name)
    }

    /// [`Self::lookup`] with the key hash already computed, which the
    /// `b`-prefix fallback reuses. The name is compared before the path: names
    /// are short and almost always differ, paths share long prefixes.
    #[inline]
    fn lookup_hashed(&self, hash: u64, group_path: &str, field_name: &str) -> Option<FieldType> {
        let e = self.entries;
        let hit = |i: usize| e[i].field_name == field_name && e[i].group_path == group_path;
        let i = self.index().by_name.find(hash, hit)?;
        Some(e[i].field_type)
    }

    /// Look up the `b`-prefixed spelling of `field_name` without building the
    /// prefixed key (see the `index` docs). Test-only: the export path already
    /// holds the hash.
    #[cfg(test)]
    pub(crate) fn lookup_b_prefixed(
        &self,
        group_path: &str,
        field_name: &str,
    ) -> Option<FieldType> {
        self.lookup_b_prefixed_hashed(name_hash(group_path, field_name), group_path, field_name)
    }

    /// The `b`-prefixed lookup under the hash of the UNprefixed key.
    #[inline]
    fn lookup_b_prefixed_hashed(
        &self,
        hash: u64,
        group_path: &str,
        field_name: &str,
    ) -> Option<FieldType> {
        let e = self.entries;
        let hit = |i: usize| {
            e[i].field_name.strip_prefix('b') == Some(field_name) && e[i].group_path == group_path
        };
        let i = self.index().by_stripped_name.find(hash, hit)?;
        Some(e[i].field_type)
    }

    /// Look up the descriptor field name for an explicit property handle.
    #[must_use]
    pub fn lookup_handle(&self, group_path: &str, handle: u32) -> Option<&'static str> {
        self.lookup_handle_hashed(handle_hash(group_path, handle), group_path, handle)
    }

    /// [`Self::lookup_handle`] with the key hash already computed from the
    /// group hash state the export path holds.
    #[inline]
    fn lookup_handle_hashed(
        &self,
        hash: u64,
        group_path: &str,
        handle: u32,
    ) -> Option<&'static str> {
        let e = self.handle_entries;
        let hit = |i: usize| e[i].handle == handle && e[i].group_path == group_path;
        let i = self.index().by_handle.find(hash, hit)?;
        Some(e[i].field_name)
    }

    /// Number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Binary search over the sorted slice: the reference the index's identity
    /// tests compare against (`table_is_sorted` keeps it correct).
    #[cfg(test)]
    pub(crate) fn lookup_by_binary_search(
        &self,
        group_path: &str,
        field_name: &str,
    ) -> Option<FieldType> {
        let idx = self
            .entries
            .binary_search_by(|entry| {
                entry
                    .group_path
                    .cmp(group_path)
                    .then_with(|| entry.field_name.cmp(field_name))
            })
            .ok()?;
        Some(self.entries[idx].field_type)
    }

    /// Handle-table counterpart of [`Self::lookup_by_binary_search`].
    #[cfg(test)]
    pub(crate) fn lookup_handle_by_binary_search(
        &self,
        group_path: &str,
        handle: u32,
    ) -> Option<&'static str> {
        let idx = self
            .handle_entries
            .binary_search_by(|entry| {
                entry
                    .group_path
                    .cmp(group_path)
                    .then_with(|| entry.handle.cmp(&handle))
            })
            .ok()?;
        Some(self.handle_entries[idx].field_name)
    }

    /// The `b`-prefix fallback as a plain lookup of the concatenated key.
    #[cfg(test)]
    pub(crate) fn lookup_b_prefixed_by_binary_search(
        &self,
        group_path: &str,
        field_name: &str,
    ) -> Option<FieldType> {
        self.lookup_by_binary_search(group_path, &format!("b{field_name}"))
    }
}

/// Result of applying the overlay to a single field.
pub struct OverlayResult {
    pub value_i64: Option<i64>,
    pub value_f64: Option<f64>,
    pub value_bool: Option<bool>,
    pub value_str: Option<String>,
}

impl OverlayResult {
    /// The all-null result: a decode was attempted and did not produce a value.
    /// The caller keeps `raw_bits`; nothing is ever fabricated in its place.
    const NONE: Self = Self {
        value_i64: None,
        value_f64: None,
        value_bool: None,
        value_str: None,
    };

    fn from_decoded(value: DecodedValue) -> Self {
        let (value_i64, value_f64, value_bool, value_str) = value.into_columns();
        Self {
            value_i64,
            value_f64,
            value_bool,
            value_str,
        }
    }

    /// The `(value_i64, value_f64, value_bool, value_str)` columns.
    #[must_use]
    pub fn into_columns(self) -> (Option<i64>, Option<f64>, Option<bool>, Option<String>) {
        (
            self.value_i64,
            self.value_f64,
            self.value_bool,
            self.value_str,
        )
    }
}

/// Apply the type overlay to a single field.
///
/// `group_state` is the cacheable half of the key hash -- see [`group_hash_state`].
/// A caller probing many fields against the same group path computes it once and
/// reuses it; a one-off caller passes [`group_hash_state`]`(group_path)`.
///
/// Returns `None` if the field has no type mapping or is Raw/Skip.
/// Returns `Some(OverlayResult)` on success (values filled) or on decode
/// failure (all None -- the raw_bits remain).
pub fn apply_overlay(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    raw_bits: Option<&[u8]>,
    bit_count: u32,
    stats: &mut OverlayStats,
) -> Option<OverlayResult> {
    apply_overlay_inner(
        table,
        group_path,
        group_state,
        field_name,
        None,
        None,
        raw_bits,
        bit_count,
        stats,
    )
}

/// Apply the overlay with the replay field's handle available as a fallback.
///
/// `group_state` is the cacheable half of the key hash -- see [`group_hash_state`].
/// Direct name lookup, including the narrow Unreal `b`-prefix fallback, stays
/// authoritative. The explicit handle is consulted only when both miss.
#[allow(clippy::too_many_arguments)]
pub fn apply_overlay_with_handle(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: u32,
    raw_bits: Option<&[u8]>,
    bit_count: u32,
    stats: &mut OverlayStats,
) -> Option<OverlayResult> {
    apply_overlay_inner(
        table,
        group_path,
        group_state,
        field_name,
        Some(handle),
        None,
        raw_bits,
        bit_count,
        stats,
    )
}

/// [`apply_overlay_with_handle`] with the field's `compatible_checksum`, which
/// the schema walk that finds the name finds on the same `NetFieldExport`.
#[allow(clippy::too_many_arguments)]
pub fn apply_overlay_with_checksum(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: u32,
    checksum: Option<u32>,
    raw_bits: Option<&[u8]>,
    bit_count: u32,
    stats: &mut OverlayStats,
) -> Option<OverlayResult> {
    apply_overlay_inner(
        table,
        group_path,
        group_state,
        field_name,
        Some(handle),
        checksum,
        raw_bits,
        bit_count,
        stats,
    )
}

/// The full resolution order a replicated field gets, for callers that hold a
/// name and a handle but not a whole row -- published once so an array leaf is
/// typed exactly as the same field outside an array.
#[must_use]
pub fn resolve_field_type(
    table: &OverlayTable,
    group_path: &str,
    field_name: Option<&str>,
    handle: Option<u32>,
) -> Option<FieldType> {
    resolve_field_type_with_checksum(table, group_path, field_name, handle, None)
}

/// [`resolve_field_type`] with the field's `compatible_checksum` in hand.
///
/// The checksum is the last resort in the order, so passing it can only resolve
/// fields that would otherwise have stayed raw.
#[must_use]
pub fn resolve_field_type_with_checksum(
    table: &OverlayTable,
    group_path: &str,
    field_name: Option<&str>,
    handle: Option<u32>,
    checksum: Option<u32>,
) -> Option<FieldType> {
    // No stats here: the refusal still applies, it is just not tallied.
    let mut refused = false;
    resolve_entry(
        table,
        group_path,
        group_hash_state(group_path),
        field_name,
        handle,
        checksum,
        &mut refused,
    )
    .map(|(field_type, _)| field_type)
}

/// Game-mode sibling classes carrying the SAME property set under another class
/// name (Swiftplay's `Swiftplay_EoRCredits_*_C` for `Bomb*_C`), mapped to the
/// class the table is keyed on: an alias, not duplicated entries that would
/// drift. Sound while same-name properties have equal widths on both classes,
/// which `check_decode_errors_corpus.py` tests on the Swiftplay replays.
/// `_ClassNetCache` and `<Class>:<Function>` forms are not aliased: the table
/// has no Bomb entries for them.
const GROUP_ALIASES: &[(&str, &str)] = &[
    (
        "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_GameState.Swiftplay_EoRCredits_GameState_C",
        "/Game/GameModes/Bomb/BombGameState.BombGameState_C",
    ),
    (
        "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C",
        "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
    ),
];

/// The class whose table entries `group_path` should fall back to, if any.
#[must_use]
fn alias_group(group_path: &str) -> Option<&'static str> {
    GROUP_ALIASES
        .iter()
        .find(|(from, _)| *from == group_path)
        .map(|(_, to)| *to)
}

/// `group_path` rewritten to the class the table and the struct-blob decoders
/// are keyed on, or unchanged. Published so `vrfkit`'s struct-blob dispatch
/// (which gates `RoundResults` and `TeamEconomy` on `BombGameState`) decides
/// "is this a game state" from this one table, Swiftplay included.
#[must_use]
pub fn canonical_group(group_path: &str) -> &str {
    alias_group(group_path).unwrap_or(group_path)
}

/// `AActor` / `USceneComponent` object references Unreal replicates on every
/// actor, always as a NetGUID; the descriptors declare them only for the
/// classes they cover (untyped on 203 group/field pairs of 02d4d478, one
/// match's lineup). The engine fixes the type, so this is a fallback by name,
/// not table rows, and it runs late, so a declared type wins.
const ENGINE_OBJECT_REFS: [&str; 4] = ["Owner", "Instigator", "AttachParent", "Controller"];

/// The type a `compatible_checksum` was learned to carry, or `None`. Unreal
/// hashes a property's type into its checksum with its name, so a checksum
/// identifies a property better than a name: across the RPC groups exactly one
/// pair of distinct names collides, while 38 of 211 parameter names carry more
/// than one checksum. `CHECKSUM_TYPES` is learned from this table's declared
/// fields and omits checksums whose donors disagreed.
#[must_use]
pub fn lookup_checksum(checksum: u32) -> Option<FieldType> {
    let index = crate::checksum_table::CHECKSUM_TYPES
        .binary_search_by_key(&checksum, |(key, _)| *key)
        .ok()?;
    Some(crate::checksum_table::CHECKSUM_TYPES[index].1)
}

/// The group's order, then the same order against the aliased class (the
/// WHOLE order, so an aliased field gets the same fallbacks), then scoped
/// types, engine object references and the checksum.
fn resolve_entry(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: Option<u32>,
    checksum: Option<u32>,
    refused: &mut bool,
) -> Option<(FieldType, DiagnosticName)> {
    if let Some(hit) = resolve_in_group(table, group_path, group_state, field_name, handle, refused)
    {
        return Some(hit);
    }
    // Not a `let` chain: those are stable only from 1.88 and the MSRV is 1.86.
    let aliased_hit = alias_group(group_path).and_then(|aliased| {
        resolve_in_group(
            table,
            aliased,
            group_hash_state(aliased),
            field_name,
            handle,
            refused,
        )
    });
    if let Some(hit) = aliased_hit {
        return Some(hit);
    }
    let name = field_name?;
    // Some groups flatten distinct properties to one name (the byte-shaped B
    // entries vs the 32-bit B field), so scoped types key on the whole
    // group/name/checksum tuple and are never donated or aliased.
    if let Some(checksum) = checksum {
        let scoped = &crate::scoped_types::SCOPED_TYPES;
        if let Ok(index) = scoped.binary_search_by_key(&(name, group_path, checksum), |entry| {
            (entry.0, entry.1, entry.2)
        }) {
            return Some((scoped[index].3, DiagnosticName::Wire));
        }
    }
    if ENGINE_OBJECT_REFS.contains(&name) {
        return Some((FieldType::ObjectNetGuid, DiagnosticName::Wire));
    }
    // Last on purpose: a type stated for the same property on another class is
    // weaker than anything declared for this one.
    lookup_checksum(checksum?).map(|field_type| (field_type, DiagnosticName::Wire))
}

/// Resolve which table entry a wire field belongs to within ONE group: name,
/// then b-prefix, then handle -- see docs/OVERLAY_RESOLUTION.md "The
/// b-prefix fallback" for why and the measured row counts.
fn resolve_in_group(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: Option<u32>,
    refused: &mut bool,
) -> Option<(FieldType, DiagnosticName)> {
    if let Some(name) = field_name {
        // One hash serves both probes; only the field name is folded here.
        let hash = name_hash_from_group(group_state, name);
        if let Some(field_type) = table
            .lookup_hashed(hash, group_path, name)
            .or_else(|| table.lookup_b_prefixed_hashed(hash, group_path, name))
        {
            return Some((field_type, DiagnosticName::Wire));
        }
    }

    let handle = handle?;
    let descriptor_name = table.lookup_handle_hashed(
        handle_hash_from_group(group_state, handle),
        group_path,
        handle,
    )?;
    // Fail closed when the wire names something ELSE at this handle: reading a
    // new property with the old one's type is the silent shape of a patch (the
    // width often fits, so no counter moves). A bare decimal (`"248"`, an
    // unresolved FName index) declares nothing and stays exempt, as
    // `a_bare_fname_index_still_reaches_the_handle_fallback` pins.
    if let Some(name) = field_name {
        if name != descriptor_name && !is_unresolved_fname_index(name) {
            *refused = true;
            return None;
        }
    }
    let field_type = table.lookup_hashed(
        name_hash_from_group(group_state, descriptor_name),
        group_path,
        descriptor_name,
    )?;
    // Report the DESCRIPTOR's name: the type came from it, and the only wire
    // name left here is a bare index like `"248"`.
    Some((field_type, DiagnosticName::Descriptor(descriptor_name)))
}

/// The name an error row reports: the wire's, or the handle descriptor's the
/// type came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticName {
    Wire,
    Descriptor(&'static str),
}

/// Everything the overlay decides about one field before reading its bits: a
/// pure function of the table, the group path, the field name, the handle and
/// the checksum, so a caller may keep it for those exact inputs and pass it to
/// [`apply_overlay_resolved`] instead of resolving the same field again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayResolution {
    entry: Option<(FieldType, DiagnosticName)>,
    refused: bool,
}

/// [`apply_overlay_with_checksum`]'s resolution step alone; see
/// [`OverlayResolution`].
#[must_use]
pub fn resolve_overlay(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: u32,
    checksum: Option<u32>,
) -> OverlayResolution {
    let mut refused = false;
    let entry = resolve_entry(
        table,
        group_path,
        group_state,
        field_name,
        Some(handle),
        checksum,
        &mut refused,
    );
    OverlayResolution { entry, refused }
}

/// Whether a declared field name is a bare decimal: an unresolved hardcoded
/// FName rendered as its index (`"215"`, `"216"`, `"248"`, `"249"` in this
/// corpus), which neither identifies a field nor contradicts a descriptor.
fn is_unresolved_fname_index(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

#[allow(clippy::too_many_arguments)]
fn apply_overlay_inner(
    table: &OverlayTable,
    group_path: &str,
    group_state: GroupHashState,
    field_name: Option<&str>,
    handle: Option<u32>,
    checksum: Option<u32>,
    raw_bits: Option<&[u8]>,
    bit_count: u32,
    stats: &mut OverlayStats,
) -> Option<OverlayResult> {
    let mut refused = false;
    let entry = resolve_entry(
        table,
        group_path,
        group_state,
        field_name,
        handle,
        checksum,
        &mut refused,
    );
    apply_overlay_resolved(
        OverlayResolution { entry, refused },
        group_path,
        field_name,
        raw_bits,
        bit_count,
        stats,
    )
}

/// [`apply_overlay_with_checksum`] from a resolution [`resolve_overlay`] made
/// for this same group path and field name: the tallies, the decode and the
/// error rows, exactly as the one-step call.
pub fn apply_overlay_resolved(
    resolution: OverlayResolution,
    group_path: &str,
    field_name: Option<&str>,
    raw_bits: Option<&[u8]>,
    bit_count: u32,
    stats: &mut OverlayStats,
) -> Option<OverlayResult> {
    let OverlayResolution { entry, refused } = resolution;
    let (field_type, diagnostic_name) = match entry {
        Some((field_type, DiagnosticName::Wire)) => (field_type, field_name.unwrap_or_default()),
        Some((field_type, DiagnosticName::Descriptor(name))) => (field_type, name),
        None => {
            if field_name.is_none() {
                stats.no_field_name += 1;
            } else {
                stats.not_in_table += 1;
            }
            // Counted BESIDE the bucket above: the overlay had a candidate and
            // declined it.
            if refused {
                stats.handle_conflicts_refused += 1;
            }
            return None;
        }
    };

    if matches!(field_type, FieldType::Raw | FieldType::Skip) {
        stats.raw_or_skip += 1;
        return None;
    }

    let data = match raw_bits {
        Some(d) if bit_count > 0 => d,
        // Zero bits is this type's value 0; `decode_enum_remaining_bits` says why.
        _ if bit_count == 0 && field_type == FieldType::EnumRemainingBits => &[],
        _ => {
            stats.decoded_err += 1;
            stats.error_report.record(
                group_path,
                diagnostic_name,
                field_type,
                bit_count,
                DecodeErrorKind::ZeroBits,
            );
            return Some(OverlayResult::NONE);
        }
    };

    match decode_field(field_type, data, bit_count) {
        Ok(value) => {
            stats.decoded_ok += 1;
            Some(OverlayResult::from_decoded(value))
        }
        Err(e) => {
            stats.decoded_err += 1;
            let kind = DecodeErrorKind::from_decode_error(&e);
            stats
                .error_report
                .record(group_path, diagnostic_name, field_type, bit_count, kind);
            Some(OverlayResult::NONE)
        }
    }
}
