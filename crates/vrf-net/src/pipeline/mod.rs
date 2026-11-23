//! Top-level replication reader that drives the full pipeline.
//!
//! One pass connects packets -> bunches -> content blocks -> fields. The
//! caller provides a [`ReplicationSink`] for the decoded events (fields, RPCs,
//! actor lifecycle) and the replay branch that selects the payload transform.
//!
//! Stages: channel (open/close, GUID preambles), spawn (dynamic-actor spawn
//! block), framing (content blocks, fields, RPCs); this file holds the sink
//! trait, partial routing and the header stages. Measured rates:
//! docs/PERFORMANCE_NOTES.md#measured-rates-reference-replay-02d4d478.
//!
//! The steady state allocates nothing per packet, bunch or content block
//! (docs/PERFORMANCE_NOTES.md#allocation-strategy): `scratch`,
//! `fragment_stage` and the channel table (a row per channel index, never per
//! bunch) are reused, and bunch payloads are sub-readers of the caller's
//! packet bytes.

mod channel;
mod framing;
mod spawn;

use vrf_bitio::BitReader;
use vrf_transform::TransformVersion;

use crate::bunch::{
    PartialBunchAccumulator, PartialDiscardCause, PartialResourceLimit, PreservedPartial,
    RawBunchHeader,
};
use crate::content::ContentBlockHeader;
use crate::error::{PartialSequenceKind, Result};
use crate::field::FieldSink;
use crate::net_guid::GuidPathSink;
use crate::packet::RawPacketReader;
use crate::stats::NetStats;
use crate::types::MAX_ACTIVE_CHANNELS;
use crate::types::NetworkGuid;

use framing::BunchContext;

/// Per-channel actor state tracked during replication.
#[derive(Debug, Clone, Default)]
pub struct ActorChannelState {
    pub channel_index: u32,
    pub is_open: bool,
    /// Set on a dormant close; sinks learn dormancy from `on_actor_close`'s
    /// `dormant`.
    pub is_dormant: bool,
    pub actor_net_guid: NetworkGuid,
    /// Archetype GUID (for dynamic actors).
    pub archetype_net_guid: NetworkGuid,
    pub level_guid: NetworkGuid,
    /// Spawn location (if dynamic and present).
    pub spawn_location: Option<crate::types::FVector>,
    /// Spawn rotation (if present).
    pub spawn_rotation: Option<crate::types::FRotator>,
    /// Spawn scale (if dynamic and present).
    pub spawn_scale: Option<crate::types::FVector>,
    /// Spawn velocity (if dynamic and present).
    pub spawn_velocity: Option<crate::types::FVector>,
    /// Packet that opened this channel.
    pub open_packet_id: i32,
}

/// Which stream grammar failed to parse inside a decoded content block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StreamKind {
    /// A property (RepLayout) stream.
    RepLayout,
    /// An RPC (ClassNetCache) stream.
    Rpc,
}

/// Where inside the walk a stream failure happened. Diagnostic only: it
/// separates, per group, an unresolved group (payload preserved whole) from a
/// stream that lost structure, which `NetStats::lost_content_blocks` does
/// only in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StreamFailureCause {
    /// The walk returned `Ok` but abandoned bits mid-block (a record overran,
    /// or an RPC record was too short for its length); records before the
    /// break are good.
    AbandonedTail,
    /// The walk returned `Err` -- a read inside the block ran off the end (or
    /// otherwise failed) and the rest of the block is unframeable.
    ReadError,
    /// The ClassNetCache function count was 0 (an unresolved group, handle
    /// width unknown). The whole decoded payload went to
    /// `on_unresolved_class_net_cache_payload`: preserved, not lost.
    UnresolvedFunctionCount,
    /// A valid post-RepLayout window retained whole because its provenance or
    /// strict ClassNetCache shape was not verified: uncertainty, not a failed
    /// read or a missing count.
    UnverifiedRepLayoutTail,
}

/// Result of handing a valid post-RepLayout tail to the sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepLayoutTailOutcome {
    /// The tail was strictly decoded as this many ClassNetCache RPCs.
    Decoded { rpc_count: u32 },
    /// The tail could not be decoded, but its complete raw bits were retained.
    Preserved { cause: StreamFailureCause },
    /// The tail could neither be decoded nor retained by this sink.
    Unpreserved { cause: StreamFailureCause },
}

/// Context for a content block that framed and decoded but whose inner stream
/// could not be walked. Reported to the sink because it holds the names (group
/// path, function count) a new build's investigation needs; a counter only
/// says that one block failed.
#[derive(Debug, Clone, Copy)]
pub struct StreamFailure {
    /// Which grammar was being parsed.
    pub kind: StreamKind,
    /// Actor whose channel carried the block.
    pub actor_net_guid: NetworkGuid,
    /// Declared payload length of the block.
    pub bit_count: u32,
    /// Function count used for the handle read (`Rpc` only; 0 for `RepLayout`,
    /// and the unresolved-group sentinel). A wrong non-zero count selects the
    /// wrong handle width.
    pub function_count: u32,
    /// Bits consumed before the failure.
    pub consumed_bits: u64,
    /// Bits abandoned as a result.
    pub remaining_bits: u64,
    /// Which stage of the walk failed. See [`StreamFailureCause`].
    pub cause: StreamFailureCause,
    /// Handle of the non-terminator record the walk was inside, once its
    /// handle read succeeded; `None` for a failed handle read or an early zero
    /// terminator, so the previous field is never blamed.
    pub record_handle: Option<u32>,
    /// Bit offset inside the block where the failing record begins, when the
    /// parser tracked one.
    pub record_offset: Option<u64>,
    /// Whether the complete failed stream reached a raw preservation row.
    pub payload_preserved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialPayloadReason {
    MissingInitial,
    OverlappingInitial,
    MismatchedContinuation,
    NonByteAlignedFragment,
    PayloadOverrun,
    ActiveStateLimit,
    BufferedBitsLimit,
    AllocationFailure,
    ChannelStateLimit,
    ChannelClosed,
    EndOfStream,
}

pub struct RejectedPartialFragment<'a> {
    pub header: &'a RawBunchHeader,
    pub payload_kind: &'static str,
    pub reason: PartialPayloadReason,
    pub bit_count: usize,
    pub payload: &'a [u8],
    pub rejection_packet_id: Option<i32>,
}

impl<'a> RejectedPartialFragment<'a> {
    /// An assembly the accumulator gave up, with the fragments it had buffered.
    fn accumulated(
        partial: &'a PreservedPartial,
        reason: PartialPayloadReason,
        rejection_packet_id: Option<i32>,
    ) -> Self {
        Self {
            header: &partial.header,
            payload_kind: "accumulated_payload",
            reason,
            bit_count: partial.bit_count,
            payload: &partial.buffer,
            rejection_packet_id,
        }
    }

    /// The fragment being handled, refused before it was buffered.
    fn current(
        header: &'a RawBunchHeader,
        reason: PartialPayloadReason,
        bit_count: usize,
        payload: &'a [u8],
    ) -> Self {
        Self {
            header,
            payload_kind: "current_fragment",
            reason,
            bit_count,
            payload,
            rejection_packet_id: Some(header.packet_id),
        }
    }
}

/// Transport identity of a successfully framed content block.
///
/// The initial packet comes from the authoritative reassembly header; it is
/// not reconstructed from packet distances. The completion identity belongs
/// to the bunch that made this block available for decoding. For an ordinary
/// non-partial bunch the two packet IDs are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentBlockContext {
    pub initial_packet_id: i32,
    pub completion_packet_id: i32,
    /// Zero-based global index of the completing bunch, including fragments.
    pub bunch_index: u64,
    /// Zero-based block index within the whole, possibly reassembled bunch.
    pub block_index: u32,
}

/// Trait for receiving all replication events: fields, RPCs and actor
/// lifecycle, with nothing discarded silently.
pub trait ReplicationSink: GuidPathSink + FieldSink {
    fn on_rejected_partial(&mut self, _partial: RejectedPartialFragment<'_>) {}
    /// Whether the sink wants per-record failure positions and decoded-payload
    /// callbacks; off by default.
    fn wants_stream_failure_details(&self) -> bool {
        false
    }

    /// Handle a ClassNetCache stream that follows a valid RepLayout zero
    /// terminator in the same content block. The reader is bounded to the tail.
    fn on_rep_layout_tail(
        &mut self,
        _actor_net_guid: NetworkGuid,
        _bit_count: u32,
        _reader: BitReader<'_>,
    ) -> RepLayoutTailOutcome {
        RepLayoutTailOutcome::Unpreserved {
            cause: StreamFailureCause::UnverifiedRepLayoutTail,
        }
    }

    /// Optional raw diagnostic sample for a chained tail that was not decoded,
    /// after its [`Self::on_stream_failure`].
    fn on_rep_layout_tail_failure_payload(
        &mut self,
        _failure: StreamFailure,
        _reader: BitReader<'_>,
    ) {
    }

    /// An actor channel was opened (new actor spawned or re-opened).
    fn on_actor_open(&mut self, state: &ActorChannelState);

    /// An actor channel was closed.
    fn on_actor_close(&mut self, channel_index: u32, actor_net_guid: NetworkGuid, dormant: bool);

    /// A content block header was parsed (before payload).
    /// Returns the function_count for ClassNetCache blocks (0 if unknown).
    fn on_content_block(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
    ) -> u32;

    /// A framed block with transport provenance. The default delegates to
    /// [`Self::on_content_block`], preserving existing consumers. Called once
    /// per non-deleted block, before any of its field or RPC callbacks.
    fn on_content_block_with_context(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
        _context: &ContentBlockContext,
    ) -> u32 {
        self.on_content_block(channel_index, actor_net_guid, header)
    }

    /// A content block was flagged as deleted.
    fn on_deleted_block(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
    );

    /// A block framed and decoded, but its inner stream could not be walked.
    /// Default no-op (the [`NetStats`] counters move either way); override to
    /// attach names the sink holds, the resolved group path in particular.
    /// The three payload callbacks for the same failure always come after it.
    fn on_stream_failure(&mut self, _failure: StreamFailure) {}

    /// The decoded payload of a block whose inner stream could not be walked,
    /// after [`Self::on_stream_failure`], only when
    /// [`Self::wants_stream_failure_details`], and not for unresolved
    /// ClassNetCache blocks ([`Self::on_unresolved_class_net_cache_payload`]).
    /// Diagnostics only; default no-op.
    fn on_stream_failure_payload(&mut self, _failure: StreamFailure, _payload: &[u8]) {}

    /// Preserve one whole decoded ClassNetCache payload whose function table
    /// could not be resolved, after its [`Self::on_stream_failure`]: exactly
    /// `ceil(failure.bit_count / 8)` bytes, unused high bits cleared.
    fn on_unresolved_class_net_cache_payload(&mut self, _failure: StreamFailure, _payload: &[u8]) {}
}

/// Leaf of the replay controller from 12.07 (12.01-12.06: BaseJanusController);
/// see `channel::is_player_controller_path`.
pub const PLAYER_CONTROLLER_LEAF: &str = "BaseReplayController";

/// One channel's row. A channel is counted from its first bunch, which may be
/// a close or a bunch for a channel that never opened, while only an open
/// produces an [`ActorChannelState`]: hence the `Option`.
#[derive(Default)]
struct ChannelSlot {
    /// How many bunches this channel has carried, 1-based. Reported in
    /// diagnostics as `channel_bunch_index`.
    bunch_count: u64,
    /// `None` until the channel opens, and again once an open bunch that did
    /// not complete its open has retired the live actor it held
    /// (`retire_after_failed_open`).
    state: Option<ActorChannelState>,
}

/// Channel index -> channel row, looked up once or twice per bunch.
type ChannelTable = vrf_schema::FxHashMap<u32, ChannelSlot>;

/// Bytes the scratch buffer starts at: above any block one bunch can carry
/// (`MAX_PACKET_SIZE_BITS`, 2,048 bytes). A reassembled partial's larger
/// block is `resize`d.
const SCRATCH_INITIAL_BYTES: usize = 4096;

/// The mutable state one bunch's processing needs: a borrow split of
/// [`ReplicationReader`], made once per packet so the bunch callback can drive
/// the pipeline inline.
struct Stage<'a> {
    stats: &'a mut NetStats,
    channels: &'a mut ChannelTable,
    transform: TransformVersion,
    scratch: &'a mut Vec<u8>,
}

/// Where a bunch sits in the stream, before any partial-header replacement.
#[derive(Clone, Copy)]
struct BunchIds {
    packet_id: i32,
    bunch_index_in_packet: u32,
    global_bunch_index: u64,
    channel_bunch_index: u64,
}

/// The main replication reader. Drives the full pipeline for one replay.
pub struct ReplicationReader {
    packet_reader: RawPacketReader,
    accumulator: PartialBunchAccumulator,
    channels: ChannelTable,
    transform: TransformVersion,
    stats: NetStats,
    /// Reusable scratch buffer for payload transforms.
    scratch: Vec<u8>,
    /// Reusable byte-aligned staging buffer for partial-bunch fragments; see
    /// `stage_fragment`.
    fragment_stage: Vec<u8>,
    /// Global bunch counter (0-based, monotonically increasing).
    global_bunch_index: u64,
}

impl ReplicationReader {
    /// Create a reader for a replay with the given branch.
    pub fn new(branch: &str) -> Result<Self> {
        let transform = TransformVersion::require(branch)?;
        Ok(Self {
            packet_reader: RawPacketReader::new(),
            accumulator: PartialBunchAccumulator::new(),
            channels: ChannelTable::default(),
            transform,
            stats: NetStats::default(),
            scratch: vec![0u8; SCRATCH_INITIAL_BYTES],
            fragment_stage: Vec::new(),
            global_bunch_index: 0,
        })
    }

    /// Access accumulated statistics.
    #[must_use]
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }

    /// Count the assemblies the replay ended in the middle of, and preserve
    /// them through `sink`. Call after the last packet (until then an
    /// unfinished partial looks in progress); they land in
    /// [`NetStats::unfinished_partials`], not `partial_errors`. Idempotent.
    pub fn finish_with_sink(&mut self, sink: &mut dyn ReplicationSink) {
        self.finish_partials(Some(sink));
    }

    /// Account for unfinished partials without a preservation consumer.
    pub fn finish(&mut self) {
        self.finish_partials(None);
    }

    /// Count every assembly still in flight and, given a sink, preserve it
    /// there as [`PartialPayloadReason::EndOfStream`].
    fn finish_partials(&mut self, mut sink: Option<&mut dyn ReplicationSink>) {
        for partial in self.accumulator.drain_unfinished() {
            self.stats.unfinished_partials += 1;
            self.stats.unfinished_partial_bits += partial.bit_count as u64;
            if let Some(sink) = sink.as_deref_mut() {
                let reason = PartialPayloadReason::EndOfStream;
                sink.on_rejected_partial(RejectedPartialFragment::accumulated(
                    &partial, reason, None,
                ));
            }
        }
    }

    /// Process one raw packet (byte slice as received from the demo frame).
    pub fn process_packet(
        &mut self,
        packet_data: &[u8],
        packet_id: i32,
        sink: &mut dyn ReplicationSink,
    ) {
        self.stats.packets += 1;

        // Bunches are processed inside the packet reader's callback
        // (docs/PERFORMANCE_NOTES.md#packet-processing-is-interleaved):
        // destructuring `self` splits the borrows, and header parsing mutates
        // only `packet_reader`, payload processing only the rest.
        let Self {
            packet_reader,
            accumulator,
            channels,
            transform,
            stats,
            scratch,
            fragment_stage,
            global_bunch_index,
        } = self;

        let mut stage = Stage {
            stats,
            channels,
            transform: *transform,
            scratch,
        };
        let mut bunch_index_in_packet: u32 = 0;

        let result = packet_reader.read_packet(packet_data, packet_id, |header, payload| {
            stage.stats.bunches += 1;

            // The global and in-packet indexes are the values *before* the
            // increment, the per-channel one the value *after*.
            let (global_index, index_in_packet) = (*global_bunch_index, bunch_index_in_packet);
            *global_bunch_index += 1;
            bunch_index_in_packet += 1;
            if header.has_channel_limit_error
                || (!stage.channels.contains_key(&header.ch_index)
                    && stage.channels.len() >= MAX_ACTIVE_CHANNELS)
            {
                stage.stats.channel_state_limit_failures += 1;
                if header.b_partial {
                    // Attempted, then refused: counted here because this
                    // return skips `process_bunch`, which counts the rest.
                    stage.stats.partial_bunches += 1;
                    let bit_count = payload.bits_remaining() as usize;
                    let staged = stage_fragment(payload.clone(), fragment_stage);
                    let reason = PartialPayloadReason::ChannelStateLimit;
                    let row = RejectedPartialFragment::current(header, reason, bit_count, staged);
                    sink.on_rejected_partial(row);
                }
                // Retired before the close, so an open+close bunch closes
                // nothing it did not open.
                Self::abandon_bunch(header, &mut payload.clone(), &mut stage);
                if header.b_close {
                    Self::close_channel(header, &mut stage, accumulator, sink);
                }
                return;
            }
            let slot = stage.channels.entry(header.ch_index).or_default();
            slot.bunch_count += 1;
            let ids = BunchIds {
                packet_id,
                bunch_index_in_packet: index_in_packet,
                global_bunch_index: global_index,
                channel_bunch_index: slot.bunch_count,
            };

            Self::process_bunch(
                header,
                payload,
                &mut stage,
                accumulator,
                fragment_stage,
                sink,
                ids,
            );
        });

        if result.is_malformed {
            stage.stats.malformed_packets += 1;
        }
        // `result.partial_error_count` is advisory; see `RawPacketReader`.
    }

    /// Route one bunch: reassemble it if partial, otherwise frame it directly.
    fn process_bunch(
        header: &mut RawBunchHeader,
        payload: BitReader<'_>,
        stage: &mut Stage<'_>,
        accumulator: &mut PartialBunchAccumulator,
        fragment_stage: &mut Vec<u8>,
        sink: &mut dyn ReplicationSink,
        ids: BunchIds,
    ) {
        let ch_index = header.ch_index;
        let bit_count = payload.bits_remaining();

        if header.b_partial {
            stage.stats.partial_bunches += 1;
            let staged = stage_fragment(payload, fragment_stage);
            // The accumulator's PayloadOverrun refusal needs a slice shorter
            // than the declared bits; staging never produces one.
            debug_assert_eq!(staged.len(), (bit_count as usize).div_ceil(8));

            // The accumulator is the one reassembly authority, so it sees none
            // of the packet reader's partial verdicts (see `RawPacketReader`):
            // the tracker can disagree with it, and its flag would veto valid
            // completions with every cause counter at 0.
            let mut fragment_header = header.clone();
            fragment_header.has_partial_error = false;
            fragment_header.partial_error_kind = None;
            fragment_header.is_partial_completed = false;

            let result = accumulator.add_fragment(
                ch_index,
                fragment_header,
                staged,
                bit_count as usize,
                &mut stage.stats.partial_errors,
                &mut stage.stats.partial_fragments,
                &mut stage.stats.partial_completed,
            );
            *header = result.header;

            let reason = result
                .error_kind
                .map(PartialDiscardCause::Sequence)
                .or_else(|| result.resource_limit.map(PartialDiscardCause::Resource))
                .map(partial_payload_reason);
            for (displaced, cause) in &result.displaced {
                let reason = partial_payload_reason(*cause);
                let row =
                    RejectedPartialFragment::accumulated(displaced, reason, Some(header.packet_id));
                sink.on_rejected_partial(row);
            }
            // A current-fragment row only for a fragment the accumulator
            // refused, under the cause it named (an overlapping initial is
            // buffered, not refused). No fallback cause: it would count bits twice.
            if let Some(reason) = reason.filter(|reason| {
                !result.should_process && *reason != PartialPayloadReason::OverlappingInitial
            }) {
                let row =
                    RejectedPartialFragment::current(header, reason, bit_count as usize, staged);
                sink.on_rejected_partial(row);
            }

            if result.overlapping_initial {
                stage.stats.partial_overlapping_initial += 1;
            }
            match result.error_kind {
                Some(PartialSequenceKind::MissingInitial) => {
                    stage.stats.partial_missing_initial += 1;
                    stage.stats.partial_missing_initial_bits += bit_count;
                    if header.b_partial_final {
                        stage.stats.partial_missing_initial_final += 1;
                    }
                    if header.b_reliable {
                        stage.stats.partial_missing_initial_reliable += 1;
                    }
                }
                Some(PartialSequenceKind::OverlappingInitial) => {}
                Some(PartialSequenceKind::MismatchedContinuation) => {
                    stage.stats.partial_mismatched_continuation += 1;
                }
                Some(PartialSequenceKind::NonByteAlignedFragment) => {
                    stage.stats.partial_non_byte_aligned += 1;
                }
                // Unreachable: `staged` is exactly `bit_count.div_ceil(8)`
                // bytes (asserted above). Counted, not a panic, so a staging
                // regression reads as this cause rather than a residual.
                Some(PartialSequenceKind::PayloadOverrun) => {
                    stage.stats.partial_payload_overrun += 1;
                }
                None => {}
            }

            stage.stats.skipped_bits += result.discarded_bits as u64;
            if result.resource_limit.is_some() {
                stage.stats.partial_resource_limit_failures += 1;
            }

            if result.should_process {
                if let Some((buf, total_bits, stored_header)) = accumulator.take_completed(ch_index)
                {
                    let Ok(mut payload_reader) = BitReader::with_bit_len(&buf, total_bits as u64)
                    else {
                        // Unreachable: the buffer holds `total_bits`. Also
                        // skips the close below.
                        stage.stats.partial_errors += 1;
                        return;
                    };
                    Self::process_complete_payload(
                        &stored_header,
                        &mut payload_reader,
                        stage,
                        sink,
                        ids,
                    );
                }
            }
        } else if bit_count != 0 || header.b_open {
            // Even an empty open must fail its actor GUID read and retire the
            // actor it displaced; payload-less closes need no payload walk.
            let mut payload = payload;
            Self::process_complete_payload(header, &mut payload, stage, sink, ids);
        }

        // A partial bunch's close flag is its final fragment's, not
        // `stored_header`'s: `UChannel::SendBunch` puts `bOpen` on the first
        // fragment and `bClose` on the last.
        if header.b_close {
            Self::close_channel(header, stage, accumulator, sink);
        }
    }

    /// Close a channel. A destroying (non-dormant) close also retires the
    /// channel row and its reassembly state, whose partial could otherwise
    /// never complete nor be counted lost; a dormant close keeps both, as the
    /// packet reader keeps its sequence state. Every `b_close` site calls this.
    fn close_channel(
        header: &RawBunchHeader,
        stage: &mut Stage<'_>,
        accumulator: &mut PartialBunchAccumulator,
        sink: &mut dyn ReplicationSink,
    ) {
        channel::handle_channel_close(header, stage.channels, stage.stats, sink);
        Self::retire_destroyed_channel(header, stage, accumulator, sink);
    }

    fn retire_destroyed_channel(
        header: &RawBunchHeader,
        stage: &mut Stage<'_>,
        accumulator: &mut PartialBunchAccumulator,
        sink: &mut dyn ReplicationSink,
    ) {
        if header.b_dormant {
            return;
        }
        stage.channels.remove(&header.ch_index);
        if let Some(discarded) = accumulator.retire_channel(header.ch_index) {
            stage.stats.partial_errors += 1;
            stage.stats.partial_channel_close += 1;
            stage.stats.skipped_bits += discarded.bit_count as u64;
            let reason = PartialPayloadReason::ChannelClosed;
            let row =
                RejectedPartialFragment::accumulated(&discarded, reason, Some(header.packet_id));
            sink.on_rejected_partial(row);
        }
    }

    /// Count a bunch-header failure (a channel-state refusal, or a failed
    /// package-map, must-be-mapped or open read: the reader stands at an
    /// indeterminate bit), abandon the bunch and retire any actor its open
    /// displaced. The charge is the whole window, by the rule on `framing::abort`.
    fn abandon_bunch(header: &RawBunchHeader, payload: &mut BitReader<'_>, stage: &mut Stage<'_>) {
        stage.stats.bunch_header_failures += 1;
        stage.stats.skipped_bits += payload.len_bits();
        payload.skip_remaining();
        Self::retire_after_failed_open(header, stage);
    }

    /// Take the channel from the live actor it held when an open bunch stops
    /// short of its open (a header failure, or clean package-map exports:
    /// nothing after them is read). `handle_channel_open` writes state only
    /// after a complete open, so later bunches would otherwise frame under the
    /// old actor's schema; they go to [`Self::drop_unopened`] instead. No
    /// close is emitted (the replay sent none); a bunch without `b_open`, and a
    /// dormant or closed state, are left alone.
    fn retire_after_failed_open(header: &RawBunchHeader, stage: &mut Stage<'_>) {
        if !header.b_open {
            return;
        }
        let live = stage
            .channels
            .get_mut(&header.ch_index)
            .filter(|slot| slot.state.as_ref().is_some_and(|state| state.is_open));
        if let Some(slot) = live {
            slot.state = None;
            stage.stats.failed_reopens_while_open += 1;
        }
    }

    /// Count and discard a bunch whose channel has no open actor: only the
    /// bits left after its preambles. `bits_remaining()` is right here, unlike
    /// in [`Self::abandon_bunch`]: nothing failed to read, so the reader stands
    /// where the unframed content begins.
    fn drop_unopened(payload: &mut BitReader<'_>, stage: &mut Stage<'_>) {
        let dropped = payload.bits_remaining();
        if dropped == 0 {
            return;
        }
        stage.stats.bunches_on_unopened_channel += 1;
        stage.stats.unopened_channel_bits += dropped;
        payload.skip_remaining();
    }

    /// Walk one whole (reassembled, if it was partial) bunch payload.
    fn process_complete_payload(
        header: &RawBunchHeader,
        payload: &mut BitReader<'_>,
        stage: &mut Stage<'_>,
        sink: &mut dyn ReplicationSink,
        ids: BunchIds,
    ) {
        let ch_index = header.ch_index;

        // A package-map export bunch ends here either way: nothing after the
        // exports is read or counted, so an open behind them is never reached
        // and the actor it displaces is retired.
        if header.b_has_package_map_exports {
            if channel::read_package_map_exports(payload, stage.stats, sink).is_ok() {
                stage.stats.package_map_exports += 1;
                Self::retire_after_failed_open(header, stage);
            } else {
                Self::abandon_bunch(header, payload, stage);
            }
            return;
        }

        // A failed must-be-mapped read abandons any open behind the list with
        // the bunch; a failed open writes no state.
        if (header.b_has_must_be_mapped_guids
            && channel::read_must_be_mapped_guids(payload, stage.stats).is_err())
            || (header.b_open
                && channel::handle_channel_open(header, payload, stage.channels, stage.stats, sink)
                    .is_err())
        {
            return Self::abandon_bunch(header, payload, stage);
        }

        // No open actor on this channel: nothing to frame the rest under.
        let Some(ch) = stage
            .channels
            .get(&ch_index)
            .and_then(|s| s.state.as_ref())
            .filter(|s| s.is_open)
        else {
            return Self::drop_unopened(payload, stage);
        };
        let (actor_net_guid, archetype_net_guid) = (ch.actor_net_guid, ch.archetype_net_guid);

        // ReadNetPlayerIndex ([`channel::is_player_controller_channel`]). The
        // cheap flags come first: the path check costs two cache lookups.
        if header.b_open
            && actor_net_guid.is_dynamic()
            && !payload.at_end()
            && channel::is_player_controller_channel(actor_net_guid, archetype_net_guid, sink)
        {
            let _ = payload.read_u8();
        }

        let ctx = BunchContext {
            header,
            ids,
            actor_net_guid,
            archetype_net_guid,
        };
        framing::frame_content_blocks(payload, stage, sink, &ctx);
    }
}

/// Copy a partial bunch's payload into `buffer`, byte-aligned, and return the
/// bytes it occupies: [`PartialBunchAccumulator::add_fragment`] concatenates
/// bytes, but a payload is a bit window at any offset. `buffer` only grows and
/// its tail is stale, but `copy_bits_to` rewrites the whole returned prefix,
/// padding bits included.
fn stage_fragment<'b>(mut payload: BitReader<'_>, buffer: &'b mut Vec<u8>) -> &'b [u8] {
    let bit_count = payload.bits_remaining();
    let byte_count = (bit_count as usize).div_ceil(8);
    if buffer.len() < byte_count {
        buffer.resize(byte_count, 0);
    }
    payload
        .copy_bits_to(buffer, bit_count)
        .expect("the buffer holds the whole window");
    &buffer[..byte_count]
}

/// Map an accumulator discard cause to the reason reported to the sink, shared
/// by a rejected fragment and every assembly it displaced so both name a
/// discard alike.
fn partial_payload_reason(cause: PartialDiscardCause) -> PartialPayloadReason {
    use PartialDiscardCause::{Resource, Sequence};
    match cause {
        Sequence(PartialSequenceKind::MissingInitial) => PartialPayloadReason::MissingInitial,
        Sequence(PartialSequenceKind::OverlappingInitial) => {
            PartialPayloadReason::OverlappingInitial
        }
        Sequence(PartialSequenceKind::MismatchedContinuation) => {
            PartialPayloadReason::MismatchedContinuation
        }
        Sequence(PartialSequenceKind::NonByteAlignedFragment) => {
            PartialPayloadReason::NonByteAlignedFragment
        }
        Sequence(PartialSequenceKind::PayloadOverrun) => PartialPayloadReason::PayloadOverrun,
        Resource(PartialResourceLimit::ActiveStates) => PartialPayloadReason::ActiveStateLimit,
        Resource(PartialResourceLimit::BufferedBits) => PartialPayloadReason::BufferedBitsLimit,
        Resource(PartialResourceLimit::Allocation) => PartialPayloadReason::AllocationFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bits::{
        BitWrite, BunchSpec, build_bunch_packet, build_packet, pack, write_bunch,
    };

    struct OwnedRejectedPartial {
        header: RawBunchHeader,
        kind: &'static str,
        reason: PartialPayloadReason,
        bit_count: usize,
        payload: Vec<u8>,
        rejection_packet_id: Option<i32>,
    }

    #[derive(Default)]
    pub(super) struct TestSink {
        fields: Vec<(u32, u32)>,
        rpcs: Vec<(u32, u32)>,
        stream_failures: Vec<StreamFailure>,
        unresolved_payloads: Vec<(StreamFailure, Vec<u8>)>,
        /// `stream_failures.len()` at each payload callback: the failure must
        /// already be there.
        failures_before_payload: Vec<usize>,
        opens: Vec<u32>,
        closes: Vec<u32>,
        paths: Vec<(u32, String)>,
        /// GUID-to-path map for `path_for_guid`; empty by default, so every
        /// lookup is `None`.
        pub(super) guid_paths: std::collections::HashMap<u32, String>,
        /// Content-block headers, in arrival order.
        content_blocks: Vec<ContentBlockHeader>,
        block_contexts: Vec<ContentBlockContext>,
        rep_layout_tails: Vec<(u32, Vec<u8>)>,
        rep_layout_tail_outcome: Option<RepLayoutTailOutcome>,
        rejected_partials: Vec<OwnedRejectedPartial>,
    }

    impl GuidPathSink for TestSink {
        fn register_path(&mut self, guid: u32, path: &str, _outer: NetworkGuid) {
            self.paths.push((guid, path.to_owned()));
        }
        fn path_for_guid(&self, guid: u32) -> Option<&str> {
            self.guid_paths.get(&guid).map(|s| s.as_str())
        }
    }

    impl FieldSink for TestSink {
        fn on_field(&mut self, handle: u32, bit_count: u32, _reader: BitReader<'_>) {
            self.fields.push((handle, bit_count));
        }
        fn on_rpc(&mut self, handle: u32, bit_count: u32, _reader: BitReader<'_>) {
            self.rpcs.push((handle, bit_count));
        }
    }

    impl ReplicationSink for TestSink {
        fn on_rejected_partial(&mut self, p: RejectedPartialFragment<'_>) {
            self.rejected_partials.push(OwnedRejectedPartial {
                header: p.header.clone(),
                kind: p.payload_kind,
                reason: p.reason,
                bit_count: p.bit_count,
                payload: p.payload.to_vec(),
                rejection_packet_id: p.rejection_packet_id,
            });
        }
        fn wants_stream_failure_details(&self) -> bool {
            true
        }

        fn on_actor_open(&mut self, state: &ActorChannelState) {
            self.opens.push(state.channel_index);
        }

        fn on_rep_layout_tail(
            &mut self,
            _actor_net_guid: NetworkGuid,
            bit_count: u32,
            mut reader: BitReader<'_>,
        ) -> RepLayoutTailOutcome {
            let mut bytes = vec![0; (bit_count as usize).div_ceil(8)];
            reader
                .copy_bits_to(&mut bytes, u64::from(bit_count))
                .expect("tail reader is bounded to the reported bit count");
            self.rep_layout_tails.push((bit_count, bytes));
            self.rep_layout_tail_outcome
                .unwrap_or(RepLayoutTailOutcome::Unpreserved {
                    cause: StreamFailureCause::UnverifiedRepLayoutTail,
                })
        }
        fn on_actor_close(&mut self, channel_index: u32, _: NetworkGuid, _: bool) {
            self.closes.push(channel_index);
        }
        fn on_content_block(
            &mut self,
            _channel_index: u32,
            _actor_net_guid: NetworkGuid,
            header: &ContentBlockHeader,
        ) -> u32 {
            self.content_blocks.push(header.clone());
            0
        }
        fn on_content_block_with_context(
            &mut self,
            channel_index: u32,
            actor_net_guid: NetworkGuid,
            header: &ContentBlockHeader,
            context: &ContentBlockContext,
        ) -> u32 {
            self.block_contexts.push(*context);
            self.on_content_block(channel_index, actor_net_guid, header)
        }
        fn on_deleted_block(
            &mut self,
            _channel_index: u32,
            _actor_net_guid: NetworkGuid,
            _header: &ContentBlockHeader,
        ) {
        }

        fn on_stream_failure(&mut self, failure: StreamFailure) {
            self.stream_failures.push(failure);
        }

        fn on_unresolved_class_net_cache_payload(
            &mut self,
            failure: StreamFailure,
            payload: &[u8],
        ) {
            self.failures_before_payload
                .push(self.stream_failures.len());
            self.unresolved_payloads.push((failure, payload.to_vec()));
        }

        fn on_stream_failure_payload(&mut self, _: StreamFailure, _: &[u8]) {
            self.failures_before_payload
                .push(self.stream_failures.len());
        }

        fn on_rep_layout_tail_failure_payload(&mut self, _: StreamFailure, _: BitReader<'_>) {
            self.failures_before_payload
                .push(self.stream_failures.len());
        }
    }

    fn reader() -> ReplicationReader {
        ReplicationReader::new("++Ares-Core+release-13.01").unwrap()
    }

    /// Process `packets` in order, packet ids from 0, on a fresh 13.01 reader.
    fn run_packets(packets: &[Vec<u8>]) -> (ReplicationReader, TestSink) {
        let (mut reader, mut sink) = (reader(), TestSink::default());
        for (packet_id, packet) in packets.iter().enumerate() {
            reader.process_packet(packet, packet_id as i32, &mut sink);
        }
        (reader, sink)
    }

    /// An IntPacked GUID. GUID 3, a static actor, is the byte 6.
    fn guid(value: u32) -> Vec<bool> {
        let mut bits = Vec::new();
        bits.int_packed(value);
        bits
    }

    /// An actor RepLayout block with an empty body: hasRepLayout, isActor,
    /// contentBits 0 -- 10 bits.
    fn empty_actor_block() -> Vec<bool> {
        let mut bits = vec![true, true];
        bits.int_packed(0);
        bits
    }

    /// A static (odd) actor's open, which has no spawn block, then an empty
    /// actor block.
    fn static_actor(actor: u32) -> Vec<bool> {
        let mut bits = guid(actor);
        bits.extend(empty_actor_block());
        bits
    }

    /// One reliable, non-partial bunch that opens `ch_index` around `payload`.
    fn build_open_bunch_packet(ch_index: u32, payload: &[bool]) -> Vec<u8> {
        let spec = BunchSpec {
            ch_index,
            b_open: true,
            ..Default::default()
        };
        build_bunch_packet(&spec, payload)
    }

    /// Channel 2 opens for static actor `actor`.
    fn open_on_two(actor: u32) -> Vec<u8> {
        build_open_bunch_packet(2, &static_actor(actor))
    }

    /// A payload-less close of channel 2, destroying or dormant.
    fn close_on_two(dormant: bool) -> Vec<u8> {
        let spec = BunchSpec {
            ch_index: 2,
            b_close: true,
            dormant,
            ..Default::default()
        };
        build_bunch_packet(&spec, &[])
    }

    /// A non-open bunch on channel 2 carrying one empty 10-bit actor block.
    fn later_block_on_two() -> Vec<u8> {
        let spec = BunchSpec {
            ch_index: 2,
            ..Default::default()
        };
        build_bunch_packet(&spec, &empty_actor_block())
    }

    /// A reliable partial fragment on channel 2.
    fn partial_packet(open: bool, initial: bool, last: bool, payload: &[bool]) -> Vec<u8> {
        build_bunch_packet(
            &BunchSpec {
                ch_index: 2,
                b_open: open,
                b_partial: true,
                b_partial_initial: initial,
                b_partial_final: last,
                ..Default::default()
            },
            payload,
        )
    }

    /// The reused staging buffer only grows: a short fragment after a long one
    /// must not read the long one's bytes back.
    #[test]
    fn fragment_staging_never_exposes_the_previous_fragment() {
        let mut buffer = Vec::new();

        let long = [0xFFu8; 4];
        let staged = stage_fragment(BitReader::with_bit_len(&long, 32).unwrap(), &mut buffer);
        assert_eq!(staged, &[0xFF; 4]);

        // Five zero bits: one byte, and the three padding bits above them must
        // be cleared even though the buffer still holds 0xFF underneath.
        let short = [0x00u8];
        let staged = stage_fragment(BitReader::with_bit_len(&short, 5).unwrap(), &mut buffer);
        assert_eq!(staged, &[0x00]);
        assert_eq!(
            buffer.len(),
            4,
            "the buffer keeps its capacity, not its data"
        );

        // A zero-bit fragment stages nothing and must report zero bytes.
        assert!(
            stage_fragment(BitReader::with_bit_len(&short, 0).unwrap(), &mut buffer).is_empty()
        );
    }

    /// An unaligned window is realigned to bit zero of the staging buffer, the
    /// only reason the copy exists.
    #[test]
    fn fragment_staging_realigns_an_offset_window() {
        let packet = [0b1111_0000u8, 0b0000_1111];
        let mut reader = BitReader::new(&packet);
        reader.skip_bits(4).unwrap();
        let window = reader.sub_reader(8).unwrap();

        let mut buffer = Vec::new();
        assert_eq!(stage_fragment(window, &mut buffer), &[0b1111_1111]);
    }

    /// A partial bunch split across two fragments reassembles and frames
    /// exactly as an unsplit one would, and applies the close flag its final
    /// fragment carried (see `process_bunch`).
    #[test]
    fn split_bunch_reassembles_and_frames() {
        for b_close in [false, true] {
            let last = BunchSpec {
                ch_index: 2,
                b_close,
                b_partial: true,
                b_partial_final: true,
                ..Default::default()
            };
            let (reader, sink) = run_packets(&[
                partial_packet(true, true, false, &guid(3)),
                build_bunch_packet(&last, &empty_actor_block()),
            ]);

            let stats = reader.stats();
            assert_eq!(stats.bunches, 2);
            assert_eq!(stats.partial_fragments, 2);
            assert_eq!(stats.partial_bunches, 2);
            assert_eq!(stats.partial_completed, 1);
            assert_eq!(stats.partial_errors, 0);
            assert_eq!(stats.actor_opens, 1, "the open is read from fragment 1");
            assert_eq!(
                stats.content_blocks, 1,
                "the block header spans neither fragment but sits after the join"
            );
            assert_eq!(stats.rep_layout_blocks, 1);
            assert_eq!(stats.skipped_bits, 0, "nothing may be abandoned");
            assert_eq!(sink.opens, vec![2]);
            assert_eq!(stats.actor_closes, u64::from(b_close));
            assert_eq!(sink.closes, if b_close { vec![2] } else { vec![] });
            // A clean pass records and drops no diagnostic event.
            assert!(
                stats.diagnostics.is_empty(),
                "a clean pass recorded diagnostic events: {:?}",
                stats.diagnostics
            );
            assert_eq!(stats.diagnostics_dropped, 0);
        }
    }

    #[test]
    fn block_provenance_uses_authoritative_initial_and_completion_packets() {
        let mut payload = guid(3);
        payload.extend(empty_actor_block());
        payload.extend(empty_actor_block());
        let (mut reader, mut sink) = (reader(), TestSink::default());
        for (packet_id, packet) in [
            (7, partial_packet(true, true, false, &payload[..8])),
            (19, partial_packet(false, false, false, &payload[8..16])),
            (41, partial_packet(false, false, true, &payload[16..])),
            (43, later_block_on_two()),
        ] {
            reader.process_packet(&packet, packet_id, &mut sink);
        }
        assert_eq!(reader.stats().partial_completed, 1);
        assert_eq!(reader.stats().partial_errors, 0);
        assert_eq!(reader.stats().content_blocks, 3);
        assert_eq!(
            sink.block_contexts,
            [
                ContentBlockContext {
                    initial_packet_id: 7,
                    completion_packet_id: 41,
                    bunch_index: 2,
                    block_index: 0,
                },
                ContentBlockContext {
                    initial_packet_id: 7,
                    completion_packet_id: 41,
                    bunch_index: 2,
                    block_index: 1,
                },
                ContentBlockContext {
                    initial_packet_id: 43,
                    completion_packet_id: 43,
                    bunch_index: 3,
                    block_index: 0,
                },
            ]
        );
    }

    /// A partial final with no initial is one error, counted once, by the
    /// accumulator: the packet reader's tracker sees it too but is advisory.
    #[test]
    fn a_partial_final_without_an_initial_is_counted_once() {
        // Its payload, actor GUID 3, is never reached.
        let (reader, sink) = run_packets(&[partial_packet(false, false, true, &guid(3))]);

        let stats = reader.stats();
        assert_eq!(stats.partial_errors, 1, "counted once, not twice");
        assert_eq!(stats.partial_bunches, 1);
        assert_eq!(stats.partial_missing_initial, 1);
        assert_eq!(stats.partial_missing_initial_final, 1);
        assert_eq!(stats.partial_missing_initial_reliable, 1);
        assert_eq!(stats.partial_missing_initial_bits, 8);
        assert_eq!(stats.partial_overlapping_initial, 0);
        assert_eq!(stats.partial_mismatched_continuation, 0);
        assert_eq!(stats.partial_non_byte_aligned, 0);
        assert_eq!(
            rejected_rows(&sink),
            vec![("current_fragment", PartialPayloadReason::MissingInitial, 8)]
        );
        let rejected = &sink.rejected_partials[0];
        assert_eq!(rejected.payload, &[6]);
        assert_eq!(rejected.header.ch_index, 2);
        assert_eq!(rejected.rejection_packet_id, Some(0));
        assert_eq!(stats.skipped_bits, 8, "the rejected payload stays in loss");
    }

    /// An out-of-range GUID count drops every path declaration in the bunch:
    /// a header failure with its bits tallied, not an export processed.
    #[test]
    fn a_package_map_export_with_an_impossible_guid_count_is_counted() {
        let mut payload = vec![false]; // hasRepLayoutExport
        payload.i32(crate::types::MAX_GUID_COUNT as i32 + 1);
        payload.repeat(true, 24); // the declarations that get dropped

        let spec = BunchSpec {
            ch_index: 2,
            b_has_package_map_exports: true,
            ..Default::default()
        };
        let (reader, _) = run_packets(&[build_bunch_packet(&spec, &payload)]);

        let stats = reader.stats();
        assert_eq!(stats.package_map_exports, 0, "nothing was exported");
        assert_eq!(stats.bunch_header_failures, 1);
        assert_eq!(stats.exported_guids, 0);
        assert_eq!(
            stats.skipped_bits, 57,
            "the whole payload: the bits already read declared dropped exports"
        );
    }

    /// A RepLayout-export bunch is skipped whole, a deliberate limitation,
    /// counted on its own line rather than in `skipped_bits`.
    #[test]
    fn a_rep_layout_export_bunch_is_counted_separately() {
        let mut payload = vec![true]; // hasRepLayoutExport: unsupported, skipped
        payload.repeat(true, 32);

        let spec = BunchSpec {
            ch_index: 2,
            b_has_package_map_exports: true,
            ..Default::default()
        };
        let (reader, _) = run_packets(&[build_bunch_packet(&spec, &payload)]);

        let stats = reader.stats();
        assert_eq!(stats.rep_layout_export_bunches, 1);
        assert_eq!(
            stats.bunch_header_failures, 0,
            "a limitation, not a failure"
        );
        assert_eq!(stats.skipped_bits, 0, "no content block was involved");
    }

    #[test]
    fn a_rejected_partial_close_still_retires_the_channel() {
        let close = BunchSpec {
            ch_index: 2,
            b_close: true,
            b_partial: true,
            b_partial_final: true,
            ..Default::default()
        };
        let (reader, _) = run_packets(&[open_on_two(3), build_bunch_packet(&close, &[true; 8])]);

        assert_eq!(reader.stats().partial_errors, 1);
        assert_eq!(reader.stats().skipped_bits, 8);
        assert_eq!(reader.stats().actor_closes, 1);
        assert!(reader.channels.is_empty());
    }

    /// Opening a channel that is already open replaces its actor, as the wire
    /// says; nothing else moves, so the replacement is counted.
    #[test]
    fn opening_an_already_open_channel_is_counted() {
        let (reader, _) = run_packets(&[open_on_two(3), open_on_two(5)]);

        let stats = reader.stats();
        assert_eq!(stats.actor_opens, 2);
        assert_eq!(stats.actor_closes, 0, "no close is fabricated");
        assert_eq!(stats.channel_reopens_while_open, 1);
    }

    /// A failed first open retires nothing, but the next bunch on the channel
    /// is dropped at the guard and counted: the open's own 8 bits are the
    /// header failure's, the later 10 are counted apart from `skipped_bits`.
    #[test]
    fn a_bunch_after_a_failed_first_open_is_counted_not_silent() {
        // Dynamic actor 4 whose spawn block is missing.
        let failed = build_open_bunch_packet(2, &guid(4));
        let (reader, sink) = run_packets(&[failed, later_block_on_two()]);

        let stats = reader.stats();
        assert_eq!(stats.bunch_header_failures, 1);
        assert_eq!(
            stats.failed_reopens_while_open, 0,
            "no live actor to retire"
        );
        assert_eq!(stats.bunches_on_unopened_channel, 1);
        assert_eq!(stats.unopened_channel_bits, 10);
        assert_eq!(stats.skipped_bits, 8, "only the failed open's own window");
        assert_eq!(stats.content_blocks, 0);
        assert!(sink.content_blocks.is_empty());
    }

    /// A dormant channel is not an open one: a failed reopen retires nothing,
    /// and a bunch that then arrives is counted, not framed under the dormant
    /// actor.
    #[test]
    fn a_bunch_on_a_dormant_channel_after_a_failed_reopen_is_counted() {
        let (reader, _) = run_packets(&[
            open_on_two(3),
            close_on_two(true),
            build_open_bunch_packet(2, &guid(4)),
            later_block_on_two(),
        ]);

        let stats = reader.stats();
        assert_eq!(stats.actor_closes, 1, "the dormant close");
        assert_eq!(stats.bunch_header_failures, 1, "the failed reopen");
        assert_eq!(stats.failed_reopens_while_open, 0, "nothing live to retire");
        assert_eq!(stats.content_blocks, 1, "only the first open's block");
        assert_eq!(
            (
                stats.bunches_on_unopened_channel,
                stats.unopened_channel_bits
            ),
            (1, 10)
        );
    }

    // --- open bunches stopped before their open, on a live channel ---
    //
    // Each probe opens channel 2 for static actor 3, sends an open bunch that
    // stops short of its open, then a non-open bunch with an empty block. The
    // later bunch must not be framed under actor 3's schema.

    /// A must-be-mapped list that declares one GUID (u16 1, little-endian) and
    /// ends there.
    fn must_be_mapped_count_without_its_guid() -> Vec<bool> {
        let mut bits = Vec::new();
        bits.u16(1);
        bits
    }

    /// What every probe must show: actor 3 was retired and counted, and the
    /// later block dropped and counted at the guard, not framed as actor 3's.
    fn assert_actor_three_retired(reader: &ReplicationReader, sink: &TestSink) {
        let stats = reader.stats();
        assert_eq!(stats.actor_opens, 1, "only actor 3's open completed");
        assert_eq!(stats.channel_reopens_while_open, 0, "no reopen succeeded");
        assert_eq!(stats.failed_reopens_while_open, 1, "actor 3 retired");
        assert_eq!(stats.content_blocks, 1, "only actor 3's own block");
        assert_eq!(sink.content_blocks.len(), 1);
        assert_eq!(stats.actor_closes, 0, "no close is fabricated");
        assert_eq!(
            (
                stats.bunches_on_unopened_channel,
                stats.unopened_channel_bits
            ),
            (1, 10),
            "the later bunch is dropped and counted, with its whole 10-bit block"
        );
    }

    /// Run a probe: actor 3's open, `reopen`, then the later block.
    fn probe(reopen: Vec<u8>) -> ReplicationReader {
        let (reader, sink) = run_packets(&[open_on_two(3), reopen, later_block_on_two()]);
        assert_actor_three_retired(&reader, &sink);
        reader
    }

    /// The open itself fails: dynamic actor 4 stops before its spawn block.
    #[test]
    fn a_failed_reopen_does_not_leave_the_previous_actor_live() {
        let reader = probe(build_open_bunch_packet(2, &guid(4)));
        assert_eq!(reader.stats().bunch_header_failures, 1);
    }

    #[test]
    fn an_empty_reopen_retires_the_previous_actor() {
        // Given: actor 3 is open before an empty complete reopen.
        let packets = [
            open_on_two(3),
            build_open_bunch_packet(2, &[]),
            later_block_on_two(),
        ];

        // When: the later actor content follows the failed reopen.
        let (reader, sink) = run_packets(&packets);

        // Then: only the original block belongs to actor 3; no close is invented.
        assert_actor_three_retired(&reader, &sink);
        assert_eq!(sink.content_blocks[0].outer_net_guid, NetworkGuid(3));
        assert!(sink.closes.is_empty());
        assert_eq!(reader.stats().bunch_header_failures, 1);
        assert_eq!(reader.stats().actor_opens_missing_spawn, 0);
        assert_eq!(
            reader.stats().skipped_bits,
            0,
            "the failed open has no bits"
        );
    }

    #[test]
    fn a_one_bit_reopen_retires_the_previous_actor() {
        // Given: a one-bit actor GUID cannot complete.
        let reopen = build_open_bunch_packet(2, &[false]);

        // When: actor 3's open, the truncated reopen and later content arrive.
        let reader = probe(reopen);

        // Then: the same failure accounting applies, charging the one wire bit.
        assert_eq!(reader.stats().bunch_header_failures, 1);
        assert_eq!(reader.stats().skipped_bits, 1);
    }

    #[test]
    fn a_payload_less_close_still_closes_the_live_actor() {
        // Given: a live actor followed by an ordinary payload-less close.
        let packets = [open_on_two(3), close_on_two(false)];

        // When: the close passes through the packet pipeline.
        let (reader, sink) = run_packets(&packets);

        // Then: the close succeeds without a header failure or failed reopen.
        assert_eq!(sink.closes, vec![2]);
        assert_eq!(reader.stats().actor_closes, 1);
        assert_eq!(reader.stats().bunch_header_failures, 0);
        assert_eq!(reader.stats().failed_reopens_while_open, 0);
        assert_eq!(reader.stats().skipped_bits, 0);
    }

    /// An open bunch whose must-be-mapped list fails to read is abandoned
    /// before its open is reached.
    #[test]
    fn an_open_whose_must_be_mapped_read_fails_retires_the_live_actor() {
        let spec = BunchSpec {
            ch_index: 2,
            b_open: true,
            b_has_must_be_mapped_guids: true,
            ..Default::default()
        };
        let reader = probe(build_bunch_packet(
            &spec,
            &must_be_mapped_count_without_its_guid(),
        ));
        assert_eq!(reader.stats().bunch_header_failures, 1);
        assert_eq!(reader.stats().skipped_bits, 16, "the abandoned window");
    }

    /// An open bunch whose package-map exports fail to read -- a negative GUID
    /// count -- is abandoned before its open is reached.
    #[test]
    fn an_open_whose_package_map_read_fails_retires_the_live_actor() {
        let mut exports = vec![false]; // hasRepLayoutExport
        exports.i32(-1).extend_bits(&static_actor(3)); // the open, never reached
        let spec = BunchSpec {
            ch_index: 2,
            b_open: true,
            b_has_package_map_exports: true,
            ..Default::default()
        };
        let reader = probe(build_bunch_packet(&spec, &exports));

        let stats = reader.stats();
        assert_eq!(stats.bunch_header_failures, 1);
        assert_eq!(stats.package_map_exports, 0);
        assert_eq!(stats.skipped_bits, 51, "the abandoned window: 1 + 32 + 18");
    }

    /// Clean exports do not save the open: nothing after an export list is
    /// read or tallied, and `failed_reopens_while_open` alone says an open
    /// was lost.
    #[test]
    fn an_open_behind_clean_package_map_exports_retires_the_live_actor() {
        let mut exports = vec![false]; // hasRepLayoutExport
        exports.i32(0).extend_bits(&static_actor(3)); // no GUIDs; the open, never read
        let spec = BunchSpec {
            ch_index: 2,
            b_open: true,
            b_has_package_map_exports: true,
            ..Default::default()
        };
        let reader = probe(build_bunch_packet(&spec, &exports));

        assert_eq!(reader.stats().package_map_exports, 1);
        assert_eq!(reader.stats().bunch_header_failures, 0);
    }

    /// An open bunch refused at the channel-state limit never reaches the
    /// header stages at all.
    #[test]
    fn an_open_refused_at_the_channel_limit_retires_the_live_actor() {
        let (mut reader, mut sink) = run_packets(&[open_on_two(3)]);
        // No room for reliable-sequence state refuses the reopen while the
        // pipeline still holds actor 3; restored for the later bunch.
        reader.packet_reader = RawPacketReader::with_max_channels(0);
        reader.process_packet(&open_on_two(3), 1, &mut sink);
        reader.packet_reader = RawPacketReader::new();
        reader.process_packet(&later_block_on_two(), 2, &mut sink);

        assert_eq!(reader.stats().channel_state_limit_failures, 1);
        assert_eq!(reader.stats().bunch_header_failures, 1, "abandoned");
        assert_actor_three_retired(&reader, &sink);
    }

    /// The refusal arm retires before it closes: a refused open+close bunch
    /// carried its own actor's close, not actor 3's, so actor 3 gets no close.
    #[test]
    fn an_open_and_close_refused_at_the_channel_limit_closes_nothing_it_did_not_open() {
        let (mut reader, mut sink) = run_packets(&[open_on_two(3)]);
        reader.packet_reader = RawPacketReader::with_max_channels(0);
        let spec = BunchSpec {
            ch_index: 2,
            b_open: true,
            b_close: true,
            ..Default::default()
        };
        reader.process_packet(&build_bunch_packet(&spec, &static_actor(3)), 1, &mut sink);

        let stats = reader.stats();
        assert_eq!(stats.channel_state_limit_failures, 1);
        assert_eq!(stats.failed_reopens_while_open, 1);
        assert_eq!(stats.actor_closes, 0, "actor 3 was never closed");
        assert!(sink.closes.is_empty(), "no close row for actor 3");
        assert!(reader.channels.is_empty(), "the channel is still retired");
    }

    /// Only an open bunch displaces an actor: a non-open bunch whose header
    /// stage fails is abandoned, and actor 3 stays live. Pins the `b_open` gate.
    #[test]
    fn a_header_failure_without_an_open_leaves_the_live_actor_open() {
        let spec = BunchSpec {
            ch_index: 2,
            b_has_must_be_mapped_guids: true,
            ..Default::default()
        };
        let failed = build_bunch_packet(&spec, &must_be_mapped_count_without_its_guid());
        let (reader, sink) = run_packets(&[open_on_two(3), failed, later_block_on_two()]);

        let stats = reader.stats();
        assert_eq!(stats.bunch_header_failures, 1, "the truncated list counts");
        assert_eq!(stats.failed_reopens_while_open, 0, "nothing was reopened");
        assert_eq!(stats.content_blocks, 2, "the later bunch is actor 3's");
        assert_eq!(sink.content_blocks.len(), 2);
        assert_eq!(stats.bunches_on_unopened_channel, 0);
    }

    /// The guard counts what it drops: the payload left after the preambles,
    /// nothing for a bunch whose preamble consumed every bit. A must-be-mapped
    /// list is read and counted before the guard either way.
    #[test]
    fn a_bunch_on_a_never_opened_channel_counts_only_what_it_drops() {
        let mut preamble = Vec::new();
        preamble.u16(1).int_packed(6); // one must-be-mapped GUID
        let spec = BunchSpec {
            ch_index: 9,
            b_has_must_be_mapped_guids: true,
            ..Default::default()
        };

        let mut bits = Vec::new();
        write_bunch(&mut bits, &spec, &preamble);
        let mut with_block = preamble.clone();
        with_block.extend(empty_actor_block());
        write_bunch(&mut bits, &spec, &with_block);
        let (reader, _) = run_packets(&[build_packet(&bits)]);

        let stats = reader.stats();
        assert_eq!(stats.bunches, 2);
        assert_eq!(stats.must_be_mapped_guids, 2);
        assert_eq!(
            stats.bunches_on_unopened_channel, 1,
            "the preamble-only bunch dropped nothing"
        );
        assert_eq!(
            stats.unopened_channel_bits, 10,
            "the 24 preamble bits were read, not dropped"
        );
        assert_eq!(stats.bunch_header_failures, 0);
        assert_eq!(stats.skipped_bits, 0);
        assert_eq!(stats.content_blocks, 0);
    }

    /// A destroyed channel's row is retired, so channels do not accumulate,
    /// and reopening it is the ordinary case, not an overwrite.
    #[test]
    fn a_destroyed_channel_is_retired_before_later_reuse() {
        let (mut reader, mut sink) = run_packets(&[open_on_two(3), close_on_two(false)]);
        assert!(
            reader.channels.is_empty(),
            "destroyed channels must not accumulate"
        );

        reader.process_packet(&open_on_two(5), 2, &mut sink);
        let stats = reader.stats();
        assert_eq!(stats.actor_opens, 2);
        assert_eq!(stats.actor_closes, 1);
        assert_eq!(stats.channel_reopens_while_open, 0);
        assert_eq!(
            reader.channels.len(),
            1,
            "the channel index remains reusable"
        );
    }

    /// At the active-channel budget no bunch, open or not, adds a channel: it
    /// is refused, abandoned and counted.
    #[test]
    fn a_bunch_beyond_the_active_channel_budget_fails_closed() {
        let max = crate::types::MAX_ACTIVE_CHANNELS;
        for b_open in [true, false] {
            let mut reader = reader();
            for ch_index in 0..max as u32 {
                reader.channels.insert(ch_index, ChannelSlot::default());
            }
            let payload = static_actor(3);
            let spec = BunchSpec {
                ch_index: max as u32,
                b_open,
                ..Default::default()
            };
            let packet = build_bunch_packet(&spec, &payload);
            reader.process_packet(&packet, 0, &mut TestSink::default());

            let stats = reader.stats();
            assert_eq!(reader.channels.len(), max);
            assert_eq!(stats.channel_state_limit_failures, 1);
            assert_eq!(stats.actor_opens, 0);
            assert_eq!(stats.bunch_header_failures, 1);
            assert_eq!(stats.skipped_bits, payload.len() as u64);
        }
    }

    #[test]
    fn raw_reliable_state_refusal_is_abandoned_and_counted_once() {
        let mut reader = reader();
        reader.packet_reader = RawPacketReader::with_max_channels(0);
        let payload = vec![true, false, true, false, true];
        let packet = build_bunch_packet(&BunchSpec::default(), &payload);
        reader.process_packet(&packet, 0, &mut TestSink::default());

        assert!(reader.channels.is_empty());
        assert_eq!(reader.stats().channel_state_limit_failures, 1);
        assert_eq!(reader.stats().bunch_header_failures, 1);
        assert_eq!(reader.stats().skipped_bits, payload.len() as u64);
    }

    /// A dynamic actor's spawn block is mandatory: a payload that ends at the
    /// actor GUID is a failed open, not an actor with archetype 0 and no
    /// transform.
    #[test]
    fn a_dynamic_open_without_its_spawn_block_is_a_failure_not_an_actor() {
        let (reader, sink) = run_packets(&[build_open_bunch_packet(2, &guid(2))]);

        let stats = reader.stats();
        assert_eq!(stats.actor_opens, 0, "no actor may be invented");
        assert_eq!(stats.actor_opens_missing_spawn, 1);
        assert_eq!(stats.bunch_header_failures, 1);
        assert!(sink.opens.is_empty(), "no open event may reach the sink");
    }

    /// A static actor has no spawn block, so its open is complete at the actor
    /// GUID.
    #[test]
    fn a_static_open_with_no_payload_left_is_still_an_actor() {
        let (reader, sink) = run_packets(&[build_open_bunch_packet(2, &guid(3))]);

        assert_eq!(reader.stats().actor_opens, 1);
        assert_eq!(reader.stats().actor_opens_missing_spawn, 0);
        assert_eq!(reader.stats().bunch_header_failures, 0);
        assert_eq!(sink.opens, vec![2]);
    }

    /// A partial whose continuation never arrives is loss only `finish` can
    /// name, not a sequence error; a second `finish` counts nothing again.
    #[test]
    fn finish_counts_an_unfinished_partial_once() {
        let (mut reader, _) = run_packets(&[partial_packet(true, true, false, &guid(3))]);
        assert_eq!(
            reader.stats().partial_errors,
            0,
            "an in-progress reassembly is not an error until the stream ends"
        );

        reader.finish();
        reader.finish();

        let stats = reader.stats();
        assert_eq!(
            (stats.unfinished_partials, stats.unfinished_partial_bits),
            (1, 8)
        );
        assert_eq!(stats.partial_errors, 0, "still not a sequence error");
    }

    #[test]
    fn reader_requires_valid_branch() {
        assert!(ReplicationReader::new("++Ares-Core+release-13.01").is_ok());
        assert!(ReplicationReader::new("++Ares-Core+release-99.99").is_err());
    }

    #[test]
    fn empty_packet_is_no_op() {
        let (reader, _) = run_packets(&[vec![]]);
        assert_eq!(reader.stats().packets, 1);
        assert_eq!(reader.stats().bunches, 0);
        assert_eq!(reader.stats().malformed_packets, 0);
    }

    #[test]
    fn malformed_packet_counted() {
        let (reader, _) = run_packets(&[vec![0x00, 0x00]]);
        assert_eq!(reader.stats().malformed_packets, 1);
    }

    /// One block handed straight to `decode_and_walk`, bypassing framing.
    struct Run {
        /// What the call returned: `false` only for a failed transform.
        transformed: bool,
        stats: NetStats,
        sink: TestSink,
        /// Where the bunch reader stood afterwards.
        position: u64,
    }

    /// Decode a `bit_count`-bit block for actor 2 from the first `window_bits`
    /// of `wire` into `scratch`: RepLayout when `function_count` is `None`,
    /// ClassNetCache with that count otherwise.
    fn decode_block(
        wire: &[u8],
        window_bits: u64,
        bit_count: usize,
        function_count: Option<u32>,
        mut scratch: Vec<u8>,
        mut sink: TestSink,
    ) -> Run {
        let mut payload = BitReader::with_bit_len(wire, window_bits).unwrap();
        let mut stats = NetStats::default();
        let mut channels = ChannelTable::default();
        let mut stage = Stage {
            stats: &mut stats,
            channels: &mut channels,
            transform: TransformVersion::V1301,
            scratch: &mut scratch,
        };
        let actor = NetworkGuid(2);
        let transformed = framing::decode_and_walk(
            &mut payload,
            bit_count,
            actor,
            function_count,
            &mut stage,
            &mut sink,
        );
        Run {
            transformed,
            stats,
            sink,
            position: payload.position(),
        }
    }

    /// The golden V13.01 vector: wire 0xBF for actor 2 decodes to 0x66, seven
    /// bits. The scratch buffer starts full of 0xFF, so a leaked tail shows.
    fn decode_golden(bit_count: usize, function_count: Option<u32>) -> Run {
        let sink = TestSink::default();
        decode_block(&[0xBF], 7, bit_count, function_count, vec![0xFF; 16], sink)
    }

    /// Decode the block whose decoded bits are `decoded_bits`, starting from
    /// an empty scratch buffer.
    fn decode_bits(decoded_bits: &[bool], function_count: Option<u32>, sink: TestSink) -> Run {
        let bit_count = decoded_bits.len();
        let wire = wire_for_short_decoded(&pack(decoded_bits), bit_count, 2);
        decode_block(
            &wire,
            bit_count as u64,
            bit_count,
            function_count,
            Vec::new(),
            sink,
        )
    }

    /// Invert the short V13.01 byte transform for a test payload: below 32
    /// bits it works byte by byte, so each byte's one preimage can be searched
    /// for without duplicating the production transform.
    fn wire_for_short_decoded(decoded: &[u8], bit_count: usize, actor: u32) -> Vec<u8> {
        assert!(bit_count < 32);
        let byte_count = bit_count.div_ceil(8);
        let mut wire = vec![0u8; byte_count];
        for offset in 0..byte_count {
            let mask = if offset + 1 == byte_count && bit_count % 8 != 0 {
                0xff >> (8 - bit_count % 8)
            } else {
                0xff
            };
            wire[offset] = (0u8..=u8::MAX)
                .find(|&candidate| {
                    let mut trial = wire.clone();
                    trial[offset] = candidate;
                    TransformVersion::V1301
                        .apply(
                            &mut trial,
                            bit_count,
                            vrf_transform::seed_for(bit_count, actor),
                        )
                        .unwrap();
                    trial[offset] & mask == decoded[offset] & mask
                })
                .expect("the transform is bijective");
        }
        wire
    }

    /// The unresolved callback receives the exact decoded block, not the wire
    /// bytes or the reusable scratch tail, and the failure names the arm: the
    /// walk never began, so offset 0 and no handle.
    #[test]
    fn unresolved_class_net_cache_exposes_the_decoded_whole_payload() {
        let Run {
            stats,
            sink,
            position,
            ..
        } = decode_golden(7, Some(0));

        assert_eq!(position, 7);
        assert_eq!(sink.unresolved_payloads.len(), 1);
        let (preserved, decoded) = &sink.unresolved_payloads[0];
        assert_eq!(preserved.kind, StreamKind::Rpc);
        assert_eq!(preserved.actor_net_guid, NetworkGuid(2));
        assert_eq!(preserved.bit_count, 7);
        assert_eq!(preserved.function_count, 0);
        assert_eq!(preserved.consumed_bits, 0);
        assert_eq!(preserved.remaining_bits, 7);
        assert_eq!(decoded, &[0x66]);
        assert_eq!(decoded[0] >> 7, 0, "high padding bit must stay zero");
        assert_eq!(sink.failures_before_payload, [1], "failure, then payload");
        let failure = &sink.stream_failures[0];
        assert_eq!(failure.cause, StreamFailureCause::UnresolvedFunctionCount);
        assert_eq!(
            (failure.record_handle, failure.record_offset),
            (None, Some(0))
        );
        assert_eq!(stats.rpcs, 0);
        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(stats.unresolved_rpc_payloads_preserved, 1);
        assert_eq!(stats.skipped_bits, 7);
    }

    /// A transform failure goes back to the framing loop, which alone holds the
    /// header and position its `ParseFailure` event needs. Framing never gets
    /// here on its own (see `SkipReason::ParseFailure`), so an 8-bit block over
    /// a 7-bit payload, called directly, is the way in.
    #[test]
    fn a_transform_failure_is_reported_back_to_framing() {
        // RepLayout, then ClassNetCache with a resolved function count.
        for function_count in [None, Some(2)] {
            let Run {
                transformed, stats, ..
            } = decode_golden(7, function_count);
            assert!(
                transformed,
                "a payload whose transform ran is reported decoded"
            );
            assert_eq!(stats.transform_failures, 0);

            let Run {
                transformed,
                stats,
                sink,
                ..
            } = decode_golden(8, function_count);
            assert!(
                !transformed,
                "an 8-bit block cannot be copied out of 7 bits"
            );
            assert_eq!(stats.transform_failures, 1);
            assert_eq!(stats.skipped_bits, 8, "the block's whole declared length");
            assert_eq!(
                stats.field_stream_failures + stats.rpc_stream_failures,
                0,
                "a transform failure is not a stream failure"
            );
            assert!(sink.stream_failures.is_empty());
            assert!(sink.fields.is_empty() && sink.rpcs.is_empty());
            assert!(
                stats.diagnostics.is_empty(),
                "the event is the framing loop's to record"
            );
        }

        // Through the framing loop, a block whose transform ran records no
        // event, whatever its inner stream does (here an unresolved payload).
        let mut bits = vec![false, true]; // ClassNetCache, isActor
        bits.int_packed(7);
        bits.extend([true, true, true, true, true, true, false]);
        let (stats, sink) = frame_bits(&bits);
        assert_eq!(stats.class_net_cache_blocks, 1);
        assert_eq!(stats.transform_failures, 0);
        assert_eq!(sink.unresolved_payloads.len(), 1);
        assert!(stats.diagnostics.is_empty());
    }

    /// A ClassNetCache walk that returns `Ok` but abandons bits is a stream
    /// failure with those bits in `skipped_bits`. With `function_count` 2 the
    /// golden 0x66 gives one handle bit, then 6 bits -- fewer than the 8 a
    /// payload length needs -- abandoned together: 7.
    #[test]
    fn class_net_cache_overrun_ok_path_is_a_stream_failure() {
        let Run {
            stats,
            sink,
            position,
            ..
        } = decode_golden(7, Some(2));

        assert_eq!(position, 7);
        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(stats.unresolved_rpc_payloads_preserved, 0);
        assert_eq!(stats.rpcs, 0);
        assert_eq!(stats.skipped_bits, 7, "the handle bit and the 6 after it");
        assert_eq!(sink.stream_failures.len(), 1);
        assert_eq!(sink.stream_failures[0].kind, StreamKind::Rpc);
        assert_eq!(sink.stream_failures[0].remaining_bits, 7);
        assert_eq!(sink.stream_failures[0].function_count, 2);
    }

    /// A zero handle closes only the RepLayout prefix. Bits after it belong to
    /// the chained ClassNetCache stream and must reach the sink exactly.
    #[test]
    fn rep_layout_zero_terminator_hands_the_exact_tail_to_the_sink() {
        let mut decoded_bits = vec![false]; // property checksum
        decoded_bits.int_packed(0); // RepLayout terminator
        decoded_bits.extend((0..13).map(|index| index % 2 == 0));

        let sink = TestSink {
            rep_layout_tail_outcome: Some(RepLayoutTailOutcome::Decoded { rpc_count: 1 }),
            ..TestSink::default()
        };
        let Run {
            transformed,
            stats,
            sink,
            ..
        } = decode_bits(&decoded_bits, None, sink);

        assert!(transformed, "the block's transform ran");
        assert!(sink.fields.is_empty());
        assert_eq!(sink.rep_layout_tails, vec![(13, vec![0x55, 0x15])]);
        assert!(sink.stream_failures.is_empty());
        assert_eq!(stats.fields, 0);
        assert_eq!(stats.rpcs, 1);
        assert_eq!(stats.field_stream_failures, 0);
        assert_eq!(stats.rpc_stream_failures, 0);
        assert_eq!(stats.skipped_bits, 0);
    }

    #[test]
    fn a_wholly_preserved_rep_layout_tail_is_an_rpc_failure_not_field_loss() {
        let mut decoded_bits = vec![false]; // property checksum
        decoded_bits.int_packed(0); // RepLayout terminator
        decoded_bits.extend((0..13).map(|index| index % 2 == 0));
        let sink = TestSink {
            rep_layout_tail_outcome: Some(RepLayoutTailOutcome::Preserved {
                cause: StreamFailureCause::UnverifiedRepLayoutTail,
            }),
            ..TestSink::default()
        };
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, sink);

        assert_eq!(stats.field_stream_failures, 0);
        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(stats.unresolved_rpc_payloads_preserved, 1);
        assert_eq!(stats.skipped_bits, 13);
        assert_eq!(sink.stream_failures.len(), 1);
        let failure = sink.stream_failures[0];
        assert_eq!(failure.kind, StreamKind::Rpc);
        assert_eq!(failure.bit_count, 13);
        assert!(failure.payload_preserved);
        assert_eq!(sink.failures_before_payload, [1], "failure, then payload");
    }

    /// A RepLayout `Ok` walk that abandoned a tail, in the record for handle 0
    /// that began at bit 1, after the checksum.
    #[test]
    fn rep_layout_overrun_ok_path_is_a_stream_failure() {
        let mut decoded_bits = vec![false]; // property checksum
        decoded_bits.int_packed(1); // handle 0
        decoded_bits.int_packed(32); // overruns the remaining 8 bits
        decoded_bits.repeat(false, 8);
        assert_eq!(decoded_bits.len(), 25);
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        assert_eq!(stats.field_stream_failures, 1);
        assert_eq!(stats.skipped_bits, 24);
        assert_eq!(sink.stream_failures.len(), 1);
        let failure = &sink.stream_failures[0];
        assert_eq!(failure.kind, StreamKind::RepLayout);
        assert_eq!(failure.consumed_bits, 1);
        assert_eq!(failure.remaining_bits, 24);
        assert_eq!(failure.cause, StreamFailureCause::AbandonedTail);
        assert_eq!(
            (failure.record_handle, failure.record_offset),
            (Some(0), Some(1))
        );
        assert!(
            sink.rep_layout_tails.is_empty(),
            "a declared-length overrun is malformed RepLayout, not a chained tail"
        );
    }

    /// The `Err` arm charges the whole block, not the reader's remainder. Nine
    /// decoded bits: the checksum, then 0x01 -- an `IntPacked` chunk promising
    /// another the block lacks -- so the handle read fails with the window
    /// consumed and `bits_remaining() == 0`.
    #[test]
    fn rep_layout_err_at_the_exact_block_end_still_charges_the_block() {
        let mut decoded_bits = vec![false]; // property checksum
        decoded_bits.u8(0x01);
        assert_eq!(decoded_bits.len(), 9);

        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        assert_eq!(stats.field_stream_failures, 1);
        assert_eq!(stats.fields, 0, "no field was emitted");
        assert_eq!(sink.stream_failures.len(), 1);
        assert_eq!(sink.stream_failures[0].remaining_bits, 0);
        assert_eq!(sink.stream_failures[0].consumed_bits, 9);
        assert_eq!(stats.skipped_bits, 9, "a failure with no bits behind it");
    }

    #[test]
    fn rep_layout_read_error_keeps_an_already_emitted_prefix_counted() {
        let mut decoded_bits = vec![false]; // property checksum
        decoded_bits.int_packed(1); // handle 0
        decoded_bits.int_packed(0); // valid zero-bit payload
        decoded_bits.u8(0x01);
        assert_eq!(decoded_bits.len(), 25);
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        assert_eq!(sink.fields, vec![(0, 0)], "the valid prefix row remains");
        assert_eq!(stats.fields, 1, "NetStats matches the emitted prefix");
        assert_eq!(stats.field_stream_failures, 1);
        assert!(sink.rep_layout_tails.is_empty());
        assert_eq!(sink.stream_failures[0].cause, StreamFailureCause::ReadError);
        assert_eq!(sink.stream_failures[0].record_handle, None);
        assert_eq!(sink.stream_failures[0].record_offset, Some(17));
    }

    #[test]
    fn rep_layout_checksum_only_is_a_stream_failure() {
        // Given: a decoded window containing only the checksum bit.
        let decoded_bits = [false];

        // When: the real transform and stream walker process that window.
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        // Then: existing accounting charges the whole failed one-bit block.
        assert_eq!(stats.field_stream_failures, 1);
        assert_eq!(stats.skipped_bits, 1);
        assert_eq!(stats.fields, 0);
        assert_eq!(sink.stream_failures.len(), 1);
        let failure = &sink.stream_failures[0];
        assert_eq!(failure.kind, StreamKind::RepLayout);
        assert_eq!(failure.cause, StreamFailureCause::ReadError);
        assert_eq!(failure.consumed_bits, 1);
        assert_eq!(failure.remaining_bits, 0);
        assert_eq!(
            (failure.record_handle, failure.record_offset),
            (None, Some(1))
        );
        assert!(sink.rep_layout_tails.is_empty());
    }

    #[test]
    fn rep_layout_checksum_and_zero_handle_is_a_clean_stream() {
        // Given: checksum followed by the complete encoded zero terminator.
        let mut decoded_bits = vec![false];
        decoded_bits.int_packed(0);

        // When: the real transform and stream walker process all nine bits.
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        // Then: the valid empty stream produces neither failure nor lost bits.
        assert_eq!(stats.field_stream_failures, 0);
        assert_eq!(stats.skipped_bits, 0);
        assert_eq!(stats.fields, 0);
        assert!(sink.stream_failures.is_empty());
        assert!(sink.rep_layout_tails.is_empty());
    }

    #[test]
    fn rep_layout_missing_terminator_keeps_the_emitted_prefix_counted() {
        // Given: one complete field ending exactly at the block boundary.
        let mut decoded_bits = vec![false];
        decoded_bits.int_packed(1).int_packed(8).u8(0x5a);

        // When: the walker reaches EOF where the next handle should begin.
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, None, TestSink::default());

        // Then: the field stays emitted while failure accounting charges the block.
        assert_eq!(sink.fields, vec![(0, 8)]);
        assert_eq!(stats.fields, 1);
        assert_eq!(stats.field_stream_failures, 1);
        assert_eq!(stats.skipped_bits, 25);
        assert_eq!(sink.stream_failures.len(), 1);
        let failure = &sink.stream_failures[0];
        assert_eq!(failure.cause, StreamFailureCause::ReadError);
        assert_eq!(failure.consumed_bits, 25);
        assert_eq!(failure.remaining_bits, 0);
        assert_eq!(
            (failure.record_handle, failure.record_offset),
            (None, Some(25))
        );
        assert!(sink.rep_layout_tails.is_empty());
    }

    /// The RPC `Err` arm, same shape: one handle bit, then 0x01.
    /// `function_count` 2 walks the stream rather than taking the unresolved
    /// arm, which has its own accounting.
    #[test]
    fn class_net_cache_err_at_the_exact_block_end_still_charges_the_block() {
        let mut decoded_bits = Vec::new();
        decoded_bits.serialized_int(0, 2).u8(0x01); // one handle bit, then 0x01
        assert_eq!(decoded_bits.len(), 9);

        let Run { stats, sink, .. } = decode_bits(&decoded_bits, Some(2), TestSink::default());

        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(stats.rpcs, 0, "no RPC was emitted");
        assert_eq!(
            stats.unresolved_rpc_payloads_preserved, 0,
            "a walked stream that failed, not an unresolved group"
        );
        assert_eq!(sink.stream_failures.len(), 1);
        assert_eq!(sink.stream_failures[0].remaining_bits, 0);
        assert_eq!(sink.failures_before_payload, [1], "failure, then payload");
        assert_eq!(stats.skipped_bits, 9, "a failure with no bits behind it");
    }

    #[test]
    fn class_net_cache_read_error_keeps_an_already_emitted_prefix_counted() {
        let mut decoded_bits = Vec::new();
        decoded_bits.serialized_int(0, 2);
        decoded_bits.int_packed(0); // valid zero-bit RPC
        decoded_bits.serialized_int(0, 2);
        decoded_bits.u8(0x01);
        assert_eq!(decoded_bits.len(), 18);
        let Run { stats, sink, .. } = decode_bits(&decoded_bits, Some(2), TestSink::default());

        assert_eq!(sink.rpcs, vec![(0, 0)], "the valid prefix row remains");
        assert_eq!(stats.rpcs, 1, "NetStats matches the emitted prefix");
        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(sink.stream_failures[0].cause, StreamFailureCause::ReadError);
        assert_eq!(sink.stream_failures[0].record_handle, Some(0));
        assert_eq!(sink.stream_failures[0].record_offset, Some(9));
    }

    /// Frame `bits` as one whole bunch payload, straight through
    /// `frame_content_blocks`: packet 42, a reliable open of channel 5 for
    /// static actor 42, bunch ids 3 / 100 / 7.
    fn frame_bits(bits: &[bool]) -> (NetStats, TestSink) {
        let data = pack(bits);
        let mut payload = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();
        let (mut stats, mut sink, mut scratch) = (NetStats::default(), TestSink::default(), vec![]);
        let header = RawBunchHeader {
            packet_id: 42,
            ch_index: 5,
            b_open: true,
            b_reliable: true,
            ..Default::default()
        };
        let ctx = BunchContext {
            header: &header,
            ids: BunchIds {
                packet_id: 42,
                bunch_index_in_packet: 3,
                global_bunch_index: 100,
                channel_bunch_index: 7,
            },
            actor_net_guid: NetworkGuid(42),
            archetype_net_guid: NetworkGuid(0),
        };
        let mut stage = Stage {
            stats: &mut stats,
            channels: &mut ChannelTable::default(),
            transform: TransformVersion::V1301,
            scratch: &mut scratch,
        };
        framing::frame_content_blocks(&mut payload, &mut stage, &mut sink, &ctx);
        assert!(
            payload.at_end(),
            "an abort leaves nothing behind for a caller to misread"
        );
        (stats, sink)
    }

    /// A subobject header that runs out at its class GUID: hasRepLayout,
    /// isActor = 0, object GUID IntPacked(5), not stably named, not deleted --
    /// 12 bits, all read, and then the class GUID's first byte is not there.
    fn truncated_subobject_header() -> Vec<bool> {
        let mut bits = vec![true, false];
        bits.int_packed(5);
        bits.extend([false, false]);
        bits
    }

    /// A header that fails to read has consumed what it read: the abort charges
    /// those 12 bits, not the 0 left (see `framing::abort`).
    #[test]
    fn a_truncated_block_header_charges_the_bits_it_consumed() {
        let (stats, sink) = frame_bits(&truncated_subobject_header());

        assert_eq!(stats.content_block_framing_failures, 1);
        assert_eq!(stats.skipped_bits, 12, "the whole failed block, not 0");
        assert_eq!(stats.content_blocks, 0);
        assert!(sink.content_blocks.is_empty());
        use crate::stats::SkipReason;
        let ev = &stats.diagnostics[0];
        assert!(matches!(ev.reason, SkipReason::HeaderReadError));
        assert_eq!(ev.bits_skipped, 12);
        assert_eq!((ev.consumed_bits, ev.remaining_bits), (0, 0));
    }

    /// A `content_bits` continuation byte at the end of the bunch consumes 8
    /// bits before the read fails: with the 2-bit actor header, 10 bits lost.
    #[test]
    fn a_truncated_content_bits_field_charges_the_bits_it_consumed() {
        let mut bits = vec![true, true]; // hasRepLayout, isActor
        bits.u8(0x01); // content_bits: more follows

        let (stats, _) = frame_bits(&bits);

        assert_eq!(stats.content_block_framing_failures, 1);
        assert_eq!(stats.skipped_bits, 10);
        assert_eq!(stats.content_blocks, 0);
        use crate::stats::SkipReason;
        let ev = &stats.diagnostics[0];
        assert!(matches!(ev.reason, SkipReason::ContentBitsReadError));
        assert_eq!(ev.bits_skipped, 10);
        assert_eq!((ev.consumed_bits, ev.remaining_bits), (2, 0));
    }

    /// The charge starts at the failing block, not at the bunch: a block that
    /// framed before it keeps its bits out of the loss.
    #[test]
    fn a_framing_abort_does_not_recharge_blocks_that_framed() {
        let mut bits = empty_actor_block(); // 10 bits, frames cleanly
        bits.extend(truncated_subobject_header()); // 12 bits, fails

        let (stats, sink) = frame_bits(&bits);

        assert_eq!(stats.content_blocks, 1);
        assert_eq!(sink.content_blocks.len(), 1);
        assert_eq!(stats.content_block_framing_failures, 1);
        assert_eq!(stats.skipped_bits, 12, "22 bits, of which 10 framed");
        #[cfg(feature = "diagnostics")]
        assert_eq!(stats.diagnostics[0].block_index_in_bunch, 1);
    }

    /// A content-block overrun is a `DiagnosticEvent` with full context,
    /// charged from the block's first bit: 2 + 16 + 8 = 26 bits, while 8
    /// remained after the read.
    #[test]
    fn content_bits_overrun_emits_diagnostic() {
        use crate::stats::SkipReason;

        // hasRepLayout 0, isActor 1, content_bits IntPacked(999) in 16 bits,
        // then 8 bits: 999 overruns what is left.
        let mut bits = vec![false, true];
        bits.int_packed(999).repeat(false, 8);
        let (stats, _) = frame_bits(&bits);

        assert_eq!(stats.malformed_content_blocks, 1);
        assert_eq!(stats.skipped_bits, 26);
        assert_eq!(stats.diagnostics.len(), 1);
        let ev = &stats.diagnostics[0];
        assert_eq!(ev.packet_id, 42);
        assert_eq!(ev.bunch_index_in_packet, 3);
        assert_eq!(ev.global_bunch_index, 100);
        assert_eq!(ev.channel_bunch_index, 7);
        assert_eq!(ev.channel_index, 5);
        assert_eq!(ev.actor_net_guid, 42);
        assert_eq!(ev.block_index_in_bunch, 0);
        assert_eq!(ev.content_bits, Some(999));
        assert_eq!((ev.remaining_bits, ev.bits_skipped), (8, 26));
        assert_eq!(ev.consumed_bits, 18);
        assert!(
            ev.bunch_flags.b_open && ev.bunch_flags.b_reliable && !ev.bunch_flags.b_partial,
            "flags are snapshotted from the bunch header on the failure path"
        );
        assert!(matches!(
            ev.reason,
            SkipReason::ContentBitsOverrun {
                declared_content_bits: 999,
                available_bits: 8
            }
        ));
    }

    /// A diagnostic event names the archetype its channel's open read (9
    /// here), not a plausible 0, and both GUIDs' paths from the sink's cache.
    #[test]
    fn a_diagnostic_event_carries_the_channel_archetype() {
        use crate::stats::SkipReason;

        let mut open = guid(2); // dynamic actor GUID
        write_minimal_spawn_data(&mut open, 9); // archetype 9, not a controller
        let mut overrun = vec![false, true]; // ClassNetCache, isActor
        overrun.int_packed(999).repeat(false, 8); // declares far more than follows
        // One packet, so the overrun is its second bunch.
        let mut spec = BunchSpec {
            ch_index: 2,
            b_open: true,
            ..Default::default()
        };
        let mut both = Vec::new();
        write_bunch(&mut both, &spec, &open);
        spec.b_open = false;
        write_bunch(&mut both, &spec, &overrun);

        let (mut reader, mut sink) = (reader(), TestSink::default());
        sink.guid_paths.insert(2, "/Game/Actor".into());
        sink.guid_paths
            .insert(9, "/Game/Default__Archetype_C".into());
        reader.process_packet(&build_packet(&both), 0, &mut sink);

        let state = reader.channels[&2]
            .state
            .as_ref()
            .expect("channel 2 opened");
        assert_eq!(
            state.archetype_net_guid,
            NetworkGuid(9),
            "the open itself must have read archetype 9, or this proves nothing"
        );
        let stats = reader.stats();
        assert_eq!(stats.malformed_content_blocks, 1);
        assert_eq!(stats.diagnostics.len(), 1);
        let ev = &stats.diagnostics[0];
        assert!(matches!(
            ev.reason,
            SkipReason::ContentBitsOverrun {
                declared_content_bits: 999,
                available_bits: 8
            }
        ));
        assert_eq!(
            (ev.packet_id, ev.channel_index, ev.actor_net_guid),
            (0, 2, 2)
        );
        assert_eq!(ev.global_bunch_index, 1);
        assert_eq!(ev.bunch_index_in_packet, 1);
        assert_eq!(ev.channel_bunch_index, 2);
        assert_eq!(ev.archetype_net_guid, 9);
        assert_eq!(ev.actor_path.as_deref(), Some("/Game/Actor"));
        assert_eq!(ev.class_path.as_deref(), Some("/Game/Default__Archetype_C"));
    }

    // --- the controller's net-player-index byte ---

    /// Write a dynamic actor's spawn block: archetype, level, and the four
    /// optional transforms (location, rotation, scale, velocity) all absent.
    /// Velocity is read unconditionally; omitting it shifts every later bit.
    fn write_minimal_spawn_data(bits: &mut Vec<bool>, archetype_guid: u32) {
        bits.int_packed(archetype_guid);
        bits.int_packed(0); // level GUID 0: not valid, returns early
        bits.extend([false; 4]); // location, rotation, scale, velocity absent
    }

    /// A controller's opening bunch carries the net-player-index byte between
    /// the spawn block and the first content-block header, consumed only when
    /// the path cache resolves the archetype to the controller
    /// (BaseReplayController, or BaseJanusController on 12.01-12.06).
    /// Without the path the byte shifts the header, and the property block
    /// (`PlayerState`, `SpawnLocation`) is never walked as the actor's.
    #[test]
    fn controller_property_block_is_reached() {
        let mut payload = guid(2); // dynamic: a spawn block follows
        write_minimal_spawn_data(&mut payload, 9);
        payload.repeat(false, 8); // the net-player-index byte
        payload.extend(empty_actor_block());
        let packet = build_open_bunch_packet(2, &payload);

        for (branch, archetype_path, reached) in [
            ("13.01", Some("Default__BaseReplayController_C"), true),
            ("12.01", Some("Default__BaseJanusController_C"), true),
            ("13.01", None, false),
        ] {
            let mut sink = TestSink::default();
            // The path arrived with exports before this bunch.
            sink.guid_paths
                .extend(archetype_path.map(|path| (9, path.to_owned())));
            let mut reader =
                ReplicationReader::new(&format!("++Ares-Core+release-{branch}")).unwrap();
            reader.process_packet(&packet, 0, &mut sink);

            let actor_blocks = sink
                .content_blocks
                .iter()
                .filter(|block| block.is_actor && block.has_rep_layout)
                .count();
            assert_eq!(reader.stats().actor_opens, 1, "{branch} {archetype_path:?}");
            if reached {
                assert_eq!(
                    reader.stats().skipped_bits,
                    0,
                    "{branch}: nothing abandoned"
                );
                assert_eq!(
                    (sink.content_blocks.len(), actor_blocks),
                    (1, 1),
                    "{branch}: exactly the property block"
                );
            } else {
                assert_eq!(actor_blocks, 0, "without the path the header misframes");
            }
        }
    }

    /// A non-controller dynamic actor has no net-player-index byte: the header
    /// follows the spawn data directly, and an unconditional byte read would
    /// eat its first eight bits.
    #[test]
    fn non_controller_dynamic_actor_skips_net_player_index_byte() {
        let mut payload = guid(2);
        write_minimal_spawn_data(&mut payload, 9); // no path in the cache
        payload.extend(empty_actor_block());
        let (reader, sink) = run_packets(&[build_open_bunch_packet(2, &payload)]);

        assert_eq!(reader.stats().actor_opens, 1);
        assert_eq!(reader.stats().skipped_bits, 0);
        assert_eq!(sink.content_blocks.len(), 1);
        assert!(sink.content_blocks[0].has_rep_layout);
        assert!(sink.content_blocks[0].is_actor);
    }

    // --- partial reassembly has exactly one authority ---
    //
    // The packet reader keeps an advisory partial tracker. Were its verdicts
    // to reach the accumulator or the rejected-row condition, a disagreement
    // would change what is reassembled and preserved with `partial_errors` at
    // 0. Each test below is one such disagreement.

    fn rejected_rows(sink: &TestSink) -> Vec<(&'static str, PartialPayloadReason, usize)> {
        sink.rejected_partials
            .iter()
            .map(|row| (row.kind, row.reason, row.bit_count))
            .collect()
    }

    fn rejected_bits(sink: &TestSink) -> u64 {
        sink.rejected_partials
            .iter()
            .map(|row| row.bit_count as u64)
            .sum()
    }

    /// A partial that is both initial and final is a whole bunch every time,
    /// and frames exactly as the same payloads sent unfragmented.
    #[test]
    fn an_initial_final_partial_is_a_whole_bunch_every_time() {
        let (reader, sink) = run_packets(&[
            partial_packet(true, true, true, &static_actor(3)),
            partial_packet(false, true, true, &empty_actor_block()),
        ]);
        // The same two payloads sent unfragmented are the reference.
        let (reference, reference_sink) = run_packets(&[open_on_two(3), later_block_on_two()]);
        assert_eq!(
            reference_sink.content_blocks.len(),
            2,
            "the reference must itself frame both bunches, or the comparison proves nothing"
        );

        let stats = reader.stats();
        assert_eq!(stats.partial_completed, 2, "both whole bunches complete");
        assert_eq!(stats.partial_errors, 0);
        assert!(
            sink.rejected_partials.is_empty(),
            "nothing was rejected: {:?}",
            rejected_rows(&sink)
        );
        assert_eq!(
            sink.content_blocks.len(),
            reference_sink.content_blocks.len()
        );
        assert_eq!(stats.actor_opens, reference.stats().actor_opens);
        assert_eq!(stats.skipped_bits, reference.stats().skipped_bits);
    }

    /// An unaligned continuation is rejected by the accumulator alone (the
    /// packet reader keeps its assembly). What follows -- a whole bunch, or an
    /// initial and final -- is still reassembled, and never also a rejected
    /// row: every rejected bit is preserved exactly once.
    #[test]
    fn what_follows_an_unaligned_fragment_is_still_processed() {
        let whole = vec![partial_packet(true, true, true, &static_actor(3))];
        let split = vec![
            partial_packet(true, true, false, &guid(3)),
            partial_packet(false, false, true, &empty_actor_block()),
        ];
        for tail in [whole, split] {
            let mut packets = vec![
                partial_packet(true, true, false, &guid(3)),
                partial_packet(false, false, false, &[true; 5]),
            ];
            packets.extend(tail);
            let (reader, sink) = run_packets(&packets);

            let stats = reader.stats();
            assert_eq!(stats.actor_opens, 1, "the bunch after it opens its actor");
            assert_eq!(sink.content_blocks.len(), 1);
            assert_eq!(stats.partial_completed, 1);
            assert_eq!(stats.partial_errors, 1, "only the unaligned continuation");
            assert_eq!(stats.partial_non_byte_aligned, 1);
            assert_eq!(stats.partial_overlapping_initial, 0);
            assert_eq!(
                rejected_rows(&sink),
                vec![
                    (
                        "accumulated_payload",
                        PartialPayloadReason::NonByteAlignedFragment,
                        8
                    ),
                    (
                        "current_fragment",
                        PartialPayloadReason::NonByteAlignedFragment,
                        5
                    ),
                ]
            );
            assert_eq!(
                sink.rejected_partials[0].payload,
                vec![6],
                "the displaced initial keeps its bytes"
            );
            assert_eq!(rejected_bits(&sink), stats.skipped_bits);
        }
    }

    /// A zero-bit final that overlaps an assembly is an error, not a
    /// completion, and leaves no complete-but-untaken state behind.
    #[test]
    fn a_zero_bit_overlapping_final_is_not_a_completion() {
        let (reader, sink) = run_packets(&[
            partial_packet(true, true, false, &guid(3)),
            partial_packet(false, true, true, &[]),
        ]);

        let stats = reader.stats();
        assert_eq!(stats.partial_completed, 0);
        assert_eq!(stats.partial_errors, 1);
        assert_eq!(stats.partial_overlapping_initial, 1);
        assert_eq!(
            reader.accumulator.active_count(),
            0,
            "no complete-but-untaken state may linger"
        );
        assert_eq!(sink.rejected_partials[0].rejection_packet_id, Some(1));
        assert_eq!(
            rejected_rows(&sink),
            vec![(
                "accumulated_payload",
                PartialPayloadReason::OverlappingInitial,
                8
            )]
        );
        assert_eq!(rejected_bits(&sink), stats.skipped_bits);
    }

    /// A refused initial fragment is one rejected row: the empty assembly
    /// started for it is not also a 0-bit `accumulated_payload` row.
    #[test]
    fn a_refused_initial_fragment_is_one_rejected_row() {
        let (reader, sink) = run_packets(&[partial_packet(false, true, false, &[true; 5])]);

        let stats = reader.stats();
        assert_eq!(stats.partial_errors, 1);
        assert_eq!(stats.partial_non_byte_aligned, 1);
        assert_eq!(
            rejected_rows(&sink),
            vec![(
                "current_fragment",
                PartialPayloadReason::NonByteAlignedFragment,
                5
            )]
        );
        assert_eq!(stats.skipped_bits, 5);
        assert_eq!(rejected_bits(&sink), stats.skipped_bits);
        assert_eq!(
            reader.accumulator.active_count(),
            0,
            "the refused initial leaves no assembly behind"
        );

        // Over a buffered assembly, the assembly it replaced is still a row,
        // under the cause that displaced it; only the empty one is not.
        let (reader, sink) = run_packets(&[
            partial_packet(true, true, false, &guid(3)),
            partial_packet(false, true, false, &[true; 5]),
        ]);
        assert_eq!(
            rejected_rows(&sink),
            vec![
                (
                    "accumulated_payload",
                    PartialPayloadReason::OverlappingInitial,
                    8
                ),
                (
                    "current_fragment",
                    PartialPayloadReason::NonByteAlignedFragment,
                    5
                ),
            ]
        );
        assert_eq!(sink.rejected_partials[0].payload, vec![6]);
        assert_eq!(rejected_bits(&sink), reader.stats().skipped_bits);
    }

    // --- the preservation hand-off: pipeline -> `on_rejected_partial` ---
    //
    // Each test pins one hand-off by its bytes, reason and counters.

    /// `finish_with_sink` is what the driver calls, not `finish`. It must count
    /// the unfinished assembly and hand the sink its exact bytes, once.
    #[test]
    fn finish_with_sink_preserves_an_unfinished_assembly_exactly() {
        let (mut reader, mut sink) = run_packets(&[partial_packet(true, true, false, &guid(3))]);
        reader.finish_with_sink(&mut sink);

        let stats = reader.stats();
        assert_eq!(
            (stats.unfinished_partials, stats.unfinished_partial_bits),
            (1, 8)
        );
        assert_eq!(
            rejected_rows(&sink),
            vec![("accumulated_payload", PartialPayloadReason::EndOfStream, 8)]
        );
        let row = &sink.rejected_partials[0];
        assert_eq!(row.payload, vec![6]);
        assert_eq!(row.rejection_packet_id, None);
        assert_eq!(
            row.header.packet_id, 0,
            "the row names the fragment's packet"
        );

        reader.finish_with_sink(&mut sink);
        assert_eq!(
            sink.rejected_partials.len(),
            1,
            "a second call preserves nothing twice"
        );
    }

    /// A partial refused at the channel-state guard keeps its own reason (not
    /// a missing initial) and still counts in `partial_bunches`, although it
    /// returns before `process_bunch`, where the others are counted.
    #[test]
    fn a_partial_refused_for_channel_state_keeps_that_reason() {
        let mut reader = reader();
        reader.packet_reader = RawPacketReader::with_max_channels(0);
        let mut sink = TestSink::default();
        let packet = partial_packet(false, true, false, &guid(3));
        reader.process_packet(&packet, 0, &mut sink);

        assert_eq!(reader.stats().channel_state_limit_failures, 1);
        assert_eq!(reader.stats().partial_bunches, 1, "still attempted");
        assert_eq!(
            rejected_rows(&sink),
            vec![(
                "current_fragment",
                PartialPayloadReason::ChannelStateLimit,
                8
            )]
        );
        assert_eq!(sink.rejected_partials[0].payload, vec![6]);
        assert_eq!(reader.stats().skipped_bits, 8);
    }

    /// Closing a channel while a partial is buffered preserves the buffered
    /// bytes under `ChannelClosed`, charged to the closing packet.
    #[test]
    fn closing_a_channel_mid_partial_preserves_the_buffered_payload() {
        let (reader, sink) = run_packets(&[
            partial_packet(true, true, false, &guid(3)),
            close_on_two(false),
        ]);

        let stats = reader.stats();
        assert_eq!(stats.partial_channel_close, 1);
        assert_eq!(stats.partial_errors, 1);
        assert_eq!(
            rejected_rows(&sink),
            vec![(
                "accumulated_payload",
                PartialPayloadReason::ChannelClosed,
                8
            )]
        );
        assert_eq!(sink.rejected_partials[0].payload, vec![6]);
        assert_eq!(sink.rejected_partials[0].rejection_packet_id, Some(1));
        assert_eq!(stats.skipped_bits, 8);
    }

    /// A dormant close keeps the channel's buffered partial (see
    /// `close_channel`): the continuation completes it and nothing is lost.
    /// The fragments are unreliable so the close does not break the sequence.
    #[test]
    fn a_dormant_close_mid_partial_keeps_the_buffered_payload() {
        let fragment = |initial, last| BunchSpec {
            ch_index: 2,
            b_open: initial,
            unreliable: true,
            b_partial: true,
            b_partial_initial: initial,
            b_partial_final: last,
            ..Default::default()
        };
        let dormant_close = BunchSpec {
            ch_index: 2,
            b_close: true,
            dormant: true,
            ..Default::default()
        };
        let mut first = Vec::new();
        write_bunch(&mut first, &fragment(true, false), &guid(3));
        write_bunch(&mut first, &dormant_close, &[]);
        let (reader, sink) = run_packets(&[
            build_packet(&first),
            build_bunch_packet(&fragment(false, true), &empty_actor_block()),
        ]);

        let stats = reader.stats();
        assert_eq!(stats.partial_channel_close, 0);
        assert_eq!(stats.partial_errors, 0);
        assert_eq!(stats.partial_completed, 1);
        assert_eq!(stats.skipped_bits, 0);
        assert!(rejected_rows(&sink).is_empty());
        assert_eq!(stats.content_blocks, 1, "the completed bunch is framed");
    }
}
