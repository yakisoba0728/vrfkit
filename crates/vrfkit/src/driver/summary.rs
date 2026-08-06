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
use super::totals::SinkTotals;

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
    pub replay_data_trailing_bytes: u64,
    pub elapsed: Duration,
    /// Event payloads whose declared word count did not fit the payload, so no
    /// typed words were exported for them. Printed only when non-zero; a
    /// non-zero value means an Event group changed shape.
    pub event_layout_mismatches: u64,
    /// The first such mismatch verbatim, so the summary can name the group.
    pub event_first_layout_mismatch: Option<String>,
    /// Everything the per-packet sinks counted. One struct rather than a dozen
    /// loose fields, because the failure this guards against is a counter that
    /// exists on `ExportStats` and reaches no summary line. See
    /// [`super::totals`].
    pub sink: SinkTotals,
}

/// Print the whole `=== Export complete ===` report.
pub(super) fn print(
    out_path: &Path,
    net_stats: &NetStats,
    totals: &RunTotals,
    error_report: &OverlayErrorReport,
    checkpoints: Option<&CheckpointStats>,
    manifest_path: &Path,
) {
    let overlay = &totals.sink.overlay;
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
    eprintln!("  Bunch header fails: {}", net_stats.bunch_header_failures);
    eprintln!("  Skipped bits:     {}", net_stats.skipped_bits);
    // Unconditional, zeros included, for the reason spelled out on the struct
    // blob line below: a line that only appears when non-zero cannot tell
    // "nothing was lost" apart from "the code that counts stopped running".
    // These five all read 0 on a healthy replay, which is exactly why a 0 that
    // is present is worth more than a line that is absent.
    eprintln!(
        "  Unfinished partials: {} ({} bits)",
        net_stats.unfinished_partials, net_stats.unfinished_partial_bits
    );
    eprintln!(
        "  Channel reopens:  {}",
        net_stats.channel_reopens_while_open
    );
    eprintln!(
        "  Opens w/o spawn:  {}",
        net_stats.actor_opens_missing_spawn
    );
    eprintln!(
        "  RepLayout exports: {}",
        net_stats.rep_layout_export_bunches
    );
    eprintln!(
        "  ReplayData unread: {} bytes",
        totals.replay_data_trailing_bytes
    );
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
        totals.sink.struct_blobs_decoded, totals.sink.struct_blobs_failed
    );
    if let Some(err) = &totals.sink.struct_blob_first_error {
        eprintln!("  Struct blob err:  {err}");
    }
    eprintln!("  Movement errors:  {}", totals.sink.movement_rpc_errors);
    if let Some(err) = &totals.sink.movement_first_error {
        eprintln!("  Movement err:     {err}");
    }
    // Printed only when non-zero: valid replays produce zero and the line would
    // otherwise be noise on the pinned summary. A non-zero value means the
    // array walker abandoned bits mid-element and flattened leaves were lost.
    if totals.sink.array_decode_errors > 0 {
        eprintln!("  Array decode err: {}", totals.sink.array_decode_errors);
    }
    if totals.sink.truncated_rpcs > 0 {
        eprintln!("  Truncated RPCs:   {}", totals.sink.truncated_rpcs);
    }
    if totals.sink.rpc_suffix_bits_dropped > 0 {
        eprintln!(
            "  RPC suffix bits:  {}",
            totals.sink.rpc_suffix_bits_dropped
        );
    }
    if totals.event_layout_mismatches > 0 {
        eprintln!("  Event layout err: {}", totals.event_layout_mismatches);
    }
    if let Some(err) = &totals.event_first_layout_mismatch {
        eprintln!("  Event layout msg: {err}");
    }
    eprintln!(
        "  MultiContents items: {}",
        totals.sink.multi_contents_items_emitted
    );
    // Printed unconditionally, including the zero, for the same reason
    // `Struct blobs` is: this is the only counter that moves when the
    // AbilitiesAndBuffs brute-force decodes anything, so a build that stopped
    // reaching it would otherwise leave every line on this summary unchanged.
    eprintln!("  CNC RPC rows:     {}", totals.sink.cnc_rpcs_emitted);
    eprintln!("  Elapsed:          {:.2?}", totals.elapsed);

    if let Some(cp) = checkpoints {
        print_checkpoints(cp);
    }

    print_file_sizes(out_path, checkpoints.is_some(), manifest_path);
    print_overlay(overlay, totals.sink.effect_blobs_decoded);
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
        cp.sink.overlay.decoded_ok,
        cp.sink.overlay.decoded_err,
        cp.sink.overlay.raw_or_skip,
        cp.sink.overlay.not_in_table,
        cp.sink.overlay.no_field_name,
        cp.sink.effect_blobs_decoded
    );
    // NOT "Struct blobs", which the main block already uses: every label here
    // is a regex anchor for check_export_baseline.py, and two blocks sharing
    // one label would leave the harness matching whichever came first.
    eprintln!(
        "  Checkpoint blobs: {} decoded / {} failed",
        cp.sink.struct_blobs_decoded, cp.sink.struct_blobs_failed
    );
    // The three failure counters the checkpoint pass used to drop on the floor.
    // Printed unconditionally, zero included: a conditional line here could not
    // tell "the checkpoint decoders ran clean" from "the checkpoint decoders
    // were never reached", which is the whole reason they are counted.
    eprintln!(
        "  Checkpoint fails: {} array / {} truncated RPC / {} movement",
        cp.sink.array_decode_errors, cp.sink.truncated_rpcs, cp.sink.movement_rpc_errors
    );
    if cp.sink.rpc_suffix_bits_dropped > 0 {
        eprintln!(
            "  Checkpoint suffix:{} RPC bits",
            cp.sink.rpc_suffix_bits_dropped
        );
    }
    eprintln!("  Checkpoint CNC:   {} RPC rows", cp.sink.cnc_rpcs_emitted);
}

/// The one table that is written only when `--checkpoints` is given.
const CHECKPOINT_TABLE: &str = "checkpoint_fields.parquet";

/// A warning line when this run left a checkpoint table it did not write.
///
/// The five main tables and the manifest are recreated on every export, but
/// [`CHECKPOINT_TABLE`] is only opened when the flag asks for it. Export replay
/// A with checkpoints and replay B without, into the same directory, and six
/// files then describe B while a seventh valid-looking Parquet still describes
/// A -- exit 0, nothing said, and the next consumer joins the two.
///
/// Deleting it is deliberately not done here: the command was asked to write
/// output, not to remove files it does not own, and a silent delete of
/// somebody's data is a worse failure than a stale file. Naming it is enough
/// to stop it being read by accident.
fn stale_checkpoint_note(out_path: &Path, with_checkpoints: bool) -> Option<String> {
    if with_checkpoints {
        return None;
    }
    let path = out_path.join(CHECKPOINT_TABLE);
    path.exists().then(|| {
        format!(
            "{} is left over from an earlier run (this export had no --checkpoints) and does NOT describe this replay",
            path.display()
        )
    })
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
        eprintln!("  {CHECKPOINT_TABLE}: {} bytes", size(CHECKPOINT_TABLE));
    }
    eprintln!("  manifest.json:    {}", manifest_path.display());
    if let Some(note) = stale_checkpoint_note(out_path, with_checkpoints) {
        eprintln!("  STALE FILE:       {note}");
    }
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
    // Unconditional, zero included. These are rows the handle fallback WOULD
    // have typed, refused because the replay declared a different, non-numeric
    // name at that handle -- the stale-mapping case that used to read a float
    // as an int and report `decoded_ok`. They land in `Not in table` instead,
    // so without this line the refusal is indistinguishable from the field
    // never having been in the table at all, and a rule that started refusing
    // everything would look exactly like a quiet build.
    eprintln!(
        "  Handle conflicts: {} refused",
        overlay.handle_conflicts_refused
    );
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

#[cfg(test)]
mod tests {
    use super::{CHECKPOINT_TABLE, stale_checkpoint_note};
    use std::fs;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vrfkit_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// An export without `--checkpoints` must not leave a checkpoint table from
    /// a different replay sitting silently in the output directory.
    ///
    /// The five main tables and the manifest are recreated on every run, but
    /// `checkpoint_fields.parquet` is only ever *opened* when the flag is
    /// given. Export replay A with checkpoints and replay B without into the
    /// same directory and the directory then describes two different matches:
    /// six files about B, one valid-looking Parquet about A, exit 0, and
    /// nothing said. Deleting it is not this command's business -- it was asked
    /// to write, not to clean up -- but staying quiet about it is how the wrong
    /// file gets read.
    #[test]
    fn a_leftover_checkpoint_table_is_named_when_this_run_did_not_write_one() {
        let dir = temp_dir("stale_cp");
        assert_eq!(
            stale_checkpoint_note(&dir, false),
            None,
            "nothing to warn about in a clean directory"
        );

        fs::write(dir.join(CHECKPOINT_TABLE), b"not really parquet").expect("write");
        let note = stale_checkpoint_note(&dir, false).expect("the leftover must be reported");
        assert!(
            note.contains(CHECKPOINT_TABLE),
            "the warning must name the file: {note}"
        );

        // With the flag, the file is this run's own output and says nothing.
        assert_eq!(stale_checkpoint_note(&dir, true), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
