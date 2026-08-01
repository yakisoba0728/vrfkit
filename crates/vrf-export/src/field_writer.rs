//! Streaming writer for the `fields` table.
//!
//! The writer accumulates rows in memory until a configurable row-group
//! threshold is reached, then flushes a complete row group to the underlying
//! Parquet file. This keeps peak memory bounded regardless of replay size.

use std::io::Write;
use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
    UInt32Array,
};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::{WriterProperties, WriterPropertiesBuilder};

use crate::error::ExportError;
use crate::schema::fields_schema_ref;

/// Default number of rows per row group. 128 Ki approximately 131 072 rows is a good
/// balance between memory use (~40 MB for this schema) and compression
/// efficiency (column chunks are large enough for ZSTD to reach steady state).
pub const DEFAULT_ROW_GROUP_SIZE: usize = 131_072;

/// A single field record ready for export.
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
    pub group_path: String,
    pub handle: u32,
    /// `None` when the field name is unknown (unmapped export index).
    pub field_name: Option<String>,
    pub bit_count: u32,
    /// Raw bit payload; `None` for zero-bit fields.
    pub raw_bits: Option<Vec<u8>>,
    pub value_i64: Option<i64>,
    pub value_f64: Option<f64>,
    pub value_bool: Option<bool>,
    pub value_str: Option<String>,
}

/// Streaming Parquet writer for field records.
///
/// # Usage
///
/// ```no_run
/// # use vrf_export::{FieldWriter, FieldRecord, ExportError};
/// # fn example() -> Result<(), ExportError> {
/// let file = std::fs::File::create("fields.parquet")?;
/// let mut writer = FieldWriter::new(file)?;
/// writer.push(FieldRecord {
///     time_ms: 1000, packet_id: 42, channel_index: 3,
///     actor_net_guid: 100, object_net_guid: None,
///     group_path: "PlayerState".into(),
///     handle: 7, field_name: Some("Health".into()),
///     bit_count: 32, raw_bits: Some(vec![0x64, 0, 0, 0]),
///     value_i64: Some(100), value_f64: None,
///     value_bool: None, value_str: None,
/// })?;
/// writer.finish()?;
/// # Ok(())
/// # }
/// ```
pub struct FieldWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<FieldRecord>,
    row_group_size: usize,
    finished: bool,
}

impl<W: Write + Send> FieldWriter<W> {
    /// Create a new writer with default settings (ZSTD compression, dictionary
    /// encoding for string columns, 128 Ki rows per row group).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, DEFAULT_ROW_GROUP_SIZE)
    }

    /// Create a new writer with a custom row-group size.
    ///
    /// Smaller values reduce peak memory but may hurt compression ratio.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = fields_schema_ref();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, Arc::clone(&schema), Some(props))?;
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(row_group_size.min(65_536)),
            row_group_size,
            finished: false,
        })
    }

    /// Push a single record. Flushes a row group when the buffer is full.
    pub fn push(&mut self, record: FieldRecord) -> Result<(), ExportError> {
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

    /// Push a batch of records. More efficient than repeated single pushes
    /// because it avoids per-record capacity checks when the batch is large.
    pub fn push_batch(
        &mut self,
        records: impl IntoIterator<Item = FieldRecord>,
    ) -> Result<(), ExportError> {
        if self.finished {
            return Err(ExportError::Usage(
                "cannot push to a finished writer".into(),
            ));
        }
        for record in records {
            self.buffer.push(record);
            if self.buffer.len() >= self.row_group_size {
                self.flush_buffer()?;
            }
        }
        Ok(())
    }

    /// Flush any remaining buffered rows and finalise the Parquet file.
    ///
    /// This **must** be called to produce a valid file. Dropping the writer
    /// without calling `finish` will leave a truncated (unreadable) file.
    pub fn finish(mut self) -> Result<(), ExportError> {
        if !self.buffer.is_empty() {
            self.flush_buffer()?;
        }
        self.writer.close()?;
        self.finished = true;
        Ok(())
    }

    /// Number of rows currently buffered (not yet flushed).
    pub fn buffered_rows(&self) -> usize {
        self.buffer.len()
    }

    // -- internal ----------------------------------------------------------

    fn writer_properties(row_group_size: usize) -> WriterProperties {
        let builder: WriterPropertiesBuilder = WriterProperties::builder()
            .set_max_row_group_row_count(Some(row_group_size))
            .set_compression(Compression::ZSTD(Default::default()))
            // Enable statistics for all columns so that readers can skip row
            // groups via predicate pushdown (e.g. "actor_net_guid = X").
            .set_statistics_enabled(parquet::file::properties::EnabledStatistics::Page)
            // Dictionary encoding for the two high-cardinality-but-repetitive
            // string columns. We enable dictionary explicitly (it's also the
            // default for Utf8, but we pin it to make intent clear).
            .set_column_dictionary_enabled(
                parquet::schema::types::ColumnPath::new(vec!["group_path".into()]),
                true,
            )
            .set_column_dictionary_enabled(
                parquet::schema::types::ColumnPath::new(vec!["field_name".into()]),
                true,
            );
        builder.build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let rows: Vec<FieldRecord> = std::mem::take(&mut self.buffer);
        let batch = Self::build_record_batch(&rows)?;
        self.writer.write(&batch)?;
        Ok(())
    }

    fn build_record_batch(rows: &[FieldRecord]) -> Result<RecordBatch, ExportError> {
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
        let value_str: ArrayRef = Arc::new(StringArray::from_iter(
            rows.iter().map(|r| r.value_str.as_deref()),
        ));

        let batch = RecordBatch::try_new(
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
                bit_count,
                raw_bits,
                value_i64,
                value_f64,
                value_bool,
                value_str,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))?;

        Ok(batch)
    }
}
