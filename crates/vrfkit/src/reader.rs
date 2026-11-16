//! Library entry point over the CLI's shared main and checkpoint decode passes.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;
use vrf_container::{EventChunk, Preamble, parse_event_chunk, parse_preamble};
use vrf_decode::OverlayErrorRow;
use vrf_export::{
    ActorRecord, FieldRecord, MovementProvenance, MovementRecord, NetGuidRecord, PartialRecord,
};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::SchemaLimits;

use crate::checkpoints::{CheckpointEvent, CheckpointOutput, CheckpointTotals, decode_checkpoint};
use crate::error::CliError;
use crate::oracle::{Verdict, verdict_from_stats};
use crate::pass::{Chunk, Pass, Replay, for_each_chunk, net_guid_rows};
use crate::reader_input::FileInput;
use crate::sink::{ExportStats, RecordBuffers};

/// A parser/I/O failure or the original error returned by a consumer callback.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReadError<E> {
    /// The replay could not be read under the parser's structural rules.
    #[error(transparent)]
    Replay(#[from] CliError),
    /// A callback stopped the read. No later callback is invoked.
    #[error("replay consumer stopped: {0}")]
    Consumer(E),
}

/// Optional work and independent schema limits for one read.
///
/// Defaults preserve the main-only callback contract. Each enabled checkpoint
/// receives a fresh cache with these same limits, separate from the main cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOptions {
    pub schema_limits: SchemaLimits,
    pub checkpoints: bool,
}

/// Records from one packet, or rejected partials flushed at end of input.
///
/// These are the same rows the CLI sends to its main-stream writers. Typed
/// values remain additive to raw windows. Successful movement decodes do not
/// necessarily emit a duplicate raw RPC. Slices are valid only during the
/// callback; cloning a row explicitly transfers its data to the consumer.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ReplayRecords<'a> {
    pub fields: &'a [FieldRecord],
    pub movement: &'a [MovementRecord],
    /// One source context for each movement row, in exactly the same order.
    /// Captured by every Reader call; ordinary CLI Parquet output is unchanged.
    /// Like the rows, these identities remain provisional until success.
    pub movement_provenance: &'a [MovementProvenance],
    pub actors: &'a [ActorRecord],
    pub partials: &'a [PartialRecord],
}

impl ReplayRecords<'_> {
    /// Whether this batch contains no emitted rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
            && self.movement.is_empty()
            && self.actors.is_empty()
            && self.partials.is_empty()
    }
}

/// One synchronous notification in file order, not necessarily timestamp order.
///
/// All callbacks are provisional until the read returns successfully. A later
/// parse error cannot roll back rows the consumer already saved. Stage external
/// output if a failed read must leave no partial result.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReplayEvent<'a> {
    /// Parsed metadata, emitted once before any records on a supported branch.
    Metadata(&'a Preamble),
    /// Main-stream rows from a packet, or final rejected partial fragments.
    Records(ReplayRecords<'a>),
    /// A native Event chunk, including its exact borrowed payload. This is not
    /// an inferred gameplay event. `vrf_container` provides measured payload
    /// decoders for known groups.
    ContainerEvent(EventChunk<'a>),
    /// An independent snapshot; emitted only when explicitly requested with
    /// [`ReadOptions::checkpoints`]. Never interpreted as a main-stream record.
    Checkpoint(CheckpointEvent<'a>),
}

/// Detached results after structural traversal and the final callback succeed.
///
/// A returned value is not a guarantee of complete semantic coverage or no
/// transport loss. Inspect [`Self::network`], [`Self::decode_counters`], frame
/// skips and the explicit checkpoint scope. The block verdict uses the same
/// limited acceptance rule as `vrfkit validate`.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReplaySummary {
    pub metadata: Preamble,
    pub packets: u32,
    pub frames: u32,
    pub replay_data_chunks: u64,
    /// Checkpoint chunks encountered while checkpoint callbacks were disabled.
    /// Zero alone does not imply snapshots were present or validated.
    pub skipped_checkpoint_chunks: u64,
    /// Optional snapshot counts, separate from every main-stream count below.
    pub checkpoints: CheckpointTotals,
    pub unknown_chunks: u64,
    pub replay_data_trailing_bytes: u64,
    pub event_chunks: u64,
    pub event_trailing_bytes: u64,
    pub field_rows: u64,
    pub movement_rows: u64,
    pub actor_rows: u64,
    pub partial_rows: u64,
    pub frame_skips: FrameSkips,
    pub non_finite_frame_times: u64,
    /// Complete replication counters and the parser's capped diagnostic list.
    pub network: NetStats,
    /// Every sink counter by its existing published name, including zeros.
    /// Future counters may be added; absence must not be interpreted as zero.
    pub decode_counters: BTreeMap<&'static str, u64>,
    /// Total overlay failures, including failures outside the retained list.
    pub decode_error_count: u64,
    /// Up to 32 most frequent overlay failure identities.
    pub decode_errors: Vec<OverlayErrorRow>,
    /// Up to 32 recorded stream-failure descriptions, as in the CLI.
    pub stream_failures: Vec<String>,
    pub first_movement_error: Option<String>,
    pub first_struct_error: Option<String>,
    /// Final resolved GUID paths, sorted by GUID. A path can become known
    /// after the packet that first referenced it; this is a final snapshot,
    /// not a claim about what was known at each earlier timestamp.
    pub net_guids: Vec<NetGuidRecord>,
}

impl ReplaySummary {
    /// The CLI's ReplayData block verdict. Checkpoints, partial reassembly
    /// rejections and type-overlay correctness are outside this verdict.
    #[must_use]
    pub fn block_verdict(&self) -> Verdict {
        verdict_from_stats(&self.network, self.replay_data_trailing_bytes)
    }
}

/// Stateless reader using the same ordered decode pipeline as the CLI.
///
/// Each call creates independent replay state, so the reader can be reused or
/// shared. [`Self::read`] borrows a complete byte slice; [`Self::read_path`]
/// reads one encoded chunk at a time from a file. At most one additional
/// decompressed ReplayData chunk is prefetched.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplayReader;

impl ReplayReader {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Read metadata without decoding packet payloads or copying `data`.
    ///
    /// Like `inspect`, this can describe a branch the full parser does not
    /// support. [`Self::read`] rejects an unsupported branch before callbacks.
    pub fn read_metadata(&self, data: &[u8]) -> Result<Preamble, CliError> {
        Ok(parse_preamble(data)?)
    }

    /// Decode the main stream and native Event chunks, lending output batches
    /// to `consumer`. No output files or terminal messages are produced.
    ///
    /// Consumer errors retain their original type and outrank a later error
    /// discovered while the current frame chunk unwinds. No further callbacks
    /// occur after the first consumer error. Panics are not intercepted.
    pub fn read<E>(
        &self,
        data: &[u8],
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        self.read_with_schema_limits(data, SchemaLimits::default(), consumer)
    }

    /// Decode with explicit limits for this read's replay-wide schema cache.
    ///
    /// The limits cover registered groups and declared field slots, including
    /// empty slots. They do not bound total memory, input or decompressed
    /// buffers, strings, GUID mappings or output retained by the consumer.
    /// Zero is allowed: zero groups refuses every declaration; zero slots
    /// allows only empty groups. Raising a limit increases the allocation an
    /// input can request. Checkpoints are still skipped.
    ///
    /// A refused declaration returns [`ReadError::Replay`]. Its error source
    /// chain retains [`crate::SchemaError::ResourceLimit`] with the requested
    /// amount and limit; no success summary is returned. Earlier callbacks
    /// remain provisional, and consumer errors keep [`Self::read`]'s priority.
    /// Limits and parser state never carry over to a later read.
    pub fn read_with_schema_limits<E>(
        &self,
        data: &[u8],
        schema_limits: SchemaLimits,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        self.read_with_options(
            data,
            ReadOptions {
                schema_limits,
                checkpoints: false,
            },
            consumer,
        )
    }

    /// Decode in file order, optionally delivering independent checkpoints.
    ///
    /// The first structural failure encountered in file order stops delivery;
    /// a later prefetched main-stream error cannot replace an earlier snapshot
    /// failure. Consumer errors retain their original value and take precedence
    /// over errors encountered while the current frame walk unwinds. No final
    /// replay summary is returned on failure, even after a snapshot summary.
    pub fn read_with_options<E>(
        &self,
        data: &[u8],
        options: ReadOptions,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        let metadata = parse_preamble(data).map_err(CliError::from)?;
        read_input(metadata, options, consumer, |replay, on_chunk| {
            for_each_chunk(data, replay, on_chunk)
        })
    }

    /// Read a file in chunks and stream decoded output with the same decoder
    /// as [`Self::read`]. The input file is only opened for reading and must
    /// remain unchanged until this call returns. It is not snapshotted.
    ///
    /// Memory depends on the largest encoded chunk, the current and one
    /// prefetched decompressed chunk, metadata and parser/consumer state;
    /// it does not retain a copy of the complete file. Checkpoint and unknown
    /// chunk payloads are range-checked and skipped without buffering.
    pub fn read_path<E>(
        &self,
        path: impl AsRef<Path>,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        self.read_path_with_schema_limits(path, SchemaLimits::default(), consumer)
    }

    /// Stream a file with explicit schema limits, as in
    /// [`Self::read_with_schema_limits`].
    ///
    /// Schema limits do not bound encoded or decompressed chunk sizes.
    /// The file is only opened for reading. See [`Self::read_path`] for scope.
    pub fn read_path_with_schema_limits<E>(
        &self,
        path: impl AsRef<Path>,
        schema_limits: SchemaLimits,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        self.read_path_with_options(
            path,
            ReadOptions {
                schema_limits,
                checkpoints: false,
            },
            consumer,
        )
    }

    /// File-backed equivalent of [`Self::read_with_options`]. Enabled snapshot
    /// bodies use one reusable encoded buffer and one decoded snapshot; skipped
    /// checkpoint bodies still require no buffering. The input must stay stable.
    pub fn read_path_with_options<E>(
        &self,
        path: impl AsRef<Path>,
        options: ReadOptions,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        let (metadata, input) = FileInput::open(path.as_ref())?;
        read_input(metadata, options, consumer, |replay, on_chunk| {
            input.for_each_chunk_with_checkpoints(replay, options.checkpoints, on_chunk)
        })
    }
}

fn read_input<E>(
    metadata: Preamble,
    options: ReadOptions,
    mut consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    chunks: impl FnOnce(
        &Replay<'_>,
        &mut dyn FnMut(Chunk<'_>) -> Result<(), CliError>,
    ) -> Result<u64, CliError>,
) -> Result<ReplaySummary, ReadError<E>> {
    let mut consumer_error = None;
    let result = read_main_stream(
        metadata,
        options,
        |event| {
            consumer(event).map_err(|error| {
                consumer_error = Some(error);
                CliError::Usage("replay consumer stopped".to_owned())
            })
        },
        chunks,
    );
    match consumer_error {
        Some(error) => Err(ReadError::Consumer(error)),
        None => result.map_err(ReadError::Replay),
    }
}

#[derive(Default)]
struct RowCounts {
    fields: u64,
    movement: u64,
    actors: u64,
    partials: u64,
}

impl RowCounts {
    fn emit(
        &mut self,
        buffers: &mut RecordBuffers,
        consumer: &mut impl FnMut(ReplayEvent<'_>) -> Result<(), CliError>,
    ) -> Result<(), CliError> {
        // The sink leaves the pass label unset; the CLI writer sets it here
        // too. An end-of-stream partial must keep its original packet id.
        for partial in &mut buffers.partials {
            partial.source = "main";
        }
        let batch = ReplayRecords {
            fields: &buffers.fields,
            movement: &buffers.movement,
            movement_provenance: &buffers.movement_provenance,
            actors: &buffers.actors,
            partials: &buffers.partials,
        };
        if !batch.is_empty() {
            consumer(ReplayEvent::Records(batch))?;
            self.fields += batch.fields.len() as u64;
            self.movement += batch.movement.len() as u64;
            self.actors += batch.actors.len() as u64;
            self.partials += batch.partials.len() as u64;
        }
        Ok(())
    }
}

fn read_main_stream(
    metadata: Preamble,
    options: ReadOptions,
    mut consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), CliError>,
    chunks: impl FnOnce(
        &Replay<'_>,
        &mut dyn FnMut(Chunk<'_>) -> Result<(), CliError>,
    ) -> Result<u64, CliError>,
) -> Result<ReplaySummary, CliError> {
    let replay = Replay::new(&metadata);
    let mut pass = Pass::with_schema_limits(&replay, options.schema_limits)?;
    pass.buffers.capture_movement_provenance = true;
    consumer(ReplayEvent::Metadata(&metadata))?;

    let mut stats = ExportStats::default();
    let mut counts = RowCounts::default();
    let mut replay_data_chunks = 0;
    let mut skipped_checkpoint_chunks = 0;
    let mut checkpoints = CheckpointTotals::default();
    let mut replay_data_trailing_bytes = 0;
    let mut event_chunks = 0;
    let mut event_trailing_bytes = 0;
    let unknown_chunks = chunks(&replay, &mut |chunk| {
        match chunk {
            Chunk::ReplayData(frames, unread) => {
                pass.walk(&frames, &mut stats, |buffers| {
                    counts.emit(buffers, &mut consumer)
                })?;
                replay_data_chunks += 1;
                replay_data_trailing_bytes += unread as u64;
            }
            Chunk::Checkpoint(payload) if options.checkpoints => {
                let index = u32::try_from(checkpoints.chunks).map_err(|_| {
                    CliError::Usage("too many checkpoint chunks to index".to_owned())
                })?;
                let mut checkpoint_stats = ExportStats::default();
                let decoded = decode_checkpoint(
                    payload,
                    &replay,
                    index,
                    options.schema_limits,
                    true,
                    checkpoints.field_rows,
                    &mut checkpoint_stats,
                    |output| {
                        let event = match output {
                            CheckpointOutput::Metadata(metadata) => {
                                CheckpointEvent::Metadata(metadata)
                            }
                            CheckpointOutput::GuidEntry(row) => {
                                return consumer(ReplayEvent::Checkpoint(
                                    CheckpointEvent::GuidEntry(&row),
                                ));
                            }
                            CheckpointOutput::ExportGroup(row) => {
                                return consumer(ReplayEvent::Checkpoint(
                                    CheckpointEvent::ExportGroup(&row),
                                ));
                            }
                            CheckpointOutput::ExportField(row) => {
                                return consumer(ReplayEvent::Checkpoint(
                                    CheckpointEvent::ExportField(&row),
                                ));
                            }
                            CheckpointOutput::Records(checkpoint, buffers) => {
                                let records = ReplayRecords {
                                    fields: &buffers.fields,
                                    movement: &buffers.movement,
                                    movement_provenance: &buffers.movement_provenance,
                                    actors: &buffers.actors,
                                    partials: &buffers.partials,
                                };
                                if records.is_empty() && buffers.checkpoint_blocks.is_empty() {
                                    return Ok(());
                                }
                                CheckpointEvent::Records {
                                    checkpoint,
                                    records,
                                    blocks: &buffers.checkpoint_blocks,
                                }
                            }
                        };
                        consumer(ReplayEvent::Checkpoint(event))
                    },
                )?;
                let summary = decoded.into_summary(checkpoint_stats);
                consumer(ReplayEvent::Checkpoint(CheckpointEvent::Summary(&summary)))?;
                checkpoints.absorb(&summary);
            }
            Chunk::Checkpoint(_) => skipped_checkpoint_chunks += 1,
            Chunk::Event(payload) => {
                let event = parse_event_chunk(payload)?;
                event_trailing_bytes += event.trailing_bytes as u64;
                consumer(ReplayEvent::ContainerEvent(event))?;
                event_chunks += 1;
            }
        }
        Ok(())
    })?;

    pass.finish();
    counts.emit(&mut pass.buffers, &mut consumer)?;
    let net_guids = net_guid_rows(&pass.cache);

    let summary = ReplaySummary {
        packets: pass.packets,
        frames: pass.frames,
        replay_data_chunks,
        skipped_checkpoint_chunks,
        checkpoints,
        unknown_chunks,
        replay_data_trailing_bytes,
        event_chunks,
        event_trailing_bytes,
        field_rows: counts.fields,
        movement_rows: counts.movement,
        actor_rows: counts.actors,
        partial_rows: counts.partials,
        frame_skips: pass.frame_skips,
        non_finite_frame_times: pass.non_finite_frame_times,
        network: pass.reader.stats().clone(),
        decode_counters: stats.counters().into_iter().collect(),
        decode_error_count: stats.overlay.error_report.total_errors(),
        decode_errors: stats.overlay.error_report.top_n(32),
        stream_failures: pass.channels.stream_failures().to_vec(),
        first_movement_error: stats.movement_first_error,
        first_struct_error: stats.struct_blob_first_error,
        net_guids,
        metadata,
    };
    Ok(summary)
}
