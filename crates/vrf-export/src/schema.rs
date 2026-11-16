//! Arrow schemas of the 13 export tables, public so a consumer can check a
//! file it did not write. Every schema compiles under `parquet` alone; a
//! table's array builder compiles only with the features that write it.

use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

/// Declares a table's columns once, in file order. `$schema()` is the Arrow
/// schema and `$row::columns(rows)` builds the matching arrays, so a column's
/// name, type and value cannot come apart (`RecordBatch::try_new` checks only
/// types). `name?:` is nullable; `dict(d, b)` is `Dictionary<Int32, Utf8>`, its
/// builder sized for `d` distinct values and `b` bytes a row. `dict_runs` has
/// the same type, batching adjacent equal `Arc<str>` values into one lookup.
macro_rules! columns {
    (
        $(#[$doc:meta])* $vis:vis fn $schema:ident;
        #[cfg($cfg:meta)] $row:ident {
            $($name:ident $(?: $opt:ident)? $(: $kind:ident)? $(($($size:literal),*))?,)+
        }
    ) => {
        $(#[$doc])*
        $vis fn $schema() -> Schema {
            Schema::new(vec![$(Field::new(
                stringify!($name),
                column!(@type $($opt)? $($kind)? $(($($size),*))?),
                column!(@nullable $($opt)?),
            )),+])
        }

        #[cfg($cfg)]
        impl crate::record::$row {
            pub(crate) fn columns<'a>(
                rows: impl ExactSizeIterator<Item = &'a Self> + Clone,
            ) -> Vec<arrow_array::ArrayRef> {
                vec![$(column!(rows $name $(@opt $opt)? $(@kind $kind)? $(($($size),*))?)),+]
            }
        }
    };
}

/// One column of [`columns!`]: its Arrow type, its nullability, or its array.
/// Non-null columns use `from_iter_values` and nullable ones `from_iter`, the
/// calls the file bytes are pinned to.
macro_rules! column {
    (@nullable) => { false };
    (@nullable $kind:ident) => { true };
    (@type bool) => { DataType::Boolean };
    (@type str) => { DataType::Utf8 };
    (@type bytes) => { DataType::Binary };
    (@type dict($distinct:literal, $bytes:literal)) => {
        DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8))
    };
    (@type dict_runs($distinct:literal, $bytes:literal)) => {
        column!(@type dict($distinct, $bytes))
    };
    (@type $prim:ident) => {
        <column!(@prim $prim) as arrow_array::types::ArrowPrimitiveType>::DATA_TYPE
    };
    (@prim u8) => { arrow_array::types::UInt8Type };
    (@prim u32) => { arrow_array::types::UInt32Type };
    (@prim u64) => { arrow_array::types::UInt64Type };
    (@prim i32) => { arrow_array::types::Int32Type };
    (@prim i64) => { arrow_array::types::Int64Type };
    (@prim f32) => { arrow_array::types::Float32Type };
    (@prim f64) => { arrow_array::types::Float64Type };
    (@array $array:expr) => { Arc::new($array) as arrow_array::ArrayRef };
    (@dict $rows:ident, $distinct:literal, $bytes:literal, $values:expr) => {{
        let mut builder =
            arrow_array::builder::StringDictionaryBuilder::<arrow_array::types::Int32Type>::with_capacity(
                $rows.len(),
                $distinct,
                $rows.len() * $bytes,
            );
        builder.extend($values);
        column!(@array builder.finish())
    }};
    // Every bool form reaches the same builder through arrow's `BooleanAdapter`.
    ($rows:ident $name:ident $(@opt)? $(@kind)? bool) => {
        column!(@array arrow_array::BooleanArray::from_iter($rows.clone().map(|r| r.$name)))
    };
    ($rows:ident $name:ident @kind str) => {
        column!(@array arrow_array::StringArray::from_iter_values($rows.clone().map(|r| &*r.$name)))
    };
    ($rows:ident $name:ident @opt str) => {
        column!(@array arrow_array::StringArray::from_iter($rows.clone().map(|r| r.$name.as_deref())))
    };
    ($rows:ident $name:ident @kind bytes) => {
        column!(@array arrow_array::BinaryArray::from_iter_values($rows.clone().map(|r| &*r.$name)))
    };
    ($rows:ident $name:ident @opt bytes) => {{
        // Start with one inline raw payload per buffered row. The reservation
        // is bounded; larger payloads still grow without truncation.
        let byte_capacity = $rows.len().min(crate::writer::MAX_BUFFERED_ROWS) * 16;
        let mut builder = arrow_array::builder::BinaryBuilder::with_capacity(
            $rows.len(),
            byte_capacity,
        );
        builder.extend($rows.clone().map(|r| r.$name.as_deref()));
        column!(@array builder.finish())
    }};
    ($rows:ident $name:ident @kind dict($distinct:literal, $bytes:literal)) => {
        column!(@dict $rows, $distinct, $bytes, $rows.clone().map(|r| Some(&*r.$name)))
    };
    ($rows:ident $name:ident @kind dict_runs($distinct:literal, $bytes:literal)) => {
        column!(@array dictionary_runs($rows.clone().map(|r| &r.$name), $distinct, $bytes))
    };
    ($rows:ident $name:ident @opt dict($distinct:literal, $bytes:literal)) => {
        column!(@dict $rows, $distinct, $bytes, $rows.clone().map(|r| r.$name.as_deref()))
    };
    ($rows:ident $name:ident @kind $prim:ident) => {
        column!(@array arrow_array::PrimitiveArray::<column!(@prim $prim)>::from_iter_values(
            $rows.clone().map(|r| r.$name),
        ))
    };
    ($rows:ident $name:ident @opt $prim:ident) => {
        column!(@array arrow_array::PrimitiveArray::<column!(@prim $prim)>::from_iter(
            $rows.clone().map(|r| r.$name),
        ))
    };
}

/// Runs preserve first-occurrence dictionary order and append one key per row.
/// Only group paths use this: their adjacent repetition avoids repeated hashes.
#[cfg(any(feature = "fields", feature = "checkpoint-context"))]
fn dictionary_runs<'a>(
    mut values: impl ExactSizeIterator<Item = &'a Arc<str>>,
    distinct: usize,
    bytes_per_row: usize,
) -> arrow_array::DictionaryArray<arrow_array::types::Int32Type> {
    let mut builder =
        arrow_array::builder::StringDictionaryBuilder::<arrow_array::types::Int32Type>::with_capacity(
            values.len(),
            distinct,
            values.len() * bytes_per_row,
        );
    if let Some(mut previous) = values.next() {
        let mut count = 1;
        for value in values {
            if Arc::ptr_eq(previous, value) || previous.as_ref() == value.as_ref() {
                count += 1;
            } else {
                builder.append_values(previous.as_ref(), count);
                previous = value;
                count = 1;
            }
        }
        builder.append_values(previous.as_ref(), count);
    }
    builder.finish()
}

/// Each schema's `Arc` twin, which `ArrowWriter` takes: one line a pairing, so
/// a mismatched pair (all return `Arc<Schema>`) is visible.
macro_rules! schema_refs {
    ($($ref_fn:ident => $schema_fn:ident),+ $(,)?) => {
        $(
            pub fn $ref_fn() -> Arc<Schema> {
                Arc::new($schema_fn())
            }
        )+
    };
}

schema_refs! {
    checkpoint_fields_schema_ref => checkpoint_fields_schema,
    checkpoint_actors_schema_ref => checkpoint_actors_schema,
    checkpoint_net_guids_schema_ref => checkpoint_net_guids_schema,
    checkpoint_blocks_schema_ref => checkpoint_blocks_schema,
    checkpoint_guid_entries_schema_ref => checkpoint_guid_entries_schema,
    checkpoint_export_groups_schema_ref => checkpoint_export_groups_schema,
    checkpoint_export_fields_schema_ref => checkpoint_export_fields_schema,
    fields_schema_ref => fields_schema,
    movement_schema_ref => movement_schema,
    actors_schema_ref => actors_schema,
    net_guids_schema_ref => net_guids_schema,
    events_schema_ref => events_schema,
    partials_schema_ref => partials_schema,
}

columns! {
    /// The two identity columns every checkpoint schema starts with.
    fn identity_schema;
    #[cfg(feature = "checkpoint-context")]
    CheckpointIdentity {
        checkpoint_index: u32,
        checkpoint_id: str,
    }
}

/// `base` with the two checkpoint identity columns in front of its own.
fn checkpoint_schema(base: Schema) -> Schema {
    Schema::new([identity_schema().fields().to_vec(), base.fields().to_vec()].concat())
}

pub fn checkpoint_fields_schema() -> Schema {
    checkpoint_schema(fields_schema())
}

pub fn checkpoint_actors_schema() -> Schema {
    checkpoint_schema(actors_schema())
}

pub fn checkpoint_net_guids_schema() -> Schema {
    checkpoint_schema(net_guids_schema())
}

pub fn checkpoint_blocks_schema() -> Schema {
    checkpoint_schema(blocks_schema())
}

pub fn checkpoint_guid_entries_schema() -> Schema {
    checkpoint_schema(guid_entries_schema())
}

pub fn checkpoint_export_groups_schema() -> Schema {
    checkpoint_schema(export_groups_schema())
}

pub fn checkpoint_export_fields_schema() -> Schema {
    checkpoint_schema(export_fields_schema())
}

columns! {
    fn blocks_schema;
    #[cfg(feature = "checkpoint-context")]
    CheckpointBlockRecord {
        block_index: u32,
        time_ms: u32,
        packet_id: u32,
        channel_index: u32,
        actor_net_guid: u32,
        object_net_guid?: u32,
        class_net_guid?: u32,
        outer_net_guid?: u32,
        has_rep_layout: bool,
        is_actor: bool,
        is_deleted: bool,
        is_stably_named: bool,
        delete_flags: u8,
        resolved_group_path: str,
        group_resolution_source: str,
        group_declared: bool,
        resolution_memo_hit: bool,
        function_count: u32,
        function_count_source: str,
        actor_archetype_path?: str,
        actor_archetype_outer_path?: str,
        actor_guid_path?: str,
        class_guid_path?: str,
        object_guid_path?: str,
        object_outer_path?: str,
        field_row_start: u64,
        field_row_count: u32,
    }
}

columns! {
    fn guid_entries_schema;
    #[cfg(feature = "checkpoint-context")]
    CheckpointGuidEntryRecord {
        ordinal: u32,
        net_guid: u32,
        outer_net_guid: u32,
        path_is_string: bool,
        literal_path?: str,
        name_index?: u32,
        flags: u8,
    }
}

columns! {
    fn export_groups_schema;
    #[cfg(feature = "checkpoint-context")]
    CheckpointExportGroupRecord {
        ordinal: u32,
        path_name_index: u32,
        group_path: str,
        declared_slots: u32,
    }
}

columns! {
    fn export_fields_schema;
    #[cfg(feature = "checkpoint-context")]
    CheckpointExportFieldRecord {
        group_ordinal: u32,
        path_name_index: u32,
        slot: u32,
        handle: u32,
        compatible_checksum: u32,
        rendered_name: str,
        exported_flag: u8,
        fname_kind: u8,
        fname_base?: str,
        fname_index?: u32,
        fname_number?: i32,
    }
}

columns! {
    /// Schema for the `fields` table (long format). An unresolved ClassNetCache
    /// block is one explicitly marked row, not split into fabricated fields.
    /// Address columns first, for predicate pushdown; the sparse value columns,
    /// at most one non-null per row, last. Nulls: [`crate::record::FieldRecord`].
    pub fn fields_schema;
    #[cfg(any(feature = "fields", feature = "checkpoint-context"))]
    FieldRecord {
        time_ms: u32,
        packet_id: u32,
        channel_index: u32,
        actor_net_guid: u32,
        object_net_guid?: u32,
        group_path: dict_runs(256, 20),
        handle: u32,
        field_name?: dict(256, 16),
        compatible_checksum?: u32,
        bit_count: u32,
        raw_bits?: bytes,
        value_i64?: i64,
        value_f64?: f64,
        value_bool?: bool,
        // A dictionary although the most varied string: enum strings and JSON repeat.
        value_str?: dict(2048, 32),
    }
}

columns! {
    /// Schema for the `movement` table: every row dense, no nulls.
    ///
    /// Unreal's left-handed Z-up coordinates: positions in cm, velocity in cm/s,
    /// yaw and pitch in degrees `[0, 360)` -- a `u16` scaled by `360/65536`, never
    /// negative, so a downward pitch reads near 360, not as a small negative.
    /// Consumers address these columns by position, so new ones are appended.
    pub fn movement_schema;
    #[cfg(feature = "movement")]
    MovementRecord {
        time_ms: u32,
        packet_id: u32,
        character_net_guid: u32,
        pos_x: f32,
        pos_y: f32,
        pos_z: f32,
        yaw: f32,
        pitch: f32,
        vel_x: f32,
        vel_y: f32,
        vel_z: f32,
        // The move header's server tick; `time_ms` is the replay-relative packet time.
        timestamp: u32,
        // One wire byte each, kept as u8 (`move_type` is effectively a bool).
        movement_state: u8,
        move_type: u8,
    }
}

columns! {
    /// Schema for the `actors` table (one row per channel open, close or
    /// dormancy). It shows actors that never replicate a field -- weapon and
    /// ability instances, DefuserItem, HeavyArmorItem -- which `fields.parquet`
    /// alone cannot resolve. Static actors and close rows have no spawn data.
    pub fn actors_schema;
    #[cfg(any(feature = "actors", feature = "checkpoint-context"))]
    ActorRecord {
        time_ms: u32,
        packet_id: u32,
        channel_index: u32,
        actor_net_guid: u32,
        // Plain Utf8 in Arrow; the Parquet column still gets a dictionary.
        event: str,
        class_path?: dict(128, 30),
        archetype_path?: dict(128, 30),
        spawn_x?: f32,
        spawn_y?: f32,
        spawn_z?: f32,
        spawn_pitch?: f32,
        spawn_yaw?: f32,
        spawn_roll?: f32,
        spawn_vx?: f32,
        spawn_vy?: f32,
        spawn_vz?: f32,
    }
}

columns! {
    /// Schema for the `net_guids` table (one row per registered NetGUID): the
    /// object path and containing object. `actors.parquet` covers only GUIDs
    /// that opened a channel, so a subobject such as a weapon's `FiringState`
    /// appears nowhere else. `outer_net_guid` is null, not 0, when no outer is
    /// declared: 0 is the engine's "invalid" sentinel, a different statement.
    pub fn net_guids_schema;
    #[cfg(any(feature = "net-guids", feature = "checkpoint-context"))]
    NetGuidRecord {
        net_guid: u32,
        path: dict(1024, 40),
        outer_net_guid?: u32,
    }
}

columns! {
    /// Schema for the `events` table (one row per Event chunk): the server's own
    /// labelled timeline. The header fields and `raw_payload`, every byte
    /// verbatim, are never null (an empty `metadata` or payload is empty).
    ///
    /// A payload's word count is not self-describing (`vrf_container::EventChunk`),
    /// so the structural overlay (tag, words, FString, trailing f32) is filled
    /// only for groups of established arity, and stays null rather than guessing
    /// for an unknown or changed one.
    pub fn events_schema;
    #[cfg(feature = "events")]
    EventRecord {
        id: str,
        group: dict(16, 24),
        metadata: str,
        time1: u32,
        time2: u32,
        // `raw_payload`'s length, readable from statistics.
        payload_size: i32,
        raw_payload: bytes,
        word0?: u32,
        word1?: u32,
        payload_tag?: u32,
        payload_name?: str,
        payload_seconds?: f32,
    }
}

columns! {
    pub fn partials_schema;
    #[cfg(feature = "partials")]
    PartialRecord {
        source: str,
        checkpoint_id?: str,
        payload_kind: str,
        reason: str,
        source_packet_id: i32,
        source_payload_bit_offset: i64,
        rejection_packet_id?: i32,
        channel_index: u32,
        channel_sequence: i32,
        open: bool,
        close: bool,
        dormant: bool,
        replication_paused: bool,
        reliable: bool,
        partial: bool,
        partial_initial: bool,
        partial_final: bool,
        has_package_map_exports: bool,
        has_must_be_mapped_guids: bool,
        close_reason: u8,
        source_payload_bit_count: i32,
        bit_count: u64,
        raw_bits: bytes,
    }
}

#[cfg(all(test, any(feature = "fields", feature = "checkpoint-context")))]
mod tests {
    use super::*;
    use arrow_array::Array;
    use arrow_array::builder::StringDictionaryBuilder;
    use arrow_array::types::Int32Type;

    #[test]
    fn dictionary_runs_keep_keys_values_and_first_occurrence_order() {
        let shared: Arc<str> = Arc::from("shared");
        let cases: Vec<Vec<Arc<str>>> = vec![
            vec![],
            vec![Arc::from("")],
            vec![shared.clone(); 16_401],
            vec![
                shared.clone(),
                shared,
                Arc::from("distinct allocation"),
                Arc::from("distinct allocation"),
                Arc::from(""),
                Arc::from(""),
                Arc::from("shared"),
                Arc::from("a\0b"),
                Arc::from("\u{03bb}"),
                Arc::from("\u{03bb}"),
                Arc::from("distinct allocation"),
            ],
            (0..513).map(|i| Arc::from(format!("unique-{i}"))).collect(),
        ];
        for values in cases {
            let actual = dictionary_runs(values.iter(), 256, 20);
            let mut elementwise = StringDictionaryBuilder::<Int32Type>::with_capacity(
                values.len(),
                256,
                values.len() * 20,
            );
            elementwise.extend(values.iter().map(|value| Some(value.as_ref())));
            let expected = elementwise.finish();
            assert_eq!(actual.keys(), expected.keys());
            assert_eq!(actual.values().to_data(), expected.values().to_data());
            assert_eq!(actual.nulls(), expected.nulls());
        }
    }
}
