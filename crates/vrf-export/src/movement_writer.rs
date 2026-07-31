//! Streaming writer for the `movement` table.
//!
//! Movement samples are the densest data in a replay (~2.4 M rows per match).
//! Every row has the same fixed schema with no nulls, which makes this table a
//! textbook case for columnar compression: each f32 column compresses very well
//! under ZSTD because adjacent position samples differ by small deltas.

use std::io::Write;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float32Array, RecordBatch, UInt32Array};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::ExportError;
use crate::schema::movement_schema_ref;

/// Default row group size for movement data.
///
/// Movement rows are smaller (11 × 4 bytes = 44 bytes per row uncompressed),
/// so we can afford a larger row group without excessive memory use. 256 Ki
/// rows ≈ 11 MB uncompressed per row group — a good chunk size for ZSTD.
pub const DEFAULT_MOVEMENT_ROW_GROUP_SIZE: usize = 262_144;

/// A single movement sample ready for export.
///
/// All fields are non-optional (the replication channel always provides the
/// full state vector; partial updates are merged upstream before reaching us).
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
}

/// Streaming Parquet writer for movement records.
///
/// # Usage
///
/// ```no_run
/// # use vrf_export::{MovementWriter, MovementRecord, ExportError};
/// # fn example() -> Result<(), ExportError> {
/// let file = std::fs::File::create("movement.parquet")?;
/// let mut writer = MovementWriter::new(file)?;
/// writer.push(MovementRecord {
///     time_ms: 5000, packet_id: 100, character_net_guid: 42,
///     pos_x: 1000.0, pos_y: 2000.0, pos_z: 300.0,
///     yaw: 45.0, pitch: -10.0,
///     vel_x: 100.0, vel_y: 0.0, vel_z: 0.0,
/// })?;
/// writer.finish()?;
/// # Ok(())
/// # }
/// ```
pub struct MovementWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    buffer: Vec<MovementRecord>,
    row_group_size: usize,
    finished: bool,
}

impl<W: Write + Send> MovementWriter<W> {
    /// Create a new writer with default settings (ZSTD, 256 Ki rows per group).
    pub fn new(sink: W) -> Result<Self, ExportError> {
        Self::with_row_group_size(sink, DEFAULT_MOVEMENT_ROW_GROUP_SIZE)
    }

    /// Create a new writer with a custom row-group size.
    pub fn with_row_group_size(sink: W, row_group_size: usize) -> Result<Self, ExportError> {
        let schema = movement_schema_ref();
        let props = Self::writer_properties(row_group_size);
        let writer = ArrowWriter::try_new(sink, Arc::clone(&schema), Some(props))?;
        Ok(Self {
            writer,
            buffer: Vec::with_capacity(row_group_size.min(65_536)),
            row_group_size,
            finished: false,
        })
    }

    /// Push a single movement record. Flushes when the buffer is full.
    pub fn push(&mut self, record: MovementRecord) -> Result<(), ExportError> {
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

    /// Push a batch of records efficiently.
    pub fn push_batch(
        &mut self,
        records: impl IntoIterator<Item = MovementRecord>,
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

    // ── internal ──────────────────────────────────────────────────────────

    fn writer_properties(row_group_size: usize) -> WriterProperties {
        WriterProperties::builder()
            .set_max_row_group_row_count(Some(row_group_size))
            .set_compression(Compression::ZSTD(Default::default()))
            .set_statistics_enabled(parquet::file::properties::EnabledStatistics::Page)
            .build()
    }

    fn flush_buffer(&mut self) -> Result<(), ExportError> {
        let rows: Vec<MovementRecord> = std::mem::take(&mut self.buffer);
        let batch = Self::build_record_batch(&rows)?;
        self.writer.write(&batch)?;
        Ok(())
    }

    fn build_record_batch(rows: &[MovementRecord]) -> Result<RecordBatch, ExportError> {
        let time_ms: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.time_ms),
        ));
        let packet_id: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.packet_id),
        ));
        let character_net_guid: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.character_net_guid),
        ));
        let pos_x: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.pos_x)));
        let pos_y: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.pos_y)));
        let pos_z: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.pos_z)));
        let yaw: ArrayRef = Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.yaw)));
        let pitch: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.pitch)));
        let vel_x: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.vel_x)));
        let vel_y: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.vel_y)));
        let vel_z: ArrayRef =
            Arc::new(Float32Array::from_iter_values(rows.iter().map(|r| r.vel_z)));

        let batch = RecordBatch::try_new(
            movement_schema_ref(),
            vec![
                time_ms,
                packet_id,
                character_net_guid,
                pos_x,
                pos_y,
                pos_z,
                yaw,
                pitch,
                vel_x,
                vel_y,
                vel_z,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))?;

        Ok(batch)
    }
}
