#![forbid(unsafe_code)]

use vrf_bitio::BitReader;
use vrf_schema::{
    NetFieldExport, NetFieldExportGroup, NetGuidCache, SchemaError, SchemaLimits,
    read_net_field_exports,
};
use vrf_testkit::{BitWrite, BitWriter, pack};

#[test]
fn tiny_declarations_cannot_grow_default_cache_without_bound() {
    // Empty groups keep this regression's allocation small on the old parser.
    let mut wire = BitWriter::new();
    wire.int_packed(4_097);
    for index in 0..4_097 {
        wire.int_packed(index)
            .int_packed(1)
            .fstring(&format!("G{index}"))
            .int_packed(0)
            .u8(0);
    }
    let bytes = pack(&wire);
    let mut cache = NetGuidCache::new();
    assert_eq!(
        read_net_field_exports(&mut BitReader::new(&bytes), &mut cache),
        Err(SchemaError::ResourceLimit {
            resource: "export groups",
            requested: 4_097,
            limit: 4_096,
        })
    );
    assert_eq!(cache.group_count(), 4_096);
    assert_eq!(cache.field_slot_count(), 0);
}

fn cache(groups: usize, slots: usize) -> NetGuidCache {
    NetGuidCache::with_limits(
        SchemaLimits::default()
            .with_max_groups(groups)
            .with_max_field_slots(slots),
    )
}

fn slot_limit(requested: usize, limit: usize) -> SchemaError {
    SchemaError::ResourceLimit {
        resource: "field slots",
        requested,
        limit,
    }
}

fn live_declaration(wire: &mut BitWriter, index: u32, path: &str, slots: u32) {
    wire.int_packed(index)
        .int_packed(1)
        .fstring(path)
        .int_packed(slots);
}

#[test]
fn live_slot_budget_persists_across_frames_and_rejects_before_field_data() {
    let mut cache = cache(4, 7);
    let mut first = BitWriter::new();
    live_declaration(first.int_packed(1), 1, "First", 4);
    first.u8(0);
    assert_eq!(
        read_net_field_exports(&mut BitReader::new(&pack(&first)), &mut cache),
        Ok(1)
    );

    let generation = cache.schema_generation();
    let mut next = BitWriter::new();
    live_declaration(next.int_packed(1), 2, "Second", 4);
    // No field flag: the resource error must precede a possible EOF.
    let bytes = pack(&next);
    let mut reader = BitReader::new(&bytes);
    assert_eq!(
        read_net_field_exports(&mut reader, &mut cache),
        Err(slot_limit(8, 7))
    );
    assert!(reader.at_end());
    assert_eq!(cache.group_count(), 1);
    assert_eq!(cache.field_slot_count(), 4);
    assert_eq!(cache.schema_generation(), generation);
    assert!(cache.get_group_by_index(2).is_none());
}

#[test]
fn live_oversized_group_keeps_its_existing_wire_error() {
    let mut wire = BitWriter::new();
    live_declaration(wire.int_packed(1), 1, "Huge", u32::MAX);
    assert_eq!(
        read_net_field_exports(&mut BitReader::new(&pack(&wire)), &mut cache(4, 7)),
        Err(SchemaError::FieldCountOverflow {
            count: u32::MAX,
            max: 65_536,
        })
    );
}

#[test]
fn a_single_declaration_is_bounded_before_any_slot_is_stored() {
    let mut cache = cache(4, 7);
    assert_eq!(
        cache.declare_export_group("TooLarge".into(), 1, 8),
        Err(slot_limit(8, 7))
    );
    assert_eq!(cache.group_count(), 0);
    assert_eq!(cache.field_slot_count(), 0);
    assert_eq!(cache.schema_generation(), 0);
}

#[test]
fn redeclarations_charge_only_growth_and_cannot_shrink_existing_slots() {
    let mut cache = cache(1, 7);
    cache.declare_export_group("Same".into(), 1, 4).unwrap();
    let descriptor = NetFieldExport {
        handle: 3,
        compatible_checksum: 42,
        name: "Preserved".into(),
    };
    assert!(cache.set_field_on_group(1, descriptor.clone()));
    cache.declare_export_group("Same".into(), 1, 4).unwrap();
    cache.declare_export_group("Same".into(), 1, 2).unwrap();
    assert_eq!(cache.field_slot_count(), 4);
    cache.declare_export_group("Same".into(), 1, 7).unwrap();
    let generation = cache.schema_generation();
    assert_eq!(
        cache.declare_export_group("Same".into(), 1, 8),
        Err(slot_limit(8, 7))
    );
    assert_eq!(cache.field_slot_count(), 7);
    assert_eq!(cache.get_group_by_index(1).unwrap().len(), 7);
    assert_eq!(
        cache.get_group_by_index(1).unwrap().get_field(3),
        Some(&descriptor)
    );
    assert_eq!(cache.schema_generation(), generation);
}

#[test]
fn replacement_releases_slots_and_failed_growth_preserves_the_old_group() {
    let mut cache = cache(2, 7);
    cache.declare_export_group("Old".into(), 1, 6).unwrap();
    cache.declare_export_group("Other".into(), 2, 1).unwrap();
    assert_eq!(
        cache.declare_export_group("TooLarge".into(), 1, 7),
        Err(slot_limit(8, 7))
    );
    assert_eq!(cache.get_group_by_index(1).unwrap().path, "Old");
    assert_eq!(cache.replaced_export_groups(), 0);

    cache.declare_export_group("New".into(), 1, 2).unwrap();
    assert_eq!(cache.field_slot_count(), 3);
    assert_eq!(cache.replaced_export_groups(), 1);
    cache.declare_export_group("Other".into(), 2, 5).unwrap();
    assert_eq!(cache.field_slot_count(), 7);
}

#[test]
fn alias_and_index_changes_merge_without_double_charging_slots() {
    let mut cache = cache(1, 4);
    cache.declare_export_group("Thing".into(), 1, 3).unwrap();
    cache
        .declare_export_group("Default__Thing".into(), 2, 4)
        .unwrap();
    assert_eq!(cache.group_count(), 1);
    assert_eq!(cache.field_slot_count(), 4);
    assert!(cache.get_group_by_index(1).is_none());
    assert_eq!(cache.get_group_by_index(2).unwrap().len(), 4);
}

#[test]
fn crossed_identity_errors_still_precede_resource_refusals() {
    let mut cache = cache(2, 2);
    cache.declare_export_group("A".into(), 1, 1).unwrap();
    cache.declare_export_group("B".into(), 2, 1).unwrap();
    let generation = cache.schema_generation();
    assert!(matches!(
        cache.declare_export_group("A".into(), 2, 3),
        Err(SchemaError::CrossedExportGroupIdentity { .. })
    ));
    assert_eq!(cache.field_slot_count(), 2);
    assert_eq!(cache.schema_generation(), generation);
}

#[test]
fn preallocated_groups_use_the_same_budget() {
    let mut cache = cache(2, 7);
    cache
        .add_export_group(NetFieldExportGroup::new("A".into(), 1, 4))
        .unwrap();
    assert_eq!(
        cache.add_export_group(NetFieldExportGroup::new("B".into(), 2, 4)),
        Err(slot_limit(8, 7))
    );
    assert_eq!(cache.group_count(), 1);
    assert_eq!(cache.field_slot_count(), 4);
}

#[test]
fn public_mutable_fields_do_not_leave_stale_budget_accounting() {
    let mut cache = cache(2, 7);
    cache.declare_export_group("A".into(), 1, 2).unwrap();
    cache
        .get_group_by_index_mut(1)
        .unwrap()
        .fields
        .resize(5, None);
    assert_eq!(cache.field_slot_count(), 5);
    assert_eq!(
        cache.declare_export_group("B".into(), 2, 3),
        Err(slot_limit(8, 7))
    );
    cache.get_group_by_index_mut(1).unwrap().fields.truncate(2);
    assert_eq!(cache.field_slot_count(), 2);
    cache.declare_export_group("B".into(), 2, 3).unwrap();
    assert_eq!(cache.field_slot_count(), 5);
}

#[test]
fn clear_resets_usage_and_preserves_configured_limits() {
    let mut cache = cache(1, 4);
    let limits = cache.limits();
    cache.declare_export_group("A".into(), 1, 4).unwrap();
    cache.get_group_by_index_mut(1).unwrap().fields.truncate(1);
    cache.clear();
    assert_eq!(cache.group_count(), 0);
    assert_eq!(cache.field_slot_count(), 0);
    assert_eq!(cache.limits(), limits);
    cache.declare_export_group("B".into(), 2, 4).unwrap();
    assert_eq!(cache.field_slot_count(), 4);
}

#[test]
fn zero_budgets_have_explicit_boundary_behavior() {
    let mut no_groups = cache(0, 0);
    assert_eq!(
        no_groups.declare_export_group("A".into(), 1, 0),
        Err(SchemaError::ResourceLimit {
            resource: "export groups",
            requested: 1,
            limit: 0,
        })
    );
    let mut no_slots = cache(1, 0);
    no_slots.declare_export_group("A".into(), 1, 0).unwrap();
    assert_eq!(no_slots.field_slot_count(), 0);
    assert_eq!(
        no_slots.declare_export_group("A".into(), 1, 1),
        Err(slot_limit(1, 0))
    );
}

#[cfg(feature = "checkpoint")]
mod checkpoints {
    use super::*;
    use vrf_schema::{
        CheckpointReadError, CheckpointTableSink, read_checkpoint_tables,
        read_checkpoint_tables_with_sink,
    };

    fn archive(groups: &[(&str, u32, u32)], write_slots: bool) -> Vec<u8> {
        let mut wire = BitWriter::new();
        wire.u32(0).u32(0).u32(0).u32(0).u32(0);
        wire.u32(groups.len() as u32);
        for &(path, index, slots) in groups {
            wire.fstring(path).int_packed(index).int_packed(slots);
            if write_slots {
                for _ in 0..slots {
                    wire.u8(0);
                }
            }
        }
        let mut bytes = pack(&wire);
        let frame_offset = (bytes.len() - 8) as u32;
        bytes[..4].copy_from_slice(&frame_offset.to_le_bytes());
        bytes
    }

    #[derive(Default)]
    struct Observer(Vec<(String, u32)>);

    impl CheckpointTableSink for Observer {
        type Error = core::convert::Infallible;

        fn on_export_group(
            &mut self,
            _: u32,
            _: u32,
            path: &str,
            slots: u32,
        ) -> Result<(), Self::Error> {
            self.0.push((path.into(), slots));
            Ok(())
        }
    }

    #[test]
    fn checkpoint_slot_limit_preserves_observed_declarations_and_returns_error() {
        let bytes = archive(&[("A", 1, 4), ("B", 2, 4)], true);
        let mut cache = cache(2, 7);
        let mut sink = Observer::default();
        let result = read_checkpoint_tables_with_sink(&bytes, &mut cache, &mut sink);
        assert!(
            matches!(result, Err(CheckpointReadError::Schema(error)) if error == slot_limit(8, 7))
        );
        assert_eq!(sink.0, [("A".into(), 4), ("B".into(), 4)]);
        assert_eq!(cache.group_count(), 1);
        assert_eq!(cache.field_slot_count(), 4);
        assert!(cache.get_group_by_index(2).is_none());
    }

    #[test]
    fn checkpoint_group_limit_includes_zero_slot_groups() {
        let bytes = archive(&[("A", 1, 0), ("B", 2, 0)], true);
        let mut cache = cache(1, 7);
        assert_eq!(
            read_checkpoint_tables(&bytes, &mut cache),
            Err(SchemaError::ResourceLimit {
                resource: "export groups",
                requested: 2,
                limit: 1
            })
        );
        assert_eq!(cache.group_count(), 1);
    }

    #[test]
    fn checkpoint_refusal_precedes_missing_slots_and_retains_large_count_errors() {
        let bytes = archive(&[("A", 1, 8)], false);
        assert_eq!(
            read_checkpoint_tables(&bytes, &mut cache(2, 7)),
            Err(slot_limit(8, 7))
        );

        let bytes = archive(&[("A", 1, u32::MAX)], false);
        assert_eq!(
            read_checkpoint_tables(&bytes, &mut cache(2, 7)),
            Err(SchemaError::CheckpointCountOverflow {
                field: "fields in a group",
                count: u32::MAX,
                max: 65_536,
            })
        );
    }
}
