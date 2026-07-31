//! Unified error type for the CLI.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("container error: {0}")]
    Container(#[from] vrf_container::ContainerError),

    #[error("frame error: {0}")]
    Frame(#[from] vrf_frame::FrameError),

    #[error("export error: {0}")]
    Export(#[from] vrf_export::ExportError),

    #[error("{0}")]
    Usage(String),
}
