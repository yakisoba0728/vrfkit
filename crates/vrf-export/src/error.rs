//! Crate-wide error type.
//!
//! All public fallible operations return [`ExportError`]. The surface is
//! deliberately small -- IO and Parquet are the two failure domains, plus a
//! caller-misuse variant.
//!
//! It is `#[non_exhaustive]` because one of those variants is feature-gated:
//! the same `match` has to compile whether or not the build has a Parquet
//! writer in it, so a caller needs a wildcard arm and the type has to say so.

use thiserror::Error;

/// Errors that can occur while writing export files.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExportError {
    /// The underlying Parquet writer encountered a codec, schema, or IO error.
    ///
    /// Only present with the `parquet` feature: without it the crate has no
    /// writers, and naming the variant would drag the dependency back in.
    /// `#[non_exhaustive]` above is what keeps a caller's `match` compiling
    /// across that boundary.
    #[cfg(feature = "parquet")]
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
