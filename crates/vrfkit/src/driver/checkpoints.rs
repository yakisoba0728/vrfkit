//! The optional Checkpoint pass.
//!
//! A checkpoint is a full-state snapshot (its own GUID cache, export map and a
//! DemoFrame re-opening every live actor), so each gets a fresh decode pass:
//! sharing its cache, reader or channels would leak snapshot opens and
//! archetype mappings into the ReplayData pass. Its rows go to separate
//! tables because packet, channel and NetGUID namespaces restart in each
//! snapshot, which also leaves the main tables byte-identical either way.

use std::io::Write;

use vrf_export::{
    CheckpointActorRecord, CheckpointActorWriter, CheckpointBlockWriter,
    CheckpointExportFieldWriter, CheckpointExportGroupWriter, CheckpointFieldRecord,
    CheckpointFieldWriter, CheckpointGuidEntryWriter, CheckpointNetGuidRecord,
    CheckpointNetGuidWriter, PartialRecord,
};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::SchemaLimits;

use super::writers::WriterThread;
use crate::checkpoints::{CheckpointOutput, decode_checkpoint};
use crate::error::CliError;
use crate::pass::{Replay, net_guid_rows};
use crate::sink::ExportStats;

/// Counters for the optional checkpoint pass, kept together so the summary
/// cannot report one and quietly omit another.
#[derive(Debug, Default)]
pub(crate) struct CheckpointStats {
    pub chunks: u64,
    /// Bytes after each chunk's archive plus archive bytes the Oodle codec
    /// never read; 0 on every corpus checkpoint.
    pub trailing_bytes: u64,
    pub guid_entries: u64,
    pub literal_paths: u64,
    pub indexed_paths: u64,
    pub resolved_path_indices: u64,
    pub group_records: u64,
    pub exported_fields: u64,
    /// DemoFrames walked, not assumed to be one per chunk.
    pub frames: u64,
    pub frame_skips: FrameSkips,
    pub non_finite_frame_times: u64,
    pub packets: u64,
    pub field_rows: u64,
    pub actor_rows_written: u64,
    pub net_guid_rows_written: u64,
    pub block_rows_written: u64,
    pub guid_entry_rows_written: u64,
    pub export_group_rows_written: u64,
    pub export_field_rows_written: u64,
    pub partial_rows: u64,
    pub partial_bits: u64,
    /// Each chunk's opens and closes (one row each) less the actor rows it
    /// wrote.
    pub actor_rows_dropped: u64,
    pub movement_rows_dropped: u64,
    /// Apart from the main pass's baseline-pinned totals, which mixing would
    /// move by a flag-dependent amount.
    pub sink: ExportStats,
    /// Replication/framing counters from every finalized checkpoint reader.
    pub net: NetStats,
}

impl CheckpointStats {
    /// Fold in one finished chunk's reader counters and its dropped actor
    /// rows.
    fn absorb_chunk_net(&mut self, chunk_net: &mut NetStats, actor_rows: u64) {
        self.actor_rows_dropped +=
            (chunk_net.actor_opens + chunk_net.actor_closes).saturating_sub(actor_rows);
        self.net.absorb(chunk_net);
    }
}

pub(super) struct CheckpointWriters<W: Write + Send> {
    pub fields: WriterThread<CheckpointFieldRecord>,
    pub actors: CheckpointActorWriter<W>,
    pub net_guids: CheckpointNetGuidWriter<W>,
    pub blocks: CheckpointBlockWriter<W>,
    pub guid_entries: CheckpointGuidEntryWriter<W>,
    pub export_groups: CheckpointExportGroupWriter<W>,
    pub export_fields: CheckpointExportFieldWriter<W>,
}

impl<W: Write + Send + 'static> CheckpointWriters<W> {
    /// Every checkpoint table's writer, over `create(file name)`.
    pub fn new(mut create: impl FnMut(&str) -> Result<W, CliError>) -> Result<Self, CliError> {
        Ok(Self {
            // The one checkpoint table large enough to take off the decode
            // thread, for the reason `writers` gives for fields and movement.
            fields: WriterThread::spawn_table(
                "checkpoint_fields",
                CheckpointFieldWriter::new(create("checkpoint_fields.parquet")?)?,
            ),
            actors: CheckpointActorWriter::new(create("checkpoint_actors.parquet")?)?,
            net_guids: CheckpointNetGuidWriter::new(create("checkpoint_net_guids.parquet")?)?,
            blocks: CheckpointBlockWriter::new(create("checkpoint_blocks.parquet")?)?,
            guid_entries: CheckpointGuidEntryWriter::new(create(
                "checkpoint_guid_entries.parquet",
            )?)?,
            export_groups: CheckpointExportGroupWriter::new(create(
                "checkpoint_export_groups.parquet",
            )?)?,
            export_fields: CheckpointExportFieldWriter::new(create(
                "checkpoint_export_fields.parquet",
            )?)?,
        })
    }

    pub fn finish(self) -> Result<(), CliError> {
        self.fields.finish()?;
        self.actors.finish()?;
        self.net_guids.finish()?;
        self.blocks.finish()?;
        self.guid_entries.finish()?;
        self.export_groups.finish()?;
        self.export_fields.finish()?;
        Ok(())
    }
}

/// What the checkpoint pass hands back to the main thread.
#[derive(Default)]
pub(super) struct CheckpointPass {
    pub stats: CheckpointStats,
    /// Labelled with their checkpoint, for the main thread's
    /// `partials.parquet` writer, which counts them.
    pub partials: Vec<PartialRecord>,
}

/// Decode every Checkpoint chunk in file order into `writers`, then close
/// them.
pub(super) fn run<W: Write + Send + 'static>(
    replay: &Replay<'_>,
    mut writers: CheckpointWriters<W>,
    chunks: impl FnOnce(&mut dyn FnMut(&[u8]) -> Result<(), CliError>) -> Result<(), CliError>,
) -> Result<CheckpointPass, CliError> {
    let mut out = CheckpointPass::default();
    chunks(&mut |payload| process_chunk(payload, replay, &mut writers, &mut out))?;
    writers.finish()?;
    Ok(out)
}

/// Decode one Checkpoint chunk and write its rows.
fn process_chunk<W: Write + Send>(
    payload: &[u8],
    replay: &Replay<'_>,
    writers: &mut CheckpointWriters<W>,
    out: &mut CheckpointPass,
) -> Result<(), CliError> {
    let CheckpointPass { stats, partials } = out;
    let checkpoint_index = u32::try_from(stats.chunks)
        .map_err(|_| CliError::Usage("too many checkpoint chunks to index".to_owned()))?;
    let mut field_records = Vec::new();
    let decoded = decode_checkpoint(
        payload,
        replay,
        checkpoint_index,
        SchemaLimits::default(),
        false,
        stats.field_rows,
        &mut stats.sink,
        |output| {
            match output {
                CheckpointOutput::Metadata(_) => {}
                CheckpointOutput::GuidEntry(row) => writers.guid_entries.push(row)?,
                CheckpointOutput::ExportGroup(row) => writers.export_groups.push(row)?,
                CheckpointOutput::ExportField(row) => writers.export_fields.push(row)?,
                CheckpointOutput::Records(metadata, buffers) => {
                    writers
                        .blocks
                        .push_batch(buffers.checkpoint_blocks.drain(..))?;
                    field_records.extend(buffers.fields.drain(..).map(|field| {
                        CheckpointFieldRecord {
                            checkpoint: metadata.identity.clone(),
                            field,
                        }
                    }));
                    writers.fields.append(&mut field_records)?;
                    writers
                        .actors
                        .push_batch(buffers.actors.drain(..).map(|actor| {
                            CheckpointActorRecord {
                                checkpoint: metadata.identity.clone(),
                                actor,
                            }
                        }))?;
                    partials.append(&mut buffers.partials);
                }
            }
            Ok(())
        },
    )?;
    let checkpoint = decoded.metadata.identity;
    let pass = decoded.pass;
    let tables = decoded.tables;
    stats.trailing_bytes += decoded.metadata.trailing_bytes;
    stats.guid_entry_rows_written += u64::from(tables.guid_count);
    stats.export_group_rows_written += u64::from(tables.group_count);
    stats.export_field_rows_written += u64::from(tables.exported_fields);
    stats.field_rows += decoded.rows.fields;
    stats.block_rows_written += decoded.rows.blocks;
    stats.actor_rows_written += decoded.rows.actors;
    stats.movement_rows_dropped += decoded.rows.movement;
    stats.absorb_chunk_net(&mut pass.reader.stats().clone(), decoded.rows.actors);

    let net_guids = net_guid_rows(&pass.cache);
    stats.net_guid_rows_written += net_guids.len() as u64;
    writers
        .net_guids
        .push_batch(
            net_guids
                .into_iter()
                .map(|net_guid| CheckpointNetGuidRecord {
                    checkpoint: checkpoint.clone(),
                    net_guid,
                }),
        )?;

    stats.chunks += 1;
    stats.guid_entries += u64::from(tables.guid_count);
    stats.literal_paths += u64::from(tables.literal_paths);
    stats.indexed_paths += u64::from(tables.indexed_paths);
    stats.resolved_path_indices += u64::from(tables.resolved_path_indices);
    stats.group_records += u64::from(tables.group_count);
    stats.exported_fields += u64::from(tables.exported_fields);
    stats.frames += u64::from(pass.frames);
    stats.frame_skips.absorb(pass.frame_skips);
    stats.non_finite_frame_times += pass.non_finite_frame_times;
    stats.packets += u64::from(pass.packets);
    Ok(())
}

#[cfg(test)]
mod tests {
    use vrf_container::{ChunkType, parse_preamble};
    use vrf_testkit::{Info, chunk, header_payload, replay_info};

    use super::*;
    use crate::pass::replay_fixtures::*;

    /// The main thread writes these rows as they come back, so each must
    /// already name its own snapshot; no measured replay has one.
    #[test]
    fn partial_rows_come_back_labelled_with_their_own_checkpoint() {
        let packet = unfinished_partial_packet();
        let snapshot = checkpoint_tables(&frame(1.5, &[], 0, &[&packet]));
        let mut data = replay_info(&Info::default());
        data.extend(chunk(0, &header_payload()));
        for index in 0..2 {
            data.extend(chunk(2, &checkpoint(index, &snapshot, 0)));
        }
        let preamble = parse_preamble(&data).unwrap();
        let writers = CheckpointWriters::new(|_| Ok(Vec::new())).unwrap();
        let replay = Replay::new(&preamble);
        let out = run(&replay, writers, |on_checkpoint| {
            for chunk in replay.chunks(&data) {
                if let (ChunkType::Checkpoint, payload) = chunk? {
                    on_checkpoint(payload)?;
                }
            }
            Ok(())
        })
        .unwrap();
        let labels: Vec<_> = (out.partials.iter())
            .map(|row| (row.source, row.checkpoint_id.as_deref()))
            .collect();
        assert_eq!(
            labels,
            [
                ("checkpoint", Some("checkpoint0")),
                ("checkpoint", Some("checkpoint1"))
            ]
        );
    }

    /// Per chunk, so one chunk's surplus cannot cancel another's loss.
    #[test]
    fn actor_rows_a_chunk_opened_or_closed_but_did_not_write_are_dropped() {
        let mut stats = CheckpointStats::default();
        let mut clean = NetStats::default();
        clean.actor_opens = 3;
        clean.actor_closes = 1;
        stats.absorb_chunk_net(&mut clean, 4);
        assert_eq!(stats.actor_rows_dropped, 0);

        let mut lossy = NetStats::default();
        lossy.actor_opens = 2;
        stats.absorb_chunk_net(&mut lossy, 1);
        assert_eq!(stats.actor_rows_dropped, 1, "one open wrote no row");

        let mut surplus = NetStats::default();
        surplus.actor_opens = 1;
        stats.absorb_chunk_net(&mut surplus, 2);
        assert_eq!(stats.actor_rows_dropped, 1, "a surplus is not a refund");
        assert_eq!((stats.net.actor_opens, stats.net.actor_closes), (6, 1));
    }
}
