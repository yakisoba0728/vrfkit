//! Aggregate statistics and diagnostics for a replication stream pass.
//!
//! Every discard, skip, or error is counted here. Silent data loss is a bug.
//!
//! A content block that cannot be framed (its header or `content_bits` does not
//! read, or `content_bits` overruns the bunch), or whose payload transform
//! fails, also records a `DiagnosticEvent` locating it: packet, bunch, channel,
//! actor, header fields and the failing bit position. A field or RPC stream
//! that fails to walk goes to the sink instead (see `SkipReason::ParseFailure`).
//! Only the event log is capped, never the counters; a healthy replay has none.

use crate::content::ContentBlockHeader;

/// Upper bound on [`NetStats::diagnostics`]: a wrong transform can fail one
/// block per bunch (530,401 on 02d4d478), ~100 MB of events; this is ~3 MB.
pub const MAX_DIAGNOSTIC_EVENTS: usize = 16_384;

/// Declares [`NetStats`] with `counters`, `counters_mut` and `absorb` over one
/// field list, so a counter cannot be summed or published by one and missed by
/// another.
macro_rules! net_stats {
    ($($(#[$doc:meta])* $name:ident,)*) => {
        /// Cumulative counters for one replay's replication pass. Outside
        /// this crate, start from `default()` and assign fields: counters
        /// are added without that path breaking.
        #[derive(Debug, Clone, Default)]
        #[non_exhaustive]
        pub struct NetStats {
            $($(#[$doc])* pub $name: u64,)*
            /// Diagnostic events, capped at [`MAX_DIAGNOSTIC_EVENTS`].
            pub diagnostics: Vec<DiagnosticEvent>,
        }

        impl NetStats {
            /// Every counter by field name, in declaration order.
            #[must_use]
            pub fn counters(&self) -> Vec<(&'static str, u64)> {
                vec![$((stringify!($name), self.$name)),*]
            }

            /// [`Self::counters`], writable.
            pub fn counters_mut(&mut self) -> Vec<(&'static str, &mut u64)> {
                vec![$((stringify!($name), &mut self.$name)),*]
            }

            /// Add every counter from a completed independent pass; its events
            /// are appended up to the cap and the rest counted as dropped.
            pub fn absorb(&mut self, other: &mut Self) {
                $(self.$name += other.$name;)*
                let room = MAX_DIAGNOSTIC_EVENTS.saturating_sub(self.diagnostics.len());
                let keep = room.min(other.diagnostics.len());
                self.diagnostics.extend(other.diagnostics.drain(..keep));
                self.diagnostics_dropped += other.diagnostics.len() as u64;
                other.diagnostics.clear();
            }
        }
    };
}

net_stats! {
    /// Packets processed, malformed ones included.
    packets,
    /// Packets abandoned as malformed: no sentinel, a bunch header that does
    /// not read, or a payload past the packet end. The bits after that point
    /// reach no bit counter.
    malformed_packets,
    /// Bunches whose header read.
    bunches,
    /// Partial reassembly rejections of any cause (sequence, alignment,
    /// resource limit, destructive close). The cause counters partition it;
    /// [`Self::partial_unclassified_errors`] is the residual.
    partial_errors,
    /// Bunches whose header declared `b_partial`, rejected ones included.
    partial_bunches,
    /// Continuations rejected because no initial fragment was buffered.
    partial_missing_initial,
    /// Of those, the ones marked final.
    partial_missing_initial_final,
    /// Of those, the ones on a reliable channel.
    partial_missing_initial_reliable,
    /// Their payload bits, all discarded before framing.
    partial_missing_initial_bits,
    /// Initial fragments that replaced an incomplete assembly on the channel.
    partial_overlapping_initial,
    /// Continuations whose reliability or sequence did not match the assembly.
    partial_mismatched_continuation,
    /// Non-final fragments rejected because their payload was not byte-aligned.
    partial_non_byte_aligned,
    /// Fragments refused because their byte slice was shorter than their
    /// declared bit count. Pipeline staging supplies the whole window, so this
    /// is 0 there; it keeps the cause classified rather than a residual.
    partial_payload_overrun,
    /// Buffered assemblies discarded by a destructive channel close.
    partial_channel_close,
    /// Partial fragments accumulated (initial + continuations).
    partial_fragments,
    /// Partial bunches that completed.
    partial_completed,
    /// Partial bunches still awaiting fragments when the stream ended. Moved
    /// only at end of stream (`finish` / `finish_with_sink`): until then they
    /// cannot be told from reassembly in progress. Not a `partial_errors`.
    unfinished_partials,
    /// Bits buffered by those bunches, and so lost. Not in
    /// [`Self::skipped_bits`]: the loss is attributable only at end of stream.
    unfinished_partial_bits,
    /// Bunch payloads abandoned because a header stage failed (package-map
    /// exports, must-be-mapped GUIDs, the channel open), or refused at a
    /// channel-state limit (then also in [`Self::channel_state_limit_failures`]).
    bunch_header_failures,
    /// Content blocks framed (actor + subobject + deleted).
    content_blocks,
    /// Content blocks with RepLayout (property) payloads.
    rep_layout_blocks,
    /// Content blocks with ClassNetCache (RPC) payloads.
    class_net_cache_blocks,
    /// Content blocks flagged as deleted.
    deleted_blocks,
    /// Fields walked (handle + payload pairs).
    fields,
    /// RPC invocations walked.
    rpcs,
    /// Bits not walked into a field or RPC: failed or abandoned block payloads
    /// (preserved unresolved ones included), abandoned bunches, and refused or
    /// discarded partial fragments. [`Self::unfinished_partial_bits`],
    /// [`Self::unopened_channel_bits`] and RepLayout-export bunches are not in it.
    skipped_bits,
    /// Content blocks whose header or `content_bits` did not read: the rest of
    /// the bunch is lost, as for an overrun. A [`Self::lost_content_blocks`] term.
    content_block_framing_failures,
    /// Content blocks whose `content_bits` overran the bunch.
    malformed_content_blocks,
    /// Content blocks whose payload transform failed: the whole declared
    /// length is skipped.
    transform_failures,
    /// Content blocks whose decoded RepLayout stream failed to walk: a partly
    /// wrong transform can frame cleanly and still fail here.
    field_stream_failures,
    /// Content blocks whose decoded ClassNetCache stream failed to walk,
    /// including a RepLayout block's post-terminator tail that did not decode.
    rpc_stream_failures,
    /// RPC stream failures whose whole payload was preserved (an unresolved
    /// group, or an unverified RepLayout tail kept whole): an inclusive subset
    /// of [`Self::rpc_stream_failures`].
    unresolved_rpc_payloads_preserved,
    /// Actor channels opened.
    actor_opens,
    /// Actor channels closed.
    actor_closes,
    /// Opens that replaced an actor still open on that channel: the new actor
    /// stands and no close is fabricated for the old one.
    channel_reopens_while_open,
    /// Dynamic-actor opens whose payload ended before the mandatory spawn block.
    actor_opens_missing_spawn,
    /// Open bunches that failed to complete on a channel holding a live actor,
    /// which is retired (no close fabricated) so later bunches are not framed
    /// under its schema. For a clean package-map-export failure this alone
    /// names the lost open. Opens carried by partial fragments are not covered.
    failed_reopens_while_open,
    /// Bunches that reached framing with payload left and no open actor on
    /// their channel, dropped whole. Never framed, so they count as bunches
    /// and bits, not in [`Self::lost_content_blocks`].
    bunches_on_unopened_channel,
    /// Payload bits those bunches held after their preambles. Not in
    /// [`Self::skipped_bits`]: nothing failed to read.
    unopened_channel_bits,
    /// Bunches refused because a channel-state table was full or a reliable
    /// sequence could not advance representably.
    channel_state_limit_failures,
    /// Partial fragments refused because active reassembly state, buffered
    /// bits, checked arithmetic or allocation reached its bound.
    partial_resource_limit_failures,
    /// Package-map export bunches read.
    package_map_exports,
    /// Package-map export bunches carrying a RepLayout export, which is not
    /// parsed: skipped whole. Their bits reach no bit counter; this is the
    /// only record.
    rep_layout_export_bunches,
    /// Net GUIDs exported via package-map.
    exported_guids,
    /// Must-be-mapped GUIDs consumed.
    must_be_mapped_guids,
    /// Events the cap refused: non-zero means [`Self::diagnostics`] is a
    /// prefix. The counters are complete either way.
    diagnostics_dropped,
}

impl NetStats {
    /// Partial errors the exclusive cause counters do not explain: a residual,
    /// so a new error path shows instead of being filed under a known cause.
    #[must_use]
    pub fn partial_unclassified_errors(&self) -> u64 {
        self.partial_errors
            .saturating_sub(self.classified_partial_errors())
    }

    /// Cause counts in excess of `partial_errors`, if attribution double-counted.
    #[must_use]
    pub fn partial_overclassified_errors(&self) -> u64 {
        self.classified_partial_errors()
            .saturating_sub(self.partial_errors)
    }

    fn classified_partial_errors(&self) -> u64 {
        self.partial_missing_initial
            + self.partial_overlapping_initial
            + self.partial_mismatched_continuation
            + self.partial_non_byte_aligned
            + self.partial_payload_overrun
            + self.partial_channel_close
            + self.partial_resource_limit_failures
    }

    /// Content blocks whose payload never reached the exported tables: one
    /// term per failure depth, because framing can look fine while the payload
    /// is unreadable. Published as manifest `quality.content_blocks_lost`.
    /// RPC failures enter netted ([`Self::rpc_payloads_lost`]): an unresolved
    /// ClassNetCache block is exported whole as one reserved row, so it is no
    /// loss (6,490 such blocks and 0 lost on 02d4d478).
    pub fn lost_content_blocks(&self) -> u64 {
        self.content_block_framing_failures
            + self.malformed_content_blocks
            + self.transform_failures
            + self.field_stream_failures
            + self.rpc_payloads_lost()
    }

    /// RPC stream failures whose payload was not preserved. Saturating: the
    /// two counters move on different paths.
    #[must_use]
    pub fn rpc_payloads_lost(&self) -> u64 {
        self.rpc_stream_failures
            .saturating_sub(self.unresolved_rpc_payloads_preserved)
    }

    /// Record one diagnostic event, or count it dropped if the log is full; the
    /// closure builds the event only when it will be kept.
    pub fn record_diagnostic(&mut self, event: impl FnOnce() -> DiagnosticEvent) {
        if self.diagnostics.len() < MAX_DIAGNOSTIC_EVENTS {
            self.diagnostics.push(event());
        } else {
            self.diagnostics_dropped += 1;
        }
    }
}

/// Why a content block or bunch tail was skipped.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// `content_bits` exceeded the bits left in the bunch: the stream is
    /// misaligned for the rest of this bunch.
    ContentBitsOverrun {
        /// The declared content payload size that was too large.
        declared_content_bits: u32,
        /// How many bits actually remained in the bunch payload.
        available_bits: u64,
    },
    /// The content block header did not read; the rest of the bunch is lost.
    HeaderReadError,
    /// The `IntPacked` content-bits field did not read.
    ContentBitsReadError,
    /// The block framed but its payload transform failed; only that block's
    /// `content_bits` are skipped. Unreachable from framing by construction
    /// (an overrun is refused first and the scratch is sized to the block), so
    /// `validate`'s skip breakdown reads 0 here. Stream failures are not
    /// events: they go to the sink's `on_stream_failure`.
    ParseFailure,
}

/// Full context snapshot at the point a content block was skipped or malformed,
/// so one event dump can identify the root cause.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DiagnosticEvent {
    /// Why this event was recorded.
    pub reason: SkipReason,
    /// Packet index (0-based, global across the replay).
    pub packet_id: i32,
    /// Bunch index within this packet (0-based).
    pub bunch_index_in_packet: u32,
    /// Global bunch index across the entire replay (0-based).
    pub global_bunch_index: u64,
    /// Per-channel bunch count (how many bunches this channel has seen).
    pub channel_bunch_index: u64,
    /// Channel index.
    pub channel_index: u32,
    /// Actor network GUID on this channel.
    pub actor_net_guid: u32,
    /// The actor's path in the sink's GUID cache when the event was recorded;
    /// usually `None`, as dynamic actors are rarely exported by path.
    pub actor_path: Option<String>,
    /// Archetype GUID the channel's open read; 0 for a static actor (no spawn
    /// block), the wire's no-object GUID rather than a failed read.
    pub archetype_net_guid: u32,
    /// The archetype's path in the sink's GUID cache (not a class path,
    /// despite the name), when it has one.
    pub class_path: Option<String>,
    /// Bunch header flags.
    pub bunch_flags: BunchFlagSnapshot,
    /// Declared bunch payload bit count (from bunch header).
    pub payload_bit_count: i32,
    /// Bits consumed in the bunch payload before this event.
    pub consumed_bits: u64,
    /// Bits remaining in the bunch payload at the point of failure.
    pub remaining_bits: u64,
    /// Content block header (if successfully read before the failure).
    pub content_block_header: Option<ContentBlockHeaderSnapshot>,
    /// The `content_bits` value read from `IntPacked` (if available).
    pub content_bits: Option<u32>,
    /// Which content block within this bunch (0-based).
    pub block_index_in_bunch: u32,
    /// Bits this event charged to [`NetStats::skipped_bits`]: for a framing
    /// abort, from the failing block's first bit to the end of the bunch (so it
    /// can exceed `remaining_bits`); for a [`SkipReason::ParseFailure`], the
    /// block's `content_bits`.
    pub bits_skipped: u64,
}

/// Snapshot of all bunch header flags for diagnostic reporting.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct BunchFlagSnapshot {
    pub b_open: bool,
    pub b_close: bool,
    pub b_reliable: bool,
    pub b_partial: bool,
    pub b_partial_initial: bool,
    pub b_partial_final: bool,
    pub b_has_package_map_exports: bool,
    pub b_has_must_be_mapped_guids: bool,
    pub b_dormant: bool,
}

/// Snapshot of the content block header fields for diagnostic reporting.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ContentBlockHeaderSnapshot {
    pub has_rep_layout: bool,
    pub is_actor: bool,
    pub object_net_guid: u32,
    pub is_stably_named: bool,
    pub is_deleted: bool,
    pub class_net_guid: u32,
    pub outer_net_guid: u32,
    pub delete_flags: u8,
}

impl From<&ContentBlockHeader> for ContentBlockHeaderSnapshot {
    fn from(h: &ContentBlockHeader) -> Self {
        Self {
            has_rep_layout: h.has_rep_layout,
            is_actor: h.is_actor,
            object_net_guid: h.object_net_guid.0,
            is_stably_named: h.is_stably_named,
            is_deleted: h.is_deleted,
            class_net_guid: h.class_net_guid.0,
            outer_net_guid: h.outer_net_guid.0,
            delete_flags: h.delete_flags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each failure depth alone reaches the loss total and the five add;
    /// preserved unresolved RPC payloads net out, saturating; loud but normal
    /// counters are no loss (02d4d478 skips 18,217,181 bits and exports whole).
    #[test]
    fn lost_content_blocks_counts_each_depth_and_nets_preserved_rpcs() {
        type Case = (fn(&mut NetStats), u64);
        let cases: [Case; 10] = [
            (|s| s.content_block_framing_failures = 3, 3),
            (|s| s.malformed_content_blocks = 3, 3),
            (|s| s.transform_failures = 3, 3),
            (|s| s.field_stream_failures = 3, 3),
            (|s| s.rpc_stream_failures = 3, 3),
            (
                |s| {
                    s.content_block_framing_failures = 16;
                    s.malformed_content_blocks = 1;
                    s.transform_failures = 2;
                    s.field_stream_failures = 4;
                    s.rpc_stream_failures = 8;
                },
                31,
            ),
            (
                |s| (s.rpc_stream_failures, s.unresolved_rpc_payloads_preserved) = (10, 10),
                0,
            ),
            (
                |s| (s.rpc_stream_failures, s.unresolved_rpc_payloads_preserved) = (10, 7),
                3,
            ),
            (
                |s| (s.rpc_stream_failures, s.unresolved_rpc_payloads_preserved) = (1, 5),
                0,
            ),
            (
                |s| {
                    s.skipped_bits = 18_217_181;
                    s.partial_fragments = 4096;
                    s.deleted_blocks = 128;
                    s.actor_closes = 1799;
                    s.must_be_mapped_guids = 64;
                },
                0,
            ),
        ];
        for (i, (set, lost)) in cases.into_iter().enumerate() {
            let mut stats = NetStats::default();
            set(&mut stats);
            assert_eq!(stats.lost_content_blocks(), lost, "case {i}");
        }
    }

    fn event(block_index: u32) -> DiagnosticEvent {
        DiagnosticEvent {
            reason: SkipReason::HeaderReadError,
            packet_id: 0,
            bunch_index_in_packet: 0,
            global_bunch_index: 0,
            channel_bunch_index: 0,
            channel_index: 0,
            actor_net_guid: 0,
            actor_path: None,
            archetype_net_guid: 0,
            class_path: None,
            bunch_flags: BunchFlagSnapshot::default(),
            payload_bit_count: 0,
            consumed_bits: 0,
            remaining_bits: 0,
            content_block_header: None,
            content_bits: None,
            block_index_in_bunch: block_index,
            bits_skipped: 0,
        }
    }

    /// Past the cap the log keeps its earliest events, which explain the rest,
    /// and counts the overflow.
    #[test]
    fn diagnostics_are_capped_and_the_overflow_is_counted() {
        let mut stats = NetStats::default();
        for i in 0..(MAX_DIAGNOSTIC_EVENTS as u32 + 5) {
            stats.record_diagnostic(|| event(i));
        }
        assert_eq!(stats.diagnostics.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(stats.diagnostics_dropped, 5);
        assert_eq!(stats.diagnostics[0].block_index_in_bunch, 0);
        assert_eq!(
            stats.diagnostics[MAX_DIAGNOSTIC_EVENTS - 1].block_index_in_bunch,
            MAX_DIAGNOSTIC_EVENTS as u32 - 1
        );
    }

    /// Absorbing a pass twice doubles every counter under its own name, and
    /// the events fill the log up to the cap, the rest counted as dropped.
    #[test]
    fn absorb_adds_every_counter_and_caps_the_events() {
        let mut pass = NetStats::default();
        for (i, (_, value)) in pass.counters_mut().into_iter().enumerate() {
            *value = i as u64 + 1;
        }
        pass.diagnostics = vec![event(1), event(2)];
        let mut total = NetStats::default();
        total.absorb(&mut pass.clone());
        total.absorb(&mut pass.clone());
        let doubled: Vec<_> = (pass.counters().into_iter())
            .map(|(name, value)| (name, 2 * value))
            .collect();
        assert_eq!(total.counters(), doubled);
        assert_eq!(total.diagnostics.len(), 4);

        let mut full = NetStats {
            diagnostics: vec![event(0); MAX_DIAGNOSTIC_EVENTS - 1],
            ..NetStats::default()
        };
        full.absorb(&mut pass.clone());
        assert_eq!(full.diagnostics.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(
            full.diagnostics[MAX_DIAGNOSTIC_EVENTS - 1].block_index_in_bunch,
            1
        );
        assert_eq!(full.diagnostics_dropped, pass.diagnostics_dropped + 1);
    }

    #[test]
    fn unknown_partial_error_paths_remain_visible_as_a_residual() {
        let stats = NetStats {
            partial_errors: 10,
            partial_missing_initial: 2,
            partial_overlapping_initial: 1,
            partial_mismatched_continuation: 1,
            partial_non_byte_aligned: 1,
            partial_payload_overrun: 1,
            partial_channel_close: 1,
            partial_resource_limit_failures: 1,
            ..Default::default()
        };
        assert_eq!(stats.partial_unclassified_errors(), 2);
        assert_eq!(stats.partial_overclassified_errors(), 0);

        let over = NetStats {
            partial_errors: 1,
            partial_missing_initial: 2,
            ..Default::default()
        };
        assert_eq!(over.partial_unclassified_errors(), 0);
        assert_eq!(over.partial_overclassified_errors(), 1);
    }
}
