//! The input structs the writers consume. They carry no Arrow or Parquet
//! types, so they compile with `parquet` off and a consumer such as `vrfkit
//! validate` gets records without arrow, parquet or zstd in its build.
//!
//! The two name columns are `Arc<str>`, interned by the producer: 475 distinct
//! `group_path` values cover a replay, so a row costs a refcount, not an
//! allocation (docs/PERFORMANCE_NOTES.md#name-interning).

use smallvec::SmallVec;
use std::sync::Arc;

/// Reserved `field_name` for a whole ClassNetCache block whose function table
/// was unresolved; such a row is not a field or RPC. Tell it apart with
/// `field_name == UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME` alone: the
/// handle is no discriminator, since array-truncation rows may use `u32::MAX`.
pub const UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME: &str =
    "__vrfkit_unresolved_class_net_cache_payload__";

/// A single fields-table record ready for export: one decoded field, or the
/// reserved whole-block record of an unresolved ClassNetCache payload, which
/// carries no typed value. The address fields are never optional; the value
/// overlay is, because a field of unknown type carries only raw bits.
#[derive(Debug, Clone)]
pub struct FieldRecord {
    pub time_ms: u32,
    pub packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    /// Subobject this block described; `None` means the actor itself. Distinct
    /// from `Some(0)`, the engine's invalid-GUID sentinel, and from
    /// `actor_net_guid`, because a character's subobjects (inventory item
    /// slots, notably) must not be merged.
    pub object_net_guid: Option<u32>,
    /// Interned: see the module docs.
    pub group_path: Arc<str>,
    pub handle: u32,
    /// `None` when the field name is unknown (unmapped export index).
    /// Interned when present: see the module docs.
    pub field_name: Option<Arc<str>>,
    /// The `compatible_checksum` the replay declares for this handle: Unreal
    /// hashes the property's name and *type* into it, so it is a build-stable
    /// address and the overlay's last-resort type lookup. **`None` means the
    /// replay declares none**: only rows resolved through a
    /// `NetFieldExportGroup` carry one, not array leaves or struct blobs.
    pub compatible_checksum: Option<u32>,
    pub bit_count: u32,
    /// Raw bit payload; `None` for zero-bit fields. Inline up to 16 bytes, not
    /// interned or arena-held:
    /// docs/PERFORMANCE_NOTES.md#raw_bits-smallvec-and-the-rejected-arena.
    pub raw_bits: Option<SmallVec<[u8; 16]>>,
    pub value_i64: Option<i64>,
    pub value_f64: Option<f64>,
    pub value_bool: Option<bool>,
    pub value_str: Option<String>,
}

/// A single ability cast, reconstructed from the `AbilityCastsThisRound` array
/// snapshots on a `PlayerState` (why: `crate::ability`). The array is reused
/// every round, so a consumer reading only its final value sees the last round
/// alone; this record is one entry of the chronological delta.
#[derive(Debug, Clone, Copy)]
pub struct AbilityCastRecord {
    /// `time_ms` of the packet that completed this cast's identity.
    pub time_ms: u32,
    pub packet_id: u32,
    pub round: u32,
    pub actor_net_guid: u32,
    /// Index within the replicated array; reused across rounds, so it is not
    /// an identity on its own.
    pub cast_index: u32,
    /// Observed 3-6; the wire value, not a letter.
    pub slot: i64,
    /// Seconds since the round's `RoundStart`.
    pub cast_time: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A single movement sample ready for export: one decoded move, nothing
/// merged. Every field is set; a variant-0 move carries no velocity and gets
/// 0.0 (none in the 157,457,629 measured, `vrf_movement` crate docs).
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
    /// Move-header byte at bits [9..17], 0 on every corpus row and exported
    /// anyway. Posture is `bCrouchHeld` on the character actor, or the ~19 cm
    /// step in `pos_z`.
    pub movement_state: u8,
    /// 0 = variant 0, 1 = variant 1.
    pub move_type: u8,
}

/// Transport and RPC identity for a movement sample delivered by a reader.
///
/// These values describe the actual decode path, not a guessed association
/// based on movement values or timestamp proximity. Packet IDs are local to
/// one replication pass; a checkpoint is a separate pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementSource {
    /// Initial fragment's packet, or this packet for a non-partial bunch.
    pub initial_packet_id: u32,
    /// Packet that completed the bunch; equals [`MovementRecord::packet_id`].
    pub completion_packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    /// `None` for an actor block. `Some(0)` remains a distinct subobject GUID.
    pub object_net_guid: Option<u32>,
    /// Zero-based global index of the completing bunch, including fragments.
    pub bunch_index: u64,
    /// Zero-based index within the whole, possibly reassembled bunch.
    pub block_index: u32,
    /// Zero-based RPC callback ordinal in the completion packet, across all
    /// channels and RPC names; it is not an index into the movement rows.
    pub rpc_index: u32,
    pub rpc_handle: u32,
}

/// Companion identity for one [`MovementRecord`], in the same row order.
///
/// The ordinary movement Parquet table does not serialize these extra fields.
/// Reader callbacks can retain them without changing that table's contract.
/// Occurrence ordinals distinguish repeated arrays, sparse indices and
/// component properties. They do not assert that an emitted prefix is the
/// final value after later empty, failed or replacing properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementProvenance {
    /// Absent if a consumer decodes without framed transport context, or if
    /// that context carries a negative packet ID.
    pub source: Option<MovementSource>,
    /// Zero-based RemoteCharacterUpdates property occurrence in this RPC.
    pub array_ordinal: u32,
    /// Zero-based arrival ordinal within this array, not its sparse index.
    pub update_ordinal: u32,
    /// Sparse array index decoded from the wire.
    pub update_index: u32,
    /// Zero-based ComponentDataStream property occurrence in this update.
    pub component_ordinal: u32,
    /// Zero-based decoded move ordinal in this component, without marker wrap.
    pub move_index: u32,
}

/// A single actor lifecycle record ready for export. The paths are not
/// interned: ~3,800 rows a match, under 0.1 MB to save.
#[derive(Debug, Clone, Default)]
pub struct ActorRecord {
    pub time_ms: u32,
    pub packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    /// "open", "close", or "dormant". Only "close" is a despawn: read as one,
    /// dormancy cuts short a persistent ability's life and double-counts its
    /// re-open as a second spawn.
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
    /// Spawn velocity in world units per second (only for dynamic actor
    /// opens; zero when the wire sent none, as the engine reads it).
    pub spawn_vx: Option<f32>,
    pub spawn_vy: Option<f32>,
    pub spawn_vz: Option<f32>,
}

/// A single NetGUID registration ready for export.
#[derive(Debug, Clone)]
pub struct NetGuidRecord {
    pub net_guid: u32,
    /// Object path as the replay declared it.
    pub path: String,
    /// Containing object's GUID. `None` when the replay declared no outer;
    /// never coerced to 0, which is the engine's invalid-GUID sentinel.
    pub outer_net_guid: Option<u32>,
}

/// Identity shared by rows decoded from one checkpoint chunk. The wire id need
/// not be unique: join checkpoint tables on it together with the zero-based
/// chunk index.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CheckpointIdentity {
    pub checkpoint_index: u32,
    pub checkpoint_id: Arc<str>,
}

impl CheckpointIdentity {
    /// The identity of the zero-based `checkpoint_index`-th checkpoint chunk,
    /// whose wire id is `checkpoint_id`.
    #[must_use]
    pub fn new(checkpoint_index: u32, checkpoint_id: impl Into<Arc<str>>) -> Self {
        Self {
            checkpoint_index,
            checkpoint_id: checkpoint_id.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointFieldRecord {
    pub checkpoint: CheckpointIdentity,
    pub field: FieldRecord,
}

#[derive(Debug, Clone)]
pub struct CheckpointActorRecord {
    pub checkpoint: CheckpointIdentity,
    pub actor: ActorRecord,
}

#[derive(Debug, Clone)]
pub struct CheckpointNetGuidRecord {
    pub checkpoint: CheckpointIdentity,
    pub net_guid: NetGuidRecord,
}

#[derive(Debug, Clone)]
pub struct CheckpointGuidEntryRecord {
    pub checkpoint: CheckpointIdentity,
    pub ordinal: u32,
    pub net_guid: u32,
    pub outer_net_guid: u32,
    pub path_is_string: bool,
    pub literal_path: Option<String>,
    pub name_index: Option<u32>,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub struct CheckpointExportGroupRecord {
    pub checkpoint: CheckpointIdentity,
    pub ordinal: u32,
    pub path_name_index: u32,
    pub group_path: String,
    pub declared_slots: u32,
}

#[derive(Debug, Clone)]
pub struct CheckpointExportFieldRecord {
    pub checkpoint: CheckpointIdentity,
    pub group_ordinal: u32,
    pub path_name_index: u32,
    pub slot: u32,
    pub handle: u32,
    pub compatible_checksum: u32,
    pub rendered_name: String,
    pub exported_flag: u8,
    pub fname_kind: u8,
    pub fname_base: Option<String>,
    pub fname_index: Option<u32>,
    pub fname_number: Option<i32>,
}

/// One checkpoint content block and the field rows emitted while walking it.
#[derive(Debug, Clone)]
pub struct CheckpointBlockRecord {
    pub checkpoint: CheckpointIdentity,
    pub block_index: u32,
    pub time_ms: u32,
    pub packet_id: u32,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    pub object_net_guid: Option<u32>,
    pub class_net_guid: Option<u32>,
    /// Effective outer from the parsed header: set for every recognized block,
    /// nullable for a header form without one. `Some(0)` is the invalid GUID.
    pub outer_net_guid: Option<u32>,
    pub has_rep_layout: bool,
    pub is_actor: bool,
    pub is_deleted: bool,
    pub is_stably_named: bool,
    pub delete_flags: u8,
    pub resolved_group_path: Arc<str>,
    pub group_resolution_source: &'static str,
    pub group_declared: bool,
    pub resolution_memo_hit: bool,
    pub function_count: u32,
    pub function_count_source: &'static str,
    pub actor_archetype_path: Option<String>,
    pub actor_archetype_outer_path: Option<String>,
    pub actor_guid_path: Option<String>,
    pub class_guid_path: Option<String>,
    pub object_guid_path: Option<String>,
    pub object_outer_path: Option<String>,
    pub field_row_start: u64,
    pub field_row_count: u32,
}

/// A single Event chunk ready for export.
#[derive(Debug, Clone)]
pub struct EventRecord {
    /// Server-assigned event id, as the wire gives it.
    pub id: String,
    /// Event group, e.g. `characterDeath`.
    pub group: String,
    /// Free-form metadata; empty is a value, not a missing one.
    pub metadata: String,
    /// First timestamp in milliseconds.
    pub time1: u32,
    /// Second timestamp in milliseconds.
    pub time2: u32,
    /// Declared payload size from the chunk header (SizeInBytes).
    pub payload_size: i32,
    /// The payload verbatim. Undecoded on purpose.
    pub raw_payload: Vec<u8>,
    /// First payload word (after the u32 group tag). `None` when the group
    /// carries none (spike events), is unknown, or the payload is too short.
    /// Layout: `vrf_container::EventChunk`.
    pub word0: Option<u32>,
    /// Second payload word. `None` unless the group carries two
    /// (characterDeath: killer then killed NetGUID).
    pub word1: Option<u32>,
    /// Leading u32 group tag from a structurally validated inner payload.
    pub payload_tag: Option<u32>,
    /// FString following the group-dependent words. This is the wire string,
    /// not an inferred event label.
    pub payload_name: Option<String>,
    /// Trailing f32 seconds value from a structurally validated inner payload.
    pub payload_seconds: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct PartialRecord {
    pub source: &'static str,
    pub checkpoint_id: Option<String>,
    pub payload_kind: &'static str,
    pub reason: &'static str,
    pub source_packet_id: i32,
    pub source_payload_bit_offset: i64,
    pub rejection_packet_id: Option<i32>,
    pub channel_index: u32,
    pub channel_sequence: i32,
    pub open: bool,
    pub close: bool,
    pub dormant: bool,
    pub replication_paused: bool,
    pub reliable: bool,
    pub partial: bool,
    pub partial_initial: bool,
    pub partial_final: bool,
    pub has_package_map_exports: bool,
    pub has_must_be_mapped_guids: bool,
    pub close_reason: u8,
    pub source_payload_bit_count: i32,
    pub bit_count: u64,
    pub raw_bits: Vec<u8>,
}
