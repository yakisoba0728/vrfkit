//! The `net_guids` table: one row per NetGUID the replay registered.
//!
//! Each row carries the GUID's object path and the GUID of its containing
//! object -- the containment hierarchy Unreal builds while reading export-GUID
//! bunches.
//!
//! Why it needs its own table: `actors.parquet` only records GUIDs that opened
//! a channel. Subobjects never do. A weapon's `FiringState` -- the only handle a
//! shot event carries back to the gun that fired it -- appears in no other
//! export. The parser has always computed this mapping and then discarded it.

use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::Schema;

use crate::error::ExportError;
use crate::record::NetGuidRecord;
use crate::schema::net_guids_schema_ref;
use crate::writer::{Table, TableWriter};

/// Default row group size for the net_guid table.
///
/// The table is small -- roughly 16 K rows for a full competitive match -- so
/// this holds it in a single row group while keeping the streaming shape the
/// other writers use.
pub const DEFAULT_NET_GUID_ROW_GROUP_SIZE: usize = 131_072;

/// Table marker for `net_guids`. See [`NetGuidWriter`].
pub struct NetGuidsTable;

/// Streaming Parquet writer for NetGUID registrations.
pub type NetGuidWriter<W> = TableWriter<NetGuidsTable, W>;

impl Table for NetGuidsTable {
    type Row = NetGuidRecord;

    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_NET_GUID_ROW_GROUP_SIZE;

    // Paths repeat heavily: 175 GUIDs share "FiringState" in one match.
    const DICTIONARY_COLUMNS: &'static [&'static str] = &["path"];

    fn schema() -> Arc<Schema> {
        net_guids_schema_ref()
    }

    fn initial_capacity(_batch_rows: usize) -> usize {
        4096
    }

    fn build_batch(rows: &[NetGuidRecord]) -> Result<RecordBatch, ExportError> {
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
