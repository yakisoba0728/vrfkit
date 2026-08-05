//! Top-level replication reader that drives the full pipeline.
//!
//! This module connects packets -> bunches -> content blocks -> fields into a
//! single pass. The caller provides:
//!
//! - A [`ReplicationSink`] to receive decoded events (fields, RPCs, actor
//!   lifecycle, etc.)
//! - A replay branch string for payload transform selection
//!
//! # Layout
//!
//! The public surface (this file) is deliberately thin: the sink trait, the
//! types it exchanges, and the packet-level driver. The three stages below it
//! live in their own modules so each can be read against the wire format it
//! implements:
//!
//! | module | scope | rate on the reference replay |
//! |---|---|---|
//! | `channel` | open/close, GUID preambles | ~2 000 opens, ~1 800 closes |
//! | `spawn` | dynamic-actor spawn block | ~2 000 |
//! | `framing` | content blocks, fields, RPCs | 608 020 blocks |
//!
//! # Allocation strategy
//!
//! The steady state of this reader allocates nothing per packet, per bunch or
//! per content block. Three buffers are owned by the reader and reused for the
//! whole replay:
//!
//! - `scratch` holds one decoded content-block payload;
//! - `fragment_stage` holds one partial-bunch fragment, byte-aligned;
//! - the channel table grows once per distinct channel index (232 on the
//!   reference replay) and never per bunch.
//!
//! Bunch payloads are *views* into the caller's packet bytes:
//! `RawPacketReader` hands the framing loop a sub-reader, and content blocks
//! and fields are sub-readers of that. An earlier version copied every bunch
//! payload into a fresh `Vec<u8>` before processing it, to satisfy a borrow
//! that a field split solves instead; see [`ReplicationReader::process_packet`].

mod channel;
mod framing;
mod spawn;

use vrf_bitio::BitReader;
use vrf_transform::TransformVersion;

use crate::bunch::{PartialBunchAccumulator, RawBunchHeader};
use crate::content::ContentBlockHeader;
use crate::error::Result;
use crate::field::FieldSink;
use crate::net_guid::GuidPathSink;
use crate::packet::RawPacketReader;
use crate::stats::NetStats;
use crate::types::NetworkGuid;

use std::collections::HashMap;

use framing::BunchContext;

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

    /// Preserve one whole decoded ClassNetCache payload whose function table
    /// could not be resolved.
    ///
    /// This is a block-level data event, not a fabricated RPC. `payload` has
    /// exactly `ceil(failure.bit_count / 8)` bytes, with unused high bits in
    /// the final byte cleared. The parser has consumed no payload bits.
    fn on_unresolved_class_net_cache_payload(&mut self, _failure: StreamFailure, _payload: &[u8]) {}
}

/// Leaf asset name of the VALORANT replay controller.
///
/// This is the only `PlayerController`-kind actor in VALORANT replays, and the
/// reference parser keys the net-player-index byte off it (see
/// `channel::is_player_controller_path`, which normalises the four spellings
/// the same asset arrives under).
pub const PLAYER_CONTROLLER_LEAF: &str = "BaseReplayController";

/// One channel's row in the table.
///
/// The bunch counter used to live in a second `HashMap<u32, u64>` keyed by the
/// same channel index, so every bunch hashed that index twice. It cannot simply
/// move into [`ActorChannelState`]: a channel is counted from its first bunch,
/// which may be a close or a bunch for a channel that never opened, while only
/// an *open* bunch produces an `ActorChannelState`. Hence the `Option`.
#[derive(Default)]
struct ChannelSlot {
    /// How many bunches this channel has carried, 1-based. Reported in
    /// diagnostics as `channel_bunch_index`.
    bunch_count: u64,
    /// `None` until the channel opens.
    state: Option<ActorChannelState>,
}

/// Channel index -> channel row. 232 entries on the reference replay, looked up
/// once or twice per bunch.
type ChannelTable = HashMap<u32, ChannelSlot>;

/// Bytes the scratch buffer starts at. A bunch payload is capped at
/// `MAX_PACKET_SIZE_BITS` (16 384 bits = 2 048 bytes) and a content block
/// cannot exceed its bunch, so this is already above any block the wire can
/// declare; the `resize` in the decode path is a safety net, not a growth
/// strategy.
const SCRATCH_INITIAL_BYTES: usize = 4096;

/// The mutable state one bunch's worth of processing needs, gathered so the
/// stage functions take a handful of arguments instead of a dozen.
///
/// This is a borrow split of [`ReplicationReader`], made once per packet so the
/// bunch callback can drive the whole downstream pipeline inline.
struct Stage<'a> {
    stats: &'a mut NetStats,
    channels: &'a mut ChannelTable,
    transform: TransformVersion,
    scratch: &'a mut Vec<u8>,
}

/// Where a bunch sits in the stream. Copied rather than borrowed because all
/// three fields are scalars the caller already holds.
///
/// Every field exists to be written into a `DiagnosticEvent`, so without that
/// feature the whole struct is carried and never read. It is still threaded
/// through, rather than `cfg`-ed out of the signatures, so the hot path reads
/// the same in both builds; the values are dead and the optimiser drops them.
#[cfg_attr(not(feature = "diagnostics"), allow(dead_code))]
#[derive(Clone, Copy)]
struct BunchIds {
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
    /// Reusable byte-aligned staging buffer for partial-bunch fragments.
    ///
    /// A bunch payload arrives as a bit window at an arbitrary offset inside
    /// the packet, but [`PartialBunchAccumulator::add_fragment`] concatenates
    /// byte slices, so a fragment has to be realigned before it can be
    /// appended. That copy is unavoidable; allocating for it is not.
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

        // Bunches are processed inline, inside the packet reader's callback.
        //
        // This used to be two phases: parse every bunch header, copying each
        // payload into a fresh `Vec<u8>`, and only then walk the copies. The
        // stated reason was that "the callback borrows self". It does not have
        // to. Destructuring `self` here gives the callback `&mut` handles to
        // the fields it needs while `packet_reader` stays borrowed by
        // `read_packet`, which the borrow checker accepts because the fields
        // are disjoint.
        //
        // What the copies cost: 530 401 bunches on the reference replay, one
        // `vec![0u8; n]` each (zero-fill, then `copy_bits_to` overwrote the
        // same bytes), plus one `Vec` per packet for the staging list. About
        // 1.06 million allocate/free pairs and two passes over ~108 MB of
        // payload, to hand the framing loop bits it could already see.
        //
        // Interleaving is safe because the two phases touch disjoint state:
        // header parsing mutates only `packet_reader` (partial tracking and the
        // reliable sequence), while payload processing mutates only the fields
        // below. `malformed_packets` is bumped between the phases but summed
        // after the loop -- a u64 nothing reads mid-stream, so its total is
        // unchanged. (`partial_errors` is owned by the reassembly accumulator's
        // `validate_sequence` in `process_bunch`; the reader's parallel partial
        // tracker no longer adds its count here -- doing both counted every
        // error twice.)
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

            // The global index is the value *before* the increment and the
            // per-channel one the value *after*; both feed `DiagnosticEvent`
            // and the asymmetry is what the fields have always meant.
            let global_index = *global_bunch_index;
            *global_bunch_index += 1;
            let slot = stage.channels.entry(header.ch_index).or_default();
            slot.bunch_count += 1;
            let ids = BunchIds {
                bunch_index_in_packet,
                global_bunch_index: global_index,
                channel_bunch_index: slot.bunch_count,
            };
            bunch_index_in_packet += 1;

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
        // `partial_errors` is counted by `validate_sequence` in `process_bunch`
        // (the reassembly authority). The reader's own `partial_error_count` is
        // NOT summed here -- doing both counted every partial error twice.
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
            let byte_count = stage_fragment(payload, fragment_stage);

            let result = accumulator.add_fragment(
                ch_index,
                header.clone(),
                &fragment_stage[..byte_count],
                bit_count as usize,
                &mut stage.stats.partial_errors,
                &mut stage.stats.partial_fragments,
                &mut stage.stats.partial_completed,
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
                    stage,
                    sink,
                    ids,
                );
            }
            return;
        }

        // Non-partial: process directly
        if bit_count == 0 {
            // Handle close
            if header.b_close {
                channel::handle_channel_close(header, stage.channels, stage.stats, sink);
            }
            return;
        }

        let mut payload = payload;
        Self::process_complete_payload(header, &mut payload, stage, sink, ids);

        if header.b_close {
            channel::handle_channel_close(header, stage.channels, stage.stats, sink);
        }
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

        // Package map exports. Count the export only when the read succeeds --
        // a partial failure used to inflate `package_map_exports` anyway.
        if header.b_has_package_map_exports {
            if channel::read_package_map_exports(payload, stage.stats, sink).is_ok() {
                stage.stats.package_map_exports += 1;
            } else {
                stage.stats.bunch_header_failures += 1;
            }
            return;
        }

        // Must-be-mapped GUIDs. A failure leaves the reader at an indeterminate
        // bit, so the rest of this bunch cannot be framed safely -- count it and
        // abandon the bunch rather than parse on as garbage.
        if header.b_has_must_be_mapped_guids
            && channel::read_must_be_mapped_guids(payload, stage.stats).is_err()
        {
            stage.stats.bunch_header_failures += 1;
            return;
        }

        // Actor channel open. On failure the channel is never inserted, so the
        // guard below skips the rest of the bunch; the count makes that skip
        // visible instead of looking like a channel that was never used.
        if header.b_open
            && channel::handle_channel_open(header, payload, stage.channels, stage.stats, sink)
                .is_err()
        {
            stage.stats.bunch_header_failures += 1;
        }

        // Look up channel -- if it never opened, or is not open now, skip.
        let Some(ch) = stage.channels.get(&ch_index).and_then(|s| s.state.as_ref()) else {
            return;
        };
        let (actor_net_guid, is_open, archetype_net_guid) =
            (ch.actor_net_guid, ch.is_open, ch.archetype_net_guid);

        if !is_open {
            return;
        }

        // ReadNetPlayerIndex. The three cheap flags are tested before the path
        // lookup, which is the whole reason this reads as a nest rather than a
        // flat `&&` chain: `is_player_controller_channel` costs two NetGuidCache
        // lookups plus two path normalisations, and it used to run on every one
        // of the 530 401 bunches to answer a question that decides something for
        // about 2 000 of them. See [`channel::is_player_controller_channel`] for
        // what the byte is and what skipping it costs.
        if header.b_open
            && actor_net_guid.is_dynamic()
            && !payload.at_end()
            && channel::is_player_controller_channel(actor_net_guid, archetype_net_guid, sink)
        {
            let _ = payload.read_u8();
        }

        // Frame content blocks
        let ctx = BunchContext { header, ids };
        framing::frame_content_blocks(payload, ch_index, actor_net_guid, stage, sink, &ctx);
    }
}

/// Copy a partial bunch's payload into `buffer`, byte-aligned, and return how
/// many bytes it occupies.
///
/// [`PartialBunchAccumulator::add_fragment`] concatenates byte slices, but a
/// bunch payload is a bit window starting at an arbitrary offset inside the
/// packet, so a fragment must be realigned first. `buffer` is the reader's
/// long-lived staging area: it only ever grows, and the bytes past the returned
/// length belong to whatever fragment used it last. `copy_bits_to` rewrites
/// every byte of `[..byte_count]` -- including the padding bits of the final
/// one -- so the prefix the caller slices is never stale.
fn stage_fragment(payload: BitReader<'_>, buffer: &mut Vec<u8>) -> usize {
    let bit_count = payload.bits_remaining();
    let byte_count = (bit_count as usize).div_ceil(8);
    if buffer.len() < byte_count {
        buffer.resize(byte_count, 0);
    }
    if bit_count > 0 {
        let mut src = payload;
        let _ = src.copy_bits_to(buffer, bit_count);
    }
    byte_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        fields: Vec<(u32, u32)>,
        rpcs: Vec<(u32, u32)>,
        unresolved_payloads: Vec<(StreamFailure, Vec<u8>)>,
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

        fn on_unresolved_class_net_cache_payload(
            &mut self,
            failure: StreamFailure,
            payload: &[u8],
        ) {
            self.unresolved_payloads.push((failure, payload.to_vec()));
        }
    }

    /// The staging buffer is reused for every partial fragment and only ever
    /// grows, so a short fragment following a long one must not be able to read
    /// the long one's bytes back. This is the failure mode the buffer reuse
    /// could have introduced; it is pinned rather than argued.
    #[test]
    fn fragment_staging_never_exposes_the_previous_fragment() {
        let mut buffer = Vec::new();

        let long = [0xFFu8; 4];
        let byte_count = stage_fragment(BitReader::with_bit_len(&long, 32), &mut buffer);
        assert_eq!(byte_count, 4);
        assert_eq!(&buffer[..byte_count], &[0xFF, 0xFF, 0xFF, 0xFF]);

        // Five zero bits: one byte, and the three padding bits above them must
        // be cleared even though the buffer still holds 0xFF underneath.
        let short = [0x00u8];
        let byte_count = stage_fragment(BitReader::with_bit_len(&short, 5), &mut buffer);
        assert_eq!(byte_count, 1);
        assert_eq!(&buffer[..byte_count], &[0x00]);
        assert_eq!(
            buffer.len(),
            4,
            "the buffer keeps its capacity, not its data"
        );

        // A zero-bit fragment stages nothing and must report zero bytes.
        assert_eq!(
            stage_fragment(BitReader::with_bit_len(&short, 0), &mut buffer),
            0
        );
    }

    // --- packet builders, for the reassembly test below ---

    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max_value: u32) {
        let mut written_value = 0u32;
        let mut mask = 1u32;
        while written_value.saturating_add(mask) < max_value {
            let bit = (value & mask) != 0;
            bits.push(bit);
            if bit {
                written_value |= mask;
            }
            mask <<= 1;
        }
    }

    fn build_packet(bits: &[bool]) -> Vec<u8> {
        let mut packet = vec![0u8; (bits.len() + 1).div_ceil(8)];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                packet[i >> 3] |= 1 << (i & 7);
            }
        }
        packet[bits.len() >> 3] |= 1 << (bits.len() & 7);
        packet
    }

    /// A partial bunch split across two fragments must reassemble and then
    /// frame exactly as an unsplit one would.
    ///
    /// The reference replay completes zero partial bunches -- an instrumented
    /// run reported `partial_fragments = 0`, `partial_completed = 0` against
    /// 530 401 bunches -- so the oracle cannot see this path at all. It is the
    /// one place where a bunch payload is copied rather than viewed, which
    /// makes it exactly the path a rewrite of the copy could break silently.
    #[test]
    fn split_bunch_reassembles_and_frames() {
        // Fragment 1: control bunch, opens channel 2, partial initial,
        // reliable. Payload is IntPacked(3): a static (odd) actor GUID, so no
        // spawn block follows.
        let mut bits = vec![
            true,  // bControl
            true,  // bOpen
            false, // bClose
            false, // bIsReplicationPaused
            true,  // bReliable
        ];
        write_int_packed(&mut bits, 2); // ChIndex
        bits.extend_from_slice(&[
            false, // bHasPackageMapExports
            false, // bHasMustBeMappedGUIDs
            true,  // bPartial
            true,  // bPartialInitial
            false, // bPartialFinal
            false, // VALORANT bit
            true,  // FName isHardcoded
        ]);
        write_int_packed(&mut bits, 1); // FName index
        write_serialized_int(&mut bits, 8, crate::types::MAX_PACKET_SIZE_BITS);
        write_int_packed(&mut bits, 3); // payload: actor GUID 3

        // Fragment 2: partial final on the same channel. Payload is one actor
        // content block with a zero-bit body.
        bits.extend_from_slice(&[
            false, // bControl
            false, // bIsReplicationPaused
            true,  // bReliable
        ]);
        write_int_packed(&mut bits, 2); // ChIndex
        bits.extend_from_slice(&[
            false, // bHasPackageMapExports
            false, // bHasMustBeMappedGUIDs
            true,  // bPartial
            false, // bPartialInitial
            true,  // bPartialFinal
            false, // VALORANT bit
            true,  // FName isHardcoded
        ]);
        write_int_packed(&mut bits, 1); // FName index
        write_serialized_int(&mut bits, 10, crate::types::MAX_PACKET_SIZE_BITS);
        bits.extend_from_slice(&[
            true, // hasRepLayout
            true, // isActor
        ]);
        write_int_packed(&mut bits, 0); // contentBits = 0

        let packet = build_packet(&bits);

        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();
        reader.process_packet(&packet, 0, &mut sink);

        let stats = reader.stats();
        assert_eq!(stats.bunches, 2);
        assert_eq!(stats.partial_fragments, 2);
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
    }

    /// A partial final with no preceding initial is one error, counted once.
    /// It used to be counted twice: once by the packet reader's parallel
    /// partial-state tracker (summed into `partial_errors` at packet end) and
    /// again by the reassembly accumulator's `validate_sequence`. The
    /// accumulator is the reassembly authority, so it owns the count.
    #[test]
    fn a_partial_final_without_an_initial_is_counted_once() {
        // One partial-final bunch on channel 2, reliable, with no initial
        // fragment anywhere. Both the reader and the accumulator detect the
        // missing initial; only the accumulator should count it.
        let mut bits = vec![
            false, // bControl
            false, // bIsReplicationPaused
            true,  // bReliable
        ];
        write_int_packed(&mut bits, 2); // ChIndex
        bits.extend_from_slice(&[
            false, // bHasPackageMapExports
            false, // bHasMustBeMappedGUIDs
            true,  // bPartial
            false, // bPartialInitial (no initial -> MissingInitial)
            true,  // bPartialFinal
            false, // VALORANT bit
            true,  // FName isHardcoded
        ]);
        write_int_packed(&mut bits, 1); // FName index
        write_serialized_int(&mut bits, 8, crate::types::MAX_PACKET_SIZE_BITS);
        write_int_packed(&mut bits, 3); // payload: actor GUID 3 (never reached)

        let packet = build_packet(&bits);
        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();
        reader.process_packet(&packet, 0, &mut sink);

        assert_eq!(
            reader.stats().partial_errors,
            1,
            "one missing-initial error, counted once (not twice)"
        );
    }

    /// A bunch whose header parse fails is counted and abandoned, not silently
    /// dropped. A truncated must-be-mapped GUID list (declares one GUID, carries
    /// none) used to be `let _ =`-ed: the reader was left stuck and the rest of
    /// the bunch (channel open, content framing) parsed garbage, with no counter
    /// moving.
    #[test]
    fn a_truncated_bunch_header_failure_is_counted_not_silent() {
        // Non-partial reliable bunch on channel 2 with a 16-bit payload: a u16
        // must-be-mapped count of 1 (LE) and then no GUID bits, so
        // read_must_be_mapped_guids EOFs on the missing GUID.
        let mut bits = vec![
            false, // bControl
            false, // bIsReplicationPaused
            true,  // bReliable
        ];
        write_int_packed(&mut bits, 2); // ChIndex
        bits.extend_from_slice(&[
            false, // bHasPackageMapExports
            true,  // bHasMustBeMappedGUIDs
            false, // bPartial
            false, // VALORANT bit
            true,  // FName isHardcoded
        ]);
        write_int_packed(&mut bits, 1); // FName index
        write_serialized_int(&mut bits, 16, crate::types::MAX_PACKET_SIZE_BITS); // 16 payload bits
        // payload: u16 count = 1 little-endian, then zero GUID bits.
        bits.extend_from_slice(&[
            true, false, false, false, false, false, false, false, // 0x01
            false, false, false, false, false, false, false, false, // 0x00
        ]);

        let packet = build_packet(&bits);
        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();
        reader.process_packet(&packet, 0, &mut sink);

        assert_eq!(
            reader.stats().bunch_header_failures,
            1,
            "truncated must-be-mapped list counted, not silently dropped"
        );
    }

    /// An unaligned window is realigned to bit zero of the staging buffer --
    /// that realignment is the only reason the copy exists.
    #[test]
    fn fragment_staging_realigns_an_offset_window() {
        let packet = [0b1111_0000u8, 0b0000_1111];
        let mut reader = BitReader::new(&packet);
        reader.skip_bits(4).unwrap();
        let window = reader.sub_reader(8).unwrap();

        let mut buffer = Vec::new();
        assert_eq!(stage_fragment(window, &mut buffer), 1);
        assert_eq!(buffer[0], 0b1111_1111);
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

    /// The unresolved callback receives the exact decoded block, not the wire
    /// bytes or the reusable scratch tail. The 7-bit literal is a golden V13.01
    /// transform vector: wire 0xBF for actor 2 decodes to 0x66.
    #[test]
    fn unresolved_class_net_cache_exposes_the_decoded_whole_payload() {
        let wire = [0xBF];
        let mut payload = BitReader::with_bit_len(&wire, 7);
        let mut scratch = vec![0xFF; 16];
        let mut stats = NetStats::default();
        let mut channels = ChannelTable::default();
        let mut sink = TestSink::default();
        let mut stage = Stage {
            stats: &mut stats,
            channels: &mut channels,
            transform: TransformVersion::V1301,
            scratch: &mut scratch,
        };

        framing::decode_and_parse_class_net_cache(
            &mut payload,
            7,
            NetworkGuid(2),
            0,
            &mut stage,
            &mut sink,
        );

        assert_eq!(payload.position(), 7);
        assert_eq!(sink.unresolved_payloads.len(), 1);
        let (failure, decoded) = &sink.unresolved_payloads[0];
        assert_eq!(failure.kind, StreamKind::Rpc);
        assert_eq!(failure.actor_net_guid, NetworkGuid(2));
        assert_eq!(failure.bit_count, 7);
        assert_eq!(failure.function_count, 0);
        assert_eq!(failure.consumed_bits, 0);
        assert_eq!(failure.remaining_bits, 7);
        assert_eq!(decoded, &[0x66]);
        assert_eq!(decoded[0] >> 7, 0, "high padding bit must stay zero");
        assert_eq!(stats.rpcs, 0);
        assert_eq!(stats.rpc_stream_failures, 1);
        assert_eq!(stats.skipped_bits, 7);
    }

    /// A field stream that returns Ok but abandons bits mid-block must land
    /// those bits in `skipped_bits`. Before the fix, `parse_class_net_cache`
    /// did `reader.skip_remaining(); break; return Ok(count)` and the framing
    /// layer only counted `skipped_bits` on Err, so the abandoned bits vanished
    /// from every counter.
    ///
    /// Same golden V13.01 vector as the unresolved test (wire 0xBF -> 0x66 for
    /// actor 2), but `function_count=2` so the parser walks the stream instead
    /// of bailing with UnresolvedFunctionCount. Decoded 0x66 yields one handle
    /// bit (handle=0) then leaves 6 bits -- fewer than the 8 an IntPacked
    /// payload-length read needs -- so the stream returns Ok(0) after skipping
    /// those 6 bits. They must be accounted.
    #[test]
    fn class_net_cache_overrun_ok_path_counts_skipped_bits() {
        let wire = [0xBF];
        let mut payload = BitReader::with_bit_len(&wire, 7);
        let mut scratch = vec![0xFF; 16];
        let mut stats = NetStats::default();
        let mut channels = ChannelTable::default();
        let mut sink = TestSink::default();
        let mut stage = Stage {
            stats: &mut stats,
            channels: &mut channels,
            transform: TransformVersion::V1301,
            scratch: &mut scratch,
        };

        framing::decode_and_parse_class_net_cache(
            &mut payload,
            7,
            NetworkGuid(2),
            2,
            &mut stage,
            &mut sink,
        );

        assert_eq!(payload.position(), 7);
        // Ok path: the block was processed, just malformed internally, so no
        // hard stream failure is recorded.
        assert_eq!(stats.rpc_stream_failures, 0);
        assert_eq!(stats.rpcs, 0);
        // The 6 abandoned bits must be counted, not silently dropped.
        assert!(
            stats.skipped_bits >= 6,
            "abandoned mid-block bits must land in skipped_bits, got {}",
            stats.skipped_bits
        );
    }

    /// Verifies that a content-block overrun produces a DiagnosticEvent with
    /// full context (packet id, channel, bunch flags, consumed/remaining bits).
    ///
    /// This exercises the same code path as the "malformed 1 / skipped 695"
    /// structural residual found in every VALORANT replay's first bunch.
    #[cfg(feature = "diagnostics")]
    #[test]
    fn content_bits_overrun_emits_diagnostic() {
        use crate::stats::SkipReason;

        let mut reader = ReplicationReader::new("++Ares-Core+release-13.01").unwrap();
        let mut sink = TestSink::default();

        // Simulate: bunch payload = 24 bits total
        //   content block header: has_rep_layout=0 (1 bit), is_actor=1 (1 bit) -> 2 bits
        //   content_bits = IntPacked(999) -> 16 bits (0x7CE in IntPacked encoding)
        //   remaining after header+content_bits = 24 - 2 - 16 = 6 bits
        //   999 > 6 -> overrun
        let mut bits: Vec<bool> = Vec::new();
        // has_rep_layout = false
        bits.push(false);
        // is_actor = true
        bits.push(true);
        // IntPacked(999): 999 = 0x3E7
        //   chunk0: (999 & 0x7F) = 0x67, more=1 -> byte = (0x67 << 1) | 1 = 0xCF
        //   chunk1: (999 >> 7) = 7, more=0 -> byte = (7 << 1) | 0 = 0x0E
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
        let mut channels = ChannelTable::default();
        let header = RawBunchHeader {
            packet_id: 42,
            b_open: true,
            b_reliable: true,
            payload_bit_count: bits.len() as i32,
            ..Default::default()
        };
        let ctx = BunchContext {
            header: &header,
            ids: BunchIds {
                bunch_index_in_packet: 3,
                global_bunch_index: 100,
                channel_bunch_index: 7,
            },
        };
        let mut stage = Stage {
            stats: &mut stats,
            channels: &mut channels,
            transform: reader.transform,
            scratch: &mut reader.scratch,
        };

        framing::frame_content_blocks(
            &mut payload_reader,
            5,
            NetworkGuid(42),
            &mut stage,
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
        assert!(
            ev.bunch_flags.b_open && ev.bunch_flags.b_reliable && !ev.bunch_flags.b_partial,
            "flags are snapshotted from the bunch header on the failure path"
        );
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
