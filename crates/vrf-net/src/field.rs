//! Field stream parsing -- self-describing property and RPC iteration.
//!
//! # Design: no skipping, no descriptors
//!
//! The field stream is self-describing: each field encodes its own handle and
//! bit length. This means we can iterate **every** field without knowing its
//! type. The caller receives `(handle, bit_count, &BitReader)` for every
//! field/RPC through a sink trait, and decides what to do with the raw bits.
//!
//! This is the fundamental difference from the upstream parser, which skips
//! fields that have no registered descriptor.
//!
//! # RepLayout field stream bit layout
//!
//! ```text
//! propertyChecksum: 1 bit (ignored)
//! loop:
//!   encodedHandle: IntPacked
//!   if encodedHandle == 0 -> break
//!   handle = encodedHandle - 1
//!   payloadBitCount: IntPacked
//!   fieldPayload: sub-reader of payloadBitCount
//!   emit (handle, payloadBitCount, fieldPayload)
//! ```
//!
//! # ClassNetCache (RPC) stream bit layout
//!
//! ```text
//! loop:
//!   handle: SerializedInt(max(function_count, 2))
//!   payloadBitCount: IntPacked
//!   rpcPayload: sub-reader of payloadBitCount
//!   emit (handle, payloadBitCount, rpcPayload)
//! ```

use vrf_bitio::BitReader;

use crate::error::{NetError, Result};

/// Sink that receives every field/RPC payload without exception.
///
/// Implementors decode the fields they care about and can simply return
/// for the rest. The raw bits are available through the sub-reader.
pub trait FieldSink {
    /// A RepLayout property field was encountered.
    ///
    /// `reader` is bounded to exactly `bit_count` bits.
    fn on_field(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>);

    /// A ClassNetCache RPC invocation was encountered.
    ///
    /// `reader` is bounded to exactly `bit_count` bits.
    fn on_rpc(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>);
}

/// Parse a RepLayout property stream, emitting every field to the sink.
///
/// Returns the number of fields emitted.
pub fn parse_rep_layout(reader: &mut BitReader<'_>, sink: &mut dyn FieldSink) -> Result<u32> {
    // Property checksum bit -- always present, always ignored.
    let _checksum = reader.read_bit()?;

    let mut field_count = 0u32;

    while !reader.at_end() {
        let encoded_handle = reader.read_int_packed()?;
        if encoded_handle == 0 {
            break;
        }

        let handle = encoded_handle - 1;
        // A zero-bit payload is valid (an empty field) and is emitted like any
        // other: `sub_reader(0)` yields an empty window and the overrun test
        // below is trivially false for it, so it needs no special case.
        let payload_bits = reader.read_int_packed()?;

        if payload_bits as u64 > reader.bits_remaining() {
            // Malformed: declared more bits than available.
            // Skip remaining and return what we have.
            reader.skip_remaining();
            break;
        }

        let sub = reader.sub_reader(payload_bits as u64)?;
        sink.on_field(handle, payload_bits, sub);
        field_count += 1;
    }

    Ok(field_count)
}

/// Parse a ClassNetCache RPC stream, emitting every invocation to the sink.
///
/// `function_count` is the number of functions in the class's net cache --
/// this is the upper bound for `read_serialized_int`. The caller provides it
/// because this layer does not have access to descriptors.
///
/// # Handle-read clamp (minimum of two)
///
/// Unreal's `UActorChannel::ReadFieldHeaderAndPayload` reads the RPC handle as:
///
/// ```text
/// ReadInt(FMath::Max(NetFieldExportGroup->NetFieldExports.Num(), 2))
/// ```
///
/// The clamp to 2 is critical: when a ClassNetCache declares exactly one
/// function export, `SerializeInt(value, 1)` would consume **zero bits**
/// (`ceil(log2(1)) = 0`), but the wire payload always contains the one-bit
/// handle written by `SerializeInt(value, 2)` on the server.
///
/// Without the clamp the reader falls one bit behind the true position,
/// causing downstream IntPacked reads to desynchronise. The four stream
/// failures in the corpus (SegmentManager x2, Spline x1, MapMissileMarker x1)
/// are all capacity-1 groups where this one-bit desync accumulates into an
/// unrecoverable error. Arithmetic confirms: consuming 1 extra bit at the
/// handle aligns every subsequent payload length and field boundary perfectly
/// to the block end with zero bits remaining.
///
/// Source: `Engine/Source/Runtime/Engine/Private/DataChannel.cpp`, confirmed by
/// `Shiqan/FortniteReplayDecompressor` (C#) and `xNocken/replay-reader` (JS).
///
/// Returns the number of RPCs emitted.
pub fn parse_class_net_cache(
    reader: &mut BitReader<'_>,
    function_count: u32,
    sink: &mut dyn FieldSink,
) -> Result<u32> {
    if function_count == 0 {
        // Zero does not mean "a class with no functions", it means the export
        // group could not be resolved, so the handle width is unknown and the
        // records cannot be walked. Returning Ok here would drop the whole
        // payload without it appearing in any counter, leaving the oracle to
        // report a clean run over data it silently threw away. Fail instead:
        // the caller counts the bits and names the group.
        return Err(NetError::UnresolvedFunctionCount);
    }

    // Unreal clamps the serialized-int maximum to at least 2 so that even a
    // single-export group consumes exactly 1 bit for the handle on the wire.
    // Without this, read_serialized_int(1) consumes 0 bits and the stream
    // desyncs by 1 bit. Capacities >= 2 are unchanged (max(N, 2) == N), and
    // zero is already rejected above.
    let handle_max = function_count.max(2);

    let mut rpc_count = 0u32;

    while !reader.at_end() {
        let handle = reader.read_serialized_int(handle_max)?;

        if reader.bits_remaining() < 8 {
            // Not enough bits for a payload length -- malformed tail.
            reader.skip_remaining();
            break;
        }

        let payload_bits = reader.read_int_packed()?;

        if payload_bits as u64 > reader.bits_remaining() {
            reader.skip_remaining();
            break;
        }

        let sub = reader.sub_reader(payload_bits as u64)?;
        sink.on_rpc(handle, payload_bits, sub);
        rpc_count += 1;
    }

    Ok(rpc_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that records all fields/RPCs.
    #[derive(Default)]
    struct RecordingSink {
        fields: Vec<(u32, u32)>,
        rpcs: Vec<(u32, u32)>,
    }

    impl FieldSink for RecordingSink {
        fn on_field(&mut self, handle: u32, bit_count: u32, _reader: BitReader<'_>) {
            self.fields.push((handle, bit_count));
        }
        fn on_rpc(&mut self, handle: u32, bit_count: u32, _reader: BitReader<'_>) {
            self.rpcs.push((handle, bit_count));
        }
    }

    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max_value: u32) {
        let mut written_value = 0u32;
        let mut mask = 1u32;
        while written_value.saturating_add(mask) < max_value {
            let bit = (value & mask) != 0;
            bits.push(bit);
            if bit {
                written_value |= mask;
            }
            mask <<= 1;
        }
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }

    #[test]
    fn rep_layout_single_field() {
        let mut bits = Vec::new();
        bits.push(false); // checksum bit
        write_int_packed(&mut bits, 1); // encodedHandle = 1 -> handle = 0
        write_int_packed(&mut bits, 32); // 32 bits payload
        // 32 bits of payload data
        bits.extend(std::iter::repeat_n(true, 32));
        write_int_packed(&mut bits, 0); // terminator

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        let count = parse_rep_layout(&mut reader, &mut sink).unwrap();

        assert_eq!(count, 1);
        assert_eq!(sink.fields, vec![(0, 32)]);
    }

    #[test]
    fn rep_layout_multiple_fields() {
        let mut bits = Vec::new();
        bits.push(true); // checksum bit
        // Field 1: handle=0, 8 bits
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 8);
        bits.extend(std::iter::repeat_n(false, 8));
        // Field 2: handle=4, 16 bits
        write_int_packed(&mut bits, 5);
        write_int_packed(&mut bits, 16);
        bits.extend(std::iter::repeat_n(true, 16));
        write_int_packed(&mut bits, 0); // terminator

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        let count = parse_rep_layout(&mut reader, &mut sink).unwrap();

        assert_eq!(count, 2);
        assert_eq!(sink.fields, vec![(0, 8), (4, 16)]);
    }

    #[test]
    fn rep_layout_empty_stream() {
        let mut bits = Vec::new();
        bits.push(false); // checksum
        write_int_packed(&mut bits, 0); // immediate terminator

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        let count = parse_rep_layout(&mut reader, &mut sink).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn class_net_cache_single_rpc() {
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 2, 10); // handle = 2, max = 10
        write_int_packed(&mut bits, 16); // 16 bits payload
        bits.extend(std::iter::repeat_n(false, 16));

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        let count = parse_class_net_cache(&mut reader, 10, &mut sink).unwrap();

        assert_eq!(count, 1);
        assert_eq!(sink.rpcs, vec![(2, 16)]);
    }

    #[test]
    fn class_net_cache_zero_functions_skips() {
        // function_count=0 means the group could not be resolved. The parser
        // must return Err so the caller can count skipped bits honestly.
        let data = [0xFF; 4];
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        assert!(parse_class_net_cache(&mut reader, 0, &mut sink).is_err());
        assert!(sink.rpcs.is_empty());
    }

    /// Capacity-1 groups must consume exactly 1 bit for the handle (the
    /// minimum-of-two clamp). Without the clamp, read_serialized_int(1)
    /// consumes 0 bits and the stream desyncs. This pins the fix for the
    /// four corpus stream failures caused by capacity-1 ClassNetCache groups.
    #[test]
    fn class_net_cache_capacity_one_consumes_one_bit() {
        // Build a stream for function_count=1:
        // - handle: serialized_int written with max=2 (server uses the clamp),
        //   value=0 -> 1 bit (the '0' bit)
        // - payload_bits: IntPacked(0) -> 8 bits
        // Total: 9 bits of meaningful data.
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 0, 2); // handle=0, written with max=2 (1 bit)
        write_int_packed(&mut bits, 0); // payload = 0 bits

        let data = bits_to_bytes(&bits);
        // Bound the reader to exactly the meaningful bit count so we can
        // verify consumption without byte-padding interference.
        let mut reader = BitReader::with_bit_len(&data, bits.len() as u64);
        let mut sink = RecordingSink::default();
        let count = parse_class_net_cache(&mut reader, 1, &mut sink).unwrap();

        assert_eq!(count, 1, "should emit exactly one RPC");
        assert_eq!(sink.rpcs, vec![(0, 0)]);
        // The handle consumed 1 bit (not 0), then IntPacked consumed 8 bits.
        // Total: 9 bits. The reader should be exactly at the end.
        assert_eq!(reader.position(), 9);
        assert!(reader.at_end());
    }

    /// Capacity-3 groups are unchanged by the minimum-of-two clamp because
    /// max(3, 2) == 3. This pins that the fix does not alter larger capacities.
    #[test]
    fn class_net_cache_capacity_three_unchanged() {
        // Build a stream for function_count=3:
        // - handle: serialized_int(value=1, max=3) -> 2 bits
        // - payload_bits: IntPacked(8) -> 8 bits
        // - payload: 8 bits
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 1, 3); // handle=1, max=3
        write_int_packed(&mut bits, 8); // 8 bits payload
        bits.extend(std::iter::repeat_n(true, 8)); // 8 bits of data

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();
        let count = parse_class_net_cache(&mut reader, 3, &mut sink).unwrap();

        assert_eq!(count, 1);
        assert_eq!(sink.rpcs, vec![(1, 8)]);
    }

    /// An unresolved group must stay loud.
    ///
    /// A function count of zero means the group could not be resolved, not that
    /// the class has no functions. If the minimum-of-two clamp were applied to
    /// zero as well, such a block would read a one-bit handle and emit
    /// plausible-looking garbage; an earlier hand walk of one desynced block
    /// produced seven consecutive plausible records that were all ghosts, so
    /// plausibility is not evidence of correctness. Failing here is what lets
    /// the oracle count the block instead of quietly trusting it.
    #[test]
    fn class_net_cache_unresolved_group_still_fails() {
        let mut bits = Vec::new();
        write_int_packed(&mut bits, 8);
        bits.extend(std::iter::repeat_n(true, 8));

        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::new(&data);
        let mut sink = RecordingSink::default();

        assert!(parse_class_net_cache(&mut reader, 0, &mut sink).is_err());
        assert!(sink.rpcs.is_empty());
    }
}
