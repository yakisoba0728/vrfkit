//! Explicit nonseekable input limits through the existing Reader decoder.

mod common;

use std::cell::Cell;
use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::rc::Rc;
use std::sync::Arc;

use common::*;
use vrfkit::{
    CheckpointEvent, ReadError, ReadOptions, ReplayError, ReplayEvent, ReplayReader, SchemaError,
    SchemaLimits,
};

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

fn main_input() -> Vec<u8> {
    let mut data = preamble();
    let frames = frame(9.0, &[], 0, &[&unresolved_packet()]);
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    data
}

fn checkpoint_input() -> Vec<u8> {
    let mut data = main_input();
    for time in [3.0, 1.0] {
        // Repeated wire ID and reversed times make file-order comparisons real.
        let frames = frame(time, &[], 0, &[&unresolved_packet()]);
        data.extend(chunk(2, &checkpoint(7, &checkpoint_tables(&frames), 2)));
    }
    data
}

/// Deliberately implements only Read, with shared evidence of the EOF probe.
struct ObservedRead<'a> {
    remaining: &'a [u8],
    bytes_read: usize,
    calls: usize,
    short_read: usize,
    interrupt: bool,
    eof: Rc<Cell<bool>>,
    failure: Option<io::Error>,
    last_request: usize,
}

impl<'a> ObservedRead<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            remaining: data,
            bytes_read: 0,
            calls: 0,
            short_read: usize::MAX,
            interrupt: false,
            eof: Rc::new(Cell::new(false)),
            failure: None,
            last_request: 0,
        }
    }
}

impl Read for ObservedRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(!buffer.is_empty(), "an empty read cannot establish EOF");
        assert!(!self.eof.get(), "do not poll again after the first EOF");
        self.calls += 1;
        self.last_request = buffer.len();
        if self.interrupt && self.calls % 2 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining.is_empty() {
            if let Some(error) = self.failure.take() {
                return Err(error);
            }
            self.eof.set(true);
            return Ok(0);
        }
        let count = buffer.len().min(self.short_read).min(self.remaining.len());
        buffer[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        self.bytes_read += count;
        Ok(count)
    }
}

#[test]
fn exact_limit_requires_eof_before_callbacks_and_matches_slice() {
    for target in [0, 8192, 16384, 16385, 32768] {
        let mut data = main_input();
        if target != 0 {
            data.extend(chunk(99, &vec![0; target - data.len() - 8]));
            assert_eq!(data.len(), target);
        }
        let mut expected = Vec::new();
        let slice = ReplayReader
            .read(&data, |event| {
                expected.push(format!("{event:?}"));
                Ok::<(), Infallible>(())
            })
            .unwrap();
        let mut source = ObservedRead::new(&data);
        let eof = Rc::clone(&source.eof);
        let mut actual = Vec::new();
        let bounded = ReplayReader
            .read_bounded(&mut source, data.len(), ReadOptions::default(), |event| {
                assert!(eof.get(), "no callback is allowed before EOF");
                actual.push(format!("{event:?}"));
                Ok::<(), Infallible>(())
            })
            .unwrap();
        assert_eq!(source.bytes_read, data.len());
        assert_eq!(source.last_request, 1, "exact limit requires a byte probe");
        assert_eq!(actual, expected);
        assert_eq!(bounded.field_rows, 1);
        assert_eq!(format!("{bounded:?}"), format!("{slice:?}"));
    }
}

#[test]
fn short_reads_and_interruptions_do_not_lose_bytes() {
    let data = main_input();
    for short_read in [1, 3, 37] {
        let mut source = ObservedRead::new(&data);
        source.short_read = short_read;
        source.interrupt = true;
        let eof = Rc::clone(&source.eof);
        let mut fields = Vec::new();
        let summary = ReplayReader
            .read_bounded(&mut source, data.len(), ReadOptions::default(), |event| {
                assert!(eof.get());
                if let ReplayEvent::Records(records) = event {
                    fields.extend_from_slice(records.fields);
                }
                Ok::<(), Infallible>(())
            })
            .unwrap();
        assert_eq!(source.bytes_read, data.len());
        assert_eq!(source.calls, 2 * (data.len().div_ceil(short_read) + 1));
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].raw_bits.as_deref(), Some(&[0x61][..]));
        assert_eq!(fields[0].time_ms, 9000);
        assert_eq!(summary.field_rows, 1);
    }
}

#[test]
fn an_excess_byte_refuses_input_before_callbacks_and_stops_reading() {
    let mut data = main_input();
    let small_limit = data.len() - 1;
    data.extend([0xff; 32768]);
    for limit in [0, 1, small_limit, 8192, 16384] {
        let mut source = ObservedRead::new(&data);
        let mut callbacks = 0;
        let error = ReplayReader
            .read_bounded(&mut source, limit, ReadOptions::default(), |_| {
                callbacks += 1;
                Ok::<(), Infallible>(())
            })
            .unwrap_err();
        assert!(matches!(error, ReadError::InputLimit { limit: actual } if actual == limit));
        assert_eq!(source.bytes_read, limit + 1);
        assert_eq!(source.last_request, 1);
        assert!(!source.eof.get());
        assert_eq!(callbacks, 0, "even metadata waits for the input check");
    }
}

#[test]
fn empty_reads_terminate_and_zero_limit_does_not_mean_unlimited() {
    let expected = ReplayReader
        .read(&[], |_| Ok::<(), Infallible>(()))
        .unwrap_err();
    for limit in [0, 1, usize::MAX] {
        let mut source = ObservedRead::new(&[]);
        let mut callbacks = 0;
        let error = ReplayReader
            .read_bounded(&mut source, limit, ReadOptions::default(), |_| {
                callbacks += 1;
                Ok::<(), Infallible>(())
            })
            .unwrap_err();
        assert!(matches!(
            error,
            ReadError::Replay(ReplayError::Container(_))
        ));
        assert_eq!(error.to_string(), expected.to_string());
        assert_eq!(source.calls, 1);
        assert!(source.eof.get());
        assert_eq!(callbacks, 0);
    }
}

#[test]
fn largest_limit_does_not_overflow_or_reserve_up_front_and_accepts_dyn_read() {
    let data = main_input();
    let mut source = ObservedRead::new(&data);
    let input: &mut dyn Read = &mut source;
    let summary = ReplayReader
        .read_bounded(input, usize::MAX, ReadOptions::default(), |_| {
            Ok::<(), Infallible>(())
        })
        .unwrap();
    assert_eq!(summary.field_rows, 1);
    assert!(source.eof.get());
}

#[derive(Debug)]
struct IoMarker(Arc<()>);

impl fmt::Display for IoMarker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("injected input error")
    }
}

impl Error for IoMarker {}

#[test]
fn io_failure_keeps_original_payload_even_on_the_exact_limit_probe() {
    let data = main_input();
    for limit in [data.len(), data.len() + 1] {
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::WouldBlock] {
            let token = Arc::new(());
            let mut source = ObservedRead::new(&data);
            source.failure = Some(io::Error::new(kind, IoMarker(Arc::clone(&token))));
            let mut callbacks = 0;
            let error = ReplayReader
                .read_bounded(&mut source, limit, ReadOptions::default(), |_| {
                    callbacks += 1;
                    Ok::<(), Infallible>(())
                })
                .unwrap_err();
            let ReadError::Replay(ReplayError::Io(error)) = error else {
                panic!("original I/O error was replaced: {error:?}");
            };
            assert_eq!(error.kind(), kind);
            let marker = error.get_ref().unwrap().downcast_ref::<IoMarker>().unwrap();
            assert!(Arc::ptr_eq(&marker.0, &token));
            assert!(!source.eof.get(), "WouldBlock must not be retried as EOF");
            assert_eq!(source.bytes_read, data.len());
            assert_eq!(callbacks, 0);
        }
    }
}

#[test]
fn callback_error_keeps_value_and_outranks_later_parse_failure() {
    let mut data = main_input();
    data.extend([1, 0, 0]); // later truncated chunk header
    let mut source = ObservedRead::new(&data);
    let eof = Rc::clone(&source.eof);
    let marker = Box::new(17);
    let pointer = &*marker as *const i32;
    let mut marker = Some(marker);
    let mut callbacks = 0;
    let result =
        ReplayReader.read_bounded(&mut source, data.len(), ReadOptions::default(), |event| {
            assert!(eof.get());
            callbacks += 1;
            if matches!(event, ReplayEvent::Records(_)) {
                Err(marker.take().unwrap())
            } else {
                Ok(())
            }
        });
    match result {
        Err(ReadError::Consumer(error)) => assert_eq!(&*error as *const i32, pointer),
        other => panic!("original callback error was not preserved: {other:?}"),
    }
    assert_eq!(source.bytes_read, data.len());
    assert_eq!(callbacks, 2);
}

#[test]
fn later_structural_failure_leaves_only_provisional_callbacks() {
    let mut data = main_input();
    data.extend([1, 0, 0]);
    let mut expected = Vec::new();
    let slice = ReplayReader
        .read(&data, |event| {
            expected.push(format!("{event:?}"));
            Ok::<(), Infallible>(())
        })
        .unwrap_err();
    let mut actual = Vec::new();
    let error = ReplayReader
        .read_bounded(
            &mut &data[..],
            data.len(),
            ReadOptions::default(),
            |event| {
                actual.push(format!("{event:?}"));
                Ok::<(), Infallible>(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, ReadError::Replay(_)));
    assert_eq!(actual.len(), 2, "metadata and one provisional record batch");
    assert_eq!(actual, expected);
    assert_eq!(error.to_string(), slice.to_string());
}

#[test]
fn checkpoint_callbacks_and_summaries_match_slice_order_for_both_options() {
    let data = checkpoint_input();
    for checkpoints in [false, true] {
        let options = ReadOptions {
            checkpoints,
            ..ReadOptions::default()
        };
        let mut expected = Vec::new();
        let slice = ReplayReader
            .read_with_options(&data, options, |event| {
                expected.push(format!("{event:?}"));
                Ok::<(), Infallible>(())
            })
            .unwrap();
        let mut source = ObservedRead::new(&data);
        source.short_read = 7;
        let eof = Rc::clone(&source.eof);
        let mut actual = Vec::new();
        let mut order = Vec::new();
        let bounded = ReplayReader
            .read_bounded(&mut source, data.len(), options, |event| {
                assert!(eof.get());
                actual.push(format!("{event:?}"));
                match event {
                    ReplayEvent::Metadata(_) => order.push("main metadata"),
                    ReplayEvent::Records(_) => order.push("main records"),
                    ReplayEvent::Checkpoint(CheckpointEvent::Metadata(_)) => {
                        order.push("checkpoint metadata");
                    }
                    ReplayEvent::Checkpoint(CheckpointEvent::Records { records, .. }) => {
                        assert_eq!(records.fields[0].raw_bits.as_deref(), Some(&[0x61][..]));
                        order.push("checkpoint records");
                    }
                    ReplayEvent::Checkpoint(CheckpointEvent::Summary(_)) => {
                        order.push("checkpoint summary");
                    }
                    _ => panic!("unexpected synthetic event: {event:?}"),
                }
                Ok::<(), Infallible>(())
            })
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(format!("{bounded:?}"), format!("{slice:?}"));
        if checkpoints {
            assert_eq!(bounded.checkpoints.chunks, 2);
            assert_eq!(bounded.checkpoints.field_rows, 2);
            assert_eq!(bounded.skipped_checkpoint_chunks, 0);
            assert_eq!(
                order,
                [
                    "main metadata",
                    "main records",
                    "checkpoint metadata",
                    "checkpoint records",
                    "checkpoint summary",
                    "checkpoint metadata",
                    "checkpoint records",
                    "checkpoint summary",
                ]
            );
        } else {
            assert_eq!(bounded.checkpoints.chunks, 0);
            assert_eq!(bounded.skipped_checkpoint_chunks, 2);
            assert_eq!(order, ["main metadata", "main records"]);
        }
    }
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
fn main_schema_limit_keeps_typed_cause() {
    let mut frames = BitWriter::new();
    frames
        .i32(0)
        .f32(1.0)
        .int_packed(1)
        .int_packed(1)
        .int_packed(1)
        .fstring("TooLarge")
        .int_packed(8);
    // No field flag follows: the custom limit must win before that parse error.
    let frames = pack(&frames);
    let mut data = preamble();
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    let error = ReplayReader
        .read_bounded(
            &mut &data[..],
            data.len(),
            ReadOptions {
                schema_limits: SchemaLimits {
                    max_groups: 1,
                    max_field_slots: 7,
                },
                checkpoints: false,
            },
            |_| Ok::<(), Infallible>(()),
        )
        .unwrap_err();
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "field slots",
            requested: 8,
            limit: 7,
        })
    );
}

#[test]
fn checkpoint_schema_limit_keeps_typed_cause() {
    let mut tables = Vec::new();
    add_u32(&mut tables, 16);
    tables.extend([0; 16]); // three reserved words, then zero GUID entries
    add_u32(&mut tables, 1);
    add_fstring(&mut tables, "TooLarge");
    add_int_packed(&mut tables, 7);
    add_int_packed(&mut tables, 1); // policy refuses before the missing field flag
    let frame_offset_word = tables.len() as u32 - 8;
    tables[..4].copy_from_slice(&frame_offset_word.to_le_bytes());
    let mut data = preamble();
    data.extend(chunk(2, &checkpoint(0, &tables, 0)));
    let error = ReplayReader
        .read_bounded(
            &mut &data[..],
            data.len(),
            ReadOptions {
                schema_limits: SchemaLimits {
                    max_groups: 0,
                    max_field_slots: 0,
                },
                checkpoints: true,
            },
            |_| Ok::<(), Infallible>(()),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ReadError::Replay(ReplayError::CheckpointSchema { .. })
    ));
    assert_eq!(
        schema_cause(&error),
        Some(&SchemaError::ResourceLimit {
            resource: "export groups",
            requested: 1,
            limit: 0,
        })
    );
}
