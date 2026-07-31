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

use crate::content::ContentBlockHeader;

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
    /// Package-map export bunches processed.
    pub package_map_exports: u64,
    /// Net GUIDs exported via package-map.
    pub exported_guids: u64,
    /// Must-be-mapped GUIDs consumed.
    pub must_be_mapped_guids: u64,
    /// Detailed diagnostic events for every skip/malformed occurrence.
    ///
    /// This is the primary debugging tool when the oracle pass rate is not 100%.
    /// Each event records the full context needed to locate the failure in the
    /// replay stream and compare with the C# reference parser.
    pub diagnostics: Vec<DiagnosticEvent>,
}

/// Why a content block or bunch tail was skipped.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// `content_bits` (from `ReadIntPacked`) exceeded `bits_remaining` in the
    /// bunch payload — the stream is irrecoverably misaligned for this bunch.
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
    /// Payload transform or field/RPC parsing failed — the decoded block was
    /// garbage. Only the bits of that one block are skipped.
    ParseFailure,
}

/// Full context snapshot at the point a content block was skipped or malformed.
///
/// Every field requested in the diagnostic specification is captured here so
/// that a single event dump is sufficient to identify the root cause.
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
