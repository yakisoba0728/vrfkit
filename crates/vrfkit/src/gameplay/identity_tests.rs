//! Synthetic packet/lifecycle evidence, independent of the timeline storage.

use super::*;
use crate::MovementSource;

const STATE: u32 = 12;
const BODY: u32 = 51;

fn packed(mut value: u32) -> Vec<u8> {
    let mut raw = Vec::new();
    loop {
        let part = ((value & 0x7f) as u8) << 1;
        value >>= 7;
        raw.push(part | u8::from(value != 0));
        if value == 0 {
            return raw;
        }
    }
}

fn declaration(packet: u32, state: u32, body: u32) -> FieldRecord {
    let raw = packed(body);
    FieldRecord {
        time_ms: 1000 - packet,
        packet_id: packet,
        channel_index: state,
        actor_net_guid: state,
        object_net_guid: None,
        group_path: BOMB_PLAYER_STATE.into(),
        handle: 14,
        field_name: Some("SpawnedCharacter".into()),
        compatible_checksum: Some(1_413_303_948),
        bit_count: raw.len() as u32 * 8,
        raw_bits: Some(raw.into()),
        value_i64: Some(i64::from(body)),
        value_f64: None,
        value_bool: None,
        value_str: None,
    }
}

fn actor(packet: u32, guid: u32, channel: u32, event: &'static str) -> ActorRecord {
    ActorRecord {
        packet_id: packet,
        time_ms: 1000 - packet,
        actor_net_guid: guid,
        channel_index: channel,
        event,
        ..ActorRecord::default()
    }
}

fn known() -> MainIdentityTimeline {
    let mut timeline = MainIdentityTimeline::new();
    timeline
        .observe_packet(
            1,
            &[actor(1, STATE, STATE, "open"), actor(1, BODY, BODY, "open")],
            &[declaration(1, STATE, BODY)],
        )
        .unwrap();
    timeline
}

/// The outcome with its exact unresolved reason: the reasons are the contract.
fn status(timeline: &MainIdentityTimeline, guid: u32, packet: u32) -> &'static str {
    match timeline.resolve_before_packet(ActorReference { net_guid: guid }, packet) {
        IdentityResolution::Null => "null",
        IdentityResolution::Unresolved(reason) => match reason {
            IdentityUnresolved::NoReference => "no reference",
            IdentityUnresolved::UnobservedPacket => "unobserved",
            IdentityUnresolved::InvalidTimeline => "invalid",
            IdentityUnresolved::Lifecycle => "lifecycle",
            IdentityUnresolved::PreviousLifetime => "previous lifetime",
            IdentityUnresolved::ConcurrentChange => "concurrent",
        },
        IdentityResolution::Ambiguous { .. } => "ambiguous",
        IdentityResolution::Resolved(_) => "resolved",
    }
}

fn movement(packet: u32) -> MovementRecord {
    MovementRecord {
        time_ms: 250,
        packet_id: packet,
        character_net_guid: BODY,
        pos_x: 1.0,
        pos_y: 2.0,
        pos_z: 3.0,
        yaw: 4.0,
        pitch: 5.0,
        vel_x: 6.0,
        vel_y: 7.0,
        vel_z: 8.0,
        timestamp: 9,
        movement_state: 0,
        move_type: 1,
    }
}

fn provenance(initial: u32, completion: u32) -> MovementProvenance {
    MovementProvenance {
        source: Some(MovementSource {
            initial_packet_id: initial,
            completion_packet_id: completion,
            channel_index: 73,
            actor_net_guid: 999,
            object_net_guid: None,
            bunch_index: 3,
            block_index: 0,
            rpc_index: 1,
            rpc_handle: 4,
        }),
        array_ordinal: 0,
        update_ordinal: 0,
        update_index: 2,
        component_ordinal: 0,
        move_index: 1,
    }
}

fn batch<'a>(
    movement: &'a [MovementRecord],
    provenance: &'a [MovementProvenance],
) -> ReplayRecords<'a> {
    ReplayRecords {
        fields: &[],
        actors: &[],
        movement,
        movement_provenance: provenance,
        partials: &[],
    }
}

#[test]
fn body_projection_preserves_complete_borrowed_evidence_and_null() {
    for value in [0, 51, 128, 65_535, u32::MAX] {
        let field = declaration(1, STATE, value);
        let observed = spawned_character_observation(&field).unwrap().unwrap();
        assert_eq!(observed.character.net_guid, value);
        assert_eq!(observed.player_state_net_guid, STATE);
        assert!(std::ptr::eq(observed.source, &field));
        assert_eq!(observed.source.raw_bits.as_deref().unwrap(), packed(value));
    }
    let mut swift = declaration(1, STATE, BODY);
    swift.group_path = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C".into();
    assert!(spawned_character_observation(&swift).unwrap().is_some());
    for name in ["Owner", "PossessedCharacter", "PlayerState"] {
        swift.field_name = Some(name.into());
        assert!(spawned_character_observation(&swift).unwrap().is_none());
    }
}

#[test]
fn identity_and_overlay_mutations_are_rejected_without_changing_raw() {
    let original = declaration(1, STATE, BODY);
    let mut variants = Vec::new();
    let mut row = original.clone();
    row.group_path = "/Game/Other.Other_C".into();
    variants.push(row);
    let mut row = original.clone();
    row.object_net_guid = Some(0);
    variants.push(row);
    let mut row = original.clone();
    row.actor_net_guid = 0;
    variants.push(row);
    let mut row = original.clone();
    row.compatible_checksum = None;
    variants.push(row);
    let mut row = original.clone();
    row.compatible_checksum = Some(17);
    variants.push(row);
    for row in variants {
        assert_eq!(
            spawned_character_observation(&row).unwrap_err(),
            IdentityError::FieldIdentity
        );
        assert_eq!(row.raw_bits, original.raw_bits);
    }
    for value in [None, Some(-1), Some(i64::from(u32::MAX) + 1)] {
        let mut row = original.clone();
        row.value_i64 = value;
        assert_eq!(
            spawned_character_observation(&row).unwrap_err(),
            IdentityError::FieldValue
        );
    }
    let mut row = original.clone();
    row.value_bool = Some(false);
    assert_eq!(
        spawned_character_observation(&row).unwrap_err(),
        IdentityError::FieldValue
    );
    row.value_bool = None;
    row.value_f64 = Some(51.0);
    assert_eq!(
        spawned_character_observation(&row).unwrap_err(),
        IdentityError::FieldValue
    );
    row.value_f64 = None;
    row.value_str = Some("51".into());
    assert_eq!(
        spawned_character_observation(&row).unwrap_err(),
        IdentityError::FieldValue
    );
}

#[test]
fn packed_guid_negative_controls_require_exact_value_width_and_termination() {
    for raw in [
        vec![],
        vec![0],
        vec![102, 0],
        vec![103],
        vec![255; 5],
        vec![255, 255, 255, 255, 32],
    ] {
        let mut row = declaration(1, STATE, BODY);
        row.bit_count = raw.len() as u32 * 8;
        row.raw_bits = Some(raw.clone().into());
        assert_eq!(
            spawned_character_observation(&row).unwrap_err(),
            IdentityError::FieldRawValue
        );
        assert_eq!(row.raw_bits.as_deref().unwrap(), raw);
    }
    for width in [0, 7, 9, 16] {
        let mut row = declaration(1, STATE, BODY);
        row.bit_count = width;
        assert_eq!(
            spawned_character_observation(&row).unwrap_err(),
            IdentityError::FieldRawValue
        );
    }
    let mut row = declaration(1, STATE, BODY);
    row.raw_bits = None;
    assert_eq!(
        spawned_character_observation(&row).unwrap_err(),
        IdentityError::FieldRawValue
    );
}

#[test]
fn later_evidence_never_backfills_earlier_or_same_packet_boundaries() {
    let mut timeline = MainIdentityTimeline::new();
    timeline
        .observe_packet(
            1,
            &[actor(1, STATE, STATE, "open"), actor(1, BODY, BODY, "open")],
            &[],
        )
        .unwrap();
    assert_eq!(status(&timeline, BODY, 2), "no reference");
    timeline
        .observe_packet(2, &[], &[declaration(2, STATE, BODY)])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 1), "no reference");
    assert_eq!(status(&timeline, BODY, 2), "no reference");
    assert_eq!(status(&timeline, BODY, 3), "resolved");
    assert_eq!(status(&timeline, BODY, 4), "unobserved");
    assert_eq!(status(&timeline, 0, 4), "null");
    let IdentityResolution::Resolved(body) =
        timeline.resolve_before_packet(ActorReference { net_guid: BODY }, 3)
    else {
        panic!()
    };
    assert_eq!(body.declaration.time_ms, 998); // Timestamp regressed; packet order wins.
    assert_eq!(body.declaration.packet_id, 2);
    assert_eq!(body.character_open.packet_id, 1);
    assert_eq!(body.player_state_net_guid, STATE);
}

#[test]
fn timeline_owns_reused_callback_buffers_and_requires_observed_lifetimes() {
    let mut timeline = MainIdentityTimeline::new();
    let mut field = declaration(1, STATE, BODY);
    timeline
        .observe_packet(1, &[], std::slice::from_ref(&field))
        .unwrap();
    assert_eq!(status(&timeline, BODY, 2), "lifecycle");
    field.raw_bits.as_mut().unwrap()[0] = 0;
    field.value_i64 = Some(0);
    timeline
        .observe_packet(
            2,
            &[actor(2, STATE, STATE, "open"), actor(2, BODY, BODY, "open")],
            &[],
        )
        .unwrap();
    assert_eq!(status(&timeline, BODY, 3), "previous lifetime"); // State lifetime starts after the write.
    timeline
        .observe_packet(3, &[], &[declaration(3, STATE, BODY)])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "resolved");
}

#[test]
fn null_then_reconnect_keeps_historical_relation_without_current_backfill() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[], &[declaration(2, STATE, 0)])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 2), "resolved");
    assert_eq!(status(&timeline, BODY, 3), "no reference");
    timeline
        .observe_packet(3, &[actor(3, 75, 75, "open")], &[declaration(3, STATE, 75)])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "no reference");
    assert_eq!(status(&timeline, 75, 4), "resolved");
    assert_eq!(status(&timeline, 75, 2), "no reference");
    assert_eq!(status(&timeline, BODY, 2), "resolved");
}

#[test]
fn conflicting_states_are_explicit_and_preserve_both_source_declarations() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, 19, 19, "open")], &[declaration(2, 19, BODY)])
        .unwrap();
    let IdentityResolution::Ambiguous { declarations } =
        timeline.resolve_before_packet(ActorReference { net_guid: BODY }, 3)
    else {
        panic!()
    };
    assert_eq!(
        declarations
            .iter()
            .map(|row| (row.actor_net_guid, row.packet_id))
            .collect::<Vec<_>>(),
        [(STATE, 1), (19, 2)]
    );
    assert_eq!(status(&timeline, BODY, 2), "resolved");
    timeline
        .observe_packet(3, &[actor(3, 19, 19, "close")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "resolved");
}

#[test]
fn dormancy_and_dormant_channel_reuse_do_not_destroy_identity() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, BODY, BODY, "dormant")], &[])
        .unwrap();
    timeline
        .observe_packet(3, &[actor(3, 99, BODY, "open")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "resolved");
    timeline
        .observe_packet(4, &[actor(4, BODY, 72, "open")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 5), "resolved");
    let IdentityResolution::Resolved(body) =
        timeline.resolve_before_packet(ActorReference { net_guid: BODY }, 5)
    else {
        panic!()
    };
    assert_eq!(body.character_open.packet_id, 1);
}

#[test]
fn close_and_active_channel_replacement_retire_lifetimes() {
    for event in [actor(2, BODY, BODY, "close"), actor(2, 99, BODY, "open")] {
        let mut timeline = known();
        timeline.observe_packet(2, &[event], &[]).unwrap();
        assert_eq!(status(&timeline, BODY, 2), "resolved");
        assert_eq!(status(&timeline, BODY, 3), "lifecycle");
        timeline
            .observe_packet(3, &[actor(3, BODY, 72, "open")], &[])
            .unwrap();
        assert_eq!(status(&timeline, BODY, 4), "previous lifetime");
        timeline
            .observe_packet(4, &[], &[declaration(4, STATE, BODY)])
            .unwrap();
        assert_eq!(status(&timeline, BODY, 5), "resolved");
    }
}

#[test]
fn same_packet_close_reopen_and_unknown_earlier_close_cannot_reuse_a_reference() {
    let mut timeline = known();
    timeline
        .observe_packet(
            2,
            &[actor(2, BODY, BODY, "close"), actor(2, BODY, BODY, "open")],
            &[declaration(2, STATE, BODY)],
        )
        .unwrap();
    assert_eq!(status(&timeline, BODY, 3), "previous lifetime");
    timeline
        .observe_packet(3, &[], &[declaration(3, STATE, BODY)])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "resolved");
    let mut timeline = MainIdentityTimeline::new();
    timeline
        .observe_packet(
            1,
            &[actor(1, STATE, STATE, "open")],
            &[declaration(1, STATE, BODY)],
        )
        .unwrap();
    timeline
        .observe_packet(2, &[actor(2, BODY, BODY, "close")], &[])
        .unwrap();
    timeline
        .observe_packet(3, &[actor(3, BODY, BODY, "open")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "previous lifetime");
}

#[test]
fn repeated_open_without_dormancy_is_unresolved_on_same_or_other_channel() {
    for channel in [BODY, 99] {
        let mut timeline = known();
        timeline
            .observe_packet(2, &[actor(2, BODY, channel, "open")], &[])
            .unwrap();
        assert_eq!(status(&timeline, BODY, 2), "resolved");
        assert_eq!(status(&timeline, BODY, 3), "lifecycle");
    }
}

#[test]
fn closed_player_state_declaration_is_a_lifecycle_absence_not_no_reference() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, STATE, STATE, "close")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 2), "resolved");
    // The declaration exists; only the PlayerState lifetime holding it ended.
    assert_eq!(status(&timeline, BODY, 3), "lifecycle");
    // A body nobody declared is still no reference beside it.
    assert_eq!(status(&timeline, 75, 3), "no reference");
}

#[test]
fn a_closed_actor_leaves_no_stale_channel_to_end_its_next_lifetime() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, BODY, BODY, "close")], &[])
        .unwrap();
    timeline
        .observe_packet(
            3,
            &[actor(3, BODY, 72, "open")],
            &[declaration(3, STATE, BODY)],
        )
        .unwrap();
    assert_eq!(status(&timeline, BODY, 4), "resolved");
    // Another actor opening on the closed lifetime's old channel must not
    // close the reopened body on channel 72.
    timeline
        .observe_packet(4, &[actor(4, 99, BODY, "open")], &[])
        .unwrap();
    assert_eq!(status(&timeline, BODY, 5), "resolved");
}

#[test]
fn malformed_observation_poisoning_prevents_stale_resolution_after_ignored_errors() {
    for bad in [
        IdentityError::FieldRawValue,
        IdentityError::PacketMismatch,
        IdentityError::ActorLifecycle,
        IdentityError::PacketOrder,
    ] {
        let mut timeline = known();
        let result = match bad {
            IdentityError::FieldRawValue => {
                let mut row = declaration(2, STATE, 0);
                row.raw_bits = Some(vec![102].into());
                timeline.observe_packet(2, &[], &[row])
            }
            IdentityError::PacketMismatch => {
                timeline.observe_packet(2, &[], &[declaration(3, STATE, 0)])
            }
            IdentityError::ActorLifecycle => {
                timeline.observe_packet(2, &[actor(2, BODY, BODY, "mystery")], &[])
            }
            IdentityError::PacketOrder => timeline.observe_packet(1, &[], &[]),
            _ => unreachable!(),
        };
        assert_eq!(result.unwrap_err(), bad);
        assert!(matches!(
            timeline.resolve_before_packet(ActorReference { net_guid: BODY }, 2),
            IdentityResolution::Unresolved(IdentityUnresolved::InvalidTimeline)
        ));
        assert_eq!(
            timeline.observe_packet(3, &[], &[]).unwrap_err(),
            IdentityError::InvalidTimeline
        );
    }
}

#[test]
fn movement_joins_exact_aligned_source_without_controller_or_timestamp_guess() {
    let mut timeline = known();
    let moves = [movement(2)];
    let provenance = [provenance(2, 2)];
    let records = batch(&moves, &provenance);
    timeline.observe_records(&records).unwrap();
    let linked = timeline.movement_identity(&records, 0).unwrap();
    assert!(std::ptr::eq(linked.movement, &moves[0]));
    assert!(std::ptr::eq(linked.provenance, &provenance[0]));
    assert_eq!(linked.provenance.source.unwrap().actor_net_guid, 999);
    let IdentityResolution::Resolved(body) = linked.identity else {
        panic!()
    };
    assert_eq!(
        (body.character_net_guid, body.player_state_net_guid),
        (BODY, STATE)
    );
}

#[test]
fn movement_same_packet_or_partial_interval_changes_remain_unresolved() {
    for initial in [2, 3] {
        let mut timeline = known();
        timeline.observe_packet(2, &[], &[]).unwrap();
        let moves = [movement(3)];
        let provenance = [provenance(initial, 3)];
        let fields = [declaration(3, STATE, 0)];
        let records = ReplayRecords {
            fields: &fields,
            ..batch(&moves, &provenance)
        };
        timeline.observe_records(&records).unwrap();
        assert!(matches!(
            timeline.movement_identity(&records, 0).unwrap().identity,
            IdentityResolution::Unresolved(IdentityUnresolved::ConcurrentChange)
        ));
    }
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, BODY, BODY, "close")], &[])
        .unwrap();
    let moves = [movement(3)];
    let provenance = [provenance(2, 3)];
    let records = batch(&moves, &provenance);
    timeline.observe_records(&records).unwrap();
    assert!(matches!(
        timeline.movement_identity(&records, 0).unwrap().identity,
        IdentityResolution::Unresolved(IdentityUnresolved::ConcurrentChange)
    ));
}

#[test]
fn movement_missing_misaligned_or_impossible_source_is_an_error() {
    let mut timeline = known();
    let moves = [movement(2)];
    let source = [provenance(2, 2)];
    let records = batch(&moves, &source);
    timeline.observe_records(&records).unwrap();
    assert_eq!(
        timeline.movement_identity(&records, 1).unwrap_err(),
        IdentityError::MovementProvenance
    );
    assert_eq!(
        timeline
            .movement_identity(&batch(&moves, &[]), 0)
            .unwrap_err(),
        IdentityError::MovementProvenance
    );
    for bad in [
        MovementProvenance {
            source: None,
            ..source[0]
        },
        provenance(3, 2),
        provenance(1, 1),
    ] {
        assert_eq!(
            timeline
                .movement_identity(&batch(&moves, &[bad]), 0)
                .unwrap_err(),
            IdentityError::MovementProvenance
        );
    }
    timeline.observe_packet(3, &[], &[]).unwrap();
    assert_eq!(
        timeline.movement_identity(&records, 0).unwrap_err(),
        IdentityError::MovementProvenance
    );
}
