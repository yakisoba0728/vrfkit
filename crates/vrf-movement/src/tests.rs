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

#[test]
fn native_unused_bit_count_above_seven_is_not_a_clean_movement_rpc() {
    let mut payload = section(&[build_move(true, 7, 1.0, 2.0, 3.0)], &[]);
    payload.u16((payload.len() + 24) as u16).u8(0xA3).u8(0xA4);
    let unused = (8 - payload.len() % 8) % 8;
    payload.repeat(false, unused);
    let mut stream = BitWriter::new();
    stream
        .i32((payload.len() / 8) as i32)
        .extend_bits(&payload)
        .u8(8);
    let (result, _) = decode(&build_rpc_payload(77, &stream));
    assert!(
        result.error_count > 0,
        "an invalid native bit count must retain the raw RPC"
    );
}

#[test]
fn native_structural_errors_and_unknown_bodies_do_not_hide_or_drop_later_updates() {
    let movement = build_move(true, 7, 1.0, 2.0, 3.0);
    let mut payload = section(&[movement.clone()], &[]);
    let terminator = payload.len() - 3;
    let boundary = payload.len();
    let input_start = boundary + 16;
    let input_end = input_start + 8;
    payload.u16(input_end as u16).u8(0xA3).u8(0xA4);
    let valid = native_envelope(&payload);
    let mut cases = Vec::new();
    let mut negative = valid.clone();
    negative[31] = true;
    cases.push(("negative byte count", negative, 0, false));
    let mut invalid_unused = valid.clone();
    let unused_start = invalid_unused.len() - 8;
    invalid_unused[unused_start..].fill(false);
    invalid_unused[unused_start + 3] = true;
    cases.push(("unused count eight", invalid_unused, 0, false));
    let mut extra = valid.clone();
    extra.u8(0);
    cases.push(("extra outer byte", extra, 0, false));
    cases.push((
        "missing unused byte",
        valid[..valid.len() - 8].to_vec(),
        0,
        false,
    ));
    cases.push(("truncated count", valid[..31].to_vec(), 0, false));
    let mut huge = valid.clone();
    huge[..31].fill(true);
    cases.push(("unbacked large count", huge, 0, false));
    let mut missing_zero = payload.clone();
    missing_zero[terminator] = true;
    cases.push((
        "missing zero terminator",
        native_envelope(&missing_zero),
        1,
        true,
    ));
    let mut bad_boundary = payload.clone();
    bad_boundary[boundary..boundary + 16].fill(false);
    cases.push((
        "input boundary before header",
        native_envelope(&bad_boundary),
        1,
        true,
    ));
    let mut far_boundary = payload.clone();
    far_boundary[boundary..boundary + 16].fill(true);
    cases.push((
        "input boundary past payload",
        native_envelope(&far_boundary),
        1,
        true,
    ));
    let mut input_magic = payload.clone();
    input_magic[input_start] = false; // A3 -> A2
    cases.push((
        "unknown input magic",
        native_envelope(&input_magic),
        1,
        true,
    ));
    let mut discrepancy_magic = payload.clone();
    discrepancy_magic[input_end] = true; // A4 -> A5
    cases.push((
        "unknown discrepancy magic",
        native_envelope(&discrepancy_magic),
        1,
        true,
    ));
    let mut input_body = payload.clone();
    input_body.insert(input_end, true);
    for bit in 0..16 {
        input_body[boundary + bit] = ((input_end + 1) >> bit) & 1 != 0;
    }
    cases.push((
        "unrepresented input body",
        native_envelope(&input_body),
        1,
        true,
    ));
    let mut discrepancy_body = payload.clone();
    discrepancy_body.bit(true);
    cases.push((
        "unrepresented discrepancy body",
        native_envelope(&discrepancy_body),
        1,
        true,
    ));

    for (name, failed, prefix_moves, valid_envelope) in cases {
        let rpc = wrap_updates_array(&updates_array(&[
            update_with_stream(11, &failed),
            update_with_stream(22, &valid),
        ]));
        let (result, moves) = decode_with_context(&rpc);
        assert_eq!(result.error_count, 1, "{name}");
        assert_eq!(result.component_streams, 2, "{name}");
        assert_eq!(result.component_complete, 1, "{name}");
        assert_eq!(result.component_errors, 1, "{name}");
        assert_eq!(
            result.component_envelopes,
            1 + u32::from(valid_envelope),
            "{name}"
        );
        assert_eq!(moves.len(), prefix_moves + 1, "{name}");
        let (last, context) = moves.last().unwrap();
        assert_eq!(last.shooter_character_net_guid, 22, "{name}");
        assert_eq!(last.timestamp, 7, "{name}");
        assert_eq!(context.update_ordinal, 1, "{name}");
        assert_eq!(context.move_index, 0, "{name}");
    }
}

#[test]
fn empty_native_arrays_require_zero_unused_bits_and_no_extra_data() {
    for unused in 0..=8 {
        let mut stream = BitWriter::new();
        stream.i32(0).u8(unused);
        let (result, moves) = decode(&build_rpc_payload(11, &stream));
        assert!(moves.is_empty());
        assert_eq!(result.component_streams, 1);
        assert_eq!(result.component_complete, u32::from(unused == 0));
        assert_eq!(result.component_empty, u32::from(unused == 0));
        assert_eq!(result.error_count, u32::from(unused != 0));
        assert_eq!(result.component_unused_bits, 0);
        assert_eq!(result.component_payload_bytes, 0);
        stream.bit(false);
        assert_eq!(decode(&build_rpc_payload(11, &stream)).0.error_count, 1);
    }
}

#[test]
fn unused_high_bits_are_outside_the_logical_payload_not_a_zero_padding_requirement() {
    let stream = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)]);
    let unused = *pack(&stream).last().unwrap() as usize;
    assert!(
        unused > 0,
        "this generated stream must exercise high padding bits"
    );
    let mut nonzero_padding = stream.clone();
    let end = nonzero_padding.len() - 8;
    nonzero_padding[end - unused..end].fill(true);
    let (original, original_moves) = decode(&build_rpc_payload(11, &stream));
    let (changed, changed_moves) = decode(&build_rpc_payload(11, &nonzero_padding));
    assert_eq!(changed.error_count, 0);
    assert_eq!(format!("{original:?}"), format!("{changed:?}"));
    assert_eq!(
        movement_values(original_moves[0]),
        movement_values(changed_moves[0])
    );
}

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

/// A native i32-sized byte array and its unused-bit byte.
fn native_envelope(payload: &[bool]) -> BitWriter {
    let unused = (8 - payload.len() % 8) % 8;
    let mut stream = BitWriter::new();
    stream
        .i32(payload.len().div_ceil(8) as i32)
        .extend_bits(payload);
    stream.repeat(false, unused).u8(unused as u8);
    stream
}

/// Observed replay form: explicit movement terminator, absolute input end,
/// header-only input component and optional discrepancy component.
fn native_stream(moves: &[BitWriter], discrepancy: bool) -> BitWriter {
    let mut payload = section(moves, &[]);
    let end_bit = (payload.len() + 24) as u16;
    payload.u16(end_bit).u8(0xA3);
    if discrepancy {
        payload.u8(0xA4);
    }
    native_envelope(&payload)
}

fn real_stream(moves: &[BitWriter]) -> BitWriter {
    native_stream(moves, true)
}

/// A valid native envelope whose movement magic is wrong.
fn bad_magic_stream() -> BitWriter {
    native_envelope(&[false; 8])
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

#[test]
fn an_out_of_range_update_index_is_counted_not_discarded_in_silence() {
    // updateCount 1, index 2, then a well-formed update only the index check
    // refuses; uncounted, the lost update looks like none was sent.
    let stream = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)]);
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
    let stream = real_stream(&[build_move(false, 7, 1.0, 2.0, 3.0)]);
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
    let stream = real_stream(&[build_move(false, 7, 1.0, 2.0, 3.0)]);
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
    let short_header = BitWriter::new().u8(0x52).clone(); // under 32 bits
    let good = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)]);
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
    let stream = real_stream(&[build_move(true, 7, 1.0, 2.0, 3.0)]);
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
fn decodes_both_move_variants_with_optional_discrepancy_headers() {
    let moves = [
        build_move(false, 42, 1.25, 2.5, 3.75),
        build_move(true, 84, 10.0, 11.0, 12.0),
    ];
    for discrepancy in [false, true] {
        let stream = native_stream(&moves, discrepancy);
        let (result, decoded) = decode(&build_rpc_payload(1234, &stream));
        assert_eq!(
            (result.total_moves, result.update_count, result.error_count),
            (2, 1, 0)
        );
        assert_eq!(result.component_streams, 1);
        assert_eq!(result.component_complete, 1);
        assert_eq!(result.component_errors, 0);
        assert_eq!(result.component_envelopes, 1);
        assert_eq!(result.component_input_headers, 1);
        assert_eq!(result.component_discrepancy_headers, u32::from(discrepancy));
        let got: Vec<_> = decoded
            .iter()
            .map(|m| {
                (
                    m.shooter_character_net_guid,
                    m.move_type,
                    m.timestamp,
                    m.movement_state,
                    [m.pos_x, m.pos_y, m.pos_z],
                    [m.vel_x, m.vel_y, m.vel_z],
                )
            })
            .collect();
        let variant0 = (1234, 0, 42, 3, [1.25, 2.5, 3.75], [0.0; 3]);
        let variant1 = (1234, 1, 84, 3, [10.0, 11.0, 12.0], [4.0, 5.0, 6.0]);
        assert_eq!(got, [variant0, variant1]);
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
fn unobserved_direct_and_sized_windows_are_refused_instead_of_losing_tails() {
    // Retain the previously accepted synthetic shapes as negative grammar
    // regressions. None has the wire's native 32-bit byte-array envelope.
    let mv = || build_move(true, 10, 1.0, 2.0, 3.0);
    let sized = |s: &[bool]| component_stream(s, true);
    let open = |s: &[bool]| component_stream(s, false);
    let after_move = section(&[mv()], &[true; 40]);
    let after_magic = section(&[], &[true; 40]);
    let no_magic = BitWriter::new().bits(0b10110, 5).clone();
    let no_marker = BitWriter::new().u8(MOVEMENT_MAGIC).bits(0b11, 2).clone();
    let ends = section(&[mv()], &[]);
    let only_zero = section(&[], &[]);
    for (name, stream) in [
        ("sized, move", sized(&after_move)),
        ("sized, magic", sized(&after_magic)),
        ("no magic", sized(&no_magic)),
        ("no marker", sized(&no_marker)),
        ("open, move", open(&after_move)),
        ("empty open window", open(&[])),
        ("ends on zero", sized(&ends)),
        ("only zero", sized(&only_zero)),
    ] {
        let (result, decoded) = decode(&build_rpc_payload(77, &stream));
        assert!(decoded.is_empty(), "{name}");
        assert_eq!(
            result.error_count, 1,
            "{name}: whole RPC must be retained raw"
        );
        assert_eq!(result.component_errors, 1, "{name}");
        assert_eq!(result.component_complete, 0, "{name}");
    }
    let (result, moves) = decode(&build_rpc_payload(77, &real_stream(&[mv(), mv()])));
    assert_eq!(moves.len(), 2);
    assert_eq!(result.error_count, 0);
}

#[test]
fn native_envelopes_and_unused_bits_are_counted_even_when_inner_parsing_fails() {
    let one = native_stream(&[build_move(false, 9, 4.0, 5.0, 6.0)], false);
    let two = native_stream(&[build_move(true, 9, 4.0, 5.0, 6.0)], true);
    let empty = native_envelope(&[]);
    let bad = bad_magic_stream();
    let streams = [&one, &two, &empty, &bad];
    let updates: Vec<_> = streams.iter().map(|s| update_with_stream(77, s)).collect();
    let (result, moves) = decode(&wrap_updates_array(&updates_array(&updates)));
    assert_eq!(moves.len(), 2);
    assert_eq!(result.error_count, 1);
    assert_eq!(result.component_streams, 4);
    assert_eq!(result.component_complete, 3);
    assert_eq!(result.component_errors, 1);
    assert_eq!(result.component_envelopes, 4);
    assert_eq!(result.component_empty, 1);
    assert_eq!(result.component_input_headers, 2);
    assert_eq!(result.component_discrepancy_headers, 1);
    assert_eq!(
        result.component_payload_bytes,
        streams
            .iter()
            .map(|s| (s.len() as u64 - 40) / 8)
            .sum::<u64>()
    );
    let unused: u64 = streams
        .iter()
        .map(|s| {
            let bits = &s[s.len() - 8..];
            bits.iter()
                .enumerate()
                .map(|(i, b)| u64::from(*b) << i)
                .sum::<u64>()
        })
        .sum();
    assert_eq!(result.component_unused_bits, unused);
}

#[test]
fn movement_context_distinguishes_repeated_arrays_and_sparse_update_indices() {
    let stream = real_stream(&[build_move(true, 17, 1.0, -2.0, 3.0)]);
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
    let good = real_stream(&[build_move(true, 17, 1.0, 2.0, 3.0)]);
    let empty = real_stream(&[]);
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
    assert_eq!(
        result.component_streams, 4,
        "the orphan has no attribution and is not attempted"
    );
    assert_eq!(result.component_complete, 3);
    assert_eq!(result.component_errors, 1);
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
    let stream = real_stream(&samples);
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
    let failed = real_stream(&[good_move.clone(), bad_move]);
    let good = real_stream(&[good_move]);
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
