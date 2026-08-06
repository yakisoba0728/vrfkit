//! The `fields` table: one row per decoded field, RPC parameter, or preserved
//! whole-block payload.
//!
//! The writer accumulates rows in memory until a configurable row-group
//! threshold is reached, then flushes a complete row group to the underlying
//! Parquet file. This keeps peak memory bounded regardless of replay size.

use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, UInt32Array,
};
use arrow_schema::Schema;

use crate::error::ExportError;
use crate::record::FieldRecord;
use crate::schema::fields_schema_ref;
use crate::writer::{Table, TableWriter};

/// Default number of rows per row group. 128 Ki approximately 131 072 rows is a good
/// balance between memory use (~40 MB for this schema) and compression
/// efficiency (column chunks are large enough for ZSTD to reach steady state).
pub const DEFAULT_ROW_GROUP_SIZE: usize = 131_072;

/// Table marker for `fields`. See [`FieldWriter`].
pub struct FieldsTable;

/// Streaming Parquet writer for field records.
pub type FieldWriter<W> = TableWriter<FieldsTable, W>;

impl Table for FieldsTable {
    type Row = FieldRecord;

    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_ROW_GROUP_SIZE;

    // The repetitive string columns. group_path and field_name are the address
    // columns (~475 distinct group paths, a few thousand field names over 1.2 M
    // rows); value_str carries the decoded typed values -- enum strings and
    // JSON blobs that repeat heavily across rows.
    const DICTIONARY_COLUMNS: &'static [&'static str] = &["group_path", "field_name", "value_str"];

    fn schema() -> Arc<Schema> {
        fields_schema_ref()
    }

    fn build_batch(rows: &[FieldRecord]) -> Result<RecordBatch, ExportError> {
        let len = rows.len();

        let time_ms: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.time_ms),
        ));
        let packet_id: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.packet_id),
        ));
        let channel_index: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.channel_index),
        ));
        let actor_net_guid: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.actor_net_guid),
        ));
        let object_net_guid: ArrayRef = Arc::new(UInt32Array::from_iter(
            rows.iter().map(|r| r.object_net_guid),
        ));

        // Dictionary-encoded group_path (non-nullable).
        //
        // `&*r.group_path` derefs the interned `Arc<str>` to the same `&str` a
        // `String` would have produced, so the builder sees an identical value
        // sequence and the encoded dictionary is unchanged. Interning must not
        // be exploited here -- e.g. by keying on `Arc::as_ptr` to skip an
        // append -- or the row-to-value mapping stops being one-to-one.
        let mut group_path_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 256, len * 20);
        for r in rows {
            group_path_builder.append_value(&r.group_path);
        }
        let group_path: ArrayRef = Arc::new(group_path_builder.finish());

        let handle: ArrayRef =
            Arc::new(UInt32Array::from_iter_values(rows.iter().map(|r| r.handle)));

        // Dictionary-encoded field_name (nullable).
        let mut field_name_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 256, len * 16);
        for r in rows {
            match &r.field_name {
                Some(name) => field_name_builder.append_value(name),
                None => field_name_builder.append_null(),
            }
        }
        let field_name: ArrayRef = Arc::new(field_name_builder.finish());

        let compatible_checksum: ArrayRef = Arc::new(UInt32Array::from_iter(
            rows.iter().map(|r| r.compatible_checksum),
        ));

        let bit_count: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.bit_count),
        ));

        let raw_bits: ArrayRef = Arc::new(BinaryArray::from_iter(
            rows.iter().map(|r| r.raw_bits.as_deref()),
        ));

        let value_i64: ArrayRef = Arc::new(Int64Array::from_iter(rows.iter().map(|r| r.value_i64)));
        let value_f64: ArrayRef =
            Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.value_f64)));
        let value_bool: ArrayRef =
            Arc::new(BooleanArray::from_iter(rows.iter().map(|r| r.value_bool)));
        // Dictionary-encoded value_str (nullable). The decoded typed values are
        // highly repetitive -- enum strings, JSON blobs, repeated struct JSON --
        // so a dictionary shrinks the column and speeds up downstream readers.
        // Same builder pattern as field_name above.
        let mut value_str_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 2048, len * 32);
        for r in rows {
            match &r.value_str {
                Some(s) => value_str_builder.append_value(s),
                None => value_str_builder.append_null(),
            }
        }
        let value_str: ArrayRef = Arc::new(value_str_builder.finish());

        RecordBatch::try_new(
            fields_schema_ref(),
            vec![
                time_ms,
                packet_id,
                channel_index,
                actor_net_guid,
                object_net_guid,
                group_path,
                handle,
                field_name,
                compatible_checksum,
                bit_count,
                raw_bits,
                value_i64,
                value_f64,
                value_bool,
                value_str,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))
    }
}
