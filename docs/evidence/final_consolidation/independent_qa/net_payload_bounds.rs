//! Independent public-API contracts for malformed partial-bunch input.
//! Copy to crates/vrf-net/tests/qa_final_payload_bounds.rs for a recorded run.

use vrf_net::bunch::{PartialBunchAccumulator, PartialBunchResult, RawBunchHeader};

fn header(channel: u32, sequence: i32, initial: bool, final_fragment: bool) -> RawBunchHeader {
    RawBunchHeader {
        packet_id: sequence,
        ch_index: channel,
        ch_sequence: sequence,
        b_reliable: true,
        b_partial: true,
        b_partial_initial: initial,
        b_partial_final: final_fragment,
        ..Default::default()
    }
}

fn add(
    accumulator: &mut PartialBunchAccumulator,
    header: RawBunchHeader,
    bytes: &[u8],
    bits: usize,
    stats: &mut [u64; 3],
) -> PartialBunchResult {
    let [errors, fragments, completed] = stats;
    accumulator.add_fragment(header.ch_index, header, bytes, bits, errors, fragments, completed)
}

#[test]
fn qa_empty_short_fragment_is_refused_without_panicking_or_claiming_bytes() {
    let mut accumulator = PartialBunchAccumulator::new();
    let mut stats = [0; 3];
    let result = add(&mut accumulator, header(3, 1, true, true), &[], 8, &mut stats);
    assert!(!result.should_process && result.header.has_partial_error);
    assert!(result.error_kind.is_some());
    assert!(result.resource_limit.is_none(), "missing source bytes are not resource exhaustion");
    assert_eq!(result.discarded_bits, 0);
    assert!(result.displaced.is_empty());
    assert_eq!(stats, [1, 0, 0]);
    assert_eq!(accumulator.active_count(), 0);
    assert_eq!(accumulator.total_buffered_bits(), 0);
}

#[test]
fn qa_short_continuation_preserves_prior_bytes_and_does_not_touch_other_channels() {
    let mut accumulator = PartialBunchAccumulator::new();
    let mut stats = [0; 3];
    add(&mut accumulator, header(3, 1, true, false), &[0xa5, 0x5a], 16, &mut stats);
    add(&mut accumulator, header(4, 1, true, false), &[0x3c], 8, &mut stats);
    assert_eq!(accumulator.active_count(), 2);
    let result = add(&mut accumulator, header(3, 2, false, true), &[0x81], 16, &mut stats);
    assert!(!result.should_process && result.header.has_partial_error);
    assert!(result.error_kind.is_some() && result.resource_limit.is_none());
    assert_eq!(result.discarded_bits, 24, "prior16 + supplied8; absent bits are not data");
    assert_eq!(result.displaced.len(), 1);
    assert_eq!(result.displaced[0].0.buffer, [0xa5, 0x5a]);
    assert_eq!(result.displaced[0].0.bit_count, 16);
    assert_eq!(result.displaced[0].0.header.packet_id, 1);
    assert_eq!(stats, [1, 2, 0]);
    assert_eq!(accumulator.active_count(), 1);
    assert_eq!(accumulator.total_buffered_bits(), 8);
    let other = add(&mut accumulator, header(4, 2, false, true), &[0x15], 5, &mut stats);
    assert!(other.should_process && !other.header.has_partial_error);
    let completed = accumulator.take_completed(4).expect("the unrelated assembly completes");
    assert_eq!(completed.0, [0x3c, 0x15]);
    assert_eq!(completed.1, 13);
    assert_eq!(stats, [1, 3, 1]);
    assert_eq!(accumulator.total_buffered_bits(), 0);
}

#[test]
fn qa_refused_channel_can_restart_and_an_absurd_bit_claim_stays_bounded() {
    let mut accumulator = PartialBunchAccumulator::new();
    let mut stats = [0; 3];
    let refused = add(&mut accumulator, header(3, 1, true, false), &[0xff], usize::MAX, &mut stats);
    assert!(refused.error_kind.is_some() && !refused.should_process);
    assert_eq!(refused.discarded_bits, 8);
    assert_eq!(accumulator.active_count(), 0);
    let accepted = add(&mut accumulator, header(3, 2, true, true), &[0x15], 5, &mut stats);
    assert!(accepted.should_process);
    let (bytes, bits, _) = accumulator.take_completed(3).unwrap();
    assert_eq!(bytes, [0x15]);
    assert_eq!(bits, 5);
    assert_eq!(stats, [1, 1, 1]);
}
