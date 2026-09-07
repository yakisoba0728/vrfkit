//! `diag` subcommand -- a stats-only pass over the whole replay.
//!
//! # Why this exists, and why not one of the two existing commands
//!
//! The failure diagnostics `validate` prints are capped twice -- the 32-line
//! `ChannelState::stream_failures` window and the capped `NetStats::diagnostics`
//! log -- which is right for a human reading one replay and useless for
//! counting a population. The bounded [`FailureAggregate`] in the sink
//! collects every failure; something has to walk a replay and read it. Two
//! candidates were rejected:
//!
//! - **`validate` walking checkpoints too** would move every counter that
//!   command's pinned baselines hold and add failure paths to a command whose
//!   job is to report, not to abort (see `oracle.rs`'s `checkpoint_scope_note`
//!   for the full argument). The oracle's scope stays exactly what it was.
//! - **`export` skipping the Parquet writes** would still be the export path:
//!   writer threads, staged output directories, a manifest, an atomic publish.
//!   None of that carries a failure counter, and a flag that quietly produces
//!   no files from the command whose contract is "writes the files" is the
//!   shape of bug this repo refuses.
//!
//! So `diag` is its own subcommand that **writes no table**: it drives the
//! same sink over the ReplayData stream and every Checkpoint chunk, keeps the
//! main and checkpoint passes separate, and emits one JSON document
//! aggregating every stream failure by kind, cause, group path, function
//! count and handle. No Parquet file is created and none is needed -- every
//! number this command reports comes from `NetStats`, the sink's own
//! counters and the failure aggregate, all of which exist before any writer.
//!
//! # What it deliberately does not do
//!
//! It prints no verdict and exits 0 for any replay it could read. Judging a
//! replay is `validate`'s job, and a second oracle would drift from the
//! first. A file that cannot be read at all is an error, as everywhere else.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use vrf_container::{
    ChunkIterator, ChunkType, decompress_checkpoint, decompress_replay_data_with_trailing,
    parse_checkpoint_chunk, parse_preamble,
};
use vrf_decode::{ArrayDecodeStats, OverlayStats};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_net::stats::NetStats;
use vrf_schema::{NetGuidCache, read_checkpoint_tables};

use crate::error::CliError;
use crate::sink::{ChannelState, ExportSink, FailureAggregate, RecordBuffers};

/// Every sink counter the diag JSON carries, summed across packets.
///
/// Mirrors what `driver::totals::SinkTotals` accumulates, deliberately
/// re-declared here: that struct lives in the `export`-gated driver and is
/// shaped for the summary's printer. This one is the subset the corpus
/// baseline (`quality_totals.json`'s `sink.*` keys) can reconcile against,
/// and it is read only by this module. If a counter is added to
/// `ExportStats` and matters here, it has to be added to `absorb` below --
/// the per-file totals would then read zero against a live counter, which is
/// the loudest way a stale list can fail.
#[derive(Debug, Default)]
struct DiagSinkTotals {
    fields_emitted: u64,
    rpcs_emitted: u64,
    actor_opens: u64,
    actor_closes: u64,
    content_blocks: u64,
    overlay: OverlayStats,
    effect_blobs_decoded: u64,
    struct_blobs_decoded: u64,
    struct_blobs_failed: u64,
    multi_contents_items_emitted: u64,
    movement_rpc_errors: u64,
    array: ArrayDecodeStats,
    array_leaf_decode_errors: u64,
    truncated_rpcs: u64,
    rpc_suffix_bits_dropped: u64,
    cnc_rpcs_emitted: u64,
    rep_layout_cnc_tails_decoded: u64,
    rep_layout_cnc_tails_preserved: u64,
}

impl DiagSinkTotals {
    fn absorb(&mut self, stats: &mut crate::sink::ExportStats) {
        self.fields_emitted += stats.fields_emitted;
        self.rpcs_emitted += stats.rpcs_emitted;
        self.actor_opens += stats.actor_opens;
        self.actor_closes += stats.actor_closes;
        self.content_blocks += stats.content_blocks;
        self.overlay.decoded_ok += stats.overlay.decoded_ok;
        self.overlay.decoded_err += stats.overlay.decoded_err;
        self.overlay.raw_or_skip += stats.overlay.raw_or_skip;
        self.overlay.not_in_table += stats.overlay.not_in_table;
        self.overlay.no_field_name += stats.overlay.no_field_name;
        self.overlay.handle_conflicts_refused += stats.overlay.handle_conflicts_refused;
        self.effect_blobs_decoded += stats.effect_blobs_decoded;
        self.struct_blobs_decoded += stats.struct_blobs_decoded;
        self.struct_blobs_failed += stats.struct_blobs_failed;
        self.multi_contents_items_emitted += stats.multi_contents_items_emitted;
        self.movement_rpc_errors += stats.movement_rpc_errors;
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
        self.rep_layout_cnc_tails_decoded += stats.rep_layout_cnc_tails_decoded;
        self.rep_layout_cnc_tails_preserved += stats.rep_layout_cnc_tails_preserved;
    }
}

/// Per-checkpoint-chunk metadata the JSON reports alongside the checkpoint
/// pass's counters, so the checkpoint walk is auditable rather than a black
/// box that printed a number.
#[derive(Debug, Default)]
struct DiagCheckpointStats {
    chunks: u64,
    frames: u64,
    packets: u64,
    trailing_bytes: u64,
    guid_entries: u64,
    group_records: u64,
    exported_fields: u64,
    /// Field rows the snapshot produced. Counted and dropped, never written:
    /// same policy as `export --checkpoints`, which writes them to
    /// `checkpoint_fields.parquet`. A diag run that wrote them would be
    /// creating the Parquet this command exists to avoid.
    field_rows_dropped: u64,
    actor_rows_dropped: u64,
    movement_rows_dropped: u64,
    net: NetStats,
    sink: DiagSinkTotals,
    failures: FailureAggregate,
}

/// Run the diag pass over one replay, writing the aggregate JSON to
/// `json_path` when given, to stdout otherwise.
pub fn run(path: &str, json_path: Option<&str>, include_payloads: bool) -> Result<(), CliError> {
    if let Some(output) = json_path {
        reject_input_output_alias(path, output)?;
    }
    eprintln!("reading {path}...");
    let data = fs::read(path)?;
    let file_size = data.len();
    let preamble = parse_preamble(&data)?;
    let branch = preamble.header.replay_version.branch.clone();
    let flags = preamble.header.flags;
    let compressed = preamble.info.compressed;
    let encrypted = preamble.info.encrypted;
    eprintln!("branch: {branch}");
    eprintln!("diag: walking ReplayData and Checkpoint chunks, writing no table...");

    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(&branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    let mut total_packets: u32 = 0;
    let mut replay_data_chunks: u64 = 0;
    let mut event_chunks: u64 = 0;
    let mut replay_data_trailing_bytes: u64 = 0;
    let mut sink_totals = DiagSinkTotals::default();
    let mut channel_state = ChannelState::new();
    channel_state.enable_failure_aggregate(include_payloads);
    let mut buffers = RecordBuffers::default();

    let mut cp_stats = DiagCheckpointStats {
        failures: FailureAggregate::new(include_payloads),
        ..DiagCheckpointStats::default()
    };

    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    while let Some(chunk) = chunk_iter.next_chunk()? {
        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        match chunk.chunk_type {
            ChunkType::Event => {
                // The server timeline is independent of the replication pass
                // and carries no stream-failure signal; counted, not parsed.
                event_chunks += 1;
            }
            ChunkType::Checkpoint => {
                process_checkpoint_chunk(
                    payload,
                    &branch,
                    flags,
                    compressed,
                    encrypted,
                    include_payloads,
                    &mut cp_stats,
                )?;
            }
            ChunkType::ReplayData => {
                let (decompressed, trailing) =
                    decompress_replay_data_with_trailing(payload, compressed, encrypted)?;
                replay_data_trailing_bytes += trailing as u64;
                replay_data_chunks += 1;
                iter_demo_frames(&decompressed, flags, &mut cache, |pkt, packet_cache| {
                    let pkt_id = total_packets;
                    total_packets += 1;
                    {
                        let mut sink =
                            ExportSink::new(packet_cache, &mut channel_state, &mut buffers);
                        sink.time_ms = pkt.time_ms;
                        sink.packet_id = pkt_id;
                        repl_reader.process_packet(pkt.data, pkt_id as i32, &mut sink);
                        sink_totals.absorb(&mut sink.stats);
                    }
                    // The records are dropped, not written; the counters they
                    // produced were already absorbed above. Draining keeps the
                    // buffers from growing to the largest packet's worth of
                    // rows times every packet after a big one.
                    buffers.fields.clear();
                    buffers.movement.clear();
                    buffers.actors.clear();
                })?;
            }
            other => {
                // `Unknown(u32)` and `Header` -- nothing the replication pass
                // reads, and nothing a counter could claim to check.
                let _ = other;
            }
        }
    }

    repl_reader.finish();
    let net_main = repl_reader.stats().clone();
    let main_failures = channel_state.take_failure_aggregate();
    let mut json = String::with_capacity(1 << 16);
    json.push_str("{\n");
    json.push_str("  \"schema_version\": 2,\n");
    json.push_str("  \"tool\": \"vrfkit diag\",\n");
    json.push_str("  \"file\": ");
    push_json_string(&mut json, path);
    json.push_str(",\n");
    json.push_str(&format!("  \"file_size\": {file_size},\n"));
    json.push_str("  \"branch\": ");
    push_json_string(&mut json, &branch);
    json.push_str(",\n");
    json.push_str("  \"build\": ");
    push_json_string(&mut json, build_label(&branch));
    json.push_str(",\n");
    json.push_str(
        "  \"options\": {\"write_tables\": false, \"walks_checkpoints\": true, \
         \"include_payloads\": ",
    );
    json.push_str(if include_payloads { "true" } else { "false" });
    json.push_str("},\n");
    json.push_str("  \"chunks\": {\"replay_data\": ");
    json.push_str(&replay_data_chunks.to_string());
    json.push_str(", \"event\": ");
    json.push_str(&event_chunks.to_string());
    json.push_str(", \"replay_data_trailing_bytes\": ");
    json.push_str(&replay_data_trailing_bytes.to_string());
    json.push_str("},\n");

    json.push_str("  \"net_main\": ");
    push_net_stats(&mut json, &net_main);
    json.push_str(",\n");
    json.push_str("  \"sink_main\": ");
    push_sink_totals(&mut json, &sink_totals);
    json.push_str(",\n");
    json.push_str("  \"checkpoint_meta\": {\"chunks\": ");
    json.push_str(&cp_stats.chunks.to_string());
    json.push_str(", \"frames\": ");
    json.push_str(&cp_stats.frames.to_string());
    json.push_str(", \"packets\": ");
    json.push_str(&cp_stats.packets.to_string());
    json.push_str(", \"trailing_bytes\": ");
    json.push_str(&cp_stats.trailing_bytes.to_string());
    json.push_str(", \"guid_entries\": ");
    json.push_str(&cp_stats.guid_entries.to_string());
    json.push_str(", \"group_records\": ");
    json.push_str(&cp_stats.group_records.to_string());
    json.push_str(", \"exported_fields\": ");
    json.push_str(&cp_stats.exported_fields.to_string());
    json.push_str(", \"field_rows_dropped\": ");
    json.push_str(&cp_stats.field_rows_dropped.to_string());
    json.push_str(", \"actor_rows_dropped\": ");
    json.push_str(&cp_stats.actor_rows_dropped.to_string());
    json.push_str(", \"movement_rows_dropped\": ");
    json.push_str(&cp_stats.movement_rows_dropped.to_string());
    json.push_str("},\n");
    json.push_str("  \"net_checkpoint\": ");
    push_net_stats(&mut json, &cp_stats.net);
    json.push_str(",\n");
    json.push_str("  \"sink_checkpoint\": ");
    push_sink_totals(&mut json, &cp_stats.sink);
    json.push_str(",\n");

    json.push_str("  \"failures\": {\n");
    json.push_str("    \"main\": ");
    push_failure_aggregate(&mut json, &main_failures);
    json.push_str(",\n    \"checkpoint\": ");
    push_failure_aggregate(&mut json, &cp_stats.failures);
    json.push_str("\n  }\n");
    json.push_str("}\n");

    match json_path {
        Some(out) => write_json_file(out, &json)?,
        None => println!("{json}"),
    }

    // A short stdout receipt even when the JSON went to a file, so a caller
    // scanning output sees the reconciliation shape without parsing JSON.
    eprintln!(
        "diag: main failures {} (payloads preserved {}, real loss {}) | \
         checkpoint failures {} (payloads preserved {}, real loss {})",
        main_failures.total_failures(),
        main_failures.preserved_unresolved(),
        main_failures.real_loss(),
        cp_stats.failures.total_failures(),
        cp_stats.failures.preserved_unresolved(),
        cp_stats.failures.real_loss(),
    );
    Ok(())
}

/// Refuse an output path that resolves to the replay itself. The diagnostic is
/// assembled in memory and written last, so without this check a successful
/// run could replace its own source with JSON.
fn reject_input_output_alias(input: &str, output: &str) -> Result<(), CliError> {
    let input = fs::canonicalize(input)?;
    let output = canonicalize_destination(output)?;
    if input == output {
        return Err(CliError::Usage(
            "--json output must differ from the input replay".to_string(),
        ));
    }
    Ok(())
}

fn canonicalize_destination(path: &str) -> Result<PathBuf, CliError> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let path = Path::new(path);
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let name = path.file_name().ok_or_else(|| {
                CliError::Usage("--json requires a file path, not a directory".to_string())
            })?;
            Ok(fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

/// Publish JSON through a new sibling file rather than opening the destination
/// inode for truncation. This keeps the replay intact when a differently named
/// hard link is supplied as `--json`; replacing that directory entry detaches
/// the link instead of writing through it.
fn write_json_file(path: &str, json: &str) -> Result<(), CliError> {
    let path = Path::new(path);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        CliError::Usage("--json requires a file path, not a directory".to_string())
    })?;

    let mut created = None;
    for attempt in 0..64u32 {
        let temp = parent.join(format!(
            ".{}.vrfkit-{}-{attempt}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => {
                created = Some((file, temp));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let Some((mut file, temp)) = created else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve a temporary diagnostic output file",
        )
        .into());
    };

    if let Err(error) = file
        .write_all(json.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    drop(file);

    #[cfg(windows)]
    if path.try_exists()? {
        if let Err(error) = fs::remove_file(path) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
    }
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// Walk one Checkpoint chunk the way `driver::checkpoints::process_chunk`
/// does, minus the writer: fresh GUID cache, export map, reader and channel
/// state per archive, because a checkpoint is an independent replay state.
fn process_checkpoint_chunk(
    payload: &[u8],
    branch: &str,
    flags: u32,
    compressed: bool,
    encrypted: bool,
    include_payloads: bool,
    cp: &mut DiagCheckpointStats,
) -> Result<(), CliError> {
    let cp_chunk = parse_checkpoint_chunk(payload)?;
    cp.trailing_bytes += cp_chunk.trailing_bytes as u64;
    let plain = decompress_checkpoint(cp_chunk.archive, compressed, encrypted)?;

    let mut cache = NetGuidCache::new();
    let tables = read_checkpoint_tables(&plain, &mut cache)
        .map_err(|e| CliError::Usage(format!("checkpoint {}: {e}", cp_chunk.id)))?;

    let frame = &plain[tables.frame_offset..];
    let mut reader = ReplicationReader::new(branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;
    let mut channels = ChannelState::new();
    channels.enable_failure_aggregate(include_payloads);
    let mut buffers = RecordBuffers::default();
    let mut packet_count = 0u64;
    // No writer here, so unlike `driver::checkpoints::process_chunk` there is
    // no error a callback cannot propagate: the closure is infallible and the
    // frame walk returns its own errors through `?`.
    let (_, frame_count) = iter_demo_frames(frame, flags, &mut cache, |pkt, packet_cache| {
        {
            let mut sink = ExportSink::new(packet_cache, &mut channels, &mut buffers);
            sink.time_ms = pkt.time_ms;
            sink.packet_id = packet_count as u32;
            reader.process_packet(pkt.data, packet_count as i32, &mut sink);
            cp.sink.absorb(&mut sink.stats);
        }
        cp.field_rows_dropped += buffers.fields.len() as u64;
        cp.actor_rows_dropped += buffers.actors.len() as u64;
        cp.movement_rows_dropped += buffers.movement.len() as u64;
        buffers.fields.clear();
        buffers.actors.clear();
        buffers.movement.clear();
        packet_count += 1;
    })?;
    reader.finish();
    let mut chunk_net = reader.stats().clone();
    cp.net.absorb(&mut chunk_net);
    let mut chunk_failures = channels.take_failure_aggregate();
    cp.failures.absorb(&mut chunk_failures);

    cp.chunks += 1;
    cp.frames += frame_count as u64;
    cp.packets += packet_count;
    cp.guid_entries += u64::from(tables.guid_count);
    cp.group_records += u64::from(tables.group_count);
    cp.exported_fields += u64::from(tables.exported_fields);
    Ok(())
}

/// `++Ares-Core+release-13.04` -> `13.04`. The corpus is labelled by this
/// suffix everywhere downstream; a branch that does not carry it stays
/// unlabelled rather than guessed.
fn build_label(branch: &str) -> &str {
    branch
        .rsplit("release-")
        .next()
        .filter(|rest| !rest.is_empty() && *rest != branch)
        .unwrap_or("unknown")
}

/// Append `s` as a quoted, escaped JSON string. Characters outside printable
/// ASCII are emitted as UTF-16 JSON escapes, including surrogate pairs, so the
/// Windows console never receives raw non-ASCII text.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for byte in s.chars() {
        match byte {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7E => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_net_stats(out: &mut String, s: &NetStats) {
    let pairs: Vec<(&str, String)> = vec![
        ("packets", s.packets.to_string()),
        ("malformed_packets", s.malformed_packets.to_string()),
        ("bunches", s.bunches.to_string()),
        ("partial_errors", s.partial_errors.to_string()),
        ("partial_fragments", s.partial_fragments.to_string()),
        ("partial_completed", s.partial_completed.to_string()),
        ("unfinished_partials", s.unfinished_partials.to_string()),
        (
            "unfinished_partial_bits",
            s.unfinished_partial_bits.to_string(),
        ),
        ("bunch_header_failures", s.bunch_header_failures.to_string()),
        ("content_blocks", s.content_blocks.to_string()),
        ("rep_layout_blocks", s.rep_layout_blocks.to_string()),
        (
            "class_net_cache_blocks",
            s.class_net_cache_blocks.to_string(),
        ),
        ("deleted_blocks", s.deleted_blocks.to_string()),
        ("fields", s.fields.to_string()),
        ("rpcs", s.rpcs.to_string()),
        ("skipped_bits", s.skipped_bits.to_string()),
        (
            "content_block_framing_failures",
            s.content_block_framing_failures.to_string(),
        ),
        (
            "malformed_content_blocks",
            s.malformed_content_blocks.to_string(),
        ),
        ("transform_failures", s.transform_failures.to_string()),
        ("field_stream_failures", s.field_stream_failures.to_string()),
        ("rpc_stream_failures", s.rpc_stream_failures.to_string()),
        (
            "unresolved_rpc_payloads_preserved",
            s.unresolved_rpc_payloads_preserved.to_string(),
        ),
        ("actor_opens", s.actor_opens.to_string()),
        ("actor_closes", s.actor_closes.to_string()),
        (
            "channel_reopens_while_open",
            s.channel_reopens_while_open.to_string(),
        ),
        (
            "actor_opens_missing_spawn",
            s.actor_opens_missing_spawn.to_string(),
        ),
        (
            "channel_state_limit_failures",
            s.channel_state_limit_failures.to_string(),
        ),
        (
            "partial_resource_limit_failures",
            s.partial_resource_limit_failures.to_string(),
        ),
        ("package_map_exports", s.package_map_exports.to_string()),
        (
            "rep_layout_export_bunches",
            s.rep_layout_export_bunches.to_string(),
        ),
        ("exported_guids", s.exported_guids.to_string()),
        ("must_be_mapped_guids", s.must_be_mapped_guids.to_string()),
        ("content_blocks_lost", s.lost_content_blocks().to_string()),
    ];
    out.push_str("{\n");
    for (i, (name, value)) in pairs.iter().enumerate() {
        out.push_str("    \"");
        out.push_str(name);
        out.push_str("\": ");
        out.push_str(value);
        if i + 1 < pairs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  }");
}

fn push_sink_totals(out: &mut String, s: &DiagSinkTotals) {
    let pairs: Vec<(&str, String)> = vec![
        ("fields_emitted", s.fields_emitted.to_string()),
        ("rpcs_emitted", s.rpcs_emitted.to_string()),
        ("actor_opens", s.actor_opens.to_string()),
        ("actor_closes", s.actor_closes.to_string()),
        ("content_blocks", s.content_blocks.to_string()),
        ("overlay_decoded_ok", s.overlay.decoded_ok.to_string()),
        ("overlay_decoded_err", s.overlay.decoded_err.to_string()),
        ("overlay_raw_or_skip", s.overlay.raw_or_skip.to_string()),
        ("overlay_not_in_table", s.overlay.not_in_table.to_string()),
        ("overlay_no_field_name", s.overlay.no_field_name.to_string()),
        (
            "overlay_handle_conflicts_refused",
            s.overlay.handle_conflicts_refused.to_string(),
        ),
        ("effect_blobs_decoded", s.effect_blobs_decoded.to_string()),
        ("struct_blobs_decoded", s.struct_blobs_decoded.to_string()),
        ("struct_blobs_failed", s.struct_blobs_failed.to_string()),
        (
            "multi_contents_items_emitted",
            s.multi_contents_items_emitted.to_string(),
        ),
        ("movement_rpc_errors", s.movement_rpc_errors.to_string()),
        (
            "array_elements_decoded",
            s.array.elements_decoded.to_string(),
        ),
        ("array_fields_emitted", s.array.fields_emitted.to_string()),
        ("array_truncations", s.array.truncations.to_string()),
        ("array_errors", s.array.errors.to_string()),
        (
            "array_unconsumed_nested_bits",
            s.array.unconsumed_nested_bits.to_string(),
        ),
        (
            "array_implicit_terminations",
            s.array.implicit_terminations.to_string(),
        ),
        (
            "array_unconsumed_root_bits",
            s.array.unconsumed_root_bits.to_string(),
        ),
        (
            "array_leaf_decode_errors",
            s.array_leaf_decode_errors.to_string(),
        ),
        ("truncated_rpcs", s.truncated_rpcs.to_string()),
        (
            "rpc_suffix_bits_dropped",
            s.rpc_suffix_bits_dropped.to_string(),
        ),
        ("cnc_rpcs_emitted", s.cnc_rpcs_emitted.to_string()),
        (
            "rep_layout_cnc_tails_decoded",
            s.rep_layout_cnc_tails_decoded.to_string(),
        ),
        (
            "rep_layout_cnc_tails_preserved",
            s.rep_layout_cnc_tails_preserved.to_string(),
        ),
    ];
    out.push_str("{\n");
    for (i, (name, value)) in pairs.iter().enumerate() {
        out.push_str("    \"");
        out.push_str(name);
        out.push_str("\": ");
        out.push_str(value);
        if i + 1 < pairs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  }");
}

fn push_failure_aggregate(out: &mut String, agg: &FailureAggregate) {
    out.push_str("{\"total_failures\": ");
    out.push_str(&agg.total_failures().to_string());
    out.push_str(", \"preserved_unresolved\": ");
    out.push_str(&agg.preserved_unresolved().to_string());
    out.push_str(", \"real_loss\": ");
    out.push_str(&agg.real_loss().to_string());
    out.push_str(", \"cell_limit\": ");
    out.push_str(&FailureAggregate::cell_limit().to_string());
    out.push_str(", \"overflow\": {\"count\": ");
    out.push_str(&agg.overflow().count.to_string());
    out.push_str(", \"bit_count_total\": ");
    out.push_str(&agg.overflow().bit_count_total.to_string());
    out.push_str(", \"consumed_bits_total\": ");
    out.push_str(&agg.overflow().consumed_bits_total.to_string());
    out.push_str(", \"abandoned_bits_total\": ");
    out.push_str(&agg.overflow().abandoned_bits_total.to_string());
    out.push('}');
    out.push_str(", \"payloads_included\": ");
    out.push_str(if agg.retains_payloads() {
        "true"
    } else {
        "false"
    });
    out.push_str(", \"cells\": [");
    for (i, (key, cell)) in agg.cells_sorted().iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"kind\": ");
        push_json_string(out, kind_name(key.kind));
        out.push_str(", \"cause\": ");
        push_json_string(out, cause_name(key.cause));
        out.push_str(", \"group_path\": ");
        push_json_string(out, &key.group_path);
        out.push_str(", \"function_count\": ");
        out.push_str(&key.function_count.to_string());
        out.push_str(", \"record_handle\": ");
        match key.record_handle {
            Some(handle) => out.push_str(&handle.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(", \"consumed_bits\": ");
        out.push_str(&key.consumed_bits.to_string());
        out.push_str(", \"payload_preserved\": ");
        out.push_str(if key.payload_preserved {
            "true"
        } else {
            "false"
        });
        out.push_str(", \"count\": ");
        out.push_str(&cell.count.to_string());
        out.push_str(", \"bit_count_total\": ");
        out.push_str(&cell.bit_count_total.to_string());
        out.push_str(", \"consumed_bits_total\": ");
        out.push_str(&cell.consumed_bits_total.to_string());
        out.push_str(", \"abandoned_bits_total\": ");
        out.push_str(&cell.abandoned_bits_total.to_string());
        out.push_str(", \"samples\": [");
        for (j, sample) in cell.samples.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            out.push_str("{\"actor_net_guid\": ");
            out.push_str(&sample.actor_net_guid.to_string());
            out.push_str(", \"bit_count\": ");
            out.push_str(&sample.bit_count.to_string());
            out.push_str(", \"consumed_bits\": ");
            out.push_str(&sample.consumed_bits.to_string());
            out.push_str(", \"payload_preserved\": ");
            out.push_str(if sample.payload_preserved {
                "true"
            } else {
                "false"
            });
            out.push_str(", \"abandoned_bits\": ");
            out.push_str(&sample.abandoned_bits.to_string());
            out.push_str(", \"record_offset\": ");
            match sample.record_offset {
                Some(offset) => out.push_str(&offset.to_string()),
                None => out.push_str("null"),
            }
            out.push_str(", \"payload_hex\": ");
            match &sample.payload_hex {
                Some(hex) => push_json_string(out, hex),
                None => out.push_str("null"),
            }
            out.push_str(", \"payload_truncated\": ");
            out.push_str(if sample.payload_truncated {
                "true"
            } else {
                "false"
            });
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

fn kind_name(kind: vrf_net::pipeline::StreamKind) -> &'static str {
    match kind {
        vrf_net::pipeline::StreamKind::RepLayout => "RepLayout",
        vrf_net::pipeline::StreamKind::Rpc => "Rpc",
    }
}

fn cause_name(cause: vrf_net::pipeline::StreamFailureCause) -> &'static str {
    match cause {
        vrf_net::pipeline::StreamFailureCause::AbandonedTail => "AbandonedTail",
        vrf_net::pipeline::StreamFailureCause::ReadError => "ReadError",
        vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount => "UnresolvedFunctionCount",
        vrf_net::pipeline::StreamFailureCause::UnverifiedRepLayoutTail => "UnverifiedRepLayoutTail",
        vrf_net::pipeline::StreamFailureCause::WindowOpenFailed => "WindowOpenFailed",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiagSinkTotals, build_label, push_json_string, reject_input_output_alias, write_json_file,
    };
    use crate::sink::ExportStats;

    /// The branch-to-build label the corpus aggregation joins on. A branch
    /// without the `release-` marker stays unlabelled rather than guessed.
    #[test]
    fn build_labels_come_from_the_release_marker() {
        assert_eq!(build_label("++Ares-Core+release-13.01"), "13.01");
        assert_eq!(build_label("++Ares-Core+release-13.05"), "13.05");
        assert_eq!(build_label("++Ares-Core+dev"), "unknown");
    }

    /// Escaping must keep the JSON one-document-parseable: quotes, backslash
    /// and control bytes never terminate the string early. A replay-declared
    /// path is the only free-form text this emitter writes.
    #[test]
    fn json_strings_escape_terminators_and_control_bytes() {
        let mut out = String::new();
        push_json_string(&mut out, "a\"b\\c\nd\u{1}e<f&>");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001e<f&>\"");
    }

    #[test]
    fn json_strings_encode_non_bmp_as_surrogate_pairs() {
        let mut out = String::new();
        push_json_string(&mut out, "x\u{1f600}y");
        assert_eq!(out, "\"x\\ud83d\\ude00y\"");
    }

    #[test]
    fn json_output_cannot_alias_the_input() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/diagnose.rs");
        let source = source.to_str().unwrap();
        let error = reject_input_output_alias(source, source).unwrap_err();
        assert!(error.to_string().contains("must differ"));
    }

    #[test]
    fn json_output_replaces_a_hardlink_without_truncating_its_source() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vrfkit-diag-hardlink-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("source.vrf");
        let output = dir.join("report.json");
        std::fs::write(&source, b"original replay bytes").unwrap();
        std::fs::hard_link(&source, &output).unwrap();

        write_json_file(output.to_str().unwrap(), "{\"ok\":true}\n").unwrap();

        assert_eq!(std::fs::read(&source).unwrap(), b"original replay bytes");
        assert_eq!(std::fs::read(&output).unwrap(), b"{\"ok\":true}\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Every counter a fresh sink holds must survive the absorb as zero -- a
    /// line here that stops compiling when `ExportStats` grows is fine, but a
    /// field that silently stops being read would print zeros forever.
    #[test]
    fn absorbing_a_default_sink_keeps_every_counter_at_zero() {
        let mut totals = DiagSinkTotals::default();
        let mut stats = ExportStats::default();
        stats.overlay.decoded_ok = 5;
        stats.array.errors = 2;
        totals.absorb(&mut stats);
        let mut stats2 = ExportStats::default();
        stats2.overlay.decoded_ok = 7;
        stats2.array.errors = 0;
        totals.absorb(&mut stats2);
        assert_eq!(totals.overlay.decoded_ok, 12);
        assert_eq!(totals.array.errors, 2);
        assert_eq!(totals.fields_emitted, 0);
    }
}
