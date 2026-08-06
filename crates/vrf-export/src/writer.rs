//! The streaming writer every table shares.
//!
//! All five tables have the same shape: buffer rows, convert a batch of them to
//! Arrow when the buffer fills, finalise on `finish`. Only three things differ
//! -- the Arrow schema, the columns worth dictionary-encoding, and how a slice
//! of rows becomes a `RecordBatch`. Those three are the [`Table`] trait;
//! everything else lives here once.
//!
//! Two thresholds, not one, and they are independent. [`MAX_BUFFERED_ROWS`] is
//! how many records are held before conversion -- what bounds this crate's
//! memory. The row-group size is what `ArrowWriter` cuts row groups at -- what
//! shapes the file. A multi-million-row export holds one batch, not one row
//! group and not the whole table.

use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::schema::types::ColumnPath;

use crate::error::ExportError;

/// The mini-batch a Parquet column writer works in, pinned rather than taken
/// from the library default.
///
/// It matters here because it is the granularity at which the column writer
/// asks "is this data page full yet". See [`MAX_BUFFERED_ROWS`]: leaving it
/// implicit would put a byte-level invariant of this crate's output in another
/// crate's default value.
pub const PARQUET_WRITE_BATCH_SIZE: usize = 1_024;

/// Rows held as records before being converted to an Arrow batch.
///
/// This is **not** the row-group size. `ArrowWriter` accumulates the batches it
/// is given and cuts a row group when `max_row_group_row_count` is reached, so
/// feeding it sixteen batches of 8,192 puts the row-group boundaries in exactly
/// the same places as one batch of 131,072 did.
///
/// # The invariant: a multiple of [`PARQUET_WRITE_BATCH_SIZE`]
///
/// Batch size is **not** free of the output bytes, and assuming it was is a
/// mistake this constant is here to prevent repeating. `write_batch` splits its
/// input into mini-batches of [`PARQUET_WRITE_BATCH_SIZE`] and evaluates the
/// data-page limits after each one, so the set of value offsets at which a page
/// may be cut is the set of multiples of that size -- *unless* a batch ends
/// part-way through one, which introduces an extra, differently-placed check
/// point and can move a page boundary.
///
/// Measured on the reference replay, all 11 Parquet outputs:
///
/// | rows per batch | result |
/// |---|---|
/// | 131,072 (the old behaviour, one batch per row group) | byte-identical |
/// | 8,192 = 8 x 1,024 | byte-identical |
/// | 3,072 = 3 x 1,024, and no divisor of either row group | byte-identical |
/// | 3,000 | **bytes moved**, in `fields`, `movement` and `checkpoint_fields` |
///
/// So the constraint is alignment to the mini-batch, not any relationship to
/// the row-group size. The assertion below enforces it at compile time.
///
/// # What it buys
///
/// Peak memory. Holding a whole row group as records cost ~20 MB of
/// `FieldRecord` plus their heap payloads, and `build_batch` doubles that for
/// the duration of the conversion because the rows and the arrays are both
/// live. This change on its own, five runs of `export` on the reference replay
/// with the previous binary as the only difference, took peak working set from
/// 172.0 MB to 105.9 MB and median wall time from 1.456 s to 1.281 s. It is the
/// single largest memory win in the rewrite.
///
/// Not smaller: below a few thousand rows the fixed cost of building fourteen
/// Arrow arrays starts to show up against the per-row work, and the dictionary
/// builders' capacity hints stop being useful.
pub const MAX_BUFFERED_ROWS: usize = 8_192;

const _: () = assert!(
    MAX_BUFFERED_ROWS % PARQUET_WRITE_BATCH_SIZE == 0,
    "MAX_BUFFERED_ROWS must be a multiple of PARQUET_WRITE_BATCH_SIZE or the \
     Parquet output moves; see the table above this constant"
);

/// Everything the generic writer needs to know about one table.
///
/// Implemented by a zero-sized marker type per table (e.g. `FieldsTable`); the
/// public writer names are type aliases over [`TableWriter`].
pub trait Table {
    /// The record type callers push.
    type Row;

    /// Rows per row group when the caller does not choose.
    ///
    /// Sized per table: it trades peak memory against how large a column chunk
    /// ZSTD gets to work on.
    const DEFAULT_ROW_GROUP_SIZE: usize;

    /// Columns to dictionary-encode. Dictionary is Parquet's default for Utf8,
    /// but the tables pin it explicitly so the intent is in the source.
    const DICTIONARY_COLUMNS: &'static [&'static str];

    /// The Arrow schema. Must match [`Self::build_batch`]'s column order:
    /// `RecordBatch::try_new` only checks types, so swapping two same-typed
    /// columns would pass and silently corrupt the export.
    fn schema() -> Arc<Schema>;

    /// How many rows to reserve in the in-memory buffer up front.
    ///
    /// Small tables (actors, net_guids, events) never fill even one batch, so
    /// reserving a whole one would be dead memory.
    fn initial_capacity(batch_rows: usize) -> usize {
        batch_rows
    }

    /// Convert a full buffer into one Arrow record batch.
    fn build_batch(rows: &[Self::Row]) -> Result<RecordBatch, ExportError>;
}

/// Streaming Parquet writer for one table.
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
///     compatible_checksum: Some(2_983_776_962),
///     bit_count: 32, raw_bits: Some(vec![0x64, 0, 0, 0].into()),
///     value_i64: Some(100), value_f64: None,
///     value_bool: None, value_str: None,
/// })?;
/// writer.finish()?;
/// # Ok(())
/// # }
/// ```
pub struct TableWriter<T: Table, W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<T::Row>,
    /// Rows per Arrow batch. See [`MAX_BUFFERED_ROWS`]; never larger than the
    /// row-group size, so a caller asking for tiny row groups still gets them.
    batch_rows: usize,
    finished: bool,
    _table: PhantomData<fn() -> T>,
}

impl<T: Table, W: Write + Send> TableWriter<T, W> {
    /// Create a writer with the table's default settings (ZSTD compression,
    /// dictionary encoding for the table's string columns, page statistics).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, T::DEFAULT_ROW_GROUP_SIZE)
    }

    /// Create a writer with a custom row-group size.
    ///
    /// Smaller values put more, smaller row groups in the file, which costs
    /// compression ratio. It does not change how many rows are held in memory
    /// at once unless it is below [`MAX_BUFFERED_ROWS`].
    ///
    /// A `row_group_size` below that ceiling is *not* subject to the
    /// mini-batch-alignment rule documented on [`MAX_BUFFERED_ROWS`], despite
    /// what the assertion there might suggest. Below the ceiling the clamp is a
    /// no-op, so one batch is one row group exactly as it was before batching
    /// and row groups were separated -- there is no partial batch inside a row
    /// group for a page boundary to land differently in.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = T::schema();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, schema, Some(props))?;
        // Floored at 1 so `with_row_group_size(sink, 0)` cannot make `push`
        // buffer forever without ever flushing.
        let batch_rows = row_group_size.clamp(1, MAX_BUFFERED_ROWS);
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(T::initial_capacity(batch_rows)),
            batch_rows,
            finished: false,
            _table: PhantomData,
        })
    }

    /// Push a single record. Converts and hands off a batch when the buffer is
    /// full; the row group is closed by `ArrowWriter`, not here.
    pub fn push(&mut self, record: T::Row) -> Result<(), ExportError> {
        self.guard_open()?;
        self.buffer.push(record);
        self.flush_if_full()
    }

    /// Push a batch of records. Cheaper than repeated single pushes because the
    /// finished-writer check happens once for the whole batch.
    pub fn push_batch(
        &mut self,
        records: impl IntoIterator<Item = T::Row>,
    ) -> Result<(), ExportError> {
        self.guard_open()?;
        for record in records {
            self.buffer.push(record);
            self.flush_if_full()?;
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

    fn guard_open(&self) -> Result<(), ExportError> {
        if self.finished {
            return Err(ExportError::Usage(
                "cannot push to a finished writer".into(),
            ));
        }
        Ok(())
    }

    fn flush_if_full(&mut self) -> Result<(), ExportError> {
        if self.buffer.len() >= self.batch_rows {
            self.flush_buffer()?;
        }
        Ok(())
    }

    fn writer_properties(row_group_size: usize) -> WriterProperties {
        let mut builder = WriterProperties::builder()
            .set_max_row_group_row_count(Some(row_group_size))
            .set_compression(Compression::ZSTD(Default::default()))
            // Statistics for all columns so that readers can skip row groups
            // via predicate pushdown (e.g. "actor_net_guid = X").
            .set_statistics_enabled(EnabledStatistics::Page)
            // Same as the library default, set explicitly because
            // MAX_BUFFERED_ROWS has to stay a multiple of it. A parquet release
            // that changed the default would otherwise move this crate's output
            // bytes with nothing in this repository having changed.
            .set_write_batch_size(PARQUET_WRITE_BATCH_SIZE);
        for column in T::DICTIONARY_COLUMNS {
            builder = builder
                .set_column_dictionary_enabled(ColumnPath::new(vec![(*column).to_owned()]), true);
        }
        builder.build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let batch = T::build_batch(&self.buffer)?;
        // Cleared, not taken: the capacity is bounded by `batch_rows` now, so
        // keeping it across flushes costs one allocation for the whole run.
        // Dropping the rows before handing the batch to the encoder also keeps
        // the records and the arrays from being live at the same time.
        self.buffer.clear();
        self.writer.write(&batch)?;
        Ok(())
    }
}
