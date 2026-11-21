//! Error types for the replication layer.

use vrf_bitio::BitError;

/// Errors that can occur while parsing the replication stream.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    #[error("bit read error: {0}")]
    Bit(#[from] BitError),

    #[error("malformed packet: last byte is zero (no sentinel)")]
    MalformedPacket,

    #[error("partial bunch sequence error on channel {channel}: {kind}")]
    PartialSequence {
        channel: u32,
        kind: PartialSequenceKind,
    },

    #[error("content block payload overrun: declared {declared} bits, only {available} remain")]
    PayloadOverrun { declared: u64, available: u64 },

    #[error("net GUID recursion depth exceeded {depth}")]
    GuidRecursionLimit { depth: u32 },

    #[error("unsupported replay branch: {0}")]
    UnsupportedBranch(#[from] vrf_transform::UnsupportedBranch),

    /// An error rather than `Ok`, so the path declarations it drops are counted.
    #[error("package-map export declared {count} GUIDs (max {max})")]
    InvalidGuidCount { count: i32, max: u32 },

    /// Function count 0 is an unresolved group, not a class with no functions.
    #[error("ClassNetCache block for an unresolved group: function count unknown")]
    UnresolvedFunctionCount,
}

/// Sub-classification of partial bunch sequence and input errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialSequenceKind {
    /// A continuation arrived without a preceding initial fragment.
    MissingInitial,
    /// A new initial arrived while a previous (incomplete) partial was in flight.
    OverlappingInitial,
    /// Reliability or sequence number mismatch on a continuation.
    MismatchedContinuation,
    /// A non-final fragment's payload was not byte-aligned.
    NonByteAlignedFragment,
    /// The supplied bytes cannot hold the declared payload bit count.
    PayloadOverrun,
}

impl core::fmt::Display for PartialSequenceKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingInitial => write!(f, "continuation without initial"),
            Self::OverlappingInitial => write!(f, "overlapping initial"),
            Self::MismatchedContinuation => write!(f, "mismatched continuation"),
            Self::NonByteAlignedFragment => write!(f, "non-byte-aligned non-final fragment"),
            Self::PayloadOverrun => write!(f, "payload shorter than declared bit count"),
        }
    }
}

pub type Result<T> = core::result::Result<T, NetError>;
