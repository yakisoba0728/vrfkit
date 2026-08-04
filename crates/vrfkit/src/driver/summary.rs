//! The export summary printed to stderr.
//!
//! Every line here is pinned by `tools/check_export_baseline.py`, which reads
//! the counters back out of this text and cross-checks three of them against
//! the row counts of the files they name. Adding, removing or renaming a line
//! breaks that harness; do it deliberately or not at all.

use std::fs;
use std::path::Path;
use std::time::Duration;

use vrf_decode::{OverlayErrorReport, OverlayStats};
use vrf_net::stats::NetStats;

use super::checkpoints::CheckpointStats;

/// Everything the run counted that is not in [`NetStats`].
pub(super) struct RunTotals {
    pub chunks_processed: u32,
    pub total_packets: u32,
    pub export_groups: usize,
    pub movement_rows: u64,
    pub net_guid_rows: usize,
    pub event_rows: u64,
    /// Payload bytes an Event chunk declared that its own header layout does
    /// not reach. Zero across the corpus; counted rather than dropped in
    /// silence.
    pub event_trailing_bytes: u64,
    pub elapsed: Duration,
    pub effect_blobs_decoded: u64,
    pub struct_blobs_decoded: u64,
    pub struct_blobs_failed: u64,
    /// First struct-blob failure verbatim. Printed so a build that reshuffles
    /// handles names itself on the summary instead of being invisible.
    pub struct_blob_first_error: Option<String>,
}

/// Print the whole `=== Export complete ===` report.
pub(super) fn print(
    out_path: &Path,
    net_stats: &NetStats,
    totals: &RunTotals,
    overlay: &OverlayStats,
    error_report: &OverlayErrorReport,
    checkpoints: Option<&CheckpointStats>,
    manifest_path: &Path,
) {
    eprintln!();
    eprintln!("=== Export complete ===");
    eprintln!("  Chunks:           {}", totals.chunks_processed);
    eprintln!("  Packets:          {}", totals.total_packets);
    eprintln!("  Export groups:    {}", totals.export_groups);
    eprintln!("  Content blocks:   {}", net_stats.content_blocks);
    eprintln!("  RepLayout blocks: {}", net_stats.rep_layout_blocks);
    eprintln!("  ClassNetCache:    {}", net_stats.class_net_cache_blocks);
    eprintln!("  Fields:           {}", net_stats.fields);
    eprintln!("  RPCs:             {}", net_stats.rpcs);
    eprintln!("  Actor opens:      {}", net_stats.actor_opens);
    eprintln!("  Actor closes:     {}", net_stats.actor_closes);
    eprintln!("  Bunches:          {}", net_stats.bunches);
    eprintln!("  Malformed pkts:   {}", net_stats.malformed_packets);
    eprintln!("  Skipped bits:     {}", net_stats.skipped_bits);
    eprintln!("  Movement rows:    {}", totals.movement_rows);
    eprintln!("  NetGUID rows:     {}", totals.net_guid_rows);
    eprintln!("  Event rows:       {}", totals.event_rows);
    if totals.event_trailing_bytes > 0 {
        eprintln!(
            "  Event unread:     {} payload bytes",
            totals.event_trailing_bytes
        );
    }
    // Printed unconditionally, including the zero. A conditional line cannot
    // distinguish "no failures" from "this build stopped reaching the decoder
    // at all", and that second case is exactly what went unnoticed on 13.02.
    eprintln!(
        "  Struct blobs:     {} decoded / {} failed",
        totals.struct_blobs_decoded, totals.struct_blobs_failed
    );
    if let Some(err) = &totals.struct_blob_first_error {
        eprintln!("  Struct blob err:  {err}");
    }
    eprintln!("  Elapsed:          {:.2?}", totals.elapsed);

    if let Some(cp) = checkpoints {
        print_checkpoints(cp);
    }

    print_file_sizes(out_path, checkpoints.is_some(), manifest_path);
    print_overlay(overlay, totals.effect_blobs_decoded);
    print_decode_errors(overlay, error_report);
}

fn print_checkpoints(cp: &CheckpointStats) {
    eprintln!();
    eprintln!("=== Checkpoints ===");
    eprintln!("  Checkpoints:      {}", cp.chunks);
    eprintln!("  GUID entries:     {}", cp.guid_entries);
    eprintln!("  Group records:    {}", cp.group_records);
    eprintln!("  Exported fields:  {}", cp.exported_fields);
    eprintln!("  Frames:           {}", cp.frames);
    eprintln!("  Frame packets:    {}", cp.packets);
    eprintln!("  Checkpoint rows:  {}", cp.field_rows);
    // Printed, not silent: a checkpoint re-opens every live actor and replays
    // its state, so these two would corrupt the tables they would otherwise
    // land in. See CheckpointStats.
    eprintln!(
        "  Dropped:          {} actor / {} movement rows (snapshot re-opens)",
        cp.actor_rows_dropped, cp.movement_rows_dropped
    );
    eprintln!(
        "  Overlay:          {} decoded / {} errors / {} raw-skip / {} not-in-table / {} unnamed / {} effect blobs",
        cp.overlay.decoded_ok,
        cp.overlay.decoded_err,
        cp.overlay.raw_or_skip,
        cp.overlay.not_in_table,
        cp.overlay.no_field_name,
        cp.effect_blobs
    );
    // NOT "Struct blobs", which the main block already uses: every label here
    // is a regex anchor for check_export_baseline.py, and two blocks sharing
    // one label would leave the harness matching whichever came first.
    eprintln!(
        "  Checkpoint blobs: {} decoded / {} failed",
        cp.struct_blobs_decoded, cp.struct_blobs_failed
    );
}

fn print_file_sizes(out_path: &Path, with_checkpoints: bool, manifest_path: &Path) {
    let size = |name: &str| {
        fs::metadata(out_path.join(name))
            .map(|m| m.len())
            .unwrap_or(0)
    };

    eprintln!();
    eprintln!("  fields.parquet:   {} bytes", size("fields.parquet"));
    eprintln!("  movement.parquet: {} bytes", size("movement.parquet"));
    eprintln!("  actors.parquet:   {} bytes", size("actors.parquet"));
    eprintln!("  net_guids.parquet:{} bytes", size("net_guids.parquet"));
    eprintln!("  events.parquet:   {} bytes", size("events.parquet"));
    if with_checkpoints {
        eprintln!(
            "  checkpoint_fields.parquet: {} bytes",
            size("checkpoint_fields.parquet")
        );
    }
    eprintln!("  manifest.json:    {}", manifest_path.display());
}

/// The denominator is every row the overlay was offered, which since RPC
/// parameter expansion means replicated properties *and* RPC parameters. The
/// two populations have very different type coverage -- the descriptor set grew
/// up around properties -- so labelling the ratio "of all fields" would read as
/// a regression when parameters were added. Name the denominator instead of
/// leaving it implicit.
fn print_overlay(overlay: &OverlayStats, effect_blobs_decoded: u64) {
    let total = overlay.decoded_ok
        + overlay.decoded_err
        + overlay.raw_or_skip
        + overlay.not_in_table
        + overlay.no_field_name;
    let pct = if total > 0 {
        (overlay.decoded_ok as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    eprintln!();
    eprintln!("=== Type overlay ===");
    eprintln!("  Decoded OK:       {}", overlay.decoded_ok);
    eprintln!("  Decode errors:    {}", overlay.decoded_err);
    eprintln!("  Raw/Skip:         {}", overlay.raw_or_skip);
    eprintln!("  Not in table:     {}", overlay.not_in_table);
    eprintln!("  No field name:    {}", overlay.no_field_name);
    eprintln!("  Rows offered:     {total}");
    eprintln!("  Typed:            {pct:.1}% (properties + RPC parameters)");
    // Reported separately because it is NOT part of the ratio above. The
    // overlay buckets are decided before the effect pass runs, so these rows
    // are already counted as `Not in table` and stay there; adding them to
    // `Decoded OK` would double-count them and move a figure the baseline
    // pins for a different reason. The two numbers answer different questions:
    // how much the static table covers, and how much this decoder recovered
    // from what the table does not.
    eprintln!("  Effect blobs:     {effect_blobs_decoded}");
}

/// Top-15 decode error breakdown. Always shown when there are any -- this is a
/// permanent diagnostic for schema-drift detection across game builds.
fn print_decode_errors(overlay: &OverlayStats, error_report: &OverlayErrorReport) {
    if overlay.decoded_err == 0 {
        return;
    }
    eprintln!();
    eprintln!(
        "=== Decode error report ({} distinct buckets, {} total) ===",
        error_report.bucket_count(),
        error_report.total_errors()
    );
    eprintln!(
        "  {:>7}  {:<6}  {:>5}  {:<20}  {:<30}  group_path",
        "count", "kind", "bits", "type", "field_name"
    );
    for row in &error_report.top_n(15) {
        // Truncate group_path for display (show last 60 chars).
        let gp_display = if row.group_path.len() > 60 {
            format!("...{}", &row.group_path[row.group_path.len() - 57..])
        } else {
            row.group_path.clone()
        };
        eprintln!(
            "  {:>7}  {:<6}  {:>5}  {:<20}  {:<30}  {}",
            row.count, row.error_kind, row.bit_count, row.declared_type, row.field_name, gp_display
        );
    }
}
