//! Independent wire fixtures and optional-projection failure contracts.

mod common;

use common::*;
use smallvec::smallvec;
use vrf_container::parse_event_chunk;
use vrfkit::gameplay::{
    ActorReference, ActorReferenceStatus, GameplayError, GameplayEvidence, GameplayKind,
    GameplaySource, RoundPhase, native_event, round_phase_event,
};
use vrfkit::{EventChunk, FieldRecord, ReadError, ReplayEvent, ReplayReader};

fn payload(tag: u32, words: &[u32], name: &str, seconds: f32, wide: bool) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    for word in words {
        bytes.extend(word.to_le_bytes());
    }
    if wide {
        let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
        bytes.extend((-(units.len() as i32)).to_le_bytes());
        for unit in units {
            bytes.extend(unit.to_le_bytes());
        }
    } else {
        bytes.extend((name.len() as i32 + 1).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.push(0);
    }
    bytes.extend(seconds.to_le_bytes());
    bytes
}

/// An Event chunk body around `bytes`; `EventChunk` is non-exhaustive, so a
/// fixture comes from the container parser like a reader's would.
fn event_body(group: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    add_fstring(&mut body, "synthetic-native-event");
    add_fstring(&mut body, group);
    add_fstring(&mut body, "wire-metadata");
    add_u32(&mut body, 1250);
    add_u32(&mut body, 1260);
    add_i32(&mut body, bytes.len() as i32);
    body.extend(bytes);
    body
}

fn event(body: &[u8]) -> EventChunk<'_> {
    let raw = parse_event_chunk(body).unwrap();
    assert_eq!(raw.trailing_bytes, 0);
    raw
}

fn phase_field() -> FieldRecord {
    FieldRecord {
        time_ms: 1200,
        packet_id: 17,
        channel_index: 11,
        actor_net_guid: 7,
        object_net_guid: None,
        group_path: "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache".into(),
        handle: 1,
        field_name: Some("MulticastSetPhase.NewPhase".into()),
        compatible_checksum: Some(3_954_716_686),
        bit_count: 8,
        raw_bits: Some(smallvec![3]),
        value_i64: Some(3),
        value_f64: None,
        value_bool: None,
        value_str: None,
    }
}

#[test]
fn five_native_groups_preserve_values_and_borrowed_provenance() {
    let cases = [
        (
            "roundStarted",
            2,
            vec![17],
            "RoundStart",
            GameplayKind::RoundStarted { round_number: 17 },
        ),
        (
            "characterDeath",
            8,
            vec![0, 91],
            "CharacterDeath",
            GameplayKind::CharacterDeath {
                killer: ActorReference { net_guid: 0 },
                victim: ActorReference { net_guid: 91 },
            },
        ),
        (
            "spikePlanted",
            4,
            vec![],
            "SpikePlanted",
            GameplayKind::SpikePlanted,
        ),
        (
            "spikeDefused",
            5,
            vec![],
            "SpikeDefused",
            GameplayKind::SpikeDefused,
        ),
        (
            "spikeExploded",
            6,
            vec![],
            "SpikeExploded",
            GameplayKind::SpikeExploded,
        ),
    ];
    for (group, tag, words, name, expected) in cases {
        for wide in [false, true] {
            let bytes = payload(
                tag,
                &words,
                &format!("EReplayEventGroup::{name}"),
                1.25,
                wide,
            );
            let body = event_body(group, &bytes);
            let raw = event(&body);
            let projected = native_event(&raw).unwrap().unwrap();
            assert_eq!(projected.kind, expected);
            assert_eq!(projected.time_ms(), 1250);
            assert_eq!(
                projected.evidence(),
                GameplayEvidence::ValidatedNativePayload
            );
            let GameplaySource::NativeContainer(source) = projected.source else {
                panic!()
            };
            assert_eq!(source.time2_ms, 1260);
            assert_eq!(source.payload_seconds.to_bits(), 1.25_f32.to_bits());
            assert_eq!(
                (source.id, source.group, source.metadata),
                (raw.id.as_str(), raw.group.as_str(), raw.metadata.as_str())
            );
            assert_eq!(source.raw_payload.as_ptr(), raw.payload.as_ptr());
            assert_eq!(source.raw_payload, bytes);
        }
    }
}

#[test]
fn native_actor_references_never_claim_resolved_players() {
    assert_eq!(
        ActorReference { net_guid: 0 }.status(),
        ActorReferenceStatus::Null
    );
    assert_eq!(
        ActorReference { net_guid: 1 }.status(),
        ActorReferenceStatus::Unresolved
    );
    assert_eq!(
        ActorReference { net_guid: u32::MAX }.status(),
        ActorReferenceStatus::Unresolved
    );
}

#[test]
fn unsupported_native_groups_stay_uninterpreted() {
    for group in [
        "futureGroup",
        "characterUltimateUsed",
        "switchTeams",
        "RoundStarted",
    ] {
        let body = event_body(group, &[0xff]);
        assert!(native_event(&event(&body)).unwrap().is_none());
    }
}

#[test]
fn native_shape_mutations_fail_without_losing_raw_payload() {
    let original = payload(
        8,
        &[13, 27],
        "EReplayEventGroup::CharacterDeath",
        1.25,
        false,
    );
    for end in 0..original.len() {
        let truncated = &original[..end];
        let body = event_body("characterDeath", truncated);
        let raw = event(&body);
        assert_eq!(
            native_event(&raw).unwrap_err(),
            GameplayError::PayloadLayout
        );
        assert_eq!(raw.payload, truncated);
    }
    let mut bad_tag = original.clone();
    bad_tag[0] = 2;
    let mut bad_name = original.clone();
    bad_name[16] = b'X';
    let mut bad_arity = original.clone();
    bad_arity.splice(12..12, [0, 0, 0, 0]);
    let mut extra = original.clone();
    extra.push(0);
    let mut unterminated = original.clone();
    let terminator = unterminated.len() - 5;
    unterminated.remove(terminator);
    let length = i32::from_le_bytes(unterminated[12..16].try_into().unwrap()) - 1;
    unterminated[12..16].copy_from_slice(&length.to_le_bytes());
    for bytes in [bad_tag, bad_name, bad_arity, extra, unterminated] {
        let body = event_body("characterDeath", &bytes);
        let raw = event(&body);
        assert_eq!(
            native_event(&raw).unwrap_err(),
            GameplayError::PayloadLayout
        );
        assert_eq!(raw.payload, bytes);
    }
}

#[test]
fn native_declarations_and_time_are_checked_separately() {
    let bytes = payload(2, &[4], "EReplayEventGroup::RoundStart", 1.25, false);
    let body = event_body("roundStarted", &bytes);
    for size in [-1, bytes.len() as i32 - 1, bytes.len() as i32 + 1] {
        let mut raw = event(&body);
        raw.size_in_bytes = size;
        assert_eq!(native_event(&raw).unwrap_err(), GameplayError::PayloadSize);
    }
    let mut raw = event(&body);
    raw.trailing_bytes = 1;
    assert_eq!(
        native_event(&raw).unwrap_err(),
        GameplayError::TrailingBytes
    );
    for seconds in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.3, -1.25] {
        let bytes = payload(2, &[4], "EReplayEventGroup::RoundStart", seconds, false);
        assert_eq!(
            native_event(&event(&event_body("roundStarted", &bytes))).unwrap_err(),
            GameplayError::PayloadTime
        );
    }
}

#[test]
fn zero_and_quantized_native_time_remain_wire_values() {
    for (seconds, milliseconds) in [(0.0, 0), (1.2509, 1250)] {
        let bytes = payload(4, &[], "EReplayEventGroup::SpikePlanted", seconds, false);
        let body = event_body("spikePlanted", &bytes);
        let mut raw = event(&body);
        raw.time1 = milliseconds;
        let projected = native_event(&raw).unwrap().unwrap();
        assert_eq!(projected.time_ms(), milliseconds);
    }
}

#[test]
fn all_phase_bytes_round_trip_without_inventing_transitions() {
    for (byte, expected) in [
        (2, RoundPhase::Reset),
        (3, RoundPhase::RoundStart),
        (4, RoundPhase::BuyPhaseEnd),
        (5, RoundPhase::UnnamedPostRound),
        (6, RoundPhase::SideSwitch),
    ] {
        let mut field = phase_field();
        field.value_i64 = Some(i64::from(byte));
        field.raw_bits = Some(smallvec![byte]);
        assert_eq!(
            round_phase_event(&field).unwrap().unwrap().kind,
            GameplayKind::RoundPhaseObserved(expected)
        );
    }
    for byte in 0..=255_u8 {
        let mut field = phase_field();
        field.value_i64 = Some(i64::from(byte));
        field.raw_bits = Some(smallvec![byte]);
        let projected = round_phase_event(&field).unwrap().unwrap();
        assert_eq!(
            projected.evidence(),
            GameplayEvidence::ValidatedReplicationValue
        );
        assert_eq!(projected.time_ms(), 1200);
        let GameplayKind::RoundPhaseObserved(phase) = projected.kind else {
            panic!()
        };
        assert_eq!(phase.wire_value(), byte);
        if !(2..=6).contains(&byte) {
            assert_eq!(phase, RoundPhase::Unknown(byte));
        }
        let GameplaySource::MainField(source) = projected.source else {
            panic!()
        };
        assert!(std::ptr::eq(source, &field));
        assert_eq!(
            round_phase_event(&field).unwrap().unwrap().kind,
            projected.kind
        );
    }
}

#[test]
fn phase_schema_identity_is_not_inferred_from_field_name() {
    let original = phase_field();
    let mut cases = Vec::new();
    let mut changed = original.clone();
    changed.group_path = "/Game/Unrelated".into();
    cases.push(changed);
    let mut changed = original.clone();
    changed.object_net_guid = Some(0);
    cases.push(changed);
    let mut changed = original.clone();
    changed.compatible_checksum = None;
    cases.push(changed);
    let mut changed = original.clone();
    changed.compatible_checksum = Some(1);
    cases.push(changed);
    for field in cases {
        assert_eq!(
            round_phase_event(&field).unwrap_err(),
            GameplayError::PhaseIdentity
        );
    }
    let mut other = original;
    other.field_name = Some("CurrentPhase".into());
    assert!(round_phase_event(&other).unwrap().is_none());
    other.field_name = None;
    assert!(round_phase_event(&other).unwrap().is_none());
}

#[test]
fn phase_overlay_must_be_present_unique_and_in_byte_range() {
    for value in [None, Some(-1), Some(256), Some(i64::MAX)] {
        let mut field = phase_field();
        field.value_i64 = value;
        assert_eq!(
            round_phase_event(&field).unwrap_err(),
            GameplayError::PhaseValue
        );
    }
    for column in 0..3 {
        let mut field = phase_field();
        match column {
            0 => field.value_f64 = Some(3.0),
            1 => field.value_bool = Some(true),
            _ => field.value_str = Some("3".into()),
        }
        assert_eq!(
            round_phase_event(&field).unwrap_err(),
            GameplayError::PhaseValue
        );
    }
}

#[test]
fn phase_raw_window_must_equal_the_overlay_exactly() {
    for bits in [0, 7, 9, u32::MAX] {
        let mut field = phase_field();
        field.bit_count = bits;
        assert_eq!(
            round_phase_event(&field).unwrap_err(),
            GameplayError::PhaseRawValue
        );
    }
    for raw in [
        None,
        Some(smallvec![]),
        Some(smallvec![4]),
        Some(smallvec![3, 0]),
    ] {
        let mut field = phase_field();
        field.raw_bits = raw;
        assert_eq!(
            round_phase_event(&field).unwrap_err(),
            GameplayError::PhaseRawValue
        );
    }
}

fn native_chunk(group: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    add_fstring(&mut body, "synthetic-native-event");
    add_fstring(&mut body, group);
    add_fstring(&mut body, "");
    add_u32(&mut body, 1250);
    add_u32(&mut body, 1250);
    add_i32(&mut body, bytes.len() as i32);
    body.extend(bytes);
    chunk(3, &body)
}

#[test]
fn optional_projection_preserves_reader_order_and_checkpoint_scope() {
    let mut bytes = replay_info(&Info::default());
    bytes.extend(chunk(0, &header_payload()));
    bytes.extend(native_chunk(
        "roundStarted",
        &payload(2, &[7], "EReplayEventGroup::RoundStart", 1.25, false),
    ));
    bytes.extend(chunk(2, &[0xde, 0xad]));
    bytes.extend(native_chunk(
        "spikeDefused",
        &payload(5, &[], "EReplayEventGroup::SpikeDefused", 1.25, false),
    ));
    let mut kinds = Vec::new();
    let summary = ReplayReader::new()
        .read(&bytes, |event| {
            if let ReplayEvent::ContainerEvent(raw) = event {
                if let Some(projected) = native_event(&raw)? {
                    kinds.push(projected.kind);
                }
            }
            Ok::<(), GameplayError>(())
        })
        .unwrap();
    assert_eq!(
        kinds,
        [
            GameplayKind::RoundStarted { round_number: 7 },
            GameplayKind::SpikeDefused
        ]
    );
    assert_eq!(summary.event_chunks, 2);
    assert_eq!(summary.skipped_checkpoint_chunks, 1);
}

#[test]
fn supported_malformed_event_can_stop_consumer_without_changing_raw_reader() {
    let mut bytes = replay_info(&Info::default());
    bytes.extend(chunk(0, &header_payload()));
    bytes.extend(native_chunk("roundStarted", &[0xff]));
    bytes.extend(native_chunk("futureGroup", &[1, 2]));
    let mut raw_count = 0;
    let summary = ReplayReader::new()
        .read(&bytes, |event| {
            if matches!(event, ReplayEvent::ContainerEvent(_)) {
                raw_count += 1;
            }
            Ok::<(), GameplayError>(())
        })
        .unwrap();
    assert_eq!(summary.event_chunks, 2);
    assert_eq!(raw_count, 2);
    let mut seen = 0;
    let err = ReplayReader::new()
        .read(&bytes, |event| {
            if let ReplayEvent::ContainerEvent(raw) = event {
                seen += 1;
                native_event(&raw)?;
            }
            Ok::<(), GameplayError>(())
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ReadError::Consumer(GameplayError::PayloadLayout)
    ));
    assert_eq!(seen, 1);
}
