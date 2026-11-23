//! The `ability_casts` table: one row per ability cast, rebuilt from the
//! `AbilityCastsThisRound` snapshots (why: `schema::ability_casts_schema`).

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;

use super::batch;
use crate::error::ExportError;
use crate::record::AbilityCastRecord;
use crate::schema::ability_casts_schema_ref;
use crate::writer::{Table, TableWriter};

/// Rows per row group by default: a match yields a few thousand rows, well
/// under one group, so the constant is small and the writer is inline.
pub const DEFAULT_ABILITY_CAST_ROW_GROUP_SIZE: usize = 131_072;

pub struct AbilityCastsTable;

/// Streaming Parquet writer for reconstructed casts.
pub type AbilityCastWriter<W> = TableWriter<AbilityCastsTable, W>;

impl Table for AbilityCastsTable {
    type Row = AbilityCastRecord;
    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_ABILITY_CAST_ROW_GROUP_SIZE;
    const DICTIONARY_COLUMNS: &'static [&'static str] = &["actor_net_guid"];
    fn schema() -> Arc<Schema> {
        ability_casts_schema_ref()
    }
    fn initial_capacity(_batch_rows: usize) -> usize {
        4096
    }
    fn build_batch(rows: &[AbilityCastRecord]) -> Result<RecordBatch, ExportError> {
        batch(Self::schema(), AbilityCastRecord::columns(rows.iter()))
    }
}
