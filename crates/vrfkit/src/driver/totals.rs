//! The one place a packet sink's counters are accumulated.
//!
//! `ExportSink` is rebuilt for every packet -- ~530,000 times on the reference
//! replay -- so anything it counts and the caller does not read is discarded
//! that many times and reads as a permanent zero. That is not a hypothetical:
//! `cnc_rpcs_emitted` is the only signal that the `AbilitiesAndBuffsComponent`
//! brute-force produced RPC structure rather than leaving an opaque blob, and
//! it reached no summary at all. A build that stopped reaching that decoder
//! would have left "Decode errors: 0" and every other line of the export
//! summary exactly where a good run leaves them.
//!
//! The checkpoint pass had its own copy of the same loop and its own subset of
//! the same omission: [`ArrayDecodeStats::errors`], `truncated_rpcs` and the
//! movement-decode errors were dropped there, so a checkpoint array that
//! overran mid-element wrote its parent raw row, lost its flattened children,
//! and recorded no failure anywhere.
//!
//! Both passes now go through [`SinkTotals::absorb`]. One function, one test,
//! and a counter added to `ExportStats` has exactly one place to be wired in.

use vrf_decode::{ArrayDecodeStats, OverlayErrorReport, OverlayStats};

use crate::sink::ExportStats;

/// Everything a packet's sink counted, summed across packets.
#[derive(Debug, Default)]
pub(super) struct SinkTotals {
    pub overlay: OverlayStats,
    pub effect_blobs_decoded: u64,
    pub struct_blobs_decoded: u64,
    pub struct_blobs_failed: u64,
    /// First failure verbatim; a later packet must not overwrite the one that
    /// names the build change.
    pub struct_blob_first_error: Option<String>,
    pub multi_contents_items_emitted: u64,
    pub movement_rpc_errors: u64,
    pub movement_first_error: Option<String>,
    pub array: ArrayDecodeStats,
    pub array_leaf_decode_errors: u64,
    pub truncated_rpcs: u64,
    pub rpc_suffix_bits_dropped: u64,
    pub cnc_rpcs_emitted: u64,
}

impl SinkTotals {
    /// Fold one packet's counters in.
    ///
    /// `error_report` is passed in rather than owned because both passes merge
    /// into the *same* report: a decode error is a decode error wherever it
    /// happened, and the breakdown the summary prints is the only place a
    /// checkpoint-only failure would ever be seen.
    ///
    /// `stats` is taken by `&mut` for the two `Option<String>` fields, which are
    /// moved out rather than cloned -- they are only ever set once per run.
    pub(super) fn absorb(
        &mut self,
        stats: &mut ExportStats,
        error_report: &mut OverlayErrorReport,
    ) {
        self.overlay.decoded_ok += stats.overlay.decoded_ok;
        self.overlay.decoded_err += stats.overlay.decoded_err;
        self.overlay.raw_or_skip += stats.overlay.raw_or_skip;
        self.overlay.not_in_table += stats.overlay.not_in_table;
        self.overlay.no_field_name += stats.overlay.no_field_name;
        self.overlay.handle_conflicts_refused += stats.overlay.handle_conflicts_refused;
        self.effect_blobs_decoded += stats.effect_blobs_decoded;
        self.struct_blobs_decoded += stats.struct_blobs_decoded;
        self.struct_blobs_failed += stats.struct_blobs_failed;
        if self.struct_blob_first_error.is_none() {
            self.struct_blob_first_error = stats.struct_blob_first_error.take();
        }
        self.multi_contents_items_emitted += stats.multi_contents_items_emitted;
        self.movement_rpc_errors += stats.movement_rpc_errors;
        if self.movement_first_error.is_none() {
            self.movement_first_error = stats.movement_first_error.take();
        }
        self.array.elements_decoded += stats.array.elements_decoded;
        self.array.fields_emitted += stats.array.fields_emitted;
        self.array.truncations += stats.array.truncations;
        self.array.errors += stats.array.errors;
        self.array.unconsumed_nested_bits += stats.array.unconsumed_nested_bits;
        self.array.implicit_terminations += stats.array.implicit_terminations;
        self.array.unconsumed_root_bits += stats.array.unconsumed_root_bits;
        self.array_leaf_decode_errors += stats.array_leaf_decode_errors;
        self.truncated_rpcs += stats.truncated_rpcs;
        self.rpc_suffix_bits_dropped += stats.rpc_suffix_bits_dropped;
        self.cnc_rpcs_emitted += stats.cnc_rpcs_emitted;
        error_report.merge_from(&stats.overlay.error_report);
    }
}
