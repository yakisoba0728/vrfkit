//! Columnar (Parquet) output for decoded replay records.
//!
//! # Why Parquet
//!
//! A single VALORANT replay produces ~780 000 content-block field records and
//! ~2.4 million movement samples. The predecessor NDJSON pipeline spent **84 %**
//! of wall time just parsing JSON. Parquet eliminates that: downstream consumers
//! (Python/pandas, DuckDB, Spark) memory-map the file and decode only the
//! columns they need, typically 10-50x faster for selective queries.
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
//! strings (typically <500 distinct values over millions of rows). Dictionary
//! encoding stores the distinct values once and references them by index,
//! shrinking data pages by 50-200x.
//!
//! ## Row-group streaming
//!
//! Writers buffer rows in memory until a configurable threshold (default 128 Ki
//! rows) and then flush a row group. This bounds memory to ~tens of MB even for
//! multi-million-row exports, while keeping row groups large enough for
//! efficient column-chunk compression and predicate pushdown.
//!
//! # Public API
//!
//! - [`FieldWriter`] -- streams decoded-field and whole-block preservation
//!   records to a Parquet file.
//! - [`MovementWriter`] -- streams movement samples to a Parquet file.
//! - [`FieldRecord`] / [`MovementRecord`] -- the input structs.
//! - [`ExportError`] -- all fallible operations funnel through this.

#![forbid(unsafe_code)]

mod actor_writer;
mod error;
mod event_writer;
mod field_writer;
mod movement_writer;
mod net_guid_writer;
mod schema;

pub use actor_writer::{ActorRecord, ActorWriter};
pub use error::ExportError;
pub use event_writer::{EventRecord, EventWriter};
pub use field_writer::{FieldRecord, FieldWriter, UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME};
pub use movement_writer::{MovementRecord, MovementWriter};
pub use net_guid_writer::{NetGuidRecord, NetGuidWriter};
