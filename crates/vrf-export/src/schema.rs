//! Arrow schema definitions for the two export tables.
//!
//! Schemas are defined once here so that writer and reader agree. The field
//! metadata (e.g. `PARQUET:field_id`) is intentionally omitted -- Parquet
//! assigns ordinal field IDs automatically, and manual IDs would only matter
//! if we needed Iceberg-style schema evolution, which we don't.

use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

/// Schema for the `fields` table (long format).
///
/// Most rows represent one decoded field. A whole ClassNetCache block whose
/// function table is unresolved is preserved as one explicitly marked row;
/// it is not split into fabricated fields. The schema itself stays unchanged.
///
/// Column ordering is deliberate: the "address" columns come first (time,
/// packet, channel, actor, group, handle, name) so that predicate pushdown on
/// actor or group benefits from row-group statistics without reading value
/// columns. The sparse value columns at the end compress to near-zero when
/// null.
pub fn fields_schema() -> Schema {
    Schema::new(vec![
        Field::new("time_ms", DataType::UInt32, false),
        Field::new("packet_id", DataType::UInt32, false),
        Field::new("channel_index", DataType::UInt32, false),
        Field::new("actor_net_guid", DataType::UInt32, false),
        // Nullable: only subobject blocks carry one, and null must stay
        // distinguishable from 0 (the engine's invalid-GUID sentinel).
        Field::new("object_net_guid", DataType::UInt32, true),
        // Dictionary<Int32, Utf8>: ~300 distinct group paths over 780k rows.
        Field::new(
            "group_path",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new("handle", DataType::UInt32, false),
        // Nullable because the field name may be unknown (unmapped export index).
        Field::new(
            "field_name",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
        Field::new("bit_count", DataType::UInt32, false),
        // Raw bit payload; nullable because zero-bit fields carry no data.
        Field::new("raw_bits", DataType::Binary, true),
        // Sparse typed-value overlay -- at most one of these is non-null per row.
        Field::new("value_i64", DataType::Int64, true),
        Field::new("value_f64", DataType::Float64, true),
        Field::new("value_bool", DataType::Boolean, true),
        Field::new("value_str", DataType::Utf8, true),
    ])
}

/// Schema for the `movement` table (fixed format: every row is identical
/// structure, no nulls).
///
/// The coordinate system matches Unreal Engine's left-handed Z-up convention.
/// Positions are in centimetres; yaw/pitch are in degrees (-180..180).
/// Velocity is cm/s as reported by the replication channel.
pub fn movement_schema() -> Schema {
    Schema::new(vec![
        Field::new("time_ms", DataType::UInt32, false),
        Field::new("packet_id", DataType::UInt32, false),
        Field::new("character_net_guid", DataType::UInt32, false),
        Field::new("pos_x", DataType::Float32, false),
        Field::new("pos_y", DataType::Float32, false),
        Field::new("pos_z", DataType::Float32, false),
        Field::new("yaw", DataType::Float32, false),
        Field::new("pitch", DataType::Float32, false),
        Field::new("vel_x", DataType::Float32, false),
        Field::new("vel_y", DataType::Float32, false),
        Field::new("vel_z", DataType::Float32, false),
    ])
}

/// Convenience: wrap a schema in an Arc (ArrowWriter expects `SchemaRef`).
pub fn fields_schema_ref() -> Arc<Schema> {
    Arc::new(fields_schema())
}

/// Convenience: wrap a schema in an Arc.
pub fn movement_schema_ref() -> Arc<Schema> {
    Arc::new(movement_schema())
}

/// Schema for the `actors` table (one row per channel open or close).
///
/// This table makes actors visible even if they never replicate a single
/// field -- e.g. weapon/ability instances, DefuserItem, HeavyArmorItem.
/// Without it, only actors that produce at least one field row in
/// `fields.parquet` can be resolved downstream.
///
/// Spawn location and rotation are nullable because static actors and
/// channel-close rows do not carry spatial data.
pub fn actors_schema() -> Schema {
    Schema::new(vec![
        Field::new("time_ms", DataType::UInt32, false),
        Field::new("packet_id", DataType::UInt32, false),
        Field::new("channel_index", DataType::UInt32, false),
        Field::new("actor_net_guid", DataType::UInt32, false),
        // "open" or "close" -- small cardinality, dictionary is overkill.
        Field::new("event", DataType::Utf8, false),
        // Nullable: class path may be unresolvable for some actors.
        Field::new(
            "class_path",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
        // Nullable: archetype path may be absent (static actors).
        Field::new(
            "archetype_path",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
        // Spawn location (nullable -- only present for dynamic actor opens).
        Field::new("spawn_x", DataType::Float32, true),
        Field::new("spawn_y", DataType::Float32, true),
        Field::new("spawn_z", DataType::Float32, true),
        // Spawn rotation (nullable).
        Field::new("spawn_pitch", DataType::Float32, true),
        Field::new("spawn_yaw", DataType::Float32, true),
        Field::new("spawn_roll", DataType::Float32, true),
    ])
}

/// Convenience: wrap actors schema in an Arc.
pub fn actors_schema_ref() -> Arc<Schema> {
    Arc::new(actors_schema())
}

/// Schema for the `net_guids` table (one row per registered NetGUID).
///
/// This is the replay's own object registry: which GUID maps to which object
/// path, and which object contains it. `actors.parquet` only covers GUIDs that
/// opened a channel, which excludes subobjects -- a weapon's `FiringState`
/// appears in no other table. Without the outer chain there is no route from a
/// shot event to the equippable that fired it.
///
/// `outer_net_guid` is nullable rather than zero-filled: GUID 0 is the engine's
/// "invalid" sentinel, so collapsing "no outer declared" onto 0 would erase the
/// distinction between an unknown parent and an explicitly invalid one.
pub fn net_guids_schema() -> Schema {
    Schema::new(vec![
        Field::new("net_guid", DataType::UInt32, false),
        // Paths repeat heavily (175 GUIDs share "FiringState" in one match).
        Field::new(
            "path",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new("outer_net_guid", DataType::UInt32, true),
    ])
}

/// Convenience: wrap net_guids schema in an Arc.
pub fn net_guids_schema_ref() -> Arc<Schema> {
    Arc::new(net_guids_schema())
}

/// Schema for the `events` table (one row per Event chunk).
///
/// Event chunks are the server's own labelled timeline -- the ground truth the
/// rest of the pipeline only reconstructs indirectly from RPCs. The six header
/// fields are decoded; the inner payload is not.
///
/// `raw_payload` is the whole payload verbatim. Its structure is observable but
/// not self-describing (see `vrf_container::EventChunk`), so decoding it into
/// named columns would mean inventing names for words whose meaning is not
/// established. A blob the caller can inspect is worth more than a column of
/// plausible-looking guesses.
///
/// No column is nullable. Every field is present in every chunk: an empty
/// `metadata` is an empty string on the wire, and a zero-length payload is an
/// empty blob -- neither is a missing value.
pub fn events_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        // ~7 distinct groups over the whole file; dictionary is nearly free.
        Field::new(
            "group",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new("metadata", DataType::Utf8, false),
        Field::new("time1", DataType::UInt32, false),
        Field::new("time2", DataType::UInt32, false),
        // The declared SizeInBytes, kept as the wire's i32. Redundant with
        // `raw_payload`'s length by construction, but readable from row-group
        // statistics without touching the binary column.
        Field::new("payload_size", DataType::Int32, false),
        Field::new("raw_payload", DataType::Binary, false),
    ])
}

/// Convenience: wrap events schema in an Arc.
pub fn events_schema_ref() -> Arc<Schema> {
    Arc::new(events_schema())
}
