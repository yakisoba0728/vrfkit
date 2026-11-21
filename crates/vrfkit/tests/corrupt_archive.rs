//! An archive the Oodle codec panics on is a `ReadError::Replay`, never a
//! panic: in memory, from a path (prefetch thread), bounded, inside a
//! checkpoint, and through the CLI subcommands.

mod common;

use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, catch_unwind};

use common::*;
use vrf_container::ContainerError;
use vrfkit::{ReadError, ReadOptions, ReplayError, ReplayReader};

const CAUGHT: &str = "decoder panicked: ";

fn preamble() -> Vec<u8> {
    let mut data = replay_info(&Info {
        compressed: true,
        ..Info::default()
    });
    data.extend(chunk(0, &header_payload()));
    data
}

/// The main stream's one ReplayData chunk is the codec-panicking archive.
fn corrupt_main_stream() -> Vec<u8> {
    let mut data = preamble();
    data.extend(chunk(1, &replay_data(&archive_that_panics_the_codec(), 64)));
    data
}

/// A clean compressed main stream, then a checkpoint whose archive panics the codec.
fn corrupt_checkpoint() -> Vec<u8> {
    let frames = frame(1.0, &[], 0, &[]);
    let mut data = preamble();
    data.extend(chunk(
        1,
        &replay_data(&archive_with_unread_input(&frames, 0), frames.len()),
    ));
    data.extend(chunk(
        2,
        &checkpoint(0, &archive_that_panics_the_codec(), 0),
    ));
    data
}

/// Every read entry point, each behind `catch_unwind` so a panic fails the
/// test with the entry point's name instead of aborting the harness.
fn every_entry_point(
    data: &[u8],
    options: ReadOptions,
    name: &str,
) -> Vec<(String, ReadError<Infallible>)> {
    let dir = scratch("corrupt-archive", name);
    let path = dir.join("input.vrf");
    std::fs::write(&path, data).unwrap();
    let ok = |_: vrfkit::ReplayEvent<'_>| Ok::<(), Infallible>(());
    let runs: Vec<(&str, Entry<'_>)> = vec![
        (
            "read_with_options",
            Box::new(|| ReplayReader.read_with_options(data, options, ok).map(drop)),
        ),
        (
            "read_path_with_options",
            Box::new(|| {
                ReplayReader
                    .read_path_with_options(&path, options, ok)
                    .map(drop)
            }),
        ),
        (
            "read_bounded",
            Box::new(|| {
                let mut source = data;
                ReplayReader
                    .read_bounded(&mut source, usize::MAX, options, ok)
                    .map(drop)
            }),
        ),
    ];
    runs.into_iter()
        .map(|(entry, run)| {
            let result = catch_unwind(AssertUnwindSafe(run))
                .unwrap_or_else(|_| panic!("{entry} panicked on a corrupt archive"));
            (entry.to_owned(), result.expect_err(entry))
        })
        .collect()
}

type Entry<'a> = Box<dyn Fn() -> Result<(), ReadError<Infallible>> + 'a>;

fn caught(error: &ReplayError) -> bool {
    matches!(error, ReplayError::Container(ContainerError::OodleDecompression(m)) if m.starts_with(CAUGHT))
}

#[test]
fn a_codec_panic_in_the_main_stream_is_a_replay_error() {
    let data = corrupt_main_stream();
    for checkpoints in [false, true] {
        let options = ReadOptions::default().with_checkpoints(checkpoints);
        for (entry, error) in every_entry_point(&data, options, &format!("main-{checkpoints}")) {
            let ReadError::Replay(replay) = &error else {
                panic!("{entry}: {error:?}");
            };
            assert!(caught(replay), "{entry}: {error:?}");
        }
    }
}

#[test]
fn a_codec_panic_in_a_checkpoint_carries_the_checkpoint() {
    let data = corrupt_checkpoint();
    let options = ReadOptions::default().with_checkpoints(true);
    for (entry, error) in every_entry_point(&data, options, "checkpoint") {
        let ReadError::Replay(ReplayError::Checkpoint {
            checkpoint_index: 0,
            error: inner,
            ..
        }) = &error
        else {
            panic!("{entry}: {error:?}");
        };
        assert!(caught(inner), "{entry}: {error:?}");
    }
}

/// The binary reports the caught panic as an error line with exit 1, and its
/// panic hook keeps the codec's own `panicked at` report off stderr.
#[test]
fn the_cli_reports_a_codec_panic_as_an_error() {
    let dir = scratch("corrupt-archive", "cli");
    let main = dir.join("main.vrf");
    let snapshot = dir.join("checkpoint.vrf");
    std::fs::write(&main, corrupt_main_stream()).unwrap();
    std::fs::write(&snapshot, corrupt_checkpoint()).unwrap();
    let mut runs = vec![
        vec!["validate", path_arg(&main)],
        vec!["diag", path_arg(&main)],
        vec!["diag", path_arg(&snapshot)],
    ];
    let out = dir.join("out");
    if cfg!(feature = "export") {
        runs.push(vec!["export", path_arg(&main), "--out", path_arg(&out)]);
    }
    for args in runs {
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(1), "{args:?}: {}", run.stderr);
        assert!(run.stderr.contains(CAUGHT), "{args:?}: {}", run.stderr);
        assert!(
            !run.stderr.contains("panicked at"),
            "{args:?}: {}",
            run.stderr
        );
    }
}
