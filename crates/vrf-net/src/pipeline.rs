//! Top-level replication reader that drives the full pipeline.
//!
//! This module connects packets ??bunches ??content blocks ??fields into a
//! single pass. The caller provides:
//!
//! - A [`ReplicationSink`] to receive decoded events (fields, RPCs, actor
//!   lifecycle, etc.)
//! - A replay branch string for payload transform selection
//!
//! # Allocation strategy
//!
//! A single `scratch` buffer is reused for every content-block payload
//! transform. The partial-bunch accumulator grows per-channel buffers only
//! when fragments are in flight.

use vrf_bitio::BitReader;
use vrf_transform::TransformVersion;

use crate::bunch::{PartialBunchAccumulator, RawBunchHeader};
use crate::content::{self, ContentBlockHeader};
use crate::error::Result;
use crate::field::{self, FieldSink};
use crate::net_guid::{self, GuidPathSink};
use crate::packet::RawPacketReader;
use crate::stats::{
    BunchFlagSnapshot, ContentBlockHeaderSnapshot, DiagnosticEvent, NetStats, SkipReason,
};
use crate::types::NetworkGuid;

use std::collections::HashMap;

/// Per-channel actor state tracked during replication.
#[derive(Debug, Clone)]
pub struct ActorChannelState {
    /// Channel index.
    pub channel_index: u32,
    /// Whether the channel is currently open.
    pub is_open: bool,
    /// Whether the channel is dormant (closed but actor alive).
    pub is_dormant: bool,
    /// Actor's network GUID.
    pub actor_net_guid: NetworkGuid,
    /// Archetype GUID (for dynamic actors).
    pub archetype_net_guid: NetworkGuid,
    /// Level GUID.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// A property (RepLayout) stream.
    RepLayout,
    /// An RPC (ClassNetCache) stream.
    Rpc,
}

/// Context for a content block that framed and decoded but whose inner stream
/// could not be walked.
///
/// Reported to the sink rather than only counted because the layer that knows
/// *names* is the sink: it resolved the group path and the function count. A
/// bare counter says "one block failed"; this says which class, which is what a
/// new game build's investigation actually needs.
#[derive(Debug, Clone, Copy)]
pub struct StreamFailure {
    /// Which grammar was being parsed.
    pub kind: StreamKind,
    /// Actor whose channel carried the block.
    pub actor_net_guid: NetworkGuid,
    /// Declared payload length of the block.
    pub bit_count: u32,
    /// Function count used for the handle read (`Rpc` only; 0 for `RepLayout`).
    ///
    /// Worth reporting because a wrong non-zero value can select the wrong
    /// serialized-int width and desynchronise the stream. The RPC parser clamps
    /// counts to at least 2, so declared counts 1 and 2 use the same wire width;
    /// zero remains the explicit unresolved-group sentinel.
    pub function_count: u32,
    /// Bits consumed before the failure.
    pub consumed_bits: u64,
    /// Bits abandoned as a result.
    pub remaining_bits: u64,
}

/// Trait for receiving all replication events.
///
/// The caller implements this to process fields, RPCs, and actor lifecycle
/// without any data being silently discarded.
pub trait ReplicationSink: GuidPathSink + FieldSink {
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

    /// A content block was flagged as deleted.
    fn on_deleted_block(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
    );

    /// A block framed and decoded, but its inner stream could not be walked.
    ///
    /// Defaulted to a no-op so a sink that does not care about failure context
    /// need not implement it; the counters in [`NetStats`] are maintained either
    /// way. Override it to attach the names the sink holds -- the resolved group
    /// path in particular, which the replication layer does not know.
    fn on_stream_failure(&mut self, _failure: StreamFailure) {}
}

/// The main replication reader. Drives the full pipeline for one replay.
pub struct ReplicationReader {
    packet_reader: RawPacketReader,
    accumulator: PartialBunchAccumulator,
    channels: HashMap<u32, ActorChannelState>,
    transform: TransformVersion,
    stats: NetStats,
    /// Reusable scratch buffer for payload transforms.
    scratch: Vec<u8>,
    /// GUIDs whose path matches the replay controller asset path.
    ///
    /// Unreal writes a 1-byte "net player index" after the spawn data for
    /// dynamic PlayerController actors. We track these GUIDs so we can
    /// consume that byte before framing content blocks.
    player_controller_guids: std::collections::HashSet<u32>,
    /// Global bunch counter (0-based, monotonically increasing).
    global_bunch_index: u64,
    /// Per-channel bunch counter: how many bunches each channel has received.
    channel_bunch_counts: HashMap<u32, u64>,
}

/// Leaf asset name of the VALORANT replay controller.
///
/// This is the only `PlayerController`-kind actor in VALORANT replays, and the
/// reference parser keys the net-player-index byte off it (see
/// [`is_player_controller_path`]).
pub const PLAYER_CONTROLLER_LEAF: &str = "BaseReplayController";

/// Whether `path` names the replay controller, in any of the spellings Unreal
/// uses for the same asset.
///
/// The same class arrives under at least four different strings depending on
/// where in the stream it was written:
///
/// | source | string |
/// |---|---|
/// | net field export group path | `/Game/Characters/_Core/BaseReplayController.BaseReplayController_C` |
/// | NetGUID path (package-map export) | `/Game/Characters/_Core/BaseReplayController` |
/// | archetype GUID path (class default object) | `Default__BaseReplayController_C` |
/// | `/_Core/` elided alias | `/Game/Characters/BaseReplayController` |
///
/// So this normalises instead of comparing: take the last `/`-separated
/// segment, drop anything before a `.` (the `Asset.Class_C` form), strip a
/// `Default__` prefix and a `_C` suffix, then compare the bare name.
///
/// Getting this wrong is silent. The index byte below is simply not consumed,
/// and every content block after it in that bunch is shifted by 8 bits ??which
/// surfaces only as one malformed block and a few hundred skipped bits.
fn is_player_controller_path(path: &str) -> bool {
    let segment = path.rsplit('/').next().unwrap_or(path);
    // `Asset.Class_C` -> `Class_C`; a bare segment is unchanged.
    let class = segment.rsplit('.').next().unwrap_or(segment);
    let class = class.strip_prefix("Default__").unwrap_or(class);
    let class = class.strip_suffix("_C").unwrap_or(class);
    class == PLAYER_CONTROLLER_LEAF
}

/// Thin wrapper around a [`ReplicationSink`] that intercepts `register_path`
/// calls to detect PlayerController GUIDs.
///
/// The reference parser's `ReadNetPlayerIndexStage` asks whether the channel's
/// class, archetype or actor path is a PlayerController. We do not keep those
/// path strings on the channel, so instead every GUID whose registered path
/// names the controller is remembered here and the check below tests the
/// channel's actor and archetype GUIDs against that set.
///
/// Registration order matters and works out: the archetype GUID's path is
/// registered by `internal_load_object` while reading the *same* bunch's spawn
/// data, which happens before the net-player-index check.
struct PathInterceptSink<'a> {
    inner: &'a mut dyn ReplicationSink,
    pc_guids: &'a mut std::collections::HashSet<u32>,
}

impl GuidPathSink for PathInterceptSink<'_> {
    fn register_path(&mut self, guid: u32, path: &str, outer_guid: NetworkGuid) {
        if is_player_controller_path(path) {
            self.pc_guids.insert(guid);
        }
        self.inner.register_path(guid, path, outer_guid);
    }
}

/// Per-bunch context carried through content-block framing for diagnostics.
///
/// This is not part of the hot path ??it is only read when a diagnostic event
/// is emitted (malformed/skipped). Keeping it in a separate struct avoids
/// adding more parameters to already-long function signatures.
struct BunchContext {
    packet_id: i32,
    bunch_index_in_packet: u32,
    global_bunch_index: u64,
    channel_bunch_index: u64,
    payload_bit_count: i32,
    flags: BunchFlagSnapshot,
}

impl ReplicationReader {
    /// Create a reader for a replay with the given branch.
    pub fn new(branch: &str) -> Result<Self> {
        let transform = TransformVersion::require(branch)?;
        Ok(Self {
            packet_reader: RawPacketReader::new(),
            accumulator: PartialBunchAccumulator::new(),
            channels: HashMap::new(),
            transform,
            stats: NetStats::default(),
            scratch: vec![0u8; 4096],
            player_controller_guids: std::collections::HashSet::new(),
            global_bunch_index: 0,
            channel_bunch_counts: HashMap::new(),
        })
    }

    /// Access accumulated statistics.
    #[must_use]
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }

    /// Process one raw packet (byte slice as received from the demo frame).
    pub fn process_packet(
        &mut self,
        packet_data: &[u8],
        packet_id: i32,
        sink: &mut dyn ReplicationSink,
    ) {
        self.stats.packets += 1;

        if packet_data.is_empty() {
            return;
        }

        if packet_data[packet_data.len() - 1] == 0 {
            self.stats.malformed_packets += 1;
            return;
        }

        // We need to collect bunch payloads because the callback borrows self.
        // Instead, we use a two-phase approach: parse headers, then process.
        let result = {
            let stats = &mut self.stats;
            let accumulator = &mut self.accumulator;
            let channels = &mut self.channels;
            let transform = self.transform;
            let scratch = &mut self.scratch;
            let pc_guids = &mut self.player_controller_guids;
            let global_bunch_index = &mut self.global_bunch_index;
            let channel_bunch_counts = &mut self.channel_bunch_counts;

            let mut bunches_to_process: Vec<(RawBunchHeader, Vec<u8>, u64, u32, u64, u64)> =
                Vec::new();
            let mut bunch_index_in_packet: u32 = 0;

            let result =
                self.packet_reader
                    .read_packet(packet_data, packet_id, |header, payload| {
                        stats.bunches += 1;
                        let current_global = *global_bunch_index;
                        *global_bunch_index += 1;
                        let ch_count = channel_bunch_counts.entry(header.ch_index).or_insert(0);
                        *ch_count += 1;
                        let current_ch_bunch = *ch_count;

                        // Collect payload data for later processing
                        let bit_count = payload.bits_remaining();
                        let byte_count = (bit_count as usize).div_ceil(8);
                        let mut data = vec![0u8; byte_count];
                        let mut payload_copy = payload;
                        if bit_count > 0 {
                            let _ = payload_copy.copy_bits_to(&mut data, bit_count);
                        }
                        bunches_to_process.push((
                            header.clone(),
                            data,
                            bit_count,
                            bunch_index_in_packet,
                            current_global,
                            current_ch_bunch,
                        ));
                        bunch_index_in_packet += 1;
                    });

            if result.is_malformed {
                stats.malformed_packets += 1;
            }
            stats.partial_errors += result.partial_error_count as u64;

            // Now process each bunch payload
            for (mut header, data, bit_count, bi_in_pkt, g_idx, ch_idx) in bunches_to_process {
                Self::process_bunch_payload(
                    &mut header,
                    &data,
                    bit_count,
                    stats,
                    accumulator,
                    channels,
                    transform,
                    scratch,
                    pc_guids,
                    sink,
                    bi_in_pkt,
                    g_idx,
                    ch_idx,
                );
            }

            result
        };

        let _ = result;
    }

    #[allow(clippy::too_many_arguments)]
    fn process_bunch_payload(
        header: &mut RawBunchHeader,
        data: &[u8],
        bit_count: u64,
        stats: &mut NetStats,
        accumulator: &mut PartialBunchAccumulator,
        channels: &mut HashMap<u32, ActorChannelState>,
        transform: TransformVersion,
        scratch: &mut Vec<u8>,
        pc_guids: &mut std::collections::HashSet<u32>,
        sink: &mut dyn ReplicationSink,
        bunch_index_in_packet: u32,
        global_bunch_index: u64,
        channel_bunch_index: u64,
    ) {
        let ch_index = header.ch_index;

        // Handle partial bunches
        if header.b_partial {
            let byte_count = (bit_count as usize).div_ceil(8);
            let result = accumulator.add_fragment(
                ch_index,
                header.clone(),
                &data[..byte_count],
                bit_count as usize,
                &mut stats.partial_errors,
                &mut stats.partial_fragments,
                &mut stats.partial_completed,
            );
            *header = result.header;

            if !result.should_process {
                return;
            }

            // Take completed payload
            if let Some((buf, total_bits, stored_header)) = accumulator.take_completed(ch_index) {
                let mut payload_reader = BitReader::with_bit_len(&buf, total_bits as u64);
                Self::process_complete_payload(
                    &stored_header,
                    &mut payload_reader,
                    stats,
                    channels,
                    transform,
                    scratch,
                    pc_guids,
                    sink,
                    bunch_index_in_packet,
                    global_bunch_index,
                    channel_bunch_index,
                );
            }
            return;
        }

        // Non-partial: process directly
        if bit_count == 0 {
            // Handle close
            if header.b_close {
                Self::handle_channel_close(header, channels, stats, sink);
            }
            return;
        }

        let mut payload_reader = BitReader::with_bit_len(data, bit_count);
        Self::process_complete_payload(
            header,
            &mut payload_reader,
            stats,
            channels,
            transform,
            scratch,
            pc_guids,
            sink,
            bunch_index_in_packet,
            global_bunch_index,
            channel_bunch_index,
        );

        if header.b_close {
            Self::handle_channel_close(header, channels, stats, sink);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_complete_payload(
        header: &RawBunchHeader,
        payload: &mut BitReader<'_>,
        stats: &mut NetStats,
        channels: &mut HashMap<u32, ActorChannelState>,
        transform: TransformVersion,
        scratch: &mut Vec<u8>,
        pc_guids: &mut std::collections::HashSet<u32>,
        sink: &mut dyn ReplicationSink,
        bunch_index_in_packet: u32,
        global_bunch_index: u64,
        channel_bunch_index: u64,
    ) {
        let ch_index = header.ch_index;

        // Package map exports
        if header.b_has_package_map_exports {
            let _ = Self::read_package_map_exports(payload, stats, pc_guids, sink);
            stats.package_map_exports += 1;
            return;
        }

        // Must-be-mapped GUIDs
        if header.b_has_must_be_mapped_guids {
            let _ = Self::read_must_be_mapped_guids(payload, stats);
        }

        // Actor channel open
        if header.b_open {
            let _ = Self::handle_channel_open(header, payload, channels, stats, pc_guids, sink);
        }

        // Look up channel ??if not open, skip
        let (actor_net_guid, is_open, archetype_net_guid) = match channels.get(&ch_index) {
            Some(ch) => (ch.actor_net_guid, ch.is_open, ch.archetype_net_guid),
            None => return,
        };

        if !is_open {
            return;
        }

        // ReadNetPlayerIndex: Unreal writes a 1-byte "player index" between the
        // actor-open spawn data and the first content block when the newly
        // opened actor is a dynamic PlayerController. Without consuming this
        // byte, all subsequent content blocks in the bunch are shifted by 8 bits.
        //
        // C# reference: ReadNetPlayerIndexStage.cs ??checks OpenedDynamicActor
        // && IsPlayerController(channel archetype/class/actor path).
        if header.b_open
            && actor_net_guid.is_dynamic()
            && !payload.at_end()
            && (pc_guids.contains(&archetype_net_guid.0) || pc_guids.contains(&actor_net_guid.0))
        {
            let _ = payload.read_u8();
        }

        // Frame content blocks
        let bunch_ctx = BunchContext {
            packet_id: header.packet_id,
            bunch_index_in_packet,
            global_bunch_index,
            channel_bunch_index,
            payload_bit_count: header.payload_bit_count,
            flags: BunchFlagSnapshot {
                b_open: header.b_open,
                b_close: header.b_close,
                b_reliable: header.b_reliable,
                b_partial: header.b_partial,
                b_partial_initial: header.b_partial_initial,
                b_partial_final: header.b_partial_final,
                b_has_package_map_exports: header.b_has_package_map_exports,
                b_has_must_be_mapped_guids: header.b_has_must_be_mapped_guids,
                b_dormant: header.b_dormant,
            },
        };
        Self::frame_content_blocks(
            payload,
            ch_index,
            actor_net_guid,
            transform,
            scratch,
            stats,
            sink,
            &bunch_ctx,
        );
    }

    fn read_package_map_exports(
        payload: &mut BitReader<'_>,
        stats: &mut NetStats,
        pc_guids: &mut std::collections::HashSet<u32>,
        sink: &mut dyn ReplicationSink,
    ) -> Result<()> {
        let has_rep_layout_export = payload.read_bit()?;
        if has_rep_layout_export {
            // Unsupported: skip remaining
            payload.skip_remaining();
            return Ok(());
        }

        let num_guids = payload.read_i32()?;
        if num_guids < 0 || num_guids as u32 > crate::types::MAX_GUID_COUNT {
            payload.skip_remaining();
            return Ok(());
        }

        let mut intercept = PathInterceptSink {
            inner: sink,
            pc_guids,
        };
        for _ in 0..num_guids {
            let _ = net_guid::internal_load_object(payload, true, 0, &mut intercept)?;
            stats.exported_guids += 1;
        }
        Ok(())
    }

    fn read_must_be_mapped_guids(payload: &mut BitReader<'_>, stats: &mut NetStats) -> Result<()> {
        let count = payload.read_u16()?;
        for _ in 0..count {
            let _guid = payload.read_int_packed()?;
            stats.must_be_mapped_guids += 1;
        }
        Ok(())
    }

    fn handle_channel_open(
        header: &RawBunchHeader,
        payload: &mut BitReader<'_>,
        channels: &mut HashMap<u32, ActorChannelState>,
        stats: &mut NetStats,
        pc_guids: &mut std::collections::HashSet<u32>,
        sink: &mut dyn ReplicationSink,
    ) -> Result<()> {
        let ch_index = header.ch_index;

        // Read actor net GUID (InternalLoadObject)
        let mut intercept = PathInterceptSink {
            inner: sink,
            pc_guids,
        };
        let actor_net_guid = net_guid::internal_load_object(payload, false, 0, &mut intercept)?;

        let mut state = ActorChannelState {
            channel_index: ch_index,
            is_open: true,
            is_dormant: false,
            actor_net_guid,
            archetype_net_guid: NetworkGuid(0),
            level_guid: NetworkGuid(0),
            spawn_location: None,
            spawn_rotation: None,
            spawn_scale: None,
            spawn_velocity: None,
            open_packet_id: header.packet_id,
        };

        // Dynamic actors have spawn data
        if actor_net_guid.is_dynamic() && !payload.at_end() {
            Self::read_dynamic_spawn_data(payload, &mut state, &mut intercept)?;
        }

        stats.actor_opens += 1;
        intercept.inner.on_actor_open(&state);
        channels.insert(ch_index, state);
        Ok(())
    }

    fn read_dynamic_spawn_data(
        payload: &mut BitReader<'_>,
        state: &mut ActorChannelState,
        sink: &mut PathInterceptSink<'_>,
    ) -> Result<()> {
        // Archetype -- may register a PlayerController path in pc_guids.
        state.archetype_net_guid = net_guid::internal_load_object(payload, false, 0, sink)?;
        // Level
        state.level_guid = net_guid::internal_load_object(payload, false, 0, sink)?;
        // Location -- defaults to the origin, not to absent.
        state.spawn_location = read_optional_quantized_vector(payload, 10, ORIGIN)?;
        // Rotation
        if payload.read_bit()? {
            state.spawn_rotation = Some(read_rotation_short(payload)?);
        }
        // Scale -- defaults to unit scale, not to the origin.
        state.spawn_scale = read_optional_quantized_vector(payload, 10, UNIT_SCALE)?;
        // Velocity -- only serialized when the actor class has
        // bReplicateMovement == true. PlayerController actors (the
        // BaseReplayController in VALORANT) set this to false, so their spawn
        // data omits velocity entirely. Detection relies on the archetype or
        // actor GUID having been registered as a PlayerController path in an
        // earlier package-map export bunch.
        //
        // For the very first dynamic actor (GUID 2), the package-map export
        // has not yet arrived so pc_guids is empty. Since the first dynamic
        // actor in VALORANT replays is always the BaseReplayController (which
        // does NOT replicate movement), we also skip velocity when the actor
        // GUID equals 2 -- the lowest possible dynamic GUID.
        let is_pc = sink.pc_guids.contains(&state.archetype_net_guid.0)
            || sink.pc_guids.contains(&state.actor_net_guid.0)
            || state.actor_net_guid.0 == 2;
        if !is_pc {
            state.spawn_velocity = read_optional_quantized_vector(payload, 10, ORIGIN)?;
        }
        Ok(())
    }

    fn handle_channel_close(
        header: &RawBunchHeader,
        channels: &mut HashMap<u32, ActorChannelState>,
        stats: &mut NetStats,
        sink: &mut dyn ReplicationSink,
    ) {
        let ch_index = header.ch_index;
        if let Some(ch) = channels.get_mut(&ch_index) {
            if !ch.is_open {
                return;
            }
            ch.is_open = false;
            ch.is_dormant = header.b_dormant;
            stats.actor_closes += 1;
            sink.on_actor_close(ch_index, ch.actor_net_guid, header.b_dormant);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_content_blocks(
        payload: &mut BitReader<'_>,
        ch_index: u32,
        actor_net_guid: NetworkGuid,
        transform: TransformVersion,
        scratch: &mut Vec<u8>,
        stats: &mut NetStats,
        sink: &mut dyn ReplicationSink,
        ctx: &BunchContext,
    ) {
        let total_payload_bits = payload.len_bits();
        let mut block_index: u32 = 0;

        while !payload.at_end() {
            let consumed_before_header = total_payload_bits - payload.bits_remaining();

            let header = match content::read_content_block_header(payload, actor_net_guid, sink) {
                Ok(h) => h,
                Err(_) => {
                    let remaining = payload.bits_remaining();
                    stats.skipped_bits += remaining;
                    stats.diagnostics.push(DiagnosticEvent {
                        reason: SkipReason::HeaderReadError,
                        packet_id: ctx.packet_id,
                        bunch_index_in_packet: ctx.bunch_index_in_packet,
                        global_bunch_index: ctx.global_bunch_index,
                        channel_bunch_index: ctx.channel_bunch_index,
                        channel_index: ch_index,
                        actor_net_guid: actor_net_guid.0,
                        actor_path: None,
                        archetype_net_guid: 0,
                        class_path: None,
                        bunch_flags: ctx.flags.clone(),
                        payload_bit_count: ctx.payload_bit_count,
                        consumed_bits: consumed_before_header,
                        remaining_bits: remaining,
                        content_block_header: None,
                        content_bits: None,
                        block_index_in_bunch: block_index,
                        bits_skipped: remaining,
                    });
                    payload.skip_remaining();
                    return;
                }
            };

            if header.is_deleted {
                sink.on_deleted_block(ch_index, actor_net_guid, &header);
                stats.deleted_blocks += 1;
                stats.content_blocks += 1;
                block_index += 1;
                continue;
            }

            // Read content payload bit count
            let consumed_before_bits_read = total_payload_bits - payload.bits_remaining();
            let content_bits = match payload.read_int_packed() {
                Ok(v) => v,
                Err(_) => {
                    let remaining = payload.bits_remaining();
                    stats.skipped_bits += remaining;
                    stats.diagnostics.push(DiagnosticEvent {
                        reason: SkipReason::ContentBitsReadError,
                        packet_id: ctx.packet_id,
                        bunch_index_in_packet: ctx.bunch_index_in_packet,
                        global_bunch_index: ctx.global_bunch_index,
                        channel_bunch_index: ctx.channel_bunch_index,
                        channel_index: ch_index,
                        actor_net_guid: actor_net_guid.0,
                        actor_path: None,
                        archetype_net_guid: 0,
                        class_path: None,
                        bunch_flags: ctx.flags.clone(),
                        payload_bit_count: ctx.payload_bit_count,
                        consumed_bits: consumed_before_bits_read,
                        remaining_bits: remaining,
                        content_block_header: Some(ContentBlockHeaderSnapshot::from(&header)),
                        content_bits: None,
                        block_index_in_bunch: block_index,
                        bits_skipped: remaining,
                    });
                    payload.skip_remaining();
                    return;
                }
            };

            if content_bits as u64 > payload.bits_remaining() {
                let remaining = payload.bits_remaining();
                stats.malformed_content_blocks += 1;
                stats.skipped_bits += remaining;
                stats.diagnostics.push(DiagnosticEvent {
                    reason: SkipReason::ContentBitsOverrun {
                        declared_content_bits: content_bits,
                        available_bits: remaining,
                    },
                    packet_id: ctx.packet_id,
                    bunch_index_in_packet: ctx.bunch_index_in_packet,
                    global_bunch_index: ctx.global_bunch_index,
                    channel_bunch_index: ctx.channel_bunch_index,
                    channel_index: ch_index,
                    actor_net_guid: actor_net_guid.0,
                    actor_path: None,
                    archetype_net_guid: 0,
                    class_path: None,
                    bunch_flags: ctx.flags.clone(),
                    payload_bit_count: ctx.payload_bit_count,
                    consumed_bits: total_payload_bits - remaining,
                    remaining_bits: remaining,
                    content_block_header: Some(ContentBlockHeaderSnapshot::from(&header)),
                    content_bits: Some(content_bits),
                    block_index_in_bunch: block_index,
                    bits_skipped: remaining,
                });
                payload.skip_remaining();
                return;
            }

            let function_count = sink.on_content_block(ch_index, actor_net_guid, &header);

            if content_bits == 0 {
                stats.content_blocks += 1;
                if header.has_rep_layout {
                    stats.rep_layout_blocks += 1;
                } else {
                    stats.class_net_cache_blocks += 1;
                }
                block_index += 1;
                continue;
            }

            // Apply transform and parse fields
            if header.has_rep_layout {
                stats.rep_layout_blocks += 1;
                Self::decode_and_parse_rep_layout(
                    payload,
                    content_bits as usize,
                    actor_net_guid,
                    transform,
                    scratch,
                    stats,
                    sink,
                );
            } else {
                stats.class_net_cache_blocks += 1;
                Self::decode_and_parse_class_net_cache(
                    payload,
                    content_bits as usize,
                    actor_net_guid,
                    function_count,
                    transform,
                    scratch,
                    stats,
                    sink,
                );
            }

            stats.content_blocks += 1;
            block_index += 1;
        }
    }

    fn decode_and_parse_rep_layout(
        payload: &mut BitReader<'_>,
        bit_count: usize,
        actor_net_guid: NetworkGuid,
        transform: TransformVersion,
        scratch: &mut Vec<u8>,
        stats: &mut NetStats,
        sink: &mut dyn ReplicationSink,
    ) {
        let byte_count = TransformVersion::output_byte_count(bit_count);
        if scratch.len() < byte_count {
            scratch.resize(byte_count, 0);
        }

        let seed = vrf_transform::seed_for(bit_count, actor_net_guid.0);
        if transform
            .decode_from(payload, bit_count, seed, scratch)
            .is_err()
        {
            stats.transform_failures += 1;
            stats.skipped_bits += bit_count as u64;
            return;
        }

        let mut field_reader = BitReader::with_bit_len(scratch, bit_count as u64);
        match field::parse_rep_layout(&mut field_reader, sink) {
            Ok(count) => stats.fields += count as u64,
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
                stats.field_stream_failures += 1;
                stats.skipped_bits += remaining;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_and_parse_class_net_cache(
        payload: &mut BitReader<'_>,
        bit_count: usize,
        actor_net_guid: NetworkGuid,
        function_count: u32,
        transform: TransformVersion,
        scratch: &mut Vec<u8>,
        stats: &mut NetStats,
        sink: &mut dyn ReplicationSink,
    ) {
        let byte_count = TransformVersion::output_byte_count(bit_count);
        if scratch.len() < byte_count {
            scratch.resize(byte_count, 0);
        }

        let seed = vrf_transform::seed_for(bit_count, actor_net_guid.0);
        if transform
            .decode_from(payload, bit_count, seed, scratch)
            .is_err()
        {
            stats.transform_failures += 1;
            stats.skipped_bits += bit_count as u64;
            return;
        }

        let mut rpc_reader = BitReader::with_bit_len(scratch, bit_count as u64);
        match field::parse_class_net_cache(&mut rpc_reader, function_count, sink) {
            Ok(count) => stats.rpcs += count as u64,
            Err(_) => {
                let remaining = rpc_reader.bits_remaining();
                sink.on_stream_failure(StreamFailure {
                    kind: StreamKind::Rpc,
                    actor_net_guid,
                    bit_count: bit_count as u32,
                    function_count,
                    consumed_bits: rpc_reader.position(),
                    remaining_bits: remaining,
                });
                stats.rpc_stream_failures += 1;
                stats.skipped_bits += remaining;
            }
        }
    }
}

///
/// ```text
/// Bit layout:
///   hasValue         : 1 bit
///   [if !hasValue ??return None]
///   isQuantized      : 1 bit
///   [if isQuantized]
///     componentInfo  : SerializedInt(128)
///     componentBitCount = info & 63
///     extraInfo = info >> 6
///     [if componentBitCount > 0] ??packed quantized
///     [else if extraInfo == 0]   ??3x f32
///     [else]                     ??3x f64
///   [else] ??3x f64
/// ```
/// Default spawn location and velocity when the wire omits them.
const ORIGIN: crate::types::FVector = crate::types::FVector {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// Default spawn scale when the wire omits it.
const UNIT_SCALE: crate::types::FVector = crate::types::FVector {
    x: 1.0,
    y: 1.0,
    z: 1.0,
};

/// A clear leading bit does not mean "absent" -- it means "take the default".
/// `ArchiveVectorReaders.ReadOptionalQuantizedVector` returns `defaultVector`
/// there, and `NewActorSerializer.cs:56-72` passes (0,0,0) for location and
/// velocity and (1,1,1) for scale.
///
/// Returning `None` instead collapsed that case into the genuinely-absent one:
/// a static actor never enters the spawn block at all, so its location is
/// unknown, while a dynamic actor with the bit clear has a known location of
/// exactly (0,0,0). On 02d4d478 that is 66 actors -- game state, player state,
/// vote and mission actors, which really do sit at the origin -- reported as
/// having no location alongside the 27 that truly have none.
fn read_optional_quantized_vector(
    reader: &mut BitReader<'_>,
    scale_factor: i32,
    default: crate::types::FVector,
) -> Result<Option<crate::types::FVector>> {
    if !reader.read_bit()? {
        return Ok(Some(default));
    }

    if reader.read_bit()? {
        // Quantized path
        let info = reader.read_serialized_int(128)?;
        let component_bit_count = info & 63;
        let extra_info = info >> 6;

        if component_bit_count > 0 {
            let x = reader.read_bits(component_bit_count)?;
            let y = reader.read_bits(component_bit_count)?;
            let z = reader.read_bits(component_bit_count)?;

            let sign_bit = 1u64 << (component_bit_count - 1);
            let fx = (x ^ sign_bit) as i64 - sign_bit as i64;
            let fy = (y ^ sign_bit) as i64 - sign_bit as i64;
            let fz = (z ^ sign_bit) as i64 - sign_bit as i64;

            let (dx, dy, dz) = if extra_info > 0 {
                (
                    fx as f64 / scale_factor as f64,
                    fy as f64 / scale_factor as f64,
                    fz as f64 / scale_factor as f64,
                )
            } else {
                (fx as f64, fy as f64, fz as f64)
            };

            Ok(Some(crate::types::FVector {
                x: dx,
                y: dy,
                z: dz,
            }))
        } else if extra_info == 0 {
            // 3x f32
            let x = reader.read_f32()? as f64;
            let y = reader.read_f32()? as f64;
            let z = reader.read_f32()? as f64;
            Ok(Some(crate::types::FVector { x, y, z }))
        } else {
            // 3x f64
            let x = reader.read_f64()?;
            let y = reader.read_f64()?;
            let z = reader.read_f64()?;
            Ok(Some(crate::types::FVector { x, y, z }))
        }
    } else {
        // Unquantized: 3x f64
        let x = reader.read_f64()?;
        let y = reader.read_f64()?;
        let z = reader.read_f64()?;
        Ok(Some(crate::types::FVector { x, y, z }))
    }
}

/// Read a compressed short rotator (3 components, each optionally present).
///
/// ```text
/// For each of pitch, yaw, roll:
///   hasComponent : 1 bit
///   [if hasComponent]
///     value      : u16 (16 bits)
///     degrees = value * (360.0 / 65536.0)
/// ```
fn read_rotation_short(reader: &mut BitReader<'_>) -> Result<crate::types::FRotator> {
    let pitch = read_compressed_short_component(reader)?;
    let yaw = read_compressed_short_component(reader)?;
    let roll = read_compressed_short_component(reader)?;
    Ok(crate::types::FRotator { pitch, yaw, roll })
}

fn read_compressed_short_component(reader: &mut BitReader<'_>) -> Result<f32> {
    if reader.read_bit()? {
        let raw = reader.read_u16()?;
        Ok(raw as f32 * (360.0 / 65536.0))
    } else {
        Ok(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        fields: Vec<(u32, u32)>,
        rpcs: Vec<(u32, u32)>,
        opens: Vec<u32>,
        closes: Vec<u32>,
        paths: Vec<(u32, String)>,
    }

    impl GuidPathSink for TestSink {
        fn register_path(&mut self, guid: u32, path: &str, _outer: NetworkGuid) {
            self.paths.push((guid, path.to_owned()));
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
        fn on_actor_open(&mut self, state: &ActorChannelState) {
            self.opens.push(state.channel_index);
        }
        fn on_actor_close(&mut self, channel_index: u32, _: NetworkGuid, _: bool) {
            self.closes.push(channel_index);
        }
        fn on_content_block(
            &mut self,
            _channel_index: u32,
            _actor_net_guid: NetworkGuid,
            _header: &ContentBlockHeader,
        ) -> u32 {
            0
        }
        fn on_deleted_block(
            &mut self,
            _channel_index: u32,
            _actor_net_guid: NetworkGuid,
            _header: &ContentBlockHeader,
        ) {
        }
    }

    #[test]
    fn reader_requires_valid_branch() {
        assert!(ReplicationReader::new("++Ares-Core+release-13.01").is_ok());
        assert!(ReplicationReader::new("++Ares-Core+release-99.99").is_err());
    }

    #[test]
    fn empty_packet_is_no_op() {
        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();
        reader.process_packet(&[], 0, &mut sink);
        assert_eq!(reader.stats().packets, 1);
        assert_eq!(reader.stats().bunches, 0);
    }

    #[test]
    fn malformed_packet_counted() {
        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();
        reader.process_packet(&[0x00, 0x00], 0, &mut sink);
        assert_eq!(reader.stats().malformed_packets, 1);
    }

    /// Verifies that a content-block overrun produces a DiagnosticEvent with
    /// full context (packet id, channel, bunch flags, consumed/remaining bits).
    ///
    /// This exercises the same code path as the "malformed 1 / skipped 695"
    /// structural residual found in every VALORANT replay's first bunch.
    #[test]
    fn content_bits_overrun_emits_diagnostic() {
        use crate::stats::SkipReason;

        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();

        // Build a minimal packet containing one bunch that:
        //   - Opens a channel (static actor GUID 3 = odd, non-dynamic ??no spawn data)
        //   - Has a content block header followed by a content_bits value that
        //     exceeds the remaining payload.
        //
        // Bunch header encoding (see packet.rs tests for reference):
        //   b_control=1, b_open=1, b_close=0, b_dormant=0, b_isReplicationPaused=0,
        //   b_reliable=1, ch_index via serialized_int(MAX_CHANNELS=10240),
        //   ch_sequence (reliable, int_packed), b_partial=0,
        //   payload_bit_count via serialized_int(MAX_PACKET_SIZE_BITS),
        //   then the payload itself, then the sentinel byte.
        //
        // For simplicity, we construct raw bytes that the packet reader will parse
        // into a valid bunch with a known payload. The payload contains:
        //   - Actor GUID (IntPacked 3 ??static, no spawn data)
        //   - Content block header: has_rep_layout=0, is_actor=1
        //   - Content bits (IntPacked): a value larger than remaining
        //
        // Rather than hand-encoding all the fiddly bits, we directly invoke
        // `frame_content_blocks` with a controlled reader.

        // Simulate: bunch payload = 24 bits total
        //   content block header: has_rep_layout=0 (1 bit), is_actor=1 (1 bit) ??2 bits
        //   content_bits = IntPacked(999) ??16 bits (0x7CE in IntPacked encoding)
        //   remaining after header+content_bits = 24 - 2 - 16 = 6 bits
        //   999 > 6 ??overrun
        let mut bits: Vec<bool> = Vec::new();
        // has_rep_layout = false
        bits.push(false);
        // is_actor = true
        bits.push(true);
        // IntPacked(999): 999 = 0x3E7
        //   chunk0: (999 & 0x7F) = 0x67, more=1 ??byte = (0x67 << 1) | 1 = 0xCF
        //   chunk1: (999 >> 7) = 7, more=0 ??byte = (7 << 1) | 0 = 0x0E
        let packed_bytes = [0xCF_u8, 0x0E];
        for &byte in &packed_bytes {
            for i in 0..8 {
                bits.push((byte & (1 << i)) != 0);
            }
        }
        // Add a few more padding bits so remaining > 0 but < 999
        bits.extend(std::iter::repeat_n(false, 8));

        let byte_count = bits.len().div_ceil(8);
        let mut data = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                data[i >> 3] |= 1 << (i & 7);
            }
        }

        let mut payload_reader = BitReader::with_bit_len(&data, bits.len() as u64);
        let mut stats = NetStats::default();
        let ctx = BunchContext {
            packet_id: 42,
            bunch_index_in_packet: 3,
            global_bunch_index: 100,
            channel_bunch_index: 7,
            payload_bit_count: bits.len() as i32,
            flags: BunchFlagSnapshot {
                b_open: true,
                b_close: false,
                b_reliable: true,
                b_partial: false,
                b_partial_initial: false,
                b_partial_final: false,
                b_has_package_map_exports: false,
                b_has_must_be_mapped_guids: false,
                b_dormant: false,
            },
        };

        ReplicationReader::frame_content_blocks(
            &mut payload_reader,
            5,
            NetworkGuid(42),
            reader.transform,
            &mut reader.scratch,
            &mut stats,
            &mut sink,
            &ctx,
        );

        // Verify diagnostic was emitted
        assert_eq!(stats.malformed_content_blocks, 1);
        assert_eq!(stats.diagnostics.len(), 1);

        let ev = &stats.diagnostics[0];
        assert_eq!(ev.packet_id, 42);
        assert_eq!(ev.bunch_index_in_packet, 3);
        assert_eq!(ev.global_bunch_index, 100);
        assert_eq!(ev.channel_bunch_index, 7);
        assert_eq!(ev.channel_index, 5);
        assert_eq!(ev.actor_net_guid, 42);
        assert_eq!(ev.block_index_in_bunch, 0);
        assert!(ev.content_bits.is_some());
        assert_eq!(ev.content_bits.unwrap(), 999);
        // remaining_bits should be 8 (the padding bits we added)
        assert_eq!(ev.remaining_bits, 8);
        assert_eq!(ev.bits_skipped, 8);
        match &ev.reason {
            SkipReason::ContentBitsOverrun {
                declared_content_bits,
                available_bits,
            } => {
                assert_eq!(*declared_content_bits, 999);
                assert_eq!(*available_bits, 8);
            }
            _ => panic!("expected ContentBitsOverrun"),
        }
    }
}
