//! `diag` subcommand -- a stats-only pass over the whole replay.
//!
//! `validate`'s failure diagnostics are capped twice (the 32-line
//! `ChannelState::stream_failures` window and the capped `NetStats`
//! diagnostics log): right for reading one replay, useless for counting a
//! population. `validate` walking checkpoints would move every counter its
//! pinned baselines hold, and an `export` without writes would still be the
//! export path. So `diag` drives the same sink over ReplayData and every
//! Checkpoint chunk, keeps the passes apart, writes no table, and emits one
//! JSON document aggregating every stream failure ([`FailureAggregate`]) by
//! kind, cause, group path, function count and handle. It prints no verdict
//! and exits 0 for any readable replay: judging is `validate`'s job.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use vrf_container::{decompress_checkpoint_with_trailing, parse_checkpoint_chunk, parse_preamble};
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::read_checkpoint_tables;

use crate::error::CliError;
use crate::pass::{Chunk, Pass, Replay, for_each_chunk};
use crate::sink::{ExportStats, FailureAggregate, MAX_FAILURE_CELLS};

/// The checkpoint pass's counters and per-chunk metadata.
#[derive(Debug, Default)]
struct DiagCheckpointStats {
    chunks: u64,
    frames: u64,
    frame_skips: FrameSkips,
    non_finite_frame_times: u64,
    packets: u64,
    /// Framing residual after each archive plus archive bytes the codec never
    /// read.
    trailing_bytes: u64,
    guid_entries: u64,
    group_records: u64,
    exported_fields: u64,
    /// Field rows the snapshot produced: counted, never written (`export
    /// --checkpoints` writes them to `checkpoint_fields.parquet`).
    field_rows_dropped: u64,
    actor_rows_dropped: u64,
    movement_rows_dropped: u64,
    net: NetStats,
    sink: ExportStats,
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
    let replay = Replay::new(&preamble);
    let branch = replay.branch;
    eprintln!("branch: {branch}");
    eprintln!("diag: walking ReplayData and Checkpoint chunks, writing no table...");

    let mut main = Pass::new(&replay)?;
    main.channels.enable_failure_aggregate(include_payloads);
    let mut replay_data_chunks: u64 = 0;
    let mut event_chunks: u64 = 0;
    let mut replay_data_trailing_bytes: u64 = 0;
    let mut sink_main = ExportStats::default();
    let mut cp_stats = DiagCheckpointStats {
        failures: FailureAggregate::new(include_payloads),
        ..DiagCheckpointStats::default()
    };

    let unknown_chunks = for_each_chunk(&data, &replay, |chunk| {
        match chunk {
            // Independent of replication, with no stream-failure signal.
            Chunk::Event(_) => event_chunks += 1,
            Chunk::Checkpoint(payload) => {
                process_checkpoint_chunk(payload, &replay, include_payloads, &mut cp_stats)?;
            }
            Chunk::ReplayData(frames, unread) => {
                replay_data_trailing_bytes += unread as u64;
                replay_data_chunks += 1;
                // Never drained: nothing is written, and each packet's sink clears them.
                main.walk(&frames, &mut sink_main, |_| Ok(()))?;
            }
        }
        Ok(())
    })?;

    main.finish();
    let net_main = main.reader.stats().clone();
    let main_failures = main.channels.take_failure_aggregate();
    let mut json = String::with_capacity(1 << 16);
    json.push_str("{\n  \"schema_version\": 4,\n  \"tool\": \"vrfkit diag\",\n  \"file\": ");
    push_json_string(&mut json, path);
    json.push_str(&format!(",\n  \"file_size\": {file_size},\n  \"branch\": "));
    push_json_string(&mut json, branch);
    json.push_str(",\n  \"build\": ");
    push_json_string(&mut json, build_label(branch));
    json.push_str(&format!(
        ",\n  \"options\": {{\"write_tables\": false, \"walks_checkpoints\": true, \
         \"include_payloads\": {include_payloads}}},\n"
    ));
    let (main_skips, cp_skips) = (&main.frame_skips, &cp_stats.frame_skips);
    let chunks = vec![
        ("replay_data", replay_data_chunks),
        ("replay_data_frames", u64::from(main.frames)),
        ("event", event_chunks),
        ("unknown", unknown_chunks),
        ("replay_data_trailing_bytes", replay_data_trailing_bytes),
        (
            "replay_data_external_data_blobs",
            main_skips.external_data_blobs,
        ),
        (
            "replay_data_external_data_bytes",
            main_skips.external_data_bytes,
        ),
        (
            "replay_data_game_specific_bytes",
            main_skips.game_specific_bytes,
        ),
        (
            "replay_data_non_finite_frame_times",
            main.non_finite_frame_times,
        ),
    ];
    let checkpoint_meta = vec![
        ("chunks", cp_stats.chunks),
        ("frames", cp_stats.frames),
        ("packets", cp_stats.packets),
        ("trailing_bytes", cp_stats.trailing_bytes),
        ("external_data_blobs", cp_skips.external_data_blobs),
        ("external_data_bytes", cp_skips.external_data_bytes),
        ("game_specific_bytes", cp_skips.game_specific_bytes),
        ("non_finite_frame_times", cp_stats.non_finite_frame_times),
        ("guid_entries", cp_stats.guid_entries),
        ("group_records", cp_stats.group_records),
        ("exported_fields", cp_stats.exported_fields),
        ("field_rows_dropped", cp_stats.field_rows_dropped),
        ("actor_rows_dropped", cp_stats.actor_rows_dropped),
        ("movement_rows_dropped", cp_stats.movement_rows_dropped),
    ];
    for (name, members) in [
        ("chunks", chunks),
        ("net_main", net_members(&net_main)),
        ("sink_main", sink_main.counters()),
        ("checkpoint_meta", checkpoint_meta),
        ("net_checkpoint", net_members(&cp_stats.net)),
        ("sink_checkpoint", cp_stats.sink.counters()),
    ] {
        json.push_str(&format!("  \"{name}\": "));
        push_members(&mut json, &members);
        json.push_str(",\n");
    }
    json.push_str("  \"failures\": {\n");
    json.push_str("    \"main\": ");
    push_failure_aggregate(&mut json, &main_failures, &net_main);
    json.push_str(",\n    \"checkpoint\": ");
    push_failure_aggregate(&mut json, &cp_stats.failures, &cp_stats.net);
    json.push_str("\n  }\n");
    json.push_str("}\n");

    match json_path {
        Some(out) => write_json_file(out, &json)?,
        None => println!("{json}"),
    }

    // A one-line stderr receipt wherever the JSON went, so a caller sees the
    // reconciliation without parsing JSON.
    eprintln!(
        "diag: main failures {} (payloads preserved {}, real loss {}, reconciled {}) | \
         checkpoint failures {} (payloads preserved {}, real loss {}, reconciled {})",
        main_failures.total_failures(),
        main_failures.preserved_unresolved(),
        main_failures.real_loss(),
        main_failures.reconciles(&net_main),
        cp_stats.failures.total_failures(),
        cp_stats.failures.preserved_unresolved(),
        cp_stats.failures.real_loss(),
        cp_stats.failures.reconciles(&cp_stats.net),
    );
    Ok(())
}

/// Refuse an output path that resolves to the replay itself: the JSON is built
/// in memory and written last, so a successful run could replace its source.
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
            let (parent, name) = json_parent_and_name(Path::new(path))?;
            Ok(fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

/// The directory `--json` goes in (`.` for a bare name) and its file name.
fn json_parent_and_name(path: &Path) -> Result<(&Path, &OsStr), CliError> {
    let name = path.file_name().ok_or_else(|| {
        CliError::Usage("--json requires a file path, not a directory".to_string())
    })?;
    Ok((usable_parent(path), name))
}

/// `path`'s directory, `.` for a bare name.
pub(crate) fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Publish JSON through a new sibling file rather than opening the destination
/// for truncation, so a `--json` that is a hard link to the replay replaces the
/// directory entry instead of writing through it.
fn write_json_file(path: &str, json: &str) -> Result<(), CliError> {
    let path = Path::new(path);
    let (parent, name) = json_parent_and_name(path)?;

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

    if let Err(error) = publish_json_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// Keep the prior report intact if the staged file cannot be published.
fn publish_json_file(staged: &Path, destination: &Path) -> std::io::Result<()> {
    // rename replaces an existing file on Windows too. Removing it first
    // would discard the prior complete report if the rename then failed.
    fs::rename(staged, destination)
}

/// Walk one Checkpoint chunk as `export --checkpoints` does, minus the
/// writers: a fresh GUID cache, export map, reader and channel state per
/// archive.
fn process_checkpoint_chunk(
    payload: &[u8],
    replay: &Replay<'_>,
    include_payloads: bool,
    cp: &mut DiagCheckpointStats,
) -> Result<(), CliError> {
    let cp_chunk = parse_checkpoint_chunk(payload)?;
    let (plain, unread) =
        decompress_checkpoint_with_trailing(cp_chunk.archive, replay.compressed, replay.encrypted)?;
    cp.trailing_bytes += (cp_chunk.trailing_bytes + unread) as u64;

    let mut pass = Pass::new(replay)?;
    pass.channels.enable_failure_aggregate(include_payloads);
    let tables = read_checkpoint_tables(&plain, &mut pass.cache)
        .map_err(|e| CliError::Usage(format!("checkpoint {}: {e}", cp_chunk.id)))?;
    pass.walk(&plain[tables.frame_offset..], &mut cp.sink, |buffers| {
        cp.field_rows_dropped += buffers.fields.len() as u64;
        cp.actor_rows_dropped += buffers.actors.len() as u64;
        cp.movement_rows_dropped += buffers.movement.len() as u64;
        Ok(())
    })?;
    pass.finish();
    cp.net.absorb(&mut pass.reader.stats().clone());
    cp.failures
        .absorb(&mut pass.channels.take_failure_aggregate());

    cp.chunks += 1;
    cp.frames += u64::from(pass.frames);
    cp.frame_skips.absorb(pass.frame_skips);
    cp.non_finite_frame_times += pass.non_finite_frame_times;
    cp.packets += u64::from(pass.packets);
    cp.guid_entries += u64::from(tables.guid_count);
    cp.group_records += u64::from(tables.group_count);
    cp.exported_fields += u64::from(tables.exported_fields);
    Ok(())
}

/// `++Ares-Core+release-13.04` -> `13.04`, the corpus label downstream; a
/// branch without the marker stays `unknown` rather than guessed.
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

/// `{`, one `"key": value` member per line, then `  }` -- the shape every flat
/// counter object in the diag JSON shares.
fn push_members(out: &mut String, members: &[(&str, u64)]) {
    out.push_str("{\n");
    for (i, (name, value)) in members.iter().enumerate() {
        let comma = if i + 1 < members.len() { "," } else { "" };
        out.push_str(&format!("    \"{name}\": {value}{comma}\n"));
    }
    out.push_str("  }");
}

/// Every counter, then the three `NetStats` derives: the partial-cause
/// residuals and `content_blocks_lost`.
fn net_members(s: &NetStats) -> Vec<(&'static str, u64)> {
    let mut members = s.counters();
    members.extend([
        (
            "partial_unclassified_errors",
            s.partial_unclassified_errors(),
        ),
        (
            "partial_overclassified_errors",
            s.partial_overclassified_errors(),
        ),
        ("content_blocks_lost", s.lost_content_blocks()),
    ]);
    members
}

fn push_failure_aggregate(out: &mut String, agg: &FailureAggregate, net: &NetStats) {
    let overflow = agg.overflow();
    out.push_str(&format!(
        "{{\"total_failures\": {}, \"preserved_unresolved\": {}, \"real_loss\": {}, \
         \"reconciled\": {}, \"cell_limit\": {}, \"overflow\": {{\"count\": {}, \
         \"bit_count_total\": {}, \"consumed_bits_total\": {}, \"abandoned_bits_total\": {}}}, \
         \"payloads_included\": {}, \"cells\": [",
        agg.total_failures(),
        agg.preserved_unresolved(),
        agg.real_loss(),
        agg.reconciles(net),
        MAX_FAILURE_CELLS,
        overflow.count,
        overflow.bit_count_total,
        overflow.consumed_bits_total,
        overflow.abandoned_bits_total,
        agg.retains_payloads(),
    ));
    for (i, (key, cell)) in agg.cells_sorted().iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"kind\": ");
        push_json_string(out, &format!("{:?}", key.kind));
        out.push_str(", \"cause\": ");
        push_json_string(out, &format!("{:?}", key.cause));
        out.push_str(", \"group_path\": ");
        push_json_string(out, &key.group_path);
        out.push_str(&format!(
            ", \"function_count\": {}, \"record_handle\": {}, \"consumed_bits\": {}, \
             \"payload_preserved\": {}, \"count\": {}, \"bit_count_total\": {}, \
             \"consumed_bits_total\": {}, \"abandoned_bits_total\": {}, \"samples\": [",
            key.function_count,
            json_number_or_null(key.record_handle),
            key.consumed_bits,
            key.payload_preserved,
            cell.count,
            cell.bit_count_total,
            cell.consumed_bits_total,
            cell.abandoned_bits_total,
        ));
        for (j, sample) in cell.samples.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!(
                "{{\"actor_net_guid\": {}, \"bit_count\": {}, \"abandoned_bits\": {}, \
                 \"record_offset\": {}, \"payload_hex\": ",
                sample.actor_net_guid,
                sample.bit_count,
                sample.abandoned_bits,
                json_number_or_null(sample.record_offset),
            ));
            match &sample.payload_hex {
                Some(hex) => push_json_string(out, hex),
                None => out.push_str("null"),
            }
            out.push_str(&format!(
                ", \"payload_truncated\": {}}}",
                sample.payload_truncated
            ));
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

fn json_number_or_null<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        NetStats, build_label, json_number_or_null, net_members, publish_json_file,
        push_json_string, reject_input_output_alias, write_json_file,
    };

    /// Each counter under its own name with its own value, then the three
    /// derives, computed from the same stats.
    #[test]
    fn net_members_are_every_counter_then_the_derives() {
        let mut stats = NetStats::default();
        for (i, (_, value)) in stats.counters_mut().into_iter().enumerate() {
            *value = 3 * i as u64 + 1;
        }
        let mut expected = stats.counters();
        expected.extend([
            (
                "partial_unclassified_errors",
                stats.partial_unclassified_errors(),
            ),
            (
                "partial_overclassified_errors",
                stats.partial_overclassified_errors(),
            ),
            ("content_blocks_lost", stats.lost_content_blocks()),
        ]);
        assert_eq!(net_members(&stats), expected);
        assert_eq!(expected.len(), 49);
    }

    #[test]
    fn build_labels_come_from_the_release_marker() {
        assert_eq!(build_label("++Ares-Core+release-13.01"), "13.01");
        assert_eq!(build_label("++Ares-Core+release-13.05"), "13.05");
        assert_eq!(build_label("++Ares-Core+dev"), "unknown");
    }

    /// A replay-declared path is the only free-form text this emitter writes.
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

    /// No replay has produced a failure cell with a record handle, so the
    /// present branch is pinned here rather than by any real output.
    #[test]
    fn optional_numbers_render_as_the_number_or_null() {
        assert_eq!(json_number_or_null(Some(7u32)), "7");
        assert_eq!(json_number_or_null(None::<u64>), "null");
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

    #[test]
    fn failed_json_publication_preserves_the_prior_report() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vrfkit-diag-failed-publication-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let output = dir.join("report.json");
        let staged = dir.join("staged.tmp");
        std::fs::write(&output, b"previous complete report").unwrap();
        std::fs::write(&staged, b"new complete report").unwrap();
        // Fault injection at the publication boundary, without a timing race.
        std::fs::remove_file(&staged).unwrap();

        let error = publish_json_file(&staged, &output).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            std::fs::read(&output).ok().as_deref(),
            Some(b"previous complete report".as_slice())
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
