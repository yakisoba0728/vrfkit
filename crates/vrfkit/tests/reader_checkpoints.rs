//! Opt-in snapshot contracts through real container, schema and packet paths.

mod common;

use std::convert::Infallible;
use std::error::Error;

use common::*;
use vrfkit::{
    CheckpointEvent, ReadError, ReadOptions, ReplayError, ReplayEvent, ReplayReader, SchemaError,
    SchemaLimits,
};

fn options() -> ReadOptions {
    ReadOptions::default().with_checkpoints(true)
}

fn preamble() -> Vec<u8> {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data
}

fn unresolved_packet() -> Vec<u8> {
    let mut payload = BitWriter::new();
    payload
        .int_packed(3)
        .extend_bits(&[false, true])
        .int_packed(7)
        .extend_bits(&[false; 7]);
    let mut bits = BitWriter::new();
    bits.extend_bits(&[true, true, false, false, true])
        .int_packed(2)
        .extend_bits(&[false, false, false, false])
        .bit(true)
        .int_packed(1)
        .serialized_int(payload.len() as u32, 2 * 1024 * 8)
        .extend_bits(&payload)
        .bit(true);
    pack(&bits)
}

/// Two GUIDs (literal and indexed), one sparse two-slot declaration, frames.
fn snapshot(path: &str, frames: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    add_int_packed(&mut body, 42);
    add_int_packed(&mut body, 0);
    body.push(1);
    add_fstring(&mut body, path);
    body.push(3);
    add_int_packed(&mut body, 43);
    add_int_packed(&mut body, 42);
    body.push(0);
    add_int_packed(&mut body, 0);
    body.push(5);
    add_u32(&mut body, 1);
    add_fstring(&mut body, path);
    add_int_packed(&mut body, 7);
    add_int_packed(&mut body, 2);
    body.push(0); // slot zero is not exported
    body.push(2); // preserve the exact nonzero exported flag
    add_int_packed(&mut body, 1);
    add_u32(&mut body, 0xdead_beef);
    body.push(0); // string FName
    add_fstring(&mut body, "Value");
    add_i32(&mut body, 1);
    let mut plain = Vec::new();
    add_u32(&mut plain, (20 + body.len() - 8) as u32);
    plain.extend([0; 12]);
    add_u32(&mut plain, 2);
    plain.extend(body);
    plain.extend(frames);
    plain
}

fn input_with_checkpoint(plain: &[u8]) -> Vec<u8> {
    let mut data = preamble();
    data.extend(chunk(2, &checkpoint(0, plain, 0)));
    data
}

#[test]
fn legacy_and_default_options_skip_even_malformed_snapshots() {
    let data = input_with_checkpoint(&[0xde, 0xad]);
    let mut legacy_events = Vec::new();
    let legacy = ReplayReader
        .read(&data, |event| {
            legacy_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    let mut default_events = Vec::new();
    let default = ReplayReader
        .read_with_options(&data, ReadOptions::default(), |event| {
            default_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(legacy_events, default_events);
    assert_eq!(format!("{legacy:?}"), format!("{default:?}"));
    assert_eq!(legacy.skipped_checkpoint_chunks, 1);
    assert_eq!(legacy.checkpoints.chunks, 0);
    assert!(
        ReplayReader
            .read_with_options(&data, options(), |_| Ok::<(), Infallible>(()))
            .is_err()
    );
}

#[test]
fn snapshots_preserve_declarations_raw_rows_order_and_independent_state() {
    let packet = unresolved_packet();
    let mut data = preamble();
    let frames = frame(9.0, &[], 0, &[&packet]);
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    for (path, time) in [("SnapshotA", 3.0), ("SnapshotB", 1.0)] {
        // Deliberately reuse the wire ID: the observed index disambiguates it.
        data.extend(chunk(
            2,
            &checkpoint(0, &snapshot(path, &frame(time, &[], 0, &[&packet])), 2),
        ));
    }
    let mut order = Vec::new();
    let mut fields = Vec::new();
    let mut summaries = Vec::new();
    let result = ReplayReader
        .read_with_options(&data, options(), |event| {
            match event {
                ReplayEvent::Metadata(_) => order.push("main-metadata"),
                ReplayEvent::Records(records) => {
                    order.push("main-records");
                    assert_eq!(records.fields[0].packet_id, 0);
                    assert_eq!(records.fields[0].time_ms, 9000);
                }
                ReplayEvent::Checkpoint(event) => match event {
                    CheckpointEvent::Metadata(metadata) => {
                        order.push("checkpoint-metadata");
                        assert_eq!(metadata.identity.checkpoint_id.as_ref(), "checkpoint0");
                        assert_eq!((metadata.time1, metadata.time2), (1000, 1000));
                        assert_eq!(metadata.trailing_bytes, 2);
                    }
                    CheckpointEvent::GuidEntry(entry) => {
                        order.push("guid");
                        assert_eq!(entry.flags, if entry.ordinal == 0 { 3 } else { 5 });
                        assert_eq!(entry.name_index, (entry.ordinal == 1).then_some(0));
                    }
                    CheckpointEvent::ExportGroup(group) => {
                        order.push("group");
                        assert_eq!((group.path_name_index, group.declared_slots), (7, 2));
                    }
                    CheckpointEvent::ExportField(field) => {
                        order.push("field-declaration");
                        assert_eq!((field.slot, field.handle), (1, 1));
                        assert_eq!(field.compatible_checksum, 0xdead_beef);
                        assert_eq!(field.exported_flag, 2);
                        assert_eq!(field.rendered_name, "Value_0");
                        assert_eq!(field.fname_number, Some(1));
                    }
                    CheckpointEvent::Records {
                        checkpoint,
                        records,
                        blocks,
                    } => {
                        order.push("checkpoint-records");
                        let index = checkpoint.identity.checkpoint_index;
                        fields.extend_from_slice(records.fields);
                        assert_eq!(records.fields.len(), 1);
                        assert_eq!(records.fields[0].packet_id, 0);
                        assert_eq!(
                            records.fields[0].time_ms,
                            if index == 0 { 3000 } else { 1000 }
                        );
                        assert_eq!(records.fields[0].raw_bits.as_deref(), Some(&[0x61][..]));
                        assert_eq!(
                            records.actors.len(),
                            1,
                            "channel state restarts per snapshot"
                        );
                        assert_eq!(blocks.len(), 1);
                        assert_eq!(blocks[0].field_row_start, u64::from(index));
                        assert_eq!(blocks[0].field_row_count, 1);
                        assert_eq!(blocks[0].block_index, 0);
                        assert_eq!(records.movement.len(), records.movement_provenance.len());
                    }
                    CheckpointEvent::Summary(summary) => {
                        order.push("checkpoint-summary");
                        summaries.push(summary.clone());
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    drop(data); // clones survive borrowed input and per-packet buffer reuse
    assert_eq!(
        order,
        [
            "main-metadata",
            "main-records",
            "checkpoint-metadata",
            "guid",
            "guid",
            "group",
            "field-declaration",
            "checkpoint-records",
            "checkpoint-summary",
            "checkpoint-metadata",
            "guid",
            "guid",
            "group",
            "field-declaration",
            "checkpoint-records",
            "checkpoint-summary"
        ]
    );
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].time_ms, 3000);
    assert_eq!(fields[1].time_ms, 1000);
    for (index, summary) in summaries.iter().enumerate() {
        assert_eq!(summary.metadata.identity.checkpoint_index, index as u32);
        assert_eq!(summary.net_guids.len(), 2);
        assert_eq!(summary.net_guids[0].net_guid, 42);
        assert_eq!(
            summary.net_guids[0].path,
            if index == 0 { "SnapshotA" } else { "SnapshotB" }
        );
        assert_eq!(summary.net_guids[1].outer_net_guid, Some(42));
        assert_eq!(
            (
                summary.tables.guid_count,
                summary.tables.group_count,
                summary.tables.exported_fields
            ),
            (2, 1, 1)
        );
        assert_eq!((summary.packets, summary.frames), (1, 1));
        assert_eq!(summary.field_rows, 1);
        assert_eq!(summary.network.unresolved_rpc_payloads_preserved, 1);
        assert_eq!(summary.decode_counters["fields_emitted"], 1);
    }
    assert_eq!(result.field_rows, 1);
    assert_eq!(result.actor_rows, 1);
    assert!(
        result.net_guids.is_empty(),
        "checkpoint GUIDs never enter main state"
    );
    assert_eq!(result.network.unresolved_rpc_payloads_preserved, 1);
    assert_eq!(result.skipped_checkpoint_chunks, 0);
    assert_eq!(result.checkpoints.chunks, 2);
    assert_eq!(result.checkpoints.field_rows, 2);
    assert_eq!(result.checkpoints.trailing_bytes, 4);
}

#[test]
fn empty_snapshot_and_empty_frame_each_complete_without_record_callbacks() {
    let mut data = input_with_checkpoint(&checkpoint_tables(&[]));
    data.extend(chunk(
        2,
        &checkpoint(1, &checkpoint_tables(&frame(2.0, &[], 0, &[])), 0),
    ));
    let mut summaries = Vec::new();
    let result = ReplayReader
        .read_with_options(&data, options(), |event| {
            if let ReplayEvent::Checkpoint(event) = event {
                match event {
                    CheckpointEvent::Metadata(_) => {}
                    CheckpointEvent::Summary(summary) => summaries.push(summary.clone()),
                    _ => panic!("empty checkpoint emitted declarations or records"),
                }
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!((summaries[0].frames, summaries[1].frames), (0, 1));
    assert_eq!(result.checkpoints.chunks, 2);
    assert_eq!(result.checkpoints.packets, 0);
    assert_eq!(result.checkpoints.field_rows, 0);
}

#[test]
fn file_input_callbacks_equal_slice_with_reused_payload_and_record_buffers() {
    let packet = unresolved_packet();
    let mut data = preamble();
    for (index, name) in [(0, "LongSnapshotName"), (1, "X"), (2, "AnotherSnapshot")] {
        data.extend(chunk(
            2,
            &checkpoint(
                index,
                &snapshot(name, &frame(index as f32, &[], 0, &[&packet, &packet])),
                index as usize,
            ),
        ));
    }
    let dir = scratch("reader-checkpoints", "buffer-reuse");
    let path = dir.join("input.vrf");
    std::fs::write(&path, &data).unwrap();
    let mut expected = Vec::new();
    let memory = ReplayReader
        .read_with_options(&data, options(), |event| {
            expected.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    let mut actual = Vec::new();
    let file = ReplayReader
        .read_path_with_options(&path, options(), |event| {
            actual.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(format!("{file:?}"), format!("{memory:?}"));
    assert_eq!(file.checkpoints.chunks, 3);
    assert_eq!(std::fs::read(path).unwrap(), data);
}

#[test]
fn checkpoint_and_main_partials_finish_in_their_own_namespaces() {
    let packet = unfinished_partial_packet();
    let frames = frame(1.0, &[], 0, &[&packet]);
    let mut data = preamble();
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    for index in 0..2 {
        data.extend(chunk(2, &checkpoint(index, &checkpoint_tables(&frames), 0)));
    }
    let mut rows = Vec::new();
    let result = ReplayReader
        .read_with_options(&data, options(), |event| {
            match event {
                ReplayEvent::Records(batch) => rows.extend_from_slice(batch.partials),
                ReplayEvent::Checkpoint(CheckpointEvent::Records { records, .. }) => {
                    rows.extend_from_slice(records.partials)
                }
                _ => {}
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(rows.len(), 3);
    let labels: Vec<_> = rows
        .iter()
        .map(|r| (r.source, r.checkpoint_id.as_deref()))
        .collect();
    assert_eq!(
        labels,
        [
            ("checkpoint", Some("checkpoint0")),
            ("checkpoint", Some("checkpoint1")),
            ("main", None)
        ]
    );
    assert!(
        rows.iter()
            .all(|r| r.source_packet_id == 0 && r.raw_bits == [6])
    );
    assert_eq!(result.partial_rows, 1);
    assert_eq!(result.checkpoints.partial_rows, 2);
    assert_eq!(result.network.unfinished_partials, 1);
}

#[test]
fn first_structural_failure_follows_file_order_even_with_main_prefetch() {
    let bad_checkpoint = chunk(2, &checkpoint(0, &[0xde, 0xad], 0));
    let bad_main = chunk(1, &replay_data(&[0xff], 1));
    let dir = scratch("reader-checkpoints", "error-order");
    let path = dir.join("input.vrf");
    for checkpoint_first in [true, false] {
        let mut data = preamble();
        for bytes in if checkpoint_first {
            [&bad_checkpoint, &bad_main]
        } else {
            [&bad_main, &bad_checkpoint]
        } {
            data.extend(bytes);
        }
        std::fs::write(&path, &data).unwrap();
        let memory = ReplayReader
            .read_with_options(&data, options(), |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        let file = ReplayReader
            .read_path_with_options(&path, options(), |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        assert_eq!(memory.to_string(), file.to_string());
        assert!(
            memory.to_string().starts_with(if checkpoint_first {
                "checkpoint checkpoint0:"
            } else {
                "frame error:"
            }),
            "{memory}"
        );
    }
}

/// The bad chunk above decompresses and fails later in the walk; this one
/// fails inside decompression, which the file path runs on its prefetch
/// thread. That thread's error must arrive, not "decompression thread stopped".
#[test]
fn prefetch_decompression_failure_matches_the_slice_path() {
    let mut body = Vec::new();
    add_u32(&mut body, 0); // time1
    add_u32(&mut body, 60_000); // time2
    add_i32(&mut body, 8); // SizeInBytes: longer than the one byte present
    add_i32(&mut body, 8); // MemorySizeInBytes
    body.push(0xff);
    let mut data = preamble();
    data.extend(chunk(1, &body));
    let dir = scratch("reader-checkpoints", "prefetch-decompress");
    let path = dir.join("input.vrf");
    std::fs::write(&path, &data).unwrap();
    for options in [ReadOptions::default(), options()] {
        let memory = ReplayReader
            .read_with_options(&data, options, |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        let file = ReplayReader
            .read_path_with_options(&path, options, |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        assert_eq!(memory.to_string(), file.to_string());
        assert!(
            memory.to_string().contains("uncompressed replay data"),
            "{memory}"
        );
        assert!(!file.to_string().contains("thread stopped"), "{file}");
    }
}

/// A frame or container failure inside a snapshot carries the snapshot's
/// identity; the same frame failure in the main stream stays bare.
#[test]
fn snapshot_structural_failures_carry_checkpoint_identity() {
    let packet = unresolved_packet();
    let mut frames = frame(1.0, &[], 0, &[&packet]);
    frames.push(0xff);
    let mut unparsed = preamble();
    unparsed.extend(chunk(2, &[1, 0]));
    let mut main = preamble();
    main.extend(chunk(1, &replay_data(&frames, frames.len())));
    let dir = scratch("reader-checkpoints", "snapshot-identity");
    let path = dir.join("input.vrf");
    for (data, id, prefix) in [
        (
            input_with_checkpoint(&checkpoint_tables(&frames)),
            Some("checkpoint0"),
            "checkpoint checkpoint0: frame error:",
        ),
        (unparsed, None, "checkpoint #0: container error:"),
    ] {
        std::fs::write(&path, &data).unwrap();
        let memory = ReplayReader
            .read_with_options(&data, options(), |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        let file = ReplayReader
            .read_path_with_options(&path, options(), |_| Ok::<(), Infallible>(()))
            .unwrap_err();
        assert_eq!(memory.to_string(), file.to_string());
        assert!(memory.to_string().starts_with(prefix), "{memory}");
        let ReadError::Replay(ReplayError::Checkpoint {
            checkpoint_index,
            checkpoint_id,
            error,
        }) = memory
        else {
            panic!("snapshot failure lost its identity: {memory:?}");
        };
        assert_eq!(checkpoint_index, 0);
        assert_eq!(checkpoint_id.as_deref(), id);
        assert!(
            matches!(
                (id, &*error),
                (Some(_), ReplayError::Frame(_)) | (None, ReplayError::Container(_))
            ),
            "{error:?}"
        );
    }
    let bare = ReplayReader
        .read_with_options(&main, options(), |_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(
        matches!(bare, ReadError::Replay(ReplayError::Frame(_))),
        "{bare:?}"
    );
}

#[test]
fn checkpoint_consumer_error_keeps_identity_and_outranks_later_frame_failure() {
    let packet = unresolved_packet();
    let mut frames = frame(1.0, &[], 0, &[&packet]);
    frames.push(0xff);
    let mut data = input_with_checkpoint(&checkpoint_tables(&frames));
    data.extend(chunk(2, &checkpoint(1, &checkpoint_tables(&[]), 0)));
    let marker = Box::new(42);
    let pointer = &*marker as *const i32;
    let mut marker = Some(marker);
    let mut calls = 0;
    let result = ReplayReader.read_with_options(&data, options(), |event| {
        calls += 1;
        if matches!(
            event,
            ReplayEvent::Checkpoint(CheckpointEvent::Records { .. })
        ) {
            Err(marker.take().unwrap())
        } else {
            Ok(())
        }
    });
    match result {
        Err(ReadError::Consumer(error)) => assert_eq!(&*error as *const i32, pointer),
        other => panic!("callback error was replaced: {other:?}"),
    }
    assert_eq!(
        calls, 3,
        "main metadata, checkpoint metadata, rejected records only"
    );
}

#[test]
fn declaration_callback_error_stops_before_later_malformed_table_bytes() {
    let mut plain = snapshot("Example", &[]);
    plain.truncate(plain.len() - 3);
    let data = input_with_checkpoint(&plain);
    let mut calls = 0;
    let result = ReplayReader.read_with_options(&data, options(), |event| {
        calls += 1;
        if matches!(
            event,
            ReplayEvent::Checkpoint(CheckpointEvent::GuidEntry(_))
        ) {
            Err("declaration-stop")
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        result,
        Err(ReadError::Consumer("declaration-stop"))
    ));
    assert_eq!(calls, 3);
}

#[test]
fn checkpoint_summary_callback_can_stop_without_a_successful_replay_summary() {
    let mut data = input_with_checkpoint(&checkpoint_tables(&[]));
    data.extend(chunk(2, &checkpoint(1, &checkpoint_tables(&[]), 0)));
    let mut calls = 0;
    let result = ReplayReader.read_with_options(&data, options(), |event| {
        calls += 1;
        if matches!(event, ReplayEvent::Checkpoint(CheckpointEvent::Summary(_))) {
            Err("summary-stop")
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(ReadError::Consumer("summary-stop"))));
    assert_eq!(calls, 3);
}

#[test]
fn checkpoint_schema_limits_are_independent_and_keep_the_typed_cause() {
    let mut data = input_with_checkpoint(&snapshot("First", &[]));
    data.extend(chunk(2, &checkpoint(1, &snapshot("Second", &[]), 0)));
    let limits = ReadOptions::default()
        .with_schema_limits(
            SchemaLimits::default()
                .with_max_groups(1)
                .with_max_field_slots(2),
        )
        .with_checkpoints(true);
    assert_eq!(
        ReplayReader
            .read_with_options(&data, limits, |_| Ok::<(), Infallible>(()))
            .unwrap()
            .checkpoints
            .chunks,
        2
    );
    let error = ReplayReader
        .read_with_options(
            &data,
            limits.with_schema_limits(limits.schema_limits.with_max_field_slots(1)),
            |_| Ok::<(), Infallible>(()),
        )
        .unwrap_err();
    let mut source: Option<&(dyn Error + 'static)> = Some(&error);
    let mut found = None;
    while let Some(current) = source {
        if let Some(schema) = current.downcast_ref::<SchemaError>() {
            found = Some(schema);
            break;
        }
        source = current.source();
    }
    assert_eq!(
        found,
        Some(&SchemaError::ResourceLimit {
            resource: "field slots",
            requested: 2,
            limit: 1
        })
    );
}

#[test]
fn late_checkpoint_failure_invalidates_earlier_snapshot_completion() {
    let mut data = input_with_checkpoint(&checkpoint_tables(&[]));
    data.extend(chunk(2, &[0xff]));
    let mut completed = 0;
    let result = ReplayReader.read_with_options(&data, options(), |event| {
        completed += usize::from(matches!(
            event,
            ReplayEvent::Checkpoint(CheckpointEvent::Summary(_))
        ));
        Ok::<(), Infallible>(())
    });
    assert!(matches!(result, Err(ReadError::Replay(_))));
    assert_eq!(completed, 1, "earlier completion is only provisional");
}
