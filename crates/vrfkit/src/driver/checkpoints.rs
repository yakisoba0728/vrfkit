//! The optional Checkpoint pass.
//!
//! A checkpoint is a full-state snapshot (its own GUID cache, export map and a
//! DemoFrame re-opening every live actor), so each gets a fresh [`Pass`]:
//! sharing its cache, reader or channels would leak snapshot opens and
//! archetype mappings into the ReplayData pass. Its rows go to separate
//! tables because packet, channel and NetGUID namespaces restart in each
//! snapshot, which also leaves the main tables byte-identical either way.

use std::io::Write;

use vrf_container::{decompress_checkpoint_with_trailing, parse_checkpoint_chunk};
use vrf_export::{
    CheckpointActorRecord, CheckpointActorWriter, CheckpointBlockWriter,
    CheckpointExportFieldRecord, CheckpointExportFieldWriter, CheckpointExportGroupRecord,
    CheckpointExportGroupWriter, CheckpointFieldRecord, CheckpointFieldWriter,
    CheckpointGuidEntryRecord, CheckpointGuidEntryWriter, CheckpointIdentity,
    CheckpointNetGuidRecord, CheckpointNetGuidWriter, PartialRecord,
};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::{CheckpointReadError, CheckpointTableSink, read_checkpoint_tables_with_sink};

use super::net_guid_rows;
use super::writers::WriterThread;
use crate::error::CliError;
use crate::pass::{Pass, Replay};
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

struct DeclarationWriter<'a, W: Write + Send> {
    checkpoint: CheckpointIdentity,
    guid_entries: &'a mut CheckpointGuidEntryWriter<W>,
    export_groups: &'a mut CheckpointExportGroupWriter<W>,
    export_fields: &'a mut CheckpointExportFieldWriter<W>,
    guid_rows: u64,
    group_rows: u64,
    field_rows: u64,
}

impl<W: Write + Send> CheckpointTableSink for DeclarationWriter<'_, W> {
    type Error = vrf_export::ExportError;

    fn on_guid_entry(
        &mut self,
        ordinal: u32,
        guid: u32,
        outer: u32,
        path_is_string: bool,
        literal_path: Option<&str>,
        name_index: Option<u32>,
        flags: u8,
    ) -> Result<(), Self::Error> {
        self.guid_entries.push(CheckpointGuidEntryRecord {
            checkpoint: self.checkpoint.clone(),
            ordinal,
            net_guid: guid,
            outer_net_guid: outer,
            path_is_string,
            literal_path: literal_path.map(Into::into),
            name_index,
            flags,
        })?;
        self.guid_rows += 1;
        Ok(())
    }

    fn on_export_group(
        &mut self,
        ordinal: u32,
        path_name_index: u32,
        group_path: &str,
        declared_slots: u32,
    ) -> Result<(), Self::Error> {
        self.export_groups.push(CheckpointExportGroupRecord {
            checkpoint: self.checkpoint.clone(),
            ordinal,
            path_name_index,
            group_path: group_path.into(),
            declared_slots,
        })?;
        self.group_rows += 1;
        Ok(())
    }

    fn on_export_field(
        &mut self,
        group_ordinal: u32,
        path_name_index: u32,
        slot: u32,
        handle: u32,
        checksum: u32,
        rendered_name: &str,
        exported_flag: u8,
        fname_kind: u8,
        base: Option<&str>,
        index: Option<u32>,
        number: Option<i32>,
    ) -> Result<(), Self::Error> {
        self.export_fields.push(CheckpointExportFieldRecord {
            checkpoint: self.checkpoint.clone(),
            group_ordinal,
            path_name_index,
            slot,
            handle,
            compatible_checksum: checksum,
            rendered_name: rendered_name.into(),
            exported_flag,
            fname_kind,
            fname_base: base.map(Into::into),
            fname_index: index,
            fname_number: number,
        })?;
        self.field_rows += 1;
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
    let cp = parse_checkpoint_chunk(payload)?;
    let (plain, unread) =
        decompress_checkpoint_with_trailing(cp.archive, replay.compressed, replay.encrypted)?;
    stats.trailing_bytes += (cp.trailing_bytes + unread) as u64;

    let checkpoint_index = u32::try_from(stats.chunks)
        .map_err(|_| CliError::Usage("too many checkpoint chunks to index".to_owned()))?;
    let checkpoint = CheckpointIdentity {
        checkpoint_index,
        checkpoint_id: cp.id.clone().into(),
    };
    let mut pass = Pass::new(replay)?;
    let mut declarations = DeclarationWriter {
        checkpoint: checkpoint.clone(),
        guid_entries: &mut writers.guid_entries,
        export_groups: &mut writers.export_groups,
        export_fields: &mut writers.export_fields,
        guid_rows: 0,
        group_rows: 0,
        field_rows: 0,
    };
    let tables = read_checkpoint_tables_with_sink(&plain, &mut pass.cache, &mut declarations)
        .map_err(|error| match error {
            CheckpointReadError::Schema(error) => {
                CliError::Usage(format!("checkpoint {}: {error}", cp.id))
            }
            CheckpointReadError::Sink(error) => CliError::Export(error),
        })?;
    if declarations.guid_rows != u64::from(tables.guid_count)
        || declarations.group_rows != u64::from(tables.group_count)
        || declarations.field_rows != u64::from(tables.exported_fields)
    {
        return Err(CliError::Usage(format!(
            "checkpoint {} declaration row counts disagree with parsed tables",
            cp.id
        )));
    }
    stats.guid_entry_rows_written += declarations.guid_rows;
    stats.export_group_rows_written += declarations.group_rows;
    stats.export_field_rows_written += declarations.field_rows;

    let actor_rows_before = stats.actor_rows_written;
    pass.block_scope = Some((checkpoint.clone(), stats.field_rows, 0));
    let mut field_records = Vec::new();
    let first_partial = partials.len();
    pass.walk(&plain[tables.frame_offset..], &mut stats.sink, |buffers| {
        stats.block_rows_written += buffers.checkpoint_blocks.len() as u64;
        writers
            .blocks
            .push_batch(buffers.checkpoint_blocks.drain(..))?;
        stats.field_rows += buffers.fields.len() as u64;
        field_records.extend(buffers.fields.drain(..).map(|field| CheckpointFieldRecord {
            checkpoint: checkpoint.clone(),
            field,
        }));
        writers.fields.append(&mut field_records)?;
        stats.actor_rows_written += buffers.actors.len() as u64;
        writers
            .actors
            .push_batch(buffers.actors.drain(..).map(|actor| CheckpointActorRecord {
                checkpoint: checkpoint.clone(),
                actor,
            }))?;
        stats.movement_rows_dropped += buffers.movement.len() as u64;
        partials.append(&mut buffers.partials);
        Ok(())
    })?;
    pass.finish();
    partials.append(&mut pass.buffers.partials);
    for record in &mut partials[first_partial..] {
        record.source = "checkpoint";
        record.checkpoint_id = Some(cp.id.clone());
    }
    let chunk_actor_rows = stats.actor_rows_written - actor_rows_before;
    stats.absorb_chunk_net(&mut pass.reader.stats().clone(), chunk_actor_rows);

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
        let mut clean = NetStats {
            actor_opens: 3,
            actor_closes: 1,
            ..NetStats::default()
        };
        stats.absorb_chunk_net(&mut clean, 4);
        assert_eq!(stats.actor_rows_dropped, 0);

        let mut lossy = NetStats {
            actor_opens: 2,
            ..NetStats::default()
        };
        stats.absorb_chunk_net(&mut lossy, 1);
        assert_eq!(stats.actor_rows_dropped, 1, "one open wrote no row");

        let mut surplus = NetStats {
            actor_opens: 1,
            ..NetStats::default()
        };
        stats.absorb_chunk_net(&mut surplus, 2);
        assert_eq!(stats.actor_rows_dropped, 1, "a surplus is not a refund");
        assert_eq!((stats.net.actor_opens, stats.net.actor_closes), (6, 1));
    }
}
