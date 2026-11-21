//! Schema errors: explicit variants, never panics, so a corrupt replay can be
//! told apart from a parser bug.

use vrf_bitio::BitError;

/// Errors that can occur while reading or maintaining the dynamic schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The underlying bit stream was truncated or malformed.
    #[error("bit-level read failed: {0}")]
    Bitio(#[from] BitError),

    /// The stream is corrupt or out of order.
    #[error("net-field export references unknown path name index {index}")]
    UnknownPathIndex { index: u32 },

    /// The cursor is misaligned: a reference would read the path's bytes as a field.
    #[error(
        "net-field export at path name index {path_name_index}: isExported is {value}, expected 0 or 1"
    )]
    BadExportedFlag { path_name_index: u32, value: u32 },

    /// A live group declared more slots than the checkpoint form permits.
    #[error("net-field export declares {count} field slots, maximum is {max}")]
    FieldCountOverflow { count: u32, max: u32 },

    /// A declaration would exceed a schema resource budget. This is a policy
    /// refusal, not a claim about the game's format; the resource names the
    /// configured cache limit or the checkpoint path-expansion limit.
    #[error("schema {resource} require {requested}, configured limit is {limit}")]
    ResourceLimit {
        resource: &'static str,
        requested: usize,
        limit: usize,
    },

    /// `path_group` and `index_group` are the canonical paths the incoming path
    /// and index select.
    #[error(
        "net-field export path '{path}' identifies '{path_group}', but index {path_name_index} identifies '{index_group}'"
    )]
    CrossedExportGroupIdentity {
        path: String,
        path_name_index: u32,
        path_group: String,
        index_group: String,
    },

    #[error("export GUID payload size is negative: {size}")]
    NegativePayloadSize { size: i32 },

    #[error("export GUID payload has {remaining} trailing byte(s)")]
    TrailingPayloadData { remaining: usize },

    #[error("net GUID object recursion depth exceeded {limit}")]
    RecursionLimitExceeded { limit: u32 },

    /// Only 0 and 1 occur across 17,186,645 corpus entries: a third value is a
    /// misaligned cursor.
    #[error("checkpoint guid entry {entry}: path discriminator is {byte}, expected 0 or 1")]
    CheckpointBadPathKind { entry: u32, byte: u8 },

    #[error(
        "checkpoint guid entry {entry}: path index {index} exceeds {literals} preceding literals"
    )]
    CheckpointPathIndexOutOfBounds {
        entry: u32,
        index: u32,
        literals: u32,
    },

    /// `handle == slot` holds for all 11,529,869 exported corpus slots; a
    /// mismatch would attach real names to the wrong handles.
    #[error("checkpoint group '{group}' slot {slot}: declared handle {handle}")]
    CheckpointHandleNotSlot {
        group: String,
        slot: u32,
        handle: u32,
    },

    /// `map_end == prologue offset + 8` holds for all 4,024 corpus checkpoints:
    /// the only end-to-end check on both table parses.
    #[error("checkpoint tables ended at {map_end}, prologue implies {expected}")]
    CheckpointFrameOffsetMismatch { map_end: usize, expected: usize },

    /// A reserved prologue word (bytes 4, 8, 12; zero in all 4,024 corpus
    /// checkpoints) is an unknown field, and every later offset is suspect.
    #[error("checkpoint prologue word at byte {offset} is {value}, expected 0")]
    CheckpointReservedWordSet { offset: usize, value: u32 },

    #[error("checkpoint {field}: count {count} exceeds maximum {max}")]
    CheckpointCountOverflow {
        field: &'static str,
        count: u32,
        max: u32,
    },

    /// A path or index declared twice in one checkpoint; `path` is the later one.
    #[error("checkpoint export group '{path}' collides at path-name index {path_name_index}")]
    CheckpointGroupCollision { path: String, path_name_index: u32 },
}

pub type Result<T> = core::result::Result<T, SchemaError>;
