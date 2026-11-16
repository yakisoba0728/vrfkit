//! Decoder tests: build a whole RPC payload bit by bit with vrf-testkit's
//! writer, not `vrf-bitio`, so a bug mirrored in both cannot cancel out.
use vrf_bitio::BitReader;
use vrf_testkit::{BitWrite, BitWriter, pack};

use crate::moves::MOVEMENT_MAGIC;
use crate::rpc::{
    COMPONENT_DATA_STREAM_HANDLE, REMOTE_CHARACTER_UPDATES_HANDLE,
    SHOOTER_CHARACTER_NET_GUID_HANDLE, decode_movement_rpc,
};
use crate::types::{MovementContext, MovementMove, RpcDecodeResult};

fn decode_with_context(rpc: &[bool]) -> (RpcDecodeResult, Vec<(MovementMove, MovementContext)>) {
    let bytes = pack(rpc);
    let mut reader = BitReader::with_bit_len(&bytes, rpc.len() as u64).unwrap();
    let mut moves = Vec::new();
    let result = crate::decode_movement_rpc_with_context(&mut reader, |movement, context| {
        moves.push((movement, context));
    })
    .unwrap();
    (result, moves)
}

fn movement_values(movement: MovementMove) -> (u32, [u64; 8], u32, [u8; 3]) {
    (
        movement.shooter_character_net_guid,
        [
            movement.pos_x,
            movement.pos_y,
            movement.pos_z,
            movement.yaw,
            movement.pitch,
            movement.vel_x,
            movement.vel_y,
            movement.vel_z,
        ]
        .map(f64::to_bits),
        movement.timestamp,
        [
            movement.movement_state,
            movement.mode_flags,
            movement.move_type,
        ],
    )
}

/// One move record: movementState 3, position `(x, y, z)` as 3 x f32, and for
/// variant 1 the velocity (4, 5, 6) at scale 10.
fn build_move(variant1: bool, timestamp: u32, x: f32, y: f32, z: f32) -> BitWriter {
    let mut w = BitWriter::new();
    // Header (moveType, rotationYawMultiplier, movementState, unusedByte),
    // rotationInput, timestamp, then position info 0: 3 x f32.
    w.bit(variant1).u8(2).u8(3).u8(0);
    w.u16(0x8000).u16(0x8000).u16(0x8000).int_packed(timestamp);
    w.serialized_int(0, 128).f32(x).f32(y).f32(z);
    // hasOptionalByte, flag48, packedAngles.
    w.bit(false).bit(false).u32(0);
    if variant1 {
        // variant1Flag, then info componentBits 10 | extraInfo 1 and 40, 50, 60.
        w.bit(true).serialized_int(10 | (1 << 6), 128);
        w.bits(40, 10).bits(50, 10).bits(60, 10);
    } else {
        // hasExternalCharacterRef 0, then 32 bits of angles.
        w.bit(false).u32(0);
    }
    w.bit(false); // errorSentinel
    w
}

/// Bits after a real envelope; the decoder never reads them.
const ENVELOPE_TRAILER_BITS: u32 = 24;

/// A movement section: the magic, `moves` under markers 1 to 7 (then 1
/// again), a `000` terminator, then `tail`.
fn section(moves: &[BitWriter], tail: &[bool]) -> BitWriter {
    let mut section = BitWriter::new();
    section.u8(MOVEMENT_MAGIC);
    for (i, mv) in moves.iter().enumerate() {
        section.bits(i as u64 % 7 + 1, 3).extend_bits(mv);
    }
    section.bits(0, 3).extend_bits(tail);
    section
}

/// A direct component stream: `movementBitCount` is the section's length when
/// `sized`, else 0 (the window runs to the end).
fn component_stream(section: &[bool], sized: bool) -> BitWriter {
    let mut stream = BitWriter::new();
    stream.u16(if sized { section.len() as u16 } else { 0 });
    stream.extend_bits(section);
    stream
}

/// `payload`, whole bytes, in the envelope: a u16 byte count, the payload,
/// then `trailer_bits` non-zero bits outside it.
fn byte_wrapped(payload: &[bool], trailer_bits: u32) -> BitWriter {
    assert_eq!(payload.len() % 8, 0, "an envelope holds whole bytes");
    let mut stream = BitWriter::new();
    stream.u16((payload.len() / 8) as u16).extend_bits(payload);
    stream.extend((0..trailer_bits).map(|i| i % 3 == 0));
    stream
}

/// The shape real replays use (crate docs, "Measured on real replays"):
/// byte-wrapped, an open window, and after the last move the `000` terminator
/// plus 8 to 15 non-zero bits to the byte boundary, within the 31 left unread.
fn real_stream(moves: &[BitWriter], trailer_bits: u32) -> BitWriter {
    let mut payload = component_stream(&section(moves, &[]), false);
    let pad = 8 + (8 - payload.len() % 8) % 8;
    payload.extend((0..pad).map(|i| i % 2 == 0));
    byte_wrapped(&payload, trailer_bits)
}

/// The byte-wrapped section `magic 0x00`, which fails to decode.
fn bad_magic_stream() -> BitWriter {
    byte_wrapped(BitWriter::new().u16(0).u8(0x00), ENVELOPE_TRAILER_BITS)
}

/// One update: the shooter GUID field (handle 2, 32 bits), then `stream` as
/// the ComponentDataStream (handle 3).
fn update_with_stream(shooter_guid: u32, stream: &[bool]) -> BitWriter {
    let mut update = BitWriter::new();
    update
        .int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1)
        .int_packed(32)
        .u32(shooter_guid)
        .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
        .int_packed(stream.len() as u32)
        .extend_bits(stream)
        .int_packed(0);
    update
}

/// An updates array declaring `updates.len()` and holding them in order.
fn updates_array(updates: &[BitWriter]) -> BitWriter {
    let mut array = BitWriter::new();
    array.int_packed(updates.len() as u32);
    for (i, update) in updates.iter().enumerate() {
        array.int_packed(i as u32 + 1).extend_bits(update);
    }
    array.int_packed(0);
    array
}

/// A RemoteCharacterUpdates array in the RPC envelope. The framing tests
/// deform the array itself and call this directly.
fn wrap_updates_array(array: &[bool]) -> BitWriter {
    let mut rpc = BitWriter::new();
    rpc.bit(false) // first bit, discarded
        .int_packed(REMOTE_CHARACTER_UPDATES_HANDLE + 1)
        .int_packed(array.len() as u32)
        .extend_bits(array)
        .int_packed(0);
    rpc
}

/// A whole RPC with one update carrying `stream`.
fn build_rpc_payload(shooter_guid: u32, stream: &[bool]) -> BitWriter {
    wrap_updates_array(&updates_array(&[update_with_stream(shooter_guid, stream)]))
}

/// Decode a built payload, returning the result and the moves it emitted.
fn decode(rpc: &[bool]) -> (RpcDecodeResult, Vec<MovementMove>) {
    let bytes = pack(rpc);
    let mut reader = BitReader::with_bit_len(&bytes, rpc.len() as u64).unwrap();
    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();
    (result, moves)
}

/// `(sized tails, sized tail bits, open tails, open tail bits)`.
fn tails(result: &RpcDecodeResult) -> (u32, u64, u32, u64) {
    (
        result.sized_section_tails,
        result.sized_section_tail_bits,
        result.open_section_tails,
        result.open_section_tail_bits,
    )
}

/// `(envelope trailer streams, envelope trailer bits)`.
fn trailers(result: &RpcDecodeResult) -> (u32, u64) {
    (
        result.envelope_trailer_streams,
        result.envelope_trailer_bits,
    )
}

#[test]
fn an_out_of_range_update_index_is_counted_not_discarded_in_silence() {
    // updateCount 1, index 2, then a well-formed update only the index check
    // refuses; uncounted, the lost update looks like none was sent.
    let stream = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)], ENVELOPE_TRAILER_BITS);
    let mut array = BitWriter::new();
    let update = update_with_stream(1111, &stream);
    array.int_packed(1).int_packed(3).extend_bits(&update);

    let (result, moves) = decode(&wrap_updates_array(&array));

    assert_eq!(result.update_count, 1);
    assert!(moves.is_empty());
    assert_eq!(result.error_count, 1, "the discarded array tail");
}

#[test]
fn a_field_longer_than_the_update_window_is_counted() {
    // The framing no longer describes the payload, and the second declared
    // update goes with the window.
    let mut array = BitWriter::new();
    // updateCount 2, index 0, then handle 5 declaring 128 bits with 8 left.
    array
        .int_packed(2)
        .int_packed(1)
        .int_packed(6)
        .int_packed(128)
        .u8(0);

    let (result, moves) = decode(&wrap_updates_array(&array));

    assert_eq!(result.update_count, 2);
    assert!(moves.is_empty());
    assert_eq!(result.error_count, 1, "an over-long field declaration");
}

#[test]
fn a_shooter_guid_field_not_32_bits_wide_and_its_orphaned_stream_are_counted() {
    // Too narrow for its u32, or wider (a u32 read would keep the low 32
    // bits): no GUID, so the stream after it is consumed undecoded.
    let stream = real_stream(
        &[build_move(false, 7, 1.0, 2.0, 3.0)],
        ENVELOPE_TRAILER_BITS,
    );
    for width in [31, 33] {
        let mut update = BitWriter::new();
        update
            .int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1)
            .int_packed(width)
            .bits(0, width)
            .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
            .int_packed(stream.bit_len())
            .extend_bits(&stream)
            .int_packed(0);

        let (result, moves) = decode(&wrap_updates_array(&updates_array(&[update])));

        assert!(moves.is_empty(), "{width} bits");
        assert_eq!(result.error_count, 2, "{width} bits: the GUID, the stream");
    }
}

#[test]
fn a_component_stream_ahead_of_its_guid_is_counted() {
    // Handle 3 before handle 2: a single pass cannot rewind to the GUID.
    let stream = real_stream(
        &[build_move(false, 7, 1.0, 2.0, 3.0)],
        ENVELOPE_TRAILER_BITS,
    );
    let mut update = BitWriter::new();
    update
        .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
        .int_packed(stream.bit_len())
        .extend_bits(&stream)
        .int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1)
        .int_packed(32)
        .u32(4321)
        .int_packed(0);

    let (result, moves) = decode(&wrap_updates_array(&updates_array(&[update])));

    assert!(moves.is_empty());
    assert_eq!(result.error_count, 1, "the orphaned stream");
}

#[test]
fn a_stream_that_fails_to_decode_does_not_drop_the_updates_after_it() {
    // The decoder is past the whole length-delimited stream before reading
    // it, so a failure inside costs only that stream.
    let short_header = BitWriter::new().u8(0x52).clone(); // under 16 bits
    let good = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)], ENVELOPE_TRAILER_BITS);
    for (name, bad) in [
        ("bad magic", bad_magic_stream()),
        ("short header", short_header),
    ] {
        let array = updates_array(&[
            update_with_stream(1111, &bad),
            update_with_stream(2222, &good),
        ]);

        let (result, moves) = decode(&wrap_updates_array(&array));

        assert_eq!(result.update_count, 2, "{name}");
        assert_eq!(result.error_count, 1, "{name}: the failed stream, once");
        assert_eq!(result.total_moves, 1, "{name}: the next update's move");
        assert_eq!(moves.len(), 1, "{name}");
        assert_eq!(moves[0].shooter_character_net_guid, 2222, "{name}");
    }
}

#[test]
fn bits_after_the_updates_array_terminator_are_counted_unless_one_int_packed() {
    // Behind the zero index only 8 bits that parse as an IntPacked are
    // expected; anything else is lost updates or a drifted cursor.
    let stream = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)], ENVELOPE_TRAILER_BITS);
    let mut update = BitWriter::new();
    update
        .int_packed(1)
        .extend_bits(&update_with_stream(1111, &stream));
    let byte = |b| BitWriter::new().u8(b).clone();
    for (name, declared, after, errors) in [
        ("an IntPacked", 0, byte(0x02), 0),
        ("a byte asking for another", 0, byte(0x01), 1),
        ("16 bits", 0, BitWriter::new().u16(0xBEEF).clone(), 1),
        ("a real update", 1, update, 1),
    ] {
        let mut array = BitWriter::new();
        array.int_packed(declared).int_packed(0).extend_bits(&after);

        let (result, moves) = decode(&wrap_updates_array(&array));

        assert!(moves.is_empty(), "{name}");
        assert_eq!(result.error_count, errors, "{name}");
    }
}

#[test]
fn decodes_both_move_variants_in_either_component_form() {
    let moves = [
        build_move(false, 42, 1.25, 2.5, 3.75),
        build_move(true, 84, 10.0, 11.0, 12.0),
    ];
    let wrapped = real_stream(&moves, ENVELOPE_TRAILER_BITS);
    let direct = component_stream(&section(&moves, &[]), true);
    for (name, stream, envelope) in [("wrapped", wrapped, (1, 24)), ("direct", direct, (0, 0))] {
        let (result, decoded) = decode(&build_rpc_payload(1234, &stream));

        let counts = (result.total_moves, result.update_count, result.error_count);
        assert_eq!(counts, (2, 1, 0), "{name}");
        assert_eq!(trailers(&result), envelope, "{name}");
        let got: Vec<_> = decoded
            .iter()
            .map(|m| {
                let pos = [m.pos_x, m.pos_y, m.pos_z];
                let vel = [m.vel_x, m.vel_y, m.vel_z];
                let guid = m.shooter_character_net_guid;
                (guid, m.move_type, m.timestamp, m.movement_state, pos, vel)
            })
            .collect();
        let variant0 = (1234, 0, 42, 3, [1.25, 2.5, 3.75], [0.0; 3]);
        let variant1 = (1234, 1, 84, 3, [10.0, 11.0, 12.0], [4.0, 5.0, 6.0]);
        assert_eq!(got, [variant0, variant1], "{name}");
    }
}

#[test]
fn bits_after_the_top_level_zero_handle_are_counted() {
    let mut rpc = wrap_updates_array(&updates_array(&[]));
    rpc.u16(0xBEEF);
    assert_eq!(decode(&rpc).0.error_count, 1);
}

#[test]
fn empty_rpc_returns_zero() {
    let (result, moves) = decode(&BitWriter::new());
    assert_eq!(result.total_moves, 0);
    assert!(moves.is_empty());
}

#[test]
fn section_tails_are_tallied_apart_by_window_and_never_as_errors() {
    // Not errors, which would change which batches keep raw bits. In an open
    // window what follows a zero marker may be other component data, so the
    // two kinds are never summed.
    let mv = || build_move(true, 10, 1.0, 2.0, 3.0);
    let sized = |s: &[bool]| component_stream(s, true);
    let open = |s: &[bool]| component_stream(s, false);
    // A move or just the magic, then `000` and 40 set bits: over 31 bits
    // after a move, so the zero marker is read.
    let (after_move, after_magic) = (section(&[mv()], &[true; 40]), section(&[], &[true; 40]));
    let no_magic = BitWriter::new().bits(0b10110, 5).clone();
    let no_marker = BitWriter::new().u8(MOVEMENT_MAGIC).bits(0b11, 2).clone();
    // Not tails: the real end, within 31 bits of the last move; an empty open
    // window (no magic, but nothing unread); a section ending on its zero
    // marker, after a move or after the magic.
    let (ends, only_000) = (section(&[mv()], &[]), section(&[], &[]));
    let real = real_stream(&[mv(), mv()], ENVELOPE_TRAILER_BITS);
    for (name, stream, moves, expected) in [
        ("sized, move", sized(&after_move), 1, (1, 40, 0, 0)),
        ("sized, magic", sized(&after_magic), 0, (1, 40, 0, 0)),
        ("no magic", sized(&no_magic), 0, (1, 5, 0, 0)),
        ("no marker", sized(&no_marker), 0, (1, 2, 0, 0)),
        ("open, move", open(&after_move), 1, (0, 0, 1, 40)),
        ("real", real, 2, (0, 0, 0, 0)),
        ("empty open window", open(&[]), 0, (0, 0, 0, 0)),
        ("ends on 000", sized(&ends), 1, (0, 0, 0, 0)),
        ("only 000", sized(&only_000), 0, (0, 0, 0, 0)),
    ] {
        let (result, decoded) = decode(&build_rpc_payload(77, &stream));

        assert_eq!(decoded.len(), moves, "{name}");
        assert_eq!(result.error_count, 0, "{name}");
        assert_eq!(tails(&result), expected, "{name}");
    }
}

#[test]
fn every_envelope_and_the_bits_after_it_are_tallied_per_stream() {
    // Summed per stream, not taken from the last one. Counted with nothing
    // after it, so a vanished trailer reads as bits short of 24 per stream,
    // and when the section inside fails.
    let mv = || build_move(false, 9, 4.0, 5.0, 6.0);
    let two = vec![real_stream(&[mv()], 24), real_stream(&[mv()], 13)];
    let bare = vec![real_stream(&[mv()], 0)];
    for (name, streams, moves, errors, expected) in [
        ("24 and 13 bits", two, 2, 0, (2, 37)),
        ("nothing after it", bare, 1, 0, (1, 0)),
        ("a failed section", vec![bad_magic_stream()], 0, 1, (1, 24)),
    ] {
        let updates: Vec<_> = streams.iter().map(|s| update_with_stream(77, s)).collect();

        let (result, decoded) = decode(&wrap_updates_array(&updates_array(&updates)));

        assert_eq!(decoded.len(), moves, "{name}");
        assert_eq!(result.error_count, errors, "{name}");
        assert_eq!(trailers(&result), expected, "{name}");
    }
}

#[test]
fn movement_context_distinguishes_repeated_arrays_and_sparse_update_indices() {
    let stream = real_stream(
        &[build_move(true, 17, 1.0, -2.0, 3.0)],
        ENVELOPE_TRAILER_BITS,
    );
    let update = update_with_stream(53, &stream);
    let mut array = BitWriter::new();
    array
        .int_packed(8)
        .int_packed(5)
        .extend_bits(&update)
        .int_packed(5)
        .extend_bits(&update)
        .int_packed(2)
        .extend_bits(&update)
        .int_packed(0);
    let mut rpc = BitWriter::new();
    rpc.bit(false);
    for _ in 0..2 {
        rpc.int_packed(REMOTE_CHARACTER_UPDATES_HANDLE + 1)
            .int_packed(array.bit_len())
            .extend_bits(&array);
    }
    rpc.int_packed(0);

    let (result, moves) = decode_with_context(&rpc);
    let actual: Vec<_> = moves.iter().map(|(_, context)| *context).collect();
    let expected: Vec<_> = (0..2)
        .flat_map(|array_ordinal| {
            [4, 4, 1]
                .into_iter()
                .enumerate()
                .map(move |(ordinal, index)| MovementContext {
                    array_ordinal,
                    update_ordinal: ordinal as u32,
                    update_index: index,
                    component_ordinal: 0,
                    move_index: 0,
                })
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(result.total_moves, 6);
    assert_eq!(result.error_count, 0);

    let (legacy_result, legacy_moves) = decode(&rpc);
    assert_eq!(format!("{result:?}"), format!("{legacy_result:?}"));
    assert_eq!(
        moves
            .into_iter()
            .map(|(movement, _)| movement_values(movement))
            .collect::<Vec<_>>(),
        legacy_moves
            .into_iter()
            .map(movement_values)
            .collect::<Vec<_>>()
    );
}

#[test]
fn movement_context_counts_empty_failed_and_orphaned_components() {
    let good = real_stream(
        &[build_move(true, 17, 1.0, 2.0, 3.0)],
        ENVELOPE_TRAILER_BITS,
    );
    let empty = real_stream(&[], ENVELOPE_TRAILER_BITS);
    let bad = bad_magic_stream();
    let mut update = BitWriter::new();
    update
        .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
        .int_packed(good.bit_len())
        .extend_bits(&good);
    update
        .int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1)
        .int_packed(32)
        .u32(53);
    for stream in [&bad, &empty, &good, &good] {
        update
            .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
            .int_packed(stream.bit_len())
            .extend_bits(stream);
    }
    update.int_packed(0);
    let (result, moves) = decode_with_context(&wrap_updates_array(&updates_array(&[update])));
    assert_eq!(
        result.error_count, 2,
        "orphaned and malformed components remain counted"
    );
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].1.component_ordinal, 3);
    assert_eq!(moves[1].1.component_ordinal, 4);
    assert_eq!(moves[0].1.move_index, 0);
    assert_eq!(moves[1].1.move_index, 0);
}

#[test]
fn movement_context_move_index_does_not_wrap_with_markers() {
    let samples: Vec<_> = (0..20)
        .map(|index| build_move(true, index, 1.0, 2.0, 3.0))
        .collect();
    let stream = real_stream(&samples, ENVELOPE_TRAILER_BITS);
    let (result, moves) = decode_with_context(&build_rpc_payload(53, &stream));
    assert_eq!(result.total_moves, 20);
    assert_eq!(result.error_count, 0);
    for (expected, (movement, context)) in moves.iter().enumerate() {
        assert_eq!(context.move_index, expected as u32);
        assert_eq!(movement.timestamp, expected as u32);
    }
}

#[test]
fn movement_context_retains_a_decoded_prefix_before_a_component_error() {
    let good_move = build_move(true, 17, 1.0, 2.0, 3.0);
    let mut bad_move = build_move(true, 18, 4.0, 5.0, 6.0);
    *bad_move.last_mut().unwrap() = true; // errorSentinel
    let failed = real_stream(&[good_move.clone(), bad_move], ENVELOPE_TRAILER_BITS);
    let good = real_stream(&[good_move], ENVELOPE_TRAILER_BITS);
    let mut update = BitWriter::new();
    update
        .int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1)
        .int_packed(32)
        .u32(53);
    for stream in [&failed, &good] {
        update
            .int_packed(COMPONENT_DATA_STREAM_HANDLE + 1)
            .int_packed(stream.bit_len())
            .extend_bits(stream);
    }
    update.int_packed(0);
    let (result, moves) = decode_with_context(&wrap_updates_array(&updates_array(&[update])));
    assert_eq!(result.error_count, 1);
    assert_eq!(result.total_moves, 2);
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].1.component_ordinal, 0);
    assert_eq!(moves[1].1.component_ordinal, 1);
    assert!(moves.iter().all(|(_, context)| context.move_index == 0));
}
