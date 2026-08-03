//! `export` subcommand driver -- full pipeline from .vrf to Parquet.
//!
//! # Architecture
//!
//! The borrow-checker constraint: `iter_demo_frames` mutably borrows the
//! `NetGuidCache` (to receive schema updates), while the `ExportSink` needs a
//! shared reference to that same cache (to resolve paths and field names).
//!
//! Solution: collect packets from one DemoFrame pass (cheap -- just byte offsets
//! into the decompressed chunk), then process them with the *updated* cache.
//! This two-phase design means path resolution always sees the latest schema.
//!
//! # Layout
//!
//! - [`writers`] -- the two large tables' writers, running off the packet loop.
//! - [`checkpoints`] -- the optional full-state snapshot pass.
//! - [`summary`] -- the stderr report, whose every line a Python harness pins.

mod checkpoints;
mod summary;
mod writers;

use std::fs;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

use vrf_container::{
    ChunkIterator, ChunkType, decompress_replay_data, parse_event_chunk, parse_preamble,
};
use vrf_decode::{OverlayErrorReport, OverlayStats};
use vrf_export::{
    ActorWriter, EventRecord, EventWriter, FieldRecord, FieldWriter, MovementRecord,
    MovementWriter, NetGuidRecord, NetGuidWriter,
};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_schema::NetGuidCache;

use crate::error::CliError;
use crate::manifest;
use crate::sink::{ChannelState, ExportSink, RecordBuffers};
use checkpoints::{CheckpointStats, ReplayContext};
use summary::RunTotals;
use writers::WriterThread;

/// A packet descriptor collected from DemoFrame iteration.
/// Stores byte offset + length into the decompressed chunk buffer.
struct PacketDesc {
    time_ms: u32,
    offset: usize,
    len: usize,
}

pub fn run(vrf_path: &str, out_dir: &str, with_checkpoints: bool) -> Result<(), CliError> {
    let start = Instant::now();

    // -- Read file ---------------------------------------------------------
    eprintln!("reading {vrf_path}...");
    let data = fs::read(vrf_path)?;
    let file_size = data.len();

    // -- Parse preamble ----------------------------------------------------
    let preamble = parse_preamble(&data)?;
    let ctx = ReplayContext {
        branch: &preamble.header.replay_version.branch,
        flags: preamble.header.flags,
        compressed: preamble.info.compressed,
        encrypted: preamble.info.encrypted,
    };

    eprintln!(
        "branch: {}, flags: 0x{:04X}, compressed: {}, duration: {} ms",
        ctx.branch, ctx.flags, ctx.compressed, preamble.info.length_in_ms
    );

    // -- Setup output ------------------------------------------------------
    let out_path = Path::new(out_dir);
    fs::create_dir_all(out_path)?;

    let create = |name: &str| -> Result<BufWriter<fs::File>, CliError> {
        Ok(BufWriter::new(fs::File::create(out_path.join(name))?))
    };

    let mut field_writer = FieldWriter::new(create("fields.parquet")?)?;
    let mut movement_writer = MovementWriter::new(create("movement.parquet")?)?;
    let mut actor_writer = ActorWriter::new(create("actors.parquet")?)?;
    // Event chunks are a couple of hundred rows and are written inline for the
    // same reason `actors` is: the encoding cost is far below a thread's worth.
    let mut event_writer = EventWriter::new(create("events.parquet")?)?;
    let mut checkpoint_writer = if with_checkpoints {
        Some(FieldWriter::new(create("checkpoint_fields.parquet")?)?)
    } else {
        None
    };

    let mut fields = WriterThread::<FieldRecord>::spawn("fields", move |rx| {
        for batch in rx {
            field_writer.push_batch(batch)?;
        }
        field_writer.finish()
    });
    let mut movement = WriterThread::<MovementRecord>::spawn("movement", move |rx| {
        for batch in rx {
            movement_writer.push_batch(batch)?;
        }
        movement_writer.finish()
    });

    // -- Setup replication reader and schema cache --------------------------
    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(ctx.branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    // -- Iterate chunks ----------------------------------------------------
    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut chunks_processed = 0u32;
    let mut total_packets: u32 = 0;
    let mut channel_state = ChannelState::new();

    // Reusable packet descriptor buffer (avoids per-chunk allocation).
    let mut packet_descs: Vec<PacketDesc> = Vec::with_capacity(4096);
    // Reusable per-packet record buffers; see `RecordBuffers`.
    let mut buffers = RecordBuffers::default();
    let mut movement_rows: u64 = 0;
    let mut event_rows: u64 = 0;
    let mut event_trailing_bytes: u64 = 0;
    let mut overlay_stats = OverlayStats::default();
    let mut error_report = OverlayErrorReport::default();
    let mut effect_blobs_decoded: u64 = 0;
    let mut cp_stats = CheckpointStats::default();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        // Sliced once for all three chunk kinds. Safe for the kinds this loop
        // ignores too: `next_chunk` refuses a chunk whose declared size runs
        // past the file, so the range is in bounds before it is returned.
        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];

        // Event chunks carry the server's own labelled timeline. They are
        // uncompressed and independent of the replication pass, so they are
        // read here and written straight out.
        if chunk.chunk_type == ChunkType::Event {
            let event = parse_event_chunk(payload)?;
            event_trailing_bytes += event.trailing_bytes as u64;
            event_writer.push(EventRecord {
                id: event.id,
                group: event.group,
                metadata: event.metadata,
                time1: event.time1,
                time2: event.time2,
                payload_size: event.size_in_bytes,
                raw_payload: event.payload.to_vec(),
            })?;
            event_rows += 1;
            continue;
        }
        if chunk.chunk_type == ChunkType::Checkpoint {
            if let Some(writer) = checkpoint_writer.as_mut() {
                checkpoints::process_chunk(
                    payload,
                    &ctx,
                    writer,
                    &mut cp_stats,
                    &mut error_report,
                )?;
            }
            continue;
        }
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        let decompressed = decompress_replay_data(payload, ctx.compressed, ctx.encrypted)?;

        // Phase 1: iterate DemoFrames -- populates cache, collects packet locations.
        packet_descs.clear();
        iter_demo_frames(&decompressed, ctx.flags, &mut cache, |pkt| {
            // pkt.data is a slice into `decompressed`. Compute its offset.
            let offset = pkt.data.as_ptr() as usize - decompressed.as_ptr() as usize;
            packet_descs.push(PacketDesc {
                time_ms: pkt.time_ms,
                offset,
                len: pkt.data.len(),
            });
        })?;

        // Phase 2: process packets through replication using the now-complete cache.
        for desc in &packet_descs {
            let pkt_data = &decompressed[desc.offset..desc.offset + desc.len];
            let pkt_id = total_packets;
            total_packets += 1;

            // Scoped so the sink's borrow of `buffers` ends before they are
            // drained. The buffers outlive the sink; that is the point.
            {
                let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut buffers);
                sink.time_ms = desc.time_ms;
                sink.packet_id = pkt_id;

                repl_reader.process_packet(pkt_data, pkt_id as i32, &mut sink);

                // Accumulate overlay stats.
                overlay_stats.decoded_ok += sink.stats.overlay.decoded_ok;
                overlay_stats.decoded_err += sink.stats.overlay.decoded_err;
                overlay_stats.raw_or_skip += sink.stats.overlay.raw_or_skip;
                overlay_stats.not_in_table += sink.stats.overlay.not_in_table;
                overlay_stats.no_field_name += sink.stats.overlay.no_field_name;
                effect_blobs_decoded += sink.stats.effect_blobs_decoded;
                error_report.merge_from(&sink.stats.overlay.error_report);
            }

            // Hand field and movement records to their writer threads.
            fields.append(&mut buffers.fields)?;
            movement_rows += buffers.movement.len() as u64;
            movement.append(&mut buffers.movement)?;
            // Drain actor lifecycle records to the inline writer.
            for record in buffers.actors.drain(..) {
                actor_writer.push(record)?;
            }
        }

        chunks_processed += 1;

        if chunks_processed % 100 == 0 {
            eprintln!(
                "  chunk {chunks_processed}: {total_packets} packets, {} groups",
                cache.group_count()
            );
        }
    }

    // -- Finish writers ----------------------------------------------------
    //
    // The two offloaded writers are joined here, before the elapsed time is
    // taken and before any file size is read, so both files are complete and
    // both results are checked.
    fields.finish()?;
    movement.finish()?;
    actor_writer.finish()?;
    event_writer.finish()?;
    if let Some(w) = checkpoint_writer.take() {
        w.finish()?;
    }

    // -- Write the NetGUID registry ----------------------------------------
    //
    // Written after the replication pass because the cache accumulates over the
    // whole replay: a GUID's outer may be declared in a later chunk than the
    // one that first referenced it. Sorted so the file is byte-reproducible
    // across runs (the cache is HashMap-backed and iterates in arbitrary order).
    let mut net_guid_writer = NetGuidWriter::new(create("net_guids.parquet")?)?;
    let mut guid_entries = cache.net_guid_entries();
    guid_entries.sort_unstable_by_key(|e| e.net_guid);
    let net_guid_rows = guid_entries.len();
    for entry in guid_entries {
        net_guid_writer.push(NetGuidRecord {
            net_guid: entry.net_guid,
            path: entry.path.to_owned(),
            outer_net_guid: entry.outer_net_guid,
        })?;
    }
    net_guid_writer.finish()?;

    let net_stats = repl_reader.stats();
    let elapsed = start.elapsed();

    // -- Write manifest ----------------------------------------------------
    //
    // Before the summary so the path the summary prints names a file that
    // exists by the time it is read.
    let manifest_path = out_path.join("manifest.json");
    manifest::write_manifest(
        &manifest_path,
        vrf_path,
        file_size,
        &preamble,
        &cache,
        net_stats,
        total_packets,
        elapsed,
    )?;

    summary::print(
        out_path,
        net_stats,
        &RunTotals {
            chunks_processed,
            total_packets,
            export_groups: cache.group_count(),
            movement_rows,
            net_guid_rows,
            event_rows,
            event_trailing_bytes,
            elapsed,
            effect_blobs_decoded,
        },
        &overlay_stats,
        &error_report,
        with_checkpoints.then_some(&cp_stats),
        &manifest_path,
    );

    Ok(())
}
