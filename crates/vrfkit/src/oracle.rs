//! `validate` subcommand -- RepLayout grammar oracle.
//!
//! # What it proves, and over what
//!
//! The scope is the **ReplayData** stream. Checkpoint chunks are counted and
//! skipped -- a checkpoint is an independent archive with its own GUID cache
//! and export map, and decoding one is what `export --checkpoints` is for. This
//! used to go unsaid while the run announced it was checking "all content
//! blocks", which is why the skip now prints its own size under `NOT COVERED`.
//!
//! If the payload transform is correct, every decoded RepLayout content block
//! satisfies this grammar:
//!
//! ```text
//! checksum_bit : 1 bit
//! loop:
//!   handle = IntPacked   (0 -> end)
//!   payload_bits = IntPacked
//!   consume payload_bits
//! total consumed == declared bit_count
//! ```
//!
//! When the transform is wrong, `IntPacked` returns nonsense (enormous handles
//! or payload sizes) or the total consumed bits don't match the declared block
//! size. A correct transform yields ~100% pass rate; an incorrect one yields ~0%.
//!
//! This oracle uses every `ReplicationReader` counter that means bytes present
//! in ReplayData could not be consumed: packet/header/framing failures,
//! transform or inner-stream failures, unfinished reassembly state, and bytes
//! trailing the declared ReplayData payload all fail the verdict. An unresolved
//! ClassNetCache table is reported separately when the sink retained the whole
//! decoded block; unsupported attribution with a recoverable raw payload is not
//! treated as data loss.
//!
//! # Diagnostics
//!
//! Every malformed or skipped event is captured with full context (packet id,
//! bunch index, channel, actor, header fields, bit positions). This is the
//! primary tool for debugging new game builds where the transform may be
//! partially incorrect. Use `validate --diagnostics` to see the full dump.
//!
//! # Resolved: the one-block-per-replay residue
//!
//! For a long stretch every `release-13.01` replay lost exactly one content block
//! and a few hundred bits, always at the same fingerprint:
//!
//! ```text
//! packet_id 0, bunch 0, channel 1, actor_net_guid 2   (the replay controller)
//! payload_bit_count 831, consumed 136, remaining 695
//! header: has_rep_layout=false is_actor=false object_net_guid=96
//!         is_stably_named=true
//! content_bits read: 8335   (only 695 bits remain -> overrun)
//! ```
//!
//! Four hypotheses were tested and ruled out before the cause was found: the
//! missing `ReadNetPlayerIndex` byte, the GUID-path spelling used to detect the
//! controller, treating the subobject GUID as an exporting GUID, and a divergence
//! in the spawn-data or header bit order (both were compared line by line against
//! the reference and match).
//!
//! What found it was an exhaustive search rather than another hypothesis: the
//! 831-bit payload was re-framed from every start offset and each walk scored on
//! whether it consumed the payload to an exact end. Offset 108 does, yielding ten
//! well-formed blocks with sequential even GUIDs (64, 6, 8, 10, ... 22) carrying a
//! mix of RPC and property payloads -- exactly the replay controller's initial
//! subobject replication. We were starting at 109, so the spawn-data read
//! over-consumed **one bit**. The same relationship held on a second replay
//! (failure 133, correct start 105), and in both cases the gap is the content
//! block header (11) plus the `IntPacked` (16) plus that one bit.
//!
//! Instrumenting the spawn read bit by bit located it precisely:
//!
//! | sub-read | bits | position |
//! |---|---|---|
//! | actor GUID `IntPacked(2)` | 8 | 0..8 |
//! | archetype `IntPacked(9)` | 8 | 8..16 |
//! | level `IntPacked(3)` | 8 | 16..24 |
//! | location, quantized 18-bit components | 63 | 24..87 |
//! | rotation (flag, pitch absent, yaw set, roll absent) | 20 | 87..107 |
//! | scale, absent | 1 | 107..108 |
//! | velocity, absent | 1 | 108..109 |
//!
//! The velocity read is the extra bit: a `PlayerController` sets
//! `bReplicateMovement = false`, so the server never serializes velocity for it
//! and the field is absent from the wire rather than present-and-empty.
//!
//! Detection normally comes from the archetype path being registered as a
//! PlayerController, but the very first bunch carries
//! `bHasPackageMapExports = false`, so no path exists yet. The fallback is the
//! actor GUID: dynamic GUIDs are even and non-zero, so 2 is the lowest one
//! possible, and the first dynamic actor a VALORANT replay opens is always the
//! replay controller. See `read_dynamic_spawn_data`.
//!
//! Immediate result of the fix: malformed blocks 215 -> 0, and 2,150
//! previously-lost content blocks (ten per replay) now decode.
//!
//! This paragraph used to end "pass rate 100.000000% at min, median and max;
//! skipped bits 153,096 -> 3,671", and those figures were retracted long
//! before this comment was. They were not a better measurement -- they were
//! taken while the parser silently dropped every content block whose
//! `_ClassNetCache` group it could not resolve, without touching a counter,
//! so the oracle scored itself on data it had thrown away. Exposing that path
//! is what moved the numbers, not a regression.
//!
//! A historical `tools/validate_corpus.py` run over the same 215 replays,
//! before unresolved whole-payload blocks were separated from true loss:
//!
//! ```text
//! blocks 136,545,822   fields 98,884,839   rpcs 75,571,092
//! malformed 0          skipped 1,972,018,965
//! pass rate: min 97.487378%   median 99.323434%   max 99.682485%
//! ```
//!
//! Those skipped bits are attribution gaps, not framing: the blocks cut
//! correctly but their group could not be determined, so the handle width was
//! unknown and their complete decoded payloads were retained as raw rows. The
//! present verdict distinguishes those from malformed/abandoned streams. The
//! figures remain historical; re-measure before quoting.
//!
//! # How the reference parser compares
//!
//! The reference keeps content-block counters in `BunchPayloadStats` but never
//! emits them from its CLI or its manifest. Instrumenting it to print them gives,
//! on the same replay:
//!
//! | counter | reference | here |
//! |---|---|---|
//! | `MalformedContentBlockCount` | 0 | 0 |
//! | `MalformedPayloadCount` | **34,292** | -- |
//! | `ContentPayloadBitsSkipped` | **49,948,659** | **0** |
//! | `ContentBlockCount` | 563,626 | 608,020 |
//!
//! Its zero is not the same as ours. It abandons 34,292 bunches one level higher,
//! at the payload stage, and never enters content-block framing for them; that is
//! where its ~50 million skipped bits (6.2 MB) go, and why its content block
//! count is ~44,000 lower. The `malformed_packet_count` that *is* in its manifest
//! is a packet-level counter from a different struct and unrelated to either.

use std::fs;
use std::time::Instant;

use vrf_container::{
    ChunkIterator, ChunkType, decompress_replay_data_with_trailing, parse_preamble,
};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_net::stats::{DiagnosticEvent, NetStats, SkipReason};
use vrf_schema::NetGuidCache;

use crate::error::CliError;
use crate::sink::{ChannelState, ExportSink, RecordBuffers};

/// What a validation run concluded, and the exit code it earns.
///
/// Before this existed `run` ended in `Ok(())` on every path, so `vrfkit
/// validate` could not report failure: a replay whose framing was wrong
/// printed a low pass rate and exited 0, and a file with no ReplayData at all
/// printed "cannot validate" and exited 0 too. The verdict was a sentence on a
/// screen rather than a result.
///
/// The two failure outcomes stay apart. "I looked and found problems" and "I
/// had nothing to look at" are different answers, and a corpus sweep that
/// merged them could not tell a broken build from a file carrying no
/// replication stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Content blocks were found and every one of them framed.
    Passed,
    /// At least one framing, payload, reassembly, or trailing-data failure was
    /// observed. See [`Verdict::decide`].
    ValidationFailed,
    /// No RepLayout or ClassNetCache blocks at all -- nothing was validated.
    NoContentBlocks,
}

impl Verdict {
    /// Decide the verdict from the two counters that carry it.
    ///
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

    /// The process exit code for this verdict.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::ValidationFailed => 1,
            Self::NoContentBlocks => 2,
        }
    }
}

/// Decide from every counter that means the validation walk lost or could not
/// consume replay data.
///
/// `partial_errors` is reported but is not a decoder verdict: it means the
/// captured stream supplied a continuation without the earlier fragment (or a
/// mismatched continuation), so there is no complete payload for this process
/// to validate. An accumulator still holding bytes at EOF is different --
/// those bytes were present and the walk abandoned them, hence
/// `unfinished_partials` is a hard failure.
fn verdict_from_stats(stats: &NetStats, replay_data_trailing_bytes: u64) -> Verdict {
    let total_with_content = stats.rep_layout_blocks + stats.class_net_cache_blocks;
    let rpc_payloads_lost = stats
        .rpc_stream_failures
        .saturating_sub(stats.unresolved_rpc_payloads_preserved);
    let failures = stats.malformed_packets
        + stats.unfinished_partials
        + stats.channel_state_limit_failures
        + stats.partial_resource_limit_failures
        + stats.bunch_header_failures
        + stats.malformed_content_blocks
        + stats.transform_failures
        + stats.field_stream_failures
        + rpc_payloads_lost
        + u64::from(replay_data_trailing_bytes != 0);
    Verdict::decide(total_with_content, failures)
}

/// Run the validate oracle. If `diagnostics` is true, print full diagnostic
/// dumps for every malformed/skipped event.
///
/// The `Result` is still the container/IO failure channel -- a file that cannot
/// be read at all is an error, not a verdict. A file that *was* read reports
/// through [`Verdict`].
pub fn run(path: &str, diagnostics: bool) -> Result<Verdict, CliError> {
    let start = Instant::now();

    eprintln!("reading {path}...");
    let data = fs::read(path)?;
    let preamble = parse_preamble(&data)?;
    let branch = &preamble.header.replay_version.branch;
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;

    eprintln!("branch: {branch}");
    eprintln!("validating RepLayout grammar on every ReplayData content block...");

    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    let mut total_packets: u32 = 0;
    // Counted, not merely skipped: see `checkpoint_scope_note`.
    let mut checkpoint_chunks: u64 = 0;
    let mut replay_data_trailing_bytes = 0u64;
    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut channel_state = ChannelState::new();
    // Reused across every packet: the oracle never drains these, and
    // `ExportSink::new` clears them, so they stay bounded by the largest packet.
    let mut buffers = RecordBuffers::default();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        if chunk.chunk_type == ChunkType::Checkpoint {
            checkpoint_chunks += 1;
            continue;
        }
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        let (decompressed, trailing) =
            decompress_replay_data_with_trailing(payload, compressed, encrypted)?;
        replay_data_trailing_bytes += trailing as u64;

        iter_demo_frames(&decompressed, flags, &mut cache, |pkt, packet_cache| {
            let mut sink = ExportSink::new(packet_cache, &mut channel_state, &mut buffers);
            sink.time_ms = pkt.time_ms;
            sink.packet_id = total_packets;
            repl_reader.process_packet(pkt.data, total_packets as i32, &mut sink);
            total_packets += 1;
        })?;
    }

    repl_reader.finish();
    let stats = repl_reader.stats();
    let elapsed = start.elapsed();

    let total_content = stats.content_blocks;
    let rep_layout = stats.rep_layout_blocks;
    let class_net = stats.class_net_cache_blocks;
    let malformed = stats.malformed_content_blocks;
    let deleted = stats.deleted_blocks;
    // A block can fail at four different depths, and only counting the shallowest
    // would overstate the verdict: framing can look fine while the payload inside
    // is unreadable. `NetStats::lost_content_blocks` owns that definition; it is
    // shared with `manifest.rs` so the number the oracle prints and the number
    // `quality.content_blocks_lost` publishes cannot drift apart.
    let rpc_payloads_lost = stats
        .rpc_stream_failures
        .saturating_sub(stats.unresolved_rpc_payloads_preserved);
    let failed = stats.lost_content_blocks();

    println!();
    println!("=== Validation Oracle ===");
    println!("  Branch:               {branch}");
    println!("  Total content blocks: {total_content}");
    println!("    RepLayout:          {rep_layout}");
    println!("    ClassNetCache:      {class_net}");
    println!("    Deleted:            {deleted}");
    println!("    Malformed packets:  {}", stats.malformed_packets);
    println!(
        "    Partial bunches:    {} errors / {} fragments / {} completed",
        stats.partial_errors, stats.partial_fragments, stats.partial_completed
    );
    println!("    Bunch header failed:{}", stats.bunch_header_failures);
    println!("    Malformed framing:  {malformed}");
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
    println!("  Packets:              {}", stats.packets);
    println!("  Bunches:              {}", stats.bunches);
    println!("  Actor opens:          {}", stats.actor_opens);
    println!("  Actor closes:         {}", stats.actor_closes);
    if let Some(note) = checkpoint_scope_note(checkpoint_chunks) {
        println!("  NOT COVERED:          {note}");
    }
    println!();

    // Oracle verdict: the fraction of content blocks (RepLayout + ClassNetCache)
    // that framed, decoded and walked cleanly. With a correct transform this
    // should be 100%; a wrong one collapses it toward zero.
    let total_with_content = rep_layout + class_net;
    let verdict = verdict_from_stats(stats, replay_data_trailing_bytes);
    if total_with_content == 0 {
        println!("  No content blocks found - cannot validate.");
    } else {
        let pass_rate = 1.0 - (failed as f64 / total_with_content as f64);
        println!(
            "  ORACLE PASS RATE:     {:.6}% ({} / {} blocks passed)",
            pass_rate * 100.0,
            total_with_content - failed,
            total_with_content
        );
        if stats.skipped_bits > 0 {
            println!(
                "  (skipped {} bits across {} failed blocks)",
                stats.skipped_bits, failed
            );
        }
    }

    // Name the payload-stage failures. The counters above say how many; these
    // lines say which class, which is what an investigation needs.
    let stream_failures = channel_state.stream_failures();
    if !stream_failures.is_empty() {
        println!();
        println!("=== Stream failures ({} shown) ===", stream_failures.len());
        for line in stream_failures {
            println!("  {line}");
        }
    }

    // Diagnostic summary -- always shown when events exist
    if !stats.diagnostics.is_empty() {
        println!();
        // The retained list is capped (an unbounded one reaches ~100 MB on a
        // replay whose transform is wrong), so its length is not the event
        // count once the cap is hit. Printing only `len()` would turn an
        // honest counter into a screen that quietly under-reports -- the exact
        // shape of the oracle bug section 5-A was about, in the display layer
        // instead of the parser.
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

        // Aggregate skipped bits by source
        print_skip_breakdown(&stats.diagnostics);

        if diagnostics {
            println!();
            for (i, event) in stats.diagnostics.iter().enumerate() {
                print_diagnostic_event(i, event);
            }
        } else {
            println!();
            println!("  (use --diagnostics to see full event dumps)");
        }
    }

    println!();
    println!("  Elapsed: {:.2?}", elapsed);
    // Last line, and the only one that is a conclusion rather than a
    // measurement. It says in words what the exit code says in a number, so a
    // human reading a terminal and a script reading `$?` cannot disagree.
    println!("  VERDICT: {}", verdict_line(verdict));

    Ok(verdict)
}

/// The scope limit to declare when the file carried Checkpoint chunks.
///
/// This oracle walks the ReplayData stream. Checkpoint chunks are skipped, and
/// that was implicit: the run announced "validating RepLayout grammar on all
/// content blocks" and then ignored every one of them, so malformed
/// replication framing inside a snapshot was never seen and nothing on the
/// screen said as much.
///
/// The skip stays. A checkpoint is an independent archive -- its own GUID
/// cache, its own export map, its own DemoFrame re-opening every live actor --
/// and walking it is what `export --checkpoints` exists for; folding that pass
/// into `validate` would change every counter this command's pinned baselines
/// hold and add three new hard-failure paths to a command whose job is to
/// report rather than to abort. What changes is that the gap now states its own
/// size instead of being inferred from a sentence that overclaimed.
fn checkpoint_scope_note(checkpoint_chunks: u64) -> Option<String> {
    (checkpoint_chunks > 0).then(|| {
        format!(
            "{checkpoint_chunks} Checkpoint chunk(s) were NOT walked - this verdict covers the ReplayData stream only (use `export --checkpoints` to decode them)"
        )
    })
}

/// The one-line conclusion printed under `VERDICT:`.
fn verdict_line(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Passed => "PASS - all ReplayData was consumed and decoded (exit 0)",
        Verdict::ValidationFailed => "FAIL - ReplayData validation found loss (exit 1)",
        Verdict::NoContentBlocks => "CANNOT VALIDATE - no content blocks found (exit 2)",
    }
}

/// Print a breakdown of where skipped bits come from.
fn print_skip_breakdown(events: &[DiagnosticEvent]) {
    let mut overrun_count = 0u32;
    let mut overrun_bits = 0u64;
    let mut header_err_count = 0u32;
    let mut header_err_bits = 0u64;
    let mut bits_read_err_count = 0u32;
    let mut bits_read_err_bits = 0u64;
    let mut parse_fail_count = 0u32;
    let mut parse_fail_bits = 0u64;

    for ev in events {
        match &ev.reason {
            SkipReason::ContentBitsOverrun { .. } => {
                overrun_count += 1;
                overrun_bits += ev.bits_skipped;
            }
            SkipReason::HeaderReadError => {
                header_err_count += 1;
                header_err_bits += ev.bits_skipped;
            }
            SkipReason::ContentBitsReadError => {
                bits_read_err_count += 1;
                bits_read_err_bits += ev.bits_skipped;
            }
            SkipReason::ParseFailure => {
                parse_fail_count += 1;
                parse_fail_bits += ev.bits_skipped;
            }
        }
    }

    println!("  Skip breakdown:");
    if overrun_count > 0 {
        println!("    ContentBitsOverrun:   {overrun_count} events, {overrun_bits} bits");
    }
    if header_err_count > 0 {
        println!("    HeaderReadError:      {header_err_count} events, {header_err_bits} bits");
    }
    if bits_read_err_count > 0 {
        println!(
            "    ContentBitsReadError: {bits_read_err_count} events, {bits_read_err_bits} bits"
        );
    }
    if parse_fail_count > 0 {
        println!("    ParseFailure:         {parse_fail_count} events, {parse_fail_bits} bits");
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
        println!("  | class_path:          {path}");
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
    use super::{Verdict, checkpoint_scope_note, verdict_from_stats};
    use vrf_net::stats::NetStats;

    /// The chunks this oracle does not walk have to say so themselves.
    ///
    /// `validate` announced that it was checking "all content blocks" and then
    /// skipped every Checkpoint chunk, so malformed replication framing inside
    /// a snapshot was never looked at and nothing on the screen admitted it.
    /// The skip is a real scope limit -- a checkpoint is a separate archive
    /// with its own GUID cache and export map, and walking it is an `export
    /// --checkpoints` job -- but a limit that is not stated reads as coverage.
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

    /// `vrfkit validate` has to be able to report failure.
    ///
    /// It could not: `run` ended in `Ok(())` on every path, so a replay whose
    /// framing was wrong printed a low pass rate and exited 0 exactly like a
    /// clean one, and a file with no ReplayData at all printed "cannot
    /// validate" and also exited 0. A verdict that cannot move is not a
    /// verdict.
    ///
    /// The two failing outcomes are kept apart. "Found problems" and "had
    /// nothing to look at" are different answers, and merging them into one
    /// non-zero code would make a corpus sweep unable to tell a broken build
    /// from a file that carries no replication stream.
    #[test]
    fn the_verdict_separates_clean_from_failed_from_unvalidatable() {
        assert_eq!(Verdict::decide(1_000, 0), Verdict::Passed);
        assert_eq!(Verdict::decide(1_000, 1), Verdict::ValidationFailed);
        assert_eq!(Verdict::decide(0, 0), Verdict::NoContentBlocks);
    }

    #[test]
    fn every_unfinished_or_payload_failure_prevents_a_pass() {
        let clean = NetStats {
            rep_layout_blocks: 1,
            ..NetStats::default()
        };
        assert_eq!(verdict_from_stats(&clean, 0), Verdict::Passed);

        for failed in [
            NetStats {
                rep_layout_blocks: 1,
                transform_failures: 1,
                ..NetStats::default()
            },
            NetStats {
                rep_layout_blocks: 1,
                field_stream_failures: 1,
                ..NetStats::default()
            },
            NetStats {
                class_net_cache_blocks: 1,
                rpc_stream_failures: 1,
                ..NetStats::default()
            },
            NetStats {
                rep_layout_blocks: 1,
                unfinished_partials: 1,
                ..NetStats::default()
            },
            NetStats {
                rep_layout_blocks: 1,
                partial_resource_limit_failures: 1,
                ..NetStats::default()
            },
            NetStats {
                rep_layout_blocks: 1,
                channel_state_limit_failures: 1,
                ..NetStats::default()
            },
        ] {
            assert_eq!(verdict_from_stats(&failed, 0), Verdict::ValidationFailed);
        }
        assert_eq!(
            verdict_from_stats(&clean, 1),
            Verdict::ValidationFailed,
            "unconsumed decompressed ReplayData bytes must fail validation"
        );

        let unresolved_but_preserved = NetStats {
            class_net_cache_blocks: 1,
            rpc_stream_failures: 1,
            unresolved_rpc_payloads_preserved: 1,
            ..NetStats::default()
        };
        assert_eq!(
            verdict_from_stats(&unresolved_but_preserved, 0),
            Verdict::Passed,
            "an unresolved RPC whose whole decoded payload was preserved is not data loss"
        );

        let missing_prior_fragments = NetStats {
            rep_layout_blocks: 1,
            partial_errors: 1,
            ..NetStats::default()
        };
        assert_eq!(
            verdict_from_stats(&missing_prior_fragments, 0),
            Verdict::Passed,
            "a missing earlier network fragment is reported, but does not prove this decoder lost bytes"
        );
    }

    /// The three outcomes must reach the shell as three different codes.
    #[test]
    fn each_verdict_earns_its_own_exit_code() {
        assert_eq!(Verdict::Passed.exit_code(), 0);
        assert_eq!(Verdict::ValidationFailed.exit_code(), 1);
        assert_eq!(Verdict::NoContentBlocks.exit_code(), 2);
    }
}
