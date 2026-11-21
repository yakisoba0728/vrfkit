//! The snapshot decoder shared by the optional Reader and CLI adapters.

use std::collections::BTreeMap;

use vrf_container::{decompress_checkpoint_with_trailing, parse_checkpoint_chunk};
use vrf_decode::OverlayErrorRow;
use vrf_export::{
    CheckpointBlockRecord, CheckpointExportFieldRecord, CheckpointExportGroupRecord,
    CheckpointGuidEntryRecord, CheckpointIdentity, NetGuidRecord,
};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::{
    CheckpointReadError, CheckpointTableSink, CheckpointTables, SchemaLimits,
    read_checkpoint_tables_with_sink,
};

use crate::error::CliError;
use crate::pass::{Pass, Replay, net_guid_rows};
use crate::reader::ReplayRecords;
use crate::sink::{ExportStats, RecordBuffers};

/// The wire identity and times of one independent snapshot.
///
/// Packet, channel and GUID namespaces restart for every checkpoint. The
/// index is its zero-based position among checkpoint chunks, even when wire
/// IDs repeat. Neither time is a conversion of the enclosed frame times.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CheckpointMetadata {
    pub identity: CheckpointIdentity,
    pub group: String,
    pub metadata: String,
    pub time1: u32,
    pub time2: u32,
    pub archive_size_bytes: u64,
    /// Outer chunk bytes and compressed-archive bytes the decoder did not use.
    pub trailing_bytes: u64,
}

/// One checkpoint notification within [`crate::ReplayEvent::Checkpoint`].
///
/// Metadata comes first, followed by declarations in wire order, packet
/// batches and final rejected partials, then one summary. All remain
/// provisional until the enclosing Reader call returns successfully. Borrowed
/// rows and summaries must be cloned to retain them past the callback.
#[derive(Debug)]
#[non_exhaustive]
pub enum CheckpointEvent<'a> {
    Metadata(&'a CheckpointMetadata),
    GuidEntry(&'a CheckpointGuidEntryRecord),
    ExportGroup(&'a CheckpointExportGroupRecord),
    ExportField(&'a CheckpointExportFieldRecord),
    Records {
        checkpoint: &'a CheckpointMetadata,
        records: ReplayRecords<'a>,
        /// Exact content-block boundaries, including empty blocks. Field-row
        /// offsets count checkpoint fields across all snapshots, as in the CLI.
        blocks: &'a [CheckpointBlockRecord],
    },
    /// This snapshot completed, but a later main or checkpoint chunk may fail.
    Summary(&'a CheckpointSummary),
}

/// Detached results for one snapshot, never mixed with main-stream state.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CheckpointSummary {
    pub metadata: CheckpointMetadata,
    pub tables: CheckpointTables,
    pub packets: u32,
    pub frames: u32,
    pub field_rows: u64,
    pub movement_rows: u64,
    pub actor_rows: u64,
    pub partial_rows: u64,
    pub block_rows: u64,
    pub frame_skips: FrameSkips,
    pub non_finite_frame_times: u64,
    pub network: NetStats,
    pub decode_counters: BTreeMap<&'static str, u64>,
    pub decode_error_count: u64,
    pub decode_errors: Vec<OverlayErrorRow>,
    pub stream_failures: Vec<String>,
    pub first_movement_error: Option<String>,
    pub first_struct_error: Option<String>,
    /// Sorted final GUID paths for this snapshot only; not an earlier-time map.
    pub net_guids: Vec<NetGuidRecord>,
}

/// Counts of completed optional snapshots. Main-stream summary fields exclude
/// these rows. Detailed counters are lent in each checkpoint's summary.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct CheckpointTotals {
    pub chunks: u64,
    pub trailing_bytes: u64,
    pub packets: u64,
    pub frames: u64,
    pub field_rows: u64,
    pub movement_rows: u64,
    pub actor_rows: u64,
    pub partial_rows: u64,
    pub block_rows: u64,
}

impl CheckpointTotals {
    pub(crate) fn absorb(&mut self, summary: &CheckpointSummary) {
        self.chunks += 1;
        self.trailing_bytes += summary.metadata.trailing_bytes;
        self.packets += u64::from(summary.packets);
        self.frames += u64::from(summary.frames);
        self.field_rows += summary.field_rows;
        self.movement_rows += summary.movement_rows;
        self.actor_rows += summary.actor_rows;
        self.partial_rows += summary.partial_rows;
        self.block_rows += summary.block_rows;
    }
}

pub(crate) enum CheckpointOutput<'a> {
    Metadata(&'a CheckpointMetadata),
    GuidEntry(CheckpointGuidEntryRecord),
    ExportGroup(CheckpointExportGroupRecord),
    ExportField(CheckpointExportFieldRecord),
    Records(&'a CheckpointMetadata, &'a mut RecordBuffers),
}

#[derive(Default)]
pub(crate) struct CheckpointRows {
    pub fields: u64,
    pub movement: u64,
    pub actors: u64,
    pub partials: u64,
    pub blocks: u64,
}

pub(crate) struct DecodedCheckpoint<'a> {
    pub metadata: CheckpointMetadata,
    pub tables: CheckpointTables,
    pub pass: Pass<'a>,
    pub rows: CheckpointRows,
}

impl DecodedCheckpoint<'_> {
    pub(crate) fn into_summary(self, stats: ExportStats) -> CheckpointSummary {
        CheckpointSummary {
            metadata: self.metadata,
            tables: self.tables,
            packets: self.pass.packets,
            frames: self.pass.frames,
            field_rows: self.rows.fields,
            movement_rows: self.rows.movement,
            actor_rows: self.rows.actors,
            partial_rows: self.rows.partials,
            block_rows: self.rows.blocks,
            frame_skips: self.pass.frame_skips,
            non_finite_frame_times: self.pass.non_finite_frame_times,
            network: self.pass.reader.stats().clone(),
            decode_counters: stats.counters().into_iter().collect(),
            decode_error_count: stats.overlay.error_report.total_errors(),
            decode_errors: stats.overlay.error_report.top_n(32),
            stream_failures: self.pass.channels.stream_failures().to_vec(),
            first_movement_error: stats.movement_first_error,
            first_struct_error: stats.struct_blob_first_error,
            net_guids: net_guid_rows(&self.pass.cache),
        }
    }
}

struct Declarations<'a, F> {
    identity: CheckpointIdentity,
    output: &'a mut F,
    guid_rows: u64,
    group_rows: u64,
    field_rows: u64,
}

impl<F: FnMut(CheckpointOutput<'_>) -> Result<(), CliError>> CheckpointTableSink
    for Declarations<'_, F>
{
    type Error = CliError;

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
        (self.output)(CheckpointOutput::GuidEntry(CheckpointGuidEntryRecord {
            checkpoint: self.identity.clone(),
            ordinal,
            net_guid: guid,
            outer_net_guid: outer,
            path_is_string,
            literal_path: literal_path.map(Into::into),
            name_index,
            flags,
        }))?;
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
        (self.output)(CheckpointOutput::ExportGroup(CheckpointExportGroupRecord {
            checkpoint: self.identity.clone(),
            ordinal,
            path_name_index,
            group_path: group_path.into(),
            declared_slots,
        }))?;
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
        (self.output)(CheckpointOutput::ExportField(CheckpointExportFieldRecord {
            checkpoint: self.identity.clone(),
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
        }))?;
        self.field_rows += 1;
        Ok(())
    }
}

/// Shared framing, table ingestion, fresh stream state and packet traversal.
/// Adapters only route the borrowed/owned records; they cannot choose a
/// different checkpoint parser or reuse a previous snapshot's cache.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_checkpoint<'a>(
    payload: &[u8],
    replay: &Replay<'a>,
    index: u32,
    schema_limits: SchemaLimits,
    capture_movement_provenance: bool,
    field_row_offset: u64,
    stats: &mut ExportStats,
    mut output: impl FnMut(CheckpointOutput<'_>) -> Result<(), CliError>,
) -> Result<DecodedCheckpoint<'a>, CliError> {
    let cp = parse_checkpoint_chunk(payload)
        .map_err(|error| CliError::from(error).in_checkpoint(index, None))?;
    let (plain, unread) =
        decompress_checkpoint_with_trailing(cp.archive, replay.compressed, replay.encrypted)
            .map_err(|error| CliError::from(error).in_checkpoint(index, Some(&cp.id)))?;
    let metadata = CheckpointMetadata {
        identity: CheckpointIdentity::new(index, cp.id),
        group: cp.group,
        metadata: cp.metadata,
        time1: cp.time1,
        time2: cp.time2,
        archive_size_bytes: cp.size_in_bytes as u64,
        trailing_bytes: (cp.trailing_bytes + unread) as u64,
    };
    let mut pass = Pass::with_schema_limits(replay, schema_limits)?;
    pass.buffers.capture_movement_provenance = capture_movement_provenance;
    output(CheckpointOutput::Metadata(&metadata))?;
    let mut declarations = Declarations {
        identity: metadata.identity.clone(),
        output: &mut output,
        guid_rows: 0,
        group_rows: 0,
        field_rows: 0,
    };
    let tables = read_checkpoint_tables_with_sink(&plain, &mut pass.cache, &mut declarations)
        .map_err(|error| match error {
            CheckpointReadError::Schema(error) => CliError::CheckpointSchema {
                checkpoint: metadata.identity.checkpoint_id.to_string(),
                error,
            },
            CheckpointReadError::Sink(error) => error,
        })?;
    if declarations.guid_rows != u64::from(tables.guid_count)
        || declarations.group_rows != u64::from(tables.group_count)
        || declarations.field_rows != u64::from(tables.exported_fields)
    {
        return Err(CliError::Usage(format!(
            "checkpoint {} declaration row counts disagree with parsed tables",
            metadata.identity.checkpoint_id
        )));
    }
    pass.block_scope = Some((metadata.identity.clone(), field_row_offset, 0));
    let mut rows = CheckpointRows::default();
    let mut emit = |buffers: &mut RecordBuffers| {
        for partial in &mut buffers.partials {
            partial.source = "checkpoint";
            partial.checkpoint_id = Some(metadata.identity.checkpoint_id.to_string());
        }
        rows.fields += buffers.fields.len() as u64;
        rows.movement += buffers.movement.len() as u64;
        rows.actors += buffers.actors.len() as u64;
        rows.partials += buffers.partials.len() as u64;
        rows.blocks += buffers.checkpoint_blocks.len() as u64;
        output(CheckpointOutput::Records(&metadata, buffers))
    };
    pass.walk(&plain[tables.frame_offset..], stats, &mut emit)
        .map_err(|error| error.in_checkpoint(index, Some(&metadata.identity.checkpoint_id)))?;
    pass.finish();
    emit(&mut pass.buffers)?;
    Ok(DecodedCheckpoint {
        metadata,
        tables,
        pass,
        rows,
    })
}
