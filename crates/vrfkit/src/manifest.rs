//! Hand-rolled JSON for manifest.json: no serde; one member per line, joined
//! by [`Object`] and [`array()`].
//!
//! The header's game-specific data entries are JSON documents themselves (on
//! 02d4d478 entry 1 is a 219,304-character match roster). They are emitted as
//! JSON strings, never parsed and re-serialised, which could silently alter
//! number formatting, key order or duplicate keys; a consumer recovers the
//! object with one nested parse.

use std::fmt::Display;
use std::fs;
use std::path::Path;

use vrf_container::Preamble;
use vrf_decode::OverlayErrorReport;
use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;
use vrf_schema::NetGuidCache;

use crate::driver::RunTotals;
use crate::driver::checkpoints::CheckpointStats;
use crate::error::CliError;
use crate::sink::{ExportStats, PlayerIdentity};

/// Every run-level value needed to judge whether the published tables are
/// complete and how much typed decoding fell back to preserved raw data.
pub(crate) struct ManifestQuality<'a> {
    pub run: &'a RunTotals,
    pub net: &'a NetStats,
    pub error_report: &'a OverlayErrorReport,
    pub checkpoints: Option<&'a CheckpointStats>,
}

pub fn write_manifest(
    path: &Path,
    source_file: &str,
    file_size: usize,
    preamble: &Preamble,
    cache: &NetGuidCache,
    players: &[(u32, &PlayerIdentity)],
    quality: &ManifestQuality<'_>,
) -> Result<(), CliError> {
    let stats = quality.net;
    let header = &preamble.header;
    let ver = &header.replay_version;
    let info = &preamble.info;

    let mut doc = Object::new(0);
    // The first six keys are read by tools/to_valplay_bundle.py; keep their
    // names and types stable.
    doc.add("source_file", json_str(source_file))
        .add("source_size_bytes", file_size)
        .add("replay_build", json_str(&ver.branch))
        .add(
            "replay_version",
            json_str(&format!("{}.{}.{}", ver.major, ver.minor, ver.patch)),
        )
        .add("replay_changelist", ver.changelist)
        .add("duration_ms", info.length_in_ms)
        .add("elapsed_ms", quality.run.elapsed.as_millis())
        .add("friendly_name", json_str(&info.friendly_name))
        .add("is_live", info.is_live)
        .add("compressed", info.compressed)
        .add("encrypted", info.encrypted)
        // Unreal FDateTime ticks (100 ns since 0001-01-01, not FILETIME's
        // 1601), raw: the wire records no timezone.
        .add("timestamp_ticks", info.timestamp)
        // The info section's unvalidated copy (480767974 on 02d4d478); the
        // header's validated network_version (19 there) is not emitted.
        .add("info_network_version", info.network_version)
        .add("network_checksum", header.network_checksum)
        .add(
            "game_network_protocol_version",
            header.game_network_protocol_version,
        )
        // Four u32 words in serialisation order; a canonical GUID string
        // would impose a byte order nothing here can confirm.
        .add(
            "guid",
            format!(
                "[{}, {}, {}, {}]",
                header.guid[0], header.guid[1], header.guid[2], header.guid[3]
            ),
        )
        .add("ue4_version", header.ue4_version)
        .add("ue5_version", header.ue5_version)
        .add("package_version_license", header.package_version_license)
        .add("flags", header.flags)
        .add("platform", json_str(&header.platform))
        .add("build_config", header.build_config)
        .add("build_target_type", header.build_target_type)
        // Expected zero; see `ReplayHeader::trailing_bytes`.
        .add("header_trailing_bytes", header.trailing_bytes)
        .add("min_record_hz", json_f32(header.min_record_hz))
        .add("max_record_hz", json_f32(header.max_record_hz))
        .add("frame_limit_in_ms", json_f32(header.frame_limit_in_ms))
        .add(
            "checkpoint_limit_in_ms",
            json_f32(header.checkpoint_limit_in_ms),
        );
    let levels = (header.level_names_and_times.iter())
        .map(|(name, time)| format!("{{ \"name\": {}, \"time_ms\": {time} }}", json_str(name)))
        .collect();
    doc.add("level_names_and_times", array(1, levels));
    // After the scalars, so the readable metadata precedes the large blob.
    let gsd = header
        .game_specific_data
        .iter()
        .map(|s| json_str(s))
        .collect();
    doc.add("game_specific_data", array(1, gsd));

    let mut summary = Object::new(1);
    summary
        .add("packet_count", quality.run.total_packets)
        .add("bunch_count", stats.bunches)
        .add("malformed_packet_count", stats.malformed_packets)
        .add("partial_error_count", stats.partial_errors)
        .add("partial_fragments", stats.partial_fragments)
        .add("partial_completed", stats.partial_completed);
    doc.add("stats", summary.render());
    let mut counts = Object::new(1);
    counts
        .add("content_blocks", stats.content_blocks)
        .add("rep_layout_blocks", stats.rep_layout_blocks)
        .add("class_net_cache_blocks", stats.class_net_cache_blocks)
        .add("deleted_blocks", stats.deleted_blocks)
        .add("fields", stats.fields)
        .add("rpcs", stats.rpcs)
        .add("actor_opens", stats.actor_opens)
        .add("actor_closes", stats.actor_closes)
        .add("exported_guids", stats.exported_guids)
        .add("skipped_bits", stats.skipped_bits)
        .add("malformed_content_blocks", stats.malformed_content_blocks);
    doc.add("counts", counts.render());
    // Every loss and fallback counter, checkpoint pass included; `stats` and
    // `counts` above stay unchanged for the readers they already have.
    doc.add("quality", quality_json(quality));

    let players = (players.iter())
        .map(|&(guid, id)| player_json(guid, id))
        .collect();
    doc.add("players", array(1, players));
    // Net-field exports the cache could not place (an out-of-range handle), and
    // declarations that renamed what later rows mean. Expected zero.
    doc.add("dropped_field_exports", cache.dropped_field_exports())
        .add("replaced_export_groups", cache.replaced_export_groups())
        .add("renamed_field_exports", cache.renamed_field_exports());
    let groups = (cache.groups().iter())
        .map(|group| {
            let fields = (group.populated_fields())
                .map(|field| {
                    format!(
                        "{{ \"handle\": {}, \"name\": {}, \"compatible_checksum\": {} }}",
                        field.handle,
                        json_str(&field.name),
                        field.compatible_checksum
                    )
                })
                .collect();
            let mut object = Object::new(2);
            object
                .add("path", json_str(&group.path))
                .add("path_name_index", group.path_name_index)
                .add("fields", array(3, fields));
            object.render()
        })
        .collect();
    doc.add("net_field_export_groups", array(1, groups));

    fs::write(path, doc.render() + "\n")?;
    Ok(())
}

/// One BombPlayerState actor's account `subject` and `SpawnedCharacter`
/// (== movement.character_net_guid): the join from actor-keyed tables to
/// playerLoadouts identities, even when two players share an agent. The last
/// body is `character_net_guid`; `character_net_guids` keeps the pawns a
/// reconnect replaced.
fn player_json(guid: u32, id: &PlayerIdentity) -> String {
    let bodies: Vec<String> = id.character_net_guids.iter().map(u32::to_string).collect();
    format!(
        "{{ \"actor_net_guid\": {guid}, \"subject\": {}, \"character_net_guid\": {}, \
         \"character_net_guids\": [{}] }}",
        json_option(id.subject.as_deref()),
        bodies.last().map_or("null", String::as_str),
        bodies.join(", "),
    )
}

fn quality_json(quality: &ManifestQuality<'_>) -> String {
    let run = quality.run;
    let mut out = Object::new(1);
    // First: the verdict on whether anything is missing from the tables, from
    // the function `validate` prints. Everything after it is evidence.
    out.add("content_blocks_lost", quality.net.lost_content_blocks())
        .add("chunks_processed", run.chunks_processed)
        .add("unknown_chunks", run.unknown_chunks)
        .add("export_groups", run.export_groups)
        .add("movement_rows", run.movement_rows)
        .add("net_guid_rows", run.net_guid_rows)
        .add("event_rows", run.event_rows)
        .add("ability_cast_rows", run.ability_cast_rows)
        .add("partial_rows", run.partial_rows)
        .add("partial_bits", run.partial_bits)
        .add("event_trailing_bytes", run.event_trailing_bytes)
        .add("replay_data_trailing_bytes", run.replay_data_trailing_bytes);
    add_frame_skips(&mut out, "frame_", &run.frame_skips);
    out.add("frame_non_finite_times", run.non_finite_frame_times)
        .add("event_layout_mismatches", run.event_layout_mismatches)
        .add(
            "event_first_layout_mismatch",
            json_option(run.event_first_layout_mismatch.as_deref()),
        )
        .add("event_payloads_decoded", run.event_payloads_decoded)
        .add(
            "event_payload_unknown_groups",
            run.event_payload_unknown_groups,
        )
        .add("overlay_error_buckets", quality.error_report.bucket_count())
        .add(
            "overlay_errors_reported",
            quality.error_report.total_errors(),
        )
        .add("checkpoints_enabled", quality.checkpoints.is_some())
        .add("net", net_json(2, quality.net))
        .add("sink", sink_json(2, &run.sink));
    let checkpoints = quality
        .checkpoints
        .map_or_else(|| "null".to_owned(), checkpoints_json);
    out.add("checkpoints", checkpoints);
    out.render()
}

fn checkpoints_json(cp: &CheckpointStats) -> String {
    let mut out = Object::new(2);
    out.add(
        "checkpoint_path_resolution_mode",
        json_str("preceding_literal_zero_based"),
    )
    .add("checkpoint_literal_paths", cp.literal_paths)
    .add("checkpoint_indexed_paths", cp.indexed_paths)
    .add("checkpoint_resolved_path_indices", cp.resolved_path_indices)
    .add("checkpoint_chunks", cp.chunks)
    // The summary's `Trailing bytes:`; its ReplayData twin is
    // `replay_data_trailing_bytes`.
    .add("checkpoint_trailing_bytes", cp.trailing_bytes)
    .add("checkpoint_guid_entries", cp.guid_entries)
    .add("checkpoint_group_records", cp.group_records)
    .add("checkpoint_exported_fields", cp.exported_fields)
    .add("checkpoint_frames", cp.frames);
    add_frame_skips(&mut out, "checkpoint_frame_", &cp.frame_skips);
    out.add(
        "checkpoint_frame_non_finite_times",
        cp.non_finite_frame_times,
    )
    .add("checkpoint_packets", cp.packets)
    .add("checkpoint_field_rows", cp.field_rows)
    .add("checkpoint_actor_rows_written", cp.actor_rows_written)
    .add("checkpoint_net_guid_rows_written", cp.net_guid_rows_written)
    .add("checkpoint_block_rows_written", cp.block_rows_written)
    .add(
        "checkpoint_guid_entry_rows_written",
        cp.guid_entry_rows_written,
    )
    .add(
        "checkpoint_export_group_rows_written",
        cp.export_group_rows_written,
    )
    .add(
        "checkpoint_export_field_rows_written",
        cp.export_field_rows_written,
    )
    .add("checkpoint_partial_rows", cp.partial_rows)
    .add("checkpoint_partial_bits", cp.partial_bits)
    .add("checkpoint_actor_rows_dropped", cp.actor_rows_dropped)
    .add("checkpoint_movement_rows_dropped", cp.movement_rows_dropped)
    .add("net", net_json(3, &cp.net))
    .add("sink", sink_json(3, &cp.sink));
    out.render()
}

/// The three [`FrameSkips`] tallies, each key after `prefix`.
fn add_frame_skips(out: &mut Object, prefix: &str, skips: &FrameSkips) {
    out.add(
        format_args!("{prefix}external_data_blobs"),
        skips.external_data_blobs,
    )
    .add(
        format_args!("{prefix}external_data_bytes"),
        skips.external_data_bytes,
    )
    .add(
        format_args!("{prefix}game_specific_bytes"),
        skips.game_specific_bytes,
    );
}

fn net_json(level: usize, stats: &NetStats) -> String {
    let mut out = Object::new(level);
    for (key, value) in stats.counters() {
        out.add(key, value);
    }
    out.add("diagnostics_retained", stats.diagnostics.len());
    out.render()
}

/// A sink counter's manifest key: `sink_` on the four that must equal the
/// `net` block's same-named counter (tools/verify_build_corpus.py), so the
/// two sources stay apart.
fn sink_key(name: &str) -> String {
    let paired = [
        "rpcs_emitted",
        "actor_opens",
        "actor_closes",
        "content_blocks",
    ];
    let prefix = if paired.contains(&name) { "sink_" } else { "" };
    format!("{prefix}{name}")
}

fn sink_json(level: usize, sink: &ExportStats) -> String {
    let mut out = Object::new(level);
    for (key, value) in sink.counters() {
        out.add(sink_key(key), value);
    }
    out.add(
        "struct_blob_first_error",
        json_option(sink.struct_blob_first_error.as_deref()),
    )
    .add(
        "movement_first_error",
        json_option(sink.movement_first_error.as_deref()),
    );
    out.render()
}

/// A JSON object whose members are rendered as they are added, one per line
/// at `level + 1` (two spaces a level); `"key": value`, the separator the
/// integration tests search for.
struct Object {
    level: usize,
    members: Vec<String>,
}

impl Object {
    fn new(level: usize) -> Self {
        Self {
            level,
            members: Vec::new(),
        }
    }

    /// `value` must already be JSON: numbers and bools print as themselves.
    fn add(&mut self, key: impl Display, value: impl Display) -> &mut Self {
        let indent = "  ".repeat(self.level + 1);
        self.members.push(format!("{indent}\"{key}\": {value}"));
        self
    }

    fn render(&self) -> String {
        format!(
            "{{\n{}\n{}}}",
            self.members.join(",\n"),
            "  ".repeat(self.level)
        )
    }
}

/// A JSON array of already-rendered items, one per line; `[]` when empty.
fn array(level: usize, items: Vec<String>) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }
    let indent = "  ".repeat(level + 1);
    let items: Vec<String> = items
        .into_iter()
        .map(|item| indent.clone() + &item)
        .collect();
    format!("[\n{}\n{}]", items.join(",\n"), "  ".repeat(level))
}

fn json_option(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_str)
}

/// JSON number for an `f32`, or `null` when it is not finite: Rust prints
/// `NaN`, `inf` and `-inf`, which JSON does not admit, and the recording-rate
/// floats come straight off the wire.
fn json_f32(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_string()
    }
}

/// JSON-escape a string: only what RFC 8259 requires (`"`, `\`, U+0000..U+001F);
/// everything else passes through into the UTF-8 manifest. That holds for the
/// UTF-16 game-specific blob too: `BitReader::read_fstring` decodes with the
/// strict `String::from_utf16`, so no unpaired surrogate reaches this function.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use vrf_decode::{DecodeErrorKind, FieldType};

    /// Every scalar member of `quality`, both passes included, against the
    /// values the test put in: each counter of both passes' `NetStats` and
    /// `ExportStats` (named by their own lists) and each `RunTotals` and
    /// `CheckpointStats` field (named here) gets a distinct number, so a key
    /// that is missing, duplicated, misspelled or wired to another field or
    /// pass changes the multiset.
    #[test]
    fn quality_json_publishes_every_member_once_with_its_own_value() {
        let mut expected: Vec<(String, String)> = Vec::new();
        let mut last = 1000u64;
        let mut next = |key: &str| {
            last += 1;
            if !key.is_empty() {
                expected.push((key.to_owned(), last.to_string()));
            }
            last
        };
        let (mut net, mut cp_net) = (NetStats::default(), NetStats::default());
        let (mut sink, mut cp_sink) = (ExportStats::default(), ExportStats::default());
        for stats in [&mut net, &mut cp_net] {
            for (name, value) in stats.counters_mut() {
                *value = next(name);
            }
        }
        let paired = [
            "rpcs_emitted",
            "actor_opens",
            "actor_closes",
            "content_blocks",
        ];
        for stats in [&mut sink, &mut cp_sink] {
            for (name, value) in stats.counters_mut() {
                let prefix = if paired.contains(&name) { "sink_" } else { "" };
                *value = next(&format!("{prefix}{name}"));
            }
        }
        let mut run = RunTotals {
            chunks_processed: next("chunks_processed") as u32,
            unknown_chunks: next("unknown_chunks"),
            // Not in `quality`: printed by the summary only.
            frames: next("") as u32,
            frame_skips: FrameSkips::default(),
            non_finite_frame_times: next("frame_non_finite_times"),
            total_packets: next("") as u32,
            export_groups: next("export_groups") as usize,
            movement_rows: next("movement_rows"),
            net_guid_rows: next("net_guid_rows") as usize,
            event_rows: next("event_rows"),
            ability_cast_rows: next("ability_cast_rows"),
            partial_rows: next("partial_rows"),
            partial_bits: next("partial_bits"),
            event_trailing_bytes: next("event_trailing_bytes"),
            replay_data_trailing_bytes: next("replay_data_trailing_bytes"),
            elapsed: Duration::ZERO,
            event_layout_mismatches: next("event_layout_mismatches"),
            event_first_layout_mismatch: Some("event".to_owned()),
            event_payloads_decoded: next("event_payloads_decoded"),
            event_payload_unknown_groups: next("event_payload_unknown_groups"),
            sink: ExportStats::default(),
            stale_checkpoint_note: None,
        };
        run.frame_skips.external_data_blobs = next("frame_external_data_blobs");
        run.frame_skips.external_data_bytes = next("frame_external_data_bytes");
        run.frame_skips.game_specific_bytes = next("frame_game_specific_bytes");
        let mut cp = CheckpointStats {
            chunks: next("checkpoint_chunks"),
            trailing_bytes: next("checkpoint_trailing_bytes"),
            guid_entries: next("checkpoint_guid_entries"),
            literal_paths: next("checkpoint_literal_paths"),
            indexed_paths: next("checkpoint_indexed_paths"),
            resolved_path_indices: next("checkpoint_resolved_path_indices"),
            group_records: next("checkpoint_group_records"),
            exported_fields: next("checkpoint_exported_fields"),
            frames: next("checkpoint_frames"),
            frame_skips: FrameSkips::default(),
            non_finite_frame_times: next("checkpoint_frame_non_finite_times"),
            packets: next("checkpoint_packets"),
            field_rows: next("checkpoint_field_rows"),
            actor_rows_written: next("checkpoint_actor_rows_written"),
            net_guid_rows_written: next("checkpoint_net_guid_rows_written"),
            block_rows_written: next("checkpoint_block_rows_written"),
            guid_entry_rows_written: next("checkpoint_guid_entry_rows_written"),
            export_group_rows_written: next("checkpoint_export_group_rows_written"),
            export_field_rows_written: next("checkpoint_export_field_rows_written"),
            partial_rows: next("checkpoint_partial_rows"),
            partial_bits: next("checkpoint_partial_bits"),
            actor_rows_dropped: next("checkpoint_actor_rows_dropped"),
            movement_rows_dropped: next("checkpoint_movement_rows_dropped"),
            sink: ExportStats::default(),
            net: NetStats::default(),
        };
        cp.frame_skips.external_data_blobs = next("checkpoint_frame_external_data_blobs");
        cp.frame_skips.external_data_bytes = next("checkpoint_frame_external_data_bytes");
        cp.frame_skips.game_specific_bytes = next("checkpoint_frame_game_specific_bytes");
        sink.struct_blob_first_error = Some("blob".to_owned());
        cp_sink.movement_first_error = Some("movement".to_owned());
        (run.sink, cp.sink, cp.net) = (sink, cp_sink, cp_net);
        let mut errors = OverlayErrorReport::default();
        for field in ["A", "B", "B"] {
            errors.record("/Group", field, FieldType::Int32, 32, DecodeErrorKind::Eof);
        }
        for (key, value) in [
            ("content_blocks_lost", net.lost_content_blocks().to_string()),
            ("overlay_error_buckets", "2".to_owned()),
            ("overlay_errors_reported", "3".to_owned()),
            ("checkpoints_enabled", "true".to_owned()),
            ("event_first_layout_mismatch", "\"event\"".to_owned()),
            (
                "checkpoint_path_resolution_mode",
                "\"preceding_literal_zero_based\"".to_owned(),
            ),
            ("diagnostics_retained", "0".to_owned()),
            ("diagnostics_retained", "0".to_owned()),
            ("struct_blob_first_error", "\"blob\"".to_owned()),
            ("struct_blob_first_error", "null".to_owned()),
            ("movement_first_error", "null".to_owned()),
            ("movement_first_error", "\"movement\"".to_owned()),
        ] {
            expected.push((key.to_owned(), value));
        }

        let json = quality_json(&ManifestQuality {
            run: &run,
            net: &net,
            error_report: &errors,
            checkpoints: Some(&cp),
        });
        let mut published: Vec<(String, String)> = (json.lines())
            .filter_map(|line| line.trim().split_once("\": "))
            .filter(|(_, value)| !value.starts_with(['{', '[']))
            .map(|(key, value)| (key[1..].to_owned(), value.trim_end_matches(',').to_owned()))
            .collect();
        published.sort();
        expected.sort();
        assert_eq!(published, expected);
    }

    /// The number a consumer reads before any coverage claim, checked with all
    /// five failure depths non-zero and unequal: a constant would read as
    /// nothing lost.
    #[test]
    fn content_blocks_lost_publishes_the_measured_number() {
        let mut net = NetStats::default();
        net.content_block_framing_failures = 8;
        net.malformed_content_blocks = 1;
        net.transform_failures = 2;
        net.field_stream_failures = 4;
        net.rpc_stream_failures = 100;
        net.unresolved_rpc_payloads_preserved = 92;
        let json = quality_json(&ManifestQuality {
            run: &RunTotals::default(),
            net: &net,
            error_report: &OverlayErrorReport::default(),
            checkpoints: None,
        });
        assert!(json.contains("\"content_blocks_lost\": 23,"), "{json}");
        assert!(json.contains("\"checkpoints\": null\n"), "{json}");
    }

    #[test]
    fn a_player_lists_every_body_and_names_the_last() {
        let mut id = PlayerIdentity {
            subject: Some("uuid".to_owned()),
            character_net_guids: vec![1510, 45530],
        };
        assert_eq!(
            player_json(9, &id),
            "{ \"actor_net_guid\": 9, \"subject\": \"uuid\", \"character_net_guid\": 45530, \
             \"character_net_guids\": [1510, 45530] }"
        );
        id.character_net_guids.clear();
        assert!(
            player_json(9, &id)
                .ends_with("\"character_net_guid\": null, \"character_net_guids\": [] }")
        );
    }

    #[test]
    fn json_str_escapes_only_what_rfc_8259_requires() {
        assert_eq!(json_str("plain"), "\"plain\"");
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_str("a\nb\rc\td"), "\"a\\nb\\rc\\td\"");
        // Control characters without a short form take the \u escape.
        assert_eq!(json_str("\u{0}\u{1}\u{1f}"), "\"\\u0000\\u0001\\u001f\"");
    }

    #[test]
    fn json_str_passes_non_ascii_through_verbatim() {
        // ASCII source (check_ascii.py), non-ASCII values. DEL and the C1
        // range sit above U+001F, so escaping them would be wrong.
        for s in [
            "\u{7f}",           // DEL
            "\u{80}\u{9f}",     // C1 controls
            "\u{e9}\u{440}",    // Latin-1 supplement, Cyrillic
            "\u{d55c}\u{ad6d}", // Hangul, as a Valorant player name may carry
            "\u{1f600}",        // astral plane (surrogate pair on the wire)
            "\u{2028}\u{2029}", // line/paragraph separator: legal JSON
            "\u{feff}",         // BOM as an interior character
        ] {
            assert_eq!(json_str(s), format!("\"{s}\""), "mangled {s:?}");
        }
    }

    #[test]
    fn json_f32_replaces_non_finite_with_null() {
        assert_eq!(json_f32(30.0), "30");
        assert_eq!(json_f32(-0.5), "-0.5");
        assert_eq!(json_f32(f32::NAN), "null");
        assert_eq!(json_f32(f32::INFINITY), "null");
        assert_eq!(json_f32(f32::NEG_INFINITY), "null");
    }

    #[test]
    fn objects_and_arrays_render_one_member_per_line() {
        let mut object = Object::new(1);
        object
            .add("empty", array(2, Vec::new()))
            .add("pair", array(2, vec!["1".into(), "2".into()]));
        assert_eq!(
            object.render(),
            "{\n    \"empty\": [],\n    \"pair\": [\n      1,\n      2\n    ]\n  }"
        );
    }
}
