//! Independent Phase D raw-cell preservation oracle through public writers.
#![cfg(all(feature = "fields", feature = "checkpoint-context"))]
#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::path::Path;

use arrow_array::Array;
use arrow_array::cast::AsArray;
use arrow_array::types::{Int64Type, UInt32Type};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use vrf_export::{
    CheckpointFieldRecord, CheckpointFieldWriter, CheckpointIdentity, FieldRecord, FieldWriter,
};

const ROWS: usize = 2 * 8_192 + 11;

fn payload(index: usize) -> Option<Vec<u8>> {
    let length = if matches!(index, 8_191 | 8_192 | 16_383) {
        131_072 + 7
    } else {
        match index % 7 {
            0 => return None,
            1 => 0,
            2 => 1,
            3 => 16,
            4 => 17,
            5 => 257,
            _ => 4_097,
        }
    };
    Some(
        (0..length)
            .map(|j| ((j * 73 + index * 19) & 255) as u8)
            .collect(),
    )
}

fn record(index: usize) -> FieldRecord {
    let raw = payload(index);
    FieldRecord {
        time_ms: index as u32,
        packet_id: index as u32,
        channel_index: 7,
        actor_net_guid: 101,
        object_net_guid: if index % 2 == 0 { None } else { Some(0) },
        group_path: "RawContract".into(),
        handle: 46,
        field_name: Some("Opaque".into()),
        compatible_checksum: None,
        bit_count: raw
            .as_ref()
            .map_or(0, |bytes| (bytes.len() * 8).saturating_sub(3) as u32),
        raw_bits: raw.map(Into::into),
        value_i64: Some(-(index as i64)),
        value_f64: None,
        value_bool: None,
        value_str: None,
    }
}

fn check_export(path: &Path, checkpoints: bool) {
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
        .unwrap()
        .with_batch_size(997)
        .build()
        .unwrap();
    let mut row = 0;
    let mut nulls = 0;
    let mut empty = 0;
    let mut oversized = 0;
    for batch in reader {
        let batch = batch.unwrap();
        let raw = batch.column_by_name("raw_bits").unwrap().as_binary::<i32>();
        let packet = batch
            .column_by_name("packet_id")
            .unwrap()
            .as_primitive::<UInt32Type>();
        let width = batch
            .column_by_name("bit_count")
            .unwrap()
            .as_primitive::<UInt32Type>();
        let typed = batch
            .column_by_name("value_i64")
            .unwrap()
            .as_primitive::<Int64Type>();
        for offset in 0..batch.num_rows() {
            assert_eq!(packet.value(offset), row as u32, "row count/order changed");
            assert_eq!(
                typed.value(offset),
                -(row as i64),
                "typed companion changed"
            );
            match payload(row) {
                None => {
                    nulls += 1;
                    assert!(
                        raw.is_null(offset),
                        "null raw cell became a value at row {row}"
                    );
                    assert_eq!(width.value(offset), 0);
                }
                Some(expected) => {
                    empty += usize::from(expected.is_empty());
                    oversized += usize::from(expected.len() > 131_072);
                    assert!(
                        !raw.is_null(offset),
                        "present raw cell became null at row {row}"
                    );
                    assert_eq!(
                        raw.value(offset),
                        expected,
                        "raw bytes changed at row {row}"
                    );
                    assert_eq!(
                        width.value(offset),
                        (expected.len() * 8).saturating_sub(3) as u32
                    );
                }
            }
            if checkpoints {
                let index = batch
                    .column_by_name("checkpoint_index")
                    .unwrap()
                    .as_primitive::<UInt32Type>();
                let id = batch
                    .column_by_name("checkpoint_id")
                    .unwrap()
                    .as_string::<i32>();
                assert_eq!(index.value(offset), (row / 4_096) as u32);
                assert_eq!(id.value(offset), format!("snapshot-{}", row / 4_096));
            }
            row += 1;
        }
    }
    assert_eq!(row, ROWS);
    assert!(nulls > 0 && empty > 0);
    assert_eq!(oversized, 3);
}

#[test]
fn independent_main_raw_cells_survive_batch_and_group_boundaries() {
    let directory = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase-d-raw-main");
    fs::create_dir_all(&directory).unwrap();
    for group in [8_199, 131_072] {
        let path = directory.join(format!("raw-{group}.parquet"));
        let mut writer =
            FieldWriter::with_row_group_size(File::create(&path).unwrap(), group).unwrap();
        writer.push_batch((0..ROWS).map(record)).unwrap();
        writer.finish().unwrap();
        check_export(&path, false);
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn independent_checkpoint_raw_cells_survive_dictionary_and_identity_boundaries() {
    let directory =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase-d-raw-checkpoint");
    fs::create_dir_all(&directory).unwrap();
    for group in [8_199, 131_072] {
        let path = directory.join(format!("raw-{group}.parquet"));
        let mut writer =
            CheckpointFieldWriter::with_row_group_size(File::create(&path).unwrap(), group)
                .unwrap();
        writer
            .push_batch((0..ROWS).map(|index| CheckpointFieldRecord {
                checkpoint: CheckpointIdentity {
                    checkpoint_index: (index / 4_096) as u32,
                    checkpoint_id: format!("snapshot-{}", index / 4_096).into(),
                },
                field: record(index),
            }))
            .unwrap();
        writer.finish().unwrap();
        check_export(&path, true);
        fs::remove_file(path).unwrap();
    }
}
