//! `export --out` as an operator meets it: the real binary, its exit code,
//! its stderr, and what is left on disk afterwards.
//!
//! `driver/publish.rs` tests the transaction on its own. These tie it to the
//! command: the refusal happens before anything is written, and the names
//! `export` creates are exactly the ones a later export to the same
//! directory accepts back -- a table added to the driver but not to its
//! output list makes the re-export below refuse the directory.
//!
//! The replay is an info section and a header and nothing else, which
//! `export` parses, writes every table empty for, and publishes.

// Without the `export` feature the binary has no `export` subcommand.
#![cfg(feature = "export")]

mod common;

use std::fs;
use std::path::Path;

use common::*;

/// What a plain export leaves in its directory, sorted.
const MAIN_OUTPUTS: [&str; 8] = [
    "ability_casts.parquet",
    "actors.parquet",
    "events.parquet",
    "fields.parquet",
    "manifest.json",
    "movement.parquet",
    "net_guids.parquet",
    "partials.parquet",
];

/// The tables only `--checkpoints` writes.
const CHECKPOINT_TABLES: [&str; 7] = [
    "checkpoint_fields.parquet",
    "checkpoint_actors.parquet",
    "checkpoint_net_guids.parquet",
    "checkpoint_blocks.parquet",
    "checkpoint_guid_entries.parquet",
    "checkpoint_export_groups.parquet",
    "checkpoint_export_fields.parquet",
];

fn replay() -> Vec<u8> {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data
}

/// The names in `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("list the directory")
        .map(|entry| {
            let name = entry.expect("read a directory entry").file_name();
            name.into_string().expect("scratch names are UTF-8")
        })
        .collect();
    names.sort();
    names
}

/// `export dir/match.vrf --out dir` must exit 1 naming the replay, the user's
/// other file and the subdirectory, and leave the directory -- and its parent
/// -- exactly as they were, never publish over them.
#[test]
fn export_refuses_a_destination_holding_the_replay_and_other_files() {
    let dir = scratch("export_destination", "refused");
    let out = dir.join("userdir");
    fs::create_dir_all(out.join("sub")).expect("create the destination");
    let replay_path = out.join("match.vrf");
    let files = [
        (replay_path.clone(), replay()),
        (out.join("precious.txt"), b"precious".to_vec()),
        (out.join("sub").join("notes.txt"), b"notes".to_vec()),
    ];
    for (path, bytes) in &files {
        fs::write(path, bytes).expect("seed the destination");
    }

    let run = vrfkit(&["export", path_arg(&replay_path), "--out", path_arg(&out)]);

    for (path, bytes) in &files {
        assert!(
            fs::read(path).ok().as_ref() == Some(bytes),
            "{} must survive the export\n{}",
            path.display(),
            run.stderr
        );
    }
    assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("3 entries an export does not write (match.vrf, precious.txt, sub)"),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("choose a new or empty directory"),
        "the refusal must say how to proceed: {}",
        run.stderr
    );
    assert_eq!(entries(&out), ["match.vrf", "precious.txt", "sub"]);
    assert_eq!(entries(&dir), ["userdir"], "no staging directory is left");
    fs::remove_dir_all(dir).ok();
}

/// A destination that does not exist yet, or exists and is empty, is
/// published into.
#[test]
fn export_publishes_into_a_missing_or_an_empty_destination() {
    let dir = scratch("export_destination", "fresh");
    let replay_path = dir.join("match.vrf");
    fs::write(&replay_path, replay()).expect("write the replay");
    let empty = dir.join("empty");
    fs::create_dir(&empty).expect("create the empty destination");

    for out in [dir.join("missing"), empty] {
        let run = vrfkit(&["export", path_arg(&replay_path), "--out", path_arg(&out)]);
        assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
        assert_eq!(entries(&out), MAIN_OUTPUTS);
    }
    assert_eq!(entries(&dir), ["empty", "match.vrf", "missing"]);
    fs::remove_dir_all(dir).ok();
}

/// A `--checkpoints` export and then a plain one into the same directory:
/// the second replaces the first -- so every name the first wrote is one a
/// re-export accepts -- and its summary names each checkpoint table it drops.
#[test]
fn a_re_export_replaces_a_prior_export_and_names_the_tables_it_drops() {
    let dir = scratch("export_destination", "reexport");
    let replay_path = dir.join("match.vrf");
    fs::write(&replay_path, replay()).expect("write the replay");
    let out = dir.join("out");

    let first = vrfkit(&[
        "export",
        path_arg(&replay_path),
        "--out",
        path_arg(&out),
        "--checkpoints",
    ]);
    assert_eq!(first.code, Some(0), "{}\n{}", first.stdout, first.stderr);
    assert_eq!(
        entries(&out).len(),
        MAIN_OUTPUTS.len() + CHECKPOINT_TABLES.len()
    );

    let second = vrfkit(&["export", path_arg(&replay_path), "--out", path_arg(&out)]);
    assert_eq!(second.code, Some(0), "{}\n{}", second.stdout, second.stderr);
    let dropped: Vec<&str> = second
        .stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("DROPPED TABLE:"))
        .collect();
    assert_eq!(dropped.len(), 1, "{}", second.stderr);
    for table in CHECKPOINT_TABLES {
        assert!(
            dropped[0].contains(table),
            "{table} missing: {}",
            dropped[0]
        );
    }
    assert_eq!(entries(&out), MAIN_OUTPUTS);
    assert_eq!(
        entries(&dir),
        ["match.vrf", "out"],
        "no staging or prior-output directory is left"
    );
    fs::remove_dir_all(dir).ok();
}
