//! Bunch header structure and partial bunch reassembly.

use crate::types::ChannelCloseReason;

/// Maximum simultaneously active partial-bunch assemblies.
///
/// A normal replay measured 0 and Unreal can only advance each one with a
/// packet-sized fragment. 4,096 leaves ample protocol headroom while bounding
/// a stream that sends one initial fragment on each new channel forever.
pub const MAX_ACTIVE_PARTIAL_BUNCHES: usize = 4_096;

/// Maximum raw bits retained across all partial-bunch assemblies (64 MiB).
pub const MAX_BUFFERED_PARTIAL_BITS: usize = 64 * 1024 * 1024 * 8;

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
    /// Per-channel reader state could not admit or advance this channel.
    pub has_channel_limit_error: bool,
}

/// Partial bunch accumulator: reassembles multi-fragment bunches.
///
/// Each non-final fragment must be byte-aligned (bit count % 8 == 0).
/// Fragments are concatenated into a growable buffer; on completion
/// the stitched payload is returned for content-block framing.
pub struct PartialBunchAccumulator {
    /// Per-channel fragment state.
    fragments: std::collections::HashMap<u32, AccumulatorState>,
    total_buffered_bits: usize,
    max_active: usize,
    max_buffered_bits: usize,
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
    /// Resource limit that refused this fragment, if any.
    pub resource_limit: Option<PartialResourceLimit>,
    /// Previously/currently buffered bits discarded by the refusal.
    pub discarded_bits: usize,
}

/// Which bounded partial-reassembly resource refused a fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialResourceLimit {
    /// Too many channel assemblies were simultaneously active.
    ActiveStates,
    /// The checked aggregate bit count overflowed or exceeded its memory cap.
    BufferedBits,
    /// Reserving the already-bounded destination buffer failed.
    Allocation,
}

impl PartialBunchAccumulator {
    /// Create a new empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MAX_ACTIVE_PARTIAL_BUNCHES, MAX_BUFFERED_PARTIAL_BITS)
    }

    fn with_limits(max_active: usize, max_buffered_bits: usize) -> Self {
        Self {
            fragments: std::collections::HashMap::new(),
            total_buffered_bits: 0,
            max_active,
            max_buffered_bits,
        }
    }

    /// Number of channel assemblies currently awaiting a final fragment.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.fragments.len()
    }

    /// Raw bits retained across all active assemblies.
    #[must_use]
    pub fn total_buffered_bits(&self) -> usize {
        self.total_buffered_bits
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
        if header.b_partial_initial
            && !self.fragments.contains_key(&ch_index)
            && self.fragments.len() >= self.max_active
        {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            return PartialBunchResult {
                should_process: false,
                header,
                resource_limit: Some(PartialResourceLimit::ActiveStates),
                discarded_bits: payload_bit_count,
            };
        }
        let (sequence_valid, sequence_discarded_bits) =
            self.validate_sequence(ch_index, &mut header, stats_partial_errors);
        if !sequence_valid {
            return PartialBunchResult {
                should_process: false,
                header,
                resource_limit: None,
                discarded_bits: sequence_discarded_bits.saturating_add(payload_bit_count),
            };
        }

        if payload_bit_count == 0 {
            if !header.b_partial_final {
                return PartialBunchResult {
                    should_process: false,
                    header,
                    resource_limit: None,
                    discarded_bits: sequence_discarded_bits,
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
                resource_limit: None,
                discarded_bits: sequence_discarded_bits,
            };
        }

        // Non-final fragments must be byte-aligned.
        if !header.b_partial_final && payload_bit_count % 8 != 0 {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            let discarded_bits = sequence_discarded_bits
                .saturating_add(self.discard(ch_index))
                .saturating_add(payload_bit_count);
            return PartialBunchResult {
                should_process: false,
                header,
                resource_limit: None,
                discarded_bits,
            };
        }

        // Append bits to accumulator.
        if let Some(state) = self.fragments.get(&ch_index) {
            let state_bits = state.bit_count;
            let new_state_bits = state_bits.checked_add(payload_bit_count);
            let new_total_bits = self.total_buffered_bits.checked_add(payload_bit_count);
            if new_state_bits.is_none()
                || new_total_bits.is_none_or(|bits| bits > self.max_buffered_bits)
            {
                *stats_partial_errors += 1;
                header.has_partial_error = true;
                let prior = self.discard(ch_index);
                return PartialBunchResult {
                    should_process: false,
                    header,
                    resource_limit: Some(PartialResourceLimit::BufferedBits),
                    discarded_bits: sequence_discarded_bits
                        .saturating_add(prior)
                        .saturating_add(payload_bit_count),
                };
            }
        }
        if let Some(state) = self.fragments.get_mut(&ch_index) {
            if !append_bits(
                &mut state.buffer,
                state.bit_count,
                payload_data,
                payload_bit_count,
            ) {
                *stats_partial_errors += 1;
                header.has_partial_error = true;
                let prior = self.discard(ch_index);
                return PartialBunchResult {
                    should_process: false,
                    header,
                    resource_limit: Some(PartialResourceLimit::Allocation),
                    discarded_bits: sequence_discarded_bits
                        .saturating_add(prior)
                        .saturating_add(payload_bit_count),
                };
            }
            state.bit_count = state
                .bit_count
                .checked_add(payload_bit_count)
                .expect("checked above");
            self.total_buffered_bits = self
                .total_buffered_bits
                .checked_add(payload_bit_count)
                .expect("checked above");
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
            resource_limit: None,
            discarded_bits: sequence_discarded_bits,
        }
    }

    /// Take the completed payload for a channel, if available.
    ///
    /// Returns `(buffer, bit_count, stored_header)`.
    pub fn take_completed(&mut self, ch_index: u32) -> Option<(Vec<u8>, usize, RawBunchHeader)> {
        if let Some(state) = self.fragments.get(&ch_index) {
            if state.is_complete {
                let state = self.fragments.remove(&ch_index).unwrap();
                self.total_buffered_bits = self.total_buffered_bits.saturating_sub(state.bit_count);
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
        self.total_buffered_bits = 0;
        (count, bits)
    }

    /// Retire any incomplete assembly for a channel that was destroyed.
    /// Returns the number of buffered bits that could not complete.
    pub fn retire_channel(&mut self, ch_index: u32) -> usize {
        self.discard(ch_index)
    }

    fn validate_sequence(
        &mut self,
        ch_index: u32,
        header: &mut RawBunchHeader,
        stats_partial_errors: &mut u64,
    ) -> (bool, usize) {
        if header.b_partial_initial {
            if let Some(existing) = self.fragments.get(&ch_index) {
                if !existing.is_complete {
                    *stats_partial_errors += 1;
                    header.has_partial_error = true;
                }
            }
            let discarded_bits = self.discard(ch_index);
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
            return (true, discarded_bits);
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
            return (false, self.discard(ch_index));
        }

        if prev_reliable != header.b_reliable {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            return (false, self.discard(ch_index));
        }

        let seq_ok = if prev_reliable {
            header.ch_sequence == prev_seq + 1
        } else {
            header.ch_sequence == prev_seq + 1 || header.ch_sequence == prev_seq
        };

        if !seq_ok {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            return (false, self.discard(ch_index));
        }

        if let Some(state) = self.fragments.get_mut(&ch_index) {
            state.ch_sequence = header.ch_sequence;
        }
        (true, 0)
    }

    fn discard(&mut self, ch_index: u32) -> usize {
        let bits = self
            .fragments
            .remove(&ch_index)
            .map_or(0, |state| state.bit_count);
        self.total_buffered_bits = self.total_buffered_bits.saturating_sub(bits);
        bits
    }
}

impl Default for PartialBunchAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Append `src_bit_count` bits from `src` at bit offset `dst_bit_offset` in `dst`.
fn append_bits(dst: &mut Vec<u8>, dst_bit_offset: usize, src: &[u8], src_bit_count: usize) -> bool {
    let Some(new_total) = dst_bit_offset.checked_add(src_bit_count) else {
        return false;
    };
    let new_byte_count = new_total.div_ceil(8);
    if new_byte_count > dst.len()
        && dst
            .try_reserve_exact(new_byte_count.saturating_sub(dst.len()))
            .is_err()
    {
        return false;
    }
    dst.resize(new_byte_count, 0);

    for i in 0..src_bit_count {
        let src_bit = (src[i >> 3] >> (i & 7)) & 1;
        let dest_bit = dst_bit_offset + i;
        if src_bit != 0 {
            dst[dest_bit >> 3] |= 1 << (dest_bit & 7);
        }
        // dst is already zeroed from resize, so no need to clear bits.
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_bits_byte_aligned() {
        let mut dst = vec![0xAA];
        assert!(append_bits(&mut dst, 8, &[0x55], 8));
        assert_eq!(dst, vec![0xAA, 0x55]);
    }

    #[test]
    fn append_bits_unaligned() {
        let mut dst = vec![0x0F]; // bits 0..3 = 1, bits 4..7 = 0
        assert!(append_bits(&mut dst, 4, &[0x03], 4)); // add 4 bits: 1100 -> 0x03 reversed
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
        assert_eq!(acc.total_buffered_bits(), 0);
    }

    #[test]
    fn partial_reassembly_refuses_more_active_states_than_its_budget() {
        let mut acc = PartialBunchAccumulator::with_limits(1, 64);
        let mut errs = 0;
        let mut frags = 0;
        let mut comps = 0;
        let initial = |ch_index| RawBunchHeader {
            ch_index,
            b_partial: true,
            b_partial_initial: true,
            ..Default::default()
        };
        let first = acc.add_fragment(1, initial(1), &[0xAA], 8, &mut errs, &mut frags, &mut comps);
        assert_eq!(first.resource_limit, None);
        let refused =
            acc.add_fragment(2, initial(2), &[0xBB], 8, &mut errs, &mut frags, &mut comps);
        assert_eq!(
            refused.resource_limit,
            Some(PartialResourceLimit::ActiveStates)
        );
        assert_eq!(refused.discarded_bits, 8);
        assert_eq!(acc.active_count(), 1);
        assert_eq!(acc.total_buffered_bits(), 8);
    }

    #[test]
    fn partial_reassembly_checks_the_total_before_growing_its_buffer() {
        let mut acc = PartialBunchAccumulator::with_limits(2, 12);
        let mut errs = 0;
        let mut frags = 0;
        let mut comps = 0;
        let initial = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_initial: true,
            ..Default::default()
        };
        acc.add_fragment(1, initial, &[0xAA], 8, &mut errs, &mut frags, &mut comps);
        let final_fragment = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_final: true,
            ..Default::default()
        };
        let refused = acc.add_fragment(
            1,
            final_fragment,
            &[0x1F],
            5,
            &mut errs,
            &mut frags,
            &mut comps,
        );
        assert_eq!(
            refused.resource_limit,
            Some(PartialResourceLimit::BufferedBits)
        );
        assert_eq!(refused.discarded_bits, 13, "8 buffered + 5 current");
        assert_eq!(acc.active_count(), 0, "oversized partial is discarded");
        assert_eq!(acc.total_buffered_bits(), 0);
    }

    #[test]
    fn rejected_partial_fragments_report_every_discarded_bit() {
        let mut acc = PartialBunchAccumulator::new();
        let mut errs = 0;
        let mut frags = 0;
        let mut comps = 0;
        let initial = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_initial: true,
            b_reliable: true,
            ch_sequence: 1,
            ..Default::default()
        };
        acc.add_fragment(1, initial, &[0xAA], 8, &mut errs, &mut frags, &mut comps);

        let mismatched = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_final: true,
            b_reliable: false,
            ch_sequence: 2,
            ..Default::default()
        };
        let rejected =
            acc.add_fragment(1, mismatched, &[0x1F], 5, &mut errs, &mut frags, &mut comps);
        assert_eq!(rejected.discarded_bits, 13, "8 buffered + 5 current");
        assert_eq!(acc.total_buffered_bits(), 0);

        let missing = RawBunchHeader {
            ch_index: 2,
            b_partial: true,
            b_partial_final: true,
            ..Default::default()
        };
        let rejected = acc.add_fragment(2, missing, &[0x7F], 7, &mut errs, &mut frags, &mut comps);
        assert_eq!(rejected.discarded_bits, 7, "the refused current fragment");
    }

    #[test]
    fn overlapping_initial_reports_the_replaced_payload_but_keeps_the_new_one() {
        let mut acc = PartialBunchAccumulator::new();
        let mut errs = 0;
        let mut frags = 0;
        let mut comps = 0;
        let initial = |sequence| RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_initial: true,
            b_reliable: true,
            ch_sequence: sequence,
            ..Default::default()
        };
        acc.add_fragment(1, initial(1), &[0xAA], 8, &mut errs, &mut frags, &mut comps);
        let replacement = acc.add_fragment(
            1,
            initial(2),
            &[0xBB, 0xCC],
            16,
            &mut errs,
            &mut frags,
            &mut comps,
        );
        assert_eq!(replacement.discarded_bits, 8);
        assert_eq!(acc.total_buffered_bits(), 16);
        assert_eq!(acc.active_count(), 1);
    }

    #[test]
    fn non_aligned_nonfinal_reports_buffered_and_current_bits() {
        let mut acc = PartialBunchAccumulator::new();
        let mut errs = 0;
        let mut frags = 0;
        let mut comps = 0;
        let initial = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            b_partial_initial: true,
            ..Default::default()
        };
        acc.add_fragment(1, initial, &[0xAA], 8, &mut errs, &mut frags, &mut comps);
        let continuation = RawBunchHeader {
            ch_index: 1,
            b_partial: true,
            ..Default::default()
        };
        let rejected = acc.add_fragment(
            1,
            continuation,
            &[0x07],
            3,
            &mut errs,
            &mut frags,
            &mut comps,
        );
        assert_eq!(rejected.discarded_bits, 11);
        assert_eq!(acc.total_buffered_bits(), 0);
    }
}
