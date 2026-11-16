//! Library entry point over the CLI's shared main-stream decode pass.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;
use vrf_container::{EventChunk, Preamble, parse_event_chunk, parse_preamble};
use vrf_decode::OverlayErrorRow;
use vrf_export::{ActorRecord, FieldRecord, MovementRecord, NetGuidRecord, PartialRecord};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::SchemaLimits;

use crate::error::CliError;
use crate::oracle::{Verdict, verdict_from_stats};
use crate::pass::{Chunk, Pass, Replay, for_each_chunk};
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
    /// Checkpoint chunks encountered but not decoded by this API. Their
    /// snapshots are independent streams; use `export --checkpoints` to audit
    /// them. Zero means none encountered, not that checkpoints were validated.
    pub skipped_checkpoint_chunks: u64,
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
/// shared. Decoded output is streamed; input is still a complete byte slice.
/// [`Self::read_path`] buffers the entire input file, just as the CLI does.
/// At most one additional decompressed ReplayData chunk is prefetched.
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
        mut consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        let mut consumer_error = None;
        let result = read_main_stream(data, schema_limits, |event| {
            consumer(event).map_err(|error| {
                consumer_error = Some(error);
                CliError::Usage("replay consumer stopped".to_owned())
            })
        });
        match consumer_error {
            Some(error) => Err(ReadError::Consumer(error)),
            None => result.map_err(ReadError::Replay),
        }
    }

    /// Read a whole file into memory, then stream decoded output with
    /// [`Self::read`]. The input file is only opened for reading.
    pub fn read_path<E>(
        &self,
        path: impl AsRef<Path>,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        self.read_path_with_schema_limits(path, SchemaLimits::default(), consumer)
    }

    /// Buffer a whole file, then decode it with [`Self::read_with_schema_limits`].
    ///
    /// Schema limits apply after reading the input and do not bound its size.
    /// The file is only opened for reading. See [`Self::read_path`] for scope.
    pub fn read_path_with_schema_limits<E>(
        &self,
        path: impl AsRef<Path>,
        schema_limits: SchemaLimits,
        consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), E>,
    ) -> Result<ReplaySummary, ReadError<E>> {
        let data = std::fs::read(path).map_err(CliError::from)?;
        self.read_with_schema_limits(&data, schema_limits, consumer)
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
    data: &[u8],
    schema_limits: SchemaLimits,
    mut consumer: impl FnMut(ReplayEvent<'_>) -> Result<(), CliError>,
) -> Result<ReplaySummary, CliError> {
    let metadata = parse_preamble(data)?;
    let replay = Replay::new(&metadata);
    let mut pass = Pass::with_schema_limits(&replay, schema_limits)?;
    consumer(ReplayEvent::Metadata(&metadata))?;

    let mut stats = ExportStats::default();
    let mut counts = RowCounts::default();
    let mut replay_data_chunks = 0;
    let mut skipped_checkpoint_chunks = 0;
    let mut replay_data_trailing_bytes = 0;
    let mut event_chunks = 0;
    let mut event_trailing_bytes = 0;
    let unknown_chunks = for_each_chunk(data, &replay, |chunk| {
        match chunk {
            Chunk::ReplayData(frames, unread) => {
                pass.walk(&frames, &mut stats, |buffers| {
                    counts.emit(buffers, &mut consumer)
                })?;
                replay_data_chunks += 1;
                replay_data_trailing_bytes += unread as u64;
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
    let mut entries = pass.cache.net_guid_entries();
    entries.sort_unstable_by_key(|entry| entry.net_guid);
    let net_guids = entries
        .into_iter()
        .map(|entry| NetGuidRecord {
            net_guid: entry.net_guid,
            path: entry.path.to_owned(),
            outer_net_guid: entry.outer_net_guid,
        })
        .collect();

    let summary = ReplaySummary {
        packets: pass.packets,
        frames: pass.frames,
        replay_data_chunks,
        skipped_checkpoint_chunks,
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
