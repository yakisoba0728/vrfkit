//! Read VALORANT replay metadata and stream decoded records without writing files.
//!
//! [`ReplayReader`] drives the same ordered decode pass as the CLI. Record
//! batches borrow reusable parser buffers; clone only the rows you need to
//! retain. A returned [`ReplaySummary`] reports the work and its limitations,
//! including checkpoint chunks that this API does not decode.
//!
//! ```no_run
//! use std::convert::Infallible;
//! use vrfkit::{ReplayEvent, ReplayReader};
//!
//! let mut fields = 0_u64;
//! let summary = ReplayReader::new().read_path("match.vrf", |event| {
//!     if let ReplayEvent::Records(batch) = event {
//!         fields += batch.fields.len() as u64;
//!     }
//!     Ok::<(), Infallible>(())
//! })?;
//! assert_eq!(fields, summary.field_rows);
//! println!("{:?}", summary.block_verdict());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Disable default features to omit Arrow, Parquet and Zstd. The reader still
//! performs the complete main-stream decode. The optional `export` feature
//! enables the CLI's Parquet writers; it does not change reader semantics.

#![forbid(unsafe_code)]

mod cli;
mod diagnose;
#[cfg(feature = "export")]
mod driver;
mod error;
mod inspect;
#[cfg(feature = "export")]
mod manifest;
mod oracle;
mod pass;
mod reader;
mod reader_input;
mod report;
mod sink;

pub use error::CliError as ReplayError;
pub use oracle::Verdict as ValidationVerdict;
pub use reader::{ReadError, ReplayEvent, ReplayReader, ReplayRecords, ReplaySummary};
pub use vrf_container::{EventChunk, Preamble};
pub use vrf_decode::OverlayErrorRow;
pub use vrf_export::{
    ActorRecord, FieldRecord, MovementProvenance, MovementRecord, MovementSource, NetGuidRecord,
    PartialRecord, UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
};
pub use vrf_frame::FrameSkips;
pub use vrf_net::stats::NetStats;
pub use vrf_schema::{SchemaError, SchemaLimits};

/// Run the existing command-line dispatcher, including its stdout/stderr output.
///
/// `args[0]` is the program name. The result is the command's exit code; a
/// successful dispatch can return a failed validation verdict. Library users
/// wanting records without terminal output should use [`ReplayReader`].
pub fn run_cli(args: &[String]) -> Result<u8, ReplayError> {
    cli::run(args)
}
