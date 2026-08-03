//! The `actors` table: one row per actor channel open or close.
//!
//! This makes actors visible even when they never replicate a field -- critical
//! for downstream resolution of weapon instances, ability instances, and
//! equipment actors.

use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, Float32Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::Schema;

use crate::error::ExportError;
use crate::record::ActorRecord;
use crate::schema::actors_schema_ref;
use crate::writer::{Table, TableWriter};

/// Default row group size for actor data.
///
/// Actor events are sparse relative to fields/movement (~4 K rows per match).
/// A single row group is fine for compression and memory, but we keep the
/// same streaming architecture for consistency.
pub const DEFAULT_ACTOR_ROW_GROUP_SIZE: usize = 131_072;

/// Table marker for `actors`. See [`ActorWriter`].
pub struct ActorsTable;

/// Streaming Parquet writer for actor lifecycle records.
pub type ActorWriter<W> = TableWriter<ActorsTable, W>;

impl Table for ActorsTable {
    type Row = ActorRecord;

    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_ACTOR_ROW_GROUP_SIZE;

    // `event` is deliberately absent: two distinct values ("open"/"close") are
    // cheaper as plain Utf8 than as a dictionary.
    const DICTIONARY_COLUMNS: &'static [&'static str] = &["class_path", "archetype_path"];

    fn schema() -> Arc<Schema> {
        actors_schema_ref()
    }

    fn initial_capacity(_batch_rows: usize) -> usize {
        4096
    }

    fn build_batch(rows: &[ActorRecord]) -> Result<RecordBatch, ExportError> {
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
        let event: ArrayRef = Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.event)));

        // Dictionary-encoded class_path (nullable).
        let mut class_path_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 128, len * 30);
        for r in rows {
            match &r.class_path {
                Some(p) => class_path_builder.append_value(p),
                None => class_path_builder.append_null(),
            }
        }
        let class_path: ArrayRef = Arc::new(class_path_builder.finish());

        // Dictionary-encoded archetype_path (nullable).
        let mut archetype_path_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 128, len * 30);
        for r in rows {
            match &r.archetype_path {
                Some(p) => archetype_path_builder.append_value(p),
                None => archetype_path_builder.append_null(),
            }
        }
        let archetype_path: ArrayRef = Arc::new(archetype_path_builder.finish());

        let spawn_x: ArrayRef = Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_x)));
        let spawn_y: ArrayRef = Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_y)));
        let spawn_z: ArrayRef = Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_z)));
        let spawn_pitch: ArrayRef =
            Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_pitch)));
        let spawn_yaw: ArrayRef =
            Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_yaw)));
        let spawn_roll: ArrayRef =
            Arc::new(Float32Array::from_iter(rows.iter().map(|r| r.spawn_roll)));

        RecordBatch::try_new(
            actors_schema_ref(),
            vec![
                time_ms,
                packet_id,
                channel_index,
                actor_net_guid,
                event,
                class_path,
                archetype_path,
                spawn_x,
                spawn_y,
                spawn_z,
                spawn_pitch,
                spawn_yaw,
                spawn_roll,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))
    }
}
