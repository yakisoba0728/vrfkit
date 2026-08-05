//! The input structs the writers consume.
//!
//! These carry no Arrow or Parquet types, so they compile with the `parquet`
//! feature off. That is what lets a consumer -- `vrfkit validate` is one --
//! drive the decode pipeline and receive records without pulling arrow,
//! parquet, zstd and their transitive graph into the build.
//!
//! # Why the string columns are `Arc<str>`
//!
//! `FieldRecord` is produced 1,246,812 times on the reference replay, and the
//! writer buffers 131,072 of them before flushing a row group. With `String`
//! that was up to three heap allocations per row and ~393,000 live allocations
//! at the peak. There are only 475 distinct `group_path` values in the whole
//! replay and a few thousand distinct field names, so an `Arc<str>` the
//! producer interns once and clones per row replaces the allocation with a
//! refcount increment.
//!
//! Arrow is unaffected: the dictionary builders are fed `&str` either way (see
//! `tables::fields`), so the value sequence handed to the encoder -- and
//! therefore the bytes on disk -- is identical. All 11 Parquet outputs of the
//! reference replay are byte-for-byte what they were before interning.

use smallvec::SmallVec;
use std::sync::Arc;

/// Reserved `field_name` for a whole ClassNetCache block whose function table
/// was unresolved.
///
/// Such a row is not a field or RPC. Consumers distinguish it with the single
/// predicate `field_name == UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME`.
/// The handle is not a discriminator because ordinary array-truncation rows
/// may also use `u32::MAX`.
pub const UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME: &str =
    "__vrfkit_unresolved_class_net_cache_payload__";

/// A single fields-table record ready for export.
///
/// Most records represent one decoded field. The reserved whole-block record
/// represents an unresolved ClassNetCache payload and carries no typed value.
///
/// The caller constructs these from the decoded replay stream. All "address"
/// fields are non-optional; the value overlay fields are `Option` because a
/// field may carry only raw bits (unknown type) or may carry a typed value.
#[derive(Debug, Clone)]
pub struct FieldRecord {
    pub time_ms: u32,
    pub packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    /// Subobject this block described, when it was not the actor itself.
    ///
    /// `None` means the block described the actor. Kept distinct from `Some(0)`
    /// because 0 is the engine's invalid-GUID sentinel, and distinct from
    /// `actor_net_guid` because a character replicates several subobjects
    /// (inventory item slots being the case that matters) whose state must not
    /// be merged.
    pub object_net_guid: Option<u32>,
    /// Interned: see the module docs.
    pub group_path: Arc<str>,
    pub handle: u32,
    /// `None` when the field name is unknown (unmapped export index).
    /// Interned when present: see the module docs.
    pub field_name: Option<Arc<str>>,
    pub bit_count: u32,
    /// Raw bit payload; `None` for zero-bit fields.
    ///
    /// Inlined as `SmallVec<[u8; 16]>`: most field payloads are <=16 bytes
    /// (u32/u64/FVector/FString-prefix), so the inline array eliminates the
    /// heap allocation on the ~1.25 M-row reference export. Larger payloads
    /// spill to the heap transparently -- SmallVec derefs to `&[u8]`, so the
    /// Arrow `BinaryArray` sees an identical byte sequence either way and the
    /// Parquet output is byte-for-byte unchanged.
    ///
    /// Not interned, and not an arena. Interning is the wrong shape: these are
    /// payload bytes rather than names, so the pool would approach one entry
    /// per row and buy nothing. An arena -- one shared buffer with per-row
    /// offsets -- would be sound, but it has to travel with the rows across the
    /// channel to the writer thread, which turns the batch type from
    /// `Vec<FieldRecord>` into a struct carrying a blob. The reason it was not
    /// taken is that the case for it shrank first: bounding the writer's buffer
    /// (see `writer::MAX_BUFFERED_ROWS`) cut the live payload vectors from
    /// ~390,000 to ~90,000, and `validate` -- which builds every record and
    /// writes no file -- brackets the whole remaining writer path at ~41 MB.
    pub raw_bits: Option<SmallVec<[u8; 16]>>,
    pub value_i64: Option<i64>,
    pub value_f64: Option<f64>,
    pub value_bool: Option<bool>,
    pub value_str: Option<String>,
}

/// A single movement sample ready for export.
///
/// All fields are non-optional (the replication channel always provides the
/// full state vector; partial updates are merged upstream before reaching us).
///
/// Field order mirrors `movement_schema()` exactly, including the three
/// trailing columns that were appended rather than interleaved.
///
/// `vrf_movement::MovementMove` also carries a `mode_flags` byte, which is not
/// mirrored here: its only construction site assigns it from the same local as
/// `movement_state`, so the two can never disagree and a `mode_flags` column
/// would be a byte-identical copy of `movement_state`.
#[derive(Debug, Clone, Copy)]
pub struct MovementRecord {
    pub time_ms: u32,
    pub packet_id: u32,
    pub character_net_guid: u32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub vel_x: f32,
    pub vel_y: f32,
    pub vel_z: f32,
    /// Server-assigned tick decoded from the move header.
    pub timestamp: u32,
    /// Posture byte (crouch / walk / run / jump).
    pub movement_state: u8,
    /// 0 = variant0 (velocity absent on the wire), 1 = variant1.
    pub move_type: u8,
}

/// A single actor lifecycle record ready for export.
///
/// The two path columns stay `Option<String>`: this table is ~3,800 rows on a
/// full match, so interning them would save under a tenth of a megabyte and
/// add a pool the producer would have to thread through two more call sites.
#[derive(Debug, Clone)]
pub struct ActorRecord {
    pub time_ms: u32,
    pub packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    /// "open" or "close".
    pub event: &'static str,
    /// Resolved class path; `None` when the GUID cache lacks the mapping.
    pub class_path: Option<String>,
    /// Archetype path; `None` for static actors or when unknown.
    pub archetype_path: Option<String>,
    /// Spawn location (only for dynamic actor opens).
    pub spawn_x: Option<f32>,
    pub spawn_y: Option<f32>,
    pub spawn_z: Option<f32>,
    /// Spawn rotation (only when present in the spawn data).
    pub spawn_pitch: Option<f32>,
    pub spawn_yaw: Option<f32>,
    pub spawn_roll: Option<f32>,
}

/// A single NetGUID registration ready for export.
#[derive(Debug, Clone)]
pub struct NetGuidRecord {
    /// The GUID itself.
    pub net_guid: u32,
    /// Object path as the replay declared it.
    pub path: String,
    /// Containing object's GUID. `None` when the replay declared no outer;
    /// never coerced to 0, which is the engine's invalid-GUID sentinel.
    pub outer_net_guid: Option<u32>,
}

/// A single Event chunk ready for export.
#[derive(Debug, Clone)]
pub struct EventRecord {
    /// Server-assigned event id, as the wire gives it.
    pub id: String,
    /// Event group, e.g. `characterDeath`.
    pub group: String,
    /// Free-form metadata. Empty is a real value, not a missing one.
    pub metadata: String,
    /// First timestamp in milliseconds.
    pub time1: u32,
    /// Second timestamp in milliseconds.
    pub time2: u32,
    /// Declared payload size from the chunk header.
    pub payload_size: i32,
    /// The payload verbatim. Undecoded on purpose.
    pub raw_payload: Vec<u8>,
    /// First payload word (after the u32 group tag), for groups that carry
    /// any. `None` when the group carries none (spike events), is unknown, or
    /// the payload is too short. See `vrf_container::EventChunk` for the
    /// payload layout.
    pub word0: Option<u32>,
    /// Second payload word. `None` unless the group carries two
    /// (characterDeath: killer then killed NetGUID).
    pub word1: Option<u32>,
}
