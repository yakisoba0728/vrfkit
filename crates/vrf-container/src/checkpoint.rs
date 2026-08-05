//! Checkpoint chunk parser.
//!
//! A Checkpoint chunk carries a full-state snapshot of the match at one
//! instant: the server's own NetGUID cache, its net-field export map, and a
//! single DemoFrame that re-opens every actor alive at that moment and re-sends
//! its complete replicated state.
//!
//! # Why this exists
//!
//! It was long assumed these chunks duplicate what the ReplayData stream
//! already carries, and the assumption was measured and found false: 6-11% of a
//! checkpoint's RepLayout field values disagree with what ReplayData carried at
//! the same timestamp, and 0.5-2% are keys ReplayData never sent at all. See
//! PROJECT_STATUS.md 22-I for the measurement and CHECKPOINT_SPEC.md for the
//! byte-level derivation.
//!
//! # Chunk layout
//!
//! The six-field header is byte-identical to an Event chunk's:
//!
//! | Offset | Type | Field |
//! |--------|------|-------|
//! | 0 | FString | Id, e.g. `checkpoint0` |
//! | ... | FString | Group, always `checkpoint` |
//! | ... | FString | Metadata, a 1-based counter |
//! | ... | u32 | Time1 |
//! | ... | u32 | Time2 |
//! | ... | i32 | SizeInBytes |
//! | ... | [u8; SizeInBytes] | Oodle archive |
//!
//! The archive body uses the same framing as a ReplayData chunk's --
//! `[i32 decompressed_size][i32 compressed_size][oodle bytes]` -- but with no
//! `MemorySizeInBytes` ahead of it, so the archive's own `decompressed_size` is
//! the only statement of the output length.
//!
//! Verified over the 215-replay corpus: 4,024 checkpoints, every one consumed
//! exactly by this layout, `compressed_size + 8 == SizeInBytes` in all of them,
//! and `Time1 == Time2` in all of them.
//!
//! # What is inside the archive
//!
//! [`decompress_checkpoint`] returns the plaintext. Its structure -- the guid
//! cache, the export-group map, and where the DemoFrame begins -- is a schema
//! concern, not a container one, and lives in `vrf_schema::checkpoint`.

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::limits::MAX_FSTRING_BYTES;

/// A parsed Checkpoint chunk header plus its still-compressed archive.
#[derive(Debug, Clone)]
pub struct CheckpointChunk<'a> {
    /// Checkpoint id, `checkpoint0`, `checkpoint1`, ... in every corpus file.
    pub id: String,
    /// Chunk group, `checkpoint` in every corpus file.
    pub group: String,
    /// Free-form metadata; a 1-based counter in every corpus file.
    pub metadata: String,
    /// Snapshot time in milliseconds. Equal to the enclosed DemoFrame's own
    /// frame time in all 4,024 corpus checkpoints.
    pub time1: u32,
    /// Second timestamp. Equal to `time1` in every corpus file; both are kept
    /// because the format keeps them separate.
    pub time2: u32,
    /// Declared archive size. Validated non-negative and within the chunk.
    pub size_in_bytes: i32,
    /// The Oodle archive, exactly `size_in_bytes` bytes.
    pub archive: &'a [u8],
    /// Bytes after the archive this layout does not account for. Zero for all
    /// 4,024 corpus checkpoints; reported so a format change is counted rather
    /// than discarded in silence.
    pub trailing_bytes: usize,
}

/// Parse a Checkpoint chunk payload.
///
/// `payload` is the region `data[chunk.data_offset .. + chunk.size_in_bytes]`
/// from a [`RawChunk`](crate::RawChunk) of type
/// [`ChunkType::Checkpoint`](crate::ChunkType::Checkpoint).
///
/// # Errors
///
/// [`ContainerError::Truncated`] if any field runs past the end of the chunk,
/// and [`ContainerError::InvalidCheckpointArchiveSize`] if `SizeInBytes` is
/// negative.
pub fn parse_checkpoint_chunk(payload: &[u8]) -> Result<CheckpointChunk<'_>, ContainerError> {
    let mut reader = BitReader::new(payload);

    let id = read_fstring(&mut reader, "checkpoint id")?;
    let group = read_fstring(&mut reader, "checkpoint group")?;
    let metadata = read_fstring(&mut reader, "checkpoint metadata")?;
    let time1 = read_u32(&mut reader, "checkpoint time1")?;
    let time2 = read_u32(&mut reader, "checkpoint time2")?;
    let size_in_bytes = read_i32(&mut reader, "checkpoint archive size")?;

    if size_in_bytes < 0 {
        return Err(ContainerError::InvalidCheckpointArchiveSize {
            size: size_in_bytes,
        });
    }
    let size = size_in_bytes as usize;

    // Every read above is byte-granular, so the reader sits on a byte boundary.
    let header_end = (reader.position() / 8) as usize;
    if header_end > payload.len() || payload.len() - header_end < size {
        return Err(ContainerError::Truncated {
            context: "checkpoint archive",
            needed: size,
            available: payload.len().saturating_sub(header_end),
        });
    }

    Ok(CheckpointChunk {
        id,
        group,
        metadata,
        time1,
        time2,
        size_in_bytes,
        archive: &payload[header_end..header_end + size],
        trailing_bytes: payload.len() - header_end - size,
    })
}

/// Decompress a checkpoint's Oodle archive into its plaintext snapshot.
///
/// `archive` is [`CheckpointChunk::archive`]. Unlike a ReplayData chunk there
/// is no `MemorySizeInBytes` to check the output length against, so the
/// archive's own `decompressed_size` is authoritative and is the only bound on
/// the allocation -- it is range-checked before use.
///
/// # Errors
///
/// [`ContainerError::EncryptedNotSupported`] when `encrypted`, and the same
/// Oodle error variants a ReplayData chunk produces. An uncompressed replay
/// returns the archive bytes unchanged.
pub fn decompress_checkpoint(
    archive: &[u8],
    compressed: bool,
    encrypted: bool,
) -> Result<Vec<u8>, ContainerError> {
    if encrypted {
        return Err(ContainerError::EncryptedNotSupported);
    }
    if !compressed {
        // No corpus file takes this path -- every observed replay is
        // compressed -- so it is deliberately the trivial one rather than a
        // guess at a framing nothing can be checked against.
        return Ok(archive.to_vec());
    }
    // `decompress_oodle_archive` takes the declared size as an i32. Real
    // callers pass a checkpoint archive whose size is already validated as an
    // i32, so this never fires on supported input; the checked conversion is
    // here so a >2 GiB slice is rejected loudly rather than silently truncated
    // by `as i32`. `Truncated` is the one variant that takes `usize` fields,
    // so it can carry the real length without itself losing precision.
    let declared_size = i32::try_from(archive.len()).map_err(|_| ContainerError::Truncated {
        context: "checkpoint archive length",
        needed: archive.len(),
        available: i32::MAX as usize,
    })?;
    crate::oodle::decompress_oodle_archive(archive, declared_size, None, "checkpoint archive")
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
        .map_err(|_| ContainerError::Truncated {
            context,
            needed: 4,
            available: (reader.bits_remaining() / 8) as usize,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTF-16LE with a negative length, which is how every corpus checkpoint
    /// string arrives.
    fn push_fstring(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().chain(core::iter::once(0)).collect();
        out.extend_from_slice(&(-(units.len() as i32)).to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }

    fn build(archive: &[u8], trailing: usize) -> Vec<u8> {
        let mut out = Vec::new();
        push_fstring(&mut out, "checkpoint0");
        push_fstring(&mut out, "checkpoint");
        push_fstring(&mut out, "1");
        out.extend_from_slice(&47u32.to_le_bytes());
        out.extend_from_slice(&47u32.to_le_bytes());
        out.extend_from_slice(&(archive.len() as i32).to_le_bytes());
        out.extend_from_slice(archive);
        out.extend(core::iter::repeat_n(0u8, trailing));
        out
    }

    #[test]
    fn parses_the_six_header_fields_and_hands_back_the_archive() {
        let chunk = build(&[1, 2, 3, 4, 5], 0);
        let cp = parse_checkpoint_chunk(&chunk).unwrap();
        assert_eq!(cp.id, "checkpoint0");
        assert_eq!(cp.group, "checkpoint");
        assert_eq!(cp.metadata, "1");
        assert_eq!(cp.time1, 47);
        assert_eq!(cp.time2, 47);
        assert_eq!(cp.size_in_bytes, 5);
        assert_eq!(cp.archive, &[1, 2, 3, 4, 5]);
        assert_eq!(cp.trailing_bytes, 0);
    }

    /// Bytes the layout does not reach are counted, not dropped. Zero across
    /// the corpus, so this is the only place the branch is exercised.
    #[test]
    fn trailing_bytes_are_reported_not_discarded() {
        let chunk = build(&[9; 3], 4);
        let cp = parse_checkpoint_chunk(&chunk).unwrap();
        assert_eq!(cp.archive, &[9, 9, 9]);
        assert_eq!(cp.trailing_bytes, 4);
    }

    #[test]
    fn a_negative_archive_size_is_an_error() {
        let mut chunk = build(&[1, 2], 0);
        let size_at = chunk.len() - 2 - 4;
        chunk[size_at..size_at + 4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(matches!(
            parse_checkpoint_chunk(&chunk),
            Err(ContainerError::InvalidCheckpointArchiveSize { size: -1 })
        ));
    }

    #[test]
    fn an_archive_running_past_the_chunk_is_truncated_not_panicking() {
        let mut chunk = build(&[1, 2], 0);
        let size_at = chunk.len() - 2 - 4;
        chunk[size_at..size_at + 4].copy_from_slice(&4096i32.to_le_bytes());
        assert!(matches!(
            parse_checkpoint_chunk(&chunk),
            Err(ContainerError::Truncated { .. })
        ));
    }

    /// A checkpoint archive states its own output length and nothing outside
    /// bounds it, so a corrupt header must not become a huge allocation.
    #[test]
    fn an_out_of_range_decompressed_size_is_rejected() {
        let mut archive = Vec::new();
        archive.extend_from_slice(&i32::MAX.to_le_bytes());
        archive.extend_from_slice(&0i32.to_le_bytes());
        assert!(matches!(
            decompress_checkpoint(&archive, true, false),
            Err(ContainerError::InvalidMemorySize { size }) if size == i32::MAX
        ));
    }

    #[test]
    fn encryption_is_refused_rather_than_guessed() {
        assert!(matches!(
            decompress_checkpoint(&[0; 8], true, true),
            Err(ContainerError::EncryptedNotSupported)
        ));
    }
}
