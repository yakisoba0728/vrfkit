//! Round-trip and stress tests for the Parquet export writers.
//!
//! These tests verify:
//! - Write -> read round-trip preserves all values and nulls.
//! - Large writes (>1 row group) produce multiple row groups.
//! - Binary column data is preserved exactly.
//! - Dictionary-encoded columns round-trip correctly.
//!
//! Every test here exercises a writer, so the file is empty unless all five
//! table features are on. That is the default; the gate exists so that
//! `--no-default-features` builds this target instead of failing to resolve
//! writers the build deliberately left out.

#![cfg(all(
    feature = "fields",
    feature = "movement",
    feature = "actors",
    feature = "net-guids",
    feature = "events"
))]

use std::fs;
use std::path::PathBuf;

use arrow_array::cast::AsArray;
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, BinaryArray, Float32Array, Int32Array, Int64Array, RecordBatch,
    StringArray, UInt8Array, UInt32Array,
};
use arrow_schema::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};

use vrf_export::{
    FieldRecord, FieldWriter, MovementRecord, MovementWriter,
    UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
};

/// Test output directory -- each test writes to a unique file.
///
/// Keyed by this crate's own source path, so two checkouts of the repository
/// cannot write over each other. They previously shared one directory under
/// the system temp dir, which is not per-checkout: a `cargo test` in a git
/// worktree and one in the main tree write the same filenames, and whichever
/// reads second reads the other's Parquet. That surfaced once already, as a
/// column-count mismatch that looked exactly like a schema bug and was not.
///
/// `CARGO_MANIFEST_DIR` is the discriminator because it differs per worktree
/// and is fixed at compile time, so every test in one binary agrees on it.
fn test_dir() -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    env!("CARGO_MANIFEST_DIR").hash(&mut hasher);
    let dir = std::env::temp_dir().join(format!("vrf_export_tests_{:016x}", hasher.finish()));
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
        // Every third record models a subobject block.
        object_net_guid: if i % 3 == 0 { None } else { Some(9000 + i) },
        group_path: format!("Group_{}", i % 5).into(),
        handle: i % 64,
        field_name: if i % 3 == 0 {
            None
        } else {
            Some(format!("Field_{}", i % 10).into())
        },
        compatible_checksum: None,
        bit_count: (i % 128) + 1,
        raw_bits: if i % 4 == 0 {
            None
        } else {
            Some(vec![(i & 0xFF) as u8; ((i % 16) + 1) as usize].into())
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
        // Vary all three so a round-trip check discriminates a real copy from
        // a constant fill.
        timestamp: i * 3,
        movement_state: (i % 5) as u8,
        move_type: (i % 2) as u8,
    }
}

#[test]
fn zero_row_group_size_returns_a_controlled_error() {
    let path = test_dir().join("zero_row_group_size.parquet");
    let file = fs::File::create(path).unwrap();

    assert!(FieldWriter::with_row_group_size(file, 0).is_err());
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

// --- Field Writer Tests ---------------------------------------------------

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
                object_net_guid: None,
                group_path: "Test".into(),
                handle: 0,
                field_name: None,
                compatible_checksum: None,
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
                object_net_guid: None,
                group_path: "Test".into(),
                handle: 1,
                field_name: Some("Health".into()),
                compatible_checksum: None,
                bit_count: 8,
                raw_bits: Some(vec![0xAB].into()),
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
    let field_name = batch
        .column(batch.schema().index_of("field_name").unwrap())
        .as_dictionary::<Int32Type>();
    assert!(field_name.is_null(0));
    assert!(!field_name.is_null(1));
    let field_name_values = field_name.downcast_dict::<StringArray>().unwrap();
    assert_eq!(field_name_values.value(1), "Health");

    // raw_bits: row 0 null, row 1 = [0xAB]
    let raw_bits = batch
        .column(batch.schema().index_of("raw_bits").unwrap())
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert!(raw_bits.is_null(0));
    assert_eq!(raw_bits.value(1), &[0xAB]);

    // value_i64: row 0 = Some(0), row 1 = null
    let value_i64 = batch
        .column(batch.schema().index_of("value_i64").unwrap())
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert!(!value_i64.is_null(0));
    assert_eq!(value_i64.value(0), 0);
    assert!(value_i64.is_null(1));

    // value_str: row 0 = null, row 1 = "hello". Now dictionary-encoded like
    // field_name, so read it back through the dictionary view rather than a
    // bare StringArray downcast.
    let value_str = batch
        .column(batch.schema().index_of("value_str").unwrap())
        .as_dictionary::<Int32Type>();
    assert!(value_str.is_null(0));
    assert!(!value_str.is_null(1));
    let value_str_values = value_str.downcast_dict::<StringArray>().unwrap();
    assert_eq!(value_str_values.value(1), "hello");
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
                object_net_guid: None,
                group_path: "Bin".into(),
                handle: 0,
                field_name: None,
                compatible_checksum: None,
                bit_count: 256 * 8,
                raw_bits: Some(payload.clone().into()),
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
        .column(batches[0].schema().index_of("raw_bits").unwrap())
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(raw_bits.value(0), payload.as_slice());
}

#[test]
fn unresolved_class_net_cache_payload_marker_roundtrips_exact_bits() {
    let path = test_dir().join("unresolved_class_net_cache_payload.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        writer
            .push(FieldRecord {
                time_ms: 1234,
                packet_id: 56,
                channel_index: 7,
                actor_net_guid: 89,
                object_net_guid: Some(144),
                group_path: "AbilitiesAndBuffsComponent".into(),
                handle: u32::MAX,
                field_name: Some(UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME.into()),
                compatible_checksum: None,
                bit_count: 7,
                raw_bits: Some(vec![0x66].into()),
                value_i64: None,
                value_f64: None,
                value_bool: None,
                value_str: None,
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);

    let handle = batch
        .column(batch.schema().index_of("handle").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(handle.value(0), u32::MAX);

    let field_name = batch
        .column(batch.schema().index_of("field_name").unwrap())
        .as_dictionary::<Int32Type>()
        .downcast_dict::<StringArray>()
        .unwrap();
    assert_eq!(
        field_name.value(0),
        UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME
    );

    let bit_count = batch
        .column(batch.schema().index_of("bit_count").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(bit_count.value(0), 7);

    let raw_bits = batch
        .column(batch.schema().index_of("raw_bits").unwrap())
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(raw_bits.value(0), &[0x66]);
    assert_eq!(raw_bits.value(0)[0] >> 7, 0);

    for name in ["value_i64", "value_f64", "value_bool", "value_str"] {
        let column = batch.column(batch.schema().index_of(name).unwrap());
        assert!(column.is_null(0), "{name} must stay null");
    }
}

#[test]
fn field_multiple_row_groups() {
    // 200_000 rows with row_group_size=65_536 -> should produce at least 3 groups.
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

    // 200_000 / 65_536 = 3.05 -> expect at least 3 row groups.
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

// --- Movement Writer Tests ------------------------------------------------

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
                timestamp: 0,
                movement_state: 0,
                move_type: 0,
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

#[test]
fn movement_state_columns_keep_their_narrow_types() {
    // The three columns added after vel_z are u32/u8/u8 on the wire. Parquet
    // has no native 8-bit physical type -- it stores them as INT32 with an
    // INTEGER(8, false) logical annotation -- so the assertion that matters is
    // that a reader still hands them back as UInt8, not silently widened.
    let path = test_dir().join("movement_narrow_types.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..64 {
            writer.push(make_movement_record(i)).unwrap();
        }
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    let schema = batch.schema();

    assert_eq!(
        schema.field_with_name("timestamp").unwrap().data_type(),
        &DataType::UInt32
    );
    for name in ["movement_state", "move_type"] {
        let field = schema.field_with_name(name).unwrap();
        assert_eq!(field.data_type(), &DataType::UInt8, "{name} was widened");
    }
    // The movement table is dense by contract; python_interop.py asserts the
    // same thing over the whole schema.
    for name in ["timestamp", "movement_state", "move_type"] {
        assert!(
            !schema.field_with_name(name).unwrap().is_nullable(),
            "{name} must not be nullable"
        );
    }

    // Appended after vel_z, not interleaved: the existing positional readers
    // in this file (column(3) = pos_x, column(8) = vel_x) must keep working.
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        vec![
            "time_ms",
            "packet_id",
            "character_net_guid",
            "pos_x",
            "pos_y",
            "pos_z",
            "yaw",
            "pitch",
            "vel_x",
            "vel_y",
            "vel_z",
            "timestamp",
            "movement_state",
            "move_type",
        ]
    );

    // mode_flags is deliberately not a column: vrf_movement assigns it from
    // the same local as movement_state, so it could only ever duplicate it.
    assert!(schema.field_with_name("mode_flags").is_err());
}

#[test]
fn movement_new_columns_roundtrip_values() {
    let path = test_dir().join("movement_new_columns_values.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = MovementWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..20 {
            writer.push(make_movement_record(i)).unwrap();
        }
        // Boundary row: the widest value each column can hold.
        let mut extreme = make_movement_record(20);
        extreme.timestamp = u32::MAX;
        extreme.movement_state = u8::MAX;
        extreme.move_type = 1;
        writer.push(extreme).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 21);

    let timestamp = batch
        .column(batch.schema().index_of("timestamp").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    let movement_state = batch
        .column(batch.schema().index_of("movement_state").unwrap())
        .as_any()
        .downcast_ref::<UInt8Array>()
        .unwrap();
    let move_type = batch
        .column(batch.schema().index_of("move_type").unwrap())
        .as_any()
        .downcast_ref::<UInt8Array>()
        .unwrap();

    for i in 0..20usize {
        let expected = make_movement_record(i as u32);
        assert_eq!(timestamp.value(i), expected.timestamp, "row {i} timestamp");
        assert_eq!(
            movement_state.value(i),
            expected.movement_state,
            "row {i} movement_state"
        );
        assert_eq!(move_type.value(i), expected.move_type, "row {i} move_type");
    }

    assert_eq!(timestamp.value(20), u32::MAX);
    assert_eq!(movement_state.value(20), u8::MAX);
    assert_eq!(move_type.value(20), 1);

    // The helper must actually vary these, or the loop above proves nothing.
    let distinct_states: std::collections::BTreeSet<u8> =
        (0..20).map(|i| movement_state.value(i)).collect();
    assert!(
        distinct_states.len() > 1,
        "movement_state did not vary across rows"
    );
}

/// Write interop files for the Python verification step (requirement section 6).
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

// ---------------------------------------------------------------------------
// net_guids table
// ---------------------------------------------------------------------------

use vrf_export::{NetGuidRecord, NetGuidWriter};

#[test]
fn net_guid_roundtrip_preserves_outer_chain() {
    let path = test_dir().join("net_guid_roundtrip.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = NetGuidWriter::with_row_group_size(file, 1024).unwrap();
        // A weapon actor: no outer.
        writer
            .push(NetGuidRecord {
                net_guid: 2910,
                path: "/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C"
                    .into(),
                outer_net_guid: None,
            })
            .unwrap();
        // Its FiringState subobject: outer points back at the weapon.
        writer
            .push(NetGuidRecord {
                net_guid: 3086,
                path: "FiringState".into(),
                outer_net_guid: Some(2910),
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 2);

    let net_guid = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(net_guid.value(0), 2910);
    assert_eq!(net_guid.value(1), 3086);

    let path_col = batch.column(1).as_dictionary::<Int32Type>();
    let path_values = path_col.downcast_dict::<StringArray>().unwrap();
    assert_eq!(path_values.value(1), "FiringState");

    let outer = batch
        .column(2)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    // A GUID with no declared outer must be null, not 0 -- 0 is a real
    // sentinel meaning "invalid GUID" and must stay distinguishable.
    assert!(outer.is_null(0));
    assert_eq!(outer.value(1), 2910);
}

#[test]
fn field_object_net_guid_roundtrips_and_is_nullable() {
    // A content block can describe the actor itself or one of its subobjects.
    // Without the subobject GUID every ItemSlot on a character collapses onto
    // one key downstream, so a player appears to hold a single item.
    // `None` means "this block described the actor", which must stay distinct
    // from any real GUID -- including 0.
    let path = test_dir().join("field_object_net_guid.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        let mut actor_block = make_field_record(1);
        actor_block.object_net_guid = None;
        let mut subobject_block = make_field_record(2);
        subobject_block.object_net_guid = Some(4242);
        writer.push(actor_block).unwrap();
        writer.push(subobject_block).unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    let idx = batch.schema().index_of("object_net_guid").unwrap();
    let col = batch
        .column(idx)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert!(col.is_null(0), "actor blocks carry no subobject GUID");
    assert_eq!(col.value(1), 4242);
}

#[test]
fn net_guid_push_finish_empty() {
    let path = test_dir().join("net_guid_empty.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let writer = NetGuidWriter::new(file).unwrap();
        writer.finish().unwrap();
    }
    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 0);
}

// ---------------------------------------------------------------------------
// events table
// ---------------------------------------------------------------------------

use vrf_export::{EventRecord, EventWriter};

/// The first `roundStarted` payload from the reference replay, byte for byte.
/// It carries an embedded 0x00 and a tail that is not valid text, which is the
/// point: this column has to survive bytes that are not a string.
const REFERENCE_EVENT_PAYLOAD: [u8; 46] = [
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1E, 0x00, 0x00, 0x00, b'E', b'R', b'e', b'p',
    b'l', b'a', b'y', b'E', b'v', b'e', b'n', b't', b'G', b'r', b'o', b'u', b'p', b':', b':', b'R',
    b'o', b'u', b'n', b'd', b'S', b't', b'a', b'r', b't', 0x00, 0x22, 0xC0, 0x7F, 0x3D,
];

#[test]
fn event_roundtrip_preserves_payload_bytes_exactly() {
    let path = test_dir().join("event_roundtrip.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = EventWriter::with_row_group_size(file, 1024).unwrap();
        writer
            .push(EventRecord {
                id: "02d4d478_DC4D6C49E0C640FD814D88134F0A8642".into(),
                group: "roundStarted".into(),
                metadata: "0".into(),
                time1: 62,
                time2: 62,
                payload_size: REFERENCE_EVENT_PAYLOAD.len() as i32,
                raw_payload: REFERENCE_EVENT_PAYLOAD.to_vec(),
                word0: None,
                word1: None,
            })
            .unwrap();
        // A second group, so the dictionary column carries more than one value.
        writer
            .push(EventRecord {
                id: "02d4d478_0B756A9C4B10407DB9D3A4093C057D43".into(),
                group: "characterDeath".into(),
                metadata: String::new(),
                time1: 50402,
                time2: 50402,
                payload_size: 3,
                raw_payload: vec![0x00, 0xFF, 0x80],
                word0: None,
                word1: None,
            })
            .unwrap();
        writer.finish().unwrap();
    }

    let batches = read_all_batches(&path);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 2);

    let id = batch
        .column(batch.schema().index_of("id").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(id.value(0), "02d4d478_DC4D6C49E0C640FD814D88134F0A8642");

    let group = batch
        .column(batch.schema().index_of("group").unwrap())
        .as_dictionary::<Int32Type>();
    let group_values = group.downcast_dict::<StringArray>().unwrap();
    assert_eq!(group_values.value(0), "roundStarted");
    assert_eq!(group_values.value(1), "characterDeath");

    // An event with no metadata carries an empty string, not a null.
    let metadata = batch
        .column(batch.schema().index_of("metadata").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(!metadata.is_null(1));
    assert_eq!(metadata.value(0), "0");
    assert_eq!(metadata.value(1), "");

    let time1 = batch
        .column(batch.schema().index_of("time1").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    let time2 = batch
        .column(batch.schema().index_of("time2").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(time1.value(1), 50402);
    assert_eq!(time2.value(1), 50402);

    let payload_size = batch
        .column(batch.schema().index_of("payload_size").unwrap())
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(payload_size.value(0), 46);
    assert_eq!(payload_size.value(1), 3);

    // The undecoded payload is the whole point of the table: every byte, in
    // order, including the embedded 0x00 and the bytes that are not text.
    let raw = batch
        .column(batch.schema().index_of("raw_payload").unwrap())
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(raw.value(0), REFERENCE_EVENT_PAYLOAD);
    assert_eq!(raw.value(1), [0x00, 0xFF, 0x80]);
    // The declared size and the stored blob must agree row for row.
    for i in 0..batch.num_rows() {
        assert_eq!(payload_size.value(i) as usize, raw.value(i).len());
    }
}

#[test]
fn event_multiple_row_groups() {
    let path = test_dir().join("event_multi_row_group.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = EventWriter::with_row_group_size(file, 64).unwrap();
        for i in 0..200u32 {
            let group = if i % 2 == 0 {
                "characterDeath"
            } else {
                "spikePlanted"
            };
            writer
                .push(EventRecord {
                    id: format!("id_{i}"),
                    group: group.into(),
                    metadata: String::new(),
                    time1: i * 1000,
                    time2: i * 1000,
                    payload_size: 4,
                    raw_payload: i.to_le_bytes().to_vec(),
                    word0: None,
                    word1: None,
                })
                .unwrap();
        }
        writer.finish().unwrap();
    }

    let file = fs::File::open(&path).unwrap();
    let reader = SerializedFileReader::new(file).unwrap();
    assert!(reader.metadata().num_row_groups() > 1);

    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 200);
}

#[test]
fn event_push_finish_empty() {
    let path = test_dir().join("event_empty.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let writer = EventWriter::new(file).unwrap();
        writer.finish().unwrap();
    }
    let batches = read_all_batches(&path);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 0);
}

/// The replay's own `compatible_checksum` survives the round trip, nulls
/// included.
///
/// It is what tells "this field is legitimately undescribed" apart from "this
/// field has a checksum the overlay never learned" -- the Phoenix case, where a
/// whole class was missing from the table and 2,791 rows read null with decode
/// errors at 0. Without the checksum in the export those two look identical
/// offline, and the only reason Phoenix was found at all is that a sibling
/// class happened to share its RPC name.
#[test]
fn compatible_checksum_round_trips_with_its_nulls() {
    let path = test_dir().join("checksum_roundtrip.parquet");
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = FieldWriter::with_row_group_size(file, 1024).unwrap();
        for i in 0..8u32 {
            let mut r = make_field_record(i);
            // Odd rows model a handle the replay declares no checksum for.
            r.compatible_checksum = if i % 2 == 0 {
                Some(1_000_000 + i)
            } else {
                None
            };
            writer.push(r).unwrap();
        }
        writer.finish().unwrap();
    }

    let file = fs::File::open(&path).unwrap();
    let batch = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let checksum = batch
        .column(batch.schema().index_of("compatible_checksum").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    for i in 0..8usize {
        if i % 2 == 0 {
            assert_eq!(checksum.value(i), 1_000_000 + i as u32, "row {i}");
        } else {
            assert!(checksum.is_null(i), "row {i} should be null");
        }
    }
}
