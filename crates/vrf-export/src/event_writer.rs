//! Streaming writer for the `events` table.
//!
//! One row per Event chunk -- the server's own labelled game timeline. Round
//! starts, character deaths, spike plants and defuses arrive here already
//! named, with a millisecond timestamp, instead of having to be inferred from
//! replicated properties and RPCs.
//!
//! Why the payload is a blob: the Event chunk header is fully decoded, but the
//! bytes it wraps are a group-dependent word list with no count on the wire
//! (see `vrf_container::EventChunk`). Emitting those words under invented
//! names would be a guess; `raw_payload` keeps every byte and claims nothing.

use std::io::Write;
use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, BinaryArray, Int32Array, RecordBatch, StringArray, UInt32Array};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::ExportError;
use crate::schema::events_schema_ref;

/// Default row group size for the events table.
///
/// A full competitive match yields a couple of hundred rows, so this holds the
/// whole table in one row group while keeping the streaming shape the other
/// writers use.
pub const DEFAULT_EVENT_ROW_GROUP_SIZE: usize = 131_072;

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
}

/// Streaming Parquet writer for Event chunks.
pub struct EventWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<EventRecord>,
    row_group_size: usize,
    finished: bool,
}

impl<W: Write + Send> EventWriter<W> {
    /// Create a new writer with default settings (ZSTD, 128 Ki rows per group).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, DEFAULT_EVENT_ROW_GROUP_SIZE)
    }

    /// Create a new writer with a custom row-group size.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = events_schema_ref();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, Arc::clone(&schema), Some(props))?;
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(256),
            row_group_size,
            finished: false,
        })
    }

    /// Push a single record. Flushes when the buffer is full.
    pub fn push(&mut self, record: EventRecord) -> Result<(), ExportError> {
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
                parquet::schema::types::ColumnPath::new(vec!["group".into()]),
                true,
            )
            .build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let rows: Vec<EventRecord> = std::mem::take(&mut self.buffer);
        let batch = Self::build_record_batch(&rows)?;
        self.writer.write(&batch)?;
        Ok(())
    }

    fn build_record_batch(rows: &[EventRecord]) -> Result<RecordBatch, ExportError> {
        let len = rows.len();

        let id: ArrayRef = Arc::new(StringArray::from_iter_values(
            rows.iter().map(|r| r.id.as_str()),
        ));

        let mut group_builder =
            StringDictionaryBuilder::<Int32Type>::with_capacity(len, 16, len * 24);
        for r in rows {
            group_builder.append_value(&r.group);
        }
        let group: ArrayRef = Arc::new(group_builder.finish());

        let metadata: ArrayRef = Arc::new(StringArray::from_iter_values(
            rows.iter().map(|r| r.metadata.as_str()),
        ));
        let time1: ArrayRef = Arc::new(UInt32Array::from_iter_values(rows.iter().map(|r| r.time1)));
        let time2: ArrayRef = Arc::new(UInt32Array::from_iter_values(rows.iter().map(|r| r.time2)));
        let payload_size: ArrayRef = Arc::new(Int32Array::from_iter_values(
            rows.iter().map(|r| r.payload_size),
        ));
        let raw_payload: ArrayRef = Arc::new(BinaryArray::from_iter_values(
            rows.iter().map(|r| r.raw_payload.as_slice()),
        ));

        RecordBatch::try_new(
            events_schema_ref(),
            vec![id, group, metadata, time1, time2, payload_size, raw_payload],
        )
        .map_err(|e| ExportError::Parquet(e.into()))
    }
}
