//! DemoFrame iteration: decompressed replay-data chunk -> `(time_ms, packet)` sequence.
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
//! Unreal's `FBinaryArchive` -- i.e. `IntPacked` is still the 7-bit-per-byte
//! encoding, `FString` is i32 length + bytes + null, etc.):
//!
//! ```text
//! +--------------------------------------------------------------------------+
//! | currentLevelIndex   : i32                                  (ignored)     |
//! | timeSeconds         : f32                                  (frame time)  |
//! +--------------------------------------------------------------------------+
//! | -- ExportData --                                                         |
//! |   numLayoutCmdExports: IntPacked -> ReadNetFieldExports                  |
//! |   numExportGuids:      IntPacked -> ReadExportGuids                      |
//! +--------------------------------------------------------------------------+
//! | -- StreamingLevelFixes --                                                |
//! |   [if HasStreamingFixes flag]:                                           |
//! |     numLevels : IntPacked                                                |
//! |     for each: FString (level name)                                       |
//! |     externalOffset : u64                                                 |
//! |   [else]:                                                                |
//! |     numLevels : IntPacked                                                |
//! |     for each: FString + FString + FTransform (10 floats = 40 bytes)      |
//! +--------------------------------------------------------------------------+
//! | -- ExternalData --                                                       |
//! |   loop:                                                                  |
//! |     numBits : IntPacked   (0 -> break)                                   |
//! |     netGuid : IntPacked   (ignored)                                      |
//! |     skip ceil(numBits/8) bytes                                           |
//! +--------------------------------------------------------------------------+
//! | -- GameSpecificFrameData --                                              |
//! |   [if GameSpecificFrameData flag]:                                       |
//! |     skipExternalOffset : u64                                             |
//! |     skip that many bytes                                                 |
//! +--------------------------------------------------------------------------+
//! | -- Packet loop --                                                        |
//! |   loop:                                                                  |
//! |     [if HasStreamingFixes]:  seenLevelIndex : IntPacked (ignored)        |
//! |     packetSize : i32                                                     |
//! |     [if packetSize == 0 -> frame ends]                                   |
//! |     [if packetSize <  0 -> error]                                        |
//! |     packet data: packetSize bytes -> emitted to caller                   |
//! +--------------------------------------------------------------------------+
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
//! The reference replay sets only `HasStreamingFixes` (header flags `0x0002`),
//! so its game-specific section is absent on every one of its 226,190 frames.
//!
//! # Module map
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | `lib` | [`iter_demo_frames`], the frame header and the packet loop |
//! | `sections` | The four fixed sections that precede the packet loop |
//! | `error` | [`FrameError`] |
//!
//! # Cargo features
//!
//! None. Every part of this crate is on the single path from a decompressed
//! chunk to a packet: the four sections are not optional stages a consumer may
//! decline, they are byte ranges that must be consumed in order for the frame
//! cursor to stay aligned. A flag over any of them would only produce a build
//! that mis-parses.

#![forbid(unsafe_code)]

mod error;
mod sections;

pub use error::FrameError;

use vrf_bitio::BitReader;
use vrf_schema::NetGuidCache;

use sections::{
    read_export_data, read_external_data, read_game_specific_frame_data, read_streaming_level_fixes,
};

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
/// Source: `Constants.cs` -- `public const int MaxPacketSizeInBits = 16384`.
const MAX_PACKET_SIZE_BYTES: i32 = 16384 / 8; // 2048

/// A single packet extracted from the DemoFrame stream.
///
/// `time_ms` is derived from the frame's `timeSeconds` field: scaled by 1000
/// in f64, rounded half away from zero, and 0 when `timeSeconds` is not
/// finite. All three match the C# reference; see the conversion site.
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
        // -- Frame header --------------------------------------------------
        let _current_level_index = reader.read_i32().map_err(FrameError::bit)?;
        let time_seconds = reader.read_f32().map_err(FrameError::bit)?;
        // Mirror the reference exactly (ReplayEventJsonWriter.cs:194):
        //   float.IsFinite(seconds)
        //     ? (long)Math.Round(seconds * 1000d, MidpointRounding.AwayFromZero)
        //     : 0
        // All three parts matter. Promoting to f64 before scaling avoids
        // rounding the product to the nearest f32 first, and rounding rather
        // than truncating is what keeps timestamps aligned -- truncation put
        // every frame whose fractional millisecond was >= 0.5 one millisecond
        // early. Rust's f64::round is already half-away-from-zero.
        //
        // The finiteness test has to be written out. `as u32` saturates, which
        // happens to give 0 for NaN and -inf but gives u32::MAX for +inf --
        // the opposite end of the range from what the reference produces. That
        // is reachable: `time_seconds` is a raw read_f32 over replay bytes with
        // no validation, so any bit pattern can arrive here.
        let time_ms = if time_seconds.is_finite() {
            (f64::from(time_seconds) * 1000.0).round() as u32
        } else {
            0
        };

        // -- ExportData ----------------------------------------------------
        read_export_data(&mut reader, cache)?;

        // -- StreamingLevelFixes -------------------------------------------
        read_streaming_level_fixes(&mut reader, has_streaming_fixes)?;

        // -- ExternalData --------------------------------------------------
        read_external_data(&mut reader)?;

        // -- GameSpecificFrameData -----------------------------------------
        read_game_specific_frame_data(&mut reader, has_game_specific)?;

        // -- Packet loop ---------------------------------------------------
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
    fn non_finite_time_seconds_yields_zero_like_the_reference() {
        // ReplayEventJsonWriter.cs:194 guards the conversion explicitly:
        //   float.IsFinite(seconds)
        //     ? (long)Math.Round(seconds * 1000d, MidpointRounding.AwayFromZero)
        //     : 0
        //
        // `time_seconds` is a raw read_f32 with no validation, so every bit
        // pattern -- including the quiet-NaN and infinity encodings -- is
        // representable in a replay. Relying on the `as u32` cast to stand in
        // for that guard only works for two of the three: the cast saturates,
        // so +inf lands on u32::MAX rather than 0.
        assert_eq!(time_ms_of(f32::NAN), 0, "NaN");
        assert_eq!(time_ms_of(f32::NEG_INFINITY), 0, "-inf");
        assert_eq!(time_ms_of(f32::INFINITY), 0, "+inf");
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
