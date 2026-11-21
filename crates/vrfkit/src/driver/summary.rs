//! The export summary printed to stderr. `tools/summary_counters.py` lists
//! the counter lines the guard tools read, as whole-line patterns its test
//! checks against this file: change a line deliberately or not at all.

use std::fs;
use std::path::Path;
use std::time::Duration;

use vrf_decode::{OverlayErrorReport, OverlayStats};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;

use super::checkpoints::CheckpointStats;
use super::{CHECKPOINT_TABLES, MAIN_TABLES};
use crate::report;
use crate::sink::ExportStats;

/// Everything the run counted that is not in [`NetStats`]. The manifest reads
/// the same struct, so it and this summary cannot report different values.
#[derive(Default)]
pub(crate) struct RunTotals {
    pub chunks_processed: u32,
    /// Chunks of a type no reader knows, skipped whole.
    pub unknown_chunks: u64,
    /// DemoFrames walked in the ReplayData stream. Packets are counted inside
    /// the frame callback, so a frame that ends before its packet loop moves
    /// only this.
    pub frames: u32,
    /// ExternalData and GameSpecificFrameData bytes those frames stepped over:
    /// length-prefixed, so no other number moves when a build starts sending
    /// them.
    pub frame_skips: FrameSkips,
    /// Those frames whose time was NaN or infinite, read as 0 ms
    /// (`vrf_frame::FrameWalk::non_finite_times`); printed as `Frame times:`.
    pub non_finite_frame_times: u64,
    pub total_packets: u32,
    pub export_groups: usize,
    pub movement_rows: u64,
    pub net_guid_rows: usize,
    pub event_rows: u64,
    pub partial_rows: u64,
    pub partial_bits: u64,
    /// Payload bytes an Event chunk declared that its own header layout does
    /// not reach; zero across the corpus.
    pub event_trailing_bytes: u64,
    pub replay_data_trailing_bytes: u64,
    pub elapsed: Duration,
    /// Known Event payloads whose arity, tag, enum name, exact consumption or
    /// time relation failed, so no structural overlay was exported for them.
    /// Non-zero means an Event group changed shape.
    pub event_layout_mismatches: u64,
    /// The first such mismatch verbatim, so the summary can name the group.
    pub event_first_layout_mismatch: Option<String>,
    /// Known Event payloads whose arity, tag, enum-name and millisecond time
    /// all matched the measured structural layout.
    pub event_payloads_decoded: u64,
    /// Event groups outside the measured public vocabulary. Their raw payload
    /// remains preserved and no structural columns are populated.
    pub event_payload_unknown_groups: u64,
    pub sink: ExportStats,
    /// [`stale_checkpoint_note`] of the destination before publication.
    pub stale_checkpoint_note: Option<String>,
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
    eprintln!("  Unknown chunks:   {}", totals.unknown_chunks);
    eprintln!("  ReplayData frames: {}", totals.frames);
    eprintln!(
        "  Frame skips:      {}",
        report::frame_skips(&totals.frame_skips)
    );
    eprintln!(
        "  Frame times:      {} non-finite",
        totals.non_finite_frame_times
    );
    eprintln!("  Packets:          {}", totals.total_packets);
    eprintln!("  Export groups:    {}", totals.export_groups);
    eprintln!("  Content blocks:   {}", net_stats.content_blocks);
    eprintln!("  RepLayout blocks: {}", net_stats.rep_layout_blocks);
    eprintln!("  ClassNetCache:    {}", net_stats.class_net_cache_blocks);
    eprintln!("  Fields:           {}", net_stats.fields);
    eprintln!("  RPCs:             {}", net_stats.rpcs);
    eprintln!("  Actor opens:      {}", net_stats.actor_opens);
    eprintln!("  Actor closes:     {}", net_stats.actor_closes);
    eprintln!(
        "  Partial raw rows: {} ({} bits)",
        totals.partial_rows, totals.partial_bits
    );
    // `fields` is the fields.parquet row count (check_export_baseline.py);
    // the other four must equal `RPCs:`, `Actor opens:`, `Actor closes:` and
    // `Content blocks:`, which vrf-net counts beside each sink call
    // (tools/verify_build_corpus.py checks them via the manifest).
    eprintln!(
        "  Sink tally:       {} fields / {} RPCs / {} opens / {} closes / {} content blocks",
        totals.sink.fields_emitted,
        totals.sink.rpcs_emitted,
        totals.sink.actor_opens,
        totals.sink.actor_closes,
        totals.sink.content_blocks
    );
    eprintln!("  Bunches:          {}", net_stats.bunches);
    eprintln!("  Malformed pkts:   {}", net_stats.malformed_packets);
    eprintln!("  Partial bunches:  {}", report::partial_bunches(net_stats));
    eprintln!("  Partial causes:   {}", report::partial_causes(net_stats));
    eprintln!("  Bunch header fails: {}", net_stats.bunch_header_failures);
    eprintln!(
        "  Content failures: {} malformed / {} transform / {} field / {} RPC loss / {} unresolved RPC raw",
        net_stats.malformed_content_blocks,
        net_stats.transform_failures,
        net_stats.field_stream_failures,
        net_stats.rpc_payloads_lost(),
        net_stats.unresolved_rpc_payloads_preserved
    );
    // Blocks whose header or `content_bits` could not be read, a depth before
    // the four above; its own line because `Content failures`' format is parsed.
    eprintln!(
        "  Content framing fails: {}",
        net_stats.content_block_framing_failures
    );
    eprintln!("  Skipped bits:     {}", net_stats.skipped_bits);
    // Every line down to `RepLayout exports` is 0 on a healthy replay.
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
    // A failed open that took a live actor off its channel, and the bunches
    // dropped afterwards for want of an open channel.
    eprintln!(
        "  Failed reopens:   {}",
        net_stats.failed_reopens_while_open
    );
    eprintln!(
        "  Unopened channel: {} bunches / {} bits",
        net_stats.bunches_on_unopened_channel, net_stats.unopened_channel_bits
    );
    eprintln!(
        "  Resource limits:  {} channel / {} partial reassembly",
        net_stats.channel_state_limit_failures, net_stats.partial_resource_limit_failures
    );
    eprintln!(
        "  RepLayout exports: {}",
        net_stats.rep_layout_export_bunches
    );
    eprintln!(
        "  GUID mapping:     {} package maps / {} exported / {} required",
        net_stats.package_map_exports, net_stats.exported_guids, net_stats.must_be_mapped_guids
    );
    eprintln!(
        "  Diagnostics:      {} retained / {} dropped",
        net_stats.diagnostics.len(),
        net_stats.diagnostics_dropped
    );
    eprintln!(
        "  ReplayData unread: {} bytes",
        totals.replay_data_trailing_bytes
    );
    eprintln!("  Movement rows:    {}", totals.movement_rows);
    eprintln!("  NetGUID rows:     {}", totals.net_guid_rows);
    eprintln!("  Event rows:       {}", totals.event_rows);
    eprintln!(
        "  Event unread:     {} payload bytes",
        totals.event_trailing_bytes
    );
    // Zero included: a moved struct handle (RoundResults 93 -> 81 in 13.02)
    // shows only here.
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
    eprintln!(
        "  Movement components: {} attempted / {} complete / {} errors / {} empty",
        totals.sink.movement_component_streams,
        totals.sink.movement_component_complete,
        totals.sink.movement_component_errors,
        totals.sink.movement_component_empty
    );
    eprintln!(
        "  Component envelopes: {} validated / {} payload bytes / {} unused bits",
        totals.sink.movement_component_envelopes,
        totals.sink.movement_component_payload_bytes,
        totals.sink.movement_component_unused_bits
    );
    eprintln!(
        "  Component headers: {} input / {} discrepancy",
        totals.sink.movement_component_input_headers,
        totals.sink.movement_component_discrepancy_headers
    );
    // Non-zero errors or truncations: bits abandoned mid-element, leaves lost.
    eprintln!(
        "  Array decode:     {} elements / {} fields / {} errors / {} truncations",
        totals.sink.array.elements_decoded,
        totals.sink.array.fields_emitted,
        totals.sink.array.errors,
        totals.sink.array.truncations
    );
    eprintln!(
        "  Array residual:   {} root bits / {} nested bits / {} implicit ends",
        totals.sink.array.unconsumed_root_bits,
        totals.sink.array.unconsumed_nested_bits,
        totals.sink.array.implicit_terminations
    );
    eprintln!(
        "  Array leaf errs:  {}",
        totals.sink.array_leaf_decode_errors
    );
    eprintln!(
        "  Target locations: {} array children",
        totals.sink.targeting_world_locations_decoded
    );
    eprintln!(
        "  Route children:   {}",
        report::route_children(&totals.sink)
    );
    eprintln!(
        "  Reward opaque:    {} empty variants",
        totals.sink.tracked_rewards_opaque_empty_variants
    );
    // Its sibling tolerance: empty deltas whose trailing zero byte was spared.
    eprintln!(
        "  ActiveBlinds trailers: {} empty deltas",
        totals.sink.active_blinds_empty_trailers
    );
    eprintln!(
        "  FocusProjectiles trailers: {} empty deltas",
        totals.sink.focus_projectiles_empty_trailers
    );
    eprintln!(
        "  FocusProjectiles refused: {} parent updates with a mismatched declaration",
        totals.sink.focus_projectiles_declaration_refusals
    );
    eprintln!("  Truncated RPCs:   {}", totals.sink.truncated_rpcs);
    eprintln!("  RPC param walks:  {}", totals.sink.rpc_param_walks);
    eprintln!(
        "  RPC suffix bits:  {}",
        totals.sink.rpc_suffix_bits_dropped
    );
    eprintln!("  Event layout err: {}", totals.event_layout_mismatches);
    eprintln!(
        "  Event payloads:    {} decoded / {} unknown groups",
        totals.event_payloads_decoded, totals.event_payload_unknown_groups
    );
    if let Some(err) = &totals.event_first_layout_mismatch {
        eprintln!("  Event layout msg: {err}");
    }
    eprintln!(
        "  MultiContents items: {}",
        totals.sink.multi_contents_items_emitted
    );
    // The only counter that moves when the AbilitiesAndBuffs brute force
    // decodes anything.
    eprintln!("  CNC RPC rows:     {}", totals.sink.cnc_rpcs_emitted);
    // Its failure side: `unwalked` says the fc=34 walk was tried and failed,
    // `attempted` is its denominator.
    eprintln!(
        "  CNC brute force:  {} attempted / {} unwalked",
        totals.sink.cnc_bruteforce_payloads_attempted, totals.sink.cnc_bruteforce_payloads_unwalked
    );
    eprintln!(
        "  RepLayout tails:  {} decoded / {} preserved",
        totals.sink.rep_layout_cnc_tails_decoded, totals.sink.rep_layout_cnc_tails_preserved
    );
    eprintln!("  Elapsed:          {:.2?}", totals.elapsed);

    if let Some(cp) = checkpoints {
        print_checkpoints(cp);
    }

    print_file_sizes(
        out_path,
        checkpoints.is_some(),
        manifest_path,
        totals.stale_checkpoint_note.as_deref(),
    );
    print_overlay(overlay, totals.sink.effect_blobs_decoded);
    print_decode_errors(error_report);
}

fn print_checkpoints(cp: &CheckpointStats) {
    eprintln!();
    eprintln!("=== Checkpoints ===");
    eprintln!("  Checkpoints:      {}", cp.chunks);
    eprintln!(
        "  Checkpoint partial raw: {} rows / {} bits",
        cp.partial_rows, cp.partial_bits
    );
    eprintln!("  Trailing bytes:   {}", cp.trailing_bytes);
    eprintln!("  GUID entries:     {}", cp.guid_entries);
    eprintln!(
        "  GUID paths: {} literals / {} indices / {} resolved",
        cp.literal_paths, cp.indexed_paths, cp.resolved_path_indices
    );
    eprintln!("  Group records:    {}", cp.group_records);
    eprintln!("  Exported fields:  {}", cp.exported_fields);
    eprintln!("  Frames:           {}", cp.frames);
    eprintln!("  Frame packets:    {}", cp.packets);
    eprintln!(
        "  Checkpoint frame skips: {}",
        report::frame_skips(&cp.frame_skips)
    );
    eprintln!(
        "  Checkpoint frame times: {} non-finite",
        cp.non_finite_frame_times
    );
    eprintln!("  Checkpoint rows:  {}", cp.field_rows);
    eprintln!("  Checkpoint actors:{} rows", cp.actor_rows_written);
    eprintln!("  Checkpoint GUID rows: {}", cp.net_guid_rows_written);
    eprintln!("  Checkpoint blocks:{} rows", cp.block_rows_written);
    eprintln!(
        "  Checkpoint GUID entries: {} rows",
        cp.guid_entry_rows_written
    );
    eprintln!(
        "  Checkpoint export groups: {} rows",
        cp.export_group_rows_written
    );
    eprintln!(
        "  Checkpoint export fields: {} rows",
        cp.export_field_rows_written
    );
    eprintln!(
        "  Checkpoint net:   {} bunches / {} blocks / {} fields / {} RPCs",
        cp.net.bunches, cp.net.content_blocks, cp.net.fields, cp.net.rpcs
    );
    eprintln!("  Checkpoint partial:{}", report::partial_bunches(&cp.net));
    eprintln!("  Checkpoint causes: {}", report::partial_causes(&cp.net));
    eprintln!(
        "  Checkpoint loss:  {} malformed packets / {} bunch headers / {} malformed blocks / {} transform / {} field / {} RPC / {} unfinished partials ({} bits) / {} skipped bits",
        cp.net.malformed_packets,
        cp.net.bunch_header_failures,
        cp.net.malformed_content_blocks,
        cp.net.transform_failures,
        cp.net.field_stream_failures,
        cp.net.rpc_payloads_lost(),
        cp.net.unfinished_partials,
        cp.net.unfinished_partial_bits,
        cp.net.skipped_bits
    );
    eprintln!(
        "  Checkpoint framing fails: {}",
        cp.net.content_block_framing_failures
    );
    eprintln!(
        "  Checkpoint raw:   {} unresolved RPC payloads preserved whole",
        cp.net.unresolved_rpc_payloads_preserved
    );
    eprintln!(
        "  Checkpoint life:  {} deleted / {} opens / {} closes / {} live reopens / {} opens without spawn",
        cp.net.deleted_blocks,
        cp.net.actor_opens,
        cp.net.actor_closes,
        cp.net.channel_reopens_while_open,
        cp.net.actor_opens_missing_spawn
    );
    // Not appended to `Checkpoint life:`, whose format stays as its readers
    // expect.
    eprintln!(
        "  Checkpoint unopened: {} bunches / {} bits / {} failed reopens",
        cp.net.bunches_on_unopened_channel,
        cp.net.unopened_channel_bits,
        cp.net.failed_reopens_while_open
    );
    eprintln!(
        "  Checkpoint limits: {} channel / {} partial reassembly",
        cp.net.channel_state_limit_failures, cp.net.partial_resource_limit_failures
    );
    eprintln!(
        "  Checkpoint GUIDs: {} package maps / {} RepLayout exports / {} exported / {} required",
        cp.net.package_map_exports,
        cp.net.rep_layout_export_bunches,
        cp.net.exported_guids,
        cp.net.must_be_mapped_guids
    );
    eprintln!(
        "  Checkpoint diag:  {} retained / {} dropped",
        cp.net.diagnostics.len(),
        cp.net.diagnostics_dropped
    );
    // Movement is a snapshot sample, not timeline data, and is not written;
    // actor rows have a table, so their dropped count should stay 0.
    eprintln!(
        "  Dropped:          {} actor / {} movement rows (checkpoint snapshot)",
        cp.actor_rows_dropped, cp.movement_rows_dropped
    );
    eprintln!(
        "  Overlay:          {} decoded / {} errors / {} raw-skip / {} not-in-table / {} unnamed / {} conflicts / {} effect blobs",
        cp.sink.overlay.decoded_ok,
        cp.sink.overlay.decoded_err,
        cp.sink.overlay.raw_or_skip,
        cp.sink.overlay.not_in_table,
        cp.sink.overlay.no_field_name,
        cp.sink.overlay.handle_conflicts_refused,
        cp.sink.effect_blobs_decoded
    );
    // Not `Struct blobs`: a label the main block uses would match two lines.
    eprintln!(
        "  Checkpoint blobs: {} decoded / {} failed",
        cp.sink.struct_blobs_decoded, cp.sink.struct_blobs_failed
    );
    // `Sink tally`'s twin, checked the same way against checkpoint_fields.parquet,
    // `Checkpoint net` and `Checkpoint life`.
    eprintln!(
        "  Checkpoint sink:  {} fields / {} RPCs / {} opens / {} closes / {} content blocks",
        cp.sink.fields_emitted,
        cp.sink.rpcs_emitted,
        cp.sink.actor_opens,
        cp.sink.actor_closes,
        cp.sink.content_blocks
    );
    if let Some(error) = &cp.sink.struct_blob_first_error {
        eprintln!("  Checkpoint blob error: {error}");
    }
    eprintln!(
        "  Checkpoint fails: {} array / {} truncated RPC / {} movement",
        cp.sink.array.errors, cp.sink.truncated_rpcs, cp.sink.movement_rpc_errors
    );
    eprintln!(
        "  Checkpoint array: {} elements / {} fields / {} truncations / {} root bits / {} nested bits / {} implicit ends",
        cp.sink.array.elements_decoded,
        cp.sink.array.fields_emitted,
        cp.sink.array.truncations,
        cp.sink.array.unconsumed_root_bits,
        cp.sink.array.unconsumed_nested_bits,
        cp.sink.array.implicit_terminations
    );
    eprintln!(
        "  Checkpoint leaf:  {} typed decode errors",
        cp.sink.array_leaf_decode_errors
    );
    eprintln!(
        "  Checkpoint targets: {} array children",
        cp.sink.targeting_world_locations_decoded
    );
    eprintln!(
        "  Checkpoint route children: {}",
        report::route_children(&cp.sink)
    );
    eprintln!("  Checkpoint RPC walks: {}", cp.sink.rpc_param_walks);
    eprintln!(
        "  Checkpoint reward opaque: {} empty variants",
        cp.sink.tracked_rewards_opaque_empty_variants
    );
    eprintln!(
        "  Checkpoint ActiveBlinds trailers: {} empty deltas",
        cp.sink.active_blinds_empty_trailers
    );
    eprintln!(
        "  Checkpoint FocusProjectiles trailers: {} empty deltas",
        cp.sink.focus_projectiles_empty_trailers
    );
    eprintln!(
        "  Checkpoint FocusProjectiles refused: {} parent updates with a mismatched declaration",
        cp.sink.focus_projectiles_declaration_refusals
    );
    eprintln!(
        "  Checkpoint movement: {} failures",
        cp.sink.movement_rpc_errors
    );
    if let Some(error) = &cp.sink.movement_first_error {
        eprintln!("  Checkpoint movement error: {error}");
    }
    eprintln!(
        "  Checkpoint movement components: {} attempted / {} complete / {} errors / {} empty",
        cp.sink.movement_component_streams,
        cp.sink.movement_component_complete,
        cp.sink.movement_component_errors,
        cp.sink.movement_component_empty
    );
    eprintln!(
        "  Checkpoint component envelopes: {} validated / {} payload bytes / {} unused bits",
        cp.sink.movement_component_envelopes,
        cp.sink.movement_component_payload_bytes,
        cp.sink.movement_component_unused_bits
    );
    eprintln!(
        "  Checkpoint component headers: {} input / {} discrepancy",
        cp.sink.movement_component_input_headers, cp.sink.movement_component_discrepancy_headers
    );
    eprintln!(
        "  Checkpoint suffix:{} RPC bits",
        cp.sink.rpc_suffix_bits_dropped
    );
    eprintln!(
        "  Checkpoint MultiContents: {} items",
        cp.sink.multi_contents_items_emitted
    );
    eprintln!("  Checkpoint CNC:   {} RPC rows", cp.sink.cnc_rpcs_emitted);
    eprintln!(
        "  Checkpoint CNC brute force: {} attempted / {} unwalked",
        cp.sink.cnc_bruteforce_payloads_attempted, cp.sink.cnc_bruteforce_payloads_unwalked
    );
    eprintln!(
        "  Checkpoint tails: {} decoded / {} preserved",
        cp.sink.rep_layout_cnc_tails_decoded, cp.sink.rep_layout_cnc_tails_preserved
    );
}

/// A warning when this run, without `--checkpoints`, drops the
/// [`CHECKPOINT_TABLES`] an earlier export left at `out_path`: publication
/// replaces the whole directory, so they are gone, not merged or left behind.
/// Call it on the destination before `OutputTransaction::publish`; afterwards
/// the directory holds only what this run wrote.
pub(super) fn stale_checkpoint_note(out_path: &Path, with_checkpoints: bool) -> Option<String> {
    if with_checkpoints {
        return None;
    }
    let paths: Vec<_> = CHECKPOINT_TABLES
        .iter()
        .map(|name| out_path.join(name))
        .filter(|path| path.exists())
        .collect();
    (!paths.is_empty()).then(|| {
        format!(
            "{} from a previous export to this destination are being dropped: this run has no \
             --checkpoints, and publishing replaces the whole destination directory rather than \
             merging into it",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

fn print_file_sizes(
    out_path: &Path,
    with_checkpoints: bool,
    manifest_path: &Path,
    stale_checkpoint_note: Option<&str>,
) {
    let size = |name: &str| file_size(&out_path.join(name));

    eprintln!();
    for table in MAIN_TABLES {
        eprintln!("  {:<18}{} bytes", format!("{table}:"), size(table));
    }
    if with_checkpoints {
        for table in CHECKPOINT_TABLES {
            eprintln!("  {table}: {} bytes", size(table));
        }
    }
    eprintln!("  manifest.json:    {}", manifest_path.display());
    if let Some(note) = stale_checkpoint_note {
        eprintln!("  DROPPED TABLE:    {note}");
    }
}

/// A file's size in bytes, or `?` when it cannot be read: a missing table
/// must not print as a plausible empty one.
fn file_size(path: &Path) -> String {
    fs::metadata(path).map_or_else(|_| "?".to_owned(), |m| m.len().to_string())
}

/// The typed ratio's denominator is every row offered: replicated properties
/// and RPC parameters, whose type coverage differs widely, so the label names
/// it rather than reading as "of all fields".
fn print_overlay(overlay: &OverlayStats, effect_blobs_decoded: u64) {
    let total = overlay.decoded_ok
        + overlay.decoded_err
        + overlay.raw_or_skip
        + overlay.not_in_table
        + overlay.no_field_name;
    eprintln!();
    eprintln!("=== Type overlay ===");
    eprintln!("  Decoded OK:       {}", overlay.decoded_ok);
    eprintln!("  Decode errors:    {}", overlay.decoded_err);
    eprintln!("  Raw/Skip:         {}", overlay.raw_or_skip);
    eprintln!("  Not in table:     {}", overlay.not_in_table);
    eprintln!("  No field name:    {}", overlay.no_field_name);
    eprintln!("  Rows offered:     {total}");
    eprintln!(
        "  Typed:            {} (properties + RPC parameters)",
        typed_share(overlay.decoded_ok, total)
    );
    // Rows the handle fallback would have typed, refused because the replay
    // declared a different, non-numeric name at that handle. They land in
    // `Not in table`; only this line tells the two apart.
    eprintln!(
        "  Handle conflicts: {} refused",
        overlay.handle_conflicts_refused
    );
    // Outside the ratio: the buckets are decided before the effect pass, so
    // these rows already count as `Not in table`; adding them to `Decoded OK`
    // would double-count them and move a figure the baseline pins.
    eprintln!("  Effect blobs:     {effect_blobs_decoded}");
}

/// `decoded_ok` as a share of the `total` rows offered, or `?` when none
/// were: no share at all, not a typed share of zero.
fn typed_share(decoded_ok: u64, total: u64) -> String {
    if total == 0 {
        return "?".to_owned();
    }
    format!("{:.1}%", (decoded_ok as f64 / total as f64) * 100.0)
}

/// Top-15 decode error breakdown, shown whenever there are any: the permanent
/// schema-drift diagnostic across game builds.
fn print_decode_errors(error_report: &OverlayErrorReport) {
    // Gated on the report, which merges ReplayData and checkpoints, not on
    // `overlay.decoded_err`, which is ReplayData alone.
    if error_report.total_errors() == 0 {
        return;
    }
    eprintln!();
    eprintln!(
        "=== Decode error report ({} distinct buckets, {} total) ===",
        error_report.bucket_count(),
        error_report.total_errors()
    );
    // The kind column is as wide as the longest label, `Malformed`.
    eprintln!(
        "  {:>7}  {:<9}  {:>5}  {:<20}  {:<30}  group_path",
        "count", "kind", "bits", "type", "field_name"
    );
    for row in &error_report.top_n(15) {
        let gp_display = display_tail(&row.group_path, 60);
        eprintln!(
            "  {:>7}  {:<9}  {:>5}  {:<20}  {:<30}  {}",
            row.count, row.error_kind, row.bit_count, row.declared_type, row.field_name, gp_display
        );
    }
}

/// The last `width - 3` chars after an ellipsis. Chars, not bytes: byte
/// slicing panicked when the cut fell inside a multi-byte character.
fn display_tail(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_owned();
    }
    let tail = width.saturating_sub(3);
    let skip = count.saturating_sub(tail);
    format!("...{}", value.chars().skip(skip).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::{CHECKPOINT_TABLES, display_tail, file_size, stale_checkpoint_note, typed_share};
    use std::fs;

    #[test]
    fn native_component_counters_reach_main_and_checkpoint_summaries() {
        // Given: zero controls, then different accumulated values in each pass.
        let mut totals = super::RunTotals::default();
        let mut cp = super::CheckpointStats::default();
        for extra in [false, true] {
            if extra {
                let result = vrf_movement::RpcDecodeResult {
                    component_streams: 1,
                    component_complete: 1,
                    component_envelopes: 1,
                    component_payload_bytes: 32,
                    component_unused_bits: 3,
                    component_input_headers: 1,
                    component_discrepancy_headers: 1,
                    ..Default::default()
                };
                totals.sink.record_movement_decode(Ok(&result));
                cp.sink.record_movement_decode(Ok(&result));
                cp.sink.record_movement_decode(Ok(&result));
            }
            // When: the real summary functions consume the same sink counters
            // that the manifest enumerates. Retained --nocapture output is
            // independently checked by the movement audit parser.
            let path = std::path::Path::new(".");
            super::print(
                path,
                &vrf_net::stats::NetStats::default(),
                &totals,
                &vrf_decode::OverlayErrorReport::default(),
                Some(&cp),
                path,
            );
            // Then: each pass retains its own aggregate; no movement loss.
            assert_eq!(
                totals.sink.movement_component_payload_bytes,
                u64::from(extra) * 32
            );
            assert_eq!(
                cp.sink.movement_component_payload_bytes,
                u64::from(extra) * 64
            );
            assert_eq!(totals.sink.movement_rpc_errors, 0);
            assert_eq!(cp.sink.movement_rpc_errors, 0);
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vrfkit_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_leftover_checkpoint_table_is_named_when_this_run_did_not_write_one() {
        let dir = temp_dir("stale_cp");
        assert_eq!(
            stale_checkpoint_note(&dir, false),
            None,
            "nothing to warn about in a clean directory"
        );

        fs::write(dir.join(CHECKPOINT_TABLES[0]), b"not really parquet").expect("write");
        let note = stale_checkpoint_note(&dir, false).expect("the leftover must be reported");
        assert!(
            note.contains(CHECKPOINT_TABLES[0]),
            "the warning must name the file: {note}"
        );

        // With the flag, the file is this run's own output and says nothing.
        assert_eq!(stale_checkpoint_note(&dir, true), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Two exports to one destination are supported, and the other's
    /// publication can swap the directory before this run's summary.
    #[test]
    fn an_unreadable_file_size_is_a_visible_absence() {
        let dir = temp_dir("sizes");
        fs::write(dir.join("five.parquet"), b"12345").expect("write");
        assert_eq!(file_size(&dir.join("five.parquet")), "5");
        assert_eq!(file_size(&dir.join("missing.parquet")), "?");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_typed_share_of_no_rows_is_unknown_not_zero() {
        assert_eq!(typed_share(0, 0), "?");
        assert_eq!(typed_share(1, 8), "12.5%");
        assert_eq!(typed_share(0, 5), "0.0%");
    }

    #[test]
    fn diagnostic_path_truncation_never_slices_inside_unicode() {
        let path = format!("{}{}", "a".repeat(59), "\u{d55c}\u{ad6d}");
        let displayed = display_tail(&path, 60);
        assert!(displayed.starts_with("..."));
        assert!(displayed.ends_with("\u{d55c}\u{ad6d}"));
        assert_eq!(displayed.chars().count(), 60);
    }
}
