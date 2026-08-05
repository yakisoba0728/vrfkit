//! The four fixed sections that precede a DemoFrame's packet loop.
//!
//! Each is a distinct sub-grammar with its own source in the C# reference, and
//! three of the four exist only to be *skipped* correctly -- getting a byte
//! count wrong here desynchronises the whole frame rather than failing, so each
//! reader is kept separate and named after the reference type it mirrors.
//!
//! # Measured shape on real replays
//!
//! Over the reference replay's 226,190 frames the streaming-level-fixes section
//! carries **29** level names in total and the external-data loop terminates
//! immediately **every** time (zero blobs). Both loops are therefore
//! effectively frame overhead, not throughput: optimising the level-name read
//! to skip its `String` allocation would remove 29 allocations from a run that
//! makes millions, so it is deliberately left reading (and validating) the
//! string rather than blind-skipping the bytes.

use vrf_bitio::BitReader;
use vrf_schema::NetGuidCache;

use crate::error::FrameError;

/// Maximum sane FString bytes when skipping level names.
const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

/// ExportData: read net field exports + export GUIDs into the cache.
///
/// Source: `ExportDataReader.Read()` -- calls `ReadNetFieldExports()` then
/// `ReadExportGuids()`. Both are byte-aligned (FBinaryArchive) reads.
///
/// This is the only section that mutates state: the schema a replay declares
/// arrives here, incrementally, frame by frame.
pub(crate) fn read_export_data(
    reader: &mut BitReader<'_>,
    cache: &mut NetGuidCache,
) -> Result<(), FrameError> {
    vrf_schema::read_net_field_exports(reader, cache).map_err(FrameError::schema)?;
    vrf_schema::read_export_guids(reader, cache).map_err(FrameError::schema)?;
    Ok(())
}

/// StreamingLevelFixes: skip level names (either compact or verbose form).
///
/// Source: `StreamingLevelFixesReader.cs`
pub(crate) fn read_streaming_level_fixes(
    reader: &mut BitReader<'_>,
    has_streaming_fixes: bool,
) -> Result<(), FrameError> {
    let num_levels = reader.read_int_packed().map_err(FrameError::bit)?;

    if has_streaming_fixes {
        // Compact form: just FString names + a u64 externalOffset.
        for _ in 0..num_levels {
            let _ = reader
                .read_fstring(MAX_FSTRING_BYTES)
                .map_err(FrameError::bit)?;
        }
        let _ = reader.read_u64().map_err(FrameError::bit)?;
    } else {
        // Verbose form: packageName + packageNameToLoad + FTransform per entry.
        // The C# code calls `_archive.ReadFTransform()`, which in their
        // implementation reads rotation(4 x f32) + translation(3 x f32) +
        // scale(3 x f32) = 10 x f32 = 40 bytes.
        //
        // No corpus replay takes this branch: all 215 set HasStreamingFixes.
        for _ in 0..num_levels {
            let _ = reader
                .read_fstring(MAX_FSTRING_BYTES)
                .map_err(FrameError::bit)?;
            let _ = reader
                .read_fstring(MAX_FSTRING_BYTES)
                .map_err(FrameError::bit)?;
            reader.skip_bits(40 * 8).map_err(FrameError::bit)?;
        }
    }

    Ok(())
}

/// ExternalData: loop reading numBits + netGuid + skip, until numBits == 0.
///
/// Source: `PlaybackPacketReader.ReadExternalData()`
pub(crate) fn read_external_data(reader: &mut BitReader<'_>) -> Result<(), FrameError> {
    loop {
        let num_bits = reader.read_int_packed().map_err(FrameError::bit)?;
        if num_bits == 0 {
            return Ok(());
        }
        let _net_guid = reader.read_int_packed().map_err(FrameError::bit)?;
        let byte_count = u64::from(num_bits.div_ceil(8));
        reader.skip_bits(byte_count * 8).map_err(FrameError::bit)?;
    }
}

/// GameSpecificFrameData: optionally read a u64 skip-offset and skip that many bytes.
///
/// Source: `GameSpecificFrameDataReader.Read()`
///
/// The reference replay does NOT set this flag (its header flags are `0x0002`),
/// so this returns on the first branch for every frame in the corpus.
pub(crate) fn read_game_specific_frame_data(
    reader: &mut BitReader<'_>,
    has_game_specific: bool,
) -> Result<(), FrameError> {
    if !has_game_specific {
        return Ok(());
    }
    let skip_offset = reader.read_u64().map_err(FrameError::bit)?;
    if skip_offset == 0 {
        return Ok(());
    }
    // `skip_offset` is a raw u64 from the wire; `* 8` is plain wrapping
    // multiplication, so a large value silently wraps to a small skip and
    // desynchronises the frame. The flag is unset on every known replay, but a
    // malformed/large offset must fail loudly rather than wrap.
    let skip_bits = skip_offset.checked_mul(8).ok_or_else(|| {
        FrameError::Bit(format!(
            "game-specific skip offset overflows: {skip_offset}"
        ))
    })?;
    reader.skip_bits(skip_bits).map_err(FrameError::bit)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::read_game_specific_frame_data;
    use vrf_bitio::BitReader;

    #[test]
    fn a_huge_game_specific_skip_offset_errors_instead_of_wrapping() {
        // u64 (1 << 61) + 1: `* 8` overflows u64 and wraps to 8. A trailing
        // byte leaves 8 bits after the u64 read, so the wrapped `skip_bits(8)`
        // would SUCCEED and the bug is a silent Ok. checked_mul must reject it.
        let bytes: [u8; 9] = [0x01, 0, 0, 0, 0, 0, 0, 0x20, 0xFF];
        let mut reader = BitReader::new(&bytes);
        let result = read_game_specific_frame_data(&mut reader, true);
        assert!(
            result.is_err(),
            "an overflowing skip offset must error, not wrap to 8 and succeed"
        );
    }
}
