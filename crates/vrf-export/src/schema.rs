//! Arrow schema definitions for the two export tables.
//!
//! Schemas are defined once here so that writer and reader agree. The field
//! metadata (e.g. `PARQUET:field_id`) is intentionally omitted — Parquet
//! assigns ordinal field IDs automatically, and manual IDs would only matter
//! if we needed Iceberg-style schema evolution, which we don't.

use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

/// Schema for the `fields` table (long format: one row = one decoded field).
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
        // Sparse typed-value overlay — at most one of these is non-null per row.
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
/// Positions are in centimetres; yaw/pitch are in degrees (−180..180).
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
