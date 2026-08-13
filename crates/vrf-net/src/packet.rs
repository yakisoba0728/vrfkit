//! Raw packet reading: sentinel-based bit sizing and bunch header extraction.
//!
//! # Packet bit size
//!
//! Unreal pads packets to byte boundaries but marks the true end with a
//! sentinel: one `1` bit followed by zero-padding to the byte boundary. The
//! reader finds this sentinel by scanning the last byte from MSB downward:
//!
//! ```text
//! bitSize = len*8 - 1
//! while (lastByte & 0x80) == 0:
//!     lastByte <<= 1
//!     bitSize -= 1
//! ```
//!
//! If the last byte is zero the packet is malformed (no sentinel exists).

use vrf_bitio::BitReader;

use crate::bunch::RawBunchHeader;
use crate::error::{PartialSequenceKind, Result};
use crate::types::{ChannelCloseReason, MAX_ACTIVE_CHANNELS, MAX_PACKET_SIZE_BITS};

use std::collections::HashMap;

/// Result of reading one packet.
#[derive(Debug, Clone)]
pub struct PacketReadResult {
    /// Number of bunches successfully parsed from this packet.
    pub bunch_count: u32,
    /// Whether the packet was malformed (last byte zero or payload overrun).
    pub is_malformed: bool,
    /// Partial-bunch sequence errors encountered.
    pub partial_error_count: u32,
    /// Bunches refused because per-channel state could not be admitted or advanced.
    pub channel_limit_count: u32,
}

/// Per-channel state for partial bunch tracking within the packet reader.
#[derive(Debug, Clone)]
struct PartialState {
    ch_sequence: i32,
    reliable: bool,
    is_complete: bool,
}

/// Stateful packet reader that tracks partial bunches and reliable sequences.
///
/// One instance lives for the duration of the replay stream. It accumulates
/// per-channel partial-bunch state and a per-channel reliable sequence counter
/// -- Unreal's `ReliableSequence` is per channel, so a single global counter
/// diverges when two channels interleave reliable bunches.
pub struct RawPacketReader {
    partial_bunches: HashMap<u32, PartialState>,
    in_reliable_sequence: HashMap<u32, i32>,
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
            partial_bunches: HashMap::new(),
            in_reliable_sequence: HashMap::new(),
            max_channels,
        }
    }

    /// Parse all bunches from a single packet's byte slice.
    ///
    /// For each successfully parsed bunch, `callback` is invoked with the
    /// parsed header and a sub-reader over the bunch's payload bits.
    /// The callback receives ownership of the payload reader so it can
    /// forward it to content-block framing.
    pub fn read_packet<F>(
        &mut self,
        packet_data: &[u8],
        packet_id: i32,
        mut callback: F,
    ) -> PacketReadResult
    where
        F: FnMut(&mut RawBunchHeader, BitReader<'_>),
    {
        if packet_data.is_empty() {
            return PacketReadResult {
                bunch_count: 0,
                is_malformed: false,
                partial_error_count: 0,
                channel_limit_count: 0,
            };
        }

        let last_byte = packet_data[packet_data.len() - 1];
        if last_byte == 0 {
            return PacketReadResult {
                bunch_count: 0,
                is_malformed: true,
                partial_error_count: 0,
                channel_limit_count: 0,
            };
        }

        let bit_size = compute_bit_size(packet_data, last_byte);
        let Ok(mut reader) = BitReader::with_bit_len(packet_data, bit_size as u64) else {
            return PacketReadResult {
                bunch_count: 0,
                is_malformed: true,
                partial_error_count: 0,
                channel_limit_count: 0,
            };
        };

        let mut bunch_count = 0u32;
        let mut partial_error_count = 0u32;
        let mut channel_limit_count = 0u32;

        while !reader.at_end() {
            let header = self.parse_bunch_header(&mut reader, packet_id);
            let header = match header {
                Ok(h) => h,
                Err(_) => {
                    return PacketReadResult {
                        bunch_count,
                        is_malformed: true,
                        partial_error_count,
                        channel_limit_count,
                    };
                }
            };

            let mut header = header;
            if header.has_channel_limit_error {
                channel_limit_count += 1;
            }

            if header.payload_bit_count as u64 > reader.bits_remaining() {
                return PacketReadResult {
                    bunch_count,
                    is_malformed: true,
                    partial_error_count,
                    channel_limit_count,
                };
            }

            if !header.has_channel_limit_error {
                self.track_partial_bunch(&mut header, &mut partial_error_count);
            }

            let payload = reader
                .sub_reader(header.payload_bit_count as u64)
                .expect("bounds already checked");

            callback(&mut header, payload);
            bunch_count += 1;
            if header.b_close && !header.b_dormant {
                self.retire_channel(header.ch_index);
            }
        }

        PacketReadResult {
            bunch_count,
            is_malformed: false,
            partial_error_count,
            channel_limit_count,
        }
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
    /// | [if bPartial]                                                |
    /// |   bPartialInitial  : 1 bit                                   |
    /// |   bPartialFinal    : 1 bit                                   |
    /// | <VALORANT>         : 1 bit (read and discarded)              |
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
            // No wrap rule is established for this replay format. Refuse the
            // bunch at the representational boundary instead of panicking in
            // debug builds or silently inventing a wrapped sequence in release.
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

        if header.b_partial {
            header.b_partial_initial = reader.read_bit()?;
            header.b_partial_final = reader.read_bit()?;
        }

        // VALORANT-specific bit: always present, always discarded.
        let _valorant_bit = reader.read_bit()?;

        // Channel name (FName): present when reliable or opening.
        if header.b_reliable || header.b_open {
            // FName: 1 bit isHardcoded + IntPacked index.
            // We consume it but don't need the value for replication.
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

    /// Track partial bunch state across fragments.
    fn track_partial_bunch(&mut self, header: &mut RawBunchHeader, partial_error_count: &mut u32) {
        if !header.b_partial {
            return;
        }

        if header.b_partial_initial {
            if !self.partial_bunches.contains_key(&header.ch_index)
                && self.partial_bunches.len() >= self.max_channels
            {
                *partial_error_count += 1;
                header.has_partial_error = true;
                return;
            }
            // Check for overlapping initial
            if let Some(existing) = self.partial_bunches.get(&header.ch_index) {
                if !existing.is_complete {
                    *partial_error_count += 1;
                    header.has_partial_error = true;
                }
            }

            self.partial_bunches.insert(
                header.ch_index,
                PartialState {
                    ch_sequence: header.ch_sequence,
                    reliable: header.b_reliable,
                    is_complete: false,
                },
            );
            return;
        }

        // Continuation or final
        let error = self.validate_continuation(header);
        if let Some(kind) = error {
            *partial_error_count += 1;
            header.has_partial_error = true;
            if kind == PartialSequenceKind::MismatchedContinuation {
                self.partial_bunches.remove(&header.ch_index);
            }
            let _ = kind; // consumed for the count
            return;
        }

        if let Some(state) = self.partial_bunches.get_mut(&header.ch_index) {
            state.ch_sequence = header.ch_sequence;

            if header.b_partial_final {
                state.is_complete = true;
                header.is_partial_completed = true;
            }
        }
        if header.b_partial_final {
            self.partial_bunches.remove(&header.ch_index);
        }
    }

    fn validate_continuation(&self, header: &RawBunchHeader) -> Option<PartialSequenceKind> {
        let state = match self.partial_bunches.get(&header.ch_index) {
            None => return Some(PartialSequenceKind::MissingInitial),
            Some(s) => s,
        };

        if state.is_complete {
            return Some(PartialSequenceKind::MissingInitial);
        }

        if state.reliable != header.b_reliable {
            return Some(PartialSequenceKind::MismatchedContinuation);
        }

        let seq_ok = if state.reliable {
            header.ch_sequence == state.ch_sequence + 1
        } else {
            header.ch_sequence == state.ch_sequence + 1 || header.ch_sequence == state.ch_sequence
        };

        if !seq_ok {
            return Some(PartialSequenceKind::MismatchedContinuation);
        }

        None
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

/// Compute the true bit size of a packet by finding the sentinel bit.
///
/// The sentinel is the highest `1` bit in the last byte; all bits above it
/// (toward MSB) are padding. The sentinel itself is not data.
///
/// The reference walk is a shift loop:
///
/// ```text
/// bitSize = len*8 - 1
/// while (lastByte & 0x80) == 0: lastByte <<= 1; bitSize -= 1
/// ```
///
/// which runs once per packet and iterates once per padding bit. It counts
/// exactly the leading zeros of the last byte, so `leading_zeros` gives the
/// same answer without the loop. `last_byte` is non-zero here -- the caller
/// rejects a zero last byte as a packet with no sentinel -- so the count is at
/// most 7 and the result never underflows.
///
/// # Panics
///
/// Panics in debug builds if `last_byte` is zero, which would mean the caller
/// skipped the malformed-packet check.
fn compute_bit_size(packet: &[u8], last_byte: u8) -> i32 {
    debug_assert!(last_byte != 0, "caller must reject a zero last byte");
    (packet.len() as i32) * 8 - 1 - last_byte.leading_zeros() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bunch::RawBunchHeader;

    /// Helper: build a packet from a list of bits, appending the sentinel.
    fn build_packet(bits: &[bool]) -> Vec<u8> {
        let total_data_bits = bits.len();
        let byte_count = (total_data_bits + 1).div_ceil(8);
        let mut packet = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                packet[i >> 3] |= 1 << (i & 7);
            }
        }
        // Set sentinel bit
        packet[total_data_bits >> 3] |= 1 << (total_data_bits & 7);
        packet
    }

    fn write_bit(bits: &mut Vec<bool>, v: bool) {
        bits.push(v);
    }

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

    fn write_payload_size(bits: &mut Vec<bool>, value: u32) {
        write_serialized_int(bits, value, MAX_PACKET_SIZE_BITS);
    }

    fn write_fname(bits: &mut Vec<bool>, index: u32) {
        write_bit(bits, true); // isHardcoded
        write_int_packed(bits, index);
    }

    /// Build a minimal bunch header with no control, no partial, no reliable.
    fn write_minimal_header(bits: &mut Vec<bool>, ch_index: u32, payload_bits: u32) {
        write_bit(bits, false); // bControl = false
        write_bit(bits, false); // bIsReplicationPaused = false
        write_bit(bits, false); // bReliable = false
        write_int_packed(bits, ch_index);
        write_bit(bits, false); // bHasPackageMapExports
        write_bit(bits, false); // bHasMustBeMappedGUIDs
        write_bit(bits, false); // bPartial
        write_bit(bits, false); // VALORANT bit
        write_payload_size(bits, payload_bits);
    }

    /// Build a reliable, non-partial bunch header on `ch_index`.
    fn write_reliable_header(bits: &mut Vec<bool>, ch_index: u32, payload_bits: u32) {
        write_bit(bits, false); // bControl = false
        write_bit(bits, false); // bIsReplicationPaused = false
        write_bit(bits, true); // bReliable = true
        write_int_packed(bits, ch_index);
        write_bit(bits, false); // bHasPackageMapExports
        write_bit(bits, false); // bHasMustBeMappedGUIDs
        write_bit(bits, false); // bPartial
        write_bit(bits, false); // VALORANT bit
        write_fname(bits, 1); // channel name: read whenever reliable
        write_payload_size(bits, payload_bits);
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
        let mut bits = Vec::new();
        write_minimal_header(&mut bits, 7, 0);
        let packet = build_packet(&bits);

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
        let mut bits = Vec::new();
        write_bit(&mut bits, true); // bControl
        write_bit(&mut bits, false); // bOpen
        write_bit(&mut bits, true); // bClose
        write_serialized_int(&mut bits, 1, ChannelCloseReason::MAX); // Dormancy
        write_bit(&mut bits, false); // bIsReplicationPaused
        write_bit(&mut bits, false); // bReliable
        write_int_packed(&mut bits, 3); // ChIndex
        write_bit(&mut bits, false); // bHasPackageMapExports
        write_bit(&mut bits, false); // bHasMustBeMappedGUIDs
        write_bit(&mut bits, false); // bPartial
        write_bit(&mut bits, false); // VALORANT bit
        write_payload_size(&mut bits, 0);
        let packet = build_packet(&bits);

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
        let mut bits = Vec::new();
        write_minimal_header(&mut bits, 0, 0);
        write_minimal_header(&mut bits, 1, 0);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut indices = Vec::new();
        reader.read_packet(&packet, 3, |h, _| indices.push(h.ch_index));
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn partial_initial_then_final_completes() {
        let mut bits = Vec::new();
        // First: partial initial, reliable
        write_bit(&mut bits, false); // bControl
        write_bit(&mut bits, false); // bIsReplicationPaused
        write_bit(&mut bits, true); // bReliable
        write_int_packed(&mut bits, 2);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bPartial
        write_bit(&mut bits, true); // bPartialInitial
        write_bit(&mut bits, false); // bPartialFinal
        write_bit(&mut bits, false); // VALORANT
        write_fname(&mut bits, 1);
        write_payload_size(&mut bits, 8);
        for _ in 0..8 {
            write_bit(&mut bits, false);
        }
        // Second: partial final, reliable
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bReliable
        write_int_packed(&mut bits, 2);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bPartial
        write_bit(&mut bits, false); // bPartialInitial
        write_bit(&mut bits, true); // bPartialFinal
        write_bit(&mut bits, false); // VALORANT
        write_fname(&mut bits, 1);
        write_payload_size(&mut bits, 4);
        for _ in 0..4 {
            write_bit(&mut bits, false);
        }
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

    #[test]
    fn continuation_without_initial_reports_error() {
        let mut bits = Vec::new();
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bReliable
        write_int_packed(&mut bits, 5);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bPartial
        write_bit(&mut bits, false); // not initial
        write_bit(&mut bits, true); // final
        write_bit(&mut bits, false); // VALORANT
        write_fname(&mut bits, 1);
        write_payload_size(&mut bits, 0);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 2, |h, _| headers.push(h.clone()));
        assert_eq!(result.partial_error_count, 1);
        assert!(headers[0].has_partial_error);
    }

    #[test]
    fn reliability_mismatch_reports_error() {
        let mut bits = Vec::new();
        // Initial: reliable
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true); // bReliable
        write_int_packed(&mut bits, 2);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true);
        write_bit(&mut bits, true); // initial
        write_bit(&mut bits, false);
        write_bit(&mut bits, false); // VALORANT
        write_fname(&mut bits, 1);
        write_payload_size(&mut bits, 0);
        // Continuation: NOT reliable
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false); // NOT reliable
        write_int_packed(&mut bits, 2);
        write_bit(&mut bits, false);
        write_bit(&mut bits, false);
        write_bit(&mut bits, true);
        write_bit(&mut bits, false); // not initial
        write_bit(&mut bits, true); // final
        write_bit(&mut bits, false); // VALORANT
        write_payload_size(&mut bits, 0);
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut headers = Vec::new();
        let result = reader.read_packet(&packet, 2, |h, _| headers.push(h.clone()));
        assert_eq!(result.partial_error_count, 1);
        assert!(headers[1].has_partial_error);
    }

    #[test]
    fn reliable_sequence_advances_per_channel_not_globally() {
        // Two channels interleaving reliable bunches. Unreal's ReliableSequence
        // is per channel, so each channel numbers its own bunches 1 then 2. A
        // single global counter hands out 1, 2, 3, 4 instead, and a later
        // continuation check would reject the valid bunch as a mismatch.
        let mut bits = Vec::new();
        write_reliable_header(&mut bits, 2, 0);
        write_reliable_header(&mut bits, 5, 0);
        write_reliable_header(&mut bits, 2, 0);
        write_reliable_header(&mut bits, 5, 0);
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
        write_reliable_header(&mut bits, 2, 0);
        write_reliable_header(&mut bits, 5, 0);
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
        let mut bits = Vec::new();
        write_reliable_header(&mut bits, 2, 0);
        let mut headers = Vec::new();
        let result = reader.read_packet(&build_packet(&bits), 0, |header, _| {
            headers.push(header.clone())
        });

        assert_eq!(result.channel_limit_count, 1);
        assert!(headers[0].has_channel_limit_error);
        assert_eq!(reader.in_reliable_sequence.get(&2), Some(&i32::MAX));
    }

    #[test]
    fn destroying_then_reusing_a_reliable_channel_restarts_its_state() {
        let mut reader = RawPacketReader::new();
        let mut open = Vec::new();
        write_reliable_header(&mut open, 2, 0);
        let mut seen = Vec::new();
        reader.read_packet(&build_packet(&open), 0, |header, _| {
            seen.push(header.ch_sequence)
        });

        let mut close = Vec::new();
        write_bit(&mut close, true); // control
        write_bit(&mut close, false); // open
        write_bit(&mut close, true); // close
        write_serialized_int(&mut close, 0, ChannelCloseReason::MAX);
        write_bit(&mut close, false); // paused
        write_bit(&mut close, true); // reliable
        write_int_packed(&mut close, 2);
        write_bit(&mut close, false); // exports
        write_bit(&mut close, false); // mapped
        write_bit(&mut close, false); // partial
        write_bit(&mut close, false); // valorant
        write_fname(&mut close, 1);
        write_payload_size(&mut close, 0);
        reader.read_packet(&build_packet(&close), 1, |header, _| {
            seen.push(header.ch_sequence)
        });

        let mut reused = Vec::new();
        write_reliable_header(&mut reused, 2, 0);
        reader.read_packet(&build_packet(&reused), 2, |header, _| {
            seen.push(header.ch_sequence)
        });
        assert_eq!(seen, [1, 2, 1]);
    }

    #[test]
    fn payload_overrun_returns_malformed() {
        let mut bits = Vec::new();
        write_minimal_header(&mut bits, 0, 17); // claims 17 bits payload
        // but we don't write any payload bits
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut count = 0;
        let result = reader.read_packet(&packet, 0, |_, _| count += 1);
        assert!(result.is_malformed);
        assert_eq!(count, 0);
    }

    #[test]
    fn payload_bits_consumed_stream_stays_aligned() {
        let mut bits = Vec::new();
        write_minimal_header(&mut bits, 0, 17);
        for _ in 0..17 {
            write_bit(&mut bits, false);
        }
        let packet = build_packet(&bits);

        let mut reader = RawPacketReader::new();
        let mut count = 0;
        reader.read_packet(&packet, 0, |_, _| count += 1);
        assert_eq!(count, 1);
    }
}
