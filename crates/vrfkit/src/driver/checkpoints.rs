//! The optional Checkpoint pass.
//!
//! A checkpoint is a full-state snapshot: its own guid cache, its own export
//! map, and one DemoFrame re-opening every actor alive at that instant.
//! Everything about it is independent of the live stream, so it gets its own
//! cache, reader, channel state and buffers. Sharing any of the four would let
//! the snapshot's channel opens and archetype mappings leak into the ReplayData
//! pass and corrupt it.
//!
//! Checkpoint fields go to their own table rather than into `fields.parquet`
//! with a source column. Two reasons, in order: a column on 1.2M rows to mark
//! 80k of them is the wrong shape, and `fields.parquet` is read by the valplay
//! adapter, whose capture predicate keys on a row having no decoded value --
//! changing that file's population risks the metric parity for no gain. The
//! file is only created when the flag asks for it, so a default export is
//! byte-identical to one from before this existed.

use std::io::Write;

use vrf_container::{decompress_checkpoint, parse_checkpoint_chunk};
use vrf_decode::OverlayErrorReport;
use vrf_export::FieldWriter;
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_net::stats::NetStats;
use vrf_schema::{NetGuidCache, read_checkpoint_tables};

use super::totals::SinkTotals;
use crate::error::CliError;
use crate::sink::{ChannelState, ExportSink, RecordBuffers};

/// Counters for the optional checkpoint pass. Kept together so the summary
/// cannot report one and quietly omit another.
#[derive(Debug, Default)]
pub(super) struct CheckpointStats {
    pub chunks: u64,
    pub guid_entries: u64,
    pub group_records: u64,
    pub exported_fields: u64,
    pub frames: u64,
    pub packets: u64,
    pub field_rows: u64,
    /// Actor opens and movement samples the snapshot produced. They are
    /// counted and dropped, not written: a checkpoint re-opens a channel for
    /// every actor alive at that instant, so folding them into
    /// `actors.parquet` would triple its rows with re-opens that are not
    /// spawns, and `movement.parquet` is a time series that a snapshot's
    /// replayed samples would duplicate. Reported so the drop is visible.
    pub actor_rows_dropped: u64,
    pub movement_rows_dropped: u64,
    /// Everything the checkpoint sinks counted.
    ///
    /// Kept separately from the ReplayData pass's totals, which the export
    /// baseline pins: mixing them would move a guarded figure by an amount that
    /// depends on a flag. Kept *at all* because the checkpoint sink is a second
    /// decode path, and a failure on it that reached no counter would be
    /// exactly the silent failure this project keeps finding.
    ///
    /// It used to be a hand-picked subset -- overlay, effect blobs, struct
    /// blobs, MultiContents -- and the ones left out were precisely the failure
    /// counters: array-decode errors, truncated RPCs and movement-decode
    /// errors. A checkpoint array that overran mid-element therefore wrote its
    /// parent raw row, lost its flattened children, and recorded nothing
    /// anywhere. Sharing [`SinkTotals`] with the main pass is what stops the
    /// two from drifting again.
    pub sink: SinkTotals,
    /// Replication/framing counters from every finalized checkpoint reader.
    pub net: NetStats,
}

/// Everything about the replay that the checkpoint pass needs and cannot
/// rediscover from the chunk alone.
pub(super) struct ReplayContext<'a> {
    pub branch: &'a str,
    pub flags: u32,
    pub compressed: bool,
    pub encrypted: bool,
}

/// Decode one Checkpoint chunk and write its field rows.
///
/// `error_report` is the *shared* one: a decode error is a decode error
/// wherever it happened, and the breakdown the summary prints is the only place
/// a checkpoint-only failure would ever be seen.
pub(super) fn process_chunk<W: Write + Send>(
    payload: &[u8],
    ctx: &ReplayContext<'_>,
    writer: &mut FieldWriter<W>,
    stats: &mut CheckpointStats,
    error_report: &mut OverlayErrorReport,
) -> Result<(), CliError> {
    let cp = parse_checkpoint_chunk(payload)?;
    let plain = decompress_checkpoint(cp.archive, ctx.compressed, ctx.encrypted)?;

    let mut cache = NetGuidCache::new();
    let tables = read_checkpoint_tables(&plain, &mut cache)
        .map_err(|e| CliError::Usage(format!("checkpoint {}: {e}", cp.id)))?;

    let frame = &plain[tables.frame_offset..];
    let mut reader = ReplicationReader::new(ctx.branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;
    let mut channels = ChannelState::new();
    let mut buffers = RecordBuffers::default();
    let mut packet_count = 0u64;
    let mut packet_error = None;
    iter_demo_frames(frame, ctx.flags, &mut cache, |pkt, packet_cache| {
        if packet_error.is_some() {
            return;
        }
        {
            let mut sink = ExportSink::new(packet_cache, &mut channels, &mut buffers);
            sink.time_ms = pkt.time_ms;
            sink.packet_id = packet_count as u32;
            reader.process_packet(pkt.data, packet_count as i32, &mut sink);
            // Same aggregation the ReplayData pass uses, so the two cannot
            // diverge on which counters they bother to read. See `totals`.
            stats.sink.absorb(&mut sink.stats, error_report);
        }
        let result = (|| -> Result<(), CliError> {
            stats.field_rows += buffers.fields.len() as u64;
            writer.push_batch(buffers.fields.drain(..))?;
            stats.actor_rows_dropped += buffers.actors.len() as u64;
            stats.movement_rows_dropped += buffers.movement.len() as u64;
            buffers.actors.clear();
            buffers.movement.clear();
            Ok(())
        })();
        if let Err(error) = result {
            packet_error = Some(error);
        }
        packet_count += 1;
    })?;
    if let Some(error) = packet_error {
        return Err(error);
    }
    reader.finish();
    let mut chunk_net = reader.stats().clone();
    stats.net.absorb(&mut chunk_net);

    stats.chunks += 1;
    stats.guid_entries += u64::from(tables.guid_count);
    stats.group_records += u64::from(tables.group_count);
    stats.exported_fields += u64::from(tables.exported_fields);
    stats.frames += 1;
    stats.packets += packet_count;
    Ok(())
}
