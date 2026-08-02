//! Crate-wide error type.
//!
//! All public fallible operations return [`ExportError`]. We keep the surface
//! small -- IO and Parquet are the two failure domains -- so callers can match
//! exhaustively without a catch-all.

use thiserror::Error;

/// Errors that can occur while writing export files.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The underlying Parquet writer encountered a codec, schema, or IO error.
    #[error("parquet write failed: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    /// A standard IO error (file creation, flush, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A logical error in the caller's data (e.g. finishing a writer that was
    /// never opened, or pushing a record after close).
    #[error("{0}")]
    Usage(String),
}
