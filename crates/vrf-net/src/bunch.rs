//! Bunch header structure and partial bunch reassembly.

use crate::types::ChannelCloseReason;

/// Parsed bunch header -- all fields that describe one bunch within a packet.
///
/// See `RawPacketReader::parse_bunch_header` in [`crate::packet`] for the bit
/// layout that produces these fields.
#[derive(Debug, Clone, Default)]
pub struct RawBunchHeader {
    /// Packet this bunch belongs to.
    pub packet_id: i32,
    /// Channel index.
    pub ch_index: u32,
    /// Channel is being opened.
    pub b_open: bool,
    /// Channel is being closed.
    pub b_close: bool,
    /// Close reason implies dormancy (actor still alive).
    pub b_dormant: bool,
    /// Replication is paused for this channel.
    pub b_is_replication_paused: bool,
    /// Bunch is reliable (has sequence guarantees).
    pub b_reliable: bool,
    /// Bunch is part of a multi-fragment sequence.
    pub b_partial: bool,
    /// First fragment of a partial bunch.
    pub b_partial_initial: bool,
    /// Last fragment of a partial bunch.
    pub b_partial_final: bool,
    /// Bunch carries package-map export data.
    pub b_has_package_map_exports: bool,
    /// Bunch carries must-be-mapped GUIDs.
    pub b_has_must_be_mapped_guids: bool,
    /// Sequence number (reliable or packet-derived).
    pub ch_sequence: i32,
    /// Reason the channel was closed.
    pub close_reason: ChannelCloseReason,
    /// Payload size in bits.
    pub payload_bit_count: i32,
    /// Bit offset where the payload begins within the packet.
    pub payload_bit_offset: i64,

    // --- tracking flags set by partial-bunch logic ---
    /// A partial-bunch sequence error was detected for this fragment.
    pub has_partial_error: bool,
    /// This fragment completed a partial bunch (was the valid final).
    pub is_partial_completed: bool,
}

/// Partial bunch accumulator: reassembles multi-fragment bunches.
///
/// Each non-final fragment must be byte-aligned (bit count % 8 == 0).
/// Fragments are concatenated into a growable buffer; on completion
/// the stitched payload is returned for content-block framing.
pub struct PartialBunchAccumulator {
    /// Per-channel fragment state.
    fragments: std::collections::HashMap<u32, AccumulatorState>,
}

struct AccumulatorState {
    ch_sequence: i32,
    reliable: bool,
    is_complete: bool,
    stored_header: RawBunchHeader,
    buffer: Vec<u8>,
    bit_count: usize,
}

/// Result of adding a fragment to the accumulator.
pub struct PartialBunchResult {
    /// Updated header (may have error flags set).
    pub header: RawBunchHeader,
    /// Whether the caller should process the completed payload.
    pub should_process: bool,
}

impl PartialBunchAccumulator {
    /// Create a new empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fragments: std::collections::HashMap::new(),
        }
    }

    /// Add a fragment. Returns whether the bunch is now complete.
    ///
    /// `payload_bits` / `payload_data` are the raw bits from the bunch payload.
    /// For non-final fragments, the bit count must be byte-aligned.
    #[allow(clippy::too_many_arguments)]
    pub fn add_fragment(
        &mut self,
        ch_index: u32,
        mut header: RawBunchHeader,
        payload_data: &[u8],
        payload_bit_count: usize,
        stats_partial_errors: &mut u64,
        stats_partial_fragments: &mut u64,
        stats_partial_completed: &mut u64,
    ) -> PartialBunchResult {
        if !self.validate_sequence(ch_index, &mut header, stats_partial_errors) {
            return PartialBunchResult {
                should_process: false,
                header,
            };
        }

        if payload_bit_count == 0 {
            if !header.b_partial_final {
                return PartialBunchResult {
                    should_process: false,
                    header,
                };
            }
            // Final with zero payload: complete it.
            if let Some(state) = self.fragments.get_mut(&ch_index) {
                state.is_complete = true;
                *stats_partial_completed += 1;
            }
            return PartialBunchResult {
                should_process: header.b_partial_final && !header.has_partial_error,
                header,
            };
        }

        // Non-final fragments must be byte-aligned.
        if !header.b_partial_final && payload_bit_count % 8 != 0 {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            self.discard(ch_index);
            return PartialBunchResult {
                should_process: false,
                header,
            };
        }

        // Append bits to accumulator.
        if let Some(state) = self.fragments.get_mut(&ch_index) {
            append_bits(
                &mut state.buffer,
                state.bit_count,
                payload_data,
                payload_bit_count,
            );
            state.bit_count += payload_bit_count;
            state.ch_sequence = header.ch_sequence;

            *stats_partial_fragments += 1;

            if header.b_partial_final {
                state.is_complete = true;
                *stats_partial_completed += 1;
            }
        }

        PartialBunchResult {
            should_process: header.b_partial_final && !header.has_partial_error,
            header,
        }
    }

    /// Take the completed payload for a channel, if available.
    ///
    /// Returns `(buffer, bit_count, stored_header)`.
    pub fn take_completed(&mut self, ch_index: u32) -> Option<(Vec<u8>, usize, RawBunchHeader)> {
        if let Some(state) = self.fragments.get(&ch_index) {
            if state.is_complete {
                let state = self.fragments.remove(&ch_index).unwrap();
                return Some((state.buffer, state.bit_count, state.stored_header));
            }
        }
        None
    }

    /// Drop every partial bunch still awaiting fragments and report
    /// `(count, buffered_bits)`.
    ///
    /// Called once at the end of a replay. Until the stream stops there is
    /// nothing to distinguish an abandoned reassembly from one still in
    /// progress, so this state cannot be judged any earlier -- which is exactly
    /// why it used to go out with the accumulator unremarked: `partial_errors`
    /// stayed zero because no sequence rule was broken, and `partial_fragments`
    /// had already counted the fragments as received.
    ///
    /// A bunch already marked complete is not counted: it was handed to the
    /// caller by [`Self::take_completed`] only if the caller asked, and a
    /// complete-but-untaken entry is the caller's choice, not a loss here.
    pub fn drain_unfinished(&mut self) -> (u64, u64) {
        let mut count = 0u64;
        let mut bits = 0u64;
        for (_, state) in self.fragments.drain() {
            if state.is_complete {
                continue;
            }
            count += 1;
            bits += state.bit_count as u64;
        }
        (count, bits)
    }

    fn validate_sequence(
        &mut self,
        ch_index: u32,
        header: &mut RawBunchHeader,
        stats_partial_errors: &mut u64,
    ) -> bool {
        if header.b_partial_initial {
            if let Some(existing) = self.fragments.get(&ch_index) {
                if !existing.is_complete {
                    *stats_partial_errors += 1;
                    header.has_partial_error = true;
                }
            }
            self.fragments.insert(
                ch_index,
                AccumulatorState {
                    ch_sequence: header.ch_sequence,
                    reliable: header.b_reliable,
                    is_complete: false,
                    stored_header: header.clone(),
                    buffer: Vec::new(),
                    bit_count: 0,
                },
            );
            return true;
        }

        // Continuation
        let (has_state, is_complete, prev_seq, prev_reliable) = match self.fragments.get(&ch_index)
        {
            Some(s) => (true, s.is_complete, s.ch_sequence, s.reliable),
            None => (false, false, 0, false),
        };

        if !has_state || is_complete {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            return false;
        }

        if prev_reliable != header.b_reliable {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            self.discard(ch_index);
            return false;
        }

        let seq_ok = if prev_reliable {
            header.ch_sequence == prev_seq + 1
        } else {
            header.ch_sequence == prev_seq + 1 || header.ch_sequence == prev_seq
        };

        if !seq_ok {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            self.discard(ch_index);
            return false;
        }

        if let Some(state) = self.fragments.get_mut(&ch_index) {
            state.ch_sequence = header.ch_sequence;
        }
        true
    }

    fn discard(&mut self, ch_index: u32) {
        self.fragments.remove(&ch_index);
    }
}

impl Default for PartialBunchAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Append `src_bit_count` bits from `src` at bit offset `dst_bit_offset` in `dst`.
fn append_bits(dst: &mut Vec<u8>, dst_bit_offset: usize, src: &[u8], src_bit_count: usize) {
    let new_total = dst_bit_offset + src_bit_count;
    let new_byte_count = new_total.div_ceil(8);
    dst.resize(new_byte_count, 0);

    for i in 0..src_bit_count {
        let src_bit = (src[i >> 3] >> (i & 7)) & 1;
        let dest_bit = dst_bit_offset + i;
        if src_bit != 0 {
            dst[dest_bit >> 3] |= 1 << (dest_bit & 7);
        }
        // dst is already zeroed from resize, so no need to clear bits.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_bits_byte_aligned() {
        let mut dst = vec![0xAA];
        append_bits(&mut dst, 8, &[0x55], 8);
        assert_eq!(dst, vec![0xAA, 0x55]);
    }

    #[test]
    fn append_bits_unaligned() {
        let mut dst = vec![0x0F]; // bits 0..3 = 1, bits 4..7 = 0
        append_bits(&mut dst, 4, &[0x03], 4); // add 4 bits: 1100 -> 0x03 reversed
        // dst should be: low nibble 0x0F, high nibble 0x30 = 0x3F
        assert_eq!(dst[0], 0x3F);
    }

    #[test]
    fn accumulator_initial_plus_final() {
        let mut acc = PartialBunchAccumulator::new();
        let mut errs = 0u64;
        let mut frags = 0u64;
        let mut comps = 0u64;

        let h1 = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_initial: true,
            b_reliable: true,
            ch_sequence: 1,
            ..Default::default()
        };
        let r1 = acc.add_fragment(1, h1, &[0xAB], 8, &mut errs, &mut frags, &mut comps);
        assert!(!r1.should_process);

        let h2 = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_final: true,
            b_reliable: true,
            ch_sequence: 2,
            ..Default::default()
        };
        let r2 = acc.add_fragment(1, h2, &[0xCD], 8, &mut errs, &mut frags, &mut comps);
        assert!(r2.should_process);
        assert_eq!(errs, 0);

        let (buf, bits, _hdr) = acc.take_completed(1).unwrap();
        assert_eq!(bits, 16);
        assert_eq!(buf[0], 0xAB);
        assert_eq!(buf[1], 0xCD);
    }
}
