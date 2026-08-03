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
//! # Writer offload
//!
//! `fields` and `movement` are the two large tables and their Parquet encoding
//! (Arrow batch build + ZSTD) was measured at 570 ms and 450 ms of a 2.60 s
//! export -- 37% of the run, executed inline in the packet loop. Each table is
//! an independent file whose writer never reads replay state, so each is moved
//! to its own thread and fed record batches over a bounded channel. The writers
//! still see every record exactly once, in stream order, and the row-group flush
//! boundary still falls on the same cumulative row counts, so the bytes are
//! unchanged; only the thread they are produced on differs.
//!
//! The channels are bounded so a slow writer applies backpressure instead of
//! growing the in-flight batch queue without limit. `actors`, `net_guids` and
//! `events` stay inline: together they are under 1% of the write cost.
//!
//! No error is dropped on this path. A writer that fails returns its error and
//! drops its receiver, which turns the next `send` into an error the packet loop
//! propagates; the deferred error is then recovered at join. A writer thread that
//! panics is reported as an error rather than being mistaken for success.

use std::fs;
use std::io::BufWriter;
use std::path::Path;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use std::time::Instant;

use vrf_container::{
    ChunkIterator, ChunkType, decompress_checkpoint, decompress_replay_data,
    parse_checkpoint_chunk, parse_event_chunk, parse_preamble,
};
use vrf_decode::{OverlayErrorReport, OverlayStats};
use vrf_export::{
    ActorWriter, EventRecord, EventWriter, ExportError, FieldRecord, FieldWriter, MovementRecord,
    MovementWriter, NetGuidRecord, NetGuidWriter,
};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_schema::{NetGuidCache, read_checkpoint_tables};

use crate::error::CliError;
use crate::manifest;
use crate::sink::ChannelState;
use crate::sink::ExportSink;
use crate::sink::RecordBuffers;

/// A packet descriptor collected from DemoFrame iteration.
/// Stores byte offset + length into the decompressed chunk buffer.
struct PacketDesc {
    time_ms: u32,
    offset: usize,
    len: usize,
}

/// Rows accumulated in the packet loop before a batch is handed to a writer
/// thread. A replay yields ~530 k packets but only ~0.8 field rows and ~3.5
/// movement rows per packet, so sending one message per packet would cost more
/// in channel traffic than the encoding it hides. At this size `fields` sends
/// ~26 messages and `movement` ~112 over a whole replay.
const WRITER_BATCH_ROWS: usize = 16_384;

/// Batches allowed in flight per writer. Bounds peak memory: 4 field batches is
/// roughly 8 MB of records plus their string payloads.
const WRITER_QUEUE_DEPTH: usize = 4;

/// A writer running on its own thread, plus the handle needed to collect its
/// result. `T` is the record type of the table it owns.
struct WriterThread<T> {
    tx: Option<SyncSender<Vec<T>>>,
    handle: thread::JoinHandle<Result<(), ExportError>>,
    batch: Vec<T>,
    /// Table name, used only to name the failing table in an error message.
    table: &'static str,
}

impl<T: Send + 'static> WriterThread<T> {
    /// Spawn a writer thread driven by `run`, which consumes every batch in
    /// stream order and then finalises the file.
    fn spawn<F>(table: &'static str, run: F) -> Self
    where
        F: FnOnce(std::sync::mpsc::Receiver<Vec<T>>) -> Result<(), ExportError> + Send + 'static,
    {
        let (tx, rx) = sync_channel::<Vec<T>>(WRITER_QUEUE_DEPTH);
        let handle = thread::spawn(move || run(rx));
        Self {
            tx: Some(tx),
            handle,
            batch: Vec::with_capacity(WRITER_BATCH_ROWS),
            table,
        }
    }

    /// Move `records` into the pending batch, shipping it once it is full.
    fn append(&mut self, records: &mut Vec<T>) -> Result<(), CliError> {
        self.batch.append(records);
        if self.batch.len() >= WRITER_BATCH_ROWS {
            self.ship()?;
        }
        Ok(())
    }

    fn ship(&mut self) -> Result<(), CliError> {
        let full = std::mem::replace(&mut self.batch, Vec::with_capacity(WRITER_BATCH_ROWS));
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| CliError::Usage(format!("{} writer already closed", self.table)))?;
        // A send failure means the writer thread returned early with an error.
        // Report it as a send failure only if joining cannot produce the real
        // cause; `finish` below re-reads the thread result either way.
        tx.send(full)
            .map_err(|_| CliError::Usage(format!("{} writer stopped early", self.table)))
    }

    /// Ship the trailing partial batch, close the channel and surface the
    /// writer's own result. Any panic in the writer becomes an error here --
    /// it must never be mistaken for a completed file.
    fn finish(mut self) -> Result<(), CliError> {
        let send_result = if self.batch.is_empty() {
            Ok(())
        } else {
            self.ship()
        };
        // Dropping the sender is what ends the writer loop.
        self.tx = None;
        let table = self.table;
        match self.handle.join() {
            Ok(Ok(())) => send_result,
            // The writer's own error is the real cause; it supersedes a send
            // failure caused by that same early return.
            Ok(Err(e)) => Err(CliError::Export(e)),
            Err(_) => Err(CliError::Usage(format!("{table} writer thread panicked"))),
        }
    }
}

/// Counters for the optional checkpoint pass. Kept together so the summary
/// cannot report one and quietly omit another.
#[derive(Debug, Default)]
struct CheckpointStats {
    chunks: u64,
    guid_entries: u64,
    group_records: u64,
    exported_fields: u64,
    frames: u64,
    packets: u64,
    field_rows: u64,
    /// Actor opens and movement samples the snapshot produced. They are
    /// counted and dropped, not written: a checkpoint re-opens a channel for
    /// every actor alive at that instant, so folding them into
    /// `actors.parquet` would triple its rows with re-opens that are not
    /// spawns, and `movement.parquet` is a time series that a snapshot's
    /// replayed samples would duplicate. Reported so the drop is visible.
    actor_rows_dropped: u64,
    movement_rows_dropped: u64,
    /// Overlay outcome for checkpoint rows.
    ///
    /// Kept separately rather than folded into the main `overlay_stats`, which
    /// the export baseline pins: mixing them would move a guarded figure by an
    /// amount that depends on a flag. Kept *at all* because the checkpoint sink
    /// is a second decode path, and a decode error on it that reached no
    /// counter would be exactly the silent failure this project keeps finding.
    overlay: OverlayStats,
    effect_blobs: u64,
}

pub fn run(vrf_path: &str, out_dir: &str, with_checkpoints: bool) -> Result<(), CliError> {
    let start = Instant::now();

    // -- Read file ---------------------------------------------------------
    eprintln!("reading {vrf_path}...");
    let data = fs::read(vrf_path)?;
    let file_size = data.len();

    // -- Parse preamble ----------------------------------------------------
    let preamble = parse_preamble(&data)?;
    let branch = &preamble.header.replay_version.branch;
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;

    eprintln!(
        "branch: {branch}, flags: 0x{flags:04X}, compressed: {compressed}, duration: {} ms",
        preamble.info.length_in_ms
    );

    // -- Setup output ------------------------------------------------------
    let out_path = Path::new(out_dir);
    fs::create_dir_all(out_path)?;

    let fields_file = BufWriter::new(fs::File::create(out_path.join("fields.parquet"))?);
    let movement_file = BufWriter::new(fs::File::create(out_path.join("movement.parquet"))?);
    let actors_file = BufWriter::new(fs::File::create(out_path.join("actors.parquet"))?);
    let events_file = BufWriter::new(fs::File::create(out_path.join("events.parquet"))?);

    let mut field_writer = FieldWriter::new(fields_file)?;
    let mut movement_writer = MovementWriter::new(movement_file)?;
    let mut actor_writer = ActorWriter::new(actors_file)?;
    // Event chunks are a couple of hundred rows and are written inline for the
    // same reason `actors` is: the encoding cost is far below a thread's worth.
    let mut event_writer = EventWriter::new(events_file)?;

    // Checkpoint fields go to their own table rather than into `fields.parquet`
    // with a source column. Two reasons, in order: a column on 1.2M rows to
    // mark 80k of them is the wrong shape, and `fields.parquet` is read by the
    // valplay adapter, whose capture predicate keys on a row having no decoded
    // value -- changing that file's population risks the metric parity for no
    // gain. The file is only created when the flag asks for it, so a default
    // export is byte-identical to one from before this existed.
    let mut checkpoint_writer = if with_checkpoints {
        let f = BufWriter::new(fs::File::create(
            out_path.join("checkpoint_fields.parquet"),
        )?);
        Some(FieldWriter::new(f)?)
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
    let mut repl_reader = ReplicationReader::new(branch)
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
    // Payload bytes an Event chunk declared that its own header layout does not
    // reach. Zero across the corpus; counted rather than dropped in silence.
    let mut event_trailing_bytes: u64 = 0;
    let mut overlay_stats = OverlayStats::default();
    let mut error_report = OverlayErrorReport::default();
    let mut effect_blobs_decoded: u64 = 0;
    let mut cp_stats = CheckpointStats::default();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        // Event chunks carry the server's own labelled timeline. They are
        // uncompressed and independent of the replication pass, so they are
        // read here and written straight out.
        if chunk.chunk_type == ChunkType::Event {
            let payload =
                &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
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
        // A checkpoint is a full-state snapshot: its own guid cache, its own
        // export map, and one DemoFrame re-opening every actor alive at that
        // instant. Everything about it is independent of the live stream, so
        // it gets its own cache, reader, channel state and buffers. Sharing
        // any of the four would let the snapshot's channel opens and archetype
        // mappings leak into the ReplayData pass and corrupt it.
        if chunk.chunk_type == ChunkType::Checkpoint {
            let Some(writer) = checkpoint_writer.as_mut() else {
                continue;
            };
            let payload =
                &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
            let cp = parse_checkpoint_chunk(payload)?;
            let plain = decompress_checkpoint(cp.archive, compressed, encrypted)?;

            let mut cp_cache = NetGuidCache::new();
            let tables = read_checkpoint_tables(&plain, &mut cp_cache)
                .map_err(|e| CliError::Usage(format!("checkpoint {}: {e}", cp.id)))?;

            let mut cp_packets: Vec<(u32, usize, usize)> = Vec::new();
            iter_demo_frames(&plain[tables.frame_offset..], flags, &mut cp_cache, |pkt| {
                let base = plain[tables.frame_offset..].as_ptr() as usize;
                cp_packets.push((
                    pkt.time_ms,
                    pkt.data.as_ptr() as usize - base,
                    pkt.data.len(),
                ));
            })?;

            let mut cp_reader = ReplicationReader::new(branch)
                .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;
            let mut cp_channels = ChannelState::new();
            let mut cp_buffers = RecordBuffers::default();
            let frame = &plain[tables.frame_offset..];
            for (i, (time_ms, off, len)) in cp_packets.iter().enumerate() {
                {
                    let mut sink =
                        ExportSink::new(&mut cp_cache, &mut cp_channels, &mut cp_buffers);
                    sink.time_ms = *time_ms;
                    sink.packet_id = i as u32;
                    cp_reader.process_packet(&frame[*off..*off + *len], i as i32, &mut sink);
                    cp_stats.overlay.decoded_ok += sink.stats.overlay.decoded_ok;
                    cp_stats.overlay.decoded_err += sink.stats.overlay.decoded_err;
                    cp_stats.overlay.raw_or_skip += sink.stats.overlay.raw_or_skip;
                    cp_stats.overlay.not_in_table += sink.stats.overlay.not_in_table;
                    cp_stats.overlay.no_field_name += sink.stats.overlay.no_field_name;
                    cp_stats.effect_blobs += sink.stats.effect_blobs_decoded;
                    error_report.merge_from(&sink.stats.overlay.error_report);
                }
                cp_stats.field_rows += cp_buffers.fields.len() as u64;
                writer.push_batch(std::mem::take(&mut cp_buffers.fields))?;
                cp_stats.actor_rows_dropped += cp_buffers.actors.len() as u64;
                cp_stats.movement_rows_dropped += cp_buffers.movement.len() as u64;
                cp_buffers.actors.clear();
                cp_buffers.movement.clear();
            }

            cp_stats.chunks += 1;
            cp_stats.guid_entries += u64::from(tables.guid_count);
            cp_stats.group_records += u64::from(tables.group_count);
            cp_stats.exported_fields += u64::from(tables.exported_fields);
            cp_stats.frames += 1;
            cp_stats.packets += cp_packets.len() as u64;
            continue;
        }
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        let decompressed = decompress_replay_data(payload, compressed, encrypted)?;

        // Phase 1: iterate DemoFrames -- populates cache, collects packet locations.
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

            // Hand field records to the fields writer thread.
            fields.append(&mut buffers.fields)?;
            // Hand movement records to the movement writer thread.
            movement_rows += buffers.movement.len() as u64;
            movement.append(&mut buffers.movement)?;
            // Drain actor lifecycle records to writer.
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

    // -- Stats -------------------------------------------------------------
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
    eprintln!("  Event rows:       {event_rows}");
    if event_trailing_bytes > 0 {
        eprintln!("  Event unread:     {event_trailing_bytes} payload bytes");
    }
    eprintln!("  Elapsed:          {:.2?}", elapsed);

    if with_checkpoints {
        eprintln!();
        eprintln!("=== Checkpoints ===");
        eprintln!("  Checkpoints:      {}", cp_stats.chunks);
        eprintln!("  GUID entries:     {}", cp_stats.guid_entries);
        eprintln!("  Group records:    {}", cp_stats.group_records);
        eprintln!("  Exported fields:  {}", cp_stats.exported_fields);
        eprintln!("  Frames:           {}", cp_stats.frames);
        eprintln!("  Frame packets:    {}", cp_stats.packets);
        eprintln!("  Checkpoint rows:  {}", cp_stats.field_rows);
        // Printed, not silent: a checkpoint re-opens every live actor and
        // replays its state, so these two would corrupt the tables they would
        // otherwise land in. See CheckpointStats.
        eprintln!(
            "  Dropped:          {} actor / {} movement rows (snapshot re-opens)",
            cp_stats.actor_rows_dropped, cp_stats.movement_rows_dropped
        );
        eprintln!(
            "  Overlay:          {} decoded / {} errors / {} raw-skip / {} not-in-table / {} unnamed / {} effect blobs",
            cp_stats.overlay.decoded_ok,
            cp_stats.overlay.decoded_err,
            cp_stats.overlay.raw_or_skip,
            cp_stats.overlay.not_in_table,
            cp_stats.overlay.no_field_name,
            cp_stats.effect_blobs
        );
    }

    // -- Write manifest ----------------------------------------------------
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

    // -- Report file sizes -------------------------------------------------
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
    let events_size = fs::metadata(out_path.join("events.parquet"))
        .map(|m| m.len())
        .unwrap_or(0);

    eprintln!();
    eprintln!("  fields.parquet:   {} bytes", fields_size);
    eprintln!("  movement.parquet: {} bytes", movement_size);
    eprintln!("  actors.parquet:   {} bytes", actors_size);
    eprintln!("  net_guids.parquet:{} bytes", net_guids_size);
    eprintln!("  events.parquet:   {} bytes", events_size);
    if with_checkpoints {
        let size = fs::metadata(out_path.join("checkpoint_fields.parquet"))
            .map(|m| m.len())
            .unwrap_or(0);
        eprintln!("  checkpoint_fields.parquet: {size} bytes");
    }
    eprintln!("  manifest.json:    {}", manifest_path.display());

    // -- Overlay statistics -------------------------------------------------
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
    // Reported separately because it is NOT part of the ratio above. The
    // overlay buckets are decided before the effect pass runs, so these rows
    // are already counted as `Not in table` and stay there; adding them to
    // `Decoded OK` would double-count them and move a figure the baseline
    // pins for a different reason. The two numbers answer different questions:
    // how much the static table covers, and how much this decoder recovered
    // from what the table does not.
    eprintln!("  Effect blobs:     {effect_blobs_decoded}");

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that fails must never be reported as a finished file.
    ///
    /// Moving the Parquet writers onto threads moved their errors off the
    /// `?` path, which is exactly the shape of a silent success. This drives the
    /// failure deliberately: the writer returns an error and drops its receiver,
    /// the producing side sees a send failure, and `finish` must surface the
    /// writer's own error rather than either the send failure or `Ok`.
    #[test]
    fn a_failed_writer_thread_is_reported_not_swallowed() {
        let mut writer = WriterThread::<u8>::spawn("test", |rx| {
            // Take one batch, then fail -- the shape of a Parquet codec error.
            let _ = rx.recv();
            Err(ExportError::Usage("writer failed".into()))
        });

        // Keep shipping until the broken channel is observed, or until enough
        // batches have gone in to guarantee it would have been.
        let mut saw_send_failure = false;
        for _ in 0..(WRITER_QUEUE_DEPTH + 4) {
            let mut batch = vec![0u8; WRITER_BATCH_ROWS];
            if writer.append(&mut batch).is_err() {
                saw_send_failure = true;
                break;
            }
        }

        let err = writer
            .finish()
            .expect_err("a failed writer must not report success");
        assert!(
            err.to_string().contains("writer failed"),
            "finish must surface the writer's own error, got: {err}"
        );
        // Not asserted as required: whether the producer noticed first is a
        // race. What must hold is that finish reports the failure either way.
        let _ = saw_send_failure;
    }

    /// A panicking writer thread must also be an error. `JoinHandle::join`
    /// returns `Err` on panic and it would be easy to discard.
    ///
    /// The panic message this prints on stderr during `cargo test` is expected.
    #[test]
    fn a_panicking_writer_thread_is_reported_not_swallowed() {
        let writer = WriterThread::<u8>::spawn("test", |_rx| panic!("writer died"));
        let err = writer
            .finish()
            .expect_err("a panicking writer must not report success");
        assert!(
            err.to_string().contains("panicked"),
            "finish must name the panic, got: {err}"
        );
    }
}
