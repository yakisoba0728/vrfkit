//! Error types for the `.vrf` container parser: typed variants, no panics, no
//! silent zeros. `BitIo` and `OodleDecompression` carry only the lower layer's
//! message; every other short read names its field and byte counts.

use thiserror::Error;
use vrf_bitio::BitError;

use crate::chunk::ChunkType;

/// Whole even where no path constructs a variant: removing one breaks
/// downstream matches. Non-exhaustive, so a new failure is not a breaking change.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ContainerError {
    #[error("file magic mismatch: expected 0x43F4EFDD, got 0x{actual:08X}")]
    FileMagicMismatch { actual: u32 },
    #[error("network magic mismatch: expected 0x2CF5A13D, got 0x{actual:08X}")]
    NetworkMagicMismatch { actual: u32 },
    #[error("unsupported file version: expected 7, got {actual}")]
    UnsupportedFileVersion { actual: u32 },
    #[error("unexpected network version: expected 19, got {actual}")]
    UnexpectedNetworkVersion { actual: u32 },
    #[error("unexpected engine network protocol version: expected 32, got {actual}")]
    UnexpectedEngineNetProtoVersion { actual: u32 },
    #[error("missing LocalFileReplay custom version (GUID 95A4F03E-7E0B-49E4-BA43-D35694FF87D9)")]
    MissingLocalReplayVersion,
    #[error("unsupported LocalFileReplay version: expected 7, got {actual}")]
    UnsupportedLocalReplayVersion { actual: i32 },
    /// Never constructed: the info parser ignores custom-version GUIDs it does not pin.
    #[error("unregistered custom version GUID: {guid:08X?}")]
    UnregisteredCustomVersion { guid: [u32; 4] },
    #[error("duplicate custom version GUID")]
    DuplicateCustomVersion,

    #[error("completed replay is marked encrypted but has no encryption key")]
    EncryptedWithoutKey,
    #[error("encrypted replay data is not supported")]
    EncryptedNotSupported,

    #[error("replay data encountered before header chunk")]
    DataBeforeHeader,
    /// Unknown types included: no payload may precede the Header unseen.
    #[error("{chunk_type:?} chunk encountered before header chunk")]
    ChunkBeforeHeader { chunk_type: ChunkType },
    #[error("no header chunk found in chunk stream")]
    MissingHeaderChunk,
    #[error("invalid chunk size: {size}")]
    InvalidChunkSize { size: i32 },
    #[error("invalid memory size: {size} (must be 0..256 MiB)")]
    InvalidMemorySize { size: i32 },
    #[error("invalid event payload size: {size}")]
    InvalidEventPayloadSize { size: i32 },
    #[error("invalid checkpoint archive size: {size}")]
    InvalidCheckpointArchiveSize { size: i32 },
    #[error("uncompressed size mismatch: SizeInBytes={size}, MemorySizeInBytes={memory_size}")]
    SizeMismatch { size: i32, memory_size: i32 },

    #[error("compressed chunk too small for Oodle header: SizeInBytes={size}, need >= 8")]
    OodleHeaderTooSmall { size: i32 },
    #[error("Oodle decompressed size {archive_size} != MemorySizeInBytes {memory_size}")]
    OodleDecompressedSizeMismatch { archive_size: i32, memory_size: i32 },
    /// `expected` is SizeInBytes - 8.
    #[error("Oodle compressed size {archive_size} != expected {expected}")]
    OodleCompressedSizeMismatch { archive_size: i32, expected: i32 },
    #[error("Oodle output size mismatch: expected {expected}, got {actual}")]
    OodleOutputSizeMismatch { expected: usize, actual: usize },
    /// The codec rejected the stream, or panicked on it: a caught panic reads
    /// `decoder panicked: <message>`.
    #[error("Oodle decompression error: {0}")]
    OodleDecompression(String),
    /// Not an empty buffer: a zero-length success would read as an empty chunk.
    #[error(
        "compressed chunk needs {needed} bytes of Oodle output, but this build \
         has the `oodle` feature disabled"
    )]
    OodleUnsupported { needed: usize },

    #[error("{field}: count {count} exceeds maximum {max}")]
    CountOverflow {
        field: &'static str,
        count: i32,
        max: i32,
    },
    #[error("{context}: needed {needed} bytes, only {available} available")]
    Truncated {
        context: &'static str,
        needed: usize,
        available: usize,
    },
    #[error("{context}: FString decode error: {source}")]
    FString {
        context: &'static str,
        #[source]
        source: BitError,
    },
    #[error("bit-IO error: {0}")]
    BitIo(String),
}
