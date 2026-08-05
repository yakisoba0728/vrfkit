//! The `events` table: one row per Event chunk.
//!
//! Event chunks are the server's own labelled game timeline. Round starts,
//! character deaths, spike plants and defuses arrive here already named, with a
//! millisecond timestamp, instead of having to be inferred from replicated
//! properties and RPCs.
//!
//! Why the payload is a blob: the Event chunk header is fully decoded, but the
//! bytes it wraps are a group-dependent word list with no count on the wire
//! (see `vrf_container::EventChunk`). Emitting those words under invented
//! names would be a guess; `raw_payload` keeps every byte and claims nothing.

use std::sync::Arc;

use arrow_array::builder::StringDictionaryBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, BinaryArray, Int32Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::Schema;

use crate::error::ExportError;
use crate::record::EventRecord;
use crate::schema::events_schema_ref;
use crate::writer::{Table, TableWriter};

/// Default row group size for the events table.
///
/// A full competitive match yields a couple of hundred rows, so this holds the
/// whole table in one row group while keeping the streaming shape the other
/// writers use.
pub const DEFAULT_EVENT_ROW_GROUP_SIZE: usize = 131_072;

/// Table marker for `events`. See [`EventWriter`].
pub struct EventsTable;

/// Streaming Parquet writer for Event chunks.
pub type EventWriter<W> = TableWriter<EventsTable, W>;

impl Table for EventsTable {
    type Row = EventRecord;

    const DEFAULT_ROW_GROUP_SIZE: usize = DEFAULT_EVENT_ROW_GROUP_SIZE;

    // ~7 distinct groups over the whole file; dictionary is nearly free. `id`
    // and `metadata` are near-unique per row and stay plain Utf8.
    const DICTIONARY_COLUMNS: &'static [&'static str] = &["group"];

    fn schema() -> Arc<Schema> {
        events_schema_ref()
    }

    fn initial_capacity(_batch_rows: usize) -> usize {
        256
    }

    fn build_batch(rows: &[EventRecord]) -> Result<RecordBatch, ExportError> {
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
        let word0: ArrayRef = Arc::new(UInt32Array::from_iter(rows.iter().map(|r| r.word0)));
        let word1: ArrayRef = Arc::new(UInt32Array::from_iter(rows.iter().map(|r| r.word1)));

        RecordBatch::try_new(
            events_schema_ref(),
            vec![
                id,
                group,
                metadata,
                time1,
                time2,
                payload_size,
                raw_payload,
                word0,
                word1,
            ],
        )
        .map_err(|e| ExportError::Parquet(e.into()))
    }
}
