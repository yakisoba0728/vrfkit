//! Independent Phase A Reader acceptance probe. Copy into crates/vrfkit/tests
//! as phase_a_native_qa.rs and run the named integration test, then remove it.
//! Transport/transform scaffolding is adapted from movement_reader.rs; the
//! native cases and assertions below were authored independently of the fix.
//! These fixtures contain no replay bytes or production parsing constants.

mod common;
use common::*;
use std::convert::Infallible;
use vrf_transform::helpers::{advance_state, initial_prng_b, swap_adjacent_bits_u64};
use vrf_transform::versions::V12_10;
use vrf_transform::{SeededTransform, TransformVersion, seed_for};
use vrfkit::{ReplayEvent, ReplayReader, ValidationVerdict};
const ACTOR: u32 = 3;
const CHARACTER: u32 = 53;
const RPC: &str = "ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous";
const CLASS: &str = "/Script/Test.ReaderMovement";

// One independently generated variant-0 move. It has no velocity payload.
fn component(timestamp: u32, a3_bits: usize, a4_bits: Option<usize>) -> BitWriter {
    let mut logical = BitWriter::new();
    logical.u8(0x52).bits(1, 3);
    logical.bit(false).u8(17).u8(9).u8(99);
    logical.u16(1).u16(2).u16(3).int_packed(timestamp);
    logical.serialized_int(0, 128).f32(-4.5).f32(7.25).f32(0.0);
    logical.bit(true).u8(42); // optional byte present
    logical.bit(true).u16(8192).u16(32768);
    logical.bit(false).u16(500).u16(1000); // variant0; no external ref
    logical.bit(false).bits(0, 3); // no error sentinel; movement terminator
    let end = logical.len() + 16 + 8 + a3_bits;
    logical.u16(end as u16).u8(0xA3);
    for i in 0..a3_bits {
        logical.bit(i % 2 == 0);
    }
    if let Some(n) = a4_bits {
        logical.u8(0xA4);
        for i in 0..n {
            logical.bit(i % 3 != 0);
        }
    }
    let unused = (8 - logical.len() % 8) % 8;
    logical.repeat(true, unused); // nonzero unused high bits are legal
    let mut out = BitWriter::new();
    out.i32((logical.len() / 8) as i32)
        .extend_bits(&logical)
        .u8(unused as u8);
    out
}

fn rpc_for(components: &[BitWriter]) -> BitWriter {
    let mut update = BitWriter::new();
    update.int_packed(3).int_packed(32).u32(CHARACTER);
    for c in components {
        update.int_packed(4).int_packed(c.bit_len()).extend_bits(c);
    }
    update.int_packed(0);
    let mut array = BitWriter::new();
    array
        .int_packed(1)
        .int_packed(1)
        .extend_bits(&update)
        .int_packed(0);
    let mut rpc = BitWriter::new();
    rpc.bit(false)
        .int_packed(2)
        .int_packed(array.bit_len())
        .extend_bits(&array)
        .int_packed(0);
    rpc
}

fn inspect(rpc: &BitWriter, timestamps: &[u32], failures_per_rpc: u64) {
    let bytes = replay(rpc);
    let mut seen = Vec::new();
    let mut raw = Vec::new();
    let summary = ReplayReader::new()
        .read(&bytes, |event| {
            if let ReplayEvent::Records(batch) = event {
                assert_eq!(batch.movement.len(), batch.movement_provenance.len());
                for (m, p) in batch.movement.iter().zip(batch.movement_provenance) {
                    assert_eq!(m.character_net_guid, CHARACTER);
                    assert_eq!(m.move_type, 0);
                    assert_eq!((m.pos_x, m.pos_y, m.pos_z), (-4.5, 7.25, 0.0));
                    assert_eq!((m.vel_x, m.vel_y, m.vel_z), (0.0, 0.0, 0.0));
                    assert_eq!((m.yaw, m.pitch), (180.0, 45.0));
                    assert_eq!(m.movement_state, 9);
                    assert_eq!(p.move_index, 0);
                    let source = p.source.unwrap();
                    assert_eq!(
                        source.initial_packet_id,
                        if m.packet_id == 5 { 0 } else { 6 }
                    );
                    assert_eq!(source.completion_packet_id, m.packet_id);
                    seen.push((m.timestamp, p.component_ordinal));
                }
                for field in batch.fields {
                    if field.field_name.as_deref() == Some(RPC) && field.bit_count > 0 {
                        raw.push((field.bit_count, field.raw_bits.clone()));
                    }
                }
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(seen.iter().map(|x| x.0).collect::<Vec<_>>(), timestamps);
    assert_eq!(summary.movement_rows, timestamps.len() as u64);
    assert_eq!(raw.len(), 2, "one fragmented and one ordinary RPC");
    for (bits, value) in raw {
        assert_eq!(bits, rpc.bit_len());
        if failures_per_rpc == 0 {
            assert!(
                value.is_none(),
                "supported movement intentionally has no duplicate raw RPC"
            );
        } else {
            assert_eq!(
                value.as_deref(),
                Some(pack(rpc).as_slice()),
                "unsupported bytes must stay in the full RPC"
            );
        }
    }
    assert_eq!(
        summary.decode_counters["movement_rpc_errors"],
        failures_per_rpc * 2
    );
    assert_eq!(
        summary.decode_counters["movement_component_errors"],
        failures_per_rpc * 2
    );
    assert_eq!(summary.network.lost_content_blocks(), 0);
    assert_eq!(summary.network.partial_errors, 0);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
    for chunk in seen.chunks(timestamps.len() / 2) {
        for (timestamp, ordinal) in chunk {
            assert_eq!(*ordinal, if *timestamp == 111 { 0 } else { 1 });
        }
    }
}

#[test]
fn independent_native_body_residues_preserve_full_rpc_and_next_component() {
    for bits in 1..=8 {
        for a4 in [None, Some(0)] {
            inspect(
                &rpc_for(&[component(111, bits, a4), component(222, 0, None)]),
                &[111, 222, 111, 222],
                1,
            );
        }
        inspect(
            &rpc_for(&[component(111, 0, Some(bits)), component(222, 0, Some(0))]),
            &[111, 222, 111, 222],
            1,
        );
    }
}

#[test]
fn independent_valid_optional_a4_and_unused_bits_omit_duplicate_raw() {
    for a4 in [None, Some(0)] {
        inspect(
            &rpc_for(&[component(111, 0, a4), component(222, 0, a4)]),
            &[111, 222, 111, 222],
            0,
        );
    }
}

#[test]
fn independent_malformed_outer_envelopes_preserve_raw_and_recover_next_component() {
    let good = component(111, 0, Some(0));
    let mut cases = Vec::new();
    let mut negative = good.clone();
    negative[..32].fill(true);
    cases.push(negative);
    let mut oversized = good.clone();
    oversized[..32].copy_from_slice(&BitWriter::new().i32(i32::MAX)[..]);
    cases.push(oversized);
    let mut extra = good.clone();
    extra.u8(0xBD);
    cases.push(extra);
    let mut missing = good.clone();
    missing.truncate(missing.len() - 8);
    cases.push(missing);
    let mut bad_unused = good.clone();
    let n = bad_unused.len();
    bad_unused[n - 8..].copy_from_slice(&BitWriter::new().u8(8)[..]);
    cases.push(bad_unused);
    for bad in cases {
        inspect(&rpc_for(&[bad, component(222, 0, None)]), &[222, 222], 1);
    }
}

/// Encode only aligned words of the declared 12.10 transform. The generated
/// block is round-tripped before framing, isolating Reader failures from a
/// malformed test encoder. Transform correctness has separate golden tests.
fn content_block(rpc: &BitWriter) -> BitWriter {
    let mut plain = BitWriter::new();
    plain
        .serialized_int(0, 2)
        .int_packed(rpc.bit_len())
        .extend_bits(&rpc);
    // Empty RPCs are legal and emit no movement. Nine bits per record can
    // reach any residue modulo64, so the inverse only needs one word width.
    while plain.len() % 64 != 0 {
        plain.serialized_int(0, 2).int_packed(0);
    }
    let bit_count = plain.len();
    let mut encoded = pack(&plain);
    let seed = seed_for(bit_count, ACTOR);
    let mut state = seed;
    let mut prng_a = V12_10::initial_prng_a(seed);
    let mut prng_b = initial_prng_b(seed);
    for bytes in encoded.chunks_exact_mut(8) {
        let wanted = u64::from_le_bytes(bytes.try_into().unwrap());
        let mut value = swap_adjacent_bits_u64(wanted);
        value ^= !u64::from(state.rotate_right(4));
        value = value.rotate_left(state.rotate_right(5) % 63 + 1);
        value = value.wrapping_add(u64::from(state.rotate_right(6)));
        value = swap_adjacent_bits_u64(value);
        value = value.rotate_left(state.rotate_right(8) % 63 + 1);
        assert_eq!(V12_10::word64(value, state), wanted);
        bytes.copy_from_slice(&value.to_le_bytes());
        advance_state(&mut state, &mut prng_a, &mut prng_b);
    }
    let mut decoded = encoded.clone();
    TransformVersion::V1210
        .apply(&mut decoded, bit_count, seed)
        .unwrap();
    assert_eq!(decoded, pack(&plain));
    let mut block = BitWriter::new();
    block
        .extend_bits(&[false, true]) // ClassNetCache, actor block
        .int_packed(bit_count as u32)
        .bytes(&encoded);
    block
}

fn packet(payload: &[bool], open: bool, partial: Option<(bool, bool)>) -> Vec<u8> {
    let mut bits = BitWriter::new();
    bits.bit(open); // control present only for the channel open
    if open {
        bits.bit(true).bit(false); // open, close
    }
    bits.bit(false)
        .bit(true) // paused, reliable
        .int_packed(2)
        .bit(false)
        .bit(false)
        .bit(partial.is_some())
        .bit(false); // exports, mapped GUIDs, partial, VALORANT bit
    if let Some((initial, final_fragment)) = partial {
        bits.bit(initial).bit(final_fragment);
    }
    bits.bit(true)
        .int_packed(1) // reliable channel FName
        .serialized_int(payload.len() as u32, 16_384)
        .extend_bits(payload)
        .bit(true); // packet sentinel
    pack(&bits)
}

fn replay(rpc: &BitWriter) -> Vec<u8> {
    let block = content_block(rpc);
    let mut opened = BitWriter::new();
    opened.int_packed(ACTOR).extend_bits(&block);
    // Empty packets create non-adjacent fragment IDs without changing the
    // reliable sequence on channel2. An ordinary packet then checks reset.
    let packets = [
        packet(&opened[..8], true, Some((true, false))),
        vec![1],
        packet(&opened[8..16], false, Some((false, false))),
        vec![1],
        vec![1],
        packet(&opened[16..], false, Some((false, true))),
        packet(&block, false, None),
    ];
    let refs: Vec<_> = packets.iter().map(Vec::as_slice).collect();
    let mut frames = frame(1.25, &[], 0, &refs);
    let mut exports = BitWriter::new();
    exports
        .int_packed(1)
        .int_packed(7)
        .int_packed(1)
        .fstring(&format!("{CLASS}_ClassNetCache"))
        .int_packed(1)
        .u8(1)
        .int_packed(0)
        .u32(0)
        .u8(0)
        .fstring(RPC)
        .i32(0);
    let mut guid = BitWriter::new();
    guid.int_packed(ACTOR).u8(1).int_packed(0).fstring(CLASS);
    exports
        .int_packed(1)
        .i32((guid.len() / 8) as i32)
        .extend_bits(&guid);
    frames.splice(8..10, pack(&exports));
    let mut result = replay_info(&Info::default());
    result.extend(chunk(0, &header_payload()));
    result.extend(chunk(1, &replay_data(&frames, frames.len())));
    result
}
