//! The chunk stream that follows the replay info: typed discriminants, raw
//! byte ranges, and the lazy iterator that walks them.
//!
//! Nothing here owns a payload. [`ChunkIterator`] hands back a [`RawChunk`]
//! naming a byte range in the caller's buffer, which is what lets a consumer
//! skip whole chunk types (all 195 Event chunks, say) without paying to
//! materialise them.

use crate::error::ContainerError;

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

/// Lazy iterator over the chunk stream following the replay info.
///
/// Each call to [`next`](ChunkIterator::next_chunk) yields the next chunk's
/// metadata and advances past its payload. The caller decides whether to
/// inspect the payload (accessible via the returned `RawChunk.data_offset` into
/// the original buffer) or skip it.
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
        let available = match self.data.len().checked_sub(self.pos) {
            None | Some(0) => return Ok(None),
            Some(n) => n,
        };
        // Need at least 8 bytes for the chunk header (u32 type + i32 size).
        if available < 8 {
            return Err(ContainerError::Truncated {
                context: "chunk header",
                needed: 8,
                available,
            });
        }

        // The two header fields are a fixed 8 bytes at a known offset, so they
        // are decoded directly rather than through a `BitReader`. The `available
        // >= 8` check above is what makes the conversion succeed; the `else`
        // arm is unreachable and re-reports truncation rather than panicking.
        let Ok(header) = <[u8; 8]>::try_from(&self.data[self.pos..self.pos + 8]) else {
            return Err(ContainerError::Truncated {
                context: "chunk header",
                needed: 8,
                available,
            });
        };
        let raw_type = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let size = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);

        if size < 0 {
            return Err(ContainerError::InvalidChunkSize { size });
        }
        let size_usize = size as usize;
        let data_offset = self.pos + 8;

        // `data_offset + size_usize` can overflow on a 32-bit target if the
        // declared size is near usize::MAX, so compare against what is left
        // instead of computing the end offset.
        if size_usize > self.data.len() - data_offset {
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
