//! `validate` subcommand -- RepLayout grammar oracle.
//!
//! The scored scope is framed content blocks in the **ReplayData** stream;
//! Checkpoint chunks are counted, skipped and declared under `NOT COVERED`
//! ([`checkpoint_scope_note`]). A decoded RepLayout property prefix satisfies
//! this grammar, and a valid zero terminator may be followed by a separate
//! ClassNetCache tail in the block:
//!
//! ```text
//! checksum_bit : 1 bit
//! loop:
//!   handle = IntPacked   (0 -> end)
//!   payload_bits = IntPacked
//!   consume payload_bits
//! property prefix + decoded or preserved tail == declared bit_count
//! ```
//!
//! A wrong transform makes `IntPacked` return nonsense or leaves the consumed
//! bits short of the declared size. This is evidence about blocks that reached
//! framing, not a whole-file losslessness proof. [`verdict_from_stats`] lists
//! what fails the verdict; an unresolved ClassNetCache block whose whole
//! decoded payload was kept is reported apart and is not loss.

use std::path::Path;
use std::time::Instant;

use vrf_net::stats::{DiagnosticEvent, NetStats, SkipReason};

use crate::error::CliError;
use crate::pass::{Chunk, Pass, Replay};
use crate::reader_input::FileInput;
use crate::report;
use crate::sink::ExportStats;

/// What a validation run concluded, and the exit code it earns.
///
/// The two failure outcomes stay apart: "found problems" and "had nothing to
/// look at" are different answers, and a corpus sweep that merged them could
/// not tell a broken build from a file carrying no replication stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// Content blocks were found and every one of them framed.
    Passed,
    /// At least one failure covered by ReplayData block validation.
    ValidationFailed,
    /// No RepLayout or ClassNetCache blocks at all -- nothing was validated.
    NoContentBlocks,
}

impl Verdict {
    /// Absence of evidence outranks: a file with no content blocks validated
    /// nothing, whatever its other counters say.
    #[must_use]
    pub fn decide(total_with_content: u64, validation_failures: u64) -> Self {
        if total_with_content == 0 {
            Self::NoContentBlocks
        } else if validation_failures > 0 {
            Self::ValidationFailed
        } else {
            Self::Passed
        }
    }

    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::ValidationFailed => 1,
            Self::NoContentBlocks => 2,
        }
    }
}

/// Decide from the hard-failure counters inside the scored validation scope.
///
/// Not terms: `partial_errors` (rejected before a complete bunch reaches
/// framing, so unscored), `unopened_channel_bits` (moves only with
/// `bunches_on_unopened_channel`) and a failed reopen (it fails the bunch
/// header, or retires its actor so the bunches after it count as unopened).
/// `unfinished_partials` is loss: bytes present at EOF and abandoned. The
/// depth sum is `NetStats::lost_content_blocks`'s alone, so this verdict and
/// `quality.content_blocks_lost` cannot drift. `bunches_on_unopened_channel`
/// is 0 on 45 replays of 24 builds, main and checkpoint passes.
/// `package_map_exports` (a superset of `rep_layout_export_bunches`) fails the
/// run: the bunch content after the exports is never read. 0 on 1,018 replays.
pub(crate) fn verdict_from_stats(stats: &NetStats, replay_data_trailing_bytes: u64) -> Verdict {
    let total_with_content = stats.rep_layout_blocks + stats.class_net_cache_blocks;
    let failures = stats.malformed_packets
        + stats.unfinished_partials
        + stats.channel_state_limit_failures
        + stats.partial_resource_limit_failures
        + stats.bunch_header_failures
        + stats.bunches_on_unopened_channel
        + stats.package_map_exports
        + stats.lost_content_blocks()
        + u64::from(replay_data_trailing_bytes != 0);
    Verdict::decide(total_with_content, failures)
}

/// Run the validate oracle; `diagnostics` prints every retained event in full.
/// A file that cannot be read is an error, not a verdict.
pub fn run(path: &str, diagnostics: bool) -> Result<Verdict, CliError> {
    let start = Instant::now();

    eprintln!("reading {path}...");
    let (preamble, input) = FileInput::open(Path::new(path))?;
    let replay = Replay::new(&preamble);
    let branch = replay.branch;

    eprintln!("branch: {branch}");
    eprintln!("validating RepLayout grammar on framed ReplayData content blocks...");

    let mut pass = Pass::new(&replay)?;
    // Counted as every pass does, never printed.
    let mut sink = ExportStats::default();
    // Counted, not merely skipped: see `checkpoint_scope_note`.
    let mut checkpoint_chunks: u64 = 0;
    let mut replay_data_trailing_bytes = 0u64;
    let unknown_chunks = input.for_each_chunk(&replay, &mut |chunk| {
        match chunk {
            Chunk::Checkpoint(_) => checkpoint_chunks += 1,
            Chunk::ReplayData(frames, unread) => {
                replay_data_trailing_bytes += unread as u64;
                // Never drained: each packet's sink clears them.
                pass.walk(&frames, &mut sink, |_| Ok(()))?;
            }
            Chunk::Event(_) => {}
        }
        Ok(())
    })?;

    pass.finish();
    let stats = pass.reader.stats();
    let elapsed = start.elapsed();

    let total_content = stats.content_blocks;
    let rep_layout = stats.rep_layout_blocks;
    let class_net = stats.class_net_cache_blocks;
    let malformed = stats.malformed_content_blocks;
    let deleted = stats.deleted_blocks;
    let rpc_payloads_lost = report::count_or_unknown(stats.rpc_payloads_lost());
    let failed = stats.lost_content_blocks();

    println!();
    println!("=== Validation Oracle ===");
    println!("  Branch:               {branch}");
    println!("  Total content blocks: {total_content}");
    println!("    RepLayout:          {rep_layout}");
    println!("    ClassNetCache:      {class_net}");
    println!("    Deleted:            {deleted}");
    println!("    Malformed packets:  {}", stats.malformed_packets);
    println!("    Partial bunches:    {}", report::partial_bunches(stats));
    println!("    Partial causes:     {}", report::partial_causes(stats));
    println!("    Bunch header failed:{}", stats.bunch_header_failures);
    println!(
        "    Failed reopens:     {}",
        stats.failed_reopens_while_open
    );
    println!(
        "    Unopened channel:   {} bunches / {} bits",
        stats.bunches_on_unopened_channel, stats.unopened_channel_bits
    );
    println!(
        "    Package map exports: {} ({} with RepLayout export)",
        stats.package_map_exports, stats.rep_layout_export_bunches
    );
    println!("    Malformed framing:  {malformed}");
    println!(
        "    Content framing fails: {}",
        stats.content_block_framing_failures
    );
    println!("    Transform failed:   {}", stats.transform_failures);
    println!("    Field stream failed:{}", stats.field_stream_failures);
    println!("    RPC payload lost:   {rpc_payloads_lost}");
    println!(
        "    RPC unresolved/raw:{}",
        stats.unresolved_rpc_payloads_preserved
    );
    println!("  Fields emitted:       {}", stats.fields);
    println!("  RPCs emitted:         {}", stats.rpcs);
    println!("  Skipped bits:         {}", stats.skipped_bits);
    println!(
        "  Unfinished partials:  {} ({} bits)",
        stats.unfinished_partials, stats.unfinished_partial_bits
    );
    println!(
        "  State resource limits: {} channel / {} partial reassembly",
        stats.channel_state_limit_failures, stats.partial_resource_limit_failures
    );
    println!(
        "  ReplayData unread:    {} bytes",
        replay_data_trailing_bytes
    );
    println!("  ReplayData frames:    {}", pass.frames);
    println!("  Unknown chunks:       {unknown_chunks}");
    println!(
        "  Frame skips:          {}",
        report::frame_skips(&pass.frame_skips)
    );
    println!(
        "  Frame times:          {} non-finite",
        pass.non_finite_frame_times
    );
    println!("  Packets:              {}", stats.packets);
    println!("  Bunches:              {}", stats.bunches);
    println!("  Actor opens:          {}", stats.actor_opens);
    println!("  Actor closes:         {}", stats.actor_closes);
    if let Some(note) = checkpoint_scope_note(checkpoint_chunks) {
        println!("  NOT COVERED:          {note}");
    }
    if let Some(note) = partial_reassembly_scope_note(stats.partial_errors) {
        println!("  NOT COVERED:          {note}");
    }
    println!();

    // The rate is over classified blocks (RepLayout + ClassNetCache).
    let total_with_content = rep_layout + class_net;
    let verdict = verdict_from_stats(stats, replay_data_trailing_bytes);
    if total_with_content == 0 {
        println!("  No content blocks found - cannot validate.");
    } else {
        println!("{}", pass_rate_line(total_with_content, failed));
    }

    // The counters above say how many payload-stage failures; these say which.
    // Both headers print at zero: a section that appears only when non-empty
    // cannot tell "nothing failed" from "this report stopped running".
    let stream_failures = pass.channels.stream_failures();
    println!();
    println!("=== Stream failures ({} shown) ===", stream_failures.len());
    for line in stream_failures {
        println!("  {line}");
    }

    println!();
    // Capped (uncapped, ~100 MB on a wrong transform), so past the cap
    // `len()` alone under-reports.
    if stats.diagnostics_dropped == 0 {
        println!(
            "=== Diagnostic Events ({} total) ===",
            stats.diagnostics.len()
        );
    } else {
        println!(
            "=== Diagnostic Events ({} total, {} shown, {} dropped at the cap) ===",
            stats.diagnostics.len() + stats.diagnostics_dropped as usize,
            stats.diagnostics.len(),
            stats.diagnostics_dropped
        );
    }

    print_skip_breakdown(&stats.diagnostics);

    if diagnostics {
        println!();
        for (i, event) in stats.diagnostics.iter().enumerate() {
            print_diagnostic_event(i, event);
        }
    } else if !stats.diagnostics.is_empty() {
        println!();
        println!("  (use --diagnostics to see full event dumps)");
    }

    println!();
    println!("  Elapsed: {:.2?}", elapsed);
    // The exit code in words, so a terminal and `$?` cannot disagree.
    println!("  VERDICT: {}", verdict_line(verdict));

    Ok(verdict)
}

/// The `NOT COVERED` line for the Checkpoint chunks this oracle skips, stated
/// rather than implied: walking them would move every counter `validate`'s
/// pinned baselines hold, and `export --checkpoints` decodes them.
fn checkpoint_scope_note(checkpoint_chunks: u64) -> Option<String> {
    (checkpoint_chunks > 0).then(|| {
        format!(
            "{checkpoint_chunks} Checkpoint chunk(s) were NOT walked - this verdict covers the ReplayData stream only (use `export --checkpoints` to decode them)"
        )
    })
}

fn partial_reassembly_scope_note(partial_errors: u64) -> Option<String> {
    (partial_errors > 0).then(|| {
        let rejection = if partial_errors == 1 {
            "rejection was"
        } else {
            "rejections were"
        };
        format!(
            "{partial_errors} partial reassembly {rejection} discarded before content-block framing - excluded from the block score and verdict"
        )
    })
}

/// The `ORACLE PASS RATE` line for `failed` of `total_with_content` blocks.
/// `failed` can exceed the classified total (a block whose header could not be
/// read is lost unclassified), so `passed` and the rate saturate at 0; the
/// rate stays `1 - failed / total`, whose last bit can differ from
/// `passed / total`.
fn pass_rate_line(total_with_content: u64, failed: u64) -> String {
    let passed = total_with_content.saturating_sub(failed);
    let pass_rate = 1.0 - (failed.min(total_with_content) as f64 / total_with_content as f64);
    format!(
        "  ORACLE PASS RATE:     {:.6}% ({} / {} blocks passed)",
        pass_rate * 100.0,
        passed,
        total_with_content
    )
}

/// The one-line conclusion printed under `VERDICT:`.
fn verdict_line(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Passed => "PASS - ReplayData block validation passed (exit 0)",
        Verdict::ValidationFailed => "FAIL - ReplayData validation found loss (exit 1)",
        Verdict::NoContentBlocks => "CANNOT VALIDATE - no content blocks found (exit 2)",
    }
}

/// Print a breakdown of where skipped bits come from, zeros included, so a
/// category that stops being recorded stays visible. `SkipReason` is
/// `#[non_exhaustive]`: a reason this match has no case for is tallied as
/// `Unclassified`, never dropped from the breakdown.
fn print_skip_breakdown(events: &[DiagnosticEvent]) {
    let mut tally = [(0u32, 0u64); 5];
    for ev in events {
        let row = &mut tally[skip_reason_row(&ev.reason)];
        row.0 += 1;
        row.1 += ev.bits_skipped;
    }
    println!("  Skip breakdown:");
    let labels = [
        "ContentBitsOverrun:",
        "HeaderReadError:",
        "ContentBitsReadError:",
        "ParseFailure:",
        "Unclassified:",
    ];
    for (label, (count, bits)) in labels.into_iter().zip(tally) {
        println!("    {label:<22}{count} events, {bits} bits");
    }
}

/// The skip-breakdown row of `reason`; the last row takes any reason added to
/// `SkipReason` after this match.
fn skip_reason_row(reason: &SkipReason) -> usize {
    match reason {
        SkipReason::ContentBitsOverrun { .. } => 0,
        SkipReason::HeaderReadError => 1,
        SkipReason::ContentBitsReadError => 2,
        SkipReason::ParseFailure => 3,
        _ => 4,
    }
}

/// Print full details for one diagnostic event.
fn print_diagnostic_event(index: usize, ev: &DiagnosticEvent) {
    println!("  +-- Diagnostic #{index} --");
    println!("  | Reason:              {:?}", ev.reason);
    println!("  | packet_id:           {}", ev.packet_id);
    println!("  | bunch_in_packet:     {}", ev.bunch_index_in_packet);
    println!("  | global_bunch_index:  {}", ev.global_bunch_index);
    println!("  | channel_bunch_index: {}", ev.channel_bunch_index);
    println!("  | channel_index:       {}", ev.channel_index);
    println!("  | actor_net_guid:      {}", ev.actor_net_guid);
    if let Some(ref path) = ev.actor_path {
        println!("  | actor_path:          {path}");
    }
    println!("  | archetype_net_guid:  {}", ev.archetype_net_guid);
    if let Some(ref path) = ev.class_path {
        println!("  | archetype_path:      {path}");
    }
    println!("  | bunch_flags:");
    let f = &ev.bunch_flags;
    println!(
        "  |   open={} close={} reliable={} partial={} partial_init={} partial_final={} pkg_map={} must_mapped={} dormant={}",
        f.b_open,
        f.b_close,
        f.b_reliable,
        f.b_partial,
        f.b_partial_initial,
        f.b_partial_final,
        f.b_has_package_map_exports,
        f.b_has_must_be_mapped_guids,
        f.b_dormant
    );
    println!("  | payload_bit_count:   {}", ev.payload_bit_count);
    println!("  | consumed_bits:       {}", ev.consumed_bits);
    println!("  | remaining_bits:      {}", ev.remaining_bits);
    if let Some(ref hdr) = ev.content_block_header {
        println!("  | content_block_header:");
        println!(
            "  |   has_rep_layout={} is_actor={} object_net_guid={} is_stably_named={} is_deleted={} class_net_guid={} outer_net_guid={} delete_flags={}",
            hdr.has_rep_layout,
            hdr.is_actor,
            hdr.object_net_guid,
            hdr.is_stably_named,
            hdr.is_deleted,
            hdr.class_net_guid,
            hdr.outer_net_guid,
            hdr.delete_flags
        );
    }
    if let Some(bits) = ev.content_bits {
        println!("  | content_bits:        {bits}");
    }
    println!("  | block_in_bunch:      {}", ev.block_index_in_bunch);
    println!("  | bits_skipped:        {}", ev.bits_skipped);
    println!("  +--------------------");
}

#[cfg(test)]
mod tests {
    use super::{
        Verdict, checkpoint_scope_note, partial_reassembly_scope_note, pass_rate_line,
        skip_reason_row, verdict_from_stats,
    };
    use vrf_net::stats::{NetStats, SkipReason};

    /// Each known reason has its own breakdown row; the `Unclassified` row
    /// (the last) is reserved for reasons this match does not know.
    #[test]
    fn every_known_skip_reason_has_its_own_row() {
        let rows: Vec<usize> = [
            SkipReason::ContentBitsOverrun {
                declared_content_bits: 9,
                available_bits: 8,
            },
            SkipReason::HeaderReadError,
            SkipReason::ContentBitsReadError,
            SkipReason::ParseFailure,
        ]
        .iter()
        .map(skip_reason_row)
        .collect();
        assert_eq!(rows, [0, 1, 2, 3]);
    }

    #[test]
    fn skipped_checkpoint_chunks_are_named_rather_than_implied() {
        assert_eq!(
            checkpoint_scope_note(0),
            None,
            "a replay with no checkpoints has no gap to declare"
        );
        let note = checkpoint_scope_note(37).expect("37 skipped chunks must be reported");
        assert!(note.contains("37"), "the note must carry the count: {note}");
    }

    #[test]
    fn partial_reassembly_rejections_are_named_as_unscored_input() {
        assert_eq!(partial_reassembly_scope_note(0), None);
        let note = partial_reassembly_scope_note(125_037)
            .expect("a non-zero rejected population must be disclosed");
        assert!(
            note.contains("125037"),
            "the note must carry the count: {note}"
        );
        assert!(
            note.contains("before content-block framing"),
            "the note must identify the scope boundary: {note}"
        );
        assert!(
            note.contains("excluded from the block score and verdict"),
            "the note must state the scoring consequence: {note}"
        );
    }

    /// A negative rate would not match the corpus sweeps' `([\d.]+)%`, so they
    /// would report no rate at all.
    #[test]
    fn the_pass_rate_saturates_like_the_passed_count() {
        assert_eq!(
            pass_rate_line(2, 3),
            "  ORACLE PASS RATE:     0.000000% (0 / 2 blocks passed)"
        );
        assert_eq!(
            pass_rate_line(4, 1),
            "  ORACLE PASS RATE:     75.000000% (3 / 4 blocks passed)"
        );
        assert_eq!(
            pass_rate_line(4, 0),
            "  ORACLE PASS RATE:     100.000000% (4 / 4 blocks passed)"
        );
    }

    #[test]
    fn the_verdict_separates_clean_from_failed_from_unvalidatable() {
        assert_eq!(Verdict::decide(1_000, 0), Verdict::Passed);
        assert_eq!(Verdict::decide(1_000, 1), Verdict::ValidationFailed);
        assert_eq!(Verdict::decide(0, 0), Verdict::NoContentBlocks);
    }

    /// Each hard-failure term alone fails a replay that otherwise passes; an
    /// unresolved RPC whose whole payload was kept and a reassembly rejection
    /// (reported as unscored) do not.
    #[test]
    fn every_unfinished_or_payload_failure_prevents_a_pass() {
        let failures: [fn(&mut NetStats); 11] = [
            |s| s.malformed_packets = 1,
            |s| s.bunch_header_failures = 1,
            |s| s.transform_failures = 1,
            |s| s.field_stream_failures = 1,
            |s| (s.class_net_cache_blocks, s.rpc_stream_failures) = (1, 1),
            |s| s.unfinished_partials = 1,
            |s| s.partial_resource_limit_failures = 1,
            |s| s.channel_state_limit_failures = 1,
            |s| s.content_block_framing_failures = 1,
            |s| (s.bunches_on_unopened_channel, s.unopened_channel_bits) = (1, 10),
            |s| s.package_map_exports = 1,
        ];
        let passes: [fn(&mut NetStats); 3] = [
            |_| {},
            |s| (s.rpc_stream_failures, s.unresolved_rpc_payloads_preserved) = (1, 1),
            |s| s.partial_errors = 1,
        ];
        let clean = || {
            let mut stats = NetStats::default();
            stats.rep_layout_blocks = 1;
            stats
        };
        let verdict = |set: fn(&mut NetStats)| {
            let mut stats = clean();
            set(&mut stats);
            verdict_from_stats(&stats, 0)
        };
        for (i, set) in failures.into_iter().enumerate() {
            assert_eq!(verdict(set), Verdict::ValidationFailed, "failure {i}");
        }
        for (i, set) in passes.into_iter().enumerate() {
            assert_eq!(verdict(set), Verdict::Passed, "pass {i}");
        }
        assert_eq!(
            verdict_from_stats(&clean(), 1),
            Verdict::ValidationFailed,
            "unconsumed decompressed ReplayData bytes must fail validation"
        );
    }

    #[test]
    fn each_verdict_earns_its_own_exit_code() {
        assert_eq!(Verdict::Passed.exit_code(), 0);
        assert_eq!(Verdict::ValidationFailed.exit_code(), 1);
        assert_eq!(Verdict::NoContentBlocks.exit_code(), 2);
    }
}
