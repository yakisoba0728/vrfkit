//! Synthetic movement through the real public Reader and partial-bunch path.

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

fn movement_rpc(input_body: &[bool]) -> BitWriter {
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
fn content_block(input_body: &[bool]) -> BitWriter {
    let rpc = movement_rpc(input_body);
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

fn replay(input_body: &[bool]) -> Vec<u8> {
    let block = content_block(input_body);
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
    let data = replay(&[]);
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

#[test]
fn reader_preserves_the_whole_rpc_when_input_component_body_is_unresolved() {
    let input_body = [true];
    let rpc = movement_rpc(&input_body);
    let expected_raw = pack(&rpc);
    let data = replay(&input_body);
    let mut fields = Vec::new();
    let mut movement_rows = 0;
    let summary = ReplayReader::new()
        .read(&data, |event| {
            if let ReplayEvent::Records(batch) = event {
                assert_eq!(batch.movement.len(), batch.movement_provenance.len());
                movement_rows += batch.movement.len();
                fields.extend(
                    batch
                        .fields
                        .iter()
                        .filter(|field| {
                            field.field_name.as_deref() == Some(RPC) && field.bit_count > 0
                        })
                        .cloned(),
                );
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();

    // Both the reassembled RPC and the later ordinary RPC keep their decoded
    // movement prefix plus every original raw bit of the unsupported body.
    assert_eq!(movement_rows, 4);
    assert_eq!(fields.len(), 2);
    for field in fields {
        assert_eq!(field.bit_count, rpc.bit_len());
        assert_eq!(field.raw_bits.as_deref(), Some(expected_raw.as_slice()));
    }
    assert_eq!(summary.movement_rows, 4);
    assert_eq!(summary.decode_counters["movement_rpc_errors"], 2);
    assert_eq!(summary.network.lost_content_blocks(), 0);
    assert_eq!(summary.network.partial_errors, 0);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
}

#[test]
fn checkpoint_movement_keeps_provenance_raw_rpc_and_all_native_counters() {
    let main = replay(&[true]);
    let preamble = vrf_container::parse_preamble(&main).unwrap();
    let mut chunks = vrf_container::ChunkIterator::new(&main, preamble.remaining_offset);
    let data = chunks.next_chunk().unwrap().unwrap();
    let payload = &main[data.data_offset..][..data.size_in_bytes as usize];
    let (frames, unread) =
        vrf_container::decompress_replay_data_with_trailing(payload, false, false).unwrap();
    assert_eq!(unread, 0);
    let mut combined = main.clone();
    for index in 0..2 {
        combined.extend(chunk(2, &checkpoint(index, &checkpoint_tables(&frames), 0)));
    }
    let mut main_moves = Vec::new();
    let mut checkpoint_moves = [Vec::new(), Vec::new()];
    let mut main_fields = Vec::new();
    let mut checkpoint_fields = [Vec::new(), Vec::new()];
    let mut completed = Vec::new();
    let summary = ReplayReader
        .read_with_options(
            &combined,
            ReadOptions {
                checkpoints: true,
                ..ReadOptions::default()
            },
            |event| {
                match event {
                    ReplayEvent::Records(batch) => {
                        main_moves.extend(
                            batch
                                .movement
                                .iter()
                                .zip(batch.movement_provenance)
                                .map(|(a, b)| (*a, *b)),
                        );
                        main_fields.extend_from_slice(batch.fields);
                    }
                    ReplayEvent::Checkpoint(CheckpointEvent::Records {
                        checkpoint,
                        records,
                        ..
                    }) => {
                        let index = checkpoint.identity.checkpoint_index as usize;
                        assert_eq!(records.movement.len(), records.movement_provenance.len());
                        checkpoint_moves[index].extend(
                            records
                                .movement
                                .iter()
                                .zip(records.movement_provenance)
                                .map(|(a, b)| (*a, *b)),
                        );
                        checkpoint_fields[index].extend_from_slice(records.fields);
                    }
                    ReplayEvent::Checkpoint(CheckpointEvent::Summary(summary)) => {
                        completed.push(summary.clone())
                    }
                    _ => {}
                }
                Ok::<(), Infallible>(())
            },
        )
        .unwrap();
    assert_eq!(main_moves.len(), 4);
    // Exact raw rows, so the checkpoint equality below carries preservation
    // into both snapshots instead of comparing two equally lossy passes.
    let rpc = movement_rpc(&[true]);
    let expected_raw = pack(&rpc);
    let raw_rpcs: Vec<_> = main_fields
        .iter()
        .filter(|field| field.field_name.as_deref() == Some(RPC) && field.bit_count > 0)
        .collect();
    assert_eq!(
        raw_rpcs.len(),
        2,
        "unresolved input body must retain raw RPCs"
    );
    for field in raw_rpcs {
        assert_eq!(field.bit_count, rpc.bit_len());
        assert_eq!(field.raw_bits.as_deref(), Some(expected_raw.as_slice()));
    }
    assert_eq!(summary.checkpoints.movement_rows, 8);
    assert_eq!(completed.len(), 2);
    for index in 0..2 {
        assert_eq!(
            format!("{:?}", checkpoint_moves[index]),
            format!("{main_moves:?}")
        );
        assert_eq!(
            format!("{:?}", checkpoint_fields[index]),
            format!("{main_fields:?}")
        );
        assert_eq!(completed[index].movement_rows, 4);
        assert_eq!(completed[index].decode_counters, summary.decode_counters);
        assert_eq!(
            completed[index].network.counters(),
            summary.network.counters()
        );
        assert_eq!(
            completed[index].decode_counters["movement_component_streams"],
            2
        );
    }
}
