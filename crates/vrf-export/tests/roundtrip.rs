//! Round-trip tests for the Parquet writers: values and nulls survive, row
//! groups are cut at the row-group size, and each file's dictionary pages match
//! its table's `DICTIONARY_COLUMNS`. Gated on the five main table features (the
//! default), so `--no-default-features` still builds this target.

#![cfg(all(
    feature = "fields",
    feature = "movement",
    feature = "actors",
    feature = "net-guids",
    feature = "events"
))]

use std::fs;
use std::path::{Path, PathBuf};

use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, Int32Type, Int64Type, UInt8Type, UInt32Type};
use arrow_array::{Array, ArrayAccessor, ArrayRef, RecordBatch, StringArray};
use arrow_schema::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};

use vrf_export::{
    ActorRecord, ActorsTable, EventRecord, EventsTable, FieldRecord, FieldWriter, FieldsTable,
    MovementRecord, MovementTable, MovementWriter, NetGuidRecord, NetGuidsTable, Table,
    TableWriter,
};

/// Test output directory: `VRFKIT_INTEROP_DIR` when set, used as the exact
/// root (CI's interop step reads `<root>/interop`), else `<target>/tmp/vrf-export`,
/// separate per checkout unless the target directory is shared.
fn test_dir() -> PathBuf {
    let dir = std::env::var_os("VRFKIT_INTEROP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vrf-export"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A FieldRecord whose values follow from `i`.
fn make_field_record(i: u32) -> FieldRecord {
    FieldRecord {
        time_ms: i * 10,
        packet_id: i,
        channel_index: i % 8,
        actor_net_guid: 1000 + (i % 20),
        // Every third record describes the actor itself, the rest a subobject.
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

/// A MovementRecord whose values follow from `i`.
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
        // All three vary, so a round trip tells a real copy from a constant.
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
fn read_all_batches(path: &Path) -> Vec<RecordBatch> {
    let file = fs::File::open(path).unwrap();
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    reader.collect::<Result<Vec<_>, _>>().unwrap()
}

/// Write `rows` through table `T`'s writer at `row_group_size`, one `push`
/// per row or all of them through one `push_batch`, and return the path.
fn write_rows<T: Table>(
    name: &str,
    row_group_size: usize,
    rows: impl IntoIterator<Item = T::Row>,
    one_batch: bool,
) -> PathBuf {
    let path = test_dir().join(format!("{name}.parquet"));
    let file = fs::File::create(&path).unwrap();
    let mut writer = TableWriter::<T, fs::File>::with_row_group_size(file, row_group_size).unwrap();
    if one_batch {
        writer.push_batch(rows).unwrap();
    } else {
        for row in rows {
            writer.push(row).unwrap();
        }
    }
    writer.finish().unwrap();
    path
}

/// Write `rows` through table `T` at 1,024 rows per row group, one `push` per
/// row, and read the file back as the single batch it holds.
fn roundtrip<T: Table>(name: &str, rows: impl IntoIterator<Item = T::Row>) -> RecordBatch {
    let mut batches = read_all_batches(&write_rows::<T>(name, 1024, rows, false));
    assert_eq!(batches.len(), 1, "{name}: one batch expected");
    batches.remove(0)
}

/// The column named `name`.
fn col<'a>(batch: &'a RecordBatch, name: &str) -> &'a ArrayRef {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column named {name}"))
}

// --- fields ---------------------------------------------------------------

#[test]
fn field_roundtrip_basic() {
    let batch = roundtrip::<FieldsTable>("field_roundtrip_basic", (0..50).map(make_field_record));
    assert_eq!(batch.num_rows(), 50);

    let time_ms = col(&batch, "time_ms").as_primitive::<UInt32Type>();
    assert_eq!(time_ms.value(0), 0);
    assert_eq!(time_ms.value(1), 10);
    assert_eq!(time_ms.value(49), 490);
}

#[test]
fn field_null_preservation() {
    let batch = roundtrip::<FieldsTable>(
        "field_null_preservation",
        [
            make_field_record(0),
            FieldRecord {
                // The top handle: array-truncation rows may use it.
                handle: u32::MAX,
                field_name: Some("Health".into()),
                raw_bits: Some(vec![0xAB].into()),
                value_f64: None,
                value_str: Some("hello".into()),
                ..make_field_record(1)
            },
        ],
    );

    let handle = col(&batch, "handle").as_primitive::<UInt32Type>();
    assert_eq!(handle.value(1), u32::MAX);

    let field_name = col(&batch, "field_name").as_dictionary::<Int32Type>();
    assert!(field_name.is_null(0));
    assert!(!field_name.is_null(1));
    let field_name_values = field_name.downcast_dict::<StringArray>().unwrap();
    assert_eq!(field_name_values.value(1), "Health");

    let raw_bits = col(&batch, "raw_bits").as_binary::<i32>();
    assert!(raw_bits.is_null(0));
    assert_eq!(raw_bits.value(1), &[0xAB]);

    let value_i64 = col(&batch, "value_i64").as_primitive::<Int64Type>();
    assert!(!value_i64.is_null(0));
    assert_eq!(value_i64.value(0), 0);
    assert!(value_i64.is_null(1));

    let value_str = col(&batch, "value_str").as_dictionary::<Int32Type>();
    assert!(value_str.is_null(0));
    assert!(!value_str.is_null(1));
    let value_str_values = value_str.downcast_dict::<StringArray>().unwrap();
    assert_eq!(value_str_values.value(1), "hello");

    for name in ["value_f64", "value_bool"] {
        assert!(
            col(&batch, name).is_null(0) && col(&batch, name).is_null(1),
            "{name}"
        );
    }
}

#[test]
fn field_binary_preservation() {
    // Every byte value, 0x00 included.
    let payload: Vec<u8> = (0..=255).collect();
    let batch = roundtrip::<FieldsTable>(
        "field_binary_preservation",
        [FieldRecord {
            bit_count: 256 * 8,
            raw_bits: Some(payload.clone().into()),
            ..make_field_record(0)
        }],
    );

    let raw_bits = col(&batch, "raw_bits").as_binary::<i32>();
    assert_eq!(raw_bits.value(0), payload.as_slice());
    let bit_count = col(&batch, "bit_count").as_primitive::<UInt32Type>();
    assert_eq!(bit_count.value(0), 256 * 8);
}

#[test]
fn raw_bits_capacity_keeps_exact_parquet_bytes_across_batches() {
    use std::sync::Arc;

    use arrow_array::BinaryArray;
    use arrow_schema::Schema;
    use vrf_export::ExportError;
    use vrf_export::writer::MAX_BUFFERED_ROWS;

    struct DefaultCapacityFields;
    impl Table for DefaultCapacityFields {
        type Row = FieldRecord;
        const DEFAULT_ROW_GROUP_SIZE: usize = FieldsTable::DEFAULT_ROW_GROUP_SIZE;
        const DICTIONARY_COLUMNS: &'static [&'static str] = FieldsTable::DICTIONARY_COLUMNS;

        fn schema() -> Arc<Schema> {
            FieldsTable::schema()
        }

        fn build_batch(rows: &[FieldRecord]) -> Result<RecordBatch, ExportError> {
            let batch = FieldsTable::build_batch(rows)?;
            let mut columns = batch.columns().to_vec();
            let reference = BinaryArray::from_iter(rows.iter().map(|row| row.raw_bits.as_deref()));
            columns[batch.schema().index_of("raw_bits").unwrap()] = Arc::new(reference);
            RecordBatch::try_new(batch.schema(), columns)
                .map_err(|error| ExportError::Parquet(error.into()))
        }
    }

    fn encode<T: Table<Row = FieldRecord>>(rows: &[FieldRecord], group_size: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut writer = TableWriter::<T, _>::with_row_group_size(&mut bytes, group_size).unwrap();
        writer.push_batch(rows.iter().cloned()).unwrap();
        writer.finish().unwrap();
        bytes
    }

    let rows: Vec<_> = (0..2 * MAX_BUFFERED_ROWS + 19)
        .map(|i| {
            let raw_bits = if i == MAX_BUFFERED_ROWS + 1 {
                // One payload exceeds even a whole default batch's reserve.
                Some(
                    (0..MAX_BUFFERED_ROWS * 16 + 31)
                        .map(|j| j as u8)
                        .collect::<Vec<_>>()
                        .into(),
                )
            } else {
                match i % 7 {
                    0 => None,
                    1 => Some(Vec::new().into()),
                    2 => Some(vec![i as u8].into()),
                    3 => Some(vec![i as u8; 16].into()),
                    4 => Some(vec![i as u8; 17].into()),
                    _ => Some((0..=255).collect::<Vec<_>>().into()),
                }
            };
            FieldRecord {
                bit_count: raw_bits
                    .as_ref()
                    .map_or(0, |raw: &smallvec::SmallVec<[u8; 16]>| {
                        (raw.len() * 8) as u32
                    }),
                raw_bits,
                ..make_field_record(i as u32)
            }
        })
        .collect();
    for group_size in [
        1_024,
        MAX_BUFFERED_ROWS + 7,
        FieldsTable::DEFAULT_ROW_GROUP_SIZE,
    ] {
        let actual = encode::<FieldsTable>(&rows, group_size);
        let expected = encode::<DefaultCapacityFields>(&rows, group_size);
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual == expected,
            "raw payload Parquet bytes differ at row-group size {group_size}"
        );
    }
}

#[test]
fn group_path_runs_keep_exact_parquet_bytes_across_batches() {
    use std::sync::Arc;

    use arrow_array::builder::StringDictionaryBuilder;
    use arrow_schema::Schema;
    use vrf_export::ExportError;
    use vrf_export::writer::MAX_BUFFERED_ROWS;

    struct ElementwiseFields;
    impl Table for ElementwiseFields {
        type Row = FieldRecord;
        const DEFAULT_ROW_GROUP_SIZE: usize = FieldsTable::DEFAULT_ROW_GROUP_SIZE;
        const DICTIONARY_COLUMNS: &'static [&'static str] = FieldsTable::DICTIONARY_COLUMNS;

        fn schema() -> Arc<Schema> {
            FieldsTable::schema()
        }

        fn build_batch(rows: &[FieldRecord]) -> Result<RecordBatch, ExportError> {
            let batch = FieldsTable::build_batch(rows)?;
            let mut columns = batch.columns().to_vec();
            let mut builder = StringDictionaryBuilder::<Int32Type>::with_capacity(
                rows.len(),
                256,
                rows.len() * 20,
            );
            builder.extend(rows.iter().map(|row| Some(row.group_path.as_ref())));
            columns[batch.schema().index_of("group_path").unwrap()] = Arc::new(builder.finish());
            RecordBatch::try_new(batch.schema(), columns)
                .map_err(|error| ExportError::Parquet(error.into()))
        }
    }

    fn encode<T: Table<Row = FieldRecord>>(rows: &[FieldRecord], group_size: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut writer = TableWriter::<T, _>::with_row_group_size(&mut bytes, group_size).unwrap();
        writer.push_batch(rows.iter().cloned()).unwrap();
        writer.finish().unwrap();
        bytes
    }

    let shared: Arc<str> = Arc::from("shared group");
    let rows: Vec<_> = (0..2 * MAX_BUFFERED_ROWS + 19)
        .map(|i| FieldRecord {
            // Runs cross both the Arrow batch and custom row-group boundary;
            // equal strings also arrive through different allocations.
            group_path: match i % 23 {
                0..=7 => shared.clone(),
                8..=15 => Arc::from("same text, distinct allocation"),
                16..=20 => Arc::from(""),
                _ => Arc::from(format!("singleton-{i}")),
            },
            ..make_field_record(i as u32)
        })
        .collect();
    for group_size in [
        1_024,
        MAX_BUFFERED_ROWS + 7,
        FieldsTable::DEFAULT_ROW_GROUP_SIZE,
    ] {
        let actual = encode::<FieldsTable>(&rows, group_size);
        let expected = encode::<ElementwiseFields>(&rows, group_size);
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual == expected,
            "Parquet bytes differ at row-group size {group_size}"
        );
    }
}

// --- movement -------------------------------------------------------------

#[test]
fn movement_keeps_column_order_narrow_types_and_exact_values() {
    // The last row holds each column's extreme and f32s that must survive bit
    // for bit.
    let extreme = MovementRecord {
        pos_x: std::f32::consts::PI,
        pos_y: -std::f32::consts::E,
        pos_z: f32::MIN_POSITIVE,
        vel_x: f32::MAX,
        vel_y: f32::MIN,
        timestamp: u32::MAX,
        movement_state: u8::MAX,
        move_type: 1,
        ..make_movement_record(20)
    };
    let rows: Vec<MovementRecord> = (0..20).map(make_movement_record).chain([extreme]).collect();
    let batch = roundtrip::<MovementTable>("movement_roundtrip", rows.iter().copied());
    assert_eq!(batch.num_rows(), rows.len());

    // Consumers address movement columns by position (column 3 = pos_x,
    // column 8 = vel_x); every column is dense.
    let schema = batch.schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        [
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
    for field in schema.fields() {
        assert!(
            !field.is_nullable(),
            "{} must not be nullable",
            field.name()
        );
    }
    // Parquet stores a u8 as INT32 with an INTEGER(8, false) annotation; a
    // reader must still hand it back as UInt8, not widened.
    assert_eq!(
        schema.field_with_name("timestamp").unwrap().data_type(),
        &DataType::UInt32
    );
    for name in ["movement_state", "move_type"] {
        let field = schema.field_with_name(name).unwrap();
        assert_eq!(field.data_type(), &DataType::UInt8, "{name} was widened");
    }

    let pos_x = col(&batch, "pos_x").as_primitive::<Float32Type>();
    let vel_x = col(&batch, "vel_x").as_primitive::<Float32Type>();
    let timestamp = col(&batch, "timestamp").as_primitive::<UInt32Type>();
    let movement_state = col(&batch, "movement_state").as_primitive::<UInt8Type>();
    let move_type = col(&batch, "move_type").as_primitive::<UInt8Type>();
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(pos_x.value(i).to_bits(), r.pos_x.to_bits(), "row {i} pos_x");
        assert_eq!(vel_x.value(i).to_bits(), r.vel_x.to_bits(), "row {i} vel_x");
        assert_eq!(timestamp.value(i), r.timestamp, "row {i} timestamp");
        assert_eq!(
            movement_state.value(i),
            r.movement_state,
            "row {i} movement_state"
        );
        assert_eq!(move_type.value(i), r.move_type, "row {i} move_type");
    }
}

/// Write the files python_interop.py verifies.
#[test]
fn write_interop_files() {
    let dir = test_dir().join("interop");
    fs::create_dir_all(&dir).unwrap();

    let file = fs::File::create(dir.join("fields_interop.parquet")).unwrap();
    let mut writer = FieldWriter::with_row_group_size(file, 4096).unwrap();
    writer
        .push_batch((0..10_000).map(make_field_record))
        .unwrap();
    writer.finish().unwrap();

    let file = fs::File::create(dir.join("movement_interop.parquet")).unwrap();
    let mut writer = MovementWriter::with_row_group_size(file, 8192).unwrap();
    writer
        .push_batch((0..50_000).map(make_movement_record))
        .unwrap();
    writer.finish().unwrap();
}

// --- actors ---------------------------------------------------------------

/// An ActorRecord whose values follow from `i` and whether it is an open.
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
        spawn_vx: is_open.then_some(-(i as f32)),
        spawn_vy: is_open.then_some(0.0),
        spawn_vz: is_open.then_some(3200.0),
    }
}

#[test]
fn actor_roundtrip_keeps_events_null_paths_and_open_only_spawns() {
    // Every third row (i % 3 == 2) is a close; class_path is None when i % 4 == 0.
    let batch = roundtrip::<ActorsTable>(
        "actor_roundtrip",
        (0..100).map(|i| make_actor_record(i, i % 3 != 2)),
    );
    assert_eq!(batch.num_rows(), 100);

    let time_ms = col(&batch, "time_ms").as_primitive::<UInt32Type>();
    assert_eq!(time_ms.value(0), 0);
    assert_eq!(time_ms.value(1), 16);

    let event = col(&batch, "event").as_string::<i32>();
    assert_eq!(event.value(0), "open");
    assert_eq!(event.value(2), "close");

    let class_path = col(&batch, "class_path").as_dictionary::<Int32Type>();
    assert!(class_path.is_null(0));
    assert!(!class_path.is_null(1));
    let class_path_values = class_path.downcast_dict::<StringArray>().unwrap();
    assert_eq!(
        class_path_values.value(1),
        "/Game/Characters/Agent_1/Agent_1_PC.Agent_1_PC_C"
    );

    // An open carries a spawn location and velocity; a close does not.
    let spawn_x = col(&batch, "spawn_x").as_primitive::<Float32Type>();
    assert_eq!(spawn_x.value(4), 40.0);
    assert!(spawn_x.is_null(5));
    let velocity = ["spawn_vx", "spawn_vy", "spawn_vz"]
        .map(|name| col(&batch, name).as_primitive::<Float32Type>().clone());
    assert_eq!(velocity.each_ref().map(|v| v.value(4)), [-4.0, 0.0, 3200.0]);
    assert!(velocity.iter().all(|v| v.is_null(5)));
}

// --- net_guids -------------------------------------------------------------

#[test]
fn net_guid_roundtrip_preserves_outer_chain() {
    let batch = roundtrip::<NetGuidsTable>(
        "net_guid_roundtrip",
        [
            // A weapon actor: no outer.
            NetGuidRecord {
                net_guid: 2910,
                path: "/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C"
                    .into(),
                outer_net_guid: None,
            },
            // Its FiringState subobject: outer points back at the weapon.
            NetGuidRecord {
                net_guid: 3086,
                path: "FiringState".into(),
                outer_net_guid: Some(2910),
            },
        ],
    );
    assert_eq!(batch.num_rows(), 2);

    let net_guid = col(&batch, "net_guid").as_primitive::<UInt32Type>();
    assert_eq!(net_guid.value(0), 2910);
    assert_eq!(net_guid.value(1), 3086);

    let path_col = col(&batch, "path").as_dictionary::<Int32Type>();
    let path_values = path_col.downcast_dict::<StringArray>().unwrap();
    assert_eq!(path_values.value(1), "FiringState");

    let outer = col(&batch, "outer_net_guid").as_primitive::<UInt32Type>();
    // No declared outer is null, not 0, the invalid-GUID sentinel.
    assert!(outer.is_null(0));
    assert_eq!(outer.value(1), 2910);
}

#[test]
fn field_object_net_guid_roundtrips_and_is_nullable() {
    // `None` (the actor itself) must stay distinct from any GUID, 0 included,
    // or a character's ItemSlots collapse onto one key downstream.
    let mut actor_block = make_field_record(1);
    actor_block.object_net_guid = None;
    let mut subobject_block = make_field_record(2);
    subobject_block.object_net_guid = Some(4242);
    let batch = roundtrip::<FieldsTable>("field_object_net_guid", [actor_block, subobject_block]);

    let object_net_guid = col(&batch, "object_net_guid").as_primitive::<UInt32Type>();
    assert!(
        object_net_guid.is_null(0),
        "actor blocks carry no subobject GUID"
    );
    assert_eq!(object_net_guid.value(1), 4242);
}

// --- events ---------------------------------------------------------------

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
    let batch = roundtrip::<EventsTable>(
        "event_roundtrip",
        [
            EventRecord {
                id: "02d4d478_DC4D6C49E0C640FD814D88134F0A8642".into(),
                group: "roundStarted".into(),
                metadata: "0".into(),
                time1: 62,
                time2: 62,
                payload_size: REFERENCE_EVENT_PAYLOAD.len() as i32,
                raw_payload: REFERENCE_EVENT_PAYLOAD.to_vec(),
                word0: None,
                word1: None,
                payload_tag: Some(2),
                payload_name: Some("EReplayEventGroup::RoundStart".into()),
                payload_seconds: Some(f32::from_bits(0x3D7F_C022)),
            },
            // A second group, so the dictionary column carries more than one value.
            EventRecord {
                id: "02d4d478_0B756A9C4B10407DB9D3A4093C057D43".into(),
                group: "characterDeath".into(),
                metadata: String::new(),
                time1: 50402,
                time2: 50402,
                payload_size: 3,
                raw_payload: vec![0x00, 0xFF, 0x80],
                word0: None,
                word1: None,
                payload_tag: None,
                payload_name: None,
                payload_seconds: None,
            },
        ],
    );
    assert_eq!(batch.num_rows(), 2);

    let id = col(&batch, "id").as_string::<i32>();
    assert_eq!(id.value(0), "02d4d478_DC4D6C49E0C640FD814D88134F0A8642");

    let group = col(&batch, "group").as_dictionary::<Int32Type>();
    let group_values = group.downcast_dict::<StringArray>().unwrap();
    assert_eq!(group_values.value(0), "roundStarted");
    assert_eq!(group_values.value(1), "characterDeath");

    // An event with no metadata carries an empty string, not a null.
    let metadata = col(&batch, "metadata").as_string::<i32>();
    assert!(!metadata.is_null(1));
    assert_eq!(metadata.value(0), "0");
    assert_eq!(metadata.value(1), "");

    let time1 = col(&batch, "time1").as_primitive::<UInt32Type>();
    let time2 = col(&batch, "time2").as_primitive::<UInt32Type>();
    assert_eq!(time1.value(1), 50402);
    assert_eq!(time2.value(1), 50402);

    let payload_size = col(&batch, "payload_size").as_primitive::<Int32Type>();
    assert_eq!(payload_size.value(0), 46);
    assert_eq!(payload_size.value(1), 3);

    // Every byte in order, the embedded 0x00 and the non-text tail included.
    let raw = col(&batch, "raw_payload").as_binary::<i32>();
    assert_eq!(raw.value(0), REFERENCE_EVENT_PAYLOAD);
    assert_eq!(raw.value(1), [0x00, 0xFF, 0x80]);
    // The declared size and the stored blob must agree row for row.
    for i in 0..batch.num_rows() {
        assert_eq!(payload_size.value(i) as usize, raw.value(i).len());
    }

    let payload_tag = col(&batch, "payload_tag").as_primitive::<UInt32Type>();
    let payload_name = col(&batch, "payload_name").as_string::<i32>();
    let payload_seconds = col(&batch, "payload_seconds").as_primitive::<Float32Type>();
    assert_eq!(payload_tag.value(0), 2);
    assert_eq!(payload_name.value(0), "EReplayEventGroup::RoundStart");
    assert_eq!(payload_seconds.value(0).to_bits(), 0x3D7F_C022);
    assert!(payload_tag.is_null(1));
    assert!(payload_name.is_null(1));
    assert!(payload_seconds.is_null(1));
}

/// The replay's `compatible_checksum` survives the round trip, nulls
/// included: null is a field the replay declares no checksum for.
#[test]
fn compatible_checksum_round_trips_with_its_nulls() {
    let rows = (0..8u32).map(|i| FieldRecord {
        // Odd rows model a handle the replay declares no checksum for.
        compatible_checksum: (i % 2 == 0).then_some(1_000_000 + i),
        ..make_field_record(i)
    });
    let batch = roundtrip::<FieldsTable>("checksum_roundtrip", rows);

    let checksum = col(&batch, "compatible_checksum").as_primitive::<UInt32Type>();
    for i in 0..8usize {
        if i % 2 == 0 {
            assert_eq!(checksum.value(i), 1_000_000 + i as u32, "row {i}");
        } else {
            assert!(checksum.is_null(i), "row {i} should be null");
        }
    }
}

// --- one writer serves every table: row groups, empty files, push_batch ---

/// An event row that varies with `i`, alternating between two groups.
fn make_event_record(i: u32) -> EventRecord {
    EventRecord {
        id: format!("id_{i}"),
        group: ["characterDeath", "spikePlanted"][(i % 2) as usize].into(),
        metadata: String::new(),
        time1: i * 1000,
        time2: i * 1000,
        payload_size: 4,
        raw_payload: i.to_le_bytes().to_vec(),
        word0: None,
        word1: None,
        payload_tag: None,
        payload_name: None,
        payload_seconds: None,
    }
}

/// Every row group's row count in file order, after checking that the whole
/// file reads back with that many rows.
fn row_groups(path: &Path) -> Vec<i64> {
    let reader = SerializedFileReader::new(fs::File::open(path).unwrap()).unwrap();
    let counts: Vec<i64> = reader
        .metadata()
        .row_groups()
        .iter()
        .map(|group| group.num_rows())
        .collect();
    let read_back: usize = read_all_batches(path)
        .iter()
        .map(RecordBatch::num_rows)
        .sum();
    assert_eq!(
        read_back as i64,
        counts.iter().sum::<i64>(),
        "{}",
        path.display()
    );
    counts
}

#[test]
fn row_groups_close_at_the_row_group_size_not_at_each_batch() {
    // Closing a group at every 8,192-row batch would give 24 groups of 8,192
    // and one of 3,392.
    const ROWS: u32 = 200_000;
    const _: () = assert!(vrf_export::writer::MAX_BUFFERED_ROWS < 65_536);
    let expected = vec![65_536, 65_536, 65_536, 3_392];

    let rows = (0..ROWS).map(make_field_record);
    let path = write_rows::<FieldsTable>("row_groups_fields", 65_536, rows, false);
    assert_eq!(row_groups(&path), expected, "fields");
    let rows = (0..ROWS).map(make_movement_record);
    let path = write_rows::<MovementTable>("row_groups_movement", 65_536, rows, false);
    assert_eq!(row_groups(&path), expected, "movement");
    let rows = (0..ROWS).map(make_event_record);
    let path = write_rows::<EventsTable>("row_groups_events", 65_536, rows, false);
    assert_eq!(row_groups(&path), expected, "events");
}

#[test]
fn a_writer_finished_with_no_rows_writes_a_readable_empty_file() {
    fn rows_in_empty_file<T: Table>(name: &str) -> i64 {
        let path = write_rows::<T>(name, T::DEFAULT_ROW_GROUP_SIZE, [], false);
        row_groups(&path).iter().sum()
    }
    assert_eq!(rows_in_empty_file::<FieldsTable>("empty_fields"), 0);
    assert_eq!(rows_in_empty_file::<MovementTable>("empty_movement"), 0);
    assert_eq!(rows_in_empty_file::<ActorsTable>("empty_actors"), 0);
    assert_eq!(rows_in_empty_file::<NetGuidsTable>("empty_net_guids"), 0);
    assert_eq!(rows_in_empty_file::<EventsTable>("empty_events"), 0);
}

/// Write rows `0..count` through `push_batch` and through one `push` per row,
/// at 128 rows per group so flushes fall inside the batch; assert the files
/// are byte-identical and return the row groups.
fn push_batch_against_push<T: Table>(name: &str, count: u32, make: fn(u32) -> T::Row) -> Vec<i64> {
    let batched = write_rows::<T>(&format!("{name}_batched"), 128, (0..count).map(make), true);
    let pushed = write_rows::<T>(&format!("{name}_pushed"), 128, (0..count).map(make), false);
    assert!(
        fs::read(&batched).unwrap() == fs::read(pushed).unwrap(),
        "{name}: push_batch wrote a different file"
    );
    row_groups(&batched)
}

#[test]
fn push_batch_writes_the_same_file_as_one_push_per_row() {
    assert_eq!(
        push_batch_against_push::<FieldsTable>("push_batch_fields", 300, make_field_record),
        vec![128, 128, 44]
    );
    assert_eq!(
        push_batch_against_push::<MovementTable>("push_batch_movement", 500, make_movement_record),
        vec![128, 128, 128, 116]
    );
}

// --- dictionary encoding is per column -------------------------------------

/// Each table's file has a dictionary for exactly its
/// `Table::DICTIONARY_COLUMNS`, read from the footer. Gated on the two features
/// the file-level gate lacks.
#[cfg(all(feature = "partials", feature = "checkpoint-context"))]
mod dictionary_encoding {
    use super::*;

    use std::collections::HashSet;
    use std::sync::Arc;

    use parquet::basic::Type as PhysicalType;
    use vrf_export::{
        CheckpointActorRecord, CheckpointActorsTable, CheckpointBlockRecord, CheckpointBlocksTable,
        CheckpointExportFieldRecord, CheckpointExportFieldsTable, CheckpointExportGroupRecord,
        CheckpointExportGroupsTable, CheckpointFieldRecord, CheckpointFieldsTable,
        CheckpointGuidEntriesTable, CheckpointGuidEntryRecord, CheckpointIdentity,
        CheckpointNetGuidRecord, CheckpointNetGuidsTable, PartialRecord, PartialsTable,
    };

    const ROWS: u32 = 8;

    /// Every listed name must be a real non-boolean column, listed once
    /// (parquet-rs silently ignores an unknown or BOOLEAN name), and every
    /// string column must be listed, as the docs promise.
    fn list_problems<T: Table>(table: &str) -> Vec<String> {
        let schema = T::schema();
        let mut problems = Vec::new();
        let mut seen = HashSet::new();
        for &name in T::DICTIONARY_COLUMNS {
            if !seen.insert(name) {
                problems.push(format!("{table}.{name}: listed twice"));
            }
            match schema.field_with_name(name) {
                Err(_) => problems.push(format!(
                    "{table}.{name}: no such column; parquet-rs ignores the name without an error"
                )),
                Ok(field) if field.data_type() == &DataType::Boolean => problems.push(format!(
                    "{table}.{name}: BOOLEAN, which parquet-rs never dictionary-encodes"
                )),
                Ok(_) => {}
            }
        }
        for field in schema.fields() {
            let is_string = match field.data_type() {
                DataType::Utf8 => true,
                DataType::Dictionary(_, value) => value.as_ref() == &DataType::Utf8,
                _ => false,
            };
            if is_string && !T::DICTIONARY_COLUMNS.contains(&field.name().as_str()) {
                problems.push(format!(
                    "{table}.{}: a string column missing from DICTIONARY_COLUMNS",
                    field.name()
                ));
            }
        }
        problems
    }

    /// Write `rows` through the real writer and return every column whose
    /// footer disagrees with `T::DICTIONARY_COLUMNS`. BOOLEAN columns are
    /// skipped (parquet-rs never gives them a dictionary, and the list check
    /// rejects one); an all-null column is reported rather than checked, so
    /// the result cannot depend on how an all-null chunk is written.
    fn footer_disagreements<T: Table>(table: &str, rows: Vec<T::Row>) -> Vec<String> {
        let name = format!("dictionary_encoding_{table}");
        let path = write_rows::<T>(&name, T::DEFAULT_ROW_GROUP_SIZE, rows, true);
        let reader = SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap();
        let metadata = reader.metadata();
        assert_eq!(
            metadata.num_row_groups(),
            1,
            "{table}: one row group expected"
        );

        let expected_checked = T::schema()
            .fields()
            .iter()
            .filter(|field| field.data_type() != &DataType::Boolean)
            .count();
        let mut problems = Vec::new();
        let mut checked = 0;
        for column in metadata.row_group(0).columns() {
            let name = column.column_descr().name();
            if column.column_type() == PhysicalType::BOOLEAN {
                continue;
            }
            let nulls = column
                .statistics()
                .and_then(|stats| stats.null_count_opt())
                .unwrap_or_else(|| panic!("{table}.{name}: the writer stopped writing statistics"));
            if nulls >= column.num_values() as u64 {
                problems.push(format!("{table}.{name}: the fixture has no non-null value"));
                continue;
            }
            checked += 1;
            let listed = T::DICTIONARY_COLUMNS.contains(&name);
            let has_dictionary = column.dictionary_page_offset().is_some();
            if listed != has_dictionary {
                problems.push(format!(
                    "{table}.{name}: DICTIONARY_COLUMNS {} it, but the file has {} dictionary page",
                    if listed { "lists" } else { "omits" },
                    if has_dictionary { "a" } else { "no" },
                ));
            }
        }
        if checked != expected_checked {
            problems.push(format!(
                "{table}: checked {checked} columns, the schema has {expected_checked} non-boolean"
            ));
        }
        problems
    }

    /// Both checks for one table: its list, then the file written from `rows`.
    fn check<T: Table>(table: &str, rows: Vec<T::Row>) -> Vec<String> {
        [
            list_problems::<T>(table),
            footer_disagreements::<T>(table, rows),
        ]
        .concat()
    }

    fn identity(i: u32) -> CheckpointIdentity {
        CheckpointIdentity {
            checkpoint_index: i / 4,
            checkpoint_id: Arc::from(format!("checkpoint-{}", i / 4)),
        }
    }

    fn field_rows() -> Vec<FieldRecord> {
        (0..ROWS)
            .map(|i| FieldRecord {
                // make_field_record never sets a checksum.
                compatible_checksum: (i % 2 == 0).then_some(1_000 + i),
                ..make_field_record(i)
            })
            .collect()
    }

    fn actor_rows() -> Vec<ActorRecord> {
        (0..ROWS)
            .map(|i| make_actor_record(i, i % 2 == 0))
            .collect()
    }

    fn net_guid_rows() -> Vec<NetGuidRecord> {
        (0..ROWS)
            .map(|i| NetGuidRecord {
                net_guid: 100 + i,
                path: format!("Path_{}", i % 3),
                outer_net_guid: (i % 2 == 0).then_some(i),
            })
            .collect()
    }

    fn event_rows() -> Vec<EventRecord> {
        (0..ROWS)
            .map(|i| EventRecord {
                id: format!("event-{i}"),
                group: format!("group{}", i % 2),
                metadata: format!("meta{}", i % 3),
                time1: 1_000 * i,
                time2: 1_000 * i + 7,
                payload_size: 4 * i as i32,
                raw_payload: vec![i as u8; 4 * i as usize],
                word0: (i % 2 == 0).then_some(i),
                word1: (i % 3 == 0).then_some(i + 1),
                payload_tag: (i % 2 == 1).then_some(2),
                payload_name: (i % 2 == 1).then(|| format!("name{i}")),
                payload_seconds: (i % 2 == 1).then_some(i as f32 * 0.5),
            })
            .collect()
    }

    fn partial_rows() -> Vec<PartialRecord> {
        (0..ROWS)
            .map(|i| PartialRecord {
                source: if i % 2 == 0 { "main" } else { "checkpoint" },
                checkpoint_id: (i % 2 == 1).then(|| format!("checkpoint-{i}")),
                payload_kind: "current_fragment",
                reason: "missing_initial",
                source_packet_id: i as i32,
                source_payload_bit_offset: 64 * i64::from(i),
                rejection_packet_id: (i % 2 == 0).then_some(i as i32 + 1),
                channel_index: i % 3,
                channel_sequence: i as i32,
                open: i % 2 == 0,
                close: false,
                dormant: false,
                replication_paused: false,
                reliable: true,
                partial: true,
                partial_initial: i == 0,
                partial_final: i == ROWS - 1,
                has_package_map_exports: false,
                has_must_be_mapped_guids: false,
                close_reason: (i % 2) as u8,
                source_payload_bit_count: 8 * i as i32,
                bit_count: 8 * u64::from(i),
                raw_bits: vec![i as u8; i as usize + 1],
            })
            .collect()
    }

    fn checkpoint_block_rows() -> Vec<CheckpointBlockRecord> {
        (0..ROWS)
            .map(|i| CheckpointBlockRecord {
                checkpoint: identity(i),
                block_index: i,
                time_ms: 7 * i,
                packet_id: i,
                channel_index: i % 3,
                actor_net_guid: 200 + i,
                object_net_guid: (i % 2 == 0).then_some(300 + i),
                class_net_guid: (i % 2 == 1).then_some(400 + i),
                outer_net_guid: (i % 3 == 0).then_some(500 + i),
                has_rep_layout: i % 2 == 0,
                is_actor: i % 3 == 0,
                is_deleted: false,
                is_stably_named: i % 2 == 1,
                delete_flags: (i % 2) as u8,
                resolved_group_path: Arc::from(format!("Group_{}", i % 2)),
                group_resolution_source: "replay_declared_group",
                group_declared: true,
                resolution_memo_hit: i % 2 == 0,
                function_count: i % 4,
                function_count_source: "rep_layout_not_applicable",
                actor_archetype_path: Some(format!("Archetype_{}", i % 2)),
                actor_archetype_outer_path: Some("ArchetypeOuter".into()),
                actor_guid_path: Some(format!("Actor_{i}")),
                class_guid_path: Some("Class".into()),
                object_guid_path: (i % 2 == 0).then(|| format!("Object_{i}")),
                object_outer_path: (i % 2 == 1).then(|| format!("Outer_{i}")),
                field_row_start: 16 * u64::from(i),
                field_row_count: i % 5,
            })
            .collect()
    }

    fn checkpoint_guid_entry_rows() -> Vec<CheckpointGuidEntryRecord> {
        (0..ROWS)
            .map(|i| CheckpointGuidEntryRecord {
                checkpoint: identity(i),
                ordinal: i,
                net_guid: 600 + i,
                outer_net_guid: 600 + i / 2,
                path_is_string: i % 2 == 0,
                literal_path: (i % 2 == 0).then(|| format!("Literal_{i}")),
                name_index: (i % 2 == 1).then_some(i),
                flags: (i % 4) as u8,
            })
            .collect()
    }

    fn checkpoint_export_group_rows() -> Vec<CheckpointExportGroupRecord> {
        (0..ROWS)
            .map(|i| CheckpointExportGroupRecord {
                checkpoint: identity(i),
                ordinal: i,
                path_name_index: 10 + i,
                group_path: format!("ExportGroup_{i}"),
                declared_slots: i % 3,
            })
            .collect()
    }

    fn checkpoint_export_field_rows() -> Vec<CheckpointExportFieldRecord> {
        (0..ROWS)
            .map(|i| CheckpointExportFieldRecord {
                checkpoint: identity(i),
                group_ordinal: i / 2,
                path_name_index: 10 + i,
                slot: i,
                handle: i + 1,
                compatible_checksum: 7_000 + i,
                rendered_name: format!("Rendered_{i}"),
                exported_flag: (i % 2) as u8,
                fname_kind: (i % 3) as u8,
                fname_base: (i % 2 == 0).then(|| format!("Base_{i}")),
                fname_index: (i % 2 == 1).then_some(i),
                fname_number: (i % 3 == 0).then_some(i as i32),
            })
            .collect()
    }

    #[test]
    fn every_table_lists_real_columns_and_writes_a_dictionary_for_exactly_them() {
        let fields = field_rows();
        let actors = actor_rows();
        let net_guids = net_guid_rows();
        let problems = [
            check::<FieldsTable>("fields", fields.clone()),
            check::<MovementTable>("movement", (0..ROWS).map(make_movement_record).collect()),
            check::<ActorsTable>("actors", actors.clone()),
            check::<NetGuidsTable>("net_guids", net_guids.clone()),
            check::<EventsTable>("events", event_rows()),
            check::<PartialsTable>("partials", partial_rows()),
            check::<CheckpointFieldsTable>(
                "checkpoint_fields",
                fields
                    .into_iter()
                    .zip(0..)
                    .map(|(field, i)| CheckpointFieldRecord {
                        checkpoint: identity(i),
                        field,
                    })
                    .collect(),
            ),
            check::<CheckpointActorsTable>(
                "checkpoint_actors",
                actors
                    .into_iter()
                    .zip(0..)
                    .map(|(actor, i)| CheckpointActorRecord {
                        checkpoint: identity(i),
                        actor,
                    })
                    .collect(),
            ),
            check::<CheckpointNetGuidsTable>(
                "checkpoint_net_guids",
                net_guids
                    .into_iter()
                    .zip(0..)
                    .map(|(net_guid, i)| CheckpointNetGuidRecord {
                        checkpoint: identity(i),
                        net_guid,
                    })
                    .collect(),
            ),
            check::<CheckpointBlocksTable>("checkpoint_blocks", checkpoint_block_rows()),
            check::<CheckpointGuidEntriesTable>(
                "checkpoint_guid_entries",
                checkpoint_guid_entry_rows(),
            ),
            check::<CheckpointExportGroupsTable>(
                "checkpoint_export_groups",
                checkpoint_export_group_rows(),
            ),
            check::<CheckpointExportFieldsTable>(
                "checkpoint_export_fields",
                checkpoint_export_field_rows(),
            ),
        ]
        .concat();
        assert!(
            problems.is_empty(),
            "{} problem(s) with DICTIONARY_COLUMNS:\n  {}",
            problems.len(),
            problems.join("\n  ")
        );
    }
}
