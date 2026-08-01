//! Streaming writer for the `actors` table.
//!
//! One row per actor channel open or close. This makes actors visible even
//! when they never replicate a field -- critical for downstream resolution of
//! weapon instances, ability instances, and equipment actors.

use std::io::Write;
use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, Float32Array, RecordBatch, StringArray, UInt32Array};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::ExportError;
use crate::schema::actors_schema_ref;

/// Default row group size for actor data.
///
/// Actor events are sparse relative to fields/movement (~4 K rows per match).
/// A single row group is fine for compression and memory, but we keep the
/// same streaming architecture for consistency.
pub const DEFAULT_ACTOR_ROW_GROUP_SIZE: usize = 131_072;

/// A single actor lifecycle record ready for export.
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

/// Streaming Parquet writer for actor lifecycle records.
pub struct ActorWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<ActorRecord>,
    row_group_size: usize,
    finished: bool,
}

impl<W: Write + Send> ActorWriter<W> {
    /// Create a new writer with default settings (ZSTD, 128 Ki rows per group).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, DEFAULT_ACTOR_ROW_GROUP_SIZE)
    }

    /// Create a new writer with a custom row-group size.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = actors_schema_ref();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, Arc::clone(&schema), Some(props))?;
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(4096),
            row_group_size,
            finished: false,
        })
    }

    /// Push a single actor record. Flushes when the buffer is full.
    pub fn push(&mut self, record: ActorRecord) -> Result<(), ExportError> {
        if self.finished {
            return Err(ExportError::Usage(
                "cannot push to a finished writer".into(),
            ));
        }
        self.buffer.push(record);
        if self.buffer.len() >= self.row_group_size {
            self.flush_buffer()?;
        }
        Ok(())
    }

    /// Flush remaining rows and finalise the file. Must be called.
    pub fn finish(mut self) -> Result<(), ExportError> {
        if !self.buffer.is_empty() {
            self.flush_buffer()?;
        }
        self.writer.close()?;
        self.finished = true;
        Ok(())
    }

    /// Number of rows currently buffered.
    pub fn buffered_rows(&self) -> usize {
        self.buffer.len()
    }

    // -- internal --

    fn writer_properties(row_group_size: usize) -> WriterProperties {
        WriterProperties::builder()
            .set_max_row_group_row_count(Some(row_group_size))
            .set_compression(Compression::ZSTD(Default::default()))
            .set_statistics_enabled(parquet::file::properties::EnabledStatistics::Page)
            .set_column_dictionary_enabled(
                parquet::schema::types::ColumnPath::new(vec!["class_path".into()]),
                true,
            )
            .set_column_dictionary_enabled(
                parquet::schema::types::ColumnPath::new(vec!["archetype_path".into()]),
                true,
            )
            .build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let rows: Vec<ActorRecord> = std::mem::take(&mut self.buffer);
        let batch = Self::build_record_batch(&rows)?;
        self.writer.write(&batch)?;
        Ok(())
    }

    fn build_record_batch(rows: &[ActorRecord]) -> Result<RecordBatch, ExportError> {
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

        let batch = RecordBatch::try_new(
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
        .map_err(|e| ExportError::Parquet(e.into()))?;

        Ok(batch)
    }
}
