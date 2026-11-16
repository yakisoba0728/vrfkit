//! Public API regressions through the real container, frame and packet paths.

mod common;

use std::convert::Infallible;
use std::error::Error;

use common::*;
use vrfkit::{ReadError, ReplayEvent, ReplayReader, SchemaError, SchemaLimits, ValidationVerdict};

fn replay(frames: &[u8]) -> Vec<u8> {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data.extend(chunk(1, &replay_data(frames, frames.len())));
    data
}

/// A static actor with an unresolved ClassNetCache block. Seven zero wire
/// bits become 0x61: the 12.10 tail XOR is 0xe5 and seed is 7 ^ actor 3.
fn unresolved_packet() -> Vec<u8> {
    let mut payload = BitWriter::new();
    payload
        .int_packed(3)
        .extend_bits(&[false, true]) // ClassNetCache, actor
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

fn event_chunk() -> Vec<u8> {
    let mut event = Vec::new();
    add_fstring(&mut event, "synthetic-event");
    add_fstring(&mut event, "unknown-test-group");
    add_fstring(&mut event, "");
    add_u32(&mut event, 1500);
    add_u32(&mut event, 1501);
    add_i32(&mut event, 3);
    event.extend([0x12, 0x34, 0x56, 0xff]); // final byte is outside declared payload
    chunk(3, &event)
}

fn schema_cause<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a SchemaError> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(schema) = error.downcast_ref::<SchemaError>() {
            return Some(schema);
        }
        current = error.source();
    }
    None
}

#[test]
fn schema_failures_keep_the_typed_cause_and_existing_display() {
    // One export refers to a group that this fresh replay has never declared.
    let mut frames = Vec::new();
    add_i32(&mut frames, 0);
    add_f32(&mut frames, 1.0);
    frames.extend([2, 14, 0]); // one export, path-name index 7, existing group
    let error = ReplayReader::new()
        .read(&replay(&frames), |_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "frame error: schema error during frame parsing: net-field export references unknown path name index 7"
    );
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::UnknownPathIndex { index: 7 })
    );
}

#[test]
fn records_and_native_events_preserve_raw_data_and_file_order() {
    let packet = unresolved_packet();
    let mut data = replay(&frame(1.25, &[], 0, &[&packet]));
    data.extend(event_chunk());
    // This deliberately invalid snapshot must be reported as skipped, not
    // silently counted as a successfully decoded checkpoint.
    data.extend(chunk(2, &[0xde, 0xad]));
    data.extend(chunk(99, &[0xbe, 0xef]));
    let mut order = Vec::new();
    let mut saved = Vec::new();
    let summary = ReplayReader::new()
        .read(&data, |event| {
            match event {
                ReplayEvent::Metadata(metadata) => {
                    order.push("metadata");
                    assert_eq!(
                        metadata.header.replay_version.branch,
                        "++Ares-Core+release-12.10"
                    );
                }
                ReplayEvent::Records(batch) => {
                    order.push("records");
                    assert!(!batch.is_empty());
                    saved.extend_from_slice(batch.fields);
                    assert_eq!(batch.actors.len(), 1);
                    assert_eq!(batch.actors[0].actor_net_guid, 3);
                }
                ReplayEvent::ContainerEvent(event) => {
                    order.push("event");
                    assert_eq!(event.payload, [0x12, 0x34, 0x56]);
                    assert_eq!(event.trailing_bytes, 1);
                    assert_eq!(event.group, "unknown-test-group");
                }
                _ => unreachable!(),
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    drop(data);
    assert_eq!(order, ["metadata", "records", "event"]);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].time_ms, 1250);
    assert_eq!(saved[0].packet_id, 0);
    assert_eq!(saved[0].bit_count, 7);
    assert_eq!(saved[0].raw_bits.as_deref(), Some(&[0x61][..]));
    assert!(saved[0].value_i64.is_none());
    assert!(saved[0].value_f64.is_none());
    assert!(saved[0].value_bool.is_none());
    assert!(saved[0].value_str.is_none());
    assert_eq!(summary.field_rows, 1);
    assert_eq!(summary.actor_rows, 1);
    assert_eq!((summary.packets, summary.frames), (1, 1));
    assert_eq!(summary.event_chunks, 1);
    assert_eq!(summary.event_trailing_bytes, 1);
    assert_eq!(summary.skipped_checkpoint_chunks, 1);
    assert_eq!(summary.unknown_chunks, 1);
    assert_eq!(summary.network.unresolved_rpc_payloads_preserved, 1);
    assert_eq!(summary.decode_counters["fields_emitted"], 1);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
}

#[test]
fn end_of_stream_partials_are_delivered_once_with_original_identity() {
    let packet = unfinished_partial_packet();
    let data = replay(&frame(1.0, &[], 0, &[&packet]));
    let mut partials = Vec::new();
    let summary = ReplayReader::new()
        .read(&data, |event| {
            if let ReplayEvent::Records(batch) = event {
                partials.extend_from_slice(batch.partials);
            }
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(partials.len(), 1);
    assert_eq!(partials[0].source, "main");
    assert_eq!(partials[0].source_packet_id, 0);
    assert_eq!(partials[0].channel_index, 2);
    assert_eq!(partials[0].bit_count, 8);
    assert_eq!(partials[0].raw_bits, [6]); // packed actor GUID 3
    assert_eq!(summary.partial_rows, 1);
    assert_eq!(summary.network.unfinished_partials, 1);
    assert_eq!(summary.block_verdict(), ValidationVerdict::NoContentBlocks);
}

#[test]
fn consumer_error_is_preserved_even_when_later_frame_data_is_malformed() {
    let packet = unresolved_packet();
    let mut frames = frame(1.0, &[], 0, &[&packet]);
    frames.push(0xff); // a later frame cannot read its header
    let mut data = replay(&frames);
    data.extend(event_chunk());
    let marker = Box::new(17);
    let marker_ptr = &*marker as *const i32;
    let mut marker = Some(marker);
    let mut calls = 0;
    let result = ReplayReader::new().read(&data, |event| {
        calls += 1;
        if matches!(event, ReplayEvent::Records(_)) {
            Err(marker.take().unwrap())
        } else {
            Ok(())
        }
    });
    match result {
        Err(ReadError::Consumer(error)) => assert_eq!(&*error as *const i32, marker_ptr),
        other => panic!("original callback error was not preserved: {other:?}"),
    }
    assert_eq!(calls, 2, "no events after metadata and the rejected batch");
}

#[test]
fn a_later_container_failure_does_not_return_a_success_summary() {
    let packet = unresolved_packet();
    let mut data = replay(&frame(1.0, &[], 0, &[&packet]));
    data.extend([1, 0, 0]); // truncated chunk header
    let mut rows = 0;
    let result = ReplayReader::new().read(&data, |event| {
        if let ReplayEvent::Records(batch) = event {
            rows += batch.fields.len();
        }
        Ok::<(), Infallible>(())
    });
    assert!(matches!(result, Err(ReadError::Replay(_))));
    assert_eq!(rows, 1, "the earlier provisional row was already delivered");
}

#[test]
fn metadata_can_describe_an_unsupported_branch_but_full_reads_refuse_it() {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(
        0,
        &header_payload_for_branch("future", 0, &[3, 0, 0, 0, 49, 56, 0]),
    ));
    let reader = ReplayReader::new();
    let metadata = reader.read_metadata(&data).unwrap();
    assert_eq!(metadata.header.replay_version.branch, "future");
    let mut callbacks = 0;
    let result = reader.read(&data, |_| {
        callbacks += 1;
        Ok::<(), Infallible>(())
    });
    assert!(matches!(result, Err(ReadError::Replay(_))));
    assert_eq!(callbacks, 0);
}

#[test]
fn repeated_file_and_memory_reads_start_with_independent_state() {
    let packet = unresolved_packet();
    let data = replay(&frame(2.0, &[(13, 2)], 3, &[&packet]));
    let scratch = scratch("reader-api", "repeat");
    let input = scratch.join("synthetic.vrf");
    std::fs::write(&input, &data).unwrap();
    let reader = ReplayReader::new();
    let first = reader
        .read_path(&input, |_| Ok::<(), Infallible>(()))
        .unwrap();
    let second = reader.read(&data, |_| Ok::<(), Infallible>(())).unwrap();
    assert_eq!(first.field_rows, 1);
    assert_eq!(first.field_rows, second.field_rows);
    assert_eq!(first.network.counters(), second.network.counters());
    assert_eq!(first.decode_counters, second.decode_counters);
    assert_eq!(first.frame_skips.external_data_bytes, 2);
    assert_eq!(first.frame_skips.game_specific_bytes, 3);
    assert_eq!(
        std::fs::read(&input).unwrap(),
        data,
        "input stays unchanged"
    );
}

#[test]
fn metadata_callback_can_stop_without_delivering_records() {
    let packet = unresolved_packet();
    let data = replay(&frame(1.0, &[], 0, &[&packet]));
    let mut callbacks = 0;
    let result = ReplayReader::new().read(&data, |_| {
        callbacks += 1;
        Err("stop")
    });
    assert!(matches!(result, Err(ReadError::Consumer("stop"))));
    assert_eq!(callbacks, 1);
}

/// Replace the fixture's one-byte empty export list after level/time with one
/// complete declaration. The rest still has no GUID exports and normal packets.
fn declared_frame(index: u32, path: &str, slots: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut exports = BitWriter::new();
    exports
        .int_packed(1)
        .int_packed(index)
        .int_packed(1)
        .fstring(path)
        .int_packed(slots)
        .u8(0);
    let mut frames = frame(1.0, &[], 0, packets);
    frames.splice(8..9, pack(&exports));
    frames
}

#[test]
fn group_limits_accumulate_across_chunks_and_reset_for_each_read() {
    let first = replay(&declared_frame(1, "First", 4, &[]));
    let mut both = first.clone();
    both.extend(event_chunk());
    let second = declared_frame(2, "Second", 4, &[]);
    both.extend(chunk(1, &replay_data(&second, second.len())));
    both.extend(event_chunk());
    let limits = SchemaLimits {
        max_groups: 1,
        max_field_slots: 8,
    };
    let reader = ReplayReader; // existing unit-struct construction stays valid
    let mut order = Vec::new();
    let error = reader
        .read_with_schema_limits(&both, limits, |event| {
            order.push(match event {
                ReplayEvent::Metadata(_) => "metadata",
                ReplayEvent::ContainerEvent(_) => "event",
                _ => panic!("schema-only frames emitted records"),
            });
            Ok::<(), Infallible>(())
        })
        .unwrap_err();
    assert_eq!(order, ["metadata", "event"]);
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "export groups",
            requested: 2,
            limit: 1,
        })
    );

    let accept = |_: ReplayEvent<'_>| Ok::<(), Infallible>(());
    assert_eq!(reader.read(&both, accept).unwrap().frames, 2);
    assert_eq!(
        reader
            .read_with_schema_limits(&first, limits, accept)
            .unwrap()
            .frames,
        1
    );
    assert_eq!(
        reader
            .read_with_schema_limits(
                &both,
                SchemaLimits {
                    max_groups: 2,
                    ..limits
                },
                accept,
            )
            .unwrap()
            .frames,
        2
    );
}

#[test]
fn slot_limits_refuse_a_declaration_before_its_truncated_field_flag() {
    let mut frames = BitWriter::new();
    frames
        .i32(0)
        .f32(1.0)
        .int_packed(1)
        .int_packed(1)
        .int_packed(1)
        .fstring("TooLarge")
        .int_packed(8);
    // The declaration ends here: no isFieldExported byte or frame tail.
    let data = replay(&pack(&frames));
    let mut callbacks = 0;
    let error = ReplayReader::new()
        .read_with_schema_limits(
            &data,
            SchemaLimits {
                max_groups: 1,
                max_field_slots: 7,
            },
            |_| {
                callbacks += 1;
                Ok::<(), Infallible>(())
            },
        )
        .unwrap_err();
    assert_eq!(callbacks, 1, "metadata precedes the refused declaration");
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "field slots",
            requested: 8,
            limit: 7,
        })
    );
    let default_error = ReplayReader::new()
        .read(&data, |_| Ok::<(), Infallible>(()))
        .unwrap_err();
    assert!(matches!(
        schema_cause(&default_error),
        Some(SchemaError::Bitio(_))
    ));
}

#[test]
fn zero_group_limit_allows_no_declarations_even_when_the_group_is_empty() {
    let reader = ReplayReader::new();
    let limits = SchemaLimits {
        max_groups: 0,
        max_field_slots: 0,
    };
    let accept = |_: ReplayEvent<'_>| Ok::<(), Infallible>(());
    assert_eq!(
        reader
            .read_with_schema_limits(&replay(&frame(1.0, &[], 0, &[])), limits, accept)
            .unwrap()
            .frames,
        1
    );
    let error = reader
        .read_with_schema_limits(&replay(&declared_frame(1, "Empty", 0, &[])), limits, accept)
        .unwrap_err();
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "export groups",
            requested: 1,
            limit: 0,
        })
    );
}

#[test]
fn zero_slot_limit_permits_empty_groups_but_refuses_their_growth() {
    let reader = ReplayReader::new();
    let limits = SchemaLimits {
        max_groups: 1,
        max_field_slots: 0,
    };
    let mut frames = declared_frame(1, "Empty", 0, &[]);
    let accept = |_: ReplayEvent<'_>| Ok::<(), Infallible>(());
    assert_eq!(
        reader
            .read_with_schema_limits(&replay(&frames), limits, accept)
            .unwrap()
            .frames,
        1
    );
    frames.extend(declared_frame(1, "Empty", 1, &[]));
    let error = reader
        .read_with_schema_limits(&replay(&frames), limits, accept)
        .unwrap_err();
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "field slots",
            requested: 1,
            limit: 0,
        })
    );
}

#[test]
fn explicit_default_limits_keep_records_native_events_and_counters() {
    let packet = unresolved_packet();
    let mut data = replay(&declared_frame(1, "Unused", 4, &[&packet]));
    data.extend(event_chunk());
    data.extend(chunk(2, &[0xde, 0xad])); // both routes still count skipped snapshots
    let reader = ReplayReader::new();
    let mut default_events = Vec::new();
    let first = reader
        .read(&data, |event| {
            default_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    let mut explicit_events = Vec::new();
    let second = reader
        .read_with_schema_limits(&data, SchemaLimits::default(), |event| {
            explicit_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(default_events, explicit_events);
    assert_eq!(first.field_rows, 1);
    assert_eq!(first.skipped_checkpoint_chunks, 1);
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
}

#[test]
fn custom_file_limits_match_memory_reads_and_leave_the_input_unchanged() {
    let packet = unresolved_packet();
    let data = replay(&declared_frame(1, "Unused", 4, &[&packet]));
    let dir = scratch("reader-api", "limits");
    let path = dir.join("synthetic.vrf");
    std::fs::write(&path, &data).unwrap();
    let reader = ReplayReader::new();
    let limits = SchemaLimits {
        max_groups: 1,
        max_field_slots: 3,
    };
    let accept = |_: ReplayEvent<'_>| Ok::<(), Infallible>(());
    let file_error = reader
        .read_path_with_schema_limits(&path, limits, accept)
        .unwrap_err();
    let memory_error = reader
        .read_with_schema_limits(&data, limits, accept)
        .unwrap_err();
    assert_eq!(schema_cause(&file_error), schema_cause(&memory_error));
    assert_eq!(
        schema_cause(&file_error),
        Some(&SchemaError::ResourceLimit {
            resource: "field slots",
            requested: 4,
            limit: 3,
        })
    );
    let exact = SchemaLimits {
        max_field_slots: 4,
        ..limits
    };
    let file = reader
        .read_path_with_schema_limits(&path, exact, accept)
        .unwrap();
    let memory = reader
        .read_with_schema_limits(&data, exact, accept)
        .unwrap();
    assert_eq!(file.field_rows, 1);
    assert_eq!(format!("{file:?}"), format!("{memory:?}"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
}

#[test]
fn consumer_error_outranks_a_later_schema_refusal_under_custom_limits() {
    let packet = unresolved_packet();
    let mut frames = declared_frame(1, "First", 4, &[&packet]);
    frames.extend(declared_frame(2, "Second", 4, &[]));
    let data = replay(&frames);
    let marker = Box::new(17);
    let marker_ptr = &*marker as *const i32;
    let mut marker = Some(marker);
    let mut calls = 0;
    let result = ReplayReader::new().read_with_schema_limits(
        &data,
        SchemaLimits {
            max_groups: 1,
            max_field_slots: 8,
        },
        |event| {
            calls += 1;
            if matches!(event, ReplayEvent::Records(_)) {
                Err(marker.take().unwrap())
            } else {
                Ok(())
            }
        },
    );
    match result {
        Err(ReadError::Consumer(error)) => assert_eq!(&*error as *const i32, marker_ptr),
        other => panic!("original callback error was not preserved: {other:?}"),
    }
    assert_eq!(calls, 2);
}
