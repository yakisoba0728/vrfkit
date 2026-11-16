//! Unified error type for the CLI.

use thiserror::Error;
use vrf_net::pipeline::ReplicationReader;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("container error: {0}")]
    Container(#[from] vrf_container::ContainerError),

    #[error("frame error: {0}")]
    Frame(#[from] vrf_frame::FrameError),

    #[error("checkpoint {checkpoint}: {error}")]
    CheckpointSchema {
        checkpoint: String,
        #[source]
        error: vrf_schema::SchemaError,
    },

    #[error("export error: {0}")]
    Export(#[from] vrf_export::ExportError),

    #[error("{0}")]
    Usage(String),
}

/// A replication reader for `branch`, or the usage error every subcommand
/// reports for a branch it cannot read. A helper, not a `From` conversion, so
/// no other vrf-net error is ever labelled a branch error.
pub fn replication_reader(branch: &str) -> Result<ReplicationReader, CliError> {
    ReplicationReader::new(branch).map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))
}
