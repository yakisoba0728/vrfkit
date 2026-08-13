//! Hash index over the overlay entry slices.
//!
//! # Why this exists
//!
//! Resolving the reference replay's 988,983 offered rows costs about 1.5
//! million `(group_path, field_name)` probes -- one per row, plus a second for
//! the `b`-prefixed spelling on each of the 511,881 that miss -- and another
//! ~0.5 million `(group_path, handle)` probes. Every one of those was a binary
//! search over a 1,255-entry table. That is ~10 comparisons, and every
//! comparison looks at `group_path` first -- paths like
//! `/Game/Characters/AggroBot/AggroBot_PC.AggroBot_PC_C` that share 20 to 40
//! leading bytes with their neighbours, so each comparison is a real memcmp
//! rather than a first-byte reject. Measured on the reference replay by running
//! the search twice and differencing, one lookup pass costs ~180 ms of a 1.58 s
//! export.
//!
//! This replaces it with open addressing on a 64-bit key hash. A lookup hashes
//! `group_path` and `field_name` once (8 bytes per multiply) and probes once;
//! the stored 32-bit tag rejects a non-matching slot without touching the
//! strings at all. Most lookups on a real replay MISS -- 511,881 of 988,983
//! offered rows are not in the table -- and a miss now ends at an empty slot
//! with zero string comparisons.
//!
//! # Answer identity
//!
//! The hash only chooses *which* entries to compare. Every candidate is still
//! confirmed by full string equality on both key halves before it is returned,
//! so a collision costs time and never an answer. `tests::overlay` walks all
//! 1,191 entries plus their `b`-stripped spellings plus synthetic misses and
//! asserts this index agrees with the binary search on every one.
//!
//! # The `b`-prefix table
//!
//! The overlay's boolean fallback asks for `b` + the wire's field name (see
//! [`super::apply_overlay_with_handle`] for why). Building that key allocated a
//! `String` on every miss -- over half a million per replay. Instead, every
//! entry whose name starts with `b` is *also* inserted under its stripped name,
//! so the fallback probe reuses the hash already computed for the direct probe
//! and never builds a key. Stripped keys are unique for the same reason direct
//! keys are: `(group_path, field_name)` is unique in the generated table, so
//! two entries in one group cannot both be `b` + X for the same X.

use super::{OverlayEntry, OverlayHandleEntry};

/// One open-addressing slot.
///
/// `entry` is the entry index plus one so that an all-zero slot means empty;
/// `tag` is the high half of the key hash and rejects a wrong slot before the
/// candidate's strings are ever read.
#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    tag: u32,
    entry: u32,
}

/// A power-of-two open-addressing table with linear probing.
///
/// Sized at >= 2x the entry count, so a probe chain always terminates on an
/// empty slot and no load-factor bookkeeping is needed: the contents are fixed
/// at build time and nothing is ever removed.
#[derive(Debug, Clone)]
struct SlotTable {
    slots: Box<[Slot]>,
    mask: usize,
}

impl SlotTable {
    fn new(entry_count: usize) -> Self {
        let capacity = entry_count.saturating_mul(2).max(8).next_power_of_two();
        Self {
            slots: vec![Slot::default(); capacity].into_boxed_slice(),
            mask: capacity - 1,
        }
    }

    fn insert(&mut self, hash: u64, entry_index: usize) {
        let mut position = (hash as usize) & self.mask;
        while self.slots[position].entry != 0 {
            position = (position + 1) & self.mask;
        }
        self.slots[position] = Slot {
            tag: (hash >> 32) as u32,
            // Entry counts are bounded by the generated table, so the `+ 1`
            // cannot wrap in any realistic input; `as u32` is the storage
            // width, not a truncation the lookup depends on.
            entry: (entry_index as u32) + 1,
        };
    }

    /// Return the first entry index whose slot tag matches and which `matches`
    /// confirms. `matches` is the authoritative comparison -- the tag is only a
    /// filter.
    #[inline]
    fn find(&self, hash: u64, mut matches: impl FnMut(usize) -> bool) -> Option<usize> {
        let tag = (hash >> 32) as u32;
        let mut position = (hash as usize) & self.mask;
        loop {
            let slot = self.slots[position];
            if slot.entry == 0 {
                return None;
            }
            if slot.tag == tag {
                let entry_index = (slot.entry - 1) as usize;
                if matches(entry_index) {
                    return Some(entry_index);
                }
            }
            position = (position + 1) & self.mask;
        }
    }
}

/// Golden-ratio seed and a 64-bit odd multiplier. Neither value is a secret --
/// the keys are a compiled-in table and a replay's declared names, and a
/// collision costs one extra string comparison, not a wrong answer -- so this
/// is a speed-first mixer, not a keyed hash.
const HASH_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
const HASH_MULT: u64 = 0x517C_C1B7_2722_0A95;

/// Fold a byte string into the running state, eight bytes per multiply.
///
/// The tail is zero-padded into a full word rather than mixed byte-by-byte;
/// the length folding in [`name_hash`] is what keeps that from making two
/// different keys collide systematically.
fn mix_bytes(mut state: u64, bytes: &[u8]) -> u64 {
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let mut word = [0u8; 8];
        word.copy_from_slice(chunk);
        state = (state ^ u64::from_le_bytes(word))
            .rotate_left(23)
            .wrapping_mul(HASH_MULT);
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        let mut word = [0u8; 8];
        word[..tail.len()].copy_from_slice(tail);
        state = (state ^ u64::from_le_bytes(word))
            .rotate_left(23)
            .wrapping_mul(HASH_MULT);
    }
    state
}

/// Final avalanche, so the high half used as the slot tag depends on every
/// input byte rather than only on the last word mixed.
fn finish(mut state: u64) -> u64 {
    state ^= state >> 33;
    state = state.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    state ^= state >> 33;
    state = state.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    state ^ (state >> 33)
}

/// Hash a `(group_path, field_name)` key.
///
/// Both lengths are folded in at the end. That is what separates `("ab", "c")`
/// from `("a", "bc")`, whose bytes concatenate identically, and what stops the
/// zero-padded tail in [`mix_bytes`] from equating keys of different length.
#[inline]
pub(super) fn name_hash(group_path: &str, field_name: &str) -> u64 {
    name_hash_from_group(group_hash_state(group_path), field_name)
}

/// Hash a `(group_path, handle)` key. Lives in its own table, so it only has to
/// separate handles from each other within a group.
#[inline]
pub(super) fn handle_hash(group_path: &str, handle: u32) -> u64 {
    handle_hash_from_group(group_hash_state(group_path), handle)
}

/// The fold state after mixing only the group path, plus its length.
///
/// A content block probes the overlay with the same `group_path` for every field
/// in it (~2M probes/replay), and 80% of blocks hit the group-path memo so the
/// path changes rarely. This struct is what the export sink caches: the
/// half-finished hash with the expensive -- long, prefix-sharing -- group path
/// already folded in, so each per-field probe only pays for the field name plus
/// the final avalanche.
///
/// Published because the sink lives in another crate; opaque because its fields
/// are an internal detail of the mix. A stale state turns overlay hits into
/// misses (fields degrade to `raw_bits`), never a wrong value -- the slot tag and
/// the full string equality check in `OverlayIndex::find_name` still guard
/// every hit.
#[derive(Debug, Clone, Copy)]
pub struct GroupHashState {
    /// State after [`mix_bytes`] on the group path, seeded with [`HASH_SEED`].
    state: u64,
    /// The group path's byte length, folded in at the finish step.
    group_len: u64,
}

/// Compute the cacheable half of a `(group_path, ...)` key hash.
///
/// This is the part the export sink amasses once per content block. Pair it with
/// `name_hash_from_group` or `handle_hash_from_group` to finish the key for a
/// single probe.
#[inline]
#[must_use]
pub fn group_hash_state(group_path: &str) -> GroupHashState {
    GroupHashState {
        state: mix_bytes(HASH_SEED, group_path.as_bytes()),
        group_len: group_path.len() as u64,
    }
}

/// Finish a `(group_path, field_name)` key hash from a cached group state.
///
/// Equivalent to [`name_hash`] when the group state was computed from the same
/// `group_path`. Both lengths are still folded in at the end -- the group length
/// lives in the cached state, the field length is mixed here.
#[inline]
pub(super) fn name_hash_from_group(group: GroupHashState, field_name: &str) -> u64 {
    let state = mix_bytes(group.state, field_name.as_bytes());
    finish(state ^ (group.group_len << 32) ^ (field_name.len() as u64))
}

/// Finish a `(group_path, handle)` key hash from a cached group state.
#[inline]
pub(super) fn handle_hash_from_group(group: GroupHashState, handle: u32) -> u64 {
    finish(group.state ^ (group.group_len << 32) ^ u64::from(handle))
}

/// The three probe tables an [`OverlayTable`](super::OverlayTable) needs.
///
/// Built once per table on first lookup. For the generated table that is
/// one direct-name insertion per overlay entry, additional stripped-name
/// aliases, and one insertion per handle entry -- paid once against millions
/// of lookups. The counts intentionally follow the generated slices rather
/// than being duplicated here.
#[derive(Debug, Clone)]
pub(super) struct OverlayIndex {
    by_name: SlotTable,
    by_stripped_name: SlotTable,
    by_handle: SlotTable,
}

impl OverlayIndex {
    pub(super) fn build(entries: &[OverlayEntry], handle_entries: &[OverlayHandleEntry]) -> Self {
        let stripped_count = entries
            .iter()
            .filter(|entry| entry.field_name.starts_with('b'))
            .count();

        let mut by_name = SlotTable::new(entries.len());
        let mut by_stripped_name = SlotTable::new(stripped_count);
        for (position, entry) in entries.iter().enumerate() {
            by_name.insert(name_hash(entry.group_path, entry.field_name), position);
            if let Some(stripped) = entry.field_name.strip_prefix('b') {
                by_stripped_name.insert(name_hash(entry.group_path, stripped), position);
            }
        }

        let mut by_handle = SlotTable::new(handle_entries.len());
        for (position, entry) in handle_entries.iter().enumerate() {
            by_handle.insert(handle_hash(entry.group_path, entry.handle), position);
        }

        Self {
            by_name,
            by_stripped_name,
            by_handle,
        }
    }

    /// Direct `(group_path, field_name)` lookup.
    ///
    /// `field_name` is compared before `group_path`: names are short and almost
    /// always differ, paths are long and share prefixes.
    #[inline]
    pub(super) fn find_name(
        &self,
        entries: &[OverlayEntry],
        hash: u64,
        group_path: &str,
        field_name: &str,
    ) -> Option<usize> {
        self.by_name.find(hash, |position| {
            let entry = &entries[position];
            entry.field_name == field_name && entry.group_path == group_path
        })
    }

    /// Lookup of the `b`-prefixed spelling of `field_name`, using the hash of
    /// the UNprefixed key -- see the module docs.
    #[inline]
    pub(super) fn find_b_prefixed_name(
        &self,
        entries: &[OverlayEntry],
        hash: u64,
        group_path: &str,
        field_name: &str,
    ) -> Option<usize> {
        self.by_stripped_name.find(hash, |position| {
            let entry = &entries[position];
            entry.field_name.strip_prefix('b') == Some(field_name) && entry.group_path == group_path
        })
    }

    #[inline]
    pub(super) fn find_handle(
        &self,
        handle_entries: &[OverlayHandleEntry],
        hash: u64,
        group_path: &str,
        handle: u32,
    ) -> Option<usize> {
        self.by_handle.find(hash, |position| {
            let entry = &handle_entries[position];
            entry.handle == handle && entry.group_path == group_path
        })
    }
}
