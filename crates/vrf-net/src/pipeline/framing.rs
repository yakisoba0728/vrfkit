//! Content-block framing: the per-block hot loop.
//!
//! This is where the replay's bulk goes -- 608 020 content blocks on the
//! reference replay against 530 401 bunches and 2 028 actor opens. Everything
//! in this module runs per block, so anything that can be hoisted out of it or
//! made conditional on a failure path belongs somewhere else.
//!
//! The loop is: read a block header, read its declared payload bit count,
//! hand the header to the sink (which answers with a function count for
//! ClassNetCache blocks), then decode the payload and walk the field or RPC
//! stream inside it.
//!
//! # Failure policy
//!
//! Three depths can fail and each is counted separately, because they mean
//! different things:
//!
//! - the block header or its bit count could not be read -- the rest of the
//!   bunch is unframeable and is abandoned;
//! - the declared bit count overruns the bunch -- likewise;
//! - the payload decoded but its inner stream did not walk -- only that one
//!   block's bits are lost, and the sink is told which class it was.

use vrf_bitio::BitReader;
use vrf_transform::TransformVersion;

use crate::bunch::RawBunchHeader;
use crate::content::{self, ContentBlockHeader};
use crate::error::NetError;
use crate::field;
use crate::stats::NetStats;
use crate::types::NetworkGuid;

use super::{ReplicationSink, Stage, StreamFailure, StreamKind};

/// Per-bunch context carried through content-block framing for diagnostics.
///
/// This is not part of the hot path -- it is only read when a diagnostic event
/// is emitted (malformed/skipped). It holds a borrow of the bunch header rather
/// than a copy of its flags so that the flag snapshot is built only on the
/// failure path; building one per bunch cost nine bool copies plus a `Vec`
/// clone per event for a field read on well under one bunch in a thousand.
///
/// Both fields are diagnostics-only; see [`super::BunchIds`] for why they are
/// still threaded through a build that has diagnostics switched off.
#[cfg_attr(not(feature = "diagnostics"), allow(dead_code))]
pub(super) struct BunchContext<'a> {
    pub header: &'a RawBunchHeader,
    pub ids: super::BunchIds,
}

/// Walk a bunch payload as a sequence of content blocks.
pub(super) fn frame_content_blocks(
    payload: &mut BitReader<'_>,
    ch_index: u32,
    actor_net_guid: NetworkGuid,
    stage: &mut Stage<'_>,
    sink: &mut dyn ReplicationSink,
    ctx: &BunchContext<'_>,
) {
    let mut block_index: u32 = 0;

    while !payload.at_end() {
        let consumed_before_header = payload.position();

        let Ok(header) = content::read_content_block_header(payload, actor_net_guid, sink) else {
            let remaining = payload.bits_remaining();
            stage.stats.skipped_bits += remaining;
            diagnostics::header_read_error(
                stage.stats,
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                consumed_before_header,
                remaining,
            );
            payload.skip_remaining();
            return;
        };

        if header.is_deleted {
            sink.on_deleted_block(ch_index, actor_net_guid, &header);
            stage.stats.deleted_blocks += 1;
            stage.stats.content_blocks += 1;
            block_index += 1;
            continue;
        }

        // Read content payload bit count
        let consumed_before_bits_read = payload.position();
        let Ok(content_bits) = payload.read_int_packed() else {
            let remaining = payload.bits_remaining();
            stage.stats.skipped_bits += remaining;
            diagnostics::content_bits_read_error(
                stage.stats,
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                consumed_before_bits_read,
                remaining,
                &header,
            );
            payload.skip_remaining();
            return;
        };

        if u64::from(content_bits) > payload.bits_remaining() {
            let remaining = payload.bits_remaining();
            stage.stats.malformed_content_blocks += 1;
            stage.stats.skipped_bits += remaining;
            diagnostics::content_bits_overrun(
                stage.stats,
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                payload.position(),
                remaining,
                &header,
                content_bits,
            );
            payload.skip_remaining();
            return;
        }

        let function_count = sink.on_content_block(ch_index, actor_net_guid, &header);

        stage.stats.content_blocks += 1;
        block_index += 1;

        if header.has_rep_layout {
            stage.stats.rep_layout_blocks += 1;
            if content_bits != 0 {
                decode_and_parse_rep_layout(
                    payload,
                    content_bits as usize,
                    actor_net_guid,
                    stage,
                    sink,
                );
            }
        } else {
            stage.stats.class_net_cache_blocks += 1;
            if content_bits != 0 {
                decode_and_parse_class_net_cache(
                    payload,
                    content_bits as usize,
                    actor_net_guid,
                    function_count,
                    stage,
                    sink,
                );
            }
        }
    }
}

/// Decode a block payload into `stage.scratch` and return the byte length, or
/// `None` when the transform failed (already counted).
///
/// The scratch buffer is grown, never shrunk, and its tail past `byte_count` is
/// left untouched -- callers that hand the decoded payload to the sink must
/// slice to `byte_count` so the previous block's bytes never leak out.
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

pub(super) fn decode_and_parse_rep_layout(
    payload: &mut BitReader<'_>,
    bit_count: usize,
    actor_net_guid: NetworkGuid,
    stage: &mut Stage<'_>,
    sink: &mut dyn ReplicationSink,
) {
    if decode_into_scratch(payload, bit_count, actor_net_guid, stage).is_none() {
        return;
    }

    let mut field_reader = BitReader::with_bit_len(stage.scratch, bit_count as u64);
    match field::parse_rep_layout(&mut field_reader, sink) {
        Ok(count) => stage.stats.fields += u64::from(count),
        Err(_) => {
            let remaining = field_reader.bits_remaining();
            sink.on_stream_failure(StreamFailure {
                kind: StreamKind::RepLayout,
                actor_net_guid,
                bit_count: bit_count as u32,
                function_count: 0,
                consumed_bits: field_reader.position(),
                remaining_bits: remaining,
            });
            stage.stats.field_stream_failures += 1;
            stage.stats.skipped_bits += remaining;
        }
    }
}

pub(super) fn decode_and_parse_class_net_cache(
    payload: &mut BitReader<'_>,
    bit_count: usize,
    actor_net_guid: NetworkGuid,
    function_count: u32,
    stage: &mut Stage<'_>,
    sink: &mut dyn ReplicationSink,
) {
    let Some(byte_count) = decode_into_scratch(payload, bit_count, actor_net_guid, stage) else {
        return;
    };

    let mut rpc_reader = BitReader::with_bit_len(stage.scratch, bit_count as u64);
    match field::parse_class_net_cache(&mut rpc_reader, function_count, sink) {
        Ok(count) => stage.stats.rpcs += u64::from(count),
        Err(error) => {
            let remaining = rpc_reader.bits_remaining();
            let failure = StreamFailure {
                kind: StreamKind::Rpc,
                actor_net_guid,
                bit_count: bit_count as u32,
                function_count,
                consumed_bits: rpc_reader.position(),
                remaining_bits: remaining,
            };
            if matches!(error, NetError::UnresolvedFunctionCount) {
                sink.on_unresolved_class_net_cache_payload(failure, &stage.scratch[..byte_count]);
            }
            sink.on_stream_failure(failure);
            stage.stats.rpc_stream_failures += 1;
            stage.stats.skipped_bits += remaining;
        }
    }
}

/// Diagnostic-event construction, compiled out entirely without the
/// `diagnostics` feature.
///
/// Each function has a no-op twin with the same signature so the framing loop
/// above reads the same either way; the arguments are all scalars the loop
/// already holds, so the no-op build optimises them away.
mod diagnostics {
    use super::{BunchContext, ContentBlockHeader, NetStats, NetworkGuid};

    #[cfg(feature = "diagnostics")]
    use crate::stats::{
        BunchFlagSnapshot, ContentBlockHeaderSnapshot, DiagnosticEvent, SkipReason,
    };

    /// The header fields every event copies verbatim from the bunch context.
    #[cfg(feature = "diagnostics")]
    fn base(
        ctx: &BunchContext<'_>,
        ch_index: u32,
        actor_net_guid: NetworkGuid,
        block_index: u32,
        consumed_bits: u64,
        remaining_bits: u64,
    ) -> DiagnosticEvent {
        let h = ctx.header;
        DiagnosticEvent {
            reason: SkipReason::HeaderReadError,
            packet_id: h.packet_id,
            bunch_index_in_packet: ctx.ids.bunch_index_in_packet,
            global_bunch_index: ctx.ids.global_bunch_index,
            channel_bunch_index: ctx.ids.channel_bunch_index,
            channel_index: ch_index,
            actor_net_guid: actor_net_guid.0,
            actor_path: None,
            archetype_net_guid: 0,
            class_path: None,
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
            content_block_header: None,
            content_bits: None,
            block_index_in_bunch: block_index,
            bits_skipped: remaining_bits,
        }
    }

    #[cfg(feature = "diagnostics")]
    pub(super) fn header_read_error(
        stats: &mut NetStats,
        ctx: &BunchContext<'_>,
        ch_index: u32,
        actor_net_guid: NetworkGuid,
        block_index: u32,
        consumed_bits: u64,
        remaining_bits: u64,
    ) {
        stats.record_diagnostic(|| {
            base(
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                consumed_bits,
                remaining_bits,
            )
        });
    }

    #[cfg(feature = "diagnostics")]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn content_bits_read_error(
        stats: &mut NetStats,
        ctx: &BunchContext<'_>,
        ch_index: u32,
        actor_net_guid: NetworkGuid,
        block_index: u32,
        consumed_bits: u64,
        remaining_bits: u64,
        header: &ContentBlockHeader,
    ) {
        stats.record_diagnostic(|| DiagnosticEvent {
            reason: SkipReason::ContentBitsReadError,
            content_block_header: Some(ContentBlockHeaderSnapshot::from(header)),
            ..base(
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                consumed_bits,
                remaining_bits,
            )
        });
    }

    #[cfg(feature = "diagnostics")]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn content_bits_overrun(
        stats: &mut NetStats,
        ctx: &BunchContext<'_>,
        ch_index: u32,
        actor_net_guid: NetworkGuid,
        block_index: u32,
        consumed_bits: u64,
        remaining_bits: u64,
        header: &ContentBlockHeader,
        content_bits: u32,
    ) {
        stats.record_diagnostic(|| DiagnosticEvent {
            reason: SkipReason::ContentBitsOverrun {
                declared_content_bits: content_bits,
                available_bits: remaining_bits,
            },
            content_block_header: Some(ContentBlockHeaderSnapshot::from(header)),
            content_bits: Some(content_bits),
            ..base(
                ctx,
                ch_index,
                actor_net_guid,
                block_index,
                consumed_bits,
                remaining_bits,
            )
        });
    }

    #[cfg(not(feature = "diagnostics"))]
    pub(super) fn header_read_error(
        _stats: &mut NetStats,
        _ctx: &BunchContext<'_>,
        _ch_index: u32,
        _actor_net_guid: NetworkGuid,
        _block_index: u32,
        _consumed_bits: u64,
        _remaining_bits: u64,
    ) {
    }

    #[cfg(not(feature = "diagnostics"))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn content_bits_read_error(
        _stats: &mut NetStats,
        _ctx: &BunchContext<'_>,
        _ch_index: u32,
        _actor_net_guid: NetworkGuid,
        _block_index: u32,
        _consumed_bits: u64,
        _remaining_bits: u64,
        _header: &ContentBlockHeader,
    ) {
    }

    #[cfg(not(feature = "diagnostics"))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn content_bits_overrun(
        _stats: &mut NetStats,
        _ctx: &BunchContext<'_>,
        _ch_index: u32,
        _actor_net_guid: NetworkGuid,
        _block_index: u32,
        _consumed_bits: u64,
        _remaining_bits: u64,
        _header: &ContentBlockHeader,
        _content_bits: u32,
    ) {
    }
}
