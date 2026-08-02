//! Error types for the `.vrf` container parser.
//!
//! Every failure mode is explicitly typed -- no panics, no silent zeros. A
//! truncated or malformed file produces a descriptive error that names the
//! field and the byte counts involved.

use thiserror::Error;

/// All errors the container parser can produce.
///
/// Designed for match-based handling: callers can distinguish "wrong magic" from
/// "truncated" from "Oodle failure" without string inspection.
#[derive(Debug, Error)]
pub enum ContainerError {
    /// The 4-byte file magic at offset 0 did not match `0x43F4EFDD`.
    #[error("file magic mismatch: expected 0x43F4EFDD, got 0x{actual:08X}")]
    FileMagicMismatch { actual: u32 },

    /// The 4-byte network magic inside the Header chunk did not match `0x2CF5A13D`.
    #[error("network magic mismatch: expected 0x2CF5A13D, got 0x{actual:08X}")]
    NetworkMagicMismatch { actual: u32 },

    /// The legacy file version is not 7.
    #[error("unsupported file version: expected 7, got {actual}")]
    UnsupportedFileVersion { actual: u32 },

    /// The network version is not 19.
    #[error("unexpected network version: expected 19, got {actual}")]
    UnexpectedNetworkVersion { actual: u32 },

    /// The engine network protocol version is not 32.
    #[error("unexpected engine network protocol version: expected 32, got {actual}")]
    UnexpectedEngineNetProtoVersion { actual: u32 },

    /// The custom version container is missing the required `LocalFileReplay` GUID.
    #[error("missing LocalFileReplay custom version (GUID 95A4F03E-7E0B-49E4-BA43-D35694FF87D9)")]
    MissingLocalReplayVersion,

    /// The `LocalFileReplay` custom version is not 7.
    #[error("unsupported LocalFileReplay version: expected 7, got {actual}")]
    UnsupportedLocalReplayVersion { actual: i32 },

    /// An unregistered GUID was found in the custom version container.
    #[error("unregistered custom version GUID: {guid:08X?}")]
    UnregisteredCustomVersion { guid: [u32; 4] },

    /// Duplicate GUID in the custom version container.
    #[error("duplicate custom version GUID")]
    DuplicateCustomVersion,

    /// A completed (non-live) replay is marked encrypted but carries no key.
    #[error("completed replay is marked encrypted but has no encryption key")]
    EncryptedWithoutKey,

    /// Encrypted replays are not supported.
    #[error("encrypted replay data is not supported")]
    EncryptedNotSupported,

    /// The chunk stream contained a ReplayData chunk before the Header chunk.
    #[error("replay data encountered before header chunk")]
    DataBeforeHeader,

    /// No Header chunk was found in the chunk stream.
    #[error("no header chunk found in chunk stream")]
    MissingHeaderChunk,

    /// A chunk's declared size is negative.
    #[error("invalid chunk size: {size}")]
    InvalidChunkSize { size: i32 },

    /// A MemorySizeInBytes field is out of the valid range.
    #[error("invalid memory size: {size} (must be 0..256 MiB)")]
    InvalidMemorySize { size: i32 },

    /// An uncompressed chunk's SizeInBytes != MemorySizeInBytes.
    #[error("uncompressed size mismatch: SizeInBytes={size}, MemorySizeInBytes={memory_size}")]
    SizeMismatch { size: i32, memory_size: i32 },

    /// The Oodle archive header needs at least 8 bytes but the chunk is smaller.
    #[error("compressed chunk too small for Oodle header: SizeInBytes={size}, need >= 8")]
    OodleHeaderTooSmall { size: i32 },

    /// The decompressed size in the Oodle header doesn't match MemorySizeInBytes.
    #[error("Oodle decompressed size {archive_size} != MemorySizeInBytes {memory_size}")]
    OodleDecompressedSizeMismatch { archive_size: i32, memory_size: i32 },

    /// The compressed size in the Oodle header doesn't match SizeInBytes - 8.
    #[error("Oodle compressed size {archive_size} != expected {expected}")]
    OodleCompressedSizeMismatch { archive_size: i32, expected: i32 },

    /// Oodle decompression returned fewer bytes than expected.
    #[error("Oodle output size mismatch: expected {expected}, got {actual}")]
    OodleOutputSizeMismatch { expected: usize, actual: usize },

    /// Oodle decompression failed.
    #[error("Oodle decompression error: {0}")]
    OodleDecompression(String),

    /// A count field exceeds its allowed maximum.
    #[error("{field}: count {count} exceeds maximum {max}")]
    CountOverflow {
        field: &'static str,
        count: i32,
        max: i32,
    },

    /// The input was too short to read the required field.
    #[error("{context}: needed {needed} bytes, only {available} available")]
    Truncated {
        context: &'static str,
        needed: usize,
        available: usize,
    },

    /// A lower-level bit-IO error propagated up.
    #[error("bit-IO error: {0}")]
    BitIo(String),
}
