//! ClassNetCache payload brute-forcer for unresolved groups.
//!
//! When a ClassNetCache block's group is never declared in the replay,
//! `function_count` -- the handle width for the RPC stream -- is unknown, and
//! `vrf_net`'s ClassNetCache parser refuses to walk it. The payload is
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
//! # What the decoder does NOT name
//!
//! The RPC payload itself does not parse as the standard RepLayout
//! `FunctionParameters` grammar -- the function at handle 1 uses a
//! class-specific serializer. This module recovers the outer RPC framing
//! (handle, payload offset/size) and, for `AbilitiesAndBuffsComponent`,
//! decomposes the inner payload into its deterministic structure (a flag bit
//! followed by a little-endian `u32` stream; see
//! `decode_abilities_and_buffs_inner`). What it does not do is assign
//! authoritative semantic names to the later words: those (ability-class
//! signature, effect specs) depend on game assets, so the raw bits are
//! preserved alongside the recovered structure.

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
    /// Another `function_count` that walks the SAME bits just as cleanly but
    /// into a DIFFERENT set of RPCs, if one exists.
    ///
    /// The search returns the first clean walk, and the module reasoned that
    /// this was safe because adjacent counts inside one handle-width band
    /// produce identical structure. True, and not the whole condition: two
    /// counts in DIFFERENT bands can both divide the same buffer exactly. This
    /// module's own fixtures depend on knowing it -- `build_one_rpc_stream`
    /// fills payloads with 1-bits specifically because a zero-filled one lets
    /// wrong counts walk cleanly.
    ///
    /// `None` means the search checked every candidate and found no competing
    /// parse, which is what makes the returned count a measurement rather than
    /// the first thing that happened to fit.
    pub ambiguous_with: Option<u32>,
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
    // The whole range is scanned even after a hit. Returning the first clean
    // walk was justified by adjacent counts in one handle-width band producing
    // identical structure -- which is true, and is a narrower statement than
    // the one being relied on. Counts in different bands can also divide the
    // same buffer exactly, and then the first hit is a choice between parses
    // rather than the only reading. See `ambiguous_with`.
    let mut chosen: Option<BruteForceResult> = None;
    for fc in 2..=MAX_FC {
        let Some(rpcs) = walk_cnc(payload, bit_count, fc) else {
            continue;
        };
        match &mut chosen {
            None => {
                chosen = Some(BruteForceResult {
                    function_count: fc,
                    rpcs,
                    ambiguous_with: None,
                });
            }
            Some(first) => {
                if first.ambiguous_with.is_none() && !same_structure(&first.rpcs, &rpcs) {
                    first.ambiguous_with = Some(fc);
                }
            }
        }
    }
    chosen
}

/// Whether two candidate walks recovered the same RPC framing.
///
/// Structure is the handle, size and position of every RPC. Two counts that
/// agree on all three describe the same stream and the choice between them does
/// not matter; a disagreement on any of them means the payload has more than
/// one clean reading.
fn same_structure(a: &[CncRpc], b: &[CncRpc]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.handle == y.handle
                && x.payload_bits == y.payload_bits
                && x.payload_offset == y.payload_offset
        })
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
    let mut reader = BitReader::with_bit_len(payload, u64::from(bit_count)).ok()?;
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

/// Decoded inner structure of an `AbilitiesAndBuffsComponent` ClassNetCache
/// RPC payload.
///
/// The outer RPC framing -- function handle plus payload size -- is recovered
/// by [`decode_cnc_payload`]. This decomposes the payload itself, which was
/// long believed to be an opaque class-specific blob. It is not: across
/// thousands of payloads the layout is fully deterministic. A single flag bit
/// (always `1`) is followed by a stream of little-endian `u32` words and an
/// optional sub-32-bit trailing residual, and `bit_count == 1 + 32 * words +
/// trailing` holds exactly on every payload.
///
/// The first two words are a prediction-key `{Current, Base}` pair: `word0`
/// is a per-actor strictly-monotonic counter, `word1` chains the previous
/// `word0`, and their difference is a small constant (1 in ~78% of payloads).
/// This stream is the Gameplay Ability System's state synchronization -- it
/// fires on every ability-system state change, not once per ability cast, so a
/// single actor emits hundreds to thousands of payloads. Every payload is
/// therefore unique, and the pair does **not** discriminate or count ability
/// casts; cast attribution must come from ability-actor spawns and the
/// `UltimateActive` flag instead. The later words carry small constant fields
/// followed, on the larger payloads, by opaque values whose meaning is
/// game-asset-dependent (the authoritative C# parser does not model this
/// stream at all), so they are exposed as a raw word list.
#[derive(Debug, Clone)]
pub struct AbilitiesActivation {
    /// The leading flag bit. Observed to be `1` on every payload; kept as a
    /// field so a future build that clears it is visible rather than silent.
    pub flag: bool,
    /// The little-endian `u32` words immediately after the flag bit.
    pub words: Vec<u32>,
    /// Trailing bits that did not form a full word, packed LSB-first.
    pub trailing: u32,
    /// Number of valid bits in `trailing` (always `0..32`).
    pub trailing_bit_count: u32,
}

impl AbilitiesActivation {
    /// The first two words -- a prediction-key `{Current, Base}` pair -- when
    /// the payload carries at least two words.
    ///
    /// These are a per-actor monotonic state-sync counter; every payload is
    /// unique. They do not group payloads into ability casts (see the struct
    /// docs).
    #[must_use]
    pub fn key_pair(&self) -> Option<(u32, u32)> {
        Some((*self.words.first()?, *self.words.get(1)?))
    }
}

/// Decompose the inner payload of an `AbilitiesAndBuffsComponent` ClassNetCache
/// RPC into its deterministic structure.
///
/// Skips the flag bit, reads whole little-endian `u32` words while at least 32
/// bits remain, and captures any trailing residual. Returns `None` only for an
/// empty payload: the decomposition is a pure bit-stream walk, so on a
/// non-empty payload it always succeeds and the words are exactly the bits the
/// wire carried.
#[must_use]
pub fn decode_abilities_and_buffs_inner(
    payload: &[u8],
    bit_count: u32,
) -> Option<AbilitiesActivation> {
    let mut reader = BitReader::with_bit_len(payload, u64::from(bit_count)).ok()?;
    if reader.bits_remaining() == 0 {
        return None;
    }
    let flag = reader.read_bit().ok()?;
    let mut words = Vec::new();
    while reader.bits_remaining() >= 32 {
        match reader.read_u32() {
            Ok(w) => words.push(w),
            Err(_) => break,
        }
    }
    let trailing_bit_count = u32::try_from(reader.bits_remaining()).unwrap_or(0);
    let trailing = if trailing_bit_count > 0 {
        reader.read_bits(trailing_bit_count).unwrap_or(0) as u32
    } else {
        0
    };
    Some(AbilitiesActivation {
        flag,
        words,
        trailing,
        trailing_bit_count,
    })
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
        // read 6 bits -> 1, 1+64=65 < 66, read extra bit -> 7 bits.
        //
        // For fc=65: ilog2=6, read 6 bits -> 1, 1+64=65 >= 65 -> 6 bits.
        // For fc=66: ilog2=6, read 6 bits -> 1, 1+64=65 < 66 -> 7 bits.
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

    /// Build a bit buffer from an explicit flag + a sequence of LE u32 words +
    /// an optional trailing residual. Mirrors the wire layout of an
    /// `AbilitiesAndBuffsComponent` RPC payload.
    fn build_activation_stream(
        flag: bool,
        words: &[u32],
        trailing_bits: u32,
        trailing: u32,
    ) -> (Vec<u8>, u32) {
        let mut bits = Vec::new();
        bits.push(flag);
        for &w in words {
            for k in 0..32 {
                bits.push((w >> k) & 1 != 0);
            }
        }
        for k in 0..trailing_bits {
            bits.push((trailing >> k) & 1 != 0);
        }
        let bit_count = bits.len() as u32;
        (bits_to_bytes(&bits), bit_count)
    }

    /// A real-shape payload: flag(1) + 5 LE u32 words, no trailing. This is
    /// the 161-bit family observed across thousands of payloads, the simplest
    /// and most common `AbilitiesAndBuffsComponent` RPC.
    #[test]
    fn abilities_inner_flag_then_u32_stream() {
        let words = [5u32, 3, 1, 0, 2];
        let (data, bit_count) = build_activation_stream(true, &words, 0, 0);
        let decoded = decode_abilities_and_buffs_inner(&data, bit_count).unwrap();
        assert!(decoded.flag);
        assert_eq!(decoded.words, words);
        assert_eq!(decoded.trailing_bit_count, 0);
        assert_eq!(decoded.key_pair(), Some((5, 3)));
    }

    /// A large payload carries a sub-32-bit trailing residual after the last
    /// whole word. The decoder must capture it without losing bits.
    #[test]
    fn abilities_inner_captures_trailing_residual() {
        // flag(1) + 1 word + 16 trailing bits = 49 bits.
        let (data, bit_count) = build_activation_stream(true, &[7], 16, 0xABCD);
        let decoded = decode_abilities_and_buffs_inner(&data, bit_count).unwrap();
        assert!(decoded.flag);
        assert_eq!(decoded.words, vec![7]);
        assert_eq!(decoded.trailing_bit_count, 16);
        assert_eq!(decoded.trailing & 0xFFFF, 0xABCD);
    }

    /// An empty payload has no flag bit and yields `None`.
    #[test]
    fn abilities_inner_empty_is_none() {
        assert!(decode_abilities_and_buffs_inner(&[], 0).is_none());
    }

    /// A flag bit alone (no words, no trailing) decodes cleanly.
    #[test]
    fn abilities_inner_flag_only() {
        let (data, bit_count) = build_activation_stream(true, &[], 0, 0);
        let decoded = decode_abilities_and_buffs_inner(&data, bit_count).unwrap();
        assert!(decoded.flag);
        assert!(decoded.words.is_empty());
        assert_eq!(decoded.trailing_bit_count, 0);
        assert_eq!(decoded.key_pair(), None);
    }

    /// A single-word payload has a flag but no key pair.
    #[test]
    fn abilities_inner_single_word_has_no_key_pair() {
        let (data, bit_count) = build_activation_stream(true, &[42], 0, 0);
        let decoded = decode_abilities_and_buffs_inner(&data, bit_count).unwrap();
        assert_eq!(decoded.words, vec![42]);
        assert_eq!(decoded.key_pair(), None);
    }

    /// A zero-filled payload can walk cleanly under SEVERAL function counts
    /// that disagree about how many RPCs it holds, and the search must say so
    /// rather than return the first one as though it were the only one.
    ///
    /// This is not hypothetical, and this module already knew it: the doc on
    /// `build_one_rpc_stream` above explains that the fixtures are filled with
    /// 1-bits precisely because "a zero-filled payload lets wrong
    /// `function_count` values walk cleanly ... producing many tiny garbage
    /// RPCs that consume the buffer". The module's own claim -- that every fc
    /// in the valid range produces identical RPC structure -- holds only
    /// within one handle-WIDTH band; it says nothing about two bands whose
    /// per-RPC size happens to divide the same buffer.
    ///
    /// 90 zero bits is exactly that: fc=2 gives a 1-bit handle and a zero
    /// payload, so 10 RPCs of 9 bits; fc=3 gives a 2-bit handle, so 9 RPCs of
    /// 10 bits. Both consume the buffer exactly and they are different parses.
    #[test]
    fn an_ambiguous_payload_reports_the_competing_function_count() {
        let data = vec![0u8; 12]; // 96 bits of storage, 90 declared
        let result = brute_force_function_count(&data, 90).expect("fc=2 walks cleanly");

        assert_eq!(result.function_count, 2);
        assert_eq!(result.rpcs.len(), 10, "fc=2 reads ten 9-bit RPCs");
        assert_eq!(
            result.ambiguous_with,
            Some(3),
            "fc=3 parses the same bits into nine RPCs and must be reported",
        );
    }

    /// The one-filled fixtures are unambiguous, so the flag must stay clear on
    /// them -- otherwise it would fire on every payload and mean nothing.
    #[test]
    fn an_unambiguous_payload_reports_no_competitor() {
        let (data, bit_count) = build_one_rpc_stream(34, 100);
        let result = brute_force_function_count(&data, bit_count).unwrap();
        assert_eq!(result.function_count, 34);
        assert_eq!(result.ambiguous_with, None, "{result:?}");
    }
}
