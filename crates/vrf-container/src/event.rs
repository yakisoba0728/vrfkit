//! Event chunk parser.
//!
//! An Event chunk carries one entry from the server's own labelled game
//! timeline: a round start, a character death, a spike plant. It is the only
//! place in the file where the server names what happened -- everything else
//! has to be reconstructed from replicated properties and RPCs.
//!
//! # Wire layout
//!
//! | Offset | Type | Field |
//! |--------|------|-------|
//! | 0 | FString | Id |
//! | ... | FString | Group |
//! | ... | FString | Metadata |
//! | ... | u32 | Time1 |
//! | ... | u32 | Time2 |
//! | ... | i32 | SizeInBytes |
//! | ... | [u8; SizeInBytes] | payload |
//!
//! Measured over the 215-replay corpus (43397 Event chunks): every chunk is
//! consumed exactly by this layout with no bytes left over, and Time1 always
//! equals Time2. Checkpoint chunks open with the same six fields, so the
//! framing is not Event-specific; this module parses Event chunks only.
//!
//! # Why the payload stays raw
//!
//! The `SizeInBytes` payload has an observable shape:
//!
//! ```text
//! [u32 group tag][N x u32 words][FString "EReplayEventGroup::<Name>"][f32 seconds]
//! ```
//!
//! but it is not self-describing. `N` varies by group (0 for SpikePlanted, 1
//! for RoundStart, 2 for CharacterDeath) and no count precedes the words, so a
//! forward read cannot tell where they end. `N` can be *solved* for from the
//! total size -- the solution is unique for all 43397 chunks in the corpus --
//! but every one of those 215 files is the same build
//! (`++Ares-Core+release-13.01`), so uniqueness is a property of this sample,
//! not of the format. The driver emits the first two words as `word0`/`word1`
//! for groups whose `N` is structurally fixed (see `vrf_export::EventRecord`);
//! the payload is
//! therefore handed to the caller byte for byte, and `trailing_bytes` reports
//! anything this layout does not account for rather than dropping it silently.

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::limits::MAX_FSTRING_BYTES;

/// A parsed Event chunk: the six header fields plus its raw payload.
///
/// `payload` borrows the chunk bytes; nothing is copied.
#[derive(Debug, Clone)]
pub struct EventChunk<'a> {
    /// Server-assigned event id, `<replay-guid>_<32 hex digits>` in every
    /// corpus file. Emitted as the wire gives it; no structure is assumed.
    pub id: String,
    /// Event group, e.g. `characterDeath`, `roundStarted`, `spikePlanted`.
    pub group: String,
    /// Free-form metadata string. Frequently empty; empty is what the wire
    /// says, not a missing value.
    pub metadata: String,
    /// First timestamp in milliseconds.
    pub time1: u32,
    /// Second timestamp in milliseconds. Equal to `time1` in every corpus file,
    /// but both are reported because the format keeps them separate.
    pub time2: u32,
    /// Declared payload size. Validated non-negative and within the chunk.
    pub size_in_bytes: i32,
    /// The payload bytes, exactly `size_in_bytes` of them.
    pub payload: &'a [u8],
    /// Bytes after the payload that this layout does not account for. Zero for
    /// all 43397 corpus chunks; reported so a format change is counted rather
    /// than discarded in silence.
    pub trailing_bytes: usize,
}

/// Parse an Event chunk payload.
///
/// `payload` is the region `data[chunk.data_offset .. + chunk.size_in_bytes]`
/// from a [`RawChunk`](crate::RawChunk) of type [`ChunkType::Event`](crate::ChunkType::Event).
///
/// # Errors
///
/// Returns [`ContainerError::Truncated`] if any field runs past the end of the
/// chunk, and [`ContainerError::InvalidEventPayloadSize`] if `SizeInBytes` is
/// negative.
pub fn parse_event_chunk(payload: &[u8]) -> Result<EventChunk<'_>, ContainerError> {
    let mut reader = BitReader::new(payload);

    let id = read_fstring(&mut reader, "event id")?;
    let group = read_fstring(&mut reader, "event group")?;
    let metadata = read_fstring(&mut reader, "event metadata")?;
    let time1 = read_u32(&mut reader, "event time1")?;
    let time2 = read_u32(&mut reader, "event time2")?;
    let size_in_bytes = read_i32(&mut reader, "event payload size")?;

    if size_in_bytes < 0 {
        return Err(ContainerError::InvalidEventPayloadSize {
            size: size_in_bytes,
        });
    }
    let size = size_in_bytes as usize;

    // Every read above is byte-granular, so the reader sits on a byte boundary.
    let header_end = (reader.position() / 8) as usize;
    if header_end > payload.len() || payload.len() - header_end < size {
        return Err(ContainerError::Truncated {
            context: "event payload",
            needed: size,
            available: payload.len().saturating_sub(header_end),
        });
    }

    Ok(EventChunk {
        id,
        group,
        metadata,
        time1,
        time2,
        size_in_bytes,
        payload: &payload[header_end..header_end + size],
        trailing_bytes: payload.len() - header_end - size,
    })
}

// --- Helpers ------------------------------------------------------------------

fn read_u32(reader: &mut BitReader<'_>, context: &'static str) -> Result<u32, ContainerError> {
    reader.read_u32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_i32(reader: &mut BitReader<'_>, context: &'static str) -> Result<i32, ContainerError> {
    reader.read_i32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_fstring(
    reader: &mut BitReader<'_>,
    context: &'static str,
) -> Result<String, ContainerError> {
    reader
        .read_fstring(MAX_FSTRING_BYTES)
        .map_err(|source| ContainerError::FString { context, source })
}
