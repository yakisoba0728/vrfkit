//! ReplayData chunk framing and Oodle archive decompression.
//!
//! A ReplayData chunk states its output length twice, as `MemorySizeInBytes` in
//! its 16-byte prologue and in the archive header, and the two must agree. A
//! Checkpoint archive has only the header's, which is why
//! `decompress_oodle_archive` takes the expected length as an `Option`.

use crate::error::ContainerError;
use crate::io::le_u32;
use crate::limits::MAX_CHUNK_SIZE;

/// Parsed metadata from a ReplayData chunk's inner framing.
#[derive(Debug, Clone)]
pub struct ReplayDataMeta {
    /// Chunk start time in ms; chunks are contiguous and the last `time2` is the
    /// info's `length_in_ms`.
    pub time1: u32,
    pub time2: u32,
    /// On-disk (compressed) size.
    pub size_in_bytes: i32,
    /// Decompressed size.
    pub memory_size_in_bytes: i32,
    /// Payload bytes past the declared archive (`payload.len() - 16 - SizeInBytes`,
    /// floored at 0; a larger `SizeInBytes` is truncation). `compressed_size` is
    /// pinned to `SizeInBytes - 8`, so this one count covers the archive too.
    pub trailing_bytes: usize,
}

/// Bytes of ReplayData prologue ahead of the archive: two u32 then two i32.
const REPLAY_DATA_PROLOGUE_BYTES: usize = 16;

/// Bytes of Oodle archive header: decompressed size then compressed size.
const OODLE_HEADER_BYTES: usize = 8;

/// Parse the inner framing of a ReplayData chunk payload: the region
/// `data[chunk.data_offset .. + chunk.size_in_bytes]` of a [`RawChunk`](crate::RawChunk)
/// of type [`ChunkType::ReplayData`](crate::ChunkType::ReplayData).
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
    let time1 = le_u32(payload, 0);
    let time2 = le_u32(payload, 4);
    let size_in_bytes = le_u32(payload, 8) as i32;
    let memory_size_in_bytes = le_u32(payload, 12) as i32;

    if !(0..=MAX_CHUNK_SIZE).contains(&memory_size_in_bytes) {
        return Err(ContainerError::InvalidMemorySize {
            size: memory_size_in_bytes,
        });
    }

    // A negative `size_in_bytes` yields zero: the decompressor rejects it, and
    // calling the whole payload "trailing" would be a second wrong answer.
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

/// Decompress a ReplayData chunk payload (from Time1 on). `compressed` is the
/// preamble's `ReplayInfo::compressed`: without it the data is returned as-is,
/// after checking `SizeInBytes == MemorySizeInBytes`; with it the data is an
/// Oodle archive:
///
/// | Offset (relative to data start) | Type | Field |
/// |---|---|---|
/// | 0 | i32 | decompressed_size (must == MemorySizeInBytes) |
/// | 4 | i32 | compressed_size (must == SizeInBytes - 8) |
/// | 8 | [u8] | Oodle-compressed bytes |
///
/// This form **drops** the count [`decompress_replay_data_with_trailing`]
/// returns, the payload bytes no reader consumed; expected 0, but a caller
/// that ignores it cannot notice a framing change.
pub fn decompress_replay_data(
    payload: &[u8],
    compressed: bool,
    encrypted: bool,
) -> Result<Vec<u8>, ContainerError> {
    decompress_replay_data_with_trailing(payload, compressed, encrypted).map(|(plain, _)| plain)
}

/// As [`decompress_replay_data`], also returning the payload bytes no reader
/// consumed: [`ReplayDataMeta::trailing_bytes`] plus the archive bytes the
/// codec never read. Both 0 in all 20,180 ReplayData archives of 1,014
/// replays, 11.06-13.06.
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

    let (plain, unread) = decompress_oodle_archive(
        data_bytes,
        meta.size_in_bytes,
        Some(meta.memory_size_in_bytes),
        "oodle compressed data",
    )?;
    Ok((plain, meta.trailing_bytes + unread))
}

/// Decompress one Oodle archive (layout on [`decompress_replay_data`]) into the
/// plaintext and the count of archive bytes the codec never read.
/// `declared_size` includes the 8-byte header; `expected_decompressed` is
/// `MemorySizeInBytes` for ReplayData and `None` for a checkpoint.
pub(crate) fn decompress_oodle_archive(
    archive: &[u8],
    declared_size: i32,
    expected_decompressed: Option<i32>,
    context: &'static str,
) -> Result<(Vec<u8>, usize), ContainerError> {
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

    let decompressed_size = le_u32(archive, 0) as i32;
    let compressed_size = le_u32(archive, 4) as i32;

    if let Some(expected) = expected_decompressed {
        if decompressed_size != expected {
            return Err(ContainerError::OodleDecompressedSizeMismatch {
                archive_size: decompressed_size,
                memory_size: expected,
            });
        }
    } else if !(0..=MAX_CHUNK_SIZE).contains(&decompressed_size) {
        // Nothing outside the archive bounds this allocation.
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

    // Bytes past `compressed_size` are `ReplayDataMeta::trailing_bytes`, which
    // the caller counts; a checkpoint passes `declared_size = archive.len()`.
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

/// Run the Oodle codec over `input`, producing exactly `decompressed_size`
/// bytes, and count the input bytes it never read.
///
/// The codec panics on some malformed streams (asserts, `unwrap`, slice
/// indexing; a single flipped byte in a real archive is enough), so the call
/// runs inside `catch_unwind` and a panic becomes
/// [`ContainerError::OodleDecompression`]. The panic hook is left alone: the
/// process-global hook belongs to the application, so the default one still
/// prints the codec's panic message to stderr before the error is returned.
/// Under `panic = "abort"` there is nothing to catch and the process exits.
#[cfg(feature = "oodle")]
fn inflate(input: &[u8], decompressed_size: usize) -> Result<(Vec<u8>, usize), ContainerError> {
    // No `AssertUnwindSafe`: the closure captures only `input` (a shared
    // `&[u8]`) and a `usize`, which the compiler checks are unwind-safe. The
    // output buffer and the extractor are created inside and dropped with the
    // unwind, so a half-written buffer can never be returned.
    match std::panic::catch_unwind(|| inflate_unguarded(input, decompressed_size)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            Err(ContainerError::OodleDecompression(format!(
                "decoder panicked: {message}"
            )))
        }
    }
}

#[cfg(feature = "oodle")]
fn inflate_unguarded(
    input: &[u8],
    decompressed_size: usize,
) -> Result<(Vec<u8>, usize), ContainerError> {
    let mut output = vec![0u8; decompressed_size];

    // A fresh Extractor per archive: it keeps bitknit/lzna state across calls
    // unless a block sets `restart_decoder`, so a reused one would turn a loud
    // "Bitknit uninitialized" into a silent decode against the previous archive.
    let mut extractor = oozextract::Extractor::new();
    // `read` over a slice, not `read_from_slice`: the codec stops once `output`
    // is full, and only this form leaves the slice at what it did not read
    // (meaningful on success only).
    let mut unread = input;
    let n = extractor
        .read(&mut unread, &mut output)
        .map_err(|e| ContainerError::OodleDecompression(format!("{e:?}")))?;

    if n != decompressed_size {
        return Err(ContainerError::OodleOutputSizeMismatch {
            expected: decompressed_size,
            actual: n,
        });
    }

    Ok((output, unread.len()))
}

/// Stand-in when the `oodle` feature is off; see [`ContainerError::OodleUnsupported`].
#[cfg(not(feature = "oodle"))]
fn inflate(input: &[u8], decompressed_size: usize) -> Result<(Vec<u8>, usize), ContainerError> {
    let _ = input;
    Err(ContainerError::OodleUnsupported {
        needed: decompressed_size,
    })
}
