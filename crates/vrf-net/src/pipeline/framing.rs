//! Content-block framing: the per-block hot loop.
//!
//! Everything here runs per block (rates:
//! docs/PERFORMANCE_NOTES.md#measured-rates-reference-replay-02d4d478), so
//! work that can be hoisted out or made conditional on a failure path
//! belongs elsewhere.
//!
//! # Failure policy
//!
//! - the block header or its bit count does not read, or the bit count
//!   overruns the bunch: the rest of the bunch is abandoned ([`abort`]);
//! - the payload decoded but its inner stream did not walk: only that block's
//!   bits are lost ([`decode_and_walk`]), and the sink is told which class.

use vrf_bitio::BitReader;
use vrf_transform::TransformVersion;

use crate::bunch::RawBunchHeader;
use crate::content::{self, ContentBlockHeader};
use crate::error::NetError;
use crate::field::{self, RepLayoutRemainder};
use crate::stats::NetStats;
use crate::types::NetworkGuid;

use crate::stats::{BunchFlagSnapshot, ContentBlockHeaderSnapshot, DiagnosticEvent, SkipReason};

use super::{
    ContentBlockContext, RepLayoutTailOutcome, ReplicationSink, Stage, StreamFailure,
    StreamFailureCause, StreamKind,
};

/// Per-bunch context. Only the channel (`header.ch_index`) and the actor are
/// read on the success path; the rest feeds diagnostic events.
pub(super) struct BunchContext<'a> {
    pub header: &'a RawBunchHeader,
    pub ids: super::BunchIds,
    pub actor_net_guid: NetworkGuid,
    /// `NetworkGuid(0)` for a static actor, whose open carries no spawn block.
    pub archetype_net_guid: NetworkGuid,
}

/// Why a block produced a diagnostic event, with what it had read by then.
#[derive(Clone, Copy)]
enum Failure<'h> {
    HeaderRead,
    ContentBitsRead(&'h ContentBlockHeader),
    /// The declared content bits overran the bunch.
    Overrun(&'h ContentBlockHeader, u32),
    /// The payload transform failed; framing cannot reach it on real input
    /// (see `SkipReason::ParseFailure`).
    Parse(&'h ContentBlockHeader, u32),
}

/// Walk a bunch payload as a sequence of content blocks.
pub(super) fn frame_content_blocks(
    payload: &mut BitReader<'_>,
    stage: &mut Stage<'_>,
    sink: &mut dyn ReplicationSink,
    ctx: &BunchContext<'_>,
) {
    let (ch_index, actor_net_guid) = (ctx.header.ch_index, ctx.actor_net_guid);
    let mut block_index: u32 = 0;

    while !payload.at_end() {
        let block_start = payload.position();
        let abort_at =
            |payload: &mut BitReader<'_>, stats: &mut NetStats, sink, consumed, failure| {
                let at = (ctx, block_index, block_start);
                abort(payload, stats, sink, at, consumed, failure);
            };

        let Ok(header) = content::read_content_block_header(payload, actor_net_guid, sink) else {
            return abort_at(payload, stage.stats, sink, block_start, Failure::HeaderRead);
        };

        if header.is_deleted {
            sink.on_deleted_block(ch_index, actor_net_guid, &header);
            stage.stats.deleted_blocks += 1;
            stage.stats.content_blocks += 1;
            block_index += 1;
            continue;
        }

        let bits_start = payload.position();
        let Ok(content_bits) = payload.read_int_packed() else {
            let failure = Failure::ContentBitsRead(&header);
            return abort_at(payload, stage.stats, sink, bits_start, failure);
        };
        if u64::from(content_bits) > payload.bits_remaining() {
            let consumed = payload.position();
            let failure = Failure::Overrun(&header, content_bits);
            return abort_at(payload, stage.stats, sink, consumed, failure);
        }

        let context = ContentBlockContext {
            initial_packet_id: ctx.header.packet_id,
            completion_packet_id: ctx.ids.packet_id,
            bunch_index: ctx.ids.global_bunch_index,
            block_index,
        };
        let function_count =
            sink.on_content_block_with_context(ch_index, actor_net_guid, &header, &context);
        stage.stats.content_blocks += 1;
        if header.has_rep_layout {
            stage.stats.rep_layout_blocks += 1;
        } else {
            stage.stats.class_net_cache_blocks += 1;
        }
        let payload_start = payload.position();
        let function_count = (!header.has_rep_layout).then_some(function_count);
        if content_bits != 0
            && !decode_and_walk(
                payload,
                content_bits as usize,
                actor_net_guid,
                function_count,
                stage,
                sink,
            )
        {
            // Counted already; this adds the event.
            let remaining = payload.len_bits() - payload_start;
            let failure = Failure::Parse(&header, content_bits);
            let skipped = u64::from(content_bits);
            let at = (ctx, block_index, payload_start);
            record(stage.stats, sink, at, remaining, skipped, failure);
        }
        block_index += 1;
    }
}

/// Where an event happened: the bunch, the block index within it, and a bit
/// position -- `block_start` in `abort`, the event's `consumed_bits` in `record`.
type At<'c, 'h> = (&'c BunchContext<'h>, u32, u64);

/// Abandon the rest of the bunch after a framing failure in the block that
/// began at `block_start`; `consumed` is the event's `consumed_bits`.
///
/// The charge runs from `block_start` to the window's end, never
/// `bits_remaining()`: a read that expires at the end has consumed its bits
/// and leaves 0 remaining, so it would charge nothing while a failure counter
/// moved. Blocks that framed earlier keep their bits.
fn abort(
    payload: &mut BitReader<'_>,
    stats: &mut NetStats,
    sink: &dyn ReplicationSink,
    (ctx, block_index, block_start): At<'_, '_>,
    consumed: u64,
    failure: Failure<'_>,
) {
    let remaining = payload.bits_remaining();
    let abandoned = payload.len_bits() - block_start;
    stats.skipped_bits += abandoned;
    if let Failure::Overrun(..) = failure {
        stats.malformed_content_blocks += 1;
    } else {
        stats.content_block_framing_failures += 1;
    }
    let at = (ctx, block_index, consumed);
    record(stats, sink, at, remaining, abandoned, failure);
    payload.skip_remaining();
}

/// Record one diagnostic event (built only if the capped log keeps it), with
/// the actor's and archetype's paths as the sink's GUID cache holds them now.
fn record(
    stats: &mut NetStats,
    sink: &dyn ReplicationSink,
    (ctx, block_index, consumed_bits): At<'_, '_>,
    remaining_bits: u64,
    bits_skipped: u64,
    failure: Failure<'_>,
) {
    stats.record_diagnostic(|| {
        let (reason, block, content_bits) = match failure {
            Failure::HeaderRead => (SkipReason::HeaderReadError, None, None),
            Failure::ContentBitsRead(block) => {
                (SkipReason::ContentBitsReadError, Some(block), None)
            }
            Failure::Overrun(block, bits) => (
                SkipReason::ContentBitsOverrun {
                    declared_content_bits: bits,
                    available_bits: remaining_bits,
                },
                Some(block),
                Some(bits),
            ),
            Failure::Parse(block, bits) => (SkipReason::ParseFailure, Some(block), Some(bits)),
        };
        let h = ctx.header;
        let path = |guid: NetworkGuid| sink.path_for_guid(guid.0).map(str::to_owned);
        DiagnosticEvent {
            reason,
            packet_id: h.packet_id,
            bunch_index_in_packet: ctx.ids.bunch_index_in_packet,
            global_bunch_index: ctx.ids.global_bunch_index,
            channel_bunch_index: ctx.ids.channel_bunch_index,
            channel_index: h.ch_index,
            actor_net_guid: ctx.actor_net_guid.0,
            actor_path: path(ctx.actor_net_guid),
            archetype_net_guid: ctx.archetype_net_guid.0,
            class_path: path(ctx.archetype_net_guid),
            bunch_flags: BunchFlagSnapshot {
                b_open: h.b_open,
                b_close: h.b_close,
                b_reliable: h.b_reliable,
                b_partial: h.b_partial,
                b_partial_initial: h.b_partial_initial,
                b_partial_final: h.b_partial_final,
                b_has_package_map_exports: h.b_has_package_map_exports,
                b_has_must_be_mapped_guids: h.b_has_must_be_mapped_guids,
                b_dormant: h.b_dormant,
            },
            payload_bit_count: h.payload_bit_count,
            consumed_bits,
            remaining_bits,
            content_block_header: block.map(ContentBlockHeaderSnapshot::from),
            content_bits,
            block_index_in_bunch: block_index,
            bits_skipped,
        }
    });
}

/// Decode a block payload into `stage.scratch` and return its byte length, or
/// `None` when the transform failed (already counted). The scratch tail past
/// `byte_count` is stale: slice to it before anything reaches the sink.
fn decode_into_scratch(
    payload: &mut BitReader<'_>,
    bit_count: usize,
    actor_net_guid: NetworkGuid,
    stage: &mut Stage<'_>,
) -> Option<usize> {
    let byte_count = TransformVersion::output_byte_count(bit_count);
    if stage.scratch.len() < byte_count {
        stage.scratch.resize(byte_count, 0);
    }

    let seed = vrf_transform::seed_for(bit_count, actor_net_guid.0);
    if stage
        .transform
        .decode_from(payload, bit_count, seed, stage.scratch)
        .is_err()
    {
        stage.stats.transform_failures += 1;
        stage.stats.skipped_bits += bit_count as u64;
        return None;
    }
    Some(byte_count)
}

/// Decode one block payload and walk it: RepLayout when `function_count` is
/// `None`, ClassNetCache with that count otherwise. Returns `false` only for
/// a failed transform, already counted; the caller, holding the block's
/// header and position, records its `ParseFailure` event.
///
/// A stream `Err` charges the whole block, never `bits_remaining()` (see
/// [`abort`]), as [`NetStats::lost_content_blocks`] counts it: records
/// emitted before it are counted *and* charged, erring loud. An `Ok` walk
/// charges only the tail it abandoned.
pub(super) fn decode_and_walk(
    payload: &mut BitReader<'_>,
    bit_count: usize,
    actor_net_guid: NetworkGuid,
    function_count: Option<u32>,
    stage: &mut Stage<'_>,
    sink: &mut dyn ReplicationSink,
) -> bool {
    let Some(byte_count) = decode_into_scratch(payload, bit_count, actor_net_guid, stage) else {
        return false;
    };
    let block = &stage.scratch[..byte_count];
    let mut reader = BitReader::with_bit_len(block, bit_count as u64)
        .expect("scratch holds ceil(bit_count / 8) bytes");
    let detailed = sink.wants_stream_failure_details();
    let mut walk = field::WalkContext::default();
    let context = detailed.then_some(&mut walk);
    let (kind, walked) = match function_count {
        None => {
            let (count, walked) = field::parse_rep_layout_content_block(&mut reader, sink, context);
            stage.stats.fields += u64::from(count);
            if let Ok(RepLayoutRemainder::ClassNetCache(tail_bits)) = walked {
                rep_layout_tail(
                    reader,
                    tail_bits,
                    actor_net_guid,
                    detailed,
                    stage.stats,
                    sink,
                );
                return true;
            }
            (
                StreamKind::RepLayout,
                walked.map(RepLayoutRemainder::bit_count),
            )
        }
        Some(function_count) => {
            let (count, walked) = field::parse_class_net_cache_content_block(
                &mut reader,
                function_count,
                sink,
                context,
            );
            stage.stats.rpcs += u64::from(count);
            (StreamKind::Rpc, walked)
        }
    };
    let whole = bit_count as u64;
    let (remaining, cause, charge) = match walked {
        Ok(0) => return true,
        Ok(abandoned) => (abandoned, StreamFailureCause::AbandonedTail, abandoned),
        Err(NetError::UnresolvedFunctionCount) => (
            reader.bits_remaining(),
            StreamFailureCause::UnresolvedFunctionCount,
            whole,
        ),
        Err(_) => (
            reader.bits_remaining(),
            StreamFailureCause::ReadError,
            whole,
        ),
    };
    let preserved = cause == StreamFailureCause::UnresolvedFunctionCount;
    let failure = StreamFailure {
        kind,
        actor_net_guid,
        bit_count: bit_count as u32,
        function_count: function_count.unwrap_or(0),
        consumed_bits: whole - remaining,
        remaining_bits: remaining,
        cause,
        record_handle: walk.last_handle,
        record_offset: detailed.then_some(walk.record_offset),
        payload_preserved: preserved,
    };
    report(stage.stats, sink, failure, charge);
    if preserved {
        sink.on_unresolved_class_net_cache_payload(failure, block);
    } else if detailed {
        sink.on_stream_failure_payload(failure, block);
    }
    true
}

/// Hand the ClassNetCache tail after a RepLayout terminator to the sink, and
/// report it as an RPC stream failure unless the sink decoded it.
fn rep_layout_tail(
    reader: BitReader<'_>,
    tail_bits: u64,
    actor_net_guid: NetworkGuid,
    detailed: bool,
    stats: &mut NetStats,
    sink: &mut dyn ReplicationSink,
) {
    let (cause, payload_preserved) =
        match sink.on_rep_layout_tail(actor_net_guid, tail_bits as u32, reader.clone()) {
            RepLayoutTailOutcome::Decoded { rpc_count } => {
                stats.rpcs += u64::from(rpc_count);
                return;
            }
            RepLayoutTailOutcome::Preserved { cause } => (cause, true),
            RepLayoutTailOutcome::Unpreserved { cause } => (cause, false),
        };
    let failure = StreamFailure {
        kind: StreamKind::Rpc,
        actor_net_guid,
        bit_count: tail_bits as u32,
        function_count: 0,
        consumed_bits: 0,
        remaining_bits: tail_bits,
        cause,
        record_handle: None,
        record_offset: None,
        payload_preserved,
    };
    report(stats, sink, failure, tail_bits);
    if detailed {
        sink.on_rep_layout_tail_failure_payload(failure, reader);
    }
}

/// Tell the sink about a stream failure and move its counters; `charge` is
/// what `skipped_bits` takes. Any payload callback comes after this.
fn report(
    stats: &mut NetStats,
    sink: &mut dyn ReplicationSink,
    failure: StreamFailure,
    charge: u64,
) {
    sink.on_stream_failure(failure);
    match failure.kind {
        StreamKind::RepLayout => stats.field_stream_failures += 1,
        StreamKind::Rpc => stats.rpc_stream_failures += 1,
    }
    stats.unresolved_rpc_payloads_preserved += u64::from(failure.payload_preserved);
    stats.skipped_bits += charge;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::SkipReason;

    /// The `ParseFailure` event names the block whose transform failed, where
    /// it sat and what it charged. Framing cannot reach this helper on real
    /// input (see `SkipReason::ParseFailure`), so it is pinned directly.
    #[test]
    fn a_parse_failure_event_names_the_block_it_skipped() {
        let bunch = RawBunchHeader {
            packet_id: 7,
            ch_index: 5,
            b_reliable: true,
            payload_bit_count: 200,
            ..Default::default()
        };
        let ctx = BunchContext {
            header: &bunch,
            ids: super::super::BunchIds {
                packet_id: 7,
                bunch_index_in_packet: 1,
                global_bunch_index: 11,
                channel_bunch_index: 4,
            },
            actor_net_guid: NetworkGuid(2),
            archetype_net_guid: NetworkGuid(9),
        };
        let block = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: true,
            ..Default::default()
        };
        let mut stats = NetStats::default();
        let mut sink = super::super::tests::TestSink::default();
        sink.guid_paths
            .insert(9, "/Game/Default__Archetype_C".into());

        record(
            &mut stats,
            &sink,
            (&ctx, 3, 120),
            80,
            33,
            Failure::Parse(&block, 33),
        );

        assert_eq!(stats.diagnostics.len(), 1);
        assert_eq!(stats.diagnostics_dropped, 0);
        let ev = &stats.diagnostics[0];
        assert!(matches!(ev.reason, SkipReason::ParseFailure));
        assert_eq!(
            (
                ev.packet_id,
                ev.bunch_index_in_packet,
                ev.global_bunch_index,
                ev.channel_bunch_index
            ),
            (7, 1, 11, 4)
        );
        assert_eq!(
            (ev.channel_index, ev.actor_net_guid, ev.archetype_net_guid),
            (5, 2, 9)
        );
        let paths = (ev.actor_path.as_deref(), ev.class_path.as_deref());
        assert_eq!(paths, (None, Some("/Game/Default__Archetype_C")));
        assert_eq!(ev.block_index_in_bunch, 3);
        assert_eq!((ev.consumed_bits, ev.remaining_bits), (120, 80));
        assert_eq!(ev.content_bits, Some(33));
        assert_eq!(
            ev.bits_skipped, 33,
            "the block's declared length, which is what skipped_bits was charged"
        );
        let snapshot = ev
            .content_block_header
            .as_ref()
            .expect("the block header read before the failure");
        assert!(snapshot.has_rep_layout && snapshot.is_actor);
        assert!(ev.bunch_flags.b_reliable && !ev.bunch_flags.b_open);
        assert_eq!(ev.payload_bit_count, 200);
    }
}
