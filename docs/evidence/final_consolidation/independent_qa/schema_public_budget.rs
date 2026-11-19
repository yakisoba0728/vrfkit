//! Independently exercise the public checkpoint path budget, without test hooks.
//! Copy to crates/vrf-schema/tests/qa_final_path_budget.rs for a recorded run.
//! Each test retains at most about 256 MiB of synthetic path payloads.

use std::convert::Infallible;

use vrf_schema::{
    CheckpointReadError, CheckpointTableSink, NetGuidCache, SchemaError,
    read_checkpoint_tables_with_sink,
};
use vrf_testkit::{add_fstring, add_int_packed, add_u32};

const PATH_BYTES: usize = 64 * 1024;
const LIMIT: usize = 256 * 1024 * 1024;
const ADMITTED: u32 = (LIMIT / PATH_BYTES) as u32;

struct Counter(u32);

impl CheckpointTableSink for Counter {
    type Error = Infallible;

    fn on_guid_entry(
        &mut self,
        ordinal: u32,
        _guid: u32,
        _outer: u32,
        path_is_string: bool,
        literal_path: Option<&str>,
        name_index: Option<u32>,
        _flags: u8,
    ) -> Result<(), Self::Error> {
        assert_eq!(ordinal, self.0);
        if ordinal == 0 {
            assert!(path_is_string);
            assert_eq!(literal_path.unwrap().len(), PATH_BYTES);
            assert_eq!(name_index, None);
        } else {
            assert!(!path_is_string);
            assert_eq!(literal_path, None);
            assert_eq!(name_index, Some(0));
        }
        self.0 += 1;
        Ok(())
    }
}

fn archive(entries: u32) -> Vec<u8> {
    let mut body = Vec::new();
    for index in 0..entries {
        add_int_packed(&mut body, index + 1);
        add_int_packed(&mut body, 0);
        body.push(u8::from(index == 0));
        if index == 0 {
            add_fstring(&mut body, &"X".repeat(PATH_BYTES));
        } else {
            add_int_packed(&mut body, 0);
        }
        body.push(3);
    }
    add_u32(&mut body, 0); // no export groups
    let mut result = Vec::new();
    add_u32(&mut result, (12 + body.len()) as u32);
    result.extend_from_slice(&[0; 12]);
    add_u32(&mut result, entries);
    result.extend(body);
    result
}

#[test]
fn qa_public_budget_accepts_exactly_256_mib_of_resolved_paths() {
    let data = archive(ADMITTED);
    assert!(data.len() < 128 * 1024);
    let mut cache = NetGuidCache::new();
    let mut observer = Counter(0);
    let tables = read_checkpoint_tables_with_sink(&data, &mut cache, &mut observer).unwrap();
    assert_eq!(tables.guid_count, ADMITTED);
    assert_eq!(tables.frame_offset, data.len());
    assert_eq!(observer.0, ADMITTED);
    assert_eq!(cache.net_guid_entries().len(), ADMITTED as usize);
    assert_eq!(cache.get_path_by_guid(ADMITTED).unwrap().len(), PATH_BYTES);
}

#[test]
fn qa_public_budget_rejects_one_extra_reference_before_callback_and_cache() {
    let data = archive(ADMITTED + 1);
    assert!(data.len() < 128 * 1024);
    let mut cache = NetGuidCache::new();
    let mut observer = Counter(0);
    let error = read_checkpoint_tables_with_sink(&data, &mut cache, &mut observer).unwrap_err();
    assert!(matches!(error, CheckpointReadError::Schema(SchemaError::ResourceLimit {
        resource: "checkpoint resolved GUID path bytes", requested, limit
    }) if requested == LIMIT + PATH_BYTES && limit == LIMIT));
    assert_eq!(observer.0, ADMITTED);
    assert_eq!(cache.net_guid_entries().len(), ADMITTED as usize);
    assert_eq!(cache.get_path_by_guid(ADMITTED).unwrap().len(), PATH_BYTES);
    assert!(cache.get_path_by_guid(ADMITTED + 1).is_none());
}
