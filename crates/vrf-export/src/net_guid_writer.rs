//! Streaming writer for the `net_guids` table.
//!
//! One row per NetGUID the replay registered, carrying its object path and the
//! GUID of its containing object. This is the containment hierarchy Unreal
//! builds while reading export-GUID bunches.
//!
//! Why it needs its own table: `actors.parquet` only records GUIDs that opened
//! a channel. Subobjects never do. A weapon's `FiringState` -- the only handle a
//! shot event carries back to the gun that fired it -- appears in no other
//! export. The parser has always computed this mapping and then discarded it.

use std::io::Write;
use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::ExportError;
use crate::schema::net_guids_schema_ref;

/// Default row group size for the net_guid table.
///
/// The table is small -- roughly 16 K rows for a full competitive match -- so
/// this holds it in a single row group while keeping the streaming shape the
/// other writers use.
pub const DEFAULT_NET_GUID_ROW_GROUP_SIZE: usize = 131_072;

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

/// Streaming Parquet writer for NetGUID registrations.
pub struct NetGuidWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<NetGuidRecord>,
    row_group_size: usize,
    finished: bool,
}

impl<W: Write + Send> NetGuidWriter<W> {
    /// Create a new writer with default settings (ZSTD, 128 Ki rows per group).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, DEFAULT_NET_GUID_ROW_GROUP_SIZE)
    }

    /// Create a new writer with a custom row-group size.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = net_guids_schema_ref();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, Arc::clone(&schema), Some(props))?;
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(4096),
            row_group_size,
            finished: false,
        })
    }

    /// Push a single record. Flushes when the buffer is full.
    pub fn push(&mut self, record: NetGuidRecord) -> Result<(), ExportError> {
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
                parquet::schema::types::ColumnPath::new(vec!["path".into()]),
                true,
            )
            .build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let rows: Vec<NetGuidRecord> = std::mem::take(&mut self.buffer);
        let batch = Self::build_record_batch(&rows)?;
        self.writer.write(&batch)?;
        Ok(())
    }

    fn build_record_batch(rows: &[NetGuidRecord]) -> Result<RecordBatch, ExportError> {
        let len = rows.len();

        let net_guid: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.net_guid),
        ));

        let mut path_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 1024, len * 40);
        for r in rows {
            path_builder.append_value(&r.path);
        }
        let path: ArrayRef = Arc::new(path_builder.finish());

        let outer_net_guid: ArrayRef = Arc::new(UInt32Array::from_iter(
            rows.iter().map(|r| r.outer_net_guid),
        ));

        RecordBatch::try_new(net_guids_schema_ref(), vec![net_guid, path, outer_net_guid])
            .map_err(|e| ExportError::Parquet(e.into()))
    }
}
