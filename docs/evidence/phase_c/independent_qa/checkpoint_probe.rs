//! Independent public-API adapter. Run only on reviewed public fixtures.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::path::PathBuf;

use vrf_export::{
    CheckpointActorRecord, CheckpointActorWriter, CheckpointBlockWriter,
    CheckpointExportFieldWriter, CheckpointExportGroupWriter, CheckpointFieldRecord,
    CheckpointFieldWriter, CheckpointGuidEntryWriter, CheckpointIdentity, CheckpointNetGuidRecord,
    CheckpointNetGuidWriter,
};
use vrfkit::{CheckpointEvent, ReadOptions, ReplayEvent, ReplayReader};

fn add(counters: &mut BTreeMap<String, u64>, category: &str, name: &str, value: u64) {
    let name = if category == "sink" {
        sink_name(name)
    } else {
        name
    };
    *counters.entry(format!("{category}\t{name}")).or_default() += value;
}

fn sink_name(name: &str) -> &str {
    match name {
        "rpcs_emitted" => "sink_rpcs_emitted",
        "actor_opens" => "sink_actor_opens",
        "actor_closes" => "sink_actor_closes",
        "content_blocks" => "sink_content_blocks",
        other => other,
    }
}

fn same(left: &CheckpointIdentity, right: &CheckpointIdentity) {
    assert_eq!(left.checkpoint_index, right.checkpoint_index);
    assert_eq!(left.checkpoint_id, right.checkpoint_id);
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let replay = args.next().ok_or("missing replay")?;
    let output = PathBuf::from(args.next().ok_or("missing new output directory")?);
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    fs::create_dir(&output)?;
    let mut fields =
        CheckpointFieldWriter::new(File::create(output.join("checkpoint_fields.parquet"))?)?;
    let mut actors =
        CheckpointActorWriter::new(File::create(output.join("checkpoint_actors.parquet"))?)?;
    let mut guids =
        CheckpointNetGuidWriter::new(File::create(output.join("checkpoint_net_guids.parquet"))?)?;
    let mut blocks =
        CheckpointBlockWriter::new(File::create(output.join("checkpoint_blocks.parquet"))?)?;
    let mut declarations = CheckpointGuidEntryWriter::new(File::create(
        output.join("checkpoint_guid_entries.parquet"),
    )?)?;
    let mut groups = CheckpointExportGroupWriter::new(File::create(
        output.join("checkpoint_export_groups.parquet"),
    )?)?;
    let mut exports = CheckpointExportFieldWriter::new(File::create(
        output.join("checkpoint_export_fields.parquet"),
    )?)?;
    let mut counters = BTreeMap::new();
    let mut active: Option<CheckpointIdentity> = None;
    let mut completed = 0_u32;
    let mut seen = [0_u64; 7];
    let mut current_rows = [0_u64; 5];
    let mut partial_bits = 0_u64;
    let mut checkpoint_diagnostics = 0_usize;
    let mut checkpoint_network = vrf_net::NetStats::default();
    let summary = ReplayReader.read_path_with_options(
        replay,
        ReadOptions {
            checkpoints: true,
            ..ReadOptions::default()
        },
        |event| {
            match event {
                ReplayEvent::Checkpoint(event) => match event {
                    CheckpointEvent::Metadata(metadata) => {
                        assert!(active.is_none(), "overlapping snapshots");
                        assert_eq!(metadata.identity.checkpoint_index, completed);
                        active = Some(metadata.identity.clone());
                        current_rows = [0; 5];
                    }
                    CheckpointEvent::GuidEntry(row) => {
                        same(active.as_ref().expect("missing metadata"), &row.checkpoint);
                        declarations.push(row.clone())?;
                        seen[0] += 1;
                    }
                    CheckpointEvent::ExportGroup(row) => {
                        same(active.as_ref().expect("missing metadata"), &row.checkpoint);
                        groups.push(row.clone())?;
                        seen[1] += 1;
                    }
                    CheckpointEvent::ExportField(row) => {
                        same(active.as_ref().expect("missing metadata"), &row.checkpoint);
                        exports.push(row.clone())?;
                        seen[2] += 1;
                    }
                    CheckpointEvent::Records {
                        checkpoint,
                        records,
                        blocks: batch_blocks,
                    } => {
                        same(
                            active.as_ref().expect("missing metadata"),
                            &checkpoint.identity,
                        );
                        assert_eq!(records.movement.len(), records.movement_provenance.len());
                        for row in records.fields {
                            fields.push(CheckpointFieldRecord {
                                checkpoint: checkpoint.identity.clone(),
                                field: row.clone(),
                            })?;
                        }
                        for row in records.actors {
                            actors.push(CheckpointActorRecord {
                                checkpoint: checkpoint.identity.clone(),
                                actor: row.clone(),
                            })?;
                        }
                        for block in batch_blocks {
                            same(&checkpoint.identity, &block.checkpoint);
                            blocks.push(block.clone())?;
                        }
                        for partial in records.partials {
                            assert_eq!(partial.source, "checkpoint");
                            assert_eq!(
                                partial.checkpoint_id.as_deref(),
                                Some(checkpoint.identity.checkpoint_id.as_ref())
                            );
                            partial_bits += partial.bit_count;
                        }
                        for (slot, count) in current_rows.iter_mut().zip([
                            records.fields.len(),
                            records.actors.len(),
                            records.movement.len(),
                            records.partials.len(),
                            batch_blocks.len(),
                        ]) {
                            *slot += count as u64;
                        }
                        seen[3] += records.fields.len() as u64;
                        seen[4] += records.actors.len() as u64;
                        seen[5] += batch_blocks.len() as u64;
                    }
                    CheckpointEvent::Summary(snapshot) => {
                        same(
                            active.as_ref().expect("missing metadata"),
                            &snapshot.metadata.identity,
                        );
                        assert_eq!(
                            current_rows,
                            [
                                snapshot.field_rows,
                                snapshot.actor_rows,
                                snapshot.movement_rows,
                                snapshot.partial_rows,
                                snapshot.block_rows
                            ]
                        );
                        for guid in &snapshot.net_guids {
                            guids.push(CheckpointNetGuidRecord {
                                checkpoint: snapshot.metadata.identity.clone(),
                                net_guid: guid.clone(),
                            })?;
                        }
                        seen[6] += snapshot.net_guids.len() as u64;
                        checkpoint_network.absorb(&mut snapshot.network.clone());
                        for (name, value) in &snapshot.decode_counters {
                            add(&mut counters, "sink", name, *value);
                        }
                        for (name, value) in [
                            ("checkpoint_chunks", 1),
                            (
                                "checkpoint_trailing_bytes",
                                snapshot.metadata.trailing_bytes,
                            ),
                            (
                                "checkpoint_guid_entries",
                                u64::from(snapshot.tables.guid_count),
                            ),
                            (
                                "checkpoint_literal_paths",
                                u64::from(snapshot.tables.literal_paths),
                            ),
                            (
                                "checkpoint_indexed_paths",
                                u64::from(snapshot.tables.indexed_paths),
                            ),
                            (
                                "checkpoint_resolved_path_indices",
                                u64::from(snapshot.tables.resolved_path_indices),
                            ),
                            (
                                "checkpoint_group_records",
                                u64::from(snapshot.tables.group_count),
                            ),
                            (
                                "checkpoint_exported_fields",
                                u64::from(snapshot.tables.exported_fields),
                            ),
                            ("checkpoint_frames", u64::from(snapshot.frames)),
                            ("checkpoint_packets", u64::from(snapshot.packets)),
                            ("checkpoint_field_rows", snapshot.field_rows),
                            ("checkpoint_actor_rows_written", snapshot.actor_rows),
                            ("checkpoint_block_rows_written", snapshot.block_rows),
                            (
                                "checkpoint_net_guid_rows_written",
                                snapshot.net_guids.len() as u64,
                            ),
                            (
                                "checkpoint_guid_entry_rows_written",
                                u64::from(snapshot.tables.guid_count),
                            ),
                            (
                                "checkpoint_export_group_rows_written",
                                u64::from(snapshot.tables.group_count),
                            ),
                            (
                                "checkpoint_export_field_rows_written",
                                u64::from(snapshot.tables.exported_fields),
                            ),
                            (
                                "checkpoint_actor_rows_dropped",
                                (snapshot.network.actor_opens + snapshot.network.actor_closes)
                                    .saturating_sub(snapshot.actor_rows),
                            ),
                            ("checkpoint_movement_rows_dropped", snapshot.movement_rows),
                            ("checkpoint_partial_rows", snapshot.partial_rows),
                            (
                                "checkpoint_frame_external_data_blobs",
                                snapshot.frame_skips.external_data_blobs,
                            ),
                            (
                                "checkpoint_frame_external_data_bytes",
                                snapshot.frame_skips.external_data_bytes,
                            ),
                            (
                                "checkpoint_frame_game_specific_bytes",
                                snapshot.frame_skips.game_specific_bytes,
                            ),
                            (
                                "checkpoint_frame_non_finite_times",
                                snapshot.non_finite_frame_times,
                            ),
                        ] {
                            add(&mut counters, "summary", name, value);
                        }
                        assert_eq!(snapshot.decode_error_count, 0);
                        assert!(snapshot.decode_errors.is_empty());
                        checkpoint_diagnostics += snapshot.stream_failures.len();
                        assert!(snapshot.first_movement_error.is_none());
                        assert!(snapshot.first_struct_error.is_none());
                        completed += 1;
                        active = None;
                    }
                    _ => return Err("unrecognized checkpoint callback".into()),
                },
                _ => assert!(active.is_none(), "main callback inside snapshot"),
            }
            Ok::<(), Box<dyn Error>>(())
        },
    )?;
    assert!(active.is_none());
    assert_eq!(summary.decode_error_count, 0);
    assert!(summary.decode_errors.is_empty());
    assert!(summary.first_movement_error.is_none());
    assert!(summary.first_struct_error.is_none());
    assert_eq!(summary.checkpoints.chunks, u64::from(completed));
    assert_eq!(summary.skipped_checkpoint_chunks, 0);
    assert_eq!(summary.checkpoints.field_rows, seen[3]);
    assert_eq!(summary.checkpoints.actor_rows, seen[4]);
    assert_eq!(summary.checkpoints.block_rows, seen[5]);
    fields.finish()?;
    actors.finish()?;
    guids.finish()?;
    blocks.finish()?;
    declarations.finish()?;
    groups.finish()?;
    exports.finish()?;
    add(
        &mut counters,
        "summary",
        "checkpoint_partial_bits",
        partial_bits,
    );
    println!(
        "D\tmain_stream_diagnostics\t{}",
        summary.stream_failures.len()
    );
    println!("D\tcheckpoint_stream_diagnostics\t{checkpoint_diagnostics}");
    for (name, value) in checkpoint_network.counters() {
        add(&mut counters, "net", name, value);
    }
    add(
        &mut counters,
        "net",
        "diagnostics_retained",
        checkpoint_network.diagnostics.len() as u64,
    );
    for (category, count) in counters {
        println!("C\t{category}\t{count}");
    }
    for (name, count) in summary.network.counters() {
        println!("M\tnet\t{name}\t{count}");
    }
    println!(
        "M\tnet\tdiagnostics_retained\t{}",
        summary.network.diagnostics.len()
    );
    for (name, count) in summary.decode_counters {
        println!("M\tsink\t{}\t{count}", sink_name(name));
    }
    for (name, count) in [
        ("guid_entries", seen[0]),
        ("groups", seen[1]),
        ("exports", seen[2]),
        ("fields", seen[3]),
        ("actors", seen[4]),
        ("blocks", seen[5]),
        ("guids", seen[6]),
    ] {
        println!("R\t{name}\t{count}");
    }
    Ok(())
}
