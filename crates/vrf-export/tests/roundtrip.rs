//! Round-trip and stress tests for the Parquet export writers.
//!
//! These tests verify:
//! - Write → read round-trip preserves all values and nulls.
//! - Large writes (>1 row group) produce multiple row groups.
//! - Binary column data is preserved exactly.
//! - Dictionary-encoded columns round-trip correctly.

use std::fs;
use std::path::PathBuf;

use arrow_array::cast::AsArray;
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, BinaryArray, Float32Array, Int64Array, RecordBatch, StringArray,
    UInt32Array,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};

use vrf_export::{FieldRecord, FieldWriter, MovementRecord, MovementWriter};

/// Test output directory — each test writes to a unique file.
fn test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("vrf_export_tests");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Helper: build a FieldRecord with predictable values based on an index.
fn make_field_record(i: u32) -> FieldRecord {
    FieldRecord {
        time_ms: i * 10,
        packet_id: i,
        channel_index: i % 8,
        actor_net_guid: 1000 + (i % 20),
        group_path: format!("Group_{}", i % 5),
        handle: i % 64,
        field_name: if i % 3 == 0 {
            None
        } else {
            Some(format!("Field_{}", i % 10))
        },
        bit_count: (i % 128) + 1,
        raw_bits: if i % 4 == 0 {
            None
        } else {
            Some(vec![(i & 0xFF) as u8; ((i % 16) + 1) as usize])
        },
        value_i64: if i % 5 == 0 {
            Some(i as i64 * 100)
        } else {
            None
        },
        value_f64: if i % 5 == 1 {
            Some(i as f64 * 0.1)
        } else {
            None
        },
        value_bool: if i % 5 == 2 { Some(i % 2 == 0) } else { None },
        value_str: if i % 5 == 3 {
            Some(format!("val_{}", i))
        } else {
            None
        },
    }
}

/// Helper: build a MovementRecord with predictable values.
fn make_movement_record(i: u32) -> MovementRecord {
    MovementRecord {
        time_ms: i * 16,
        packet_id: i / 2,
        character_net_guid: 100 + (i % 10),
        pos_x: i as f32 * 1.5,
        pos_y: i as f32 * 2.0,
        pos_z: 100.0 + (i as f32 * 0.1),
        yaw: ((i % 360) as f32) - 180.0,
        pitch: ((i % 180) as f32) - 90.0,
        vel_x: if i % 3 == 0 { 0.0 } else { i as f32 },
        vel_y: if i % 3 == 1 { 0.0 } else { -(i as f32) },
        vel_z: 0.0,
    }
}

/// Read all record batches from a Parquet file.
fn read_all_batches(path: &std::path::Path) -> Vec<RecordBatch> {
    let file = fs::File::open(path).unwrap();
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    reader.collect::<Result<Vec<_>, _>>().unwrap()
}

// ─── Field Writer Tests ───────────────────────────────────────────────────

#[test]
fn field_roundtrip_basic() {
    let path = test_dir().join("field_roundtrip_basic.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..50 {
            writer.push(make_field_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 50);

    // Verify first batch content.
    let batch = &batches[0];
    let time_ms = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(time_ms.value(0), 0);
    assert_eq!(time_ms.value(1), 10);
    assert_eq!(time_ms.value(49), 490);
}

#[test]
fn field_null_preservation() {
    let path = test_dir().join("field_null_preservation.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        // Row 0: field_name=None, raw_bits=None, value_i64=Some(0)
        writer
            .push(FieldRecord {
                time_ms: 0,
                packet_id: 0,
                channel_index: 0,
                actor_net_guid: 1,
                group_path: "Test".into(),
                handle: 0,
                field_name: None,
                bit_count: 0,
                raw_bits: None,
                value_i64: Some(0),
                value_f64: None,
                value_bool: None,
                value_str: None,
            })
            .unwrap();
        // Row 1: field_name=Some, raw_bits=Some, value_str=Some
        writer
            .push(FieldRecord {
                time_ms: 1,
                packet_id: 1,
                channel_index: 0,
                actor_net_guid: 1,
                group_path: "Test".into(),
                handle: 1,
                field_name: Some("Health".into()),
                bit_count: 8,
                raw_bits: Some(vec![0xAB]),
                value_i64: None,
                value_f64: None,
                value_bool: None,
                value_str: Some("hello".into()),
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];

    // field_name: row 0 null, row 1 = "Health"
    let field_name = batch.column(6).as_dictionary::<Int32Type>();
    assert!(field_name.is_null(0));
    assert!(!field_name.is_null(1));
    let field_name_values = field_name.downcast_dict::<StringArray>().unwrap();
    assert_eq!(field_name_values.value(1), "Health");

    // raw_bits: row 0 null, row 1 = [0xAB]
    let raw_bits = batch
        .column(8)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert!(raw_bits.is_null(0));
    assert_eq!(raw_bits.value(1), &[0xAB]);

    // value_i64: row 0 = Some(0), row 1 = null
    let value_i64 = batch
        .column(9)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert!(!value_i64.is_null(0));
    assert_eq!(value_i64.value(0), 0);
    assert!(value_i64.is_null(1));

    // value_str: row 0 = null, row 1 = "hello"
    let value_str = batch
        .column(12)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(value_str.is_null(0));
    assert_eq!(value_str.value(1), "hello");
}

#[test]
fn field_binary_preservation() {
    // Verify that arbitrary binary data (including 0x00 bytes) survives.
    let payload: Vec<u8> = (0..=255).collect();
    let path = test_dir().join("field_binary_preservation.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        writer
            .push(FieldRecord {
                time_ms: 0,
                packet_id: 0,
                channel_index: 0,
                actor_net_guid: 0,
                group_path: "Bin".into(),
                handle: 0,
                field_name: None,
                bit_count: 256 * 8,
                raw_bits: Some(payload.clone()),
                value_i64: None,
                value_f64: None,
                value_bool: None,
                value_str: None,
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let raw_bits = batches[0]
        .column(8)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(raw_bits.value(0), payload.as_slice());
}

#[test]
fn field_multiple_row_groups() {
    // 200_000 rows with row_group_size=65_536 → should produce at least 3 groups.
    let row_count = 200_000u32;
    let row_group_size = 65_536;
    let path = test_dir().join("field_multiple_row_groups.parquet");

    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, row_group_size).unwrap();
        for i in 0..row_count {
            writer.push(make_field_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    // Use the low-level reader to count row groups.
    let file = fs::File::open(&path).unwrap();
    let file_reader = SerializedFileReader::new(file).unwrap();
    let metadata = file_reader.metadata();
    let num_row_groups = metadata.num_row_groups();

    // 200_000 / 65_536 = 3.05 → expect at least 3 row groups.
    assert!(
        num_row_groups >= 3,
        "expected at least 3 row groups, got {num_row_groups}"
    );

    // Verify total row count.
    let total: i64 = (0..num_row_groups)
        .map(|i| metadata.row_group(i).num_rows())
        .sum();
    assert_eq!(total, row_count as i64);
}

#[test]
fn field_push_batch() {
    let path = test_dir().join("field_push_batch.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        let records: Vec<FieldRecord> = (0..300).map(make_field_record).collect();
        writer.push_batch(records).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 300);
}

// ─── Movement Writer Tests ────────────────────────────────────────────────

#[test]
fn movement_roundtrip_basic() {
    let path = test_dir().join("movement_roundtrip_basic.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..100 {
            writer.push(make_movement_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 100);

    let batch = &batches[0];
    let pos_x = batch
        .column(3)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert!((pos_x.value(0) - 0.0).abs() < f32::EPSILON);
    assert!((pos_x.value(1) - 1.5).abs() < f32::EPSILON);
}

#[test]
fn movement_multiple_row_groups() {
    let row_count = 200_000u32;
    let row_group_size = 65_536;
    let path = test_dir().join("movement_multiple_row_groups.parquet");

    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, row_group_size).unwrap();
        for i in 0..row_count {
            writer.push(make_movement_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    let file = fs::File::open(&path).unwrap();
    let file_reader = SerializedFileReader::new(file).unwrap();
    let metadata = file_reader.metadata();
    let num_row_groups = metadata.num_row_groups();
    assert!(
        num_row_groups >= 3,
        "expected at least 3 row groups, got {num_row_groups}"
    );

    let total: i64 = (0..num_row_groups)
        .map(|i| metadata.row_group(i).num_rows())
        .sum();
    assert_eq!(total, row_count as i64);
}

#[test]
fn movement_f32_precision() {
    let path = test_dir().join("movement_f32_precision.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 1024).unwrap();
        writer
            .push(MovementRecord {
                time_ms: 0,
                packet_id: 0,
                character_net_guid: 1,
                pos_x: std::f32::consts::PI,
                pos_y: -std::f32::consts::E,
                pos_z: f32::MIN_POSITIVE,
                yaw: 179.99,
                pitch: -89.5,
                vel_x: f32::MAX,
                vel_y: f32::MIN,
                vel_z: 0.0,
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    let pos_x = batch
        .column(3)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(pos_x.value(0), std::f32::consts::PI);
    let vel_x = batch
        .column(8)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(vel_x.value(0), f32::MAX);
}

#[test]
fn movement_push_batch() {
    let path = test_dir().join("movement_push_batch.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 1024).unwrap();
        let records: Vec<MovementRecord> = (0..500).map(make_movement_record).collect();
        writer.push_batch(records).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 500);
}

/// Write interop files for the Python verification step (requirement §6).
#[test]
fn write_interop_files() {
    let dir = test_dir().join("interop");
    fs::create_dir_all(&dir).unwrap();

    let field_path = dir.join("fields_interop.parquet");
    let movement_path = dir.join("movement_interop.parquet");

    // Write 10_000 field records.
    {
        let file = fs::File::create(&field_path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 4096).unwrap();
        for i in 0..10_000u32 {
            writer.push(make_field_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    // Write 50_000 movement records.
    {
        let file = fs::File::create(&movement_path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 8192).unwrap();
        for i in 0..50_000u32 {
            writer.push(make_movement_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    // Verify files exist and have non-trivial size.
    assert!(field_path.exists());
    assert!(movement_path.exists());
    let field_size = fs::metadata(&field_path).unwrap().len();
    let movement_size = fs::metadata(&movement_path).unwrap().len();
    assert!(field_size > 1000, "fields file too small: {field_size}");
    assert!(
        movement_size > 1000,
        "movement file too small: {movement_size}"
    );

    // Print paths for the Python script to find.
    println!("INTEROP_FIELDS={}", field_path.display());
    println!("INTEROP_MOVEMENT={}", movement_path.display());
    println!("FIELD_SIZE_BYTES={field_size}");
    println!("MOVEMENT_SIZE_BYTES={movement_size}");
}

// --- Actor Writer Tests ---

use vrf_export::{ActorRecord, ActorWriter};

/// Helper: build an ActorRecord with predictable values.
fn make_actor_record(i: u32, is_open: bool) -> ActorRecord {
    ActorRecord {
        time_ms: i * 16,
        packet_id: i / 2,
        channel_index: i % 64,
        actor_net_guid: 2000 + i,
        event: if is_open { "open" } else { "close" },
        class_path: if i % 4 == 0 {
            None
        } else {
            Some(format!(
                "/Game/Characters/Agent_{}/Agent_{}_PC.Agent_{}_PC_C",
                i % 5,
                i % 5,
                i % 5
            ))
        },
        archetype_path: if is_open && i % 3 != 0 {
            Some(format!(
                "/Game/Characters/Agent_{}/Agent_{}_PC.Default__Agent_{}_PC_C",
                i % 5,
                i % 5,
                i % 5
            ))
        } else {
            None
        },
        spawn_x: if is_open { Some(i as f32 * 10.0) } else { None },
        spawn_y: if is_open { Some(i as f32 * 20.0) } else { None },
        spawn_z: if is_open { Some(100.0) } else { None },
        spawn_pitch: if is_open && i % 2 == 0 {
            Some(5.0)
        } else {
            None
        },
        spawn_yaw: if is_open && i % 2 == 0 {
            Some(90.0)
        } else {
            None
        },
        spawn_roll: if is_open && i % 2 == 0 {
            Some(0.0)
        } else {
            None
        },
    }
}

#[test]
fn actor_roundtrip_basic() {
    let path = test_dir().join("actor_roundtrip_basic.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = ActorWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..100 {
            writer.push(make_actor_record(i, i % 3 != 2)).unwrap();
        }
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 100);

    // Verify first batch content.
    let batch = &batches[0];
    let time_ms = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(time_ms.value(0), 0);
    assert_eq!(time_ms.value(1), 16);

    // Verify event column (string).
    let event = batch
        .column(4)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(event.value(0), "open");
    // Index 2 is the first "close" (i=2, i%3==2).
    assert_eq!(event.value(2), "close");
}

#[test]
fn actor_null_class_path() {
    let path = test_dir().join("actor_null_class_path.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = ActorWriter::with_row_group_size(file, 1024).unwrap();
        // Row 0: class_path = None (i=0, i%4==0)
        writer.push(make_actor_record(0, true)).unwrap();
        // Row 1: class_path = Some (i=1, i%4!=0)
        writer.push(make_actor_record(1, true)).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];

    // class_path: row 0 null, row 1 non-null
    let class_path = batch.column(5).as_dictionary::<Int32Type>();
    assert!(class_path.is_null(0));
    assert!(!class_path.is_null(1));
    let class_path_values = class_path.downcast_dict::<StringArray>().unwrap();
    assert_eq!(
        class_path_values.value(1),
        "/Game/Characters/Agent_1/Agent_1_PC.Agent_1_PC_C"
    );
}

#[test]
fn actor_spawn_location_nullable() {
    let path = test_dir().join("actor_spawn_location.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = ActorWriter::with_row_group_size(file, 1024).unwrap();
        // Open: has spawn location
        writer.push(make_actor_record(5, true)).unwrap();
        // Close: no spawn location
        writer.push(make_actor_record(5, false)).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];

    let spawn_x = batch
        .column(7)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    // Open row has spawn_x = 50.0
    assert!(!spawn_x.is_null(0));
    assert!((spawn_x.value(0) - 50.0).abs() < f32::EPSILON);
    // Close row has null spawn_x
    assert!(spawn_x.is_null(1));
}

#[test]
fn actor_push_finish_empty() {
    // Verify that finishing a writer with zero rows produces a valid file.
    let path = test_dir().join("actor_empty.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let writer = ActorWriter::new(file).unwrap();
        writer.finish().unwrap();
    }
    // File should still be valid Parquet with 0 rows.
    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 0);
}
