//! The replay info section plus the mandatory first (Header) chunk.
//!
//! These two are parsed together because neither is useful alone: the info
//! section says whether payloads are compressed, and the header says which
//! build recorded the replay and which DemoFrame sections are present. A
//! caller needs both before it can read a single packet.

use crate::chunk::{ChunkIterator, ChunkType};
use crate::error::ContainerError;
use crate::header::{self, ReplayHeader};
use crate::info::{self, ReplayInfo};

/// Result of parsing the preamble: info, header, and the byte offset where the
/// remaining chunks start.
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

    // The first chunk must be Header. Anything else before it is skipped,
    // except ReplayData -- packets that arrive before the schema-bearing header
    // cannot be interpreted, so that ordering is an error rather than a skip.
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
