//! ClassNetCache payload brute-forcer for unresolved groups.
//!
//! When a ClassNetCache block's group is never declared in the replay,
//! `function_count` -- the handle width for the RPC stream -- is unknown, and
//! [`vrf_net::field::parse_class_net_cache`] refuses to walk it. The payload is
//! preserved as raw bits (one `__vrfkit_unresolved_class_net_cache_payload__`
//! row), but every RPC inside is lost.
//!
//! This module recovers that structure. For a payload whose group is a known
//! bare instance name (e.g. `AbilitiesAndBuffsComponent`), it brute-forces
//! `function_count` from 2 to 256 by walking the ClassNetCache stream with each
//! candidate and checking whether the walk consumes the buffer exactly.
//!
//! # Wire format
//!
//! The ClassNetCache RPC stream (confirmed against `parse_class_net_cache` in
//! `vrf-net` and the C# `ParseClassNetCachePayload`) is:
//!
//! ```text
//! loop:
//!   handle       = SerializedInt(max(function_count, 2))
//!   payload_bits = IntPacked
//!   payload      = sub-reader of payload_bits bits
//! ```
//!
//! There is no checksum bit and no explicit handle-0 terminator: the loop runs
//! until the bit reader is at end. A "clean walk" is one where every iteration
//! reads a handle, a well-formed payload-length, and a payload that fits, and
//! the final payload ends exactly at the stream boundary.
//!
//! # The function_count ambiguity
//!
//! `SerializedInt(max)` spends `floor(log2(max))` bits unconditionally plus one
//! extra bit conditionally. For handle values that are small relative to `max`,
//! several adjacent `function_count` values produce the same handle width and
//! therefore the same walk. On `AbilitiesAndBuffsComponent` every payload
//! contains a single RPC at handle 1, and `function_count` 34-65 all walk
//! cleanly -- the handle takes 6 bits in every case. The brute-force returns
//! the **minimum** valid `function_count`, which is sufficient to decode the
//! stream: every fc in the valid range produces identical RPC structure.
//!
//! # What the decoder does NOT do
//!
//! The RPC payload itself may use custom serialization rather than the standard
//! RepLayout `FunctionParameters` grammar. On `AbilitiesAndBuffsComponent` the
//! inner payload does not parse as `FunctionParameters` -- the function at
//! handle 1 uses a class-specific serializer. This module recovers the outer
//! RPC framing (handle, payload offset/size) but leaves the inner payload as
//! raw bits.

use vrf_bitio::BitReader;

/// Maximum `function_count` to try. Real ClassNetCache groups in VALORANT
/// declare at most a few dozen functions; 256 is a generous ceiling.
const MAX_FC: u32 = 256;

/// One decoded RPC from a brute-forced ClassNetCache stream.
#[derive(Debug, Clone)]
pub struct CncRpc {
    /// The function handle (0-indexed).
    pub handle: u32,
    /// Payload bit count.
    pub payload_bits: u32,
    /// Bit offset of the payload within the original buffer.
    pub payload_offset: u64,
}

/// The result of brute-forcing a ClassNetCache payload.
#[derive(Debug, Clone)]
pub struct BruteForceResult {
    /// The minimum `function_count` that produces a clean walk.
    pub function_count: u32,
    /// The RPCs decoded with that function_count.
    pub rpcs: Vec<CncRpc>,
}

/// Brute-force `function_count` for a ClassNetCache payload whose group is
/// unresolved.
///
/// Tries each `function_count` from 2 to `MAX_FC`. The minimum value whose walk
/// consumes the entire buffer cleanly (zero residual bits, at least one RPC) is
/// returned. If no value works, returns `None`.
///
/// The payload is the transform-decoded byte buffer of the content block, and
/// `bit_count` is its declared bit length (the same pair the preservation row
/// stores).
///
/// The caller is expected to gate this on a specific group path (e.g.
/// `AbilitiesAndBuffsComponent`) so that the brute-force is not attempted on
/// every unresolved block. A group whose structure is genuinely unknown will
/// return `None` and the preservation row stays as the only record.
#[must_use]
pub fn brute_force_function_count(payload: &[u8], bit_count: u32) -> Option<BruteForceResult> {
    for fc in 2..=MAX_FC {
        if let Some(rpcs) = walk_cnc(payload, bit_count, fc) {
            return Some(BruteForceResult {
                function_count: fc,
                rpcs,
            });
        }
    }
    None
}

/// Walk a ClassNetCache stream with a known `function_count` and return the
/// RPCs it contains.
///
/// This is the per-payload decode used after the function_count has been
/// determined (e.g. by [`brute_force_function_count`] on a representative
/// sample, or by a hardcoded constant for a known group). Returns `None` when
/// the walk is not clean -- the caller should keep the preservation row in that
/// case.
#[must_use]
pub fn decode_cnc_payload(
    payload: &[u8],
    bit_count: u32,
    function_count: u32,
) -> Option<Vec<CncRpc>> {
    walk_cnc(payload, bit_count, function_count)
}

/// Walk a ClassNetCache stream with a given `function_count`.
///
/// Mirrors `parse_class_net_cache` in `vrf-net` but returns the RPC list
/// instead of driving a sink, and returns `None` when the walk is not clean
/// (residual bits or malformed reads) rather than partial results.
fn walk_cnc(payload: &[u8], bit_count: u32, function_count: u32) -> Option<Vec<CncRpc>> {
    let mut reader = BitReader::with_bit_len(payload, u64::from(bit_count));
    let handle_max = function_count.max(2);
    let mut rpcs = Vec::new();

    while !reader.at_end() {
        let handle = reader.read_serialized_int(handle_max).ok()?;

        // `parse_class_net_cache` requires at least 8 bits for the payload
        // length IntPacked read; fewer means a malformed tail.
        if reader.bits_remaining() < 8 {
            return None;
        }

        let payload_bits = reader.read_int_packed().ok()?;

        if u64::from(payload_bits) > reader.bits_remaining() {
            return None;
        }

        let payload_offset = reader.position();
        // Advance past the payload.
        let Ok(mut sub) = reader.sub_reader(u64::from(payload_bits)) else {
            return None;
        };
        sub.skip_remaining();

        rpcs.push(CncRpc {
            handle,
            payload_bits,
            payload_offset,
        });
    }

    // A clean walk leaves zero residual bits. Sub-byte padding would be
    // ignored by `at_end()` but cannot carry an element, so we also reject it
    // -- a payload that is not this format usually parses into something
    // plausible and then leaves a tail, and the zero-residual check is what
    // separates "decoded" from "decoded into a plausible-looking structure".
    if reader.bits_remaining() != 0 || rpcs.is_empty() {
        return None;
    }

    Some(rpcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write an Unreal IntPacked value into a bit vector.
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

    /// Write a SerializedInt value with a given max.
    fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max: u32) {
        let mut written = 0u32;
        let mut mask = 1u32;
        while written.saturating_add(mask) < max {
            let bit = (value & mask) != 0;
            bits.push(bit);
            if bit {
                written |= mask;
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

    /// Build a ClassNetCache stream with one RPC at handle 1, using a given
    /// function_count to determine the handle width.
    ///
    /// The payload is filled with 1-bits (`true`) rather than zeros. This is
    /// load-bearing for the brute-force tests: a zero-filled payload lets
    /// wrong `function_count` values walk cleanly because every misaligned
    /// IntPacked read returns 0 (zero-cost payload), producing many tiny
    /// garbage RPCs that consume the buffer. A one-filled payload makes those
    /// misaligned reads return large values that overrun the stream, so only
    /// the correct handle width walks cleanly.
    fn build_one_rpc_stream(function_count: u32, payload_bits: u32) -> (Vec<u8>, u32) {
        let mut bits = Vec::new();
        let handle_max = function_count.max(2);
        write_serialized_int(&mut bits, 1, handle_max);
        write_int_packed(&mut bits, payload_bits);
        bits.extend(std::iter::repeat_n(true, payload_bits as usize));
        let bit_count = bits.len() as u32;
        (bits_to_bytes(&bits), bit_count)
    }

    /// A single-RPC payload at handle 1 should resolve to the minimum
    /// function_count whose handle width matches the one it was written with.
    #[test]
    fn single_rpc_finds_minimum_fc() {
        // fc=34: for handle=1, ilog2(34)=5 bits, then 1+32=33 < 34, so the
        // extra bit is read: 6 bits total. The minimum fc that produces a
        // 6-bit handle for value=1 is 34 (fc=33 gives 5 bits, fc=34 forces
        // the extra read).
        let (data, bit_count) = build_one_rpc_stream(34, 100);
        let result = brute_force_function_count(&data, bit_count);
        assert!(result.is_some(), "should find a valid fc");
        let result = result.unwrap();
        assert_eq!(result.function_count, 34);
        assert_eq!(result.rpcs.len(), 1);
        assert_eq!(result.rpcs[0].handle, 1);
        assert_eq!(result.rpcs[0].payload_bits, 100);
    }

    /// A stream written with a larger fc that changes the handle width should
    /// still resolve correctly.
    #[test]
    fn single_rpc_with_larger_fc() {
        // fc=128: for handle=1, ilog2(128)=7, 1+128=129 >= 128, so 7 bits.
        // fc=65..127 also gives 7 bits for value=1 (1+64=65 < 65..127), but
        // fc=64 gives 6 bits (1+64=65 >= 64). The minimum fc requiring the
        // 7-bit encoding is 66. But wait -- for value=1 with fc=66: ilog2=6,
        // read 6 bits → 1, 1+64=65 < 66, read extra bit → 7 bits.
        //
        // For fc=65: ilog2=6, read 6 bits → 1, 1+64=65 >= 65 → 6 bits.
        // For fc=66: ilog2=6, read 6 bits → 1, 1+64=65 < 66 → 7 bits.
        //
        // So the minimum fc for 7-bit handle is 66, not 128.
        let (data, bit_count) = build_one_rpc_stream(128, 50);
        let result = brute_force_function_count(&data, bit_count);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.function_count, 66);
        assert_eq!(result.rpcs[0].handle, 1);
    }

    /// Multiple RPCs in one stream should all be recovered.
    #[test]
    fn multiple_rpcs() {
        let mut bits = Vec::new();
        let handle_max = 10u32;
        // RPC 1: handle=3, payload=16 bits of 1s
        write_serialized_int(&mut bits, 3, handle_max);
        write_int_packed(&mut bits, 16);
        bits.extend(std::iter::repeat_n(true, 16));
        // RPC 2: handle=7, payload=8 bits of 1s
        write_serialized_int(&mut bits, 7, handle_max);
        write_int_packed(&mut bits, 8);
        bits.extend(std::iter::repeat_n(true, 8));

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;
        let result = brute_force_function_count(&data, bit_count);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.rpcs.len(), 2);
        assert_eq!(result.rpcs[0].handle, 3);
        assert_eq!(result.rpcs[0].payload_bits, 16);
        assert_eq!(result.rpcs[1].handle, 7);
        assert_eq!(result.rpcs[1].payload_bits, 8);
    }

    /// A zero-bit payload returns None (no RPCs).
    #[test]
    fn empty_payload_returns_none() {
        let data = vec![];
        let result = brute_force_function_count(&data, 0);
        assert!(result.is_none());
    }

    /// The brute-force returns the minimum valid fc: multiple fc values in the
    /// same bit-width band produce the same walk, and we pick the smallest.
    #[test]
    fn returns_minimum_valid_fc() {
        // fc=34 and fc=65 both produce 6-bit handles for value=1.
        // Build with fc=50 (also 6 bits for value=1):
        let (data, bit_count) = build_one_rpc_stream(50, 64);
        let result = brute_force_function_count(&data, bit_count).unwrap();
        // The minimum fc where the 6-bit handle walks cleanly is 34.
        assert_eq!(result.function_count, 34);
    }
}
