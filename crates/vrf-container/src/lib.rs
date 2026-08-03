//! `.vrf` container parser: replay info, header, chunk stream, and Oodle decompression.
//!
//! # Container layout
//!
//! A VALORANT replay file (`.vrf`) uses Unreal Engine's local-file replay format.
//! It starts with a fixed **replay info** section, followed by a stream of typed
//! **chunks**. The first chunk must always be a **Header** chunk.
//!
//! ```text
//! +-------------------------------------------+
//! | ReplayInfo                                |
//! |  +- FileMagic (0x43F4EFDD)                |
//! |  +- LegacyFileVersion (7)                 |
//! |  +- CustomVersionContainer                |
//! |  +- Summary (LengthInMs ... EncryptionKey)|
//! +-------------------------------------------+
//! | Chunk 0 (Header)                          |
//! |  +- ChunkType (u32) + SizeInBytes (i32)   |
//! |  +- Header payload                        |
//! +-------------------------------------------+
//! | Chunk 1..N (ReplayData/Checkpoint/Event)  |
//! |  +- ChunkType (u32) + SizeInBytes (i32)   |
//! |  +- Payload (possibly Oodle-compressed)   |
//! +-------------------------------------------+
//! ```
//!
//! # Chunk iteration
//!
//! Use [`ChunkIterator`] to walk chunks lazily without buffering the whole file.
//! For the common case (parse info + header, then iterate data chunks), see
//! [`parse_preamble`] which returns both the parsed preamble and a positioned
//! iterator ready for the remaining chunks.
//!
//! # Oodle decompression
//!
//! ReplayData chunk payloads are compressed with Oodle (Kraken/Mermaid/Selkie).
//! Use [`decompress_replay_data`] to decode a raw chunk payload into plaintext
//! bytes suitable for packet framing.
//!
//! # Event chunks
//!
//! Event chunk payloads are uncompressed and carry the server's own labelled
//! game timeline. Use [`parse_event_chunk`] to read one into an [`EventChunk`];
//! its inner payload is handed back raw, for the reason documented there.

#![forbid(unsafe_code)]

use vrf_bitio::BitReader;

mod error;
mod event;
mod header;
mod info;

pub use error::ContainerError;
pub use event::{EventChunk, parse_event_chunk};
pub use header::{ReplayHeader, ReplayVersion};
pub use info::ReplayInfo;

// --- Constants ----------------------------------------------------------------

/// File-level magic number.
///
/// Source: `ReplayInfoReader.cs` -- `private const uint FileMagic = 0x43F4EFDD`.
/// Present at byte offset 0 of every `.vrf` file.
const FILE_MAGIC: u32 = 0x43F4_EFDD;

/// Network-level magic number inside the Header chunk.
///
/// Source: `Constants.cs` -- `public const uint NetworkMagic = 0x2CF5A13D`.
const NETWORK_MAGIC: u32 = 0x2CF5_A13D;

/// Expected legacy file version in the replay info section.
///
/// Source: `LocalFileReplayCustomVersions.cs` -- version 7 is the only supported
/// value; older versions used a different serialisation layout.
const EXPECTED_FILE_VERSION: u32 = 7;

/// Expected network version in both the info section and the header chunk.
///
/// Source: `Constants.cs` -- `public const uint ExpectedNetworkVersion = 19`.
const EXPECTED_NETWORK_VERSION: u32 = 19;

/// Expected engine network protocol version inside the Header chunk.
///
/// Source: `Constants.cs` --
/// `public const uint ExpectedEngineNetworkProtocolVersion = 32`.
const EXPECTED_ENGINE_NET_PROTO_VERSION: u32 = 32;

/// Maximum sane custom version count (guard against corrupt length fields).
///
/// Source: `Constants.cs` -- `public const int MaxCustomVersionCount = 1024`.
const MAX_CUSTOM_VERSION_COUNT: i32 = 1024;

/// Maximum byte count for a serialised FString (friendly name, branch, etc).
///
/// Source: `Constants.cs` -- `public const int MaxFStringSerializedBytes = 1024 * 1024`.
/// The info's FriendlyName uses a tighter 64 KiB limit (see info module).
const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

/// Maximum byte count for a serialised encryption key.
///
/// Source: `ReplayInfoReader.cs` -- `MaxEncryptionKeySizeBytes = 4096`.
const MAX_ENCRYPTION_KEY_BYTES: i32 = 4096;

/// Maximum byte count for the replay info's FriendlyName.
///
/// Source: `ReplayInfoReader.cs` -- `MaxFriendlyNameSerializedBytes = 64 * 1024`.
/// This is tighter than the general FString limit because the friendly name
/// is user-controlled metadata.
const MAX_FRIENDLY_NAME_BYTES: i64 = 64 * 1024;

/// Maximum number of level name entries in the header.
///
/// Source: `ReplayHeaderReader.cs` -- `MaxLevelNamesAndTimes = 1024`.
const MAX_LEVEL_NAMES_AND_TIMES: i32 = 1024;

/// Maximum number of game-specific data strings in the header.
///
/// Source: `ReplayHeaderReader.cs` -- `MaxGameSpecificDataEntries = 128`.
const MAX_GAME_SPECIFIC_DATA: i32 = 128;

/// Maximum decompressed chunk size (256 MiB). Guards against corrupt size fields
/// causing unbounded allocation.
///
/// Source: `ReplayDataChunkPayloadReader.cs` -- `MaxChunkSize = 1024 * 1024 * 256`.
const MAX_CHUNK_SIZE: i32 = 256 * 1024 * 1024;

/// Each custom version entry is a 16-byte GUID + 4-byte version number = 20 bytes.
///
/// Source: `ReplayHeaderReader.cs` -- `CustomVersionEntryByteCount = 20`.
const CUSTOM_VERSION_ENTRY_BYTES: u32 = 20;

/// The expected GUID for the `LocalFileReplay` custom version.
///
/// Source: `LocalFileReplayCustomVersions.cs` --
/// `Guid.Parse("95A4F03E-7E0B-49E4-BA43-D35694FF87D9")`.
///
/// Stored as four little-endian u32 in Unreal's GUID serialisation order.
const LOCAL_REPLAY_GUID: [u32; 4] = [0x95A4_F03E, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9];

/// Expected `LocalFileReplay` custom version number.
///
/// Source: `LocalFileReplayCustomVersions.cs` -- `public const int CustomVersions = 7`.
const LOCAL_REPLAY_VERSION: i32 = 7;

// --- Chunk types --------------------------------------------------------------

/// Discriminant for the framing chunks that follow the replay info.
///
/// Source: `ReplayChunkType.cs`.
///
/// | Value | Meaning |
/// |-------|---------|
/// | 0 | Header -- must be the first chunk |
/// | 1 | ReplayData -- compressed playback packets |
/// | 2 | Checkpoint |
/// | 3 | Event |
/// | 0xFFFFFFFF | Unknown / padding |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkType {
    Header,
    ReplayData,
    Checkpoint,
    Event,
    Unknown(u32),
}

impl ChunkType {
    /// Convert a raw u32 to the typed enum.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Header,
            1 => Self::ReplayData,
            2 => Self::Checkpoint,
            3 => Self::Event,
            other => Self::Unknown(other),
        }
    }

    /// Convert back to the wire representation.
    #[must_use]
    pub const fn to_raw(self) -> u32 {
        match self {
            Self::Header => 0,
            Self::ReplayData => 1,
            Self::Checkpoint => 2,
            Self::Event => 3,
            Self::Unknown(v) => v,
        }
    }
}

// --- Raw chunk descriptor -----------------------------------------------------

/// A single chunk's type and byte range, without owning its payload.
///
/// Use this to index the chunk stream or skip chunks you don't need.
#[derive(Debug, Clone)]
pub struct RawChunk {
    /// Discriminant read from the stream.
    pub chunk_type: ChunkType,
    /// Declared payload size in bytes (may be zero for empty chunks).
    pub size_in_bytes: i32,
    /// Byte offset of the *payload* within the input slice (past the 8-byte
    /// chunk header: 4 bytes type + 4 bytes size).
    pub data_offset: usize,
}

// --- ReplayData payload descriptor --------------------------------------------

/// Parsed metadata from a ReplayData chunk's inner framing.
///
/// ```text
/// +------------------------------------------------+
/// | u32 Time1                                      |
/// | u32 Time2                                      |
/// | i32 SizeInBytes      (compressed payload size) |
/// | i32 MemorySizeInBytes (decompressed size)      |
/// | [SizeInBytes] data                             |
/// +------------------------------------------------+
/// ```
#[derive(Debug, Clone)]
pub struct ReplayDataMeta {
    /// First timestamp (server tick at chunk start).
    pub time1: u32,
    /// Second timestamp (server tick at chunk end).
    pub time2: u32,
    /// On-disk (compressed) size of the data blob.
    pub size_in_bytes: i32,
    /// Decompressed size -- allocate this many bytes for Oodle output.
    pub memory_size_in_bytes: i32,
}

// --- Chunk iterator -----------------------------------------------------------

/// Lazy iterator over the chunk stream following the replay info.
///
/// Each call to [`next`](ChunkIterator::next) yields the next chunk's metadata
/// and advances past its payload. The caller decides whether to inspect the
/// payload (accessible via the returned `RawChunk.data_offset` into the original
/// buffer) or skip it.
pub struct ChunkIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ChunkIterator<'a> {
    /// Create an iterator starting at byte position `offset` within `data`.
    ///
    /// `offset` should point to the first chunk header (immediately after the
    /// replay info section).
    #[must_use]
    pub const fn new(data: &'a [u8], offset: usize) -> Self {
        Self { data, pos: offset }
    }

    /// Current byte position in the underlying buffer.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Whether the iterator has reached the end of the buffer.
    #[must_use]
    pub const fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Read the next chunk header, advance past its payload, and return metadata.
    ///
    /// Returns `None` when the buffer is exhausted. Returns an error if the
    /// remaining bytes are too few for a chunk header or if the declared size
    /// would exceed the buffer.
    pub fn next_chunk(&mut self) -> Result<Option<RawChunk>, ContainerError> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        // Need at least 8 bytes for the chunk header (u32 type + i32 size).
        if self.data.len() - self.pos < 8 {
            return Err(ContainerError::Truncated {
                context: "chunk header",
                needed: 8,
                available: self.data.len() - self.pos,
            });
        }

        let mut reader = BitReader::new(&self.data[self.pos..]);
        let raw_type = reader
            .read_u32()
            .map_err(|e| ContainerError::BitIo(e.to_string()))?;
        let size = reader
            .read_i32()
            .map_err(|e| ContainerError::BitIo(e.to_string()))?;

        if size < 0 {
            return Err(ContainerError::InvalidChunkSize { size });
        }
        let size_usize = size as usize;
        let data_offset = self.pos + 8;

        if data_offset + size_usize > self.data.len() {
            return Err(ContainerError::Truncated {
                context: "chunk payload",
                needed: size_usize,
                available: self.data.len() - data_offset,
            });
        }

        let chunk = RawChunk {
            chunk_type: ChunkType::from_raw(raw_type),
            size_in_bytes: size,
            data_offset,
        };

        self.pos = data_offset + size_usize;
        Ok(Some(chunk))
    }
}

// --- Preamble (info + header) -------------------------------------------------

/// Result of parsing the preamble: info, header, and an iterator positioned at
/// the first post-header chunk.
#[derive(Debug)]
pub struct Preamble {
    pub info: ReplayInfo,
    pub header: ReplayHeader,
    /// Byte offset where the remaining chunks start (after the header chunk).
    pub remaining_offset: usize,
}

/// Parse the replay info and first (Header) chunk, returning the structured
/// preamble and the byte offset where subsequent chunks begin.
///
/// This is the primary entry point for reading a `.vrf` file.
///
/// # Errors
///
/// Returns [`ContainerError`] if magic numbers don't match, required fields are
/// missing, or the data is truncated.
pub fn parse_preamble(data: &[u8]) -> Result<Preamble, ContainerError> {
    let (replay_info, info_end) = info::parse_replay_info(data)?;

    // The first chunk must be Header.
    let mut iter = ChunkIterator::new(data, info_end);
    loop {
        let chunk = iter
            .next_chunk()?
            .ok_or(ContainerError::MissingHeaderChunk)?;

        match chunk.chunk_type {
            ChunkType::Header => {
                let payload =
                    &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
                let replay_header = header::parse_replay_header(payload)?;
                return Ok(Preamble {
                    info: replay_info,
                    header: replay_header,
                    remaining_offset: iter.position(),
                });
            }
            ChunkType::ReplayData => {
                return Err(ContainerError::DataBeforeHeader);
            }
            _ => {
                // Skip non-header, non-data chunks before the header (e.g. Unknown).
            }
        }
    }
}

// --- ReplayData decompression -------------------------------------------------

/// Parse the inner framing of a ReplayData chunk payload and return metadata.
///
/// The payload bytes are the region `data[chunk.data_offset .. + chunk.size_in_bytes]`
/// from a [`RawChunk`] of type [`ChunkType::ReplayData`].
///
/// # Layout
///
/// | Offset | Type | Field |
/// |--------|------|-------|
/// | 0 | u32 | Time1 |
/// | 4 | u32 | Time2 |
/// | 8 | i32 | SizeInBytes (compressed payload) |
/// | 12 | i32 | MemorySizeInBytes (decompressed) |
/// | 16 | [u8] | compressed data |
pub fn parse_replay_data_meta(payload: &[u8]) -> Result<ReplayDataMeta, ContainerError> {
    if payload.len() < 16 {
        return Err(ContainerError::Truncated {
            context: "replay data meta",
            needed: 16,
            available: payload.len(),
        });
    }
    let mut reader = BitReader::new(payload);
    let time1 = reader
        .read_u32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let time2 = reader
        .read_u32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let size_in_bytes = reader
        .read_i32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let memory_size_in_bytes = reader
        .read_i32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    if !(0..=MAX_CHUNK_SIZE).contains(&memory_size_in_bytes) {
        return Err(ContainerError::InvalidMemorySize {
            size: memory_size_in_bytes,
        });
    }

    Ok(ReplayDataMeta {
        time1,
        time2,
        size_in_bytes,
        memory_size_in_bytes,
    })
}

/// Decompress a ReplayData chunk payload using Oodle.
///
/// `payload` is the full chunk payload (starting at Time1). `compressed` is the
/// `ReplayInfo.compressed` flag from the preamble.
///
/// When `compressed` is false, the data portion is returned as-is (after
/// validating that `SizeInBytes == MemorySizeInBytes`).
///
/// When `compressed` is true, the data portion contains an Oodle archive:
///
/// | Offset (relative to data start) | Type | Field |
/// |---|---|---|
/// | 0 | i32 | decompressed_size (must == MemorySizeInBytes) |
/// | 4 | i32 | compressed_size (must == SizeInBytes - 8) |
/// | 8 | [u8] | Oodle-compressed bytes |
///
/// # Errors
///
/// Returns [`ContainerError`] for size mismatches, truncation, or Oodle failures.
pub fn decompress_replay_data(
    payload: &[u8],
    compressed: bool,
    encrypted: bool,
) -> Result<Vec<u8>, ContainerError> {
    if encrypted {
        return Err(ContainerError::EncryptedNotSupported);
    }

    let meta = parse_replay_data_meta(payload)?;

    // Data starts at byte 16 of the payload.
    let data_start = 16usize;
    let data_bytes = &payload[data_start..];

    if !compressed {
        // Uncompressed: sizes must match.
        if meta.size_in_bytes != meta.memory_size_in_bytes {
            return Err(ContainerError::SizeMismatch {
                size: meta.size_in_bytes,
                memory_size: meta.memory_size_in_bytes,
            });
        }
        let size = meta.size_in_bytes as usize;
        if data_bytes.len() < size {
            return Err(ContainerError::Truncated {
                context: "uncompressed replay data",
                needed: size,
                available: data_bytes.len(),
            });
        }
        return Ok(data_bytes[..size].to_vec());
    }

    // Compressed: Oodle archive header (8 bytes) + compressed payload.
    if meta.size_in_bytes < 8 {
        return Err(ContainerError::OodleHeaderTooSmall {
            size: meta.size_in_bytes,
        });
    }

    if data_bytes.len() < 8 {
        return Err(ContainerError::Truncated {
            context: "oodle archive header",
            needed: 8,
            available: data_bytes.len(),
        });
    }

    let mut hdr_reader = BitReader::new(&data_bytes[..8]);
    let decompressed_size = hdr_reader
        .read_i32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let compressed_size = hdr_reader
        .read_i32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    if decompressed_size != meta.memory_size_in_bytes {
        return Err(ContainerError::OodleDecompressedSizeMismatch {
            archive_size: decompressed_size,
            memory_size: meta.memory_size_in_bytes,
        });
    }

    let expected_compressed_size = meta.size_in_bytes - 8;
    if compressed_size != expected_compressed_size {
        return Err(ContainerError::OodleCompressedSizeMismatch {
            archive_size: compressed_size,
            expected: expected_compressed_size,
        });
    }

    let compressed_data = &data_bytes[8..];
    if compressed_data.len() < compressed_size as usize {
        return Err(ContainerError::Truncated {
            context: "oodle compressed data",
            needed: compressed_size as usize,
            available: compressed_data.len(),
        });
    }

    let input = &compressed_data[..compressed_size as usize];
    let mut output = vec![0u8; meta.memory_size_in_bytes as usize];

    let mut extractor = oozextract::Extractor::new();
    let n = extractor
        .read_from_slice(input, &mut output)
        .map_err(|e| ContainerError::OodleDecompression(format!("{e:?}")))?;

    if n != meta.memory_size_in_bytes as usize {
        return Err(ContainerError::OodleOutputSizeMismatch {
            expected: meta.memory_size_in_bytes as usize,
            actual: n,
        });
    }

    Ok(output)
}

// --- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests;
