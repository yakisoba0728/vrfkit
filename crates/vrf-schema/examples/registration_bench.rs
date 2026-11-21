//! Focused schema re-declaration workload; not a whole-replay benchmark.
//!
//! `cargo run --release -p vrf-schema --example registration_bench -- add`
//! measures adding preallocated empty groups. Use `declare` to measure the
//! wire readers' admission-before-allocation entry point instead.

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Instant;
use vrf_schema::{NetFieldExportGroup, NetGuidCache};

fn main() {
    const GROUPS: u32 = 512;
    const ROUNDS: u32 = 8;
    let mode = std::env::args().nth(1).unwrap_or_else(|| "add".into());
    assert!(
        matches!(mode.as_str(), "add" | "declare"),
        "use add or declare"
    );
    let declare = mode == "declare";
    let paths: Vec<String> = (0..GROUPS)
        .map(|index| format!("/Script/Benchmark.Group{index}"))
        .collect();
    let mut cache = NetGuidCache::new();
    for (index, path) in paths.iter().enumerate() {
        cache
            .add_export_group(NetFieldExportGroup::new(path.clone(), index as u32, 32))
            .expect("valid initial group");
    }
    let start = Instant::now();
    for _ in 0..ROUNDS {
        for (index, path) in paths.iter().enumerate() {
            let result = if declare {
                cache.declare_export_group(path.clone(), index as u32, 32)
            } else {
                cache.add_export_group(NetFieldExportGroup::new(path.clone(), index as u32, 32))
            };
            black_box(result.expect("valid identical declaration"));
        }
    }
    let elapsed = start.elapsed();
    for (index, path) in paths.iter().enumerate() {
        let group = cache
            .get_group_by_path(path)
            .expect("path remains registered");
        assert_eq!(group.path_name_index, index as u32);
        assert_eq!(group.fields.len(), 32);
    }
    assert_eq!(cache.field_slot_count(), GROUPS as usize * 32);
    assert_eq!(cache.schema_generation(), u64::from(GROUPS * (ROUNDS + 1)));
    println!(
        "{{\"mode\":\"{}\",\"groups\":{},\"redeclarations\":{},\"slots_per_group\":32,\"seconds\":{:.9}}}",
        mode,
        cache.group_count(),
        GROUPS * ROUNDS,
        elapsed.as_secs_f64()
    );
}
