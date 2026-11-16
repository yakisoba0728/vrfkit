//! Independent packet-boundary, lifecycle, and checkpoint callback contracts.
//! Copy to crates/vrfkit/tests/qa_phase_c_contracts.rs for a recorded QA run.

mod common;

use std::convert::Infallible;

use common::*;
use vrfkit::gameplay::{
    ActorReference, IdentityError, IdentityResolution, IdentityUnresolved, MainIdentityTimeline,
    spawned_character_observation,
};
use vrfkit::{
    ActorRecord, CheckpointEvent, FieldRecord, MovementProvenance, MovementRecord, MovementSource,
    ReadError, ReadOptions, ReplayEvent, ReplayReader, ReplayRecords,
};

fn actor(packet: u32, guid: u32, channel: u32, event: &'static str) -> ActorRecord {
    ActorRecord {
        packet_id: packet,
        time_ms: 9000 - packet,
        actor_net_guid: guid,
        channel_index: channel,
        event,
        ..Default::default()
    }
}

fn reference(packet: u32, state: u32, body: u32) -> FieldRecord {
    let mut raw = Vec::new();
    add_int_packed(&mut raw, body);
    FieldRecord {
        time_ms: 7000 - packet,
        packet_id: packet,
        channel_index: state,
        actor_net_guid: state,
        object_net_guid: None,
        group_path: "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C".into(),
        handle: 19,
        field_name: Some("SpawnedCharacter".into()),
        compatible_checksum: Some(1413303948),
        bit_count: raw.len() as u32 * 8,
        raw_bits: Some(raw.into()),
        value_i64: Some(body.into()),
        value_f64: None,
        value_bool: None,
        value_str: None,
    }
}

fn known() -> MainIdentityTimeline {
    let mut value = MainIdentityTimeline::new();
    value
        .observe_packet(
            1,
            &[actor(1, 11, 11, "open"), actor(1, 22, 22, "open")],
            &[reference(1, 11, 22)],
        )
        .unwrap();
    value
}

fn resolution(timeline: &MainIdentityTimeline, packet: u32) -> IdentityResolution<'_> {
    timeline.resolve_before_packet(ActorReference { net_guid: 22 }, packet)
}

fn movement(packet: u32) -> MovementRecord {
    MovementRecord {
        time_ms: 0,
        packet_id: packet,
        character_net_guid: 22,
        pos_x: 1.25,
        pos_y: -2.5,
        pos_z: 3.75,
        yaw: 179.0,
        pitch: -89.0,
        vel_x: 9.0,
        vel_y: 8.0,
        vel_z: 7.0,
        timestamp: 2,
        movement_state: 0,
        move_type: 1,
    }
}

fn source(initial: u32, completion: u32) -> MovementProvenance {
    MovementProvenance {
        source: Some(MovementSource {
            initial_packet_id: initial,
            completion_packet_id: completion,
            channel_index: 91,
            actor_net_guid: 999,
            object_net_guid: Some(0),
            bunch_index: 8,
            block_index: 3,
            rpc_index: 2,
            rpc_handle: 17,
        }),
        array_ordinal: 4,
        update_ordinal: 2,
        update_index: 9,
        component_ordinal: 1,
        move_index: 3,
    }
}

fn records<'a>(
    moves: &'a [MovementRecord],
    sources: &'a [MovementProvenance],
) -> ReplayRecords<'a> {
    ReplayRecords {
        fields: &[],
        actors: &[],
        movement: moves,
        movement_provenance: sources,
        partials: &[],
    }
}

#[test]
fn qa_identity_strict_packet_boundary_retains_owned_evidence() {
    let mut timeline = MainIdentityTimeline::new();
    let mut field = reference(10, 11, 22);
    timeline
        .observe_packet(
            10,
            &[actor(10, 11, 11, "open"), actor(10, 22, 22, "open")],
            std::slice::from_ref(&field),
        )
        .unwrap();
    field.value_i64 = Some(0);
    field.raw_bits.as_mut().unwrap()[0] = 0;
    assert!(matches!(
        resolution(&timeline, 10),
        IdentityResolution::Unresolved(IdentityUnresolved::NoReference)
    ));
    let IdentityResolution::Resolved(body) = resolution(&timeline, 11) else {
        panic!("prior packet missing")
    };
    assert_eq!(body.declaration.value_i64, Some(22));
    assert_eq!(body.declaration.raw_bits.as_deref(), Some(&[44][..]));
    assert_eq!(body.declaration.time_ms, 6990);
    timeline
        .observe_packet(20, &[], &[reference(20, 11, 0)])
        .unwrap();
    assert!(matches!(
        resolution(&timeline, 20),
        IdentityResolution::Resolved(_)
    ));
    assert!(matches!(
        resolution(&timeline, 21),
        IdentityResolution::Unresolved(IdentityUnresolved::NoReference)
    ));
    assert!(matches!(
        resolution(&timeline, 11),
        IdentityResolution::Resolved(_)
    ));
    assert!(matches!(
        timeline.resolve_before_packet(ActorReference { net_guid: 0 }, 999),
        IdentityResolution::Null
    ));
}

#[test]
fn qa_identity_conflict_close_reopen_and_dormancy_are_distinct() {
    let mut timeline = known();
    timeline
        .observe_packet(2, &[actor(2, 33, 33, "open")], &[reference(2, 33, 22)])
        .unwrap();
    let IdentityResolution::Ambiguous { declarations } = resolution(&timeline, 3) else {
        panic!("conflict hidden")
    };
    assert_eq!(
        declarations
            .iter()
            .map(|row| row.actor_net_guid)
            .collect::<Vec<_>>(),
        [11, 33]
    );
    timeline
        .observe_packet(3, &[], &[reference(3, 33, 0)])
        .unwrap();
    assert!(matches!(
        resolution(&timeline, 4),
        IdentityResolution::Resolved(_)
    ));
    timeline
        .observe_packet(4, &[actor(4, 22, 22, "close")], &[])
        .unwrap();
    assert!(matches!(
        resolution(&timeline, 5),
        IdentityResolution::Unresolved(IdentityUnresolved::Lifecycle)
    ));
    timeline
        .observe_packet(5, &[actor(5, 22, 88, "open")], &[])
        .unwrap();
    assert!(matches!(
        resolution(&timeline, 6),
        IdentityResolution::Unresolved(IdentityUnresolved::PreviousLifetime)
    ));
    timeline
        .observe_packet(6, &[], &[reference(6, 11, 22)])
        .unwrap();
    timeline
        .observe_packet(7, &[actor(7, 22, 88, "dormant")], &[])
        .unwrap();
    timeline
        .observe_packet(8, &[actor(8, 22, 44, "open")], &[])
        .unwrap();
    let IdentityResolution::Resolved(body) = resolution(&timeline, 9) else {
        panic!("dormancy ended life")
    };
    assert_eq!(body.character_open.packet_id, 5);
    timeline
        .observe_packet(9, &[actor(9, 22, 44, "open")], &[])
        .unwrap();
    assert!(matches!(
        resolution(&timeline, 10),
        IdentityResolution::Unresolved(IdentityUnresolved::Lifecycle)
    ));
    assert!(matches!(
        resolution(&timeline, 9),
        IdentityResolution::Resolved(_)
    ));
}

#[test]
fn qa_movement_uses_initial_fragment_and_preserves_exact_rows() {
    let mut timeline = known();
    timeline
        .observe_packet(3, &[], &[reference(3, 11, 22)])
        .unwrap();
    let moves = [movement(4)];
    let sources = [source(2, 4)];
    let batch = records(&moves, &sources);
    timeline.observe_records(&batch).unwrap();
    let result = timeline.movement_identity(&batch, 0).unwrap();
    assert!(std::ptr::eq(result.movement, &moves[0]));
    assert!(std::ptr::eq(result.provenance, &sources[0]));
    assert_eq!(result.movement.pos_x.to_bits(), 1.25_f32.to_bits());
    assert_eq!(result.provenance.source.unwrap().object_net_guid, Some(0));
    assert!(matches!(
        result.identity,
        IdentityResolution::Unresolved(IdentityUnresolved::ConcurrentChange)
    ));
    let sources = [source(4, 4)];
    let batch = records(&moves, &sources);
    assert!(matches!(
        timeline.movement_identity(&batch, 0).unwrap().identity,
        IdentityResolution::Resolved(_)
    ));
}

#[test]
fn qa_identity_rejects_raw_and_schema_mutations_and_invalidates_atomically() {
    for variant in 0..5 {
        let mut field = reference(2, 11, 22);
        match variant {
            0 => field.compatible_checksum = Some(1413303949),
            1 => field.object_net_guid = Some(0),
            2 => field.raw_bits.as_mut().unwrap()[0] = 46,
            3 => {
                field.raw_bits.as_mut().unwrap().push(0);
                field.bit_count += 8;
            }
            _ => field.value_bool = Some(false),
        }
        assert!(spawned_character_observation(&field).is_err());
        let mut timeline = known();
        assert!(
            timeline
                .observe_packet(2, &[actor(2, 22, 22, "close")], &[field])
                .is_err()
        );
        assert!(matches!(
            resolution(&timeline, 2),
            IdentityResolution::Unresolved(IdentityUnresolved::InvalidTimeline)
        ));
        assert_eq!(
            timeline.observe_packet(3, &[], &[]),
            Err(IdentityError::InvalidTimeline)
        );
    }
}

#[test]
fn qa_movement_rejects_absent_misaligned_and_reversed_provenance() {
    let moves = [movement(3)];
    let valid = [source(2, 3)];
    let mut timeline = known();
    timeline.observe_records(&records(&moves, &valid)).unwrap();
    let mut absent = valid;
    absent[0].source = None;
    let bad = [source(4, 3)];
    let mismatch = [source(2, 2)];
    for sources in [&[][..], &absent[..], &bad[..], &mismatch[..]] {
        assert_eq!(
            timeline
                .movement_identity(&records(&moves, sources), 0)
                .unwrap_err(),
            IdentityError::MovementProvenance
        );
    }
    assert!(matches!(
        resolution(&timeline, 2),
        IdentityResolution::Resolved(_)
    ));
}

fn replay_with_partial_checkpoints() -> Vec<u8> {
    let packet = unfinished_partial_packet();
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    let frames = frame(9.0, &[], 0, &[&packet]);
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    for time in [5.0, 1.0] {
        data.extend(chunk(
            2,
            &checkpoint(0, &checkpoint_tables(&frame(time, &[], 0, &[&packet])), 0),
        ));
    }
    data
}

#[test]
fn qa_checkpoint_partial_ownership_and_separate_counters() {
    let data = replay_with_partial_checkpoints();
    let mut snapshots = Vec::new();
    let mut partials = Vec::new();
    let mut indices = Vec::new();
    let result = ReplayReader
        .read_with_options(
            &data,
            ReadOptions {
                checkpoints: true,
                ..Default::default()
            },
            |event| {
                match event {
                    ReplayEvent::Checkpoint(CheckpointEvent::Records {
                        checkpoint,
                        records,
                        ..
                    }) => {
                        if !records.partials.is_empty() {
                            indices.push(checkpoint.identity.checkpoint_index);
                            partials.extend_from_slice(records.partials);
                        }
                    }
                    ReplayEvent::Checkpoint(CheckpointEvent::Summary(summary)) => {
                        snapshots.push(summary.clone())
                    }
                    ReplayEvent::Records(records) => partials.extend_from_slice(records.partials),
                    _ => {}
                }
                Ok::<(), Infallible>(())
            },
        )
        .unwrap();
    drop(data);
    assert_eq!(indices, [0, 1]);
    assert_eq!(snapshots.len(), 2);
    for snapshot in snapshots {
        assert_eq!(snapshot.network.unfinished_partials, 1);
        assert_eq!(snapshot.partial_rows, 1);
        assert_eq!(
            snapshot.metadata.identity.checkpoint_id.as_ref(),
            "checkpoint0"
        );
    }
    assert_eq!(result.network.unfinished_partials, 1);
    assert_eq!(result.partial_rows, 1);
    assert_eq!(result.checkpoints.partial_rows, 2);
    assert_eq!(partials.len(), 3);
    assert!(
        partials
            .iter()
            .all(|row| row.raw_bits == [6] && row.source_packet_id == 0)
    );
    assert_eq!(
        partials.iter().map(|row| row.source).collect::<Vec<_>>(),
        ["checkpoint", "checkpoint", "main"]
    );
}

#[test]
fn qa_checkpoint_consumer_error_at_each_phase_prevents_later_work() {
    for stop in ["metadata", "records", "summary"] {
        let mut data = replay_with_partial_checkpoints();
        data.extend(chunk(1, &replay_data(&[0xff], 1)));
        let marker = Box::new(123_u32);
        let pointer = &*marker as *const u32;
        let mut marker = Some(marker);
        let mut saw_stop = false;
        let result = ReplayReader.read_with_options(
            &data,
            ReadOptions {
                checkpoints: true,
                ..Default::default()
            },
            |event| {
                assert!(!saw_stop, "callback after consumer rejection");
                let label = match event {
                    ReplayEvent::Checkpoint(CheckpointEvent::Metadata(_)) => "metadata",
                    ReplayEvent::Checkpoint(CheckpointEvent::Records { .. }) => "records",
                    ReplayEvent::Checkpoint(CheckpointEvent::Summary(_)) => "summary",
                    _ => "other",
                };
                if label == stop {
                    saw_stop = true;
                    Err(marker.take().unwrap())
                } else {
                    Ok(())
                }
            },
        );
        assert!(saw_stop);
        match result {
            Err(ReadError::Consumer(value)) => assert_eq!(&*value as *const u32, pointer),
            other => panic!("consumer cause lost: {other:?}"),
        }
    }
}
