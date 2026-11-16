//! Public-API regressions for retired versus concurrently written identity evidence.

use vrfkit::gameplay::{ActorReference, IdentityResolution, MainIdentityTimeline};
use vrfkit::{ActorRecord, FieldRecord};

const OLD_STATE: u32 = 12;
const NEW_STATE: u32 = 19;
const BODY: u32 = 51;

fn actor(packet: u32, guid: u32, event: &'static str) -> ActorRecord {
    ActorRecord {
        packet_id: packet,
        time_ms: packet * 100,
        actor_net_guid: guid,
        channel_index: guid,
        event,
        ..ActorRecord::default()
    }
}

fn declaration(packet: u32, state: u32) -> FieldRecord {
    FieldRecord {
        time_ms: packet * 100,
        packet_id: packet,
        channel_index: state,
        actor_net_guid: state,
        object_net_guid: None,
        group_path: "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C".into(),
        handle: 14,
        field_name: Some("SpawnedCharacter".into()),
        compatible_checksum: Some(1_413_303_948),
        bit_count: 8,
        raw_bits: Some(vec![BODY as u8 * 2].into()),
        value_i64: Some(i64::from(BODY)),
        value_f64: None,
        value_bool: None,
        value_str: None,
    }
}

fn known() -> MainIdentityTimeline {
    let mut timeline = MainIdentityTimeline::new();
    timeline
        .observe_packet(
            1,
            &[actor(1, OLD_STATE, "open"), actor(1, BODY, "open")],
            &[declaration(1, OLD_STATE)],
        )
        .unwrap();
    timeline
}

fn resolved_state(timeline: &MainIdentityTimeline, boundary: u32) -> u32 {
    match timeline.resolve_before_packet(ActorReference { net_guid: BODY }, boundary) {
        IdentityResolution::Resolved(body) => body.player_state_net_guid,
        other => panic!("expected one current observed relation, got {other:?}"),
    }
}

#[test]
fn ended_player_state_declaration_does_not_make_fresh_relation_ambiguous() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, OLD_STATE, "close")], &[])
        .unwrap();
    timeline
        .observe_packet(3, &[actor(3, OLD_STATE, "open")], &[])
        .unwrap();
    timeline
        .observe_packet(
            4,
            &[actor(4, NEW_STATE, "open")],
            &[declaration(4, NEW_STATE)],
        )
        .unwrap();
    assert_eq!(resolved_state(&timeline, 2), OLD_STATE);
    assert_eq!(resolved_state(&timeline, 5), NEW_STATE);
}

#[test]
fn ended_character_declaration_does_not_make_fresh_relation_ambiguous() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, BODY, "close")], &[])
        .unwrap();
    timeline
        .observe_packet(3, &[actor(3, BODY, "open")], &[])
        .unwrap();
    timeline
        .observe_packet(
            4,
            &[actor(4, NEW_STATE, "open")],
            &[declaration(4, NEW_STATE)],
        )
        .unwrap();
    assert_eq!(resolved_state(&timeline, 2), OLD_STATE);
    assert_eq!(resolved_state(&timeline, 5), NEW_STATE);
}

#[test]
fn same_packet_close_and_declaration_remain_ambiguous() {
    let mut timeline = known();
    timeline
        .observe_packet(
            2,
            &[actor(2, BODY, "close"), actor(2, BODY, "open")],
            &[declaration(2, OLD_STATE)],
        )
        .unwrap();
    timeline
        .observe_packet(
            3,
            &[actor(3, NEW_STATE, "open")],
            &[declaration(3, NEW_STATE)],
        )
        .unwrap();
    match timeline.resolve_before_packet(ActorReference { net_guid: BODY }, 4) {
        IdentityResolution::Ambiguous { declarations } => {
            assert_eq!(declarations.len(), 2);
            assert_eq!(declarations[0].actor_net_guid, OLD_STATE);
            assert_eq!(declarations[1].actor_net_guid, NEW_STATE);
        }
        other => panic!("same-packet inter-vector ordering is unknown, got {other:?}"),
    }
    assert_eq!(resolved_state(&timeline, 2), OLD_STATE);
}
