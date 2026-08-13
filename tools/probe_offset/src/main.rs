//! Exhaustive offset probe for the malformed content block.
//!
//! Processes the full replay through the replication pipeline, captures the raw
//! payload of the bunch that triggers ContentBitsOverrun, then tries
//! content-block framing at every bit offset from 0 to 200.
//!
//! Usage:
//!   probe-offset <path-to-vrf> [<path-to-vrf-2> ...]

use std::env;
use std::fs;
use std::process;

use vrf_bitio::BitReader;
use vrf_container::{ChunkIterator, ChunkType, decompress_replay_data, parse_preamble};
use vrf_frame::iter_demo_frames;
use vrf_net::content::ContentBlockHeader;
use vrf_net::field::FieldSink;
use vrf_net::net_guid::GuidPathSink;
use vrf_net::pipeline::{ActorChannelState, ReplicationReader, ReplicationSink};
use vrf_net::stats::SkipReason;
use vrf_net::types::NetworkGuid;
use vrf_schema::NetGuidCache;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: probe-offset <vrf-file> [<vrf-file> ...]");
        process::exit(1);
    }

    for path in &args[1..] {
        println!("=== Processing: {path} ===");
        if let Err(e) = process_file(path) {
            eprintln!("  ERROR: {e}");
        }
        println!();
    }
}

/// Minimal sink that does nothing except fulfill the trait.
#[derive(Default)]
struct MinimalSink;

impl GuidPathSink for MinimalSink {
    fn register_path(&mut self, _guid: u32, _path: &str, _outer: NetworkGuid) {}
}

impl FieldSink for MinimalSink {
    fn on_field(&mut self, _handle: u32, _bit_count: u32, _reader: BitReader<'_>) {}
    fn on_rpc(&mut self, _handle: u32, _bit_count: u32, _reader: BitReader<'_>) {}
}

impl ReplicationSink for MinimalSink {
    fn on_actor_open(&mut self, _state: &ActorChannelState) {}
    fn on_actor_close(
        &mut self,
        _channel_index: u32,
        _actor_net_guid: NetworkGuid,
        _dormant: bool,
    ) {
    }
    fn on_content_block(
        &mut self,
        _channel_index: u32,
        _actor_net_guid: NetworkGuid,
        _header: &ContentBlockHeader,
    ) -> u32 {
        0
    }
    fn on_deleted_block(
        &mut self,
        _channel_index: u32,
        _actor_net_guid: NetworkGuid,
        _header: &ContentBlockHeader,
    ) {
    }
}

fn process_file(path: &str) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("read: {e}"))?;
    let preamble = parse_preamble(&data).map_err(|e| format!("preamble: {e}"))?;
    let branch = &preamble.header.replay_version.branch;
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;

    println!("  branch: {branch}");

    // Process full replay through replication reader
    let mut cache = NetGuidCache::new();
    let mut repl_reader =
        ReplicationReader::new(branch).map_err(|e| format!("unsupported branch: {e}"))?;
    let mut sink = MinimalSink;

    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut total_packets: u32 = 0;

    // Capture packets for the later offset probe, while processing each one
    // immediately so it sees exactly the schema state at its wire position.
    let mut all_packets: Vec<Vec<u8>> = Vec::new();

    while let Some(chunk) = chunk_iter.next_chunk().map_err(|e| format!("chunk: {e}"))? {
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }
        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        let decompressed = decompress_replay_data(payload, compressed, encrypted)
            .map_err(|e| format!("decompress: {e}"))?;

        iter_demo_frames(&decompressed, flags, &mut cache, |pkt, _packet_cache| {
            all_packets.push(pkt.data.to_vec());
            repl_reader.process_packet(pkt.data, total_packets as i32, &mut sink);
            total_packets += 1;
        })
        .map_err(|e| format!("frames: {e}"))?;
    }

    let stats = repl_reader.stats();
    println!("  total packets: {total_packets}");
    println!("  bunches: {}", stats.bunches);
    println!("  content_blocks: {}", stats.content_blocks);
    println!(
        "  malformed_content_blocks: {}",
        stats.malformed_content_blocks
    );
    println!("  skipped_bits: {}", stats.skipped_bits);
    println!("  diagnostics: {}", stats.diagnostics.len());

    // Find the ContentBitsOverrun diagnostic
    for (idx, diag) in stats.diagnostics.iter().enumerate() {
        if let SkipReason::ContentBitsOverrun {
            declared_content_bits,
            available_bits,
        } = &diag.reason
        {
            println!();
            println!("  --- Diagnostic #{idx} ---");
            println!(
                "    reason: ContentBitsOverrun {{ declared={declared_content_bits}, available={available_bits} }}"
            );
            println!("    packet_id: {}", diag.packet_id);
            println!("    bunch_index_in_packet: {}", diag.bunch_index_in_packet);
            println!("    channel_index: {}", diag.channel_index);
            println!("    actor_net_guid: {}", diag.actor_net_guid);
            println!("    archetype_net_guid: {}", diag.archetype_net_guid);
            println!("    payload_bit_count: {}", diag.payload_bit_count);
            println!("    consumed_bits: {}", diag.consumed_bits);
            println!("    remaining_bits: {}", diag.remaining_bits);
            println!("    block_index_in_bunch: {}", diag.block_index_in_bunch);
            println!("    bits_skipped: {}", diag.bits_skipped);
            if let Some(hdr) = &diag.content_block_header {
                println!(
                    "    header: has_rep_layout={}, is_actor={}, object_net_guid={}, is_stably_named={}",
                    hdr.has_rep_layout, hdr.is_actor, hdr.object_net_guid, hdr.is_stably_named
                );
            }
            println!(
                "    bunch_flags: open={}, reliable={}, partial={}, pkg_map={}, must_mapped={}",
                diag.bunch_flags.b_open,
                diag.bunch_flags.b_reliable,
                diag.bunch_flags.b_partial,
                diag.bunch_flags.b_has_package_map_exports,
                diag.bunch_flags.b_has_must_be_mapped_guids
            );

            // Now extract the raw bunch payload from the packet
            let pkt_id = diag.packet_id as usize;
            if pkt_id < all_packets.len() {
                let pkt_data = &all_packets[pkt_id];
                println!();
                println!(
                    "    Extracting raw payload from packet {pkt_id} ({} bytes)...",
                    pkt_data.len()
                );

                // Parse through bunches to find the one at bunch_index_in_packet
                extract_and_probe_bunch(
                    pkt_data,
                    diag.bunch_index_in_packet,
                    diag.payload_bit_count as u64,
                    diag.consumed_bits,
                    diag.remaining_bits,
                );
            }
        }
    }

    Ok(())
}

fn extract_and_probe_bunch(
    packet_data: &[u8],
    target_bunch_index: u32,
    expected_payload_bits: u64,
    consumed_at_failure: u64,
    remaining_at_failure: u64,
) {
    let last_byte = match packet_data.last() {
        Some(&b) if b != 0 => b,
        _ => {
            println!("    ERROR: malformed packet");
            return;
        }
    };
    let bit_size = compute_bit_size(packet_data, last_byte);
    let Ok(mut reader) = BitReader::with_bit_len(packet_data, bit_size as u64) else {
        println!("    ERROR: packet bit length exceeds its byte buffer");
        return;
    };

    // Parse bunch headers until we reach the target bunch
    let mut bunch_idx: u32 = 0;
    loop {
        let pos_before = reader.position();

        // Parse bunch header
        let header = match parse_bunch_header_full(&mut reader) {
            Ok(h) => h,
            Err(e) => {
                println!("    ERROR parsing bunch header #{bunch_idx}: {e}");
                return;
            }
        };

        if header.payload_bit_count as u64 > reader.bits_remaining() {
            println!("    ERROR: bunch #{bunch_idx} payload overruns packet");
            return;
        }

        if bunch_idx == target_bunch_index {
            let actual_payload_bits = header.payload_bit_count as u64;
            println!("    Found target bunch at bit offset {pos_before}");
            println!(
                "    payload_bit_count: {actual_payload_bits} (expected: {expected_payload_bits})"
            );

            // Extract the payload
            let byte_count = (actual_payload_bits as usize).div_ceil(8);
            let mut payload = vec![0u8; byte_count];
            if reader
                .copy_bits_to(&mut payload, actual_payload_bits)
                .is_err()
            {
                println!("    ERROR: cannot copy payload bits");
                return;
            }

            println!(
                "    consumed_at_failure: {consumed_at_failure}, remaining: {remaining_at_failure}"
            );
            println!("    payload total: {actual_payload_bits} bits");
            println!();

            // Print hex dump of the payload
            print!("    payload hex: ");
            for i in 0..byte_count.min(64) {
                print!("{:02x} ", payload[i]);
                if (i + 1) % 32 == 0 && i + 1 < byte_count.min(64) {
                    print!("\n                 ");
                }
            }
            println!();
            println!();

            // Now do the exhaustive offset scan
            do_offset_scan(&payload, actual_payload_bits, consumed_at_failure);
            return;
        }

        // Skip this bunch's payload
        if reader.skip_bits(header.payload_bit_count as u64).is_err() {
            println!("    ERROR: cannot skip bunch #{bunch_idx} payload");
            return;
        }
        bunch_idx += 1;
    }
}

fn do_offset_scan(payload_data: &[u8], payload_bits: u64, consumed_at_failure: u64) {
    println!("    === OFFSET SCAN (bit offsets 0..200) ===");
    println!(
        "    {:>6} | {:>6} | {:>10} | {:>10} | {}",
        "offset", "blocks", "consumed", "remaining", "failure reason"
    );
    println!("    {}", "-".repeat(80));

    let mut passing_offsets: Vec<(u64, u32, u64, u64)> = Vec::new();

    for start_offset in 0..=200u64 {
        if start_offset >= payload_bits {
            break;
        }
        let (blocks, consumed, remaining, failure) =
            try_content_blocks_at_offset(payload_data, payload_bits, start_offset);

        let pass = failure.is_empty() && remaining < 8;
        if pass {
            passing_offsets.push((start_offset, blocks, consumed, remaining));
        }

        // Only print interesting lines (pass, near current offset, or every 10)
        let near_current = (start_offset as i64 - consumed_at_failure as i64).unsigned_abs() <= 5;
        let interesting = pass || near_current || start_offset % 20 == 0;
        if interesting {
            let marker = if pass { " <<< PASS" } else { "" };
            println!(
                "    {:>6} | {:>6} | {:>10} | {:>10} | {}{}",
                start_offset, blocks, consumed, remaining, failure, marker
            );
        }
    }

    println!();
    println!("    === SUMMARY ===");
    println!("    Total offsets tested: {}", 201u64.min(payload_bits));
    println!("    Passing offsets: {}", passing_offsets.len());
    for (off, blocks, consumed, remaining) in &passing_offsets {
        println!(
            "      offset {off}: {blocks} blocks, consumed {consumed} bits, remaining {remaining} bits"
        );
    }

    if passing_offsets.is_empty() {
        println!("    NO offset passes => not a simple shift problem.");
    } else {
        println!();
        println!("    Current parser offset (consumed_at_failure): {consumed_at_failure}");
        for (off, _, _, _) in &passing_offsets {
            let diff = *off as i64 - consumed_at_failure as i64;
            println!("      Passing offset {off}: diff from {consumed_at_failure} = {diff} bits");
        }
    }

    // Detailed dump of passing offsets
    println!();
    println!("    === DETAILED BLOCK DUMP for passing offsets ===");
    for (off, _, _, _) in &passing_offsets {
        println!();
        println!("    --- offset {off} ---");
        dump_blocks_at_offset(payload_data, payload_bits, *off);
    }

    // Additional experiments
    println!();
    println!("    === EXPERIMENT: IntPacked values at offsets around failure point ===");
    let start = if consumed_at_failure > 20 {
        consumed_at_failure - 20
    } else {
        0
    };
    let end = (consumed_at_failure + 30).min(payload_bits);
    for off in start..=end {
        let Ok(mut r) = BitReader::with_bit_len(payload_data, payload_bits) else {
            continue;
        };
        if r.skip_bits(off).is_err() {
            continue;
        }
        match r.read_int_packed() {
            Ok(val) => {
                let bits_used = r.position() - off;
                let rem = payload_bits - r.position();
                let fits = (val as u64) <= rem;
                if fits || (off >= consumed_at_failure - 5 && off <= consumed_at_failure + 5) {
                    println!(
                        "      offset {off}: IntPacked={val} ({bits_used} bits), remaining={rem}, fits={}",
                        if fits { "YES" } else { "NO" }
                    );
                }
            }
            Err(_) => {}
        }
    }

    // Try reading pure RepLayout stream (no header, just field handles + sizes)
    println!();
    println!(
        "    === EXPERIMENT: no-header framing (pure content_bits only) at offsets around failure ==="
    );
    let start = if consumed_at_failure > 5 {
        consumed_at_failure - 5
    } else {
        0
    };
    let end = (consumed_at_failure + 20).min(payload_bits);
    for off in start..=end {
        let (blocks, consumed, remaining, failure) =
            try_pure_content_bits_at_offset(payload_data, payload_bits, off);
        let pass = failure.is_empty() && remaining < 8;
        if pass || blocks > 0 {
            let marker = if pass { " <<< PASS" } else { "" };
            println!(
                "      offset {off}: {blocks} blocks, consumed {consumed}, remaining {remaining}, {failure}{marker}"
            );
        }
    }
}

/// Dump detailed info about each block at a passing offset.
fn dump_blocks_at_offset(payload_data: &[u8], payload_bits: u64, start: u64) {
    let Ok(mut reader) = BitReader::with_bit_len(payload_data, payload_bits) else {
        println!("      declared payload length exceeds its byte buffer");
        return;
    };
    if reader.skip_bits(start).is_err() {
        println!("      cannot seek");
        return;
    }

    let mut block_idx = 0u32;
    loop {
        if reader.at_end() || reader.bits_remaining() == 0 {
            break;
        }

        let pos_before = reader.position();

        // Read header bits
        let has_rep_layout = match reader.read_bit() {
            Ok(v) => v,
            Err(_) => break,
        };
        let is_actor = match reader.read_bit() {
            Ok(v) => v,
            Err(_) => break,
        };

        if is_actor {
            // Read content_bits
            let content_bits = match reader.read_int_packed() {
                Ok(v) => v,
                Err(e) => {
                    println!("      block {block_idx}: actor, content_bits read error: {e}");
                    break;
                }
            };
            let hdr_end = reader.position();
            if content_bits > 0 {
                if (content_bits as u64) > reader.bits_remaining() {
                    println!(
                        "      block {block_idx}: actor, rep_layout={has_rep_layout}, content_bits={content_bits} OVERRUN at pos {hdr_end}"
                    );
                    break;
                }
                let _ = reader.skip_bits(content_bits as u64);
            }
            println!(
                "      block {block_idx}: is_actor, rep_layout={has_rep_layout}, content_bits={content_bits}, span={}..{}",
                pos_before,
                reader.position()
            );
            block_idx += 1;
            continue;
        }

        // Read obj guid
        let obj_guid = match reader.read_int_packed() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: obj_guid error: {e}");
                break;
            }
        };

        let is_stably_named = match reader.read_bit() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: is_stably_named error: {e}");
                break;
            }
        };

        if is_stably_named {
            let content_bits = match reader.read_int_packed() {
                Ok(v) => v,
                Err(e) => {
                    println!("      block {block_idx}: stably_named content_bits error: {e}");
                    break;
                }
            };
            if content_bits > 0 {
                if (content_bits as u64) > reader.bits_remaining() {
                    println!(
                        "      block {block_idx}: stably_named obj={obj_guid}, content_bits={content_bits} OVERRUN"
                    );
                    break;
                }
                let _ = reader.skip_bits(content_bits as u64);
            }
            println!(
                "      block {block_idx}: stably_named, obj_guid={obj_guid}, rep_layout={has_rep_layout}, content_bits={content_bits}, span={}..{}",
                pos_before,
                reader.position()
            );
            block_idx += 1;
            continue;
        }

        let is_deleted = match reader.read_bit() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: is_deleted error: {e}");
                break;
            }
        };

        if is_deleted {
            let flags = match reader.read_u8() {
                Ok(v) => v,
                Err(e) => {
                    println!("      block {block_idx}: delete_flags error: {e}");
                    break;
                }
            };
            println!(
                "      block {block_idx}: DELETED, obj_guid={obj_guid}, flags=0x{flags:02x}, span={}..{}",
                pos_before,
                reader.position()
            );
            block_idx += 1;
            continue;
        }

        let class_guid = match reader.read_int_packed() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: class_guid error: {e}");
                break;
            }
        };

        if class_guid == 0 {
            // Treated as deleted
            println!(
                "      block {block_idx}: implicit_deleted (class_guid=0), obj_guid={obj_guid}, span={}..{}",
                pos_before,
                reader.position()
            );
            block_idx += 1;
            continue;
        }

        let use_actor_outer = match reader.read_bit() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: use_actor_outer error: {e}");
                break;
            }
        };

        let outer_guid = if use_actor_outer {
            0
        } else {
            match reader.read_int_packed() {
                Ok(v) => v,
                Err(e) => {
                    println!("      block {block_idx}: outer_guid error: {e}");
                    break;
                }
            }
        };

        let content_bits = match reader.read_int_packed() {
            Ok(v) => v,
            Err(e) => {
                println!("      block {block_idx}: content_bits error: {e}");
                break;
            }
        };

        if content_bits > 0 {
            if (content_bits as u64) > reader.bits_remaining() {
                println!(
                    "      block {block_idx}: subobj, obj={obj_guid}, class={class_guid}, outer={outer_guid}, use_actor_outer={use_actor_outer}, rep_layout={has_rep_layout}, content_bits={content_bits} OVERRUN (remaining={})",
                    reader.bits_remaining()
                );
                break;
            }
            let _ = reader.skip_bits(content_bits as u64);
        }

        println!(
            "      block {block_idx}: subobj, obj={obj_guid}, class={class_guid}, outer={outer_guid}, use_actor_outer={use_actor_outer}, rep_layout={has_rep_layout}, content_bits={content_bits}, span={}..{}",
            pos_before,
            reader.position()
        );
        block_idx += 1;
    }
}

/// Try to frame content blocks starting at bit offset `start` in the payload.
fn try_content_blocks_at_offset(
    payload_data: &[u8],
    payload_bits: u64,
    start: u64,
) -> (u32, u64, u64, String) {
    let Ok(mut reader) = BitReader::with_bit_len(payload_data, payload_bits) else {
        return (
            0,
            0,
            payload_bits.saturating_sub(start),
            "declared payload length exceeds buffer".to_string(),
        );
    };
    if reader.skip_bits(start).is_err() {
        return (0, 0, payload_bits - start, "cannot seek".to_string());
    }

    let mut blocks = 0u32;

    loop {
        if reader.at_end() || reader.bits_remaining() == 0 {
            break;
        }

        // Try reading content block header
        let header_result = try_read_header(&mut reader);
        match header_result {
            Err(reason) => {
                let consumed = reader.position() - start;
                let remaining = payload_bits - reader.position();
                return (blocks, consumed, remaining, reason);
            }
            Ok(is_deleted) => {
                if is_deleted {
                    blocks += 1;
                    continue;
                }
            }
        }

        // Read content_bits as IntPacked
        let content_bits = match reader.read_int_packed() {
            Ok(v) => v,
            Err(e) => {
                let consumed = reader.position() - start;
                let remaining = payload_bits - reader.position();
                return (blocks, consumed, remaining, format!("content_bits: {e}"));
            }
        };

        if content_bits == 0 {
            blocks += 1;
            continue;
        }

        if (content_bits as u64) > reader.bits_remaining() {
            let consumed = reader.position() - start;
            let remaining = reader.bits_remaining();
            return (
                blocks,
                consumed,
                remaining,
                format!("overrun: content_bits={content_bits} > remaining={remaining}"),
            );
        }

        if reader.skip_bits(content_bits as u64).is_err() {
            let consumed = reader.position() - start;
            let remaining = reader.bits_remaining();
            return (blocks, consumed, remaining, "skip failed".to_string());
        }

        blocks += 1;
    }

    let remaining = payload_bits - reader.position();
    (blocks, reader.position() - start, remaining, String::new())
}

/// Try reading just IntPacked content_bits values repeatedly (no header).
fn try_pure_content_bits_at_offset(
    payload_data: &[u8],
    payload_bits: u64,
    start: u64,
) -> (u32, u64, u64, String) {
    let Ok(mut reader) = BitReader::with_bit_len(payload_data, payload_bits) else {
        return (
            0,
            0,
            payload_bits.saturating_sub(start),
            "declared payload length exceeds buffer".to_string(),
        );
    };
    if reader.skip_bits(start).is_err() {
        return (0, 0, payload_bits - start, "cannot seek".to_string());
    }

    let mut blocks = 0u32;

    loop {
        if reader.at_end() || reader.bits_remaining() == 0 {
            break;
        }

        let content_bits = match reader.read_int_packed() {
            Ok(v) => v,
            Err(e) => {
                let consumed = reader.position() - start;
                let remaining = payload_bits - reader.position();
                return (blocks, consumed, remaining, format!("read: {e}"));
            }
        };

        if content_bits == 0 {
            // terminator
            blocks += 1;
            continue;
        }

        if (content_bits as u64) > reader.bits_remaining() {
            let consumed = reader.position() - start;
            let remaining = reader.bits_remaining();
            return (
                blocks,
                consumed,
                remaining,
                format!("overrun: {content_bits}>{remaining}"),
            );
        }

        if reader.skip_bits(content_bits as u64).is_err() {
            let consumed = reader.position() - start;
            let remaining = reader.bits_remaining();
            return (blocks, consumed, remaining, "skip".to_string());
        }

        blocks += 1;
    }

    let remaining = payload_bits - reader.position();
    (blocks, reader.position() - start, remaining, String::new())
}

/// Try to read a content block header. Returns Ok(is_deleted).
fn try_read_header(reader: &mut BitReader<'_>) -> Result<bool, String> {
    let _has_rep_layout = reader
        .read_bit()
        .map_err(|e| format!("has_rep_layout: {e}"))?;
    let is_actor = reader.read_bit().map_err(|e| format!("is_actor: {e}"))?;
    if is_actor {
        return Ok(false);
    }

    let obj_guid = reader
        .read_int_packed()
        .map_err(|e| format!("obj_guid: {e}"))?;
    if obj_guid == 0 {
        // guid 0 is invalid, but the parser doesn't early-return here.
        // The real code continues with is_stably_named check.
    }

    let is_stably_named = reader
        .read_bit()
        .map_err(|e| format!("is_stably_named: {e}"))?;
    if is_stably_named {
        return Ok(false);
    }

    let is_deleted = reader.read_bit().map_err(|e| format!("is_deleted: {e}"))?;
    if is_deleted {
        let _flags = reader.read_u8().map_err(|e| format!("delete_flags: {e}"))?;
        return Ok(true);
    }

    let class_guid = reader
        .read_int_packed()
        .map_err(|e| format!("class_guid: {e}"))?;
    if class_guid == 0 {
        return Ok(true); // invalid class -> treated as deleted
    }

    let use_actor_outer = reader
        .read_bit()
        .map_err(|e| format!("use_actor_outer: {e}"))?;
    if !use_actor_outer {
        let _outer = reader
            .read_int_packed()
            .map_err(|e| format!("outer_guid: {e}"))?;
    }

    Ok(false)
}

struct BunchHeader {
    payload_bit_count: i32,
}

fn parse_bunch_header_full(reader: &mut BitReader<'_>) -> Result<BunchHeader, String> {
    let b_control = reader.read_bit().map_err(|e| format!("bControl: {e}"))?;

    let mut b_open = false;
    let mut b_close = false;

    if b_control {
        b_open = reader.read_bit().map_err(|e| format!("bOpen: {e}"))?;
        b_close = reader.read_bit().map_err(|e| format!("bClose: {e}"))?;
    }

    if b_close {
        // CloseReason: SerializedInt(15)
        let _close_reason = reader
            .read_serialized_int(15)
            .map_err(|e| format!("closeReason: {e}"))?;
    }

    // bIsReplicationPaused
    let _b_paused = reader.read_bit().map_err(|e| format!("bPaused: {e}"))?;

    let b_reliable = reader.read_bit().map_err(|e| format!("bReliable: {e}"))?;

    // Channel index: IntPacked (not serialized_int!)
    let _ch_index = reader
        .read_int_packed()
        .map_err(|e| format!("ch_index: {e}"))?;

    // bHasPackageMapExports
    let _pkg_map = reader.read_bit().map_err(|e| format!("bPkgMap: {e}"))?;

    // bHasMustBeMappedGUIDs
    let _must_mapped = reader.read_bit().map_err(|e| format!("bMustMapped: {e}"))?;

    // bPartial
    let b_partial = reader.read_bit().map_err(|e| format!("bPartial: {e}"))?;

    if b_partial {
        let _initial = reader
            .read_bit()
            .map_err(|e| format!("bPartialInit: {e}"))?;
        let _fin = reader
            .read_bit()
            .map_err(|e| format!("bPartialFinal: {e}"))?;
    }

    // VALORANT-specific bit (always present, always discarded)
    let _valorant_bit = reader
        .read_bit()
        .map_err(|e| format!("valorant_bit: {e}"))?;

    // Channel name (FName): present when reliable or opening
    if b_reliable || b_open {
        let _is_hardcoded = reader
            .read_bit()
            .map_err(|e| format!("fname_hardcoded: {e}"))?;
        let _name_index = reader
            .read_int_packed()
            .map_err(|e| format!("fname_index: {e}"))?;
    }

    // Payload bit count: SerializedInt(16384)
    let payload_bit_count = reader
        .read_serialized_int(16384)
        .map_err(|e| format!("payload: {e}"))?;

    Ok(BunchHeader {
        payload_bit_count: payload_bit_count as i32,
    })
}

fn compute_bit_size(data: &[u8], mut last_byte: u8) -> u64 {
    let byte_count = data.len() as u64;
    let mut bit_size = byte_count * 8 - 1;
    while (last_byte & 0x80) == 0 {
        last_byte <<= 1;
        bit_size -= 1;
    }
    bit_size
}
