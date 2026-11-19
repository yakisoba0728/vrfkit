//! Independent malformed-GUID preservation contract.
//! Fixture framing reused from baseline movement_reader.rs; assertions are new.

mod common;

use std::convert::Infallible;

use common::*;
use vrf_transform::helpers::{advance_state, initial_prng_b, swap_adjacent_bits_u64};
use vrf_transform::versions::V12_10;
use vrf_transform::{SeededTransform, TransformVersion, seed_for};
use vrfkit::{CheckpointEvent, ReadOptions, ReplayEvent, ReplayReader, ValidationVerdict};

const ACTOR: u32 = 3;
const CHARACTER: u32 = 53;
const RPC: &str = "ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous";
const CLASS: &str = "/Script/Test.ReaderMovement";

fn movement_rpc(malformed_width: u32) -> BitWriter {
    let input_body: &[bool] = &[];
    let mut data = BitWriter::new();
    data.u8(0x52);
    for index in 0..2 {
        data.bits(index + 1, 3)
            .bit(true)
            .u8(0)
            .u8(3)
            .u8(0)
            .u16(0)
            .u16(0)
            .u16(0)
            .int_packed(500 + index as u32)
            .serialized_int(0, 128)
            .f32(1.0 + index as f32)
            .f32(-2.0)
            .f32(3.0)
            .bit(false)
            .bit(false)
            .u16(8_192)
            .u16(16_384)
            .bit(false)
            .serialized_int(0, 128)
            .f32(4.0)
            .f32(5.0)
            .f32(6.0)
            .bit(false);
    }
    data.bits(0, 3);
    // Native component envelope: movement, the absolute input-section end,
    // and the component headers with an optional unresolved input body.
    // No replay bytes are embedded.
    let input_end = data.len() + 16 + 8 + input_body.len();
    data.u16(input_end as u16)
        .u8(0xA3)
        .extend_bits(input_body)
        .u8(0xA4);
    let unused = (8 - data.len() % 8) % 8;
    data.repeat(false, unused);
    let mut component = BitWriter::new();
    component
        .i32((data.len() / 8) as i32)
        .extend_bits(&data)
        .u8(unused as u8);
    let mut update = BitWriter::new();
    update
        .int_packed(3)
        .int_packed(32)
        .u32(CHARACTER)
        .int_packed(4)
        .int_packed(component.bit_len())
        .extend_bits(&component)
        .int_packed(3)
        .int_packed(malformed_width)
        .repeat(false, malformed_width as usize)
        .int_packed(4)
        .int_packed(component.bit_len())
        .extend_bits(&component)
        .int_packed(3)
        .int_packed(32)
        .u32(CHARACTER + 1)
        .int_packed(4)
        .int_packed(component.bit_len())
        .extend_bits(&component)
        .int_packed(0);
    let mut array = BitWriter::new();
    array
        .int_packed(8)
        .int_packed(5)
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

/// Encode only aligned words of the declared 12.10 transform. The generated
/// block is round-tripped before framing, isolating Reader failures from a
/// malformed test encoder. Transform correctness has separate golden tests.
fn content_block(malformed_width: u32) -> BitWriter {
    let rpc = movement_rpc(malformed_width);
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

fn replay(malformed_width: u32) -> Vec<u8> {
    let block = content_block(malformed_width);
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


#[test]
fn qa_malformed_guid_never_reuses_identity_and_keeps_raw_main_and_checkpoints() {
    for width in [0, 1, 31, 33, 64] {
        let rpc = movement_rpc(width);
        let raw = pack(&rpc);
        let mut data = replay(width);
        let preamble = vrf_container::parse_preamble(&data).unwrap();
        let mut chunks = vrf_container::ChunkIterator::new(&data, preamble.remaining_offset);
        let chunk_data = chunks.next_chunk().unwrap().unwrap();
        let payload = &data[chunk_data.data_offset..][..chunk_data.size_in_bytes as usize];
        let (frames, unread) =
            vrf_container::decompress_replay_data_with_trailing(payload, false, false).unwrap();
        assert_eq!(unread, 0);
        for index in 0..2 {
            data.extend(chunk(2, &checkpoint(index, &checkpoint_tables(&frames), 0)));
        }
        let mut moves = [Vec::new(), Vec::new(), Vec::new()];
        let mut parents = [Vec::new(), Vec::new(), Vec::new()];
        let mut snapshots = Vec::new();
        let summary = ReplayReader.read_with_options(
            &data,
            ReadOptions { checkpoints: true, ..Default::default() },
            |event| {
                let (scope, records) = match event {
                    ReplayEvent::Records(records) => (0, records),
                    ReplayEvent::Checkpoint(CheckpointEvent::Records { checkpoint, records, .. }) =>
                        (checkpoint.identity.checkpoint_index as usize + 1, records),
                    ReplayEvent::Checkpoint(CheckpointEvent::Summary(summary)) => {
                        snapshots.push(summary.clone());
                        return Ok::<(), Infallible>(());
                    }
                    _ => return Ok::<(), Infallible>(()),
                };
                for (movement, origin) in records.movement.iter().zip(records.movement_provenance) {
                    moves[scope].push((movement.character_net_guid, origin.component_ordinal,
                        origin.move_index, origin.source.unwrap().initial_packet_id,
                        origin.source.unwrap().completion_packet_id));
                }
                parents[scope].extend(records.fields.iter().filter(|field|
                    field.field_name.as_deref() == Some(RPC) && field.bit_count > 0).cloned());
                Ok::<(), Infallible>(())
            },
        ).unwrap();
        assert_eq!(summary.movement_rows, 8, "malformed width {width}");
        assert_eq!(summary.checkpoints.movement_rows, 16);
        assert_eq!(snapshots.len(), 2);
        for scope in 0..3 {
            assert_eq!(moves[scope].len(), 8);
            assert_eq!(parents[scope].len(), 2);
            for (index, &(guid, component, move_index, initial, completion)) in moves[scope].iter().enumerate() {
                assert_eq!(guid, CHARACTER + u32::from(index % 4 >= 2));
                assert_eq!(component, if index % 4 < 2 { 0 } else { 2 });
                assert_eq!(move_index, (index % 2) as u32);
                assert_eq!((initial, completion), if index < 4 { (0, 5) } else { (6, 6) });
            }
            for parent in &parents[scope] {
                assert_eq!(parent.bit_count, rpc.bit_len());
                assert_eq!(parent.raw_bits.as_deref(), Some(raw.as_slice()));
            }
        }
        for counters in std::iter::once(&summary.decode_counters).chain(snapshots.iter().map(|s| &s.decode_counters)) {
            assert_eq!(counters["movement_rpc_errors"], 4);
            assert_eq!(counters["movement_component_streams"], 4);
            assert_eq!(counters["movement_component_complete"], 4);
            assert_eq!(counters["movement_component_errors"], 0);
            assert_eq!(counters["movement_component_empty"], 0);
            assert_eq!(counters["movement_component_envelopes"], 4);
            assert!(counters["movement_component_payload_bytes"] > 0);
            assert!(counters.contains_key("movement_component_unused_bits"));
            assert_eq!(counters["movement_component_input_headers"], 4);
            assert_eq!(counters["movement_component_discrepancy_headers"], 4);
        }
        assert_eq!(summary.network.lost_content_blocks(), 0);
        assert_eq!(summary.network.partial_errors, 0);
        assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
    }
}
