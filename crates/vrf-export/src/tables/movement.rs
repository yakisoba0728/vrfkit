//! The `movement` table: one row per movement sample.
//!
//! Movement samples are the densest data in a replay (~1.8 M rows per match).
//! Every row has the same fixed schema with no nulls, which makes this table a
//! textbook case for columnar compression: each f32 column compresses very well
//! under ZSTD because adjacent position samples differ by small deltas.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float32Array, RecordBatch, UInt8Array, UInt32Array};
use arrow_schema::Schema;

use crate::error::ExportError;
use crate::record::MovementRecord;
use crate::schema::movement_schema_ref;
use crate::writer::{Table, TableWriter};

/// Default row group size for movement data.
///
/// Movement rows are smaller (11 x 4 bytes + 1 x 4 + 2 x 1 = 50 bytes per row
/// uncompressed), so we can afford a larger row group without excessive memory
/// use. 256 Ki rows approximately 13 MB uncompressed per row group -- a good
/// chunk size for ZSTD.
pub const DEFAULT_MOVEMENT_ROW_GROUP_SIZE: usize = 262_144;

/// Table marker for `movement`. See [`MovementWriter`].
pub struct MovementTable;

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
///     timestamp: 31_337, movement_state: 2, move_type: 1,
/// })?;
/// writer.finish()?;
/// # Ok(())
/// # }
/// ```
pub type MovementWriter<W> = TableWriter<MovementTable, W>;

impl Table for MovementTable {
    type Row = MovementRecord;

    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_MOVEMENT_ROW_GROUP_SIZE;

    // Every column is a fixed-width number; there is nothing to dictionary.
    const DICTIONARY_COLUMNS: &'static [&'static str] = &[];

    fn schema() -> Arc<Schema> {
        movement_schema_ref()
    }

    fn build_batch(rows: &[MovementRecord]) -> Result<RecordBatch, ExportError> {
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
        let timestamp: ArrayRef = Arc::new(UInt32Array::from_iter_values(
            rows.iter().map(|r| r.timestamp),
        ));
        let movement_state: ArrayRef = Arc::new(UInt8Array::from_iter_values(
            rows.iter().map(|r| r.movement_state),
        ));
        let move_type: ArrayRef = Arc::new(UInt8Array::from_iter_values(
            rows.iter().map(|r| r.move_type),
        ));

        // Order must match movement_schema() exactly -- RecordBatch::try_new
        // only checks types, so a swap between two same-typed columns (e.g.
        // movement_state and move_type) would pass and corrupt the export.
        RecordBatch::try_new(
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
                timestamp,
                movement_state,
                move_type,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))
    }
}
