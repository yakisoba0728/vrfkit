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

    /// A live export group declared more field slots than the protocol's
    /// checkpoint form permits.
    #[error("net-field export declares {count} field slots, maximum is {max}")]
    FieldCountOverflow {
        /// The rejected slot count.
        count: u32,
        /// The configured maximum.
        max: u32,
    },

    /// Reserving storage for a bounded live export group failed.
    #[error("could not reserve {count} net-field export slots")]
    FieldAllocationFailed {
        /// The requested, already-bounded slot count.
        count: u32,
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

    /// A checkpoint guid-cache entry's path discriminator was neither 0 nor 1.
    ///
    /// Only those two values occur across 17,186,645 corpus entries. A third
    /// means the cursor is not where this parser thinks it is.
    #[error("checkpoint guid entry {entry}: path discriminator is {byte}, expected 0 or 1")]
    CheckpointBadPathKind {
        /// Index of the entry being read.
        entry: u32,
        /// The rejected byte.
        byte: u8,
    },

    /// A checkpoint export-group slot declared a handle other than its own
    /// index.
    ///
    /// `handle == slot` holds for all 11,529,869 exported slots in the corpus.
    /// A mismatch means the record stream has desynchronised, and continuing
    /// would attach real names to the wrong handles -- which reads as valid
    /// data and is the failure this project has been bitten by repeatedly.
    #[error("checkpoint group '{group}' slot {slot}: declared handle {handle}")]
    CheckpointHandleNotSlot {
        /// Path of the group being read.
        group: String,
        /// Slot index the handle should have equalled.
        slot: u32,
        /// The handle actually read.
        handle: u32,
    },

    /// The export-group map did not end where the archive prologue said the
    /// DemoFrame begins.
    ///
    /// `map_end == prologue_offset + 8` holds for all 4,024 corpus
    /// checkpoints. This is the only end-to-end check on the two table parses:
    /// a mis-read count lands the cursor somewhere plausible and nothing else
    /// would notice.
    #[error("checkpoint tables ended at {map_end}, prologue implies {expected}")]
    CheckpointFrameOffsetMismatch {
        /// Where parsing actually finished.
        map_end: usize,
        /// Where the prologue said it should.
        expected: usize,
    },

    /// One of the checkpoint prologue's reserved words was non-zero.
    ///
    /// Words at byte 4, 8 and 12 are zero in all 4,024 corpus checkpoints.
    /// A non-zero one means this build writes a field the parser does not know
    /// about, and every offset after it is suspect.
    #[error("checkpoint prologue word at byte {offset} is {value}, expected 0")]
    CheckpointReservedWordSet {
        /// Byte offset of the word.
        offset: usize,
        /// The unexpected value.
        value: u32,
    },

    /// A checkpoint count field exceeded its sanity bound.
    #[error("checkpoint {field}: count {count} exceeds maximum {max}")]
    CheckpointCountOverflow {
        /// Which count overflowed.
        field: &'static str,
        /// The rejected value.
        count: u32,
        /// The configured maximum.
        max: u32,
    },
}

/// Result alias for schema operations.
pub type Result<T> = core::result::Result<T, SchemaError>;
