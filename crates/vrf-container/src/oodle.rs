//! ReplayData chunk framing and Oodle archive decompression.
//!
//! Two framings meet here. A ReplayData chunk states its decompressed length
//! twice -- once in its own 16-byte prologue as `MemorySizeInBytes`, once
//! inside the Oodle archive header -- and the parser requires them to agree. A
//! Checkpoint chunk (see [`crate::checkpoint`]) reuses the archive half only,
//! and has no outer statement to check against, which is why
//! `decompress_oodle_archive` takes the expected length as an `Option`.
//!
//! # Compression is optional at the crate level
//!
//! Every replay observed to date is Oodle-compressed, but the format allows
//! plaintext chunks, and a consumer that only ever sees those does not need the
//! decoder linked in. The `oodle` feature (on by default) gates the
//! `oozextract` dependency; with it off the plaintext paths still work and a
//! compressed archive reports [`ContainerError::OodleUnsupported`] rather than
//! silently returning nothing.

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::limits::MAX_CHUNK_SIZE;

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
    /// Bytes of the chunk payload past the declared archive that this framing
    /// does not account for: `payload.len() - 16 - SizeInBytes`, floored at
    /// zero (a `SizeInBytes` larger than the payload is truncation, reported by
    /// the decompressor, not a negative residual).
    ///
    /// Reported for the same reason [`CheckpointChunk::trailing_bytes`] is: the
    /// data-bearing slices are cut to the declared length
    /// (`data_bytes[..size]`, `compressed_data[..compressed_size]`), so anything
    /// past it is replay data that would otherwise be discarded with no error
    /// and no tally. Expected to be zero; a non-zero value means the chunk
    /// framing has changed.
    ///
    /// [`CheckpointChunk::trailing_bytes`]: crate::CheckpointChunk::trailing_bytes
    pub trailing_bytes: usize,
}

/// Bytes of ReplayData prologue ahead of the archive: two u32 then two i32.
const REPLAY_DATA_PROLOGUE_BYTES: usize = 16;

/// Bytes of Oodle archive header: decompressed size then compressed size.
const OODLE_HEADER_BYTES: usize = 8;

/// Parse the inner framing of a ReplayData chunk payload and return metadata.
///
/// The payload bytes are the region `data[chunk.data_offset .. + chunk.size_in_bytes]`
/// from a [`RawChunk`](crate::RawChunk) of type [`ChunkType::ReplayData`](crate::ChunkType::ReplayData).
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
    if payload.len() < REPLAY_DATA_PROLOGUE_BYTES {
        return Err(ContainerError::Truncated {
            context: "replay data meta",
            needed: REPLAY_DATA_PROLOGUE_BYTES,
            available: payload.len(),
        });
    }
    let mut reader = BitReader::new(payload);
    let time1 = read_u32(&mut reader)?;
    let time2 = read_u32(&mut reader)?;
    let size_in_bytes = read_i32(&mut reader)?;
    let memory_size_in_bytes = read_i32(&mut reader)?;

    if !(0..=MAX_CHUNK_SIZE).contains(&memory_size_in_bytes) {
        return Err(ContainerError::InvalidMemorySize {
            size: memory_size_in_bytes,
        });
    }

    // What the chunk carries past the archive the prologue declares. A negative
    // `size_in_bytes` yields zero here; it is rejected by the decompressor,
    // and calling the whole payload "trailing" would be a second wrong answer.
    let available = payload.len() - REPLAY_DATA_PROLOGUE_BYTES;
    let trailing_bytes =
        usize::try_from(size_in_bytes).map_or(0, |declared| available.saturating_sub(declared));

    Ok(ReplayDataMeta {
        time1,
        time2,
        size_in_bytes,
        memory_size_in_bytes,
        trailing_bytes,
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
///
/// # Trailing bytes
///
/// This form **drops** [`ReplayDataMeta::trailing_bytes`]. Both data paths cut
/// the payload to the declared length, so a chunk carrying more than its
/// `SizeInBytes` accounts for loses the excess here with no error and no tally.
/// Use [`decompress_replay_data_with_trailing`] to see it; the count is
/// expected to be zero, and a caller that ignores it is choosing not to notice
/// a framing change rather than being unable to.
pub fn decompress_replay_data(
    payload: &[u8],
    compressed: bool,
    encrypted: bool,
) -> Result<Vec<u8>, ContainerError> {
    decompress_replay_data_with_trailing(payload, compressed, encrypted).map(|(plain, _)| plain)
}

/// As [`decompress_replay_data`], also reporting the bytes past the declared
/// archive that the framing does not account for.
///
/// The count is [`ReplayDataMeta::trailing_bytes`], computed once from the
/// prologue. The inner `compressed_data[..compressed_size]` slice in
/// `decompress_oodle_archive` discards exactly the same bytes -- for a
/// ReplayData chunk `compressed_size` is pinned to `SizeInBytes - 8` and the
/// archive slice runs to the end of the payload, so the two residuals are one
/// residual -- and a checkpoint archive has none at all, because its declared
/// size *is* its slice length. One count covers both.
pub fn decompress_replay_data_with_trailing(
    payload: &[u8],
    compressed: bool,
    encrypted: bool,
) -> Result<(Vec<u8>, usize), ContainerError> {
    if encrypted {
        return Err(ContainerError::EncryptedNotSupported);
    }

    let meta = parse_replay_data_meta(payload)?;
    let data_bytes = &payload[REPLAY_DATA_PROLOGUE_BYTES..];

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
        return Ok((data_bytes[..size].to_vec(), meta.trailing_bytes));
    }

    let plain = decompress_oodle_archive(
        data_bytes,
        meta.size_in_bytes,
        Some(meta.memory_size_in_bytes),
        "oodle compressed data",
    )?;
    Ok((plain, meta.trailing_bytes))
}

/// Decompress one Oodle archive: an 8-byte header followed by the codec stream.
///
/// ```text
/// | i32 decompressed_size |
/// | i32 compressed_size   |
/// | [compressed_size] bytes |
/// ```
///
/// Shared by ReplayData and Checkpoint chunks, which frame their archives
/// identically and differ only in what states the expected output length.
/// `declared_size` is the archive's declared byte count including the 8-byte
/// header. `expected_decompressed` is the length an outer field claims -- a
/// ReplayData chunk has `MemorySizeInBytes` and passes it, a checkpoint has no
/// such field and passes `None`, which makes the header's own
/// `decompressed_size` the sole authority.
///
/// Every check a ReplayData chunk performed before this was factored out is
/// still performed here, in the same order, so its error behaviour is
/// unchanged; the corpus run over 215 files is what pins that.
pub(crate) fn decompress_oodle_archive(
    archive: &[u8],
    declared_size: i32,
    expected_decompressed: Option<i32>,
    context: &'static str,
) -> Result<Vec<u8>, ContainerError> {
    if declared_size < OODLE_HEADER_BYTES as i32 {
        return Err(ContainerError::OodleHeaderTooSmall {
            size: declared_size,
        });
    }
    if archive.len() < OODLE_HEADER_BYTES {
        return Err(ContainerError::Truncated {
            context: "oodle archive header",
            needed: OODLE_HEADER_BYTES,
            available: archive.len(),
        });
    }

    let mut hdr_reader = BitReader::new(&archive[..OODLE_HEADER_BYTES]);
    let decompressed_size = read_i32(&mut hdr_reader)?;
    let compressed_size = read_i32(&mut hdr_reader)?;

    if let Some(expected) = expected_decompressed {
        if decompressed_size != expected {
            return Err(ContainerError::OodleDecompressedSizeMismatch {
                archive_size: decompressed_size,
                memory_size: expected,
            });
        }
    } else if !(0..=MAX_CHUNK_SIZE).contains(&decompressed_size) {
        // Nothing outside the archive bounds this allocation, so the header
        // has to be range-checked before it is trusted with a `vec![0; n]`.
        return Err(ContainerError::InvalidMemorySize {
            size: decompressed_size,
        });
    }

    let expected_compressed_size = declared_size - OODLE_HEADER_BYTES as i32;
    if compressed_size != expected_compressed_size {
        return Err(ContainerError::OodleCompressedSizeMismatch {
            archive_size: compressed_size,
            expected: expected_compressed_size,
        });
    }

    // Anything past `compressed_size` in this slice is not counted here. It is
    // the same excess `ReplayDataMeta::trailing_bytes` already measured -- the
    // caller hands the whole post-prologue region as `archive`, and
    // `compressed_size` is pinned to `declared_size - 8` just above -- and a
    // checkpoint passes `declared_size = archive.len()`, so there is nothing
    // left over on that path at all. Counting it again here would double it.
    let compressed_data = &archive[OODLE_HEADER_BYTES..];
    if compressed_data.len() < compressed_size as usize {
        return Err(ContainerError::Truncated {
            context,
            needed: compressed_size as usize,
            available: compressed_data.len(),
        });
    }

    let input = &compressed_data[..compressed_size as usize];
    inflate(input, decompressed_size as usize)
}

/// Run the Oodle codec over `input`, producing exactly `decompressed_size` bytes.
#[cfg(feature = "oodle")]
fn inflate(input: &[u8], decompressed_size: usize) -> Result<Vec<u8>, ContainerError> {
    let mut output = vec![0u8; decompressed_size];

    // A fresh extractor per archive, deliberately.
    //
    // `Extractor::new` allocates and zeroes ~768 KiB (two 256 KiB scratch
    // arrays plus a 256 KiB BytesMut), and the reference replay decompresses 19
    // ReplayData archives plus 18 checkpoints, so hoisting it into a
    // thread-local was measured: it is worth 2.3% on `export` and 1.8% on
    // `validate`. It is not taken, because `Extractor` carries `bitknit_state`
    // and `lzna_state` across calls and only clears them when a block header
    // sets `restart_decoder`. A reused extractor would therefore decode a
    // stream whose first quantum does NOT request a restart against the
    // *previous* archive's decoder state, where a fresh one fails with
    // "Bitknit uninitialized". No archive that decodes correctly today would
    // change -- the divergence is confined to inputs that currently error --
    // but turning a loud failure into a silent decode is exactly what this
    // crate does not do. Reinstate this if `oozextract` grows a `reset()`.
    let mut extractor = oozextract::Extractor::new();
    let n = extractor
        .read_from_slice(input, &mut output)
        .map_err(|e| ContainerError::OodleDecompression(format!("{e:?}")))?;

    if n != decompressed_size {
        return Err(ContainerError::OodleOutputSizeMismatch {
            expected: decompressed_size,
            actual: n,
        });
    }

    Ok(output)
}

/// Stand-in used when the `oodle` feature is off.
///
/// Reports rather than returning an empty buffer: a caller that asked for a
/// compressed archive from a build without a decoder has a configuration
/// problem, and a zero-length "success" would be indistinguishable from an
/// empty chunk downstream.
#[cfg(not(feature = "oodle"))]
fn inflate(input: &[u8], decompressed_size: usize) -> Result<Vec<u8>, ContainerError> {
    let _ = input;
    Err(ContainerError::OodleUnsupported {
        needed: decompressed_size,
    })
}

fn read_u32(reader: &mut BitReader<'_>) -> Result<u32, ContainerError> {
    reader
        .read_u32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))
}

fn read_i32(reader: &mut BitReader<'_>) -> Result<i32, ContainerError> {
    reader
        .read_i32()
        .map_err(|e| ContainerError::BitIo(e.to_string()))
}
