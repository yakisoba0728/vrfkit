//! `export` subcommand driver — full pipeline from .vrf to Parquet.
//!
//! # Architecture
//!
//! The borrow-checker constraint: `iter_demo_frames` mutably borrows the
//! `NetGuidCache` (to receive schema updates), while the `ExportSink` needs a
//! shared reference to that same cache (to resolve paths and field names).
//!
//! Solution: collect packets from one DemoFrame pass (cheap — just byte offsets
//! into the decompressed chunk), then process them with the *updated* cache.
//! This two-phase design means path resolution always sees the latest schema.

use std::fs;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

use vrf_container::{ChunkIterator, ChunkType, decompress_replay_data, parse_preamble};
use vrf_decode::{OverlayErrorReport, OverlayStats};
use vrf_export::{ActorWriter, FieldWriter, MovementWriter, NetGuidRecord, NetGuidWriter};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_schema::NetGuidCache;

use crate::error::CliError;
use crate::manifest;
use crate::sink::ChannelState;
use crate::sink::ExportSink;

/// A packet descriptor collected from DemoFrame iteration.
/// Stores byte offset + length into the decompressed chunk buffer.
struct PacketDesc {
    time_ms: u32,
    offset: usize,
    len: usize,
}

pub fn run(vrf_path: &str, out_dir: &str) -> Result<(), CliError> {
    let start = Instant::now();

    // ── Read file ─────────────────────────────────────────────────────────
    eprintln!("reading {vrf_path}...");
    let data = fs::read(vrf_path)?;
    let file_size = data.len();

    // ── Parse preamble ────────────────────────────────────────────────────
    let preamble = parse_preamble(&data)?;
    let branch = &preamble.header.replay_version.branch;
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;

    eprintln!(
        "branch: {branch}, flags: 0x{flags:04X}, compressed: {compressed}, duration: {} ms",
        preamble.info.length_in_ms
    );

    // ── Setup output ──────────────────────────────────────────────────────
    let out_path = Path::new(out_dir);
    fs::create_dir_all(out_path)?;

    let fields_file = BufWriter::new(fs::File::create(out_path.join("fields.parquet"))?);
    let movement_file = BufWriter::new(fs::File::create(out_path.join("movement.parquet"))?);
    let actors_file = BufWriter::new(fs::File::create(out_path.join("actors.parquet"))?);

    let mut field_writer = FieldWriter::new(fields_file)?;
    let mut movement_writer = MovementWriter::new(movement_file)?;
    let mut actor_writer = ActorWriter::new(actors_file)?;

    // ── Setup replication reader and schema cache ──────────────────────────
    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    // ── Iterate chunks ────────────────────────────────────────────────────
    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut chunks_processed = 0u32;
    let mut total_packets: u32 = 0;
    let mut channel_state = ChannelState::new();

    // Reusable packet descriptor buffer (avoids per-chunk allocation).
    let mut packet_descs: Vec<PacketDesc> = Vec::with_capacity(4096);
    let mut movement_rows: u64 = 0;
    let mut overlay_stats = OverlayStats::default();
    let mut error_report = OverlayErrorReport::default();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        let decompressed = decompress_replay_data(payload, compressed, encrypted)?;

        // Phase 1: iterate DemoFrames — populates cache, collects packet locations.
        packet_descs.clear();
        iter_demo_frames(&decompressed, flags, &mut cache, |pkt| {
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

            let mut sink = ExportSink::new(&mut cache, &mut channel_state);
            sink.time_ms = desc.time_ms;
            sink.packet_id = pkt_id;

            repl_reader.process_packet(pkt_data, pkt_id as i32, &mut sink);

            // Drain field records to writer.
            for record in sink.field_records.drain(..) {
                field_writer.push(record)?;
            }
            // Drain movement records to writer.
            for record in sink.movement_records.drain(..) {
                movement_writer.push(record)?;
                movement_rows += 1;
            }
            // Drain actor lifecycle records to writer.
            for record in sink.actor_records.drain(..) {
                actor_writer.push(record)?;
            }
            // Accumulate overlay stats.
            overlay_stats.decoded_ok += sink.stats.overlay.decoded_ok;
            overlay_stats.decoded_err += sink.stats.overlay.decoded_err;
            overlay_stats.raw_or_skip += sink.stats.overlay.raw_or_skip;
            overlay_stats.not_in_table += sink.stats.overlay.not_in_table;
            overlay_stats.no_field_name += sink.stats.overlay.no_field_name;
            error_report.merge_from(&sink.stats.overlay.error_report);
        }

        chunks_processed += 1;

        if chunks_processed % 100 == 0 {
            eprintln!(
                "  chunk {chunks_processed}: {total_packets} packets, {} groups",
                cache.group_count()
            );
        }
    }

    // ── Finish writers ────────────────────────────────────────────────────
    field_writer.finish()?;
    movement_writer.finish()?;
    actor_writer.finish()?;

    // ── Write the NetGUID registry ────────────────────────────────────────
    //
    // Written after the replication pass because the cache accumulates over the
    // whole replay: a GUID's outer may be declared in a later chunk than the
    // one that first referenced it. Sorted so the file is byte-reproducible
    // across runs (the cache is HashMap-backed and iterates in arbitrary order).
    let net_guids_file = BufWriter::new(fs::File::create(out_path.join("net_guids.parquet"))?);
    let mut net_guid_writer = NetGuidWriter::new(net_guids_file)?;
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

    // ── Stats ─────────────────────────────────────────────────────────────
    let net_stats = repl_reader.stats();
    let elapsed = start.elapsed();

    eprintln!();
    eprintln!("=== Export complete ===");
    eprintln!("  Chunks:           {chunks_processed}");
    eprintln!("  Packets:          {total_packets}");
    eprintln!("  Export groups:    {}", cache.group_count());
    eprintln!("  Content blocks:   {}", net_stats.content_blocks);
    eprintln!("  RepLayout blocks: {}", net_stats.rep_layout_blocks);
    eprintln!("  ClassNetCache:    {}", net_stats.class_net_cache_blocks);
    eprintln!("  Fields:           {}", net_stats.fields);
    eprintln!("  RPCs:             {}", net_stats.rpcs);
    eprintln!("  Actor opens:      {}", net_stats.actor_opens);
    eprintln!("  Actor closes:     {}", net_stats.actor_closes);
    eprintln!("  Bunches:          {}", net_stats.bunches);
    eprintln!("  Malformed pkts:   {}", net_stats.malformed_packets);
    eprintln!("  Skipped bits:     {}", net_stats.skipped_bits);
    eprintln!("  Movement rows:    {movement_rows}");
    eprintln!("  NetGUID rows:     {net_guid_rows}");
    eprintln!("  Elapsed:          {:.2?}", elapsed);

    // ── Write manifest ────────────────────────────────────────────────────
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

    // ── Report file sizes ─────────────────────────────────────────────────
    let fields_size = fs::metadata(out_path.join("fields.parquet"))
        .map(|m| m.len())
        .unwrap_or(0);
    let movement_size = fs::metadata(out_path.join("movement.parquet"))
        .map(|m| m.len())
        .unwrap_or(0);
    let actors_size = fs::metadata(out_path.join("actors.parquet"))
        .map(|m| m.len())
        .unwrap_or(0);
    let net_guids_size = fs::metadata(out_path.join("net_guids.parquet"))
        .map(|m| m.len())
        .unwrap_or(0);

    eprintln!();
    eprintln!("  fields.parquet:   {} bytes", fields_size);
    eprintln!("  movement.parquet: {} bytes", movement_size);
    eprintln!("  actors.parquet:   {} bytes", actors_size);
    eprintln!("  net_guids.parquet:{} bytes", net_guids_size);
    eprintln!("  manifest.json:    {}", manifest_path.display());

    // ── Overlay statistics ─────────────────────────────────────────────────
    //
    // The denominator is every row the overlay was offered, which since RPC
    // parameter expansion means replicated properties *and* RPC parameters. The
    // two populations have very different type coverage -- the descriptor set
    // grew up around properties -- so labelling the ratio "of all fields" would
    // read as a regression when parameters were added. Name the denominator
    // instead of leaving it implicit.
    let total_overlay = overlay_stats.decoded_ok
        + overlay_stats.decoded_err
        + overlay_stats.raw_or_skip
        + overlay_stats.not_in_table
        + overlay_stats.no_field_name;
    let pct = if total_overlay > 0 {
        (overlay_stats.decoded_ok as f64 / total_overlay as f64) * 100.0
    } else {
        0.0
    };
    eprintln!();
    eprintln!("=== Type overlay ===");
    eprintln!("  Decoded OK:       {}", overlay_stats.decoded_ok);
    eprintln!("  Decode errors:    {}", overlay_stats.decoded_err);
    eprintln!("  Raw/Skip:         {}", overlay_stats.raw_or_skip);
    eprintln!("  Not in table:     {}", overlay_stats.not_in_table);
    eprintln!("  No field name:    {}", overlay_stats.no_field_name);
    eprintln!("  Rows offered:     {total_overlay}");
    eprintln!("  Typed:            {pct:.1}% (properties + RPC parameters)");

    // Print top-15 decode error breakdown (always -- this is a permanent
    // diagnostic for schema-drift detection across game builds).
    if overlay_stats.decoded_err > 0 {
        let top = error_report.top_n(15);
        eprintln!();
        eprintln!(
            "=== Decode error report ({} distinct buckets, {} total) ===",
            error_report.bucket_count(),
            error_report.total_errors()
        );
        eprintln!(
            "  {:>7}  {:<6}  {:>5}  {:<20}  {:<30}  group_path",
            "count", "kind", "bits", "type", "field_name"
        );
        for row in &top {
            // Truncate group_path for display (show last 60 chars).
            let gp_display = if row.group_path.len() > 60 {
                format!("...{}", &row.group_path[row.group_path.len() - 57..])
            } else {
                row.group_path.clone()
            };
            eprintln!(
                "  {:>7}  {:<6}  {:>5}  {:<20}  {:<30}  {}",
                row.count,
                row.error_kind,
                row.bit_count,
                row.declared_type,
                row.field_name,
                gp_display
            );
        }
    }

    Ok(())
}
