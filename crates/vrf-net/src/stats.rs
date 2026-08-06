//! Aggregate statistics and diagnostics for a replication stream pass.
//!
//! Every discard, skip, or error is counted here. Silent data loss is a bug.
//!
//! # Diagnostics
//!
//! When content blocks fail to parse (malformed payload size, read errors in
//! the block header, or transform failures), a [`DiagnosticEvent`] is recorded
//! with full context: packet, bunch, channel, actor, header fields, and the
//! exact bit position where failure occurred. This is essential for debugging
//! new game builds where the payload transform may not yet be correct.
//!
//! The event log is bounded. On a healthy replay it stays empty -- 02d4d478
//! records zero events -- but a replay whose transform is wrong can fail one
//! block per bunch, and at 530 401 bunches an unbounded `Vec` of ~200-byte
//! events is 100 MB of diagnostics for a run whose whole point is that it
//! failed. [`MAX_DIAGNOSTIC_EVENTS`] caps the log and
//! [`NetStats::diagnostics_dropped`] counts what the cap refused, so the loss
//! is reported rather than silent. The counters above the log are never capped:
//! `skipped_bits` and the failure counts remain exact totals.
//!
//! The whole of this machinery is behind the default-on `diagnostics` feature.
//! A consumer that only wants framing and counters can switch it off and lose
//! [`DiagnosticEvent`], [`SkipReason`], the two snapshot types and the two
//! fields below; nothing else in the crate changes shape.

#[cfg(feature = "diagnostics")]
use crate::content::ContentBlockHeader;

/// Upper bound on [`NetStats::diagnostics`].
///
/// Sized so a full log is a few megabytes rather than a few hundred: an event
/// is roughly 200 bytes, so this is about 3 MB. Anything past it is counted in
/// [`NetStats::diagnostics_dropped`], never dropped quietly.
#[cfg(feature = "diagnostics")]
pub const MAX_DIAGNOSTIC_EVENTS: usize = 16_384;

/// Cumulative counters for one replay's replication pass.
#[derive(Debug, Clone, Default)]
pub struct NetStats {
    /// Total packets processed (including malformed ones).
    pub packets: u64,
    /// Packets whose last byte was zero (sentinel missing).
    pub malformed_packets: u64,
    /// Total bunches parsed (header successfully read).
    pub bunches: u64,
    /// Partial-bunch sequence errors (fragment discarded).
    pub partial_errors: u64,
    /// Partial fragments accumulated (initial + continuations).
    pub partial_fragments: u64,
    /// Partial bunches that completed successfully.
    pub partial_completed: u64,
    /// Partial bunches still awaiting fragments when the replay ended.
    ///
    /// Moved only by [`crate::ReplicationReader::finish`], which must be called
    /// once after the last packet. Reassembly state that never completes is
    /// indistinguishable from reassembly in progress until the stream stops, so
    /// this is the only point at which the loss can be named. `partial_errors`
    /// does not cover it: nothing was out of sequence, the continuation simply
    /// never arrived.
    pub unfinished_partials: u64,
    /// Bits buffered by those unfinished partial bunches, and therefore lost.
    ///
    /// Kept out of [`Self::skipped_bits`], which is the content-block tally the
    /// oracle divides by failed blocks; these bits never reached framing.
    pub unfinished_partial_bits: u64,
    /// Bunch payloads whose header parse failed -- package-map exports,
    /// must-be-mapped GUIDs, or the channel-open block. The prior code did
    /// `let _ =` on these `Result`s, so a channel that failed to open was
    /// invisible (every later bunch on it skipped silently at the channel
    /// guard) and a truncated must-be-mapped list left the reader stuck and
    /// parsed the rest of the bunch as garbage. Counted to make both loud.
    pub bunch_header_failures: u64,
    /// Content blocks framed (actor + subobject + deleted).
    pub content_blocks: u64,
    /// Content blocks with RepLayout (property) payloads.
    pub rep_layout_blocks: u64,
    /// Content blocks with ClassNetCache (RPC) payloads.
    pub class_net_cache_blocks: u64,
    /// Content blocks flagged as deleted.
    pub deleted_blocks: u64,
    /// Total fields emitted (handle + payload pairs).
    pub fields: u64,
    /// Total RPC invocations emitted.
    pub rpcs: u64,
    /// Bits skipped due to malformed content block payloads.
    pub skipped_bits: u64,
    /// Malformed content block payloads (overrun).
    pub malformed_content_blocks: u64,
    /// Content blocks whose payload transform or bit copy failed.
    ///
    /// Counted separately from [`Self::malformed_content_blocks`] because the
    /// failure is at a different layer: the block was framed correctly but its
    /// payload could not be turned into readable bits at all, so the whole
    /// declared length is skipped.
    pub transform_failures: u64,
    /// Content blocks whose decoded RepLayout field stream failed to parse.
    ///
    /// The header framed and the payload decoded, but walking the handle /
    /// payload-length pairs inside it hit an error. These matter to the oracle:
    /// a partially wrong transform can leave block framing intact while making
    /// the field streams inside unreadable, and counting only framing failures
    /// would report a perfect pass rate for it.
    pub field_stream_failures: u64,
    /// Content blocks whose decoded ClassNetCache (RPC) stream failed to parse.
    pub rpc_stream_failures: u64,
    /// Actor channels opened.
    pub actor_opens: u64,
    /// Actor channels closed.
    pub actor_closes: u64,
    /// Opens that replaced an actor still recorded as open on that channel.
    ///
    /// Channel 5 opens for actor A and opens again for actor B with no close in
    /// between: A's state is overwritten and every later block on the channel is
    /// attributed to B. Nothing misframes, so no other counter moves. The
    /// replacement is kept -- the wire says B owns the channel now -- but A gets
    /// no close row, and a fabricated one would be data the replay never sent.
    /// This counts the fabrication that was NOT made.
    pub channel_reopens_while_open: u64,
    /// Dynamic-actor opens whose payload ended before the mandatory spawn block.
    ///
    /// The spawn block is not optional for a dynamic actor: the reference reads
    /// archetype, level, transform and velocity unconditionally. A payload that
    /// stops at the actor GUID used to be accepted as a successful open, which
    /// emitted an actor with archetype and level GUID 0 and no transforms while
    /// `bunch_header_failures` stayed at zero. Such an open now fails like any
    /// other truncated read; this names the specific shape so a corpus run can
    /// say whether it ever happens.
    pub actor_opens_missing_spawn: u64,
    /// Package-map export bunches processed.
    pub package_map_exports: u64,
    /// Package-map export bunches carrying a RepLayout export instead of GUIDs.
    ///
    /// That variant is not parsed -- the bunch is skipped whole. It is counted
    /// separately rather than folded into [`Self::skipped_bits`] because the
    /// oracle reads that tally as "bits lost across failed content blocks", and
    /// no content block is involved here.
    pub rep_layout_export_bunches: u64,
    /// Net GUIDs exported via package-map.
    pub exported_guids: u64,
    /// Must-be-mapped GUIDs consumed.
    pub must_be_mapped_guids: u64,
    /// Detailed diagnostic events for every skip/malformed occurrence, capped
    /// at [`MAX_DIAGNOSTIC_EVENTS`].
    ///
    /// This is the primary debugging tool when the oracle pass rate is not 100%.
    /// Each event records the full context needed to locate the failure in the
    /// replay stream and compare with the C# reference parser.
    #[cfg(feature = "diagnostics")]
    pub diagnostics: Vec<DiagnosticEvent>,
    /// Diagnostic events the cap refused to record.
    ///
    /// Non-zero means [`Self::diagnostics`] is a prefix of what happened, not
    /// the whole of it. The failure *counters* remain complete either way; only
    /// the per-event context is truncated.
    #[cfg(feature = "diagnostics")]
    pub diagnostics_dropped: u64,
}

impl NetStats {
    /// Record one diagnostic event, or count it as dropped if the log is full.
    ///
    /// The event is built by the closure so that a full log costs a length
    /// compare rather than the construction of an event nobody will read.
    #[cfg(feature = "diagnostics")]
    pub fn record_diagnostic(&mut self, event: impl FnOnce() -> DiagnosticEvent) {
        if self.diagnostics.len() < MAX_DIAGNOSTIC_EVENTS {
            self.diagnostics.push(event());
        } else {
            self.diagnostics_dropped += 1;
        }
    }
}

/// Why a content block or bunch tail was skipped.
#[cfg(feature = "diagnostics")]
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// `content_bits` (from `ReadIntPacked`) exceeded `bits_remaining` in the
    /// bunch payload -- the stream is irrecoverably misaligned for this bunch.
    ContentBitsOverrun {
        /// The declared content payload size that was too large.
        declared_content_bits: u32,
        /// How many bits actually remained in the bunch payload.
        available_bits: u64,
    },
    /// Reading the content block header failed (e.g. not enough bits for a
    /// GUID or the deletion flags). The remaining bunch payload is discarded.
    HeaderReadError,
    /// Reading the `IntPacked` content-bits field itself failed.
    ContentBitsReadError,
    /// Payload transform or field/RPC parsing failed -- the decoded block was
    /// garbage. Only the bits of that one block are skipped.
    ParseFailure,
}

/// Full context snapshot at the point a content block was skipped or malformed.
///
/// Every field requested in the diagnostic specification is captured here so
/// that a single event dump is sufficient to identify the root cause.
#[cfg(feature = "diagnostics")]
#[derive(Debug, Clone)]
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
    /// Resolved path for the actor (if available).
    pub actor_path: Option<String>,
    /// Archetype GUID.
    pub archetype_net_guid: u32,
    /// Class path (if resolved).
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
    /// Number of bits actually skipped in this event.
    pub bits_skipped: u64,
}

/// Snapshot of all bunch header flags for diagnostic reporting.
#[cfg(feature = "diagnostics")]
#[derive(Debug, Clone)]
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
#[cfg(feature = "diagnostics")]
#[derive(Debug, Clone)]
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

#[cfg(feature = "diagnostics")]
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

#[cfg(all(test, feature = "diagnostics"))]
mod tests {
    use super::*;

    fn dummy_event(block_index: u32) -> DiagnosticEvent {
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
            bunch_flags: BunchFlagSnapshot {
                b_open: false,
                b_close: false,
                b_reliable: false,
                b_partial: false,
                b_partial_initial: false,
                b_partial_final: false,
                b_has_package_map_exports: false,
                b_has_must_be_mapped_guids: false,
                b_dormant: false,
            },
            payload_bit_count: 0,
            consumed_bits: 0,
            remaining_bits: 0,
            content_block_header: None,
            content_bits: None,
            block_index_in_bunch: block_index,
            bits_skipped: 0,
        }
    }

    /// Past the cap the log stops growing and the overflow is counted, not
    /// dropped quietly. A replay whose transform is wrong fails roughly one
    /// block per bunch, and 530 401 unbounded events is ~100 MB of context for
    /// a run whose counters already say it failed.
    #[test]
    fn diagnostics_are_capped_and_the_overflow_is_counted() {
        let mut stats = NetStats::default();
        for i in 0..(MAX_DIAGNOSTIC_EVENTS as u32 + 5) {
            stats.record_diagnostic(|| dummy_event(i));
        }
        assert_eq!(stats.diagnostics.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(stats.diagnostics_dropped, 5);
        assert_eq!(
            stats.diagnostics[0].block_index_in_bunch, 0,
            "the log keeps the earliest events, which are the ones that explain the rest"
        );
        assert_eq!(
            stats.diagnostics[MAX_DIAGNOSTIC_EVENTS - 1].block_index_in_bunch,
            MAX_DIAGNOSTIC_EVENTS as u32 - 1
        );
    }

    /// A healthy pass records nothing and drops nothing. The reference replay
    /// is this case: zero events across 608 020 content blocks.
    #[test]
    fn a_clean_pass_records_no_diagnostics() {
        let stats = NetStats::default();
        assert!(stats.diagnostics.is_empty());
        assert_eq!(stats.diagnostics_dropped, 0);
    }
}
