//! The opt-in, bounded aggregate of stream failures.
//!
//! [`ChannelState::push_stream_failure`](super::ChannelState::push_stream_failure)
//! keeps the first `MAX_STREAM_FAILURE_RECORDS` failure lines and silently
//! drops the rest: that buffer exists for a human reading one replay's
//! `validate` output, and on the 2026-09-07 corpus every one of 714 replays
//! saturated it, so what the window showed was "the first 32 failures, all
//! from the match's opening seconds" -- a biased sample that no reweighting
//! can turn into a population figure.
//!
//! This module is the counterpart the window can never be: one cell per
//! distinct (kind, cause, group path, function count, handle, consumed bits,
//! preservation state) combination,
//! counting every failure. The total counters are exact; distinct cells and
//! optional payload samples are capped so malformed input cannot make memory
//! grow without bound. The two invariants
//! the rest of the code relies on:
//!
//! 1. `total_failures` == `field_stream_failures + rpc_stream_failures` for
//!    the same pass. Every site that bumps one of those counters also calls
//!    [`FailureAggregate::note_failure`] (see `framing.rs` in `vrf-net` and
//!    `stream.rs` here), so a mismatch is a wiring bug, not sampling noise.
//! 2. `preserved_unresolved` is the historical counter name for failures whose
//!    whole decoded stream reached a raw preservation row. It includes both
//!    unresolved ClassNetCache blocks and unparsed post-RepLayout tails. They
//!    are a subset of `total_failures`, never part of real loss; the counterpart is
//!    `real_loss()` == `total_failures - preserved_unresolved`, comparable
//!    with `field_stream_failures + max(0, rpc_stream_failures -
//!    unresolved_rpc_payloads_preserved)`.
//!
//! Nothing here feeds a verdict, a counter the summary prints, or any row the
//! tables carry. It is enabled and read only by the `diag` subcommand.

use std::sync::Arc;

use vrf_net::pipeline::{StreamFailure, StreamFailureCause, StreamKind};
use vrf_schema::FxHashMap;

/// Detailed failure records retained per cell. Counts continue after sample
/// retention stops.
pub const MAX_SAMPLES_PER_CELL: usize = 3;

/// Maximum number of distinct keyed cells retained for one pass. Exact totals
/// continue in `overflow` after this limit, but new attacker-controlled group
/// paths cannot grow the map indefinitely.
pub const MAX_FAILURE_CELLS: usize = 4096;

/// Longest payload retained by one sample, in bytes. The unresolved RPC
/// payloads worth reading (the AbilitiesAndBuffs GAS state-sync stream, the
/// brute-forceable ClassNetCache blocks) are far below this; the cap only
/// stops one huge block from dominating the output.
pub const MAX_SAMPLE_PAYLOAD_BYTES: usize = 96;

/// Payloads longer than this are not retained at all rather than truncated to
/// a prefix that misrepresents the block.
const MAX_SAMPLE_PAYLOAD_BLOCK_BITS: u64 = 8 * 1024;

/// One aggregate cell: every failure sharing one key.
#[derive(Debug, Default, Clone)]
pub struct FailureCell {
    /// How many failures landed here. Never sampled.
    pub count: u64,
    /// Sum of the blocks' declared bit lengths.
    pub bit_count_total: u64,
    /// Sum of bits consumed before each failure.
    pub consumed_bits_total: u64,
    /// Sum of bits abandoned by each failure.
    pub abandoned_bits_total: u64,
    /// First samples, bounded by [`MAX_SAMPLES_PER_CELL`].
    pub samples: Vec<FailureSample>,
}

/// One representative failure. Everything except `payload_hex` is exact for
/// that event; `payload_hex` is the block's decoded leading bytes, present
/// only where the caller had the payload and it fit the caps.
#[derive(Debug, Clone)]
pub struct FailureSample {
    pub actor_net_guid: u32,
    pub bit_count: u32,
    pub consumed_bits: u64,
    /// Whether the complete failed stream was retained as raw bits.
    pub payload_preserved: bool,
    pub abandoned_bits: u64,
    pub record_offset: Option<u64>,
    pub payload_hex: Option<String>,
    pub payload_truncated: bool,
}

/// The dimensions every failure is aggregated under.
///
/// `consumed_bits` is part of the key, not a sum: it is the dimension that
/// separates a grammar drift that always stops at the same offset (e.g. a
/// field stream that consumes 185 of 200 bits) from one that stops at random
/// offsets, and a sum could only give an average over a mixture. In practice
/// a group fails at very few distinct offsets, so the cell count stays small.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FailureKey {
    /// Which grammar failed.
    pub kind: StreamKind,
    /// Which stage of the walk failed.
    pub cause: StreamFailureCause,
    /// Group path resolved for the block at failure time. A path the resolver
    /// could not name appears as whatever the resolver left -- a bare instance
    /// name or `<unknown:{guid}>` -- which is recorded as-is rather than
    /// guessed into a class.
    pub group_path: Arc<str>,
    /// Function count the RPC handle read used (0 for RepLayout and for
    /// unresolved groups).
    pub function_count: u32,
    /// Handle of the failing record, when the walk tracked one.
    pub record_handle: Option<u32>,
    /// Bits consumed before the failure, exact.
    pub consumed_bits: u64,
    /// Whether every bit in the failed stream reached a raw preservation row.
    pub payload_preserved: bool,
}

/// The bounded aggregate for one pass (main ReplayData or checkpoint).
#[derive(Debug, Clone)]
pub struct FailureAggregate {
    cells: FxHashMap<FailureKey, FailureCell>,
    /// Failures whose key arrived after the distinct-cell cap was reached.
    overflow: FailureCell,
    /// Every failure counted. Reconciles with
    /// `field_stream_failures + rpc_stream_failures`.
    total_failures: u64,
    /// The subset whose failed stream was preserved as a whole. The name is
    /// historical; it includes unresolved ClassNetCache blocks and unparsed
    /// post-RepLayout tails. Reconciles with
    /// `unresolved_rpc_payloads_preserved`.
    preserved_unresolved: u64,
    /// Whether decoded payload bytes may be retained in bounded samples.
    retain_payloads: bool,
}

impl Default for FailureAggregate {
    fn default() -> Self {
        Self::new(false)
    }
}

impl FailureAggregate {
    /// Create an aggregate. Payload retention is disabled unless the caller
    /// explicitly opts in; all counters and non-payload dimensions remain.
    pub fn new(retain_payloads: bool) -> Self {
        Self {
            cells: FxHashMap::default(),
            overflow: FailureCell::default(),
            total_failures: 0,
            preserved_unresolved: 0,
            retain_payloads,
        }
    }

    /// Maximum number of distinct keyed cells retained.
    pub fn cell_limit() -> usize {
        MAX_FAILURE_CELLS
    }

    /// Record one failure for counting. Samples are NOT taken here: they come
    /// from [`Self::note_payload`], which only the callers that hold the
    /// decoded bytes can serve (the unresolved-preservation callback and the
    /// failure-payload callback). Whether the whole payload reached a
    /// preservation row is carried by `failure` so the aggregate key and
    /// reconciled counter cannot disagree.
    pub fn note_failure(&mut self, failure: &StreamFailure, group_path: Arc<str>) {
        let key = FailureKey {
            kind: failure.kind,
            cause: failure.cause,
            group_path,
            function_count: failure.function_count,
            record_handle: failure.record_handle,
            consumed_bits: failure.consumed_bits,
            payload_preserved: failure.payload_preserved,
        };
        let cell = if let Some(cell) = self.cells.get_mut(&key) {
            cell
        } else if self.cells.len() < MAX_FAILURE_CELLS {
            self.cells.entry(key).or_default()
        } else {
            &mut self.overflow
        };
        cell.count += 1;
        cell.bit_count_total += u64::from(failure.bit_count);
        cell.consumed_bits_total += failure.consumed_bits;
        cell.abandoned_bits_total += failure.remaining_bits;
        self.total_failures += 1;
        if failure.payload_preserved {
            self.preserved_unresolved += 1;
        }
    }

    /// Attach one real-payload sample to a cell. Called from
    /// `on_unresolved_class_net_cache_payload` (whose block `on_stream_failure`
    /// counts right after) and from `on_stream_failure_payload` (whose block
    /// `on_stream_failure` has already counted) -- either way the count is
    /// moved by exactly one [`Self::note_failure`] per failure, never here.
    pub fn note_payload(&mut self, failure: &StreamFailure, group_path: Arc<str>, payload: &[u8]) {
        if !self.retain_payloads {
            return;
        }
        let key = FailureKey {
            kind: failure.kind,
            cause: failure.cause,
            group_path,
            function_count: failure.function_count,
            record_handle: failure.record_handle,
            consumed_bits: failure.consumed_bits,
            payload_preserved: failure.payload_preserved,
        };
        let cell = if let Some(cell) = self.cells.get_mut(&key) {
            cell
        } else if self.cells.len() < MAX_FAILURE_CELLS {
            self.cells.entry(key).or_default()
        } else {
            return;
        };
        if cell.samples.len() >= MAX_SAMPLES_PER_CELL {
            return;
        }
        if payload.len() as u64 > MAX_SAMPLE_PAYLOAD_BLOCK_BITS / 8 {
            // Too big to represent honestly with a prefix; record the event's
            // shape without a payload instead.
            cell.samples.push(FailureSample {
                actor_net_guid: failure.actor_net_guid.0,
                bit_count: failure.bit_count,
                consumed_bits: failure.consumed_bits,
                payload_preserved: failure.payload_preserved,
                abandoned_bits: failure.remaining_bits,
                record_offset: failure.record_offset,
                payload_hex: None,
                payload_truncated: false,
            });
            return;
        }
        let truncated = payload.len() > MAX_SAMPLE_PAYLOAD_BYTES;
        let take = payload.len().min(MAX_SAMPLE_PAYLOAD_BYTES);
        cell.samples.push(FailureSample {
            actor_net_guid: failure.actor_net_guid.0,
            bit_count: failure.bit_count,
            consumed_bits: failure.consumed_bits,
            payload_preserved: failure.payload_preserved,
            abandoned_bits: failure.remaining_bits,
            record_offset: failure.record_offset,
            payload_hex: Some(hex(&payload[..take])),
            payload_truncated: truncated,
        });
    }

    /// Merge another pass's aggregate in (checkpoint chunks are walked one
    /// archive at a time, each with its own channel state). Counts and sums
    /// add; samples fill this side up to the cap from the other's.
    pub fn absorb(&mut self, other: &mut Self) {
        self.total_failures += other.total_failures;
        self.preserved_unresolved += other.preserved_unresolved;
        self.overflow.count += other.overflow.count;
        self.overflow.bit_count_total += other.overflow.bit_count_total;
        self.overflow.consumed_bits_total += other.overflow.consumed_bits_total;
        self.overflow.abandoned_bits_total += other.overflow.abandoned_bits_total;
        let mut other_cells: Vec<_> = other.cells.drain().collect();
        other_cells.sort_by(|(a, _), (b, _)| compare_keys(a, b));
        for (key, mut other_cell) in other_cells {
            let cell = if let Some(cell) = self.cells.get_mut(&key) {
                cell
            } else if self.cells.len() < MAX_FAILURE_CELLS {
                self.cells.entry(key).or_default()
            } else {
                self.overflow.count += other_cell.count;
                self.overflow.bit_count_total += other_cell.bit_count_total;
                self.overflow.consumed_bits_total += other_cell.consumed_bits_total;
                self.overflow.abandoned_bits_total += other_cell.abandoned_bits_total;
                continue;
            };
            cell.count += other_cell.count;
            cell.bit_count_total += other_cell.bit_count_total;
            cell.consumed_bits_total += other_cell.consumed_bits_total;
            cell.abandoned_bits_total += other_cell.abandoned_bits_total;
            let take = if self.retain_payloads {
                MAX_SAMPLES_PER_CELL
                    .saturating_sub(cell.samples.len())
                    .min(other_cell.samples.len())
            } else {
                0
            };
            cell.samples.extend(other_cell.samples.drain(..take));
            other_cell.samples.clear();
        }
    }

    /// Every failure counted. Reconciles with
    /// `field_stream_failures + rpc_stream_failures` for the same pass.
    pub fn total_failures(&self) -> u64 {
        self.total_failures
    }

    /// The wholly preserved subset (historical name). Reconciles with
    /// `unresolved_rpc_payloads_preserved` for the same pass.
    pub fn preserved_unresolved(&self) -> u64 {
        self.preserved_unresolved
    }

    /// Stream failures not wholly preserved by the unresolved-RPC callback.
    /// Earlier fields in a failed block may still have emitted rows; this is
    /// a block count, not a count of entirely missing payloads or values.
    /// Reconciles with `field_stream_failures +
    /// max(0, rpc_stream_failures - unresolved_rpc_payloads_preserved)`.
    pub fn real_loss(&self) -> u64 {
        self.total_failures - self.preserved_unresolved
    }

    /// Failures counted exactly but omitted from keyed cells after the cap.
    pub fn overflow(&self) -> &FailureCell {
        &self.overflow
    }

    /// Whether raw decoded payload bytes were explicitly enabled.
    pub fn retains_payloads(&self) -> bool {
        self.retain_payloads
    }

    /// The cells, ordered for deterministic output: count descending, then the
    /// key's fields ascending. Ties in count must not shuffle between runs.
    pub fn cells_sorted(&self) -> Vec<(&FailureKey, &FailureCell)> {
        let mut cells: Vec<(&FailureKey, &FailureCell)> = self.cells.iter().collect();
        cells.sort_by(|(a, ac), (b, bc)| bc.count.cmp(&ac.count).then_with(|| compare_keys(a, b)));
        cells
    }
}

fn compare_keys(a: &FailureKey, b: &FailureKey) -> std::cmp::Ordering {
    kind_rank(a.kind)
        .cmp(&kind_rank(b.kind))
        .then_with(|| cause_rank(a.cause).cmp(&cause_rank(b.cause)))
        .then_with(|| a.group_path.cmp(&b.group_path))
        .then_with(|| a.function_count.cmp(&b.function_count))
        .then_with(|| a.record_handle.cmp(&b.record_handle))
        .then_with(|| a.consumed_bits.cmp(&b.consumed_bits))
        .then_with(|| a.payload_preserved.cmp(&b.payload_preserved))
}

fn kind_rank(kind: StreamKind) -> u8 {
    match kind {
        StreamKind::RepLayout => 0,
        StreamKind::Rpc => 1,
    }
}

fn cause_rank(cause: StreamFailureCause) -> u8 {
    match cause {
        StreamFailureCause::AbandonedTail => 0,
        StreamFailureCause::ReadError => 1,
        StreamFailureCause::UnresolvedFunctionCount => 2,
        StreamFailureCause::UnverifiedRepLayoutTail => 3,
        StreamFailureCause::WindowOpenFailed => 4,
    }
}

/// Lowercase hex without separators.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xF) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_net::types::NetworkGuid;

    fn failure(kind: StreamKind, cause: StreamFailureCause, consumed: u64) -> StreamFailure {
        StreamFailure {
            kind,
            actor_net_guid: NetworkGuid(7),
            bit_count: 200,
            function_count: 0,
            consumed_bits: consumed,
            remaining_bits: 200 - consumed,
            cause,
            record_handle: Some(3),
            record_offset: Some(consumed),
            payload_preserved: false,
        }
    }

    /// A cell whose payload was preserved is a subset, never real loss, and
    /// the two totals stay separable.
    #[test]
    fn preserved_unresolved_is_counted_but_not_as_loss() {
        let mut agg = FailureAggregate::new(true);
        let mut f = failure(
            StreamKind::Rpc,
            StreamFailureCause::UnresolvedFunctionCount,
            0,
        );
        f.payload_preserved = true;
        agg.note_payload(&f, Arc::from("AbilitiesAndBuffsComponent"), &[0xAA, 0xBB]);
        agg.note_failure(&f, Arc::from("AbilitiesAndBuffsComponent"));
        assert_eq!(agg.total_failures(), 1);
        assert_eq!(agg.preserved_unresolved(), 1);
        assert_eq!(agg.real_loss(), 0);
        let (key, cell) = &agg.cells_sorted()[0];
        assert_eq!(key.cause, StreamFailureCause::UnresolvedFunctionCount);
        assert_eq!(cell.count, 1);
        // The sample carries the real payload bytes, not a stub.
        assert_eq!(cell.samples[0].payload_hex.as_deref(), Some("aabb"));
    }

    /// A RepLayout failure and a genuinely lost RPC failure are both real
    /// loss, separated by kind.
    #[test]
    fn real_loss_keeps_rep_layout_and_rpc_apart() {
        let mut agg = FailureAggregate::default();
        let field = failure(
            StreamKind::RepLayout,
            StreamFailureCause::AbandonedTail,
            185,
        );
        let rpc = failure(StreamKind::Rpc, StreamFailureCause::ReadError, 0);
        agg.note_failure(
            &field,
            Arc::from("/Script/ShooterGame.AresAbilitySystemComponent"),
        );
        agg.note_failure(&rpc, Arc::from("SomeUnresolved"));
        assert_eq!(agg.total_failures(), 2);
        assert_eq!(agg.preserved_unresolved(), 0);
        assert_eq!(agg.real_loss(), 2);
        let cells = agg.cells_sorted();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].0.kind, StreamKind::RepLayout);
        assert_eq!(cells[0].0.cause, StreamFailureCause::AbandonedTail);
        assert_eq!(cells[1].0.kind, StreamKind::Rpc);
        assert_eq!(cells[1].0.cause, StreamFailureCause::ReadError);
    }

    /// Counts continue past the sample cap; only samples stop.
    #[test]
    fn counts_continue_past_the_sample_cap() {
        let mut agg = FailureAggregate::default();
        let f = failure(
            StreamKind::RepLayout,
            StreamFailureCause::AbandonedTail,
            185,
        );
        let path = Arc::from("/Script/ShooterGame.AresAbilitySystemComponent");
        for _ in 0..1000 {
            agg.note_failure(&f, Arc::clone(&path));
        }
        let (_, cell) = &agg.cells_sorted()[0];
        assert_eq!(cell.count, 1000, "the count is never capped");
        assert_eq!(
            cell.samples.len(),
            0,
            "counting alone takes no samples; payloads do"
        );
        assert_eq!(agg.total_failures(), 1000);
    }

    /// Absorb adds counts and moves samples up to the cap, so a checkpoint
    /// pass merged into a main pass cannot duplicate or lose a total.
    #[test]
    fn absorb_adds_counts_and_fills_samples() {
        let path = Arc::from("/Script/ShooterGame.X");
        let f = failure(
            StreamKind::RepLayout,
            StreamFailureCause::AbandonedTail,
            100,
        );
        let mut main = FailureAggregate::new(true);
        for _ in 0..4 {
            main.note_payload(&f, Arc::clone(&path), &[0xAA]);
            main.note_failure(&f, Arc::clone(&path));
        }
        let mut cp = FailureAggregate::new(true);
        for _ in 0..6 {
            cp.note_payload(&f, Arc::clone(&path), &[0xBB]);
            cp.note_failure(&f, Arc::clone(&path));
        }
        main.absorb(&mut cp);
        assert_eq!(main.total_failures(), 10);
        assert_eq!(main.real_loss(), 10);
        let (_, cell) = &main.cells_sorted()[0];
        assert_eq!(cell.count, 10);
        assert_eq!(cell.samples.len(), MAX_SAMPLES_PER_CELL);
        assert!(cp.cells_sorted().is_empty(), "absorb drains the source");
    }

    /// A payload too large to represent honestly is recorded as a shape-only
    /// sample, never as a prefix that misrepresents the block.
    #[test]
    fn an_oversized_payload_is_not_truncated_to_a_prefix() {
        let mut agg = FailureAggregate::new(true);
        let mut f = failure(
            StreamKind::Rpc,
            StreamFailureCause::UnresolvedFunctionCount,
            0,
        );
        f.payload_preserved = true;
        let big = vec![0x77; (MAX_SAMPLE_PAYLOAD_BLOCK_BITS / 8) as usize + 1];
        agg.note_payload(&f, Arc::from("Big"), &big);
        agg.note_failure(&f, Arc::from("Big"));
        let (_, cell) = &agg.cells_sorted()[0];
        assert_eq!(cell.count, 1);
        assert!(cell.samples[0].payload_hex.is_none());
    }

    #[test]
    fn default_does_not_retain_payloads() {
        let mut agg = FailureAggregate::default();
        let f = failure(StreamKind::Rpc, StreamFailureCause::ReadError, 0);
        agg.note_payload(&f, Arc::from("Group"), &[0xAA]);
        agg.note_failure(&f, Arc::from("Group"));
        assert!(agg.cells_sorted()[0].1.samples.is_empty());
    }

    #[test]
    fn distinct_cell_cap_preserves_exact_totals_in_overflow() {
        let mut agg = FailureAggregate::default();
        let f = failure(StreamKind::RepLayout, StreamFailureCause::ReadError, 0);
        for index in 0..=MAX_FAILURE_CELLS {
            agg.note_failure(&f, Arc::from(format!("Group{index}")));
        }

        assert_eq!(agg.total_failures(), (MAX_FAILURE_CELLS + 1) as u64);
        assert_eq!(agg.cells_sorted().len(), MAX_FAILURE_CELLS);
        assert_eq!(agg.overflow().count, 1);
        let retained: u64 = agg.cells_sorted().iter().map(|(_, cell)| cell.count).sum();
        assert_eq!(retained + agg.overflow().count, agg.total_failures());
    }
}
