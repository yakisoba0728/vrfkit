//! Columnar (Parquet) output for decoded replay records.
//!
//! # Why Parquet
//!
//! A single VALORANT replay produces ~1.25 million content-block field records
//! and ~1.8 million movement samples. The predecessor NDJSON pipeline spent
//! **84 %** of wall time just parsing JSON. Parquet eliminates that: downstream
//! consumers (Python/pandas, DuckDB, Spark) memory-map the file and decode only
//! the columns they need, typically 10-50x faster for selective queries.
//!
//! # Layout
//!
//! - [`record`] -- the input structs. No Arrow types, never feature-gated.
//! - [`schema`] -- the Arrow schema for each table, defined once so writer and
//!   reader agree.
//! - [`writer`] -- the buffer-and-flush row-group machinery, written once.
//! - [`tables`] -- one module per table, supplying the three things that
//!   actually differ between them.
//!
//! # Schema design choices
//!
//! ## `fields` table -- sparse value columns vs. Union
//!
//! Every ordinary decoded-field record carries at most one typed value (i64,
//! f64, bool, or str); whole-block preservation records carry none. We
//! represent this as **four nullable columns** rather than an Arrow
//! DenseUnion because:
//!
//! 1. **Compression**: nullable columns where >90 % of values are null compress
//!    to nearly zero -- the validity bitmap itself is run-length-encoded inside
//!    Parquet. A Union column, on the other hand, must store type-id + offset
//!    arrays that are poorly compressible when the mix is heterogeneous.
//! 2. **Ecosystem compatibility**: DuckDB, pandas, and PyArrow handle nullable
//!    primitives without issue, while Union support varies across versions and
//!    can disable predicate pushdown.
//! 3. **Simplicity**: four extra columns with known types are trivial to filter
//!    (`WHERE value_i64 IS NOT NULL`); Union requires type-aware dispatch.
//!
//! ## Dictionary encoding
//!
//! `group_path` and `field_name` are dominated by a tiny set of repeated
//! strings (475 distinct group paths over 1.25 M rows on the reference replay).
//! Dictionary encoding stores the distinct values once and references them by
//! index, shrinking data pages by 50-200x. The producer interns the same two
//! columns as `Arc<str>`; see [`record`] for why, and note that the interning
//! is invisible to Arrow -- the builders are fed `&str` either way.
//!
//! ## Streaming
//!
//! Row groups stay large (128 Ki rows for `fields`, 256 Ki for `movement`) so
//! column chunks are big enough for efficient compression and predicate
//! pushdown. Memory is bounded separately, by converting a much smaller batch
//! of records to Arrow at a time; `ArrowWriter` accumulates those into row
//! groups. See [`writer::MAX_BUFFERED_ROWS`] for the one constraint that ties
//! the two together, which is not the one it looks like.
//!
//! # Feature flags
//!
//! | feature | default | effect |
//! |---|---|---|
//! | `parquet` | yes | arrow + parquet + the writer machinery |
//! | `fields`, `movement`, `actors`, `net-guids`, `events` | yes | one writer each; each implies `parquet` |
//! | `snappy` | no | adds Snappy to the Parquet codec set |
//!
//! With `--no-default-features` the crate is the record structs and
//! [`ExportError`] alone, and arrow/parquet/zstd leave the dependency graph
//! entirely. That is the configuration `vrfkit validate` uses: it drives the
//! whole decode pipeline, which produces records, and writes no file.
//!
//! The writers always compress with ZSTD, so zstd is **not** optional -- making
//! it a feature would let a build produce a file this crate cannot describe.
//! Snappy is never selected by any writer, so it is opt-in for consumers who
//! want the codec available for reading.

#![forbid(unsafe_code)]

mod error;
pub mod record;
#[cfg(feature = "parquet")]
pub mod schema;
#[cfg(feature = "parquet")]
pub mod tables;
#[cfg(feature = "parquet")]
pub mod writer;

pub use error::ExportError;
pub use record::{
    ActorRecord, EventRecord, FieldRecord, MovementRecord, NetGuidRecord,
    UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
};

#[cfg(feature = "actors")]
pub use tables::actors::{ActorWriter, ActorsTable, DEFAULT_ACTOR_ROW_GROUP_SIZE};
#[cfg(feature = "events")]
pub use tables::events::{DEFAULT_EVENT_ROW_GROUP_SIZE, EventWriter, EventsTable};
#[cfg(feature = "fields")]
pub use tables::fields::{DEFAULT_ROW_GROUP_SIZE, FieldWriter, FieldsTable};
#[cfg(feature = "movement")]
pub use tables::movement::{DEFAULT_MOVEMENT_ROW_GROUP_SIZE, MovementTable, MovementWriter};
#[cfg(feature = "net-guids")]
pub use tables::net_guids::{DEFAULT_NET_GUID_ROW_GROUP_SIZE, NetGuidWriter, NetGuidsTable};
#[cfg(feature = "parquet")]
pub use writer::{Table, TableWriter};
