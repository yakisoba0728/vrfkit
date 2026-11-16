//! Synthetic movement through the real public Reader and partial-bunch path.

mod common;

use std::convert::Infallible;

use common::*;
use vrf_transform::helpers::{advance_state, initial_prng_b, swap_adjacent_bits_u64};
use vrf_transform::versions::V12_10;
use vrf_transform::{SeededTransform, TransformVersion, seed_for};
use vrfkit::{ReplayEvent, ReplayReader, ValidationVerdict};

const ACTOR: u32 = 3;
const CHARACTER: u32 = 53;
const RPC: &str = "ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous";
const CLASS: &str = "/Script/Test.ReaderMovement";

fn movement_rpc() -> BitWriter {
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
    // and the two empty component headers. No replay bytes are embedded.
    let input_end = data.len() + 16 + 8;
    data.u16(input_end as u16).u8(0xA3).u8(0xA4);
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
fn content_block() -> BitWriter {
    let rpc = movement_rpc();
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

fn replay() -> Vec<u8> {
    let block = content_block();
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
fn reader_delivers_paired_movement_provenance_after_real_partial_reassembly() {
    let data = replay();
    let mut observed = Vec::new();
    let mut movement_batches = 0;
    let summary = ReplayReader::new()
        .read(&data, |event| {
            if let ReplayEvent::Records(batch) = event {
                assert_eq!(batch.movement.len(), batch.movement_provenance.len());
                if !batch.movement.is_empty() {
                    movement_batches += 1;
                }
                for (movement, provenance) in batch.movement.iter().zip(batch.movement_provenance) {
                    observed.push((*movement, *provenance));
                }
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(movement_batches, 2);
    assert_eq!(summary.movement_rows, 4);
    assert_eq!(observed.len(), 4);
    assert_eq!(summary.packets, 7);
    assert_eq!(summary.network.partial_completed, 1);
    assert_eq!(summary.network.partial_errors, 0);
    assert_eq!(summary.network.lost_content_blocks(), 0);
    assert_eq!(summary.decode_counters["movement_rpc_errors"], 0);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
    for (index, (movement, provenance)) in observed.iter().enumerate() {
        let move_index = (index % 2) as u32;
        let completion = if index < 2 { 5 } else { 6 };
        let initial = if index < 2 { 0 } else { 6 };
        let source = provenance.source.expect("real framing supplies a source");
        assert_eq!(movement.packet_id, completion);
        assert_eq!(movement.character_net_guid, CHARACTER);
        assert_eq!(movement.timestamp, 500 + move_index);
        assert_eq!(movement.pos_x, 1.0 + move_index as f32);
        assert_eq!((movement.pos_y, movement.pos_z), (-2.0, 3.0));
        assert_eq!(
            (movement.vel_x, movement.vel_y, movement.vel_z),
            (4.0, 5.0, 6.0)
        );
        assert_eq!((movement.yaw, movement.pitch), (90.0, 45.0));
        assert_eq!(movement.movement_state, 3);
        assert_eq!(source.initial_packet_id, initial);
        assert_eq!(source.completion_packet_id, completion);
        assert_eq!(source.actor_net_guid, ACTOR);
        assert_eq!(source.object_net_guid, None);
        assert_eq!(source.channel_index, 2);
        assert_eq!(source.bunch_index, if index < 2 { 2 } else { 3 });
        assert_eq!(source.block_index, 0);
        assert_eq!(source.rpc_index, 0);
        assert_eq!(source.rpc_handle, 0);
        assert_eq!(provenance.array_ordinal, 0);
        assert_eq!(provenance.update_ordinal, 0);
        assert_eq!(provenance.update_index, 4);
        assert_eq!(provenance.component_ordinal, 0);
        assert_eq!(provenance.move_index, move_index);
    }
}
