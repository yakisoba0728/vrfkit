//! Raw packet reading: sentinel-based bit sizing and bunch header extraction.
//!
//! Unreal pads a packet to a byte boundary and marks its true end with a
//! sentinel `1` bit (see `compute_bit_size`); a zero last byte has none and
//! the packet is malformed.

use vrf_bitio::BitReader;

use crate::bunch::{RawBunchHeader, continues};
use crate::error::{PartialSequenceKind, Result};
use crate::types::{ChannelCloseReason, MAX_ACTIVE_CHANNELS, MAX_PACKET_SIZE_BITS};

use vrf_schema::FxHashMap;

/// Result of reading one packet.
///
/// [`crate::ReplicationReader`] reads only `is_malformed`; the rest is for
/// direct callers of the public [`RawPacketReader::read_packet`].
#[derive(Debug, Clone)]
pub struct PacketReadResult {
    /// Number of bunches successfully parsed from this packet.
    pub bunch_count: u32,
    /// Whether the packet was malformed: no sentinel, a bunch header that
    /// does not read, or a payload overrun.
    pub is_malformed: bool,
    /// Errors found by the advisory partial tracker (see [`RawPacketReader`]);
    /// not [`crate::NetStats::partial_errors`], and never summed into it.
    pub partial_error_count: u32,
    /// Bunches refused because per-channel state could not be admitted or advanced.
    pub channel_limit_count: u32,
}

/// Per-channel state for partial bunch tracking within the packet reader.
#[derive(Debug, Clone)]
struct PartialState {
    ch_sequence: i32,
    reliable: bool,
}

/// Stateful packet reader, one per replay stream: per-channel partial-bunch
/// tracking and reliable sequence numbers.
///
/// The reliable sequence is per channel, like Unreal's `ReliableSequence` (a
/// global counter diverges when channels interleave reliable bunches); it
/// reaches the pipeline's reassembly accumulator as `ch_sequence`.
///
/// The partial tracker is advisory: the pipeline strips its outputs
/// (`has_partial_error`, `is_partial_completed`,
/// [`PacketReadResult::partial_error_count`]) and its accumulator is the one
/// partial authority. It stays because `read_packet` is public; without it
/// that count would be a permanent 0.
pub struct RawPacketReader {
    partial_bunches: FxHashMap<u32, PartialState>,
    in_reliable_sequence: FxHashMap<u32, i32>,
    max_channels: usize,
}

impl RawPacketReader {
    /// Create a fresh reader with no channel state.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_channels(MAX_ACTIVE_CHANNELS)
    }

    pub(crate) fn with_max_channels(max_channels: usize) -> Self {
        Self {
            partial_bunches: FxHashMap::default(),
            in_reliable_sequence: FxHashMap::default(),
            max_channels,
        }
    }

    /// Parse all bunches from one packet's bytes, handing `callback` each
    /// parsed header and an owned sub-reader over exactly its payload bits.
    pub fn read_packet<F>(
        &mut self,
        packet_data: &[u8],
        packet_id: i32,
        mut callback: F,
    ) -> PacketReadResult
    where
        F: FnMut(&mut RawBunchHeader, BitReader<'_>),
    {
        let mut result = PacketReadResult {
            bunch_count: 0,
            is_malformed: false,
            partial_error_count: 0,
            channel_limit_count: 0,
        };

        let Some(&last_byte) = packet_data.last() else {
            return result;
        };
        if last_byte == 0 {
            result.is_malformed = true;
            return result;
        }

        let Some(bit_size) = compute_bit_size(packet_data.len(), last_byte) else {
            result.is_malformed = true;
            return result;
        };
        let Ok(mut reader) = BitReader::with_bit_len(packet_data, bit_size) else {
            result.is_malformed = true;
            return result;
        };

        while !reader.at_end() {
            let Ok(mut header) = self.parse_bunch_header(&mut reader, packet_id) else {
                result.is_malformed = true;
                return result;
            };
            if header.has_channel_limit_error {
                result.channel_limit_count += 1;
            }

            if header.payload_bit_count as u64 > reader.bits_remaining() {
                result.is_malformed = true;
                return result;
            }

            if !header.has_channel_limit_error {
                self.track_partial_bunch(&mut header, &mut result.partial_error_count);
            }

            let payload = reader
                .sub_reader(header.payload_bit_count as u64)
                .expect("bounds already checked");

            callback(&mut header, payload);
            result.bunch_count += 1;
            if header.b_close && !header.b_dormant {
                self.retire_channel(header.ch_index);
            }
        }

        result
    }

    /// Parse a single bunch header from the bit stream.
    ///
    /// ```text
    /// Bit layout (VALORANT replay):
    /// +--------------------------------------------------------------+
    /// | bControl           : 1 bit                                   |
    /// | [if bControl]                                                |
    /// |   bOpen            : 1 bit                                   |
    /// |   bClose           : 1 bit                                   |
    /// |   [if bClose]                                                |
    /// |     CloseReason    : SerializedInt(15)                       |
    /// | bIsReplicationPaused : 1 bit                                 |
    /// | bReliable          : 1 bit                                   |
    /// | ChIndex            : IntPacked                               |
    /// | bHasPackageMapExports : 1 bit                                |
    /// | bHasMustBeMappedGUIDs : 1 bit                                |
    /// | bPartial           : 1 bit                                   |
    /// | <VALORANT>         : 1 bit (meaning unknown; discarded)      |
    /// | [if bPartial]                                                |
    /// |   bPartialInitial  : 1 bit                                   |
    /// |   bPartialFinal    : 1 bit                                   |
    /// | [if bReliable || bOpen]                                      |
    /// |   ChName           : FName (1 bit isHardcoded + IntPacked)   |
    /// | PayloadBitCount    : SerializedInt(16384)                    |
    /// +--------------------------------------------------------------+
    /// ```
    fn parse_bunch_header(
        &mut self,
        reader: &mut BitReader<'_>,
        packet_id: i32,
    ) -> Result<RawBunchHeader> {
        let mut header = RawBunchHeader {
            packet_id,
            ..Default::default()
        };

        let b_control = reader.read_bit()?;
        if b_control {
            header.b_open = reader.read_bit()?;
            header.b_close = reader.read_bit()?;
        }

        if header.b_close {
            let raw = reader.read_serialized_int(ChannelCloseReason::MAX)?;
            header.close_reason = ChannelCloseReason::from_raw(raw);
            header.b_dormant = header.close_reason == ChannelCloseReason::Dormancy;
        }

        header.b_is_replication_paused = reader.read_bit()?;
        header.b_reliable = reader.read_bit()?;
        header.ch_index = reader.read_int_packed()?;
        header.b_has_package_map_exports = reader.read_bit()?;
        header.b_has_must_be_mapped_guids = reader.read_bit()?;
        header.b_partial = reader.read_bit()?;

        if header.b_reliable {
            // No wrap rule is established: overflow refuses the bunch rather
            // than panicking (debug) or inventing a wrapped sequence (release).
            header.ch_sequence = match self.in_reliable_sequence.get(&header.ch_index).copied() {
                Some(previous) => match previous.checked_add(1) {
                    Some(next) => next,
                    None => {
                        header.has_channel_limit_error = true;
                        previous
                    }
                },
                None => 1,
            };
        } else if header.b_partial {
            header.ch_sequence = packet_id;
        }

        // Always present after bPartial: its position is corpus-verified,
        // its meaning is not.
        let _valorant_bit = reader.read_bit()?;

        if header.b_partial {
            header.b_partial_initial = reader.read_bit()?;
            header.b_partial_final = reader.read_bit()?;
        }

        // Channel FName, present when reliable or opening: read, not needed.
        if header.b_reliable || header.b_open {
            let _is_hardcoded = reader.read_bit()?;
            let _name_index = reader.read_int_packed()?;
        }

        header.payload_bit_count = reader.read_serialized_int(MAX_PACKET_SIZE_BITS)? as i32;
        header.payload_bit_offset = reader.position() as i64;

        if header.b_reliable && !header.has_channel_limit_error {
            if !self.in_reliable_sequence.contains_key(&header.ch_index)
                && self.in_reliable_sequence.len() >= self.max_channels
            {
                header.has_channel_limit_error = true;
            } else {
                self.in_reliable_sequence
                    .insert(header.ch_index, header.ch_sequence);
            }
        }

        Ok(header)
    }

    /// Track partial bunch state across fragments; advisory, see
    /// [`RawPacketReader`].
    fn track_partial_bunch(&mut self, header: &mut RawBunchHeader, partial_error_count: &mut u32) {
        if !header.b_partial {
            return;
        }

        if header.b_partial_initial {
            let overlapping = self.partial_bunches.contains_key(&header.ch_index);

            // An initial that is also final is a whole bunch: nothing is left
            // in flight, so no state is admitted or kept for it.
            if header.b_partial_final {
                if overlapping {
                    *partial_error_count += 1;
                    header.has_partial_error = true;
                }
                self.partial_bunches.remove(&header.ch_index);
                header.is_partial_completed = !header.has_partial_error;
                return;
            }

            if !self.partial_bunches.contains_key(&header.ch_index)
                && self.partial_bunches.len() >= self.max_channels
            {
                *partial_error_count += 1;
                header.has_partial_error = true;
                return;
            }
            if overlapping {
                *partial_error_count += 1;
                header.has_partial_error = true;
            }

            self.partial_bunches.insert(
                header.ch_index,
                PartialState {
                    ch_sequence: header.ch_sequence,
                    reliable: header.b_reliable,
                },
            );
            return;
        }

        // Continuation or final
        if let Some(kind) = self.validate_continuation(header) {
            *partial_error_count += 1;
            header.has_partial_error = true;
            if kind == PartialSequenceKind::MismatchedContinuation {
                self.partial_bunches.remove(&header.ch_index);
            }
            return;
        }

        if let Some(state) = self.partial_bunches.get_mut(&header.ch_index) {
            state.ch_sequence = header.ch_sequence;
            if header.b_partial_final {
                header.is_partial_completed = true;
            }
        }
        if header.b_partial_final {
            self.partial_bunches.remove(&header.ch_index);
        }
    }

    fn validate_continuation(&self, header: &RawBunchHeader) -> Option<PartialSequenceKind> {
        let Some(state) = self.partial_bunches.get(&header.ch_index) else {
            return Some(PartialSequenceKind::MissingInitial);
        };
        let ok = state.reliable == header.b_reliable
            && continues(state.reliable, state.ch_sequence, header.ch_sequence);
        (!ok).then_some(PartialSequenceKind::MismatchedContinuation)
    }

    fn retire_channel(&mut self, ch_index: u32) {
        self.partial_bunches.remove(&ch_index);
        self.in_reliable_sequence.remove(&ch_index);
    }
}

impl Default for RawPacketReader {
    fn default() -> Self {
        Self::new()
    }
}

/// The bits below the sentinel, the last byte's highest `1` bit: the byte's
/// leading zeros are padding. The caller rejects a zero last byte, so the
/// count is at most 7. Checked wide arithmetic also protects direct callers
/// whose packets did not pass through the frame reader's small-packet limit.
fn compute_bit_size(byte_count: usize, last_byte: u8) -> Option<u64> {
    debug_assert!(last_byte != 0, "caller must reject a zero last byte");
    u64::try_from(byte_count)
        .ok()?
        .checked_mul(8)?
        .checked_sub(1 + u64::from(last_byte.leading_zeros()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bunch::RawBunchHeader;
    use crate::test_bits::{
        BunchSpec, build_bunch_packet, build_packet, write_bunch, write_bunch_header,
    };

    #[test]
    fn packet_bit_size_keeps_wide_lengths_and_exact_sentinel_positions() {
        for byte_count in [1, 2048, 268_435_456, 268_435_457] {
            for sentinel in 0..8 {
                let expected = ((byte_count as u64 - 1) * 8) + sentinel;
                assert_eq!(compute_bit_size(byte_count, 1 << sentinel), Some(expected));
            }
        }
    }

    /// An unreliable bunch on `ch_index` with no other flag set: no control
    /// bits and no channel name.
    fn unreliable(ch_index: u32) -> BunchSpec {
        BunchSpec {
            ch_index,
            unreliable: true,
            ..Default::default()
        }
    }

    /// A reliable bunch on `ch_index` with no other flag set.
    fn reliable(ch_index: u32) -> BunchSpec {
        BunchSpec {
            ch_index,
            ..Default::default()
        }
    }

    /// A reliable partial fragment on `ch_index`.
    fn fragment(ch_index: u32, initial: bool, last: bool) -> BunchSpec {
        BunchSpec {
            ch_index,
            b_partial: true,
            b_partial_initial: initial,
            b_partial_final: last,
            ..Default::default()
        }
    }

    #[test]
    fn last_byte_zero_returns_malformed() {
        let mut reader = RawPacketReader::new();
        let result = reader.read_packet(&[0x00, 0x00, 0x00], 0, |_, _| {});
        assert!(result.is_malformed);
        assert_eq!(result.bunch_count, 0);
    }

    #[test]
    fn empty_data_returns_zero_bunches() {
        let mut reader = RawPacketReader::new();
        let result = reader.read_packet(&[], 0, |_, _| {});
        assert_eq!(result.bunch_count, 0);
        assert!(!result.is_malformed);
    }

    #[test]
    fn single_bunch_parses_header_fields() {
        let packet = build_bunch_packet(&unreliable(7), &[]);

        let mut reader = RawPacketReader::new();
        let mut captured: Option<RawBunchHeader> = None;
        reader.read_packet(&packet, 2, |h, _| captured = Some(h.clone()));

        let h = captured.unwrap();
        assert_eq!(h.packet_id, 2);
        assert_eq!(h.ch_index, 7);
        assert!(!h.b_open);
        assert!(!h.b_close);
        assert!(!h.b_reliable);
        assert!(!h.b_partial);
        assert_eq!(h.payload_bit_count, 0);
    }

    #[test]
    fn control_bunch_with_close_parses_close_reason() {
        let close = BunchSpec {
            b_close: true,
            dormant: true,
            ..unreliable(3)
        };
        let packet = build_bunch_packet(&close, &[]);

        let mut reader = RawPacketReader::new();
        let mut captured: Option<RawBunchHeader> = None;
        reader.read_packet(&packet, 1, |h, _| captured = Some(h.clone()));

        let h = captured.unwrap();
        assert!(!h.b_open);
        assert!(h.b_close);
        assert!(h.b_dormant);
        assert_eq!(h.close_reason, ChannelCloseReason::Dormancy);
    }

    #[test]
    fn multiple_bunches_parsed() {
        // A 17-bit first payload: the next header starts off a byte boundary.
        let mut bits = Vec::new();
        write_bunch(&mut bits, &unreliable(0), &[false; 17]);
        write_bunch(&mut bits, &unreliable(1), &[]);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut indices = Vec::new();
        let result = reader.read_packet(&packet, 3, |h, _| indices.push(h.ch_index));
        assert_eq!(indices, vec![0, 1]);
        assert!(!result.is_malformed);
    }

    #[test]
    fn captured_partial_headers_assign_boundary_bits_in_wire_order() {
        // Header prefixes captured from 13.05 packets 790-792 (the initial is
        // byte-identical on 13.01), not made by the test writers.
        let fixtures = [
            (&[0x10, 0xa0, 0x90, 0x7b][..], true, false, 15_816),
            (&[0x10, 0x20, 0x90, 0x7b][..], false, false, 15_816),
            (&[0x10, 0x20, 0x67, 0x93][..], false, true, 2_483),
        ];

        for (packet_id, (bytes, initial, is_final, payload_bits)) in
            (790..).zip(fixtures.into_iter())
        {
            let mut bits = BitReader::with_bit_len(bytes, 31).unwrap();
            let mut packet_reader = RawPacketReader::new();
            let header = packet_reader
                .parse_bunch_header(&mut bits, packet_id)
                .unwrap();
            assert!(header.b_partial);
            assert_eq!(header.b_partial_initial, initial);
            assert_eq!(header.b_partial_final, is_final);
            assert_eq!(header.payload_bit_count, payload_bits);
            assert_eq!(header.payload_bit_offset, 31);
        }
    }

    #[test]
    fn partial_initial_then_final_completes() {
        let mut bits = Vec::new();
        write_bunch(&mut bits, &fragment(2, true, false), &[false; 8]);
        write_bunch(&mut bits, &fragment(2, false, true), &[false; 4]);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 0, |h, _| headers.push(h.clone()));

        assert_eq!(headers.len(), 2);
        assert_eq!(result.partial_error_count, 0);
        assert!(headers[0].b_partial_initial);
        assert!(!headers[0].has_partial_error);
        assert!(headers[1].b_partial_final);
        assert!(headers[1].is_partial_completed);
        assert_eq!(
            reader.partial_bunches.len(),
            0,
            "completed packet-level partial state must be retired"
        );
    }

    /// A partial that is both initial and final is a whole bunch and leaves no
    /// tracker state, so the next one on the channel is not an overlap.
    #[test]
    fn an_initial_final_partial_leaves_no_tracker_state() {
        let mut bits = Vec::new();
        for _ in 0..2 {
            write_bunch(&mut bits, &fragment(2, true, true), &[false; 8]);
        }
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 0, |h, _| headers.push(h.clone()));

        assert_eq!(headers.len(), 2);
        assert_eq!(result.partial_error_count, 0);
        assert!(!headers[1].has_partial_error, "not an overlapping initial");
        assert!(headers[0].is_partial_completed && headers[1].is_partial_completed);
        assert!(reader.partial_bunches.is_empty());
    }

    /// An initial on a channel whose assembly is still in flight overlaps it,
    /// whether or not it is also final: one error, flagged on the new initial,
    /// which replaces the tracked assembly or, as a whole bunch, leaves none.
    #[test]
    fn an_initial_over_one_in_flight_is_an_overlapping_initial() {
        for last in [false, true] {
            let mut bits = Vec::new();
            write_bunch(&mut bits, &fragment(2, true, false), &[false; 8]);
            write_bunch(&mut bits, &fragment(2, true, last), &[false; 8]);
            let packet = build_packet(&bits);

            let mut reader = RawPacketReader::new();
            let mut headers = Vec::new();
            let result = reader.read_packet(&packet, 0, |h, _| headers.push(h.clone()));

            assert_eq!(headers.len(), 2);
            assert_eq!(result.partial_error_count, 1, "last: {last}");
            assert!(!headers[0].has_partial_error);
            assert!(headers[1].has_partial_error, "last: {last}");
            assert!(!headers[1].is_partial_completed, "last: {last}");
            assert_eq!(reader.partial_bunches.len(), usize::from(!last));
        }
    }

    #[test]
    fn continuation_without_initial_reports_error() {
        let packet = build_bunch_packet(&fragment(5, false, true), &[]);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 2, |h, _| headers.push(h.clone()));
        assert_eq!(result.partial_error_count, 1);
        assert!(headers[0].has_partial_error);
    }

    #[test]
    fn reliability_mismatch_reports_error() {
        let mut bits = Vec::new();
        write_bunch(&mut bits, &fragment(2, true, false), &[]);
        let unreliable_final = BunchSpec {
            unreliable: true,
            ..fragment(2, false, true)
        };
        write_bunch(&mut bits, &unreliable_final, &[]);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 2, |h, _| headers.push(h.clone()));
        assert_eq!(result.partial_error_count, 1);
        assert!(headers[1].has_partial_error);
    }

    /// The advisory tracker's continuation rule does not overflow either: an
    /// unreliable partial's sequence is its packet id.
    #[test]
    fn a_continuation_at_the_largest_packet_id_does_not_overflow() {
        let mut bits = Vec::new();
        for (initial, last) in [(true, false), (false, true)] {
            let spec = BunchSpec {
                unreliable: true,
                ..fragment(2, initial, last)
            };
            write_bunch(&mut bits, &spec, &[false; 8]);
        }
        let mut reader = RawPacketReader::new();
        let result = reader.read_packet(&build_packet(&bits), i32::MAX, |_, _| {});
        assert_eq!((result.bunch_count, result.partial_error_count), (2, 0));
    }

    #[test]
    fn reliable_sequence_advances_per_channel_not_globally() {
        // Two channels interleaving reliable bunches each number their own 1
        // then 2; a global counter would hand out 1..4 and fail continuations.
        let mut bits = Vec::new();
        write_bunch(&mut bits, &reliable(2), &[]);
        write_bunch(&mut bits, &reliable(5), &[]);
        write_bunch(&mut bits, &reliable(2), &[]);
        write_bunch(&mut bits, &reliable(5), &[]);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut seen = Vec::new();
        reader.read_packet(&packet, 0, |h, _| seen.push((h.ch_index, h.ch_sequence)));

        assert_eq!(seen, vec![(2, 1), (5, 1), (2, 2), (5, 2)]);
    }

    #[test]
    fn reliable_sequence_state_refuses_new_channel_keys_past_its_budget() {
        let mut reader = RawPacketReader::with_max_channels(1);
        let mut bits = Vec::new();
        write_bunch(&mut bits, &reliable(2), &[]);
        write_bunch(&mut bits, &reliable(5), &[]);
        let packet = build_packet(&bits);
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 0, |header, _| headers.push(header.clone()));

        assert_eq!(reader.in_reliable_sequence.len(), 1);
        assert_eq!(result.channel_limit_count, 1);
        assert!(!headers[0].has_channel_limit_error);
        assert!(headers[1].has_channel_limit_error);
    }

    #[test]
    fn reliable_sequence_overflow_fails_closed_without_panicking() {
        let mut reader = RawPacketReader::new();
        reader.in_reliable_sequence.insert(2, i32::MAX);
        let mut headers = Vec::new();
        let result = reader.read_packet(&build_bunch_packet(&reliable(2), &[]), 0, |header, _| {
            headers.push(header.clone())
        });

        assert_eq!(result.channel_limit_count, 1);
        assert!(headers[0].has_channel_limit_error);
        assert_eq!(reader.in_reliable_sequence.get(&2), Some(&i32::MAX));
    }

    #[test]
    fn destroying_then_reusing_a_reliable_channel_restarts_its_state() {
        let mut reader = RawPacketReader::new();
        let mut seen = Vec::new();
        reader.read_packet(&build_bunch_packet(&reliable(2), &[]), 0, |header, _| {
            seen.push(header.ch_sequence)
        });

        let destroy = BunchSpec {
            b_close: true,
            ..reliable(2)
        };
        reader.read_packet(&build_bunch_packet(&destroy, &[]), 1, |header, _| {
            seen.push(header.ch_sequence)
        });

        reader.read_packet(&build_bunch_packet(&reliable(2), &[]), 2, |header, _| {
            seen.push(header.ch_sequence)
        });
        assert_eq!(seen, [1, 2, 1]);
    }

    /// A dormant close keeps the channel's state: its reliable sequence runs
    /// on, and an unreliable partial in flight across it still completes.
    #[test]
    fn a_dormant_close_keeps_the_channels_sequence_and_partial_state() {
        let dormant_close = BunchSpec {
            b_close: true,
            dormant: true,
            ..reliable(2)
        };
        let unreliable_fragment = |initial, last| BunchSpec {
            unreliable: true,
            ..fragment(2, initial, last)
        };

        let mut bits = Vec::new();
        write_bunch(&mut bits, &unreliable_fragment(true, false), &[false; 8]);
        write_bunch(&mut bits, &reliable(2), &[]);
        write_bunch(&mut bits, &dormant_close, &[]);
        let mut reader = RawPacketReader::new();
        let mut seen = Vec::new();
        let first = reader.read_packet(&build_packet(&bits), 0, |header, _| {
            seen.push(header.ch_sequence)
        });

        let mut headers = Vec::new();
        let second = reader.read_packet(
            &build_bunch_packet(&unreliable_fragment(false, true), &[false; 8]),
            1,
            |header, _| headers.push(header.clone()),
        );
        reader.read_packet(&build_bunch_packet(&reliable(2), &[]), 2, |header, _| {
            seen.push(header.ch_sequence)
        });

        assert_eq!(
            (first.partial_error_count, second.partial_error_count),
            (0, 0)
        );
        assert!(headers[0].is_partial_completed, "the partial survives");
        assert_eq!(seen, [0, 1, 2, 3], "the reliable sequence does not restart");
    }

    #[test]
    fn payload_overrun_returns_malformed() {
        let mut bits = Vec::new();
        write_bunch_header(&mut bits, &unreliable(0), 17); // claims 17 bits, carries none
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut count = 0;
        let result = reader.read_packet(&packet, 0, |_, _| count += 1);
        assert!(result.is_malformed);
        assert_eq!(count, 0);
    }
}
