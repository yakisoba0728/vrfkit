//! `validate` subcommand — RepLayout grammar oracle.
//!
//! # What it proves
//!
//! If the payload transform is correct, every decoded RepLayout content block
//! satisfies this grammar:
//!
//! ```text
//! checksum_bit : 1 bit
//! loop:
//!   handle = IntPacked   (0 → end)
//!   payload_bits = IntPacked
//!   consume payload_bits
//! total consumed == declared bit_count
//! ```
//!
//! When the transform is wrong, `IntPacked` returns nonsense (enormous handles
//! or payload sizes) or the total consumed bits don't match the declared block
//! size. A correct transform yields ~100% pass rate; an incorrect one yields ~0%.
//!
//! This oracle uses the ReplicationReader's stats: blocks that fail
//! `parse_rep_layout` contribute to `skipped_bits`. Blocks with zero
//! `malformed_content_blocks` and minimal `skipped_bits` relative to total
//! content prove the transform is correct.
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
//! Corpus-wide result after the fix (`tools/validate_corpus.py`, 215 replays):
//! pass rate **100.000000%** at min, median and max; malformed blocks 215 -> 0;
//! skipped bits 153,096 -> 3,671; and 2,150 previously-lost content blocks (ten
//! per replay) now decode.
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
//! | `MalformedPayloadCount` | **34,292** | — |
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

use vrf_container::{ChunkIterator, ChunkType, decompress_replay_data, parse_preamble};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_net::stats::{DiagnosticEvent, SkipReason};
use vrf_schema::NetGuidCache;

use crate::error::CliError;
use crate::sink::{ChannelState, ExportSink};

/// Run the validate oracle. If `diagnostics` is true, print full diagnostic
/// dumps for every malformed/skipped event.
pub fn run(path: &str, diagnostics: bool) -> Result<(), CliError> {
    let start = Instant::now();

    eprintln!("reading {path}...");
    let data = fs::read(path)?;
    let preamble = parse_preamble(&data)?;
    let branch = &preamble.header.replay_version.branch;
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;

    eprintln!("branch: {branch}");
    eprintln!("validating RepLayout grammar on all content blocks...");

    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    let mut total_packets: u32 = 0;
    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut packet_descs: Vec<(u32, usize, usize)> = Vec::with_capacity(4096);
    let mut channel_state = ChannelState::new();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        let decompressed = decompress_replay_data(payload, compressed, encrypted)?;

        // Phase 1: frame iteration (populates cache, collects packet offsets)
        packet_descs.clear();
        iter_demo_frames(&decompressed, flags, &mut cache, |pkt| {
            let offset = pkt.data.as_ptr() as usize - decompressed.as_ptr() as usize;
            packet_descs.push((pkt.time_ms, offset, pkt.data.len()));
        })?;

        // Phase 2: process packets
        for &(time_ms, offset, len) in &packet_descs {
            let pkt_data = &decompressed[offset..offset + len];
            let mut sink = ExportSink::new(&mut cache, &mut channel_state);
            sink.time_ms = time_ms;
            sink.packet_id = total_packets;
            repl_reader.process_packet(pkt_data, total_packets as i32, &mut sink);
            total_packets += 1;
        }
    }

    let stats = repl_reader.stats();
    let elapsed = start.elapsed();

    let total_content = stats.content_blocks;
    let rep_layout = stats.rep_layout_blocks;
    let class_net = stats.class_net_cache_blocks;
    let malformed = stats.malformed_content_blocks;
    let deleted = stats.deleted_blocks;
    // A block can fail at three different depths, and only counting the shallowest
    // would overstate the verdict: framing can look fine while the payload inside
    // is unreadable. All three are failures for oracle purposes.
    let payload_failures =
        stats.transform_failures + stats.field_stream_failures + stats.rpc_stream_failures;
    let failed = malformed + payload_failures;

    println!();
    println!("=== Validation Oracle ===");
    println!("  Branch:               {branch}");
    println!("  Total content blocks: {total_content}");
    println!("    RepLayout:          {rep_layout}");
    println!("    ClassNetCache:      {class_net}");
    println!("    Deleted:            {deleted}");
    println!("    Malformed framing:  {malformed}");
    println!("    Transform failed:   {}", stats.transform_failures);
    println!("    Field stream failed:{}", stats.field_stream_failures);
    println!("    RPC stream failed:  {}", stats.rpc_stream_failures);
    println!("  Fields emitted:       {}", stats.fields);
    println!("  RPCs emitted:         {}", stats.rpcs);
    println!("  Skipped bits:         {}", stats.skipped_bits);
    println!("  Packets:              {}", stats.packets);
    println!("  Bunches:              {}", stats.bunches);
    println!("  Actor opens:          {}", stats.actor_opens);
    println!("  Actor closes:         {}", stats.actor_closes);
    println!();

    // Oracle verdict: the fraction of content blocks (RepLayout + ClassNetCache)
    // that framed, decoded and walked cleanly. With a correct transform this
    // should be 100%; a wrong one collapses it toward zero.
    let total_with_content = rep_layout + class_net;
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

    // Diagnostic summary — always shown when events exist
    if !stats.diagnostics.is_empty() {
        println!();
        println!(
            "=== Diagnostic Events ({} total) ===",
            stats.diagnostics.len()
        );

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

    Ok(())
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
    println!("  ┌── Diagnostic #{index} ──");
    println!("  │ Reason:              {:?}", ev.reason);
    println!("  │ packet_id:           {}", ev.packet_id);
    println!("  │ bunch_in_packet:     {}", ev.bunch_index_in_packet);
    println!("  │ global_bunch_index:  {}", ev.global_bunch_index);
    println!("  │ channel_bunch_index: {}", ev.channel_bunch_index);
    println!("  │ channel_index:       {}", ev.channel_index);
    println!("  │ actor_net_guid:      {}", ev.actor_net_guid);
    if let Some(ref path) = ev.actor_path {
        println!("  │ actor_path:          {path}");
    }
    println!("  │ archetype_net_guid:  {}", ev.archetype_net_guid);
    if let Some(ref path) = ev.class_path {
        println!("  │ class_path:          {path}");
    }
    println!("  │ bunch_flags:");
    let f = &ev.bunch_flags;
    println!(
        "  │   open={} close={} reliable={} partial={} partial_init={} partial_final={} pkg_map={} must_mapped={} dormant={}",
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
    println!("  │ payload_bit_count:   {}", ev.payload_bit_count);
    println!("  │ consumed_bits:       {}", ev.consumed_bits);
    println!("  │ remaining_bits:      {}", ev.remaining_bits);
    if let Some(ref hdr) = ev.content_block_header {
        println!("  │ content_block_header:");
        println!(
            "  │   has_rep_layout={} is_actor={} object_net_guid={} is_stably_named={} is_deleted={} class_net_guid={} outer_net_guid={} delete_flags={}",
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
        println!("  │ content_bits:        {bits}");
    }
    println!("  │ block_in_bunch:      {}", ev.block_index_in_bunch);
    println!("  │ bits_skipped:        {}", ev.bits_skipped);
    println!("  └────────────────────");
}
