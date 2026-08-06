//! Error types for the replication layer.

use vrf_bitio::BitError;

/// Errors that can occur while parsing the replication stream.
///
/// All variants carry enough context to locate the failure in the stream.
/// None of them panic; a malformed bunch is discarded and counted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// A bit-level read failed (EOF or malformed primitive).
    #[error("bit read error: {0}")]
    Bit(#[from] BitError),

    /// The packet's last byte was zero, so the sentinel cannot be found.
    #[error("malformed packet: last byte is zero (no sentinel)")]
    MalformedPacket,

    /// A partial bunch fragment violated sequence rules.
    #[error("partial bunch sequence error on channel {channel}: {kind}")]
    PartialSequence {
        /// Channel index where the error occurred.
        channel: u32,
        /// Description of the violation.
        kind: PartialSequenceKind,
    },

    /// Content block payload declared more bits than remain in the stream.
    #[error("content block payload overrun: declared {declared} bits, only {available} remain")]
    PayloadOverrun {
        /// Declared bit count.
        declared: u64,
        /// Bits actually remaining.
        available: u64,
    },

    /// InternalLoadObject recursion exceeded the safety limit.
    #[error("net GUID recursion depth exceeded {depth}")]
    GuidRecursionLimit {
        /// The depth at which recursion was halted.
        depth: u32,
    },

    /// An unsupported replay branch was encountered for payload transform.
    #[error("unsupported replay branch: {0}")]
    UnsupportedBranch(#[from] vrf_transform::UnsupportedBranch),

    /// A package-map export bunch declared an impossible GUID count.
    ///
    /// Negative, or above [`MAX_GUID_COUNT`](crate::types::MAX_GUID_COUNT). The
    /// declarations after it cannot be walked, so the bunch is abandoned -- as
    /// an error rather than a quiet `Ok`, because every path declaration in it
    /// is lost and actors that needed those paths would otherwise fail to
    /// resolve with nothing pointing at the cause.
    #[error("package-map export declared {count} GUIDs (max {max})")]
    InvalidGuidCount {
        /// The declared count, as read from the wire.
        count: i32,
        /// The accepted maximum.
        max: u32,
    },

    /// A ClassNetCache block arrived for a group whose function count is unknown.
    ///
    /// Distinguished from a class that genuinely has no functions: zero means the
    /// export group could not be resolved, so the handle width is unknown and the
    /// record stream cannot be walked at all. This is an error rather than a quiet
    /// return so the bits are counted and the group is named, instead of the
    /// payload disappearing with the oracle still reporting a clean run.
    #[error("ClassNetCache block for an unresolved group: function count unknown")]
    UnresolvedFunctionCount,
}

/// Sub-classification of partial bunch sequence errors.
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
}

impl core::fmt::Display for PartialSequenceKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingInitial => write!(f, "continuation without initial"),
            Self::OverlappingInitial => write!(f, "overlapping initial"),
            Self::MismatchedContinuation => write!(f, "mismatched continuation"),
            Self::NonByteAlignedFragment => write!(f, "non-byte-aligned non-final fragment"),
        }
    }
}

pub type Result<T> = core::result::Result<T, NetError>;
