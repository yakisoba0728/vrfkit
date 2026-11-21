//! The replay info and the first (Header) chunk: a caller needs both before one
//! packet, for the compression flag, the build and the DemoFrame sections.

use crate::chunk::{ChunkIterator, ChunkType};
use crate::error::ContainerError;
use crate::header::{self, ReplayHeader};
use crate::info::{self, ReplayInfo};
use crate::limits::{MAX_CUSTOM_VERSION_COUNT, MAX_ENCRYPTION_KEY_BYTES, MAX_FRIENDLY_NAME_BYTES};

use std::io::{Read, Seek, SeekFrom};

/// An I/O failure or the unchanged structural error from parsing a preamble.
#[derive(Debug, thiserror::Error)]
pub enum PreambleReadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Container(#[from] ContainerError),
}

/// The parsed info and header, and where the chunk stream resumes.
#[derive(Debug)]
#[non_exhaustive]
pub struct Preamble {
    pub info: ReplayInfo,
    pub header: ReplayHeader,
    /// Byte offset of the first chunk after the Header chunk.
    pub remaining_offset: usize,
}

/// The entry point for reading a `.vrf` file.
pub fn parse_preamble(data: &[u8]) -> Result<Preamble, ContainerError> {
    let (replay_info, info_end) = info::parse_replay_info(data)?;

    // Header must be first; see `ContainerError::ChunkBeforeHeader`.
    let mut iter = ChunkIterator::new(data, info_end);
    let chunk = iter
        .next_chunk()?
        .ok_or(ContainerError::MissingHeaderChunk)?;

    match chunk.chunk_type {
        ChunkType::Header => {
            let payload =
                &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
            let replay_header = header::parse_replay_header(payload)?;
            Ok(Preamble {
                info: replay_info,
                header: replay_header,
                remaining_offset: iter.position(),
            })
        }
        ChunkType::ReplayData => Err(ContainerError::DataBeforeHeader),
        ChunkType::Checkpoint | ChunkType::Event | ChunkType::Unknown(_) => {
            Err(ContainerError::ChunkBeforeHeader {
                chunk_type: chunk.chunk_type,
            })
        }
    }
}

/// Read a preamble from the beginning of a seekable replay, leaving the cursor
/// immediately after its Header chunk. The existing slice parsers still
/// validate both sections. Only the bounded info prefix and the declared
/// Header payload are buffered; subsequent replay chunks are not retained.
///
/// The source must remain unchanged for the duration of the read. This does
/// not provide a snapshot of a concurrently modified file.
pub fn read_preamble<R: Read + Seek>(reader: &mut R) -> Result<Preamble, PreambleReadError> {
    let file_len = reader.seek(SeekFrom::End(0))?;
    reader.rewind()?;
    // Fixed info fields, every permitted custom version, the largest friendly
    // name and encryption key. Read at most this prefix to reuse the exact
    // parser, including its error precedence, without buffering the replay.
    const MAX_INFO_BYTES: u64 = 12
        + MAX_CUSTOM_VERSION_COUNT as u64 * 20
        + 12
        + 4
        + MAX_FRIENDLY_NAME_BYTES as u64
        + 4
        + 8
        + 4
        + 4
        + 4
        + MAX_ENCRYPTION_KEY_BYTES as u64;
    let mut prefix = Vec::new();
    reader
        .by_ref()
        .take(MAX_INFO_BYTES)
        .read_to_end(&mut prefix)?;
    let (info, info_end) = info::parse_replay_info(&prefix)?;
    drop(prefix);

    reader.seek(SeekFrom::Start(info_end as u64))?;
    let available = file_len.saturating_sub(info_end as u64);
    if available == 0 {
        return Err(ContainerError::MissingHeaderChunk.into());
    }
    if available < 8 {
        return Err(ContainerError::Truncated {
            context: "chunk header",
            needed: 8,
            available: available as usize,
        }
        .into());
    }
    let mut wire = [0; 8];
    reader.read_exact(&mut wire)?;
    let kind = ChunkType::from_raw(u32::from_le_bytes(wire[..4].try_into().unwrap()));
    let size = i32::from_le_bytes(wire[4..].try_into().unwrap());
    if size < 0 {
        return Err(ContainerError::InvalidChunkSize { size }.into());
    }
    let available = available - 8;
    if size as u64 > available {
        return Err(ContainerError::Truncated {
            context: "chunk payload",
            needed: size as usize,
            available: available as usize,
        }
        .into());
    }
    // ChunkIterator verifies the range before rejecting a pre-header kind.
    match kind {
        ChunkType::Header => {}
        ChunkType::ReplayData => return Err(ContainerError::DataBeforeHeader.into()),
        _ => return Err(ContainerError::ChunkBeforeHeader { chunk_type: kind }.into()),
    }
    let mut payload = vec![0; size as usize];
    reader.read_exact(&mut payload)?;
    let header = header::parse_replay_header(&payload)?;
    let remaining_offset = info_end
        .checked_add(8)
        .and_then(|at| at.checked_add(size as usize))
        .ok_or_else(|| std::io::Error::other("preamble offset exceeds the addressable range"))?;
    Ok(Preamble {
        info,
        header,
        remaining_offset,
    })
}
