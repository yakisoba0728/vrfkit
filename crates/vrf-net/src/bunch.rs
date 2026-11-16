//! Bunch header structure and partial bunch reassembly.

use crate::error::PartialSequenceKind;
use crate::types::ChannelCloseReason;

/// Maximum simultaneously active partial-bunch assemblies.
///
/// Bounds a stream that starts an assembly on each new channel forever.
pub const MAX_ACTIVE_PARTIAL_BUNCHES: usize = 4_096;

/// Maximum raw bits retained across all partial-bunch assemblies (64 MiB).
pub const MAX_BUFFERED_PARTIAL_BITS: usize = 64 * 1024 * 1024 * 8;

/// Parsed bunch header: every field that describes one bunch within a packet,
/// in the bit layout `RawPacketReader::parse_bunch_header` ([`crate::packet`])
/// documents.
#[derive(Debug, Clone, Default)]
pub struct RawBunchHeader {
    pub packet_id: i32,
    pub ch_index: u32,
    pub b_open: bool,
    pub b_close: bool,
    /// Close reason implies dormancy (actor still alive).
    pub b_dormant: bool,
    pub b_is_replication_paused: bool,
    pub b_reliable: bool,
    pub b_partial: bool,
    pub b_partial_initial: bool,
    pub b_partial_final: bool,
    pub b_has_package_map_exports: bool,
    pub b_has_must_be_mapped_guids: bool,
    /// Sequence number: the channel's reliable sequence, or for an unreliable
    /// partial the packet id.
    pub ch_sequence: i32,
    pub close_reason: ChannelCloseReason,
    pub payload_bit_count: i32,
    /// Bit offset where the payload begins within the packet.
    pub payload_bit_offset: i64,

    // --- tracking flags set by partial-bunch logic ---
    /// A partial-bunch sequence error was detected for this fragment.
    pub has_partial_error: bool,
    /// Exact sequence/alignment failure, when one was identified.
    pub partial_error_kind: Option<PartialSequenceKind>,
    /// This fragment completed a partial bunch (was the valid final).
    pub is_partial_completed: bool,
    /// Per-channel reader state could not admit or advance this channel.
    pub has_channel_limit_error: bool,
}

/// Reassembles multi-fragment bunches: each channel's fragments are
/// concatenated and the stitched payload is handed back for framing. Every
/// non-final fragment must be byte-aligned.
pub struct PartialBunchAccumulator {
    fragments: vrf_schema::FxHashMap<u32, AccumulatorState>,
    total_buffered_bits: usize,
    max_active: usize,
    max_buffered_bits: usize,
}

struct AccumulatorState {
    ch_sequence: i32,
    reliable: bool,
    is_complete: bool,
    /// The initial fragment's header and the bytes appended so far.
    partial: PreservedPartial,
}

/// Raw partial payload removed from reassembly without being decoded.
pub struct PreservedPartial {
    pub header: RawBunchHeader,
    pub buffer: Vec<u8>,
    pub bit_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialDiscardCause {
    Sequence(PartialSequenceKind),
    Resource(PartialResourceLimit),
}

/// Result of adding a fragment to the accumulator.
#[derive(Default)]
pub struct PartialBunchResult {
    /// Updated header (may have error flags set).
    pub header: RawBunchHeader,
    /// Whether the caller should process the completed payload.
    pub should_process: bool,
    /// Resource limit that refused this fragment, if any.
    pub resource_limit: Option<PartialResourceLimit>,
    /// Previously/currently buffered bits discarded by the refusal.
    pub discarded_bits: usize,
    /// Sequence, alignment or payload-length cause when this fragment was rejected.
    pub error_kind: Option<PartialSequenceKind>,
    /// The fragment replaced an incomplete initial, independently of whether
    /// a second error later rejected the replacement.
    pub overlapping_initial: bool,
    /// Earlier in-flight payload displaced while handling this fragment.
    pub displaced: Vec<(PreservedPartial, PartialDiscardCause)>,
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
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MAX_ACTIVE_PARTIAL_BUNCHES, MAX_BUFFERED_PARTIAL_BITS)
    }

    fn with_limits(max_active: usize, max_buffered_bits: usize) -> Self {
        Self {
            fragments: vrf_schema::FxHashMap::default(),
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

    /// Add one fragment and report the outcome: whether a completed payload is
    /// ready (`should_process`) and every bit and payload this call discarded.
    /// A byte slice shorter than its declared bit count is refused before
    /// sequence admission. The prior assembly is returned in `displaced`;
    /// `discarded_bits` counts only supplied bits, never the missing bytes.
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
        if payload_bit_count.div_ceil(8) > payload_data.len() {
            let kind = PartialSequenceKind::PayloadOverrun;
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            header.partial_error_kind = Some(kind);
            let displaced = self.take_as(ch_index, PartialDiscardCause::Sequence(kind));
            let discarded_bits = displaced.iter().fold(
                payload_data.len().saturating_mul(8),
                |bits, (partial, _)| bits.saturating_add(partial.bit_count),
            );
            return PartialBunchResult {
                header,
                error_kind: Some(kind),
                discarded_bits,
                displaced,
                ..Default::default()
            };
        }
        if header.b_partial_initial
            && !self.fragments.contains_key(&ch_index)
            && self.fragments.len() >= self.max_active
        {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            return PartialBunchResult {
                header,
                resource_limit: Some(PartialResourceLimit::ActiveStates),
                discarded_bits: payload_bit_count,
                ..Default::default()
            };
        }
        let (sequence_valid, mut displaced) =
            self.validate_sequence(ch_index, &mut header, stats_partial_errors);
        let sequence_discarded_bits = displaced.iter().map(|(p, _)| p.bit_count).sum::<usize>();
        let overlapping_initial =
            header.partial_error_kind == Some(PartialSequenceKind::OverlappingInitial);
        if !sequence_valid {
            return PartialBunchResult {
                error_kind: header.partial_error_kind,
                header,
                discarded_bits: sequence_discarded_bits.saturating_add(payload_bit_count),
                overlapping_initial,
                displaced,
                ..Default::default()
            };
        }

        if payload_bit_count == 0 {
            // Still a fragment that arrived, counted as the non-empty path does.
            *stats_partial_fragments += 1;
            // An errored final is not a completion (see the non-empty path).
            // The assembly it just started holds no bits: retiring loses none.
            if header.b_partial_final && header.has_partial_error {
                self.retire_channel(ch_index);
            } else if header.b_partial_final {
                if let Some(state) = self.fragments.get_mut(&ch_index) {
                    state.is_complete = true;
                    header.is_partial_completed = true;
                    *stats_partial_completed += 1;
                }
            }
            return PartialBunchResult {
                should_process: header.b_partial_final && !header.has_partial_error,
                header,
                discarded_bits: sequence_discarded_bits,
                overlapping_initial,
                displaced,
                ..Default::default()
            };
        }

        // Refuse the fragment: retire the channel's assembly and report it
        // after whatever `validate_sequence` displaced -- unless this is an
        // initial, whose assembly is the empty one just started for it: no
        // bits, and the caller reports the fragment itself.
        let mut refuse = |acc: &mut Self,
                          mut header: RawBunchHeader,
                          mut displaced: Vec<(PreservedPartial, PartialDiscardCause)>,
                          cause: PartialDiscardCause|
         -> PartialBunchResult {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            let prior = acc
                .retire_channel(ch_index)
                .filter(|_| !header.b_partial_initial);
            let discarded_bits = sequence_discarded_bits
                .saturating_add(prior.as_ref().map_or(0, |p| p.bit_count))
                .saturating_add(payload_bit_count);
            displaced.extend(prior.map(|p| (p, cause)));
            let (error_kind, resource_limit) = match cause {
                PartialDiscardCause::Sequence(kind) => (Some(kind), None),
                PartialDiscardCause::Resource(limit) => (None, Some(limit)),
            };
            PartialBunchResult {
                header,
                resource_limit,
                discarded_bits,
                error_kind,
                overlapping_initial,
                displaced,
                ..Default::default()
            }
        };

        // Non-final fragments must be byte-aligned.
        if !header.b_partial_final && payload_bit_count % 8 != 0 {
            let cause = PartialDiscardCause::Sequence(PartialSequenceKind::NonByteAlignedFragment);
            return refuse(self, header, displaced, cause);
        }

        if let Some(state) = self.fragments.get(&ch_index) {
            let new_state_bits = state.partial.bit_count.checked_add(payload_bit_count);
            let new_total_bits = self.total_buffered_bits.checked_add(payload_bit_count);
            if new_state_bits.is_none()
                || new_total_bits.is_none_or(|bits| bits > self.max_buffered_bits)
            {
                let cause = PartialDiscardCause::Resource(PartialResourceLimit::BufferedBits);
                return refuse(self, header, displaced, cause);
            }
        }
        if let Some(state) = self.fragments.get_mut(&ch_index) {
            let partial = &mut state.partial;
            debug_assert!(partial.bit_count % 8 == 0, "appending after a final");
            if !append_bytes(&mut partial.buffer, payload_data, payload_bit_count) {
                let cause = PartialDiscardCause::Resource(PartialResourceLimit::Allocation);
                return refuse(self, header, displaced, cause);
            }
            partial.bit_count = partial
                .bit_count
                .checked_add(payload_bit_count)
                .expect("checked above");
            self.total_buffered_bits = self
                .total_buffered_bits
                .checked_add(payload_bit_count)
                .expect("checked above");
            *stats_partial_fragments += 1;

            // An overlapping initial that is also final arrives here errored:
            // marked complete it would be neither taken nor drained, its bits
            // in no counter while `partial_completed` claimed success.
            if header.b_partial_final && !header.has_partial_error {
                state.is_complete = true;
                header.is_partial_completed = true;
                *stats_partial_completed += 1;
            }
        }

        // The error-final case above: discard rather than leave it to leak,
        // and fold its bits into what this call reports lost.
        let error_final_bits = if header.b_partial_final && header.has_partial_error {
            let kind = header
                .partial_error_kind
                .unwrap_or(PartialSequenceKind::OverlappingInitial);
            let removed = self.take_as(ch_index, PartialDiscardCause::Sequence(kind));
            let bits = removed.iter().map(|(p, _)| p.bit_count).sum();
            displaced.extend(removed);
            bits
        } else {
            0
        };

        PartialBunchResult {
            should_process: header.b_partial_final && !header.has_partial_error,
            error_kind: header.partial_error_kind,
            header,
            discarded_bits: sequence_discarded_bits.saturating_add(error_final_bits),
            overlapping_initial,
            displaced,
            ..Default::default()
        }
    }

    /// The channel's completed payload: `(buffer, bit_count, stored_header)`.
    pub fn take_completed(&mut self, ch_index: u32) -> Option<(Vec<u8>, usize, RawBunchHeader)> {
        if !self.fragments.get(&ch_index)?.is_complete {
            return None;
        }
        self.retire_channel(ch_index)
            .map(|taken| (taken.buffer, taken.bit_count, taken.header))
    }

    /// Drop and return every assembly still awaiting fragments, at end of
    /// stream (until then an abandoned one looks in progress). A complete but
    /// untaken entry is not a loss and is skipped.
    pub fn drain_unfinished(&mut self) -> Vec<PreservedPartial> {
        self.total_buffered_bits = 0;
        self.fragments
            .drain()
            .filter(|(_, state)| !state.is_complete)
            .map(|(_, state)| state.partial)
            .collect()
    }

    /// Remove the channel's assembly, finished or not, and return it; its bits
    /// leave the buffered total. The pipeline calls this when a close destroys
    /// the channel.
    pub fn retire_channel(&mut self, ch_index: u32) -> Option<PreservedPartial> {
        let state = self.fragments.remove(&ch_index)?;
        self.total_buffered_bits = self
            .total_buffered_bits
            .saturating_sub(state.partial.bit_count);
        Some(state.partial)
    }

    fn validate_sequence(
        &mut self,
        ch_index: u32,
        header: &mut RawBunchHeader,
        stats_partial_errors: &mut u64,
    ) -> (bool, Vec<(PreservedPartial, PartialDiscardCause)>) {
        if header.b_partial_initial {
            if let Some(existing) = self.fragments.get(&ch_index) {
                if !existing.is_complete {
                    *stats_partial_errors += 1;
                    header.has_partial_error = true;
                    header.partial_error_kind = Some(PartialSequenceKind::OverlappingInitial);
                }
            }
            let displaced = self.take_as(
                ch_index,
                PartialDiscardCause::Sequence(PartialSequenceKind::OverlappingInitial),
            );
            self.fragments.insert(
                ch_index,
                AccumulatorState {
                    ch_sequence: header.ch_sequence,
                    reliable: header.b_reliable,
                    is_complete: false,
                    partial: PreservedPartial {
                        header: header.clone(),
                        buffer: Vec::new(),
                        bit_count: 0,
                    },
                },
            );
            return (true, displaced);
        }

        // Continuation: it needs an assembly in flight, of the same
        // reliability, at the next sequence number (an unreliable one may
        // also repeat it). The sequence is compared only once the rest holds.
        let error = match self.fragments.get(&ch_index) {
            None => Some(PartialSequenceKind::MissingInitial),
            Some(state) if state.is_complete => Some(PartialSequenceKind::MissingInitial),
            Some(state) if state.reliable != header.b_reliable => {
                Some(PartialSequenceKind::MismatchedContinuation)
            }
            Some(state) => (!continues(state.reliable, state.ch_sequence, header.ch_sequence))
                .then_some(PartialSequenceKind::MismatchedContinuation),
        };
        if let Some(kind) = error {
            *stats_partial_errors += 1;
            header.has_partial_error = true;
            header.partial_error_kind = Some(kind);
            return (
                false,
                self.take_as(ch_index, PartialDiscardCause::Sequence(kind)),
            );
        }

        if let Some(state) = self.fragments.get_mut(&ch_index) {
            state.ch_sequence = header.ch_sequence;
        }
        (true, Vec::new())
    }

    /// [`Self::retire_channel`], paired with the cause that displaced it.
    fn take_as(
        &mut self,
        ch_index: u32,
        cause: PartialDiscardCause,
    ) -> Vec<(PreservedPartial, PartialDiscardCause)> {
        self.retire_channel(ch_index)
            .map(|p| (p, cause))
            .into_iter()
            .collect()
    }
}

impl Default for PartialBunchAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a continuation at sequence `next` follows an assembly last at
/// `prev`: the next number, or for an unreliable one also a repeat.
pub(crate) fn continues(reliable: bool, prev: i32, next: i32) -> bool {
    prev.checked_add(1) == Some(next) || (!reliable && next == prev)
}

/// Append `bit_count` bits of `src` to `dst`, which ends on a byte boundary
/// (only a final fragment is unaligned, and nothing follows it), clearing the
/// last byte's unused high bits. `false` when the reservation fails.
fn append_bytes(dst: &mut Vec<u8>, src: &[u8], bit_count: usize) -> bool {
    let byte_count = bit_count.div_ceil(8);
    if dst.try_reserve_exact(byte_count).is_err() {
        return false;
    }
    dst.extend_from_slice(&src[..byte_count]);
    if bit_count % 8 != 0 {
        let last = dst.len() - 1;
        dst[last] &= (1 << (bit_count % 8)) - 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_bytes_keeps_only_the_declared_bits() {
        let mut dst = vec![0xAA];
        assert!(append_bytes(&mut dst, &[0x55], 8));
        assert!(append_bytes(&mut dst, &[0xFF], 5));
        assert_eq!(dst, vec![0xAA, 0x55, 0x1F]);
    }

    /// The three counters `add_fragment` moves, threaded through every call.
    #[derive(Default)]
    struct Counters {
        errs: u64,
        frags: u64,
        comps: u64,
    }

    impl Counters {
        /// Add a fragment on its header's own channel.
        fn add(
            &mut self,
            acc: &mut PartialBunchAccumulator,
            header: RawBunchHeader,
            data: &[u8],
            bits: usize,
        ) -> PartialBunchResult {
            let ch_index = header.ch_index;
            acc.add_fragment(
                ch_index,
                header,
                data,
                bits,
                &mut self.errs,
                &mut self.frags,
                &mut self.comps,
            )
        }
    }

    /// An initial fragment on `ch_index`.
    fn initial(ch_index: u32, ch_sequence: i32, b_reliable: bool) -> RawBunchHeader {
        RawBunchHeader {
            ch_index,
            b_partial: true,
            b_partial_initial: true,
            b_reliable,
            ch_sequence,
            ..Default::default()
        }
    }

    /// An unreliable continuation on `ch_index`, final when `last`.
    fn continuation(ch_index: u32, ch_sequence: i32, last: bool) -> RawBunchHeader {
        RawBunchHeader {
            ch_index,
            b_partial: true,
            b_partial_final: last,
            ch_sequence,
            ..Default::default()
        }
    }

    /// The reliable final fragment that follows `initial(ch_index, 1, true)`.
    fn reliable_final(ch_index: u32) -> RawBunchHeader {
        RawBunchHeader {
            b_reliable: true,
            ..continuation(ch_index, 2, true)
        }
    }

    #[test]
    fn accumulator_initial_plus_final() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();

        let r1 = c.add(&mut acc, initial(1, 1, true), &[0xAB], 8);
        assert!(!r1.should_process);

        let r2 = c.add(&mut acc, reliable_final(1), &[0xCD], 8);
        assert!(r2.should_process);
        assert_eq!(c.errs, 0);

        let (buf, bits, _hdr) = acc.take_completed(1).unwrap();
        assert_eq!(bits, 16);
        assert_eq!(buf[0], 0xAB);
        assert_eq!(buf[1], 0xCD);
        assert_eq!(acc.total_buffered_bits(), 0);
    }

    #[test]
    fn a_short_fragment_is_rejected_without_panicking_or_inventing_bits() {
        for (bytes, bits) in [(&[][..], 8), (&[0xAB][..], 9), (&[0xCD][..], usize::MAX)] {
            let mut acc = PartialBunchAccumulator::new();
            let mut c = Counters::default();
            let result = c.add(&mut acc, initial(1, 1, true), bytes, bits);
            assert!(result.header.has_partial_error);
            assert_eq!(result.error_kind, Some(PartialSequenceKind::PayloadOverrun));
            assert!(!result.should_process);
            assert_eq!(result.discarded_bits, bytes.len() * 8);
            assert_eq!(acc.active_count(), 0);
            assert_eq!(acc.total_buffered_bits(), 0);
            assert_eq!((c.errs, c.frags, c.comps), (1, 0, 0));
        }
    }

    #[test]
    fn a_short_fragment_preserves_the_retired_assembly_and_allows_recovery() {
        for replacement in [false, true] {
            let mut acc = PartialBunchAccumulator::new();
            let mut c = Counters::default();
            c.add(&mut acc, initial(1, 1, true), &[0xAA], 8);
            let header = if replacement {
                initial(1, 2, true)
            } else {
                reliable_final(1)
            };
            let rejected = c.add(&mut acc, header, &[0xBB], 16);
            assert!(rejected.header.has_partial_error);
            assert!(!rejected.should_process);
            assert_eq!(rejected.discarded_bits, 16, "only the two supplied bytes");
            assert_eq!(rejected.displaced.len(), 1);
            assert_eq!(rejected.displaced[0].0.buffer, [0xAA]);
            assert_eq!(rejected.displaced[0].0.bit_count, 8);
            assert_eq!(
                rejected.displaced[0].1,
                PartialDiscardCause::Sequence(PartialSequenceKind::PayloadOverrun)
            );
            assert_eq!(acc.active_count(), 0);
            assert_eq!(acc.total_buffered_bits(), 0);
            assert_eq!((c.errs, c.frags, c.comps), (1, 1, 0));

            c.add(&mut acc, initial(1, 1, true), &[0xCC], 8);
            assert!(
                c.add(&mut acc, reliable_final(1), &[0xDD], 8)
                    .should_process
            );
            let (bytes, bits, _) = acc.take_completed(1).unwrap();
            assert_eq!((bytes, bits), (vec![0xCC, 0xDD], 16));
            assert_eq!((c.errs, c.frags, c.comps), (1, 3, 1));
        }
    }

    /// A zero-payload final is still the second fragment: `partial_fragments`
    /// counts it, not only `partial_completed`.
    #[test]
    fn a_zero_payload_final_fragment_still_counts_as_a_fragment() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();

        c.add(&mut acc, initial(1, 1, true), &[0xAB], 8);
        assert_eq!(c.frags, 1);

        let r2 = c.add(&mut acc, reliable_final(1), &[], 0);
        assert!(r2.should_process);
        assert_eq!(c.errs, 0);
        assert_eq!(c.comps, 1);
        assert_eq!(
            c.frags, 2,
            "two fragments arrived to complete this bunch, not one"
        );
    }

    /// A final that arrives already errored (it re-declares `b_partial_initial`
    /// over an in-flight assembly) is not a completion, and its buffered bits
    /// are reported discarded rather than leaked.
    #[test]
    fn an_error_flagged_final_fragment_is_not_reported_as_a_completion() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();

        c.add(&mut acc, initial(1, 1, true), &[0xAA], 8);
        assert_eq!(c.frags, 1);

        // Overlapping initial: also final, on the same header.
        let h2 = RawBunchHeader {
            b_partial_final: true,
            ..initial(1, 2, true)
        };
        let r2 = c.add(&mut acc, h2, &[0xBB], 8);

        assert_eq!(c.errs, 1, "the overlapping initial must be flagged");
        assert!(!r2.should_process);
        assert_eq!(
            c.comps, 0,
            "an errored final must not be counted as a completion"
        );
        assert_eq!(c.frags, 2, "the second fragment still arrived");
        assert_eq!(
            r2.discarded_bits, 16,
            "the discarded initial (8 bits) plus the errored final's own \
             buffered bits (8 bits), not left to leak uncounted"
        );
        assert!(
            acc.take_completed(1).is_none(),
            "nothing was left behind to hand to a caller"
        );
        assert!(
            acc.drain_unfinished().is_empty(),
            "nothing was left behind for drain_unfinished to skip, either"
        );
    }

    /// An unreliable continuation may repeat the largest sequence number: no
    /// `+ 1` overflow.
    #[test]
    fn a_continuation_at_the_largest_sequence_does_not_overflow() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, i32::MAX, false), &[0xAA], 8);
        let last = c.add(&mut acc, continuation(1, i32::MAX, true), &[0xBB], 8);
        assert!(last.should_process);
        assert_eq!(c.errs, 0);
    }

    #[test]
    fn a_reliable_continuation_may_not_repeat_its_sequence() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 5, true), &[0xAA], 8);
        let mut repeat = continuation(1, 5, true);
        repeat.b_reliable = true;
        let last = c.add(&mut acc, repeat, &[0xBB], 8);
        assert_eq!(
            last.error_kind,
            Some(PartialSequenceKind::MismatchedContinuation)
        );
        assert_eq!(c.errs, 1);
    }

    #[test]
    fn partial_reassembly_refuses_more_active_states_than_its_budget() {
        let mut acc = PartialBunchAccumulator::with_limits(1, 64);
        let mut c = Counters::default();
        let first = c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
        assert_eq!(first.resource_limit, None);
        let refused = c.add(&mut acc, initial(2, 0, false), &[0xBB], 8);
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
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
        let refused = c.add(&mut acc, continuation(1, 0, true), &[0x1F], 5);
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
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 1, true), &[0xAA], 8);

        // Unreliable after a reliable initial.
        let rejected = c.add(&mut acc, continuation(1, 2, true), &[0x1F], 5);
        assert_eq!(
            rejected.error_kind,
            Some(PartialSequenceKind::MismatchedContinuation)
        );
        assert_eq!(rejected.discarded_bits, 13, "8 buffered + 5 current");
        assert_eq!(acc.total_buffered_bits(), 0);

        let rejected = c.add(&mut acc, continuation(2, 0, true), &[0x7F], 7);
        assert_eq!(
            rejected.error_kind,
            Some(PartialSequenceKind::MissingInitial)
        );
        assert_eq!(rejected.discarded_bits, 7, "the refused current fragment");
    }

    #[test]
    fn overlapping_initial_reports_the_replaced_payload_but_keeps_the_new_one() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 1, true), &[0xAA], 8);
        let replacement = c.add(&mut acc, initial(1, 2, true), &[0xBB, 0xCC], 16);
        assert_eq!(replacement.discarded_bits, 8);
        assert_eq!(
            replacement.error_kind,
            Some(PartialSequenceKind::OverlappingInitial)
        );
        assert!(replacement.overlapping_initial);
        assert_eq!(replacement.displaced.len(), 1);
        assert_eq!(
            replacement.displaced[0].1,
            PartialDiscardCause::Sequence(PartialSequenceKind::OverlappingInitial)
        );
        assert_eq!(acc.total_buffered_bits(), 16);
        assert_eq!(acc.active_count(), 1);
    }

    #[test]
    fn non_aligned_nonfinal_reports_buffered_and_current_bits() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
        let rejected = c.add(&mut acc, continuation(1, 0, false), &[0x07], 3);
        assert_eq!(rejected.discarded_bits, 11);
        assert_eq!(
            rejected.error_kind,
            Some(PartialSequenceKind::NonByteAlignedFragment)
        );
        assert_eq!(acc.total_buffered_bits(), 0);
        assert_eq!(rejected.displaced.len(), 1);
        assert_eq!(
            rejected.displaced[0].1,
            PartialDiscardCause::Sequence(PartialSequenceKind::NonByteAlignedFragment)
        );
    }

    /// A refused initial displaces nothing of its own: the empty assembly
    /// `validate_sequence` started for it before the alignment and budget
    /// checks is retired, not handed back as a displaced payload.
    #[test]
    fn a_refused_initial_displaces_nothing_of_its_own() {
        let displaced_bits = |result: &PartialBunchResult| {
            result
                .displaced
                .iter()
                .map(|(p, _)| p.bit_count)
                .collect::<Vec<_>>()
        };
        let mut c = Counters::default();

        let mut acc = PartialBunchAccumulator::new();
        let unaligned = c.add(&mut acc, initial(1, 0, false), &[0x1F], 5);
        assert_eq!(
            unaligned.error_kind,
            Some(PartialSequenceKind::NonByteAlignedFragment)
        );
        assert_eq!(displaced_bits(&unaligned), Vec::<usize>::new());
        assert_eq!(unaligned.discarded_bits, 5);
        assert_eq!((acc.active_count(), acc.total_buffered_bits()), (0, 0));

        let mut acc = PartialBunchAccumulator::with_limits(2, 4);
        let over_budget = c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
        assert_eq!(
            over_budget.resource_limit,
            Some(PartialResourceLimit::BufferedBits)
        );
        assert_eq!(displaced_bits(&over_budget), Vec::<usize>::new());
        assert_eq!(over_budget.discarded_bits, 8);
        assert_eq!((acc.active_count(), acc.total_buffered_bits()), (0, 0));
        assert_eq!(c.errs, 2);
    }

    #[test]
    fn overlapping_empty_initial_retains_its_cause() {
        let mut acc = PartialBunchAccumulator::new();
        let mut c = Counters::default();
        c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
        let result = c.add(&mut acc, initial(1, 0, false), &[], 0);
        assert!(result.overlapping_initial);
        assert_eq!(c.errs, 1);
    }

    /// An overlapping initial that is also refused on its own (unaligned, or
    /// over the buffered-bits budget) keeps both causes and counts two
    /// errors; only the replaced 8-bit assembly is a displaced payload, not
    /// the empty one started for the refused initial.
    #[test]
    fn an_overlapping_initial_refused_on_its_own_keeps_both_causes() {
        let unaligned = Some(PartialSequenceKind::NonByteAlignedFragment);
        let over_budget = Some(PartialResourceLimit::BufferedBits);
        for (mut acc, data, bits, error_kind, resource_limit) in [
            (
                PartialBunchAccumulator::new(),
                &[0x07][..],
                3,
                unaligned,
                None,
            ),
            (
                PartialBunchAccumulator::with_limits(2, 12),
                &[0xBB, 0xCC][..],
                16,
                None,
                over_budget,
            ),
        ] {
            let mut c = Counters::default();
            c.add(&mut acc, initial(1, 0, false), &[0xAA], 8);
            let result = c.add(&mut acc, initial(1, 0, false), data, bits);
            assert!(result.overlapping_initial);
            assert_eq!(result.error_kind, error_kind);
            assert_eq!(result.resource_limit, resource_limit);
            assert_eq!(result.displaced.len(), 1);
            assert_eq!(
                result.displaced[0].1,
                PartialDiscardCause::Sequence(PartialSequenceKind::OverlappingInitial)
            );
            assert_eq!(result.displaced[0].0.bit_count, 8);
            assert_eq!(result.discarded_bits, 8 + bits, "replaced + current");
            assert_eq!(c.errs, 2);
        }
    }
}
