//! DemoFrame iteration: decompressed replay-data chunk → `(time_ms, packet)` sequence.
//!
//! # Why a separate crate
//!
//! The DemoFrame layer sits between the **container** (which hands us Oodle-
//! decompressed byte slices) and the **replication reader** (which processes
//! individual network packets). It is a distinct parsing stage with its own
//! error modes: a valid container can produce an invalid frame stream, and
//! framing bugs must not crash the container parser. Isolating it keeps both
//! sides testable independently.
//!
//! # DemoFrame wire layout
//!
//! Each decompressed ReplayData chunk is a *sequence* of DemoFrames.
//! A single DemoFrame has this structure (all reads are **byte-aligned**, using
//! Unreal's `FBinaryArchive` — i.e. `IntPacked` is still the 7-bit-per-byte
//! encoding, `FString` is i32 length + bytes + null, etc.):
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │ currentLevelIndex   : i32                                  (ignored)     │
//! │ timeSeconds         : f32                                  (frame time)  │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │ ── ExportData ──                                                        │
//! │   numLayoutCmdExports: IntPacked → ReadNetFieldExports                  │
//! │   numExportGuids:      IntPacked → ReadExportGuids                      │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │ ── StreamingLevelFixes ──                                               │
//! │   [if HasStreamingFixes flag]:                                           │
//! │     numLevels : IntPacked                                               │
//! │     for each: FString (level name)                                      │
//! │     externalOffset : u64                                                │
//! │   [else]:                                                               │
//! │     numLevels : IntPacked                                               │
//! │     for each: FString + FString + FTransform (10 floats = 40 bytes)     │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │ ── ExternalData ──                                                      │
//! │   loop:                                                                 │
//! │     numBits : IntPacked   (0 → break)                                   │
//! │     netGuid : IntPacked   (ignored)                                     │
//! │     skip ceil(numBits/8) bytes                                          │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │ ── GameSpecificFrameData ──                                             │
//! │   [if GameSpecificFrameData flag]:                                      │
//! │     skipExternalOffset : u64                                            │
//! │     skip that many bytes                                                │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │ ── Packet loop ──                                                       │
//! │   loop:                                                                 │
//! │     [if HasStreamingFixes]:  seenLevelIndex : IntPacked (ignored)        │
//! │     packetSize : i32                                                    │
//! │     [if packetSize == 0 → frame ends]                                   │
//! │     [if packetSize <  0 → error]                                        │
//! │     packet data: packetSize bytes → emitted to caller                   │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Flag semantics
//!
//! The header `flags` field (from [`vrf_container::ReplayHeader::flags`]) controls
//! two optional steps:
//!
//! | Bit | Name | Effect |
//! |-----|------|--------|
//! | 1 (0x02) | `HasStreamingFixes` | Enables the streaming-level-fixes path and per-packet `seenLevelIndex` |
//! | 3 (0x08) | `GameSpecificFrameData` | Enables the game-specific skip section |
//!
//! All VALORANT replays observed to date set **both** flags.

#![forbid(unsafe_code)]

mod error;

pub use error::FrameError;

use vrf_bitio::BitReader;
use vrf_schema::NetGuidCache;

/// Replay header flags that control DemoFrame parsing.
///
/// Source: `Replay.Models/Replay/ReplayHeaderFlags.cs`
/// ```csharp
/// HasStreamingFixes = 1 << 1,   // 0x02
/// GameSpecificFrameData = 1 << 3, // 0x08
/// ```
pub const FLAG_HAS_STREAMING_FIXES: u32 = 1 << 1;
pub const FLAG_GAME_SPECIFIC_FRAME_DATA: u32 = 1 << 3;

/// Maximum packet size (bytes). Unreal's `MAX_PACKET_SIZE` = 2 KiB.
/// Source: `Constants.cs` — `public const int MaxPacketSizeInBits = 16384`.
const MAX_PACKET_SIZE_BYTES: i32 = 16384 / 8; // 2048

/// Maximum sane FString bytes when skipping level names.
const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

/// A single packet extracted from the DemoFrame stream.
///
/// `time_ms` is derived from the frame's `timeSeconds` field (converted
/// `* 1000` and truncated to u32 — matching the C# parser's practice).
#[derive(Debug, Clone)]
pub struct DemoPacket<'a> {
    /// Time of the enclosing DemoFrame, in milliseconds.
    pub time_ms: u32,
    /// Sequential packet index (0-based across the entire chunk).
    pub packet_index: u32,
    /// Raw packet bytes (pass to `ReplicationReader::process_packet`).
    pub data: &'a [u8],
}

/// Iterate all DemoFrames in a decompressed ReplayData chunk, emitting packets.
///
/// `data` is the full decompressed chunk. `flags` is `ReplayHeader.flags`.
/// `cache` receives schema updates (net-field exports and export GUIDs)
/// that arrive in the ExportData section of each frame.
///
/// The callback `on_packet` is invoked for every packet in the stream, with
/// the frame's time and the packet's raw bytes. This is allocation-free for
/// the packet data (slices into `data`).
///
/// Returns the total number of packets yielded.
pub fn iter_demo_frames(
    data: &[u8],
    flags: u32,
    cache: &mut NetGuidCache,
    mut on_packet: impl FnMut(DemoPacket<'_>),
) -> Result<u32, FrameError> {
    let has_streaming_fixes = (flags & FLAG_HAS_STREAMING_FIXES) != 0;
    let has_game_specific = (flags & FLAG_GAME_SPECIFIC_FRAME_DATA) != 0;

    let mut reader = BitReader::new(data);
    let mut packet_index: u32 = 0;

    while !reader.at_end() {
        // ── Frame header ──────────────────────────────────────────────────
        let _current_level_index = reader.read_i32().map_err(FrameError::bit)?;
        let time_seconds = reader.read_f32().map_err(FrameError::bit)?;
        // Mirror the reference exactly (ReplayEventJsonWriter.cs:194):
        //   (long)Math.Round(seconds * 1000d, MidpointRounding.AwayFromZero)
        // Both halves matter. Promoting to f64 before scaling avoids rounding
        // the product to the nearest f32 first, and rounding rather than
        // truncating is what keeps timestamps aligned -- truncation put every
        // frame whose fractional millisecond was >= 0.5 one millisecond early.
        // Rust's f64::round is already half-away-from-zero, and the `as u32`
        // cast saturates, so non-finite input yields 0 as the reference does.
        let time_ms = (f64::from(time_seconds) * 1000.0).round() as u32;

        // ── ExportData ────────────────────────────────────────────────────
        read_export_data(&mut reader, cache)?;

        // ── StreamingLevelFixes ───────────────────────────────────────────
        read_streaming_level_fixes(&mut reader, has_streaming_fixes)?;

        // ── ExternalData ──────────────────────────────────────────────────
        read_external_data(&mut reader)?;

        // ── GameSpecificFrameData ─────────────────────────────────────────
        read_game_specific_frame_data(&mut reader, has_game_specific)?;

        // ── Packet loop ───────────────────────────────────────────────────
        loop {
            if has_streaming_fixes {
                let _seen_level_index = reader.read_int_packed().map_err(FrameError::bit)?;
            }

            let packet_size = reader.read_i32().map_err(FrameError::bit)?;
            if packet_size == 0 {
                break;
            }
            if packet_size < 0 {
                return Err(FrameError::NegativePacketSize { size: packet_size });
            }
            if packet_size > MAX_PACKET_SIZE_BYTES {
                return Err(FrameError::PacketTooLarge {
                    size: packet_size,
                    max: MAX_PACKET_SIZE_BYTES,
                });
            }

            let packet_size_usize = packet_size as usize;
            let bit_count = (packet_size_usize as u64) * 8;
            if reader.bits_remaining() < bit_count {
                return Err(FrameError::Truncated {
                    context: "packet data",
                    needed: packet_size_usize,
                    available: (reader.bits_remaining() / 8) as usize,
                });
            }

            // Extract packet bytes as a slice into the original data buffer.
            // Position in the data array: start_bit is always 0 for our reader,
            // so absolute bit position = reader.position().
            let byte_offset = (reader.position() / 8) as usize;
            let packet_data = &data[byte_offset..byte_offset + packet_size_usize];
            reader.skip_bits(bit_count).map_err(FrameError::bit)?;

            on_packet(DemoPacket {
                time_ms,
                packet_index,
                data: packet_data,
            });
            packet_index += 1;
        }
    }

    Ok(packet_index)
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// ExportData: read net field exports + export GUIDs into the cache.
///
/// Source: `ExportDataReader.Read()` — calls `ReadNetFieldExports()` then
/// `ReadExportGuids()`. Both are byte-aligned (FBinaryArchive) reads.
fn read_export_data(
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
fn read_streaming_level_fixes(
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
        // FTransform = rotation(4 doubles) + translation(3 doubles) + scale(3 doubles)
        // = 10 × f64 = 80 bytes. But Unreal's FTransform in FBinaryArchive is typically
        // serialized as 3 × FVector (rotation quaternion not stored as doubles in this
        // context — let me check). Actually looking at the C# code it calls
        // `_archive.ReadFTransform()` which in their implementation reads
        // rotation(4×f32) + translation(3×f32) + scale(3×f32) = 10 × f32 = 40 bytes.
        for _ in 0..num_levels {
            let _ = reader
                .read_fstring(MAX_FSTRING_BYTES)
                .map_err(FrameError::bit)?;
            let _ = reader
                .read_fstring(MAX_FSTRING_BYTES)
                .map_err(FrameError::bit)?;
            // FTransform: Rotation(4×f32) + Translation(3×f32) + Scale3D(3×f32) = 40 bytes
            reader.skip_bits(40 * 8).map_err(FrameError::bit)?;
        }
    }

    Ok(())
}

/// ExternalData: loop reading numBits + netGuid + skip, until numBits == 0.
///
/// Source: `PlaybackPacketReader.ReadExternalData()`
fn read_external_data(reader: &mut BitReader<'_>) -> Result<(), FrameError> {
    loop {
        let num_bits = reader.read_int_packed().map_err(FrameError::bit)?;
        if num_bits == 0 {
            return Ok(());
        }
        let _net_guid = reader.read_int_packed().map_err(FrameError::bit)?;
        let byte_count = num_bits.div_ceil(8) as u64;
        reader.skip_bits(byte_count * 8).map_err(FrameError::bit)?;
    }
}

/// GameSpecificFrameData: optionally read a u64 skip-offset and skip that many bytes.
///
/// Source: `GameSpecificFrameDataReader.Read()`
fn read_game_specific_frame_data(
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
    reader.skip_bits(skip_offset * 8).map_err(FrameError::bit)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal DemoFrame with one packet.
    fn build_minimal_frame(time_secs: f32, packet: &[u8], flags: u32) -> Vec<u8> {
        let mut data = Vec::new();
        // currentLevelIndex: i32
        data.extend_from_slice(&0i32.to_le_bytes());
        // timeSeconds: f32
        data.extend_from_slice(&time_secs.to_le_bytes());
        // ExportData: numLayoutCmdExports = 0
        data.push(0); // IntPacked(0) = single byte 0x00
        // ExportData: numExportGuids = 0
        data.push(0);
        // StreamingLevelFixes (with HasStreamingFixes set):
        if flags & FLAG_HAS_STREAMING_FIXES != 0 {
            // numLevels = 0
            data.push(0);
            // externalOffset = 0
            data.extend_from_slice(&0u64.to_le_bytes());
        } else {
            // numLevels = 0
            data.push(0);
        }
        // ExternalData: numBits = 0 (terminator)
        data.push(0);
        // GameSpecificFrameData (with flag set):
        if flags & FLAG_GAME_SPECIFIC_FRAME_DATA != 0 {
            // skipExternalOffset = 0
            data.extend_from_slice(&0u64.to_le_bytes());
        }
        // Packet loop:
        if flags & FLAG_HAS_STREAMING_FIXES != 0 {
            // seenLevelIndex (IntPacked)
            data.push(0);
        }
        // packetSize
        let pkt_size = packet.len() as i32;
        data.extend_from_slice(&pkt_size.to_le_bytes());
        // packet data
        data.extend_from_slice(packet);
        // End of frame: seenLevelIndex + size=0
        if flags & FLAG_HAS_STREAMING_FIXES != 0 {
            data.push(0);
        }
        data.extend_from_slice(&0i32.to_le_bytes());
        data
    }

    #[test]
    fn minimal_frame_yields_one_packet() {
        let flags = FLAG_HAS_STREAMING_FIXES | FLAG_GAME_SPECIFIC_FRAME_DATA;
        let packet_payload = &[0xDE, 0xAD, 0xBE, 0xEF];
        let data = build_minimal_frame(12.5, packet_payload, flags);
        let mut cache = NetGuidCache::new();
        let mut received = Vec::new();

        let count = iter_demo_frames(&data, flags, &mut cache, |pkt| {
            received.push((pkt.time_ms, pkt.data.to_vec()));
        })
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, 12500);
        assert_eq!(received[0].1, packet_payload);
    }

    /// Run one frame and return the packet's `time_ms`.
    fn time_ms_of(time_secs: f32) -> u32 {
        let flags = FLAG_HAS_STREAMING_FIXES | FLAG_GAME_SPECIFIC_FRAME_DATA;
        let data = build_minimal_frame(time_secs, &[0x00], flags);
        let mut cache = NetGuidCache::new();
        let mut out = 0;
        iter_demo_frames(&data, flags, &mut cache, |pkt| out = pkt.time_ms).unwrap();
        out
    }

    #[test]
    fn time_ms_rounds_half_away_from_zero_like_the_reference() {
        // ReplayEventJsonWriter.cs:194 --
        //   (long)Math.Round(seconds * 1000d, MidpointRounding.AwayFromZero)
        //
        // Truncating instead put roughly half of all timestamps 1 ms early,
        // which is the entire "systematic -1ms offset" this project had
        // recorded as a bunch-boundary choice. It is neither systematic nor
        // about boundaries: it only shows up when the fractional millisecond
        // is >= 0.5.
        assert_eq!(time_ms_of(12.5), 12500, "exact value must be unchanged");
        assert_eq!(time_ms_of(0.0004), 0, "below half a ms rounds down");
        assert_eq!(time_ms_of(0.0005), 1, "exactly half rounds away from zero");
        assert_eq!(time_ms_of(0.0006), 1, "above half rounds up");
        assert_eq!(time_ms_of(1.9999), 2000, "carries into the next second");
    }

    #[test]
    fn time_ms_matches_the_reference_formula_across_a_match() {
        // Sweep realistic frame timestamps -- a competitive match runs to
        // roughly 2,300 s -- against the reference expression, computed in
        // double precision exactly as ReplayEventJsonWriter.cs does. This
        // catches both the rounding rule and any f32-vs-f64 drift in the
        // multiply, which a handful of named cases would not.
        let mut checked = 0;
        for step in 0..2000 {
            let secs = (step as f32) * 1.1597; // ~0 to ~2319 s, uneven fractions
            let expected = (f64::from(secs) * 1000.0).round() as u32;
            assert_eq!(time_ms_of(secs), expected, "at {secs} s");
            checked += 1;
        }
        assert_eq!(checked, 2000);
    }

    #[test]
    fn empty_data_yields_zero_packets() {
        let mut cache = NetGuidCache::new();
        let count = iter_demo_frames(
            &[],
            FLAG_HAS_STREAMING_FIXES | FLAG_GAME_SPECIFIC_FRAME_DATA,
            &mut cache,
            |_| {},
        )
        .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn negative_packet_size_is_error() {
        let flags = FLAG_HAS_STREAMING_FIXES | FLAG_GAME_SPECIFIC_FRAME_DATA;
        let mut data = Vec::new();
        data.extend_from_slice(&0i32.to_le_bytes()); // levelIndex
        data.extend_from_slice(&1.0f32.to_le_bytes()); // time
        data.push(0); // export data count
        data.push(0); // export guid count
        data.push(0); // numLevels
        data.extend_from_slice(&0u64.to_le_bytes()); // externalOffset
        data.push(0); // external data terminator
        data.extend_from_slice(&0u64.to_le_bytes()); // game specific
        data.push(0); // seenLevelIndex
        data.extend_from_slice(&(-1i32).to_le_bytes()); // negative size!

        let mut cache = NetGuidCache::new();
        let err = iter_demo_frames(&data, flags, &mut cache, |_| {}).unwrap_err();
        assert!(matches!(err, FrameError::NegativePacketSize { size: -1 }));
    }
}
