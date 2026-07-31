//! Error types for schema operations.
//!
//! Every failure is an explicit variant rather than a panic, because a corrupt or
//! truncated replay must be distinguishable from a logic error in the parser.

use vrf_bitio::BitError;

/// Errors that can occur while reading or maintaining the dynamic schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// The underlying bit stream was truncated or malformed.
    #[error("bit-level read failed: {0}")]
    Bitio(#[from] BitError),

    /// A net-field export references a `path_name_index` that has never been
    /// registered. This means the stream is either corrupt or out-of-order.
    #[error("net-field export references unknown path name index {index}")]
    UnknownPathIndex {
        /// The unresolved index.
        index: u32,
    },

    /// An export GUID payload declared a negative size.
    #[error("export GUID payload size is negative: {size}")]
    NegativePayloadSize {
        /// The rejected size value.
        size: i32,
    },

    /// The export GUID payload was not fully consumed after reading.
    #[error("export GUID payload has {remaining} trailing byte(s)")]
    TrailingPayloadData {
        /// Bytes left over.
        remaining: usize,
    },

    /// NetGUID object recursion exceeded the safety limit.
    #[error("net GUID object recursion depth exceeded {limit}")]
    RecursionLimitExceeded {
        /// The configured maximum.
        limit: u32,
    },
}

/// Result alias for schema operations.
pub type Result<T> = core::result::Result<T, SchemaError>;
