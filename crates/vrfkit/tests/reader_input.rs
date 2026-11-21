//! File input and slice input must deliver the same provisional records.

mod common;

use std::convert::Infallible;
use std::fs;
use std::io::{Seek, SeekFrom, Write};

use common::*;
use vrfkit::{ReadError, ReplayEvent, ReplayReader};

fn preamble() -> Vec<u8> {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data
}

fn event() -> Vec<u8> {
    let mut bytes = Vec::new();
    add_fstring(&mut bytes, "synthetic");
    add_fstring(&mut bytes, "unknown");
    add_fstring(&mut bytes, "");
    add_u32(&mut bytes, 1);
    add_u32(&mut bytes, 2);
    add_i32(&mut bytes, 3);
    bytes.extend([0x12, 0x34, 0x56, 0xff]);
    chunk(3, &bytes)
}

fn mixed_replay() -> Vec<u8> {
    let mut data = preamble();
    data.extend(event());
    for time in [1.0, 2.0] {
        let packet = unfinished_partial_packet();
        let frames = frame(time, &[], 0, &[&packet]);
        data.extend(chunk(1, &replay_data(&frames, frames.len())));
        data.extend(chunk(99, &[7; 123]));
        data.extend(chunk(2, &[8; 17]));
        data.extend(chunk(0, &header_payload()));
        data.extend(event());
    }
    data
}

#[test]
fn file_callbacks_and_summary_equal_slice_input_in_wire_order() {
    let data = mixed_replay();
    let dir = scratch("reader-input", "mixed");
    let path = dir.join("replay.vrf");
    fs::write(&path, &data).unwrap();
    let mut memory_events = Vec::new();
    let memory = ReplayReader
        .read(&data, |event| {
            memory_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    let mut file_events = Vec::new();
    let file = ReplayReader
        .read_path(&path, |event| {
            file_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(memory_events, file_events);
    assert_eq!(format!("{memory:?}"), format!("{file:?}"));
    assert_eq!(file.event_chunks, 3);
    assert_eq!(file.event_trailing_bytes, 3);
    assert_eq!(file.skipped_checkpoint_chunks, 2);
    assert_eq!(file.unknown_chunks, 2);
    assert!(file.partial_rows > 0);
    assert_eq!(fs::read(path).unwrap(), data);
}

#[test]
fn every_truncated_chunk_preserves_errors_and_prior_callbacks() {
    let data = mixed_replay();
    let start = preamble().len();
    let dir = scratch("reader-input", "truncation");
    let path = dir.join("replay.vrf");
    for end in start..data.len() {
        let data = &data[..end];
        fs::write(&path, data).unwrap();
        let mut expected_events = Vec::new();
        let expected = ReplayReader.read(data, |event| {
            expected_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        });
        let mut actual_events = Vec::new();
        let actual = ReplayReader.read_path(&path, |event| {
            actual_events.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        });
        assert_eq!(
            format!("{actual:?}"),
            format!("{expected:?}"),
            "prefix {end}"
        );
        assert_eq!(actual_events, expected_events, "prefix {end}");
    }
}

#[test]
fn skipped_payloads_still_refuse_negative_or_out_of_file_lengths() {
    let dir = scratch("reader-input", "skipped-bounds");
    let path = dir.join("replay.vrf");
    for kind in [0, 1, 2, 3, 99] {
        for size in [-1, i32::MAX] {
            let mut data = preamble();
            data.extend(event());
            add_u32(&mut data, kind);
            add_i32(&mut data, size);
            fs::write(&path, &data).unwrap();
            let expected = ReplayReader
                .read(&data, |_| Ok::<(), Infallible>(()))
                .unwrap_err();
            let mut events = 0;
            let actual = ReplayReader
                .read_path(&path, |e| {
                    events += usize::from(matches!(e, ReplayEvent::ContainerEvent(_)));
                    Ok::<(), Infallible>(())
                })
                .unwrap_err();
            assert_eq!(actual.to_string(), expected.to_string());
            assert_eq!(events, 1, "earlier callback remains provisional");
        }
    }
}

#[test]
fn stopping_at_an_event_preserves_original_error_over_a_later_failure() {
    let mut data = preamble();
    data.extend(event());
    add_u32(&mut data, 1);
    add_i32(&mut data, -1);
    let dir = scratch("reader-input", "stop");
    let path = dir.join("replay.vrf");
    fs::write(&path, data).unwrap();
    let marker = Box::new(42);
    let marker_ptr = &*marker as *const i32;
    let mut marker = Some(marker);
    let mut calls = 0;
    let result = ReplayReader.read_path(&path, |event| {
        calls += 1;
        if matches!(event, ReplayEvent::ContainerEvent(_)) {
            Err(marker.take().unwrap())
        } else {
            Ok(())
        }
    });
    match result {
        Err(ReadError::Consumer(error)) => assert_eq!(&*error as *const i32, marker_ptr),
        other => panic!("consumer error replaced: {other:?}"),
    }
    assert_eq!(calls, 2);
}

#[test]
fn large_sparse_skipped_chunks_are_counted_without_decoding() {
    let dir = scratch("reader-input", "sparse");
    let path = dir.join("replay.vrf");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&preamble()).unwrap();
    for kind in [2_u32, 99] {
        file.write_all(&kind.to_le_bytes()).unwrap();
        file.write_all(&(32_i32 * 1024 * 1024).to_le_bytes())
            .unwrap();
        file.seek(SeekFrom::Current(32 * 1024 * 1024)).unwrap();
    }
    file.write_all(&event()).unwrap();
    drop(file);
    let summary = ReplayReader
        .read_path(&path, |_| Ok::<(), Infallible>(()))
        .unwrap();
    assert_eq!(summary.skipped_checkpoint_chunks, 1);
    assert_eq!(summary.unknown_chunks, 1);
    assert_eq!(summary.event_chunks, 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn metadata_stop_and_missing_path_keep_existing_error_categories() {
    let dir = scratch("reader-input", "metadata-stop");
    let path = dir.join("replay.vrf");
    let mut data = preamble();
    data.extend([1, 2, 3]);
    fs::write(&path, data).unwrap();
    let mut calls = 0;
    assert!(matches!(
        ReplayReader.read_path(&path, |_| {
            calls += 1;
            Err("stop")
        }),
        Err(ReadError::Consumer("stop"))
    ));
    assert_eq!(calls, 1);
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        ReplayReader.read_path(path, |_| Ok::<(), Infallible>(())),
        Err(ReadError::Replay(vrfkit::ReplayError::Io(_)))
    ));
}
