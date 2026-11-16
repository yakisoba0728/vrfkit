//! File-backed CLI passes must preserve rows, stream separation and publication.

mod common;

use std::fs;
use std::io::{Seek, SeekFrom, Write};

use common::*;

fn preamble() -> Vec<u8> {
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data
}

fn main_chunk() -> Vec<u8> {
    let frames = frame(1.0, &[], 0, &[]);
    chunk(1, &replay_data(&frames, frames.len()))
}

#[test]
fn validate_seeks_over_sparse_checkpoints_and_unknown_payloads() {
    let dir = scratch("cli-input", "validate-sparse");
    let path = dir.join("replay.vrf");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&preamble()).unwrap();
    file.write_all(&main_chunk()).unwrap();
    for kind in [2_u32, 99] {
        file.write_all(&kind.to_le_bytes()).unwrap();
        file.write_all(&(64_i32 * 1024 * 1024).to_le_bytes())
            .unwrap();
        file.seek(SeekFrom::Current(64 * 1024 * 1024)).unwrap();
    }
    file.write_all(&main_chunk()).unwrap();
    drop(file);

    let run = vrfkit(&["validate", path_arg(&path)]);
    assert_eq!(run.code, Some(2), "{}\n{}", run.stdout, run.stderr);
    assert_eq!(line_value(&run.stdout, "ReplayData frames:"), "2");
    assert_eq!(line_value(&run.stdout, "Unknown chunks:"), "1");
    assert!(run.stdout.contains("1 Checkpoint"), "{}", run.stdout);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn validate_rejects_a_late_invalid_skipped_payload() {
    let dir = scratch("cli-input", "validate-invalid");
    let path = dir.join("replay.vrf");
    for kind in [0, 2, 99] {
        for size in [-1, 64] {
            let mut data = preamble();
            data.extend(main_chunk());
            add_u32(&mut data, kind);
            add_i32(&mut data, size);
            fs::write(&path, data).unwrap();
            let run = vrfkit(&["validate", path_arg(&path)]);
            assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
            assert!(!run.stdout.contains("=== Validation Oracle ==="));
            assert!(
                run.stderr.contains("chunk"),
                "expected container error: {}",
                run.stderr
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "export")]
#[test]
fn sparse_unknown_chunks_do_not_change_main_or_checkpoint_tables() {
    let dir = scratch("cli-input", "export-sparse");
    let compact = dir.join("compact.vrf");
    let padded = dir.join("padded.vrf");
    let mut data = preamble();
    let snapshot = checkpoint_tables(&frame(2.0, &[], 0, &[&unfinished_partial_packet()]));
    for index in 0..2 {
        data.extend(main_chunk());
        data.extend(chunk(2, &checkpoint(index, &snapshot, 0)));
    }
    fs::write(&compact, &data).unwrap();
    let mut file = fs::File::create(&padded).unwrap();
    file.write_all(&data).unwrap();
    file.write_all(&99_u32.to_le_bytes()).unwrap();
    file.write_all(&(128_i32 * 1024 * 1024).to_le_bytes())
        .unwrap();
    file.seek(SeekFrom::Current(128 * 1024 * 1024)).unwrap();
    file.write_all(&chunk(0, &header_payload())).unwrap();
    drop(file);

    let mut outputs = Vec::new();
    for (index, path) in [&compact, &padded].into_iter().enumerate() {
        let out = dir.join(format!("out{index}"));
        let run = vrfkit(&[
            "export",
            path_arg(path),
            "--out",
            path_arg(&out),
            "--checkpoints",
        ]);
        assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
        let manifest = fs::read_to_string(out.join("manifest.json")).unwrap();
        assert_eq!(
            json_u64(&manifest, "source_size_bytes"),
            fs::metadata(path).unwrap().len()
        );
        assert_eq!(json_u64(&manifest, "checkpoint_chunks"), 2);
        let mut tables: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "parquet")
            })
            .map(|path| {
                (
                    path.file_name().unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                )
            })
            .collect();
        tables.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(tables.len(), 13);
        outputs.push(tables);
    }
    assert_eq!(
        outputs[0], outputs[1],
        "input padding cannot change any table"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "export")]
#[test]
fn late_input_and_checkpoint_errors_leave_prior_export_unchanged() {
    let dir = scratch("cli-input", "export-rollback");
    let path = dir.join("replay.vrf");
    let out = dir.join("out");
    fs::write(&path, preamble()).unwrap();
    let first = vrfkit(&["export", path_arg(&path), "--out", path_arg(&out)]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let before: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), fs::read(path).unwrap())
        })
        .collect();

    let snapshot = checkpoint_tables(&frame(2.0, &[], 0, &[]));
    let mut prefix = preamble();
    prefix.extend(main_chunk());
    prefix.extend(chunk(2, &checkpoint(0, &snapshot, 0)));
    let mut bad_header = Vec::new();
    add_u32(&mut bad_header, 99);
    add_i32(&mut bad_header, 1024);
    for suffix in [bad_header, chunk(2, &[0xde, 0xad])] {
        let mut data = prefix.clone();
        data.extend(suffix);
        fs::write(&path, &data).unwrap();
        let run = vrfkit(&[
            "export",
            path_arg(&path),
            "--out",
            path_arg(&out),
            "--checkpoints",
        ]);
        assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
        for (path, bytes) in &before {
            assert_eq!(&fs::read(path).unwrap(), bytes);
        }
        assert_eq!(fs::read_dir(&out).unwrap().count(), before.len());
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            2,
            "no abandoned staging"
        );
        assert_eq!(fs::read(&path).unwrap(), data);
    }
    fs::remove_dir_all(dir).unwrap();
}
